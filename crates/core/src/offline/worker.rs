//! Atomic `.part` resumable download worker (`DownloadManager`).

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use reqwest::header::{CONTENT_RANGE, RANGE, RETRY_AFTER};
use resonance_contract::{
    AppEvent, CoreError, CoreResult, DownloadRecord, DownloadStatus, EventBus, LyricsProvider,
    MusicProvider, OfflineStorageError, OfflineStore, StreamInfo, Track, TrackId,
};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore, watch};
use tokio::task::JoinHandle;

use crate::cache::{ArtworkCache, CHUNK_SIZE, ChunkCache};
use crate::offline::{
    DEFAULT_MAX_OFFLINE_BYTES, MAX_SINGLE_TRACK_BYTES, final_filename, part_filename,
    redact_error_message, resolve_confined_path, tagger, validate_video_id,
};
use crate::unix_secs;

const DEFAULT_CONCURRENCY: usize = 2;
const MODE_RUNNING: u8 = 0;
const MODE_PAUSE: u8 = 1;
const MODE_CANCEL: u8 = 2;

struct JobHandle {
    cancel_tx: watch::Sender<u8>,
    job_gen: u64,
    join: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct DownloadManager {
    provider: Arc<dyn MusicProvider>,
    offline: Arc<dyn OfflineStore>,
    chunks: Arc<ChunkCache>,
    artwork: Arc<ArtworkCache>,
    lyrics: Arc<dyn LyricsProvider>,
    bus: EventBus,
    http: reqwest::Client,
    concurrency: Arc<Semaphore>,
    max_offline_bytes: u64,
    active_jobs: Arc<Mutex<HashMap<TrackId, JobHandle>>>,
    next_gen: Arc<AtomicU64>,
}

impl DownloadManager {
    pub fn new(
        provider: Arc<dyn MusicProvider>,
        offline: Arc<dyn OfflineStore>,
        chunks: Arc<ChunkCache>,
        artwork: Arc<ArtworkCache>,
        lyrics: Arc<dyn LyricsProvider>,
        bus: EventBus,
    ) -> CoreResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string())))?;

        Ok(Self {
            provider,
            offline,
            chunks,
            artwork,
            lyrics,
            bus,
            http,
            concurrency: Arc::new(Semaphore::new(DEFAULT_CONCURRENCY)),
            max_offline_bytes: DEFAULT_MAX_OFFLINE_BYTES,
            active_jobs: Arc::new(Mutex::new(HashMap::new())),
            next_gen: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn with_max_offline_bytes(mut self, max_bytes: u64) -> Self {
        self.max_offline_bytes = max_bytes;
        self
    }

    /// Reconciles interrupted downloads on startup and purges orphan `.part`/`.tmp` files.
    pub async fn startup_recovery(&self) -> CoreResult<usize> {
        let root = self.offline.downloads_root();
        let records = self.offline.list_downloads()?;
        let now = unix_secs();
        let mut recovered = 0usize;
        let mut known_ids = HashSet::with_capacity(records.len());

        for rec in records {
            known_ids.insert(rec.video_id.clone());
            if matches!(
                rec.status,
                DownloadStatus::Downloading | DownloadStatus::Tagging
            ) {
                let target_status = if let Ok(fname) =
                    final_filename(&rec.video_id, rec.itag, rec.codec, &rec.mime_type)
                    && let Ok(part_path) = resolve_confined_path(&root, &part_filename(&fname))
                    && part_path.is_file()
                    && fs::metadata(&part_path)
                        .map(|m| m.len() == rec.bytes_downloaded)
                        .unwrap_or(false)
                {
                    DownloadStatus::Paused
                } else {
                    DownloadStatus::Queued
                };

                let new_bytes = if target_status == DownloadStatus::Paused {
                    rec.bytes_downloaded
                } else {
                    0
                };
                self.offline.update_download_progress(
                    &rec.video_id,
                    target_status,
                    new_bytes,
                    rec.total_bytes,
                    now,
                )?;
                if let Ok(Some(updated)) = self.offline.get_download(&rec.video_id) {
                    self.bus.publish(AppEvent::DownloadUpdated(updated));
                }
                recovered += 1;
            }
        }

        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if name.ends_with(".part") || name.ends_with(".tmp") {
                    let is_orphan = extract_video_id_from_temp(name)
                        .map(|vid| !known_ids.contains(vid))
                        .unwrap_or(true);
                    if is_orphan {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }

        Ok(recovered)
    }

    /// Enqueues a track for offline download (idempotent if already completed or active).
    pub async fn enqueue(&self, track: Track) -> CoreResult<DownloadRecord> {
        validate_video_id(&track.id)?;
        let root = self.offline.downloads_root();
        let mut jobs = self.active_jobs.lock().await;

        if jobs.contains_key(&track.id)
            && let Some(existing) = self.offline.get_download(&track.id)?
        {
            return Ok(existing);
        }

        if let Some(existing) = self.offline.get_download(&track.id)? {
            match existing.status {
                DownloadStatus::Completed => {
                    if let Some(rel) = existing.relative_path.as_deref()
                        && let Ok(p) = resolve_confined_path(&root, rel)
                        && p.is_file()
                        && fs::metadata(&p)
                            .map(|m| m.len() == existing.total_bytes && m.len() > 0)
                            .unwrap_or(false)
                    {
                        return Ok(existing);
                    }
                }
                DownloadStatus::Queued | DownloadStatus::Downloading | DownloadStatus::Tagging => {
                    return Ok(existing);
                }
                DownloadStatus::Paused | DownloadStatus::Failed | DownloadStatus::Cancelled => {}
            }
        }

        let stats = self.offline.offline_stats(self.max_offline_bytes)?;
        if stats.total_bytes >= self.max_offline_bytes {
            return Err(CoreError::OfflineStorage(
                OfflineStorageError::QuotaExceeded {
                    used_bytes: stats.total_bytes,
                    limit_bytes: self.max_offline_bytes,
                },
            ));
        }

        let now = unix_secs();
        let record = DownloadRecord {
            video_id: track.id.clone(),
            track: track.clone(),
            status: DownloadStatus::Queued,
            itag: 140,
            codec: resonance_contract::AudioCodec::Aac,
            mime_type: "audio/mp4".to_owned(),
            relative_path: None,
            bytes_downloaded: 0,
            total_bytes: 0,
            sha256_hex: None,
            tags_embedded: false,
            created_at_unix_secs: now,
            updated_at_unix_secs: now,
            completed_at_unix_secs: None,
            error_message: None,
        };
        self.offline.upsert_download(&record)?;
        self.bus.publish(AppEvent::DownloadUpdated(record.clone()));

        self.spawn_worker_locked(&mut jobs, record.clone());
        Ok(record)
    }

    /// Pauses an active download, keeping its `.part` file for later resume.
    pub async fn pause(&self, video_id: &str) -> CoreResult<()> {
        validate_video_id(video_id)?;
        let handle = {
            let mut jobs = self.active_jobs.lock().await;
            jobs.remove(video_id)
        };
        if let Some(mut h) = handle {
            let _ = h.cancel_tx.send(MODE_PAUSE);
            if let Some(join) = h.join.take() {
                let _ = join.await;
            }
            return Ok(());
        }

        if let Some(rec) = self.offline.get_download(video_id)?
            && matches!(
                rec.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            )
        {
            self.offline.update_download_progress(
                video_id,
                DownloadStatus::Paused,
                rec.bytes_downloaded,
                rec.total_bytes,
                unix_secs(),
            )?;
            if let Ok(Some(updated)) = self.offline.get_download(video_id) {
                self.bus.publish(AppEvent::DownloadUpdated(updated));
            }
        }
        Ok(())
    }

    /// Resumes a paused or failed download from its existing `.part` byte offset.
    pub async fn resume(&self, video_id: &str) -> CoreResult<DownloadRecord> {
        validate_video_id(video_id)?;
        let mut jobs = self.active_jobs.lock().await;
        let Some(existing) = self.offline.get_download(video_id)? else {
            return Err(CoreError::OfflineStorage(OfflineStorageError::NotFound {
                video_id: video_id.to_owned(),
            }));
        };

        if jobs.contains_key(video_id) {
            return Ok(existing);
        }

        if existing.status == DownloadStatus::Completed {
            return Ok(existing);
        }
        if matches!(
            existing.status,
            DownloadStatus::Queued | DownloadStatus::Downloading | DownloadStatus::Tagging
        ) {
            return Ok(existing);
        }

        let stats = self.offline.offline_stats(self.max_offline_bytes)?;
        if stats.total_bytes >= self.max_offline_bytes {
            return Err(CoreError::OfflineStorage(
                OfflineStorageError::QuotaExceeded {
                    used_bytes: stats.total_bytes,
                    limit_bytes: self.max_offline_bytes,
                },
            ));
        }

        let now = unix_secs();
        self.offline.update_download_progress(
            video_id,
            DownloadStatus::Queued,
            existing.bytes_downloaded,
            existing.total_bytes,
            now,
        )?;
        let updated = self
            .offline
            .get_download(video_id)?
            .unwrap_or_else(|| existing.clone());
        self.bus.publish(AppEvent::DownloadUpdated(updated.clone()));

        self.spawn_worker_locked(&mut jobs, updated.clone());
        Ok(updated)
    }

    /// Cancels an active or queued download, removing any `.part` file.
    pub async fn cancel(&self, video_id: &str) -> CoreResult<()> {
        validate_video_id(video_id)?;
        let handle = {
            let mut jobs = self.active_jobs.lock().await;
            jobs.remove(video_id)
        };
        if let Some(mut h) = handle {
            let _ = h.cancel_tx.send(MODE_CANCEL);
            if let Some(join) = h.join.take() {
                let _ = join.await;
            }
            return Ok(());
        }

        if let Some(rec) = self.offline.get_download(video_id)? {
            self.remove_part_file(&rec);
            self.offline.mark_download_failed(
                video_id,
                DownloadStatus::Cancelled,
                None,
                unix_secs(),
            )?;
            if let Ok(Some(updated)) = self.offline.get_download(video_id) {
                self.bus.publish(AppEvent::DownloadUpdated(updated));
            }
        }
        Ok(())
    }

    /// Cancels any active job, removes the SQLite row, deletes finalized and `.part` files,
    /// and emits `AppEvent::DownloadRemoved`.
    pub async fn delete(&self, video_id: &str) -> CoreResult<()> {
        validate_video_id(video_id)?;
        let handle = {
            let mut jobs = self.active_jobs.lock().await;
            jobs.remove(video_id)
        };
        if let Some(mut h) = handle {
            let _ = h.cancel_tx.send(MODE_CANCEL);
            if let Some(join) = h.join.take() {
                let _ = join.await;
            }
        }

        if let Some(removed) = self.offline.delete_download_record(video_id)? {
            let root = self.offline.downloads_root();
            if let Some(rel) = removed.relative_path.as_deref()
                && let Ok(final_path) = resolve_confined_path(&root, rel)
            {
                let _ = fs::remove_file(final_path);
            }
            self.remove_part_file(&removed);
        }

        self.bus.publish(AppEvent::DownloadRemoved {
            video_id: video_id.to_owned(),
        });
        Ok(())
    }

    fn remove_part_file(&self, rec: &DownloadRecord) {
        let root = self.offline.downloads_root();
        if let Ok(fname) = final_filename(&rec.video_id, rec.itag, rec.codec, &rec.mime_type)
            && let Ok(part_path) = resolve_confined_path(&root, &part_filename(&fname))
        {
            let _ = fs::remove_file(part_path);
        }
    }

    fn spawn_worker_locked(
        &self,
        jobs: &mut HashMap<String, JobHandle>,
        initial_record: DownloadRecord,
    ) {
        let video_id = initial_record.video_id.clone();
        if jobs.contains_key(&video_id) {
            return;
        }
        let job_gen = self.next_gen.fetch_add(1, Ordering::SeqCst);
        let (cancel_tx, cancel_rx) = watch::channel(MODE_RUNNING);

        let mgr = self.clone();
        let vid_for_cleanup = video_id.clone();
        let join = tokio::spawn(async move {
            mgr.run_download_job(initial_record, cancel_rx).await;
            let mut jobs = mgr.active_jobs.lock().await;
            if jobs
                .get(&vid_for_cleanup)
                .is_some_and(|h| h.job_gen == job_gen)
            {
                jobs.remove(&vid_for_cleanup);
            }
        });

        jobs.insert(
            video_id,
            JobHandle {
                cancel_tx,
                job_gen,
                join: Some(join),
            },
        );
    }

    async fn run_download_job(
        &self,
        mut record: DownloadRecord,
        mut cancel_rx: watch::Receiver<u8>,
    ) {
        let video_id = record.video_id.clone();

        // 1. Acquire concurrency permit (cancel-aware).
        let _permit = tokio::select! {
            _ = cancel_rx.changed() => {
                let mode = *cancel_rx.borrow();
                self.handle_abort(&record, mode, None).await;
                return;
            }
            res = self.concurrency.clone().acquire_owned() => {
                match res {
                    Ok(p) => p,
                    Err(_) => return,
                }
            }
        };

        let mode = *cancel_rx.borrow();
        if mode != MODE_RUNNING {
            self.handle_abort(&record, mode, None).await;
            return;
        }

        // 2. Resolve stream (AuthMode isolation stays inside MusicProvider).
        let mut stream = tokio::select! {
            _ = cancel_rx.changed() => {
                let mode = *cancel_rx.borrow();
                self.handle_abort(&record, mode, None).await;
                return;
            }
            res = self.provider.resolve_stream(&video_id) => {
                match res {
                    Ok(s) => s,
                    Err(e) => {
                        let msg = redact_error_message(&e.to_string());
                        self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
                        return;
                    }
                }
            }
        };

        // 3. Validate size & quota.
        if stream.content_length > MAX_SINGLE_TRACK_BYTES {
            self.mark_failed_and_emit(
                &video_id,
                DownloadStatus::Failed,
                Some("track exceeds 512 MB offline limit"),
            );
            return;
        }

        if let Ok(stats) = self.offline.offline_stats(self.max_offline_bytes) {
            let projected = stats.total_bytes.saturating_add(
                stream
                    .content_length
                    .saturating_sub(record.bytes_downloaded),
            );
            if projected > self.max_offline_bytes {
                self.mark_failed_and_emit(
                    &video_id,
                    DownloadStatus::Failed,
                    Some("offline storage quota exceeded"),
                );
                return;
            }
        }

        let root = self.offline.downloads_root();
        let mut final_name;
        let mut final_path;
        let mut part_path;
        let mut total_bytes;
        let mut offset;

        let mut restart_count = 0u8;
        'download_attempt: loop {
            // Persist resolved stream metadata (never StreamInfo.url).
            record.itag = stream.itag;
            record.codec = stream.codec;
            record.mime_type = stream.mime_type.clone();
            record.total_bytes = stream.content_length;
            record.status = DownloadStatus::Downloading;
            record.updated_at_unix_secs = unix_secs();
            record.error_message = None;
            let _ = self.offline.upsert_download(&record);
            self.bus.publish(AppEvent::DownloadUpdated(record.clone()));

            let Ok(fname) =
                final_filename(&video_id, stream.itag, stream.codec, &stream.mime_type)
            else {
                self.mark_failed_and_emit(
                    &video_id,
                    DownloadStatus::Failed,
                    Some("invalid video id"),
                );
                return;
            };
            final_name = fname;
            let part_name = part_filename(&final_name);
            let Ok(fpath) = resolve_confined_path(&root, &final_name) else {
                self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some("invalid path"));
                return;
            };
            final_path = fpath;
            let Ok(ppath) = resolve_confined_path(&root, &part_name) else {
                self.mark_failed_and_emit(
                    &video_id,
                    DownloadStatus::Failed,
                    Some("invalid part path"),
                );
                return;
            };
            part_path = ppath;

            // 4. Open .part file and align resume offset to CHUNK_SIZE.
            let mut file = match OpenOptions::new()
                .create(true)
                .truncate(restart_count > 0)
                .read(true)
                .write(true)
                .open(&part_path)
            {
                Ok(f) => f,
                Err(e) => {
                    let msg = redact_error_message(&e.to_string());
                    self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
                    return;
                }
            };

            total_bytes = stream.content_length;
            let part_len = file.metadata().map(|m| m.len()).unwrap_or(0);
            let capped_len = if total_bytes > 0 {
                part_len.min(total_bytes)
            } else {
                part_len
            };
            let aligned_offset = (capped_len / CHUNK_SIZE) * CHUNK_SIZE;
            if let Err(e) = file
                .set_len(aligned_offset)
                .and_then(|_| file.seek(SeekFrom::Start(aligned_offset)).map(|_| ()))
            {
                let msg = redact_error_message(&e.to_string());
                self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
                return;
            }

            offset = aligned_offset;
            let mut chunk_idx = aligned_offset / CHUNK_SIZE;
            let mut last_reported_bytes = offset;
            let mut last_report_time = Instant::now();

            // 5. Chunked download loop.
            while total_bytes == 0 || offset < total_bytes {
                let mode = *cancel_rx.borrow();
                if mode != MODE_RUNNING {
                    drop(file);
                    self.handle_abort(&record, mode, Some(&part_path)).await;
                    return;
                }

                // Tier 1 ChunkCache reuse (zero network I/O).
                if let Ok(Some((bytes, chunk_total))) =
                    self.chunks.read_chunk(&video_id, stream.itag, chunk_idx)
                {
                    if total_bytes == 0 && chunk_total > 0 {
                        total_bytes = chunk_total;
                    }
                    if let Err(e) = file.write_all(&bytes) {
                        let msg = redact_error_message(&e.to_string());
                        self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
                        return;
                    }
                    offset += bytes.len() as u64;
                    chunk_idx += 1;
                    record.bytes_downloaded = offset;
                    record.total_bytes = total_bytes;
                    self.maybe_report_progress(
                        &mut record,
                        &mut last_reported_bytes,
                        &mut last_report_time,
                    );
                    continue;
                }

                // Upstream Range fetch with 403 re-resolve (max 2) and 429 backoff (max 4).
                let chunk_bytes = match self
                    .fetch_chunk_with_recovery(
                        &video_id,
                        &mut stream,
                        chunk_idx,
                        &mut total_bytes,
                        &mut cancel_rx,
                    )
                    .await
                {
                    ChunkFetch::Data(b) => b,
                    ChunkFetch::StreamChanged(new_stream) => {
                        drop(file);
                        let _ = fs::remove_file(&part_path);
                        if restart_count >= 1 {
                            self.mark_failed_and_emit(
                                &video_id,
                                DownloadStatus::Failed,
                                Some("stream format changed repeatedly during download"),
                            );
                            return;
                        }
                        restart_count += 1;
                        stream = new_stream;
                        record.bytes_downloaded = 0;
                        continue 'download_attempt;
                    }
                    ChunkFetch::Aborted(mode) => {
                        drop(file);
                        self.handle_abort(&record, mode, Some(&part_path)).await;
                        return;
                    }
                    ChunkFetch::Paused(err_msg) => {
                        let _ = file.flush();
                        let _ = file.sync_all();
                        drop(file);
                        record.bytes_downloaded = offset;
                        record.total_bytes = total_bytes;
                        let _ = self.offline.update_download_progress(
                            &video_id,
                            DownloadStatus::Paused,
                            offset,
                            total_bytes,
                            unix_secs(),
                        );
                        self.mark_failed_and_emit(
                            &video_id,
                            DownloadStatus::Paused,
                            Some(&err_msg),
                        );
                        return;
                    }
                    ChunkFetch::Failed(err_msg) => {
                        drop(file);
                        self.mark_failed_and_emit(
                            &video_id,
                            DownloadStatus::Failed,
                            Some(&err_msg),
                        );
                        return;
                    }
                };

                if chunk_bytes.is_empty() {
                    if total_bytes == 0 {
                        total_bytes = offset;
                    }
                    break;
                }

                if let Err(e) = file.write_all(&chunk_bytes) {
                    let msg = redact_error_message(&e.to_string());
                    self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
                    return;
                }
                offset += chunk_bytes.len() as u64;
                chunk_idx += 1;
                record.bytes_downloaded = offset;
                record.total_bytes = total_bytes;

                if total_bytes == 0 && (chunk_bytes.len() as u64) < CHUNK_SIZE {
                    total_bytes = offset;
                    record.total_bytes = total_bytes;
                }

                self.maybe_report_progress(
                    &mut record,
                    &mut last_reported_bytes,
                    &mut last_report_time,
                );
            }

            // 6. Flush + sync_all and close .part handle.
            if let Err(e) = file.flush().and_then(|_| file.sync_all()) {
                let msg = redact_error_message(&e.to_string());
                self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
                return;
            }
            drop(file);
            break;
        }

        // 7. Transition to Tagging.
        let now = unix_secs();
        record.status = DownloadStatus::Tagging;
        record.bytes_downloaded = offset;
        record.total_bytes = total_bytes.max(offset);
        record.updated_at_unix_secs = now;
        let _ = self.offline.update_download_progress(
            &video_id,
            DownloadStatus::Tagging,
            record.bytes_downloaded,
            record.total_bytes,
            now,
        );
        self.bus.publish(AppEvent::DownloadUpdated(record.clone()));

        // 8. Best-effort artwork & lyrics fetch (5s timeout each).
        let artwork_bytes = self.fetch_artwork_best_effort(&record.track).await;
        let lyrics_doc = tokio::time::timeout(
            Duration::from_secs(5),
            self.lyrics.fetch_lyrics(&record.track),
        )
        .await
        .ok()
        .and_then(|r| r.ok())
        .flatten();

        // 9 & 10. Embed metadata & compute SHA-256 on blocking pool.
        let part_for_tag = part_path.clone();
        let track_for_tag = record.track.clone();
        let tag_and_hash =
            tokio::task::spawn_blocking(move || -> CoreResult<(bool, String, u64)> {
                let tagged = tagger::embed_metadata(
                    &part_for_tag,
                    &track_for_tag,
                    artwork_bytes.as_deref(),
                    lyrics_doc.as_ref(),
                )?;

                let mut f = fs::File::open(&part_for_tag).map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string()))
                })?;
                let final_len = f
                    .metadata()
                    .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string())))?
                    .len();
                let mut hasher = Sha256::new();
                let mut buf = [0u8; 64 * 1024];
                loop {
                    let n = f.read(&mut buf).map_err(|e| {
                        CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string()))
                    })?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                let sha_hex = hex::encode(hasher.finalize());
                Ok((tagged, sha_hex, final_len))
            })
            .await;

        let (tags_embedded, sha256_hex, final_len) = match tag_and_hash {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                let _ = fs::remove_file(&part_path);
                let msg = redact_error_message(&e.to_string());
                self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
                return;
            }
            Err(e) => {
                let msg = redact_error_message(&e.to_string());
                self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
                return;
            }
        };

        // 11. Final cancel check before atomic rename.
        let mode = *cancel_rx.borrow();
        if mode != MODE_RUNNING {
            self.handle_abort(&record, mode, Some(&part_path)).await;
            return;
        }

        // 12. Atomic commit: fs::rename(.part -> final) + dir sync on Unix.
        if let Err(e) = fs::rename(&part_path, &final_path) {
            let msg = redact_error_message(&e.to_string());
            self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
            return;
        }
        #[cfg(unix)]
        if let Ok(dir_f) = fs::File::open(&root) {
            let _ = dir_f.sync_all();
        }

        // 13. Mark completed & emit event.
        let done_ts = unix_secs();
        if let Err(e) = self.offline.mark_download_completed(
            &video_id,
            &final_name,
            final_len,
            &sha256_hex,
            tags_embedded,
            done_ts,
        ) {
            let msg = redact_error_message(&e.to_string());
            self.mark_failed_and_emit(&video_id, DownloadStatus::Failed, Some(&msg));
            return;
        }

        if let Ok(Some(completed)) = self.offline.get_download(&video_id) {
            self.bus.publish(AppEvent::DownloadUpdated(completed));
        }
    }

    async fn fetch_chunk_with_recovery(
        &self,
        video_id: &str,
        stream: &mut StreamInfo,
        chunk_idx: u64,
        total_bytes: &mut u64,
        cancel_rx: &mut watch::Receiver<u8>,
    ) -> ChunkFetch {
        let mut reresolve_count = 0u32;
        let mut rate_limit_count = 0u32;

        loop {
            let mode = *cancel_rx.borrow();
            if mode != MODE_RUNNING {
                return ChunkFetch::Aborted(mode);
            }

            let start = chunk_idx * CHUNK_SIZE;
            let end = if *total_bytes > 0 {
                (start + CHUNK_SIZE - 1).min(*total_bytes - 1)
            } else {
                start + CHUNK_SIZE - 1
            };

            let resp = tokio::select! {
                _ = cancel_rx.changed() => {
                    return ChunkFetch::Aborted(*cancel_rx.borrow());
                }
                res = self.http.get(&stream.url).header(RANGE, format!("bytes={start}-{end}")).send() => {
                    match res {
                        Ok(r) => r,
                        Err(e) => {
                            let safe = redact_error_message(&e.without_url().to_string());
                            return ChunkFetch::Paused(safe);
                        }
                    }
                }
            };

            if resp.status() == reqwest::StatusCode::FORBIDDEN {
                if reresolve_count < 2 {
                    reresolve_count += 1;
                    let fresh = tokio::select! {
                        _ = cancel_rx.changed() => {
                            return ChunkFetch::Aborted(*cancel_rx.borrow());
                        }
                        res = self.provider.resolve_stream(video_id) => res,
                    };
                    match fresh {
                        Ok(s) => {
                            if s.itag != stream.itag
                                || (*total_bytes > 0
                                    && s.content_length > 0
                                    && s.content_length != *total_bytes)
                            {
                                return ChunkFetch::StreamChanged(s);
                            }
                            *stream = s;
                            continue;
                        }
                        Err(e) => {
                            let safe = redact_error_message(&e.to_string());
                            return ChunkFetch::Paused(safe);
                        }
                    }
                }
                return ChunkFetch::Paused("HTTP 403 after stream re-resolution".to_owned());
            }

            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                if rate_limit_count < 4 {
                    let wait_dur = parse_retry_after(resp.headers().get(RETRY_AFTER))
                        .unwrap_or_else(|| {
                            let base_secs = 1u64 << rate_limit_count;
                            let jitter_ms = unix_secs().wrapping_mul(31) % 250;
                            Duration::from_secs(base_secs) + Duration::from_millis(jitter_ms)
                        });
                    rate_limit_count += 1;
                    if !wait_dur.is_zero() {
                        tokio::select! {
                            _ = cancel_rx.changed() => {
                                return ChunkFetch::Aborted(*cancel_rx.borrow());
                            }
                            _ = tokio::time::sleep(wait_dur) => {}
                        }
                    }
                    continue;
                }
                return ChunkFetch::Paused("Rate limited (429)".to_owned());
            }

            if !resp.status().is_success() {
                return ChunkFetch::Failed(format!("HTTP {} on chunk {chunk_idx}", resp.status()));
            }

            if *total_bytes == 0
                && let Some(cr) = resp
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                && let Some((_, tot)) = cr.rsplit_once('/')
                && let Ok(parsed_tot) = tot.parse::<u64>()
            {
                *total_bytes = parsed_tot;
            }

            let bytes = tokio::select! {
                _ = cancel_rx.changed() => {
                    return ChunkFetch::Aborted(*cancel_rx.borrow());
                }
                res = resp.bytes() => {
                    match res {
                        Ok(b) => b.to_vec(),
                        Err(e) => {
                            let safe = redact_error_message(&e.without_url().to_string());
                            return ChunkFetch::Paused(safe);
                        }
                    }
                }
            };

            return ChunkFetch::Data(bytes);
        }
    }

    async fn fetch_artwork_best_effort(&self, track: &Track) -> Option<Vec<u8>> {
        let url = track.artwork_url.as_deref()?;
        if let Some(cached) = self.artwork.get(url, 512) {
            return Some(cached);
        }
        let resp = tokio::time::timeout(Duration::from_secs(5), self.http.get(url).send())
            .await
            .ok()?
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let bytes = tokio::time::timeout(Duration::from_secs(5), resp.bytes())
            .await
            .ok()?
            .ok()?
            .to_vec();
        if bytes.is_empty() || bytes.len() > tagger::MAX_EMBEDDED_ARTWORK_BYTES {
            return None;
        }
        let _ = self.artwork.put(url, 512, bytes.clone());
        Some(bytes)
    }

    fn maybe_report_progress(
        &self,
        record: &mut DownloadRecord,
        last_reported_bytes: &mut u64,
        last_report_time: &mut Instant,
    ) {
        if record.bytes_downloaded.saturating_sub(*last_reported_bytes) >= CHUNK_SIZE
            || last_report_time.elapsed() >= Duration::from_millis(250)
        {
            let now = unix_secs();
            record.updated_at_unix_secs = now;
            let _ = self.offline.update_download_progress(
                &record.video_id,
                DownloadStatus::Downloading,
                record.bytes_downloaded,
                record.total_bytes,
                now,
            );
            self.bus.publish(AppEvent::DownloadUpdated(record.clone()));
            *last_reported_bytes = record.bytes_downloaded;
            *last_report_time = Instant::now();
        }
    }

    async fn handle_abort(&self, record: &DownloadRecord, mode: u8, part_path: Option<&PathBuf>) {
        if mode == MODE_CANCEL {
            if let Some(p) = part_path {
                let _ = fs::remove_file(p);
            } else {
                self.remove_part_file(record);
            }
            self.mark_failed_and_emit(&record.video_id, DownloadStatus::Cancelled, None);
        } else {
            let bytes = part_path
                .and_then(|p| fs::metadata(p).ok().map(|m| m.len()))
                .unwrap_or(record.bytes_downloaded);
            let now = unix_secs();
            let _ = self.offline.update_download_progress(
                &record.video_id,
                DownloadStatus::Paused,
                bytes,
                record.total_bytes,
                now,
            );
            if let Ok(Some(updated)) = self.offline.get_download(&record.video_id) {
                self.bus.publish(AppEvent::DownloadUpdated(updated));
            }
        }
    }

    fn mark_failed_and_emit(&self, video_id: &str, status: DownloadStatus, err: Option<&str>) {
        let now = unix_secs();
        let _ = self
            .offline
            .mark_download_failed(video_id, status, err, now);
        if let Ok(Some(updated)) = self.offline.get_download(video_id) {
            self.bus.publish(AppEvent::DownloadUpdated(updated));
        }
    }
}

enum ChunkFetch {
    Data(Vec<u8>),
    StreamChanged(StreamInfo),
    Aborted(u8),
    Paused(String),
    Failed(String),
}

fn parse_retry_after(header: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let raw = header?.to_str().ok()?.trim();
    let secs = raw.parse::<u64>().ok()?;
    if secs == 0 {
        Some(Duration::ZERO)
    } else {
        Some(Duration::from_secs(secs.clamp(2, 60)))
    }
}

fn extract_video_id_from_temp(filename: &str) -> Option<&str> {
    let without_tmp = filename
        .strip_suffix(".part")
        .or_else(|| filename.strip_suffix(".tmp"))?;
    let (stem, _ext) = without_tmp.rsplit_once('.')?;
    let (video_id, _itag) = stem.rsplit_once('_')?;
    Some(video_id)
}
