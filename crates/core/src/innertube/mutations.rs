//! Authenticated InnerTube write mutations (`/like/*`, `/browse/edit_playlist`, `/atr`)
//! and the non-blocking background [`MutationWorker`] queue.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use resonance_contract::{
    AppEvent, AuthError, CoreError, EventBus, InnerTubeMutations, LikeAction, MutationCommand,
    MutationError, MutationReceipt, PlaylistEditOp, ProviderError, WatchtimeReport,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::innertube::auth::{apply_cookie_rotation, build_auth_headers, redact_secrets};
use crate::innertube::client::InnerTubeClient;
use crate::innertube::clients::WEB_REMIX;
use crate::unix_secs;

pub const MUTATION_QUEUE_CAPACITY: usize = 64;
const MAX_RETRY_ATTEMPTS: usize = 3;
const RETRY_BACKOFF_MS: [u64; MAX_RETRY_ATTEMPTS] = [500, 1500, 3000];

fn redact_and_cap(raw: &str) -> String {
    redact_secrets(raw).chars().take(200).collect()
}

fn validate_innertube_id(id: &str, field: &str) -> Result<(), MutationError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(MutationError::InvalidInput(format!(
            "invalid {field}: must match [A-Za-z0-9_-]{{1,128}}"
        )));
    }
    Ok(())
}

fn validate_cpn(cpn: &str) -> Result<(), MutationError> {
    if cpn.len() != 16
        || !cpn
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(MutationError::InvalidInput(
            "invalid cpn: must be 16 base64url characters".to_owned(),
        ));
    }
    Ok(())
}

fn map_core_error(err: CoreError) -> MutationError {
    match err {
        CoreError::Auth(AuthError::NotSignedIn) => MutationError::NotAuthenticated,
        CoreError::Auth(AuthError::Revoked) => MutationError::Http {
            status: 401,
            message: "session revoked".to_owned(),
        },
        CoreError::Auth(other) => MutationError::Network(redact_and_cap(&other.to_string())),
        CoreError::Provider(ProviderError::RateLimited { retry_after_secs }) => {
            MutationError::RateLimited { retry_after_secs }
        }
        CoreError::Provider(ProviderError::Http { status, message }) => MutationError::Http {
            status,
            message: redact_and_cap(&message),
        },
        CoreError::Provider(ProviderError::Network(msg)) => {
            MutationError::Network(redact_and_cap(&msg))
        }
        other => MutationError::Network(redact_and_cap(&other.to_string())),
    }
}

fn make_receipt(endpoint: &str, resp: Option<&Value>) -> MutationReceipt {
    let now = unix_secs();
    let server_status = resp
        .and_then(|v| v.get("status").and_then(Value::as_str))
        .map(str::to_owned);
    MutationReceipt {
        idempotency_key: format!("{endpoint}:{now}:{}", rand::random::<u32>()),
        endpoint: endpoint.to_owned(),
        server_status,
        executed_at_unix_secs: now,
    }
}

#[async_trait]
impl InnerTubeMutations for InnerTubeClient {
    async fn report_atr(&self, report: &WatchtimeReport) -> Result<MutationReceipt, MutationError> {
        let Some(session) = self.current_session() else {
            return Err(MutationError::NotAuthenticated);
        };
        validate_innertube_id(&report.video_id, "video_id")?;
        validate_cpn(&report.cpn)?;
        if report.end_secs < report.start_secs {
            return Err(MutationError::InvalidInput(
                "end_secs must be >= start_secs".to_owned(),
            ));
        }

        let base = self.web_remix_base_url();
        let atr_url = if base != WEB_REMIX.api_base {
            format!("{}/atr", base.trim_end_matches('/'))
        } else {
            format!("{}/api/stats/atr", WEB_REMIX.origin)
        };

        let st = report.start_secs.to_string();
        let et = report.end_secs.to_string();
        let len = report.duration_secs.to_string();
        let url = url::Url::parse_with_params(
            &atr_url,
            [
                ("docid", report.video_id.as_str()),
                ("cpn", report.cpn.as_str()),
                ("ver", "2"),
                ("st", st.as_str()),
                ("et", et.as_str()),
                ("len", len.as_str()),
                ("el", "detailpage"),
                ("ns", "yt"),
                ("c", WEB_REMIX.client_name),
                ("cver", WEB_REMIX.client_version),
            ],
        )
        .map_err(|e| MutationError::InvalidInput(format!("invalid atr url: {e}")))?;

        let auth_headers = build_auth_headers(&WEB_REMIX, &session, unix_secs())
            .map_err(|e| MutationError::Network(redact_and_cap(&e.to_string())))?;

        let mut req = self
            .http_client()
            .get(url)
            .header("User-Agent", WEB_REMIX.user_agent)
            .header("Origin", WEB_REMIX.origin);
        for (k, v) in auth_headers {
            req = req.header(k, v);
        }
        if let Some(vd) = self.current_visitor_data().filter(|s| !s.is_empty()) {
            req = req.header("X-Goog-Visitor-Id", vd);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| MutationError::Network(redact_and_cap(&e.to_string())))?;

        let status = resp.status().as_u16();
        let set_cookies: Vec<String> = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|h| h.to_str().ok().map(str::to_owned))
            .collect();
        if !set_cookies.is_empty()
            && let Some(updated) =
                apply_cookie_rotation(&session, set_cookies.iter().map(String::as_str))
        {
            let _ = self.sign_in(updated);
        }

