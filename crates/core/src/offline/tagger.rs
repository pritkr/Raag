//! `lofty` metadata, FrontCover artwork, and synced LRC lyrics tagger.

use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::{FileType, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};
use resonance_contract::{
    CoreError, CoreResult, LyricSyncKind, LyricsDocument, OfflineStorageError, Track,
};

pub const MAX_EMBEDDED_ARTWORK_BYTES: usize = 4 * 1024 * 1024; // 4 MB

fn mime_from_magic(bytes: &[u8]) -> Option<MimeType> {
    if bytes.is_empty() || bytes.len() > MAX_EMBEDDED_ARTWORK_BYTES {
        return None;
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(MimeType::Jpeg)
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(MimeType::Png)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(MimeType::Unknown("image/webp".to_owned()))
    } else {
        None
    }
}

fn format_lyrics_for_tag(doc: &LyricsDocument) -> String {
    if doc.sync_kind != LyricSyncKind::Plain {
        doc.lines
            .iter()
            .map(|line| {
                let min = line.start_ms / 60_000;
                let sec = (line.start_ms % 60_000) / 1_000;
                let cs = (line.start_ms % 1_000) / 10;
                format!("[{min:02}:{sec:02}.{cs:02}]{}", line.text)
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        doc.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Embeds title, artist, album, lyrics, and FrontCover artwork into `part_path` using `lofty`.
///
/// Returns `Ok(true)` if tags were written and verified, `Ok(false)` if the container
/// (such as Matroska/WebM) is not tag-writable or tagging was safely skipped without
/// mutating audio frames.
pub fn embed_metadata(
    part_path: &Path,
    track: &Track,
    artwork: Option<&[u8]>,
    lyrics: Option<&LyricsDocument>,
) -> CoreResult<bool> {
    let probe = match Probe::open(part_path) {
        Ok(p) => match p.guess_file_type() {
            Ok(guessed) => guessed,
            Err(e) => {
                log::warn!("offline tagger: guess_file_type failed ({e}); skipping tags");
                return Ok(false);
            }
        },
        Err(e) => {
            log::warn!("offline tagger: probe failed ({e}); skipping tags");
            return Ok(false);
        }
    };

    let mut tagged_file = match probe.read() {
        Ok(f) => f,
        Err(e) => {
            log::warn!("offline tagger: read failed ({e}); skipping tags");
            return Ok(false);
        }
    };

    let tag_type = match tagged_file.file_type() {
        FileType::Mp4 => TagType::Mp4Ilst,
        FileType::Opus | FileType::Flac | FileType::Mpeg => tagged_file.primary_tag_type(),
        _ => {
            // Raw WebM/Matroska Opus or unsupported containers are not tag-writable via lofty;
            // metadata & lyrics remain available in SQLite without risking audio corruption.
            return Ok(false);
        }
    };

    if tagged_file.tag(tag_type).is_none() {
        tagged_file.insert_tag(Tag::new(tag_type));
    }
    let Some(tag) = tagged_file.tag_mut(tag_type) else {
        return Ok(false);
    };

    tag.set_title(track.title.clone());
    tag.set_artist(track.artist_display());
    if let Some(album) = &track.album
        && !album.title.is_empty()
    {
        tag.set_album(album.title.clone());
    }

    if let Some(doc) = lyrics
        && !doc.lines.is_empty()
    {
        let formatted = format_lyrics_for_tag(doc);
        if !formatted.is_empty() {
            tag.insert_text(ItemKey::Lyrics, formatted);
        }
    }

    if let Some(bytes) = artwork
        && let Some(mime) = mime_from_magic(bytes)
    {
        let pic = Picture::new_unchecked(PictureType::CoverFront, Some(mime), None, bytes.to_vec());
        tag.push_picture(pic);
    }

    if let Err(e) = tag.save_to_path(part_path, WriteOptions::default()) {
        log::warn!("offline tagger: save_to_path failed ({e})");
        let len = std::fs::metadata(part_path).map(|m| m.len()).unwrap_or(0);
        if len == 0 {
            return Err(CoreError::OfflineStorage(OfflineStorageError::Tagging(
                format!("file truncated during tagging: {e}"),
            )));
        }
        return Ok(false);
    }

    let post_len = std::fs::metadata(part_path)
        .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string())))?
        .len();
    if post_len == 0 {
        return Err(CoreError::OfflineStorage(OfflineStorageError::Tagging(
            "tagged file has zero length".to_owned(),
        )));
    }

    let post_probe = Probe::open(part_path).map_err(|e| {
        CoreError::OfflineStorage(OfflineStorageError::Tagging(format!(
            "post-tag verification failed: {e}"
        )))
    })?;
    post_probe.guess_file_type().map_err(|e| {
        CoreError::OfflineStorage(OfflineStorageError::Tagging(format!(
            "post-tag file type verification failed: {e}"
        )))
    })?;

    Ok(true)
}