        if status == 401 {
            let _ = self.sign_out();
            return Err(MutationError::Http {
                status: 401,
                message: "session revoked".to_owned(),
            });
        }
        if status == 429 {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            return Err(MutationError::RateLimited {
                retry_after_secs: retry_after,
            });
        }
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(MutationError::Http {
                status,
                message: redact_and_cap(&text),
            });
        }

        Ok(make_receipt("atr", None))
    }

    async fn set_like(
        &self,
        video_id: &str,
        action: LikeAction,
    ) -> Result<MutationReceipt, MutationError> {
        if self.current_session().is_none() {
            return Err(MutationError::NotAuthenticated);
        }
        validate_innertube_id(video_id, "video_id")?;

        let endpoint = match action {
            LikeAction::Like => "like/like",
            LikeAction::Dislike => "like/dislike",
            LikeAction::RemoveLike => "like/removelike",
        };
        let payload = json!({
            "target": {
                "videoId": video_id,
            }
        });
        let resp = self
            .post_authenticated_web_remix(endpoint, payload)
            .await
            .map_err(map_core_error)?;
        self.evict_swr_cache("collection:FEmusic_liked_videos");
        Ok(make_receipt(endpoint, Some(&resp)))
    }

    async fn edit_playlist(
        &self,
        playlist_id: &str,
        op: &PlaylistEditOp,
    ) -> Result<MutationReceipt, MutationError> {
        if self.current_session().is_none() {
            return Err(MutationError::NotAuthenticated);
        }
        validate_innertube_id(playlist_id, "playlist_id")?;

        let action_json = match op {
            PlaylistEditOp::AddVideo { video_id } => {
                validate_innertube_id(video_id, "video_id")?;
                json!({
                    "action": "ACTION_ADD_VIDEO",
                    "addedVideoId": video_id,
                })
            }
            PlaylistEditOp::RemoveVideo {
                video_id,
                set_video_id,
            } => {
                validate_innertube_id(video_id, "video_id")?;
                if set_video_id.trim().is_empty() {
                    return Err(MutationError::InvalidInput(
                        "set_video_id cannot be empty".to_owned(),
                    ));
                }
                json!({
                    "action": "ACTION_REMOVE_VIDEO",
                    "removedVideoId": video_id,
                    "setVideoId": set_video_id,
                })
            }
            PlaylistEditOp::MoveVideoBefore {
                set_video_id,
                predecessor_set_video_id,
            } => {
                if set_video_id.trim().is_empty() {
                    return Err(MutationError::InvalidInput(
                        "set_video_id cannot be empty".to_owned(),
                    ));
                }
                let mut obj = serde_json::Map::new();
                obj.insert("action".into(), json!("ACTION_MOVE_VIDEO_BEFORE"));
                obj.insert("setVideoId".into(), json!(set_video_id));
                if let Some(pred) = predecessor_set_video_id {
                    obj.insert("movedSetVideoIdPredecessor".into(), json!(pred));
                }
                Value::Object(obj)
            }
        };

        let payload = json!({
            "playlistId": playlist_id,
            "actions": [action_json],
        });
        let resp = self
            .post_authenticated_web_remix("browse/edit_playlist", payload)
            .await
            .map_err(map_core_error)?;
        self.evict_swr_cache(&format!("collection:{playlist_id}"));
        Ok(make_receipt("browse/edit_playlist", Some(&resp)))
    }
}

/// Non-blocking bounded FIFO mutation queue with `/atr` coalescing,
/// exponential 5xx/429 backoff, and immediate `401` session revocation + drain.
#[derive(Clone)]
pub struct MutationWorker {
    tx: mpsc::Sender<MutationCommand>,
}

impl MutationWorker {
    pub fn spawn(client: Arc<InnerTubeClient>, bus: EventBus) -> Self {
        let (tx, rx) = mpsc::channel::<MutationCommand>(MUTATION_QUEUE_CAPACITY);
        tokio::spawn(run_worker(client, bus, rx));
        Self { tx }
    }

    pub fn enqueue(&self, cmd: MutationCommand) -> Result<(), MutationError> {
        self.tx.try_send(cmd).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => MutationError::QueueFull {
                capacity: MUTATION_QUEUE_CAPACITY,
            },
            mpsc::error::TrySendError::Closed(_) => {
                MutationError::Network("mutation worker closed".to_owned())
            }
        })
    }
}

fn push_with_atr_coalescing(queue: &mut VecDeque<MutationCommand>, cmd: MutationCommand) {
    if let MutationCommand::ReportAtr(ref incoming) = cmd {
        for existing in queue.iter_mut() {
            if let MutationCommand::ReportAtr(prev) = existing
                && prev.video_id == incoming.video_id
                && prev.cpn == incoming.cpn
            {
                prev.end_secs = prev.end_secs.max(incoming.end_secs);
                prev.duration_secs = incoming.duration_secs;
                prev.is_final |= incoming.is_final;
                return;
            }
        }
    }
    queue.push_back(cmd);
}

async fn execute_once(
    client: &InnerTubeClient,
    cmd: &MutationCommand,
) -> Result<MutationReceipt, MutationError> {
    match cmd {
        MutationCommand::ReportAtr(report) => client.report_atr(report).await,
        MutationCommand::SetLike { video_id, action } => {
            client.set_like(video_id, *action).await
        }
        MutationCommand::EditPlaylist { playlist_id, op } => {
            client.edit_playlist(playlist_id, op).await
        }
    }
}

async fn execute_with_retries(
    client: &InnerTubeClient,
    cmd: &MutationCommand,
) -> Result<MutationReceipt, MutationError> {
    let mut attempt = 0usize;
    loop {
        match execute_once(client, cmd).await {
            Ok(receipt) => return Ok(receipt),
            Err(MutationError::RateLimited { retry_after_secs })
                if attempt < MAX_RETRY_ATTEMPTS =>
            {
                let delay_secs = retry_after_secs.unwrap_or(2).clamp(2, 30);
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            }
            Err(MutationError::Http { status, .. })
                if status >= 500 && attempt < MAX_RETRY_ATTEMPTS =>
            {
                let delay_ms = RETRY_BACKOFF_MS[attempt];
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            Err(MutationError::Network(_)) if attempt < MAX_RETRY_ATTEMPTS => {
                let delay_ms = RETRY_BACKOFF_MS[attempt];
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

async fn run_worker(
    client: Arc<InnerTubeClient>,
    bus: EventBus,
    mut rx: mpsc::Receiver<MutationCommand>,
) {
    let mut pending: VecDeque<MutationCommand> = VecDeque::new();
    while let Some(first) = rx.recv().await {
        push_with_atr_coalescing(&mut pending, first);
        while let Ok(more) = rx.try_recv() {
            push_with_atr_coalescing(&mut pending, more);
        }

        while let Some(cmd) = pending.pop_front() {
            // Drain any newly arrived commands so ATR pings that arrived during
            // an in-flight request can also coalesce cleanly.
            while let Ok(more) = rx.try_recv() {
                push_with_atr_coalescing(&mut pending, more);
            }

            match execute_with_retries(&client, &cmd).await {
                Ok(receipt) => {
                    bus.publish(AppEvent::MutationCompleted(receipt));
                }
                Err(err @ (MutationError::Http { status: 401, .. } | MutationError::NotAuthenticated)) => {
                    let _ = client.sign_out();
                    bus.publish(AppEvent::AuthStateChanged {
                        signed_in: false,
                        account_name: None,
                    });
                    bus.publish(AppEvent::MutationFailed {
                        command: cmd,
                        reason: redact_and_cap(&err.to_string()),
                    });
                    // Drain all remaining queued mutations to prevent retry storms with revoked cookies.
                    pending.clear();
                    while rx.try_recv().is_ok() {}
                    break;
                }
                Err(err) => {
                    bus.publish(AppEvent::MutationFailed {
                        command: cmd,
                        reason: redact_and_cap(&err.to_string()),
                    });
                }
            }
        }
    }
}
