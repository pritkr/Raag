//! Android Media Lifecycle, Keystore & Sign-In Bidirectional JNI Bridge (`ResonanceNativeBridge.kt`).
//!
//! Connects Rust `AudioBackend` & `AuthStore` to Kotlin Media3/ExoPlayer (`PlaybackForegroundService.kt`),
//! `KeystoreBridge.kt` (`AndroidKeyStore` AES-256-GCM), and `MainActivity.kt` (`; wv`-stripped WebView).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(target_os = "android")]
use std::sync::OnceLock;

use async_trait::async_trait;
use resonance_contract::{
    AudioBackend, AudioTelemetry, AuthError, AuthSession, AuthStore, CoreError, CoreResult,
    CredentialBackendKind, MediaCommand, NowPlayingMetadata, PlaybackError, PlayerState,
};
use resonance_core::innertube::FileAuthStore;
use resonance_core::innertube::auth::{extract_cookie, redact_secrets};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub const ANDROID_KEYSTORE_KEY: &str = "ytmusic_auth_session_v1";

// Inbound MediaCommand opcodes — must stay 1:1 with `ResonanceNativeBridge.kt`
pub const CMD_PLAY: i32 = 1;
pub const CMD_PAUSE: i32 = 2;
pub const CMD_TOGGLE_PLAY_PAUSE: i32 = 3;
pub const CMD_NEXT: i32 = 4;
pub const CMD_PREVIOUS: i32 = 5;
pub const CMD_SEEK_MS: i32 = 6;
pub const CMD_BECOMING_NOISY: i32 = 7;
pub const CMD_AUDIO_FOCUS_LOST_TRANSIENT: i32 = 8;
pub const CMD_AUDIO_FOCUS_GAINED: i32 = 9;
pub const CMD_STOP: i32 = 10;

// Inbound ExoPlayer state codes — must stay 1:1 with `ResonanceNativeBridge.kt`
pub const STATE_ERROR: i32 = -1;
pub const STATE_IDLE: i32 = 1;
pub const STATE_BUFFERING: i32 = 2;
pub const STATE_READY: i32 = 3;
pub const STATE_ENDED: i32 = 4;
pub const STATE_PLAYING: i32 = 5;
pub const STATE_PAUSED: i32 = 6;

/// Ensures `<data_dir>/downloads` exists and contains an empty `.nomedia` sentinel
/// so Android `MediaStore` does not index internal app downloads.
pub fn ensure_downloads_dir(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = data_dir.join("downloads");
    std::fs::create_dir_all(&dir)?;
    let nomedia = dir.join(".nomedia");
    if !nomedia.exists() {
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&nomedia);
    }
    Ok(dir)
}

/// Returns `true` only for `http://127.0.0.1[:port]/...` or `http://localhost[:port]/...`.
pub fn is_loopback_http_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return false;
    }
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if let Some(p) = port
        && (p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    host == "127.0.0.1" || host == "localhost"
}

/// Recorded outbound bridge calls for deterministic offline verification.
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeCall {
    LoadAndPlay {
        url: String,
        title: String,
        artist: String,
        duration_ms: u64,
        generation: u64,
    },
    Play,
    Pause,
    Stop,
    SeekMs(u64),
    SetVolume(f32),
    SetPlaybackSpeed {
        speed: f32,
        preserve_pitch: bool,
    },
    UpdateMetadata {
        title: String,
        playing: bool,
        position_ms: u64,
    },
    KeystoreWrite {
        key: String,
    },
    KeystoreRead {
        key: String,
    },
    KeystoreDelete {
        key: String,
    },
    LaunchSignInWebView,
}

/// Telemetry & auth callbacks delivered from Kotlin `ResonanceNativeBridge` into Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AndroidTelemetryEvent {
    Progress {
        generation: u64,
        position_ms: u64,
        duration_ms: u64,
    },
    PlaybackState {
        generation: u64,
        state_code: i32,
        error_msg: Option<String>,
    },
    AuthCookiesCaptured {
        cookie_header: String,
    },
    AuthWebViewCancelled,
}

/// Testable seam abstracting outbound Rust -> Kotlin JNI calls.
pub trait AndroidBridgeInvoker: Send + Sync {
    fn audio_load_and_play(
        &self,
        url: &str,
        meta: &NowPlayingMetadata,
        generation: u64,
    ) -> CoreResult<()>;
    fn audio_play(&self) -> CoreResult<()>;
    fn audio_pause(&self) -> CoreResult<()>;
    fn audio_stop(&self) -> CoreResult<()>;
    fn audio_seek_ms(&self, position_ms: u64) -> CoreResult<()>;
    fn audio_set_volume(&self, volume: f32) -> CoreResult<()>;
    fn audio_set_playback_speed(&self, speed: f32, preserve_pitch: bool) -> CoreResult<()>;
    fn audio_update_metadata(&self, meta: &NowPlayingMetadata, playing: bool, position_ms: u64);

    fn keystore_write(&self, key: &str, plaintext: &str) -> Result<bool, String>;
    fn keystore_read(&self, key: &str) -> Result<Option<String>, String>;
    fn keystore_delete(&self, key: &str) -> Result<(), String>;

    fn launch_sign_in_webview(&self) -> CoreResult<()>;
}

/// Deterministic in-memory `AndroidBridgeInvoker` used on desktop CI / `cargo test`.
#[derive(Default)]
pub struct MockAndroidBridgeInvoker {
    calls: Mutex<Vec<BridgeCall>>,
    keystore: Mutex<HashMap<String, String>>,
    fail_keystore_write: AtomicBool,
    fail_keystore_read: AtomicBool,
    fail_keystore_delete: AtomicBool,
}

impl MockAndroidBridgeInvoker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_keystore_write_failure(&self, fail: bool) {
        self.fail_keystore_write.store(fail, Ordering::SeqCst);
    }

    pub fn set_keystore_read_failure(&self, fail: bool) {
        self.fail_keystore_read.store(fail, Ordering::SeqCst);
    }

    pub fn set_keystore_delete_failure(&self, fail: bool) {
        self.fail_keystore_delete.store(fail, Ordering::SeqCst);
    }

    pub fn calls(&self) -> Vec<BridgeCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn keystore_snapshot(&self) -> HashMap<String, String> {
        self.keystore.lock().unwrap().clone()
    }
}

impl AndroidBridgeInvoker for MockAndroidBridgeInvoker {
    fn audio_load_and_play(
        &self,
        url: &str,
        meta: &NowPlayingMetadata,
        generation: u64,
    ) -> CoreResult<()> {
        self.calls.lock().unwrap().push(BridgeCall::LoadAndPlay {
            url: url.to_owned(),
            title: meta.title.clone(),
            artist: meta.artist.clone(),
            duration_ms: meta.duration_ms,
            generation,
        });
        Ok(())
    }

    fn audio_play(&self) -> CoreResult<()> {
        self.calls.lock().unwrap().push(BridgeCall::Play);
        Ok(())
    }

    fn audio_pause(&self) -> CoreResult<()> {
        self.calls.lock().unwrap().push(BridgeCall::Pause);
        Ok(())
    }

    fn audio_stop(&self) -> CoreResult<()> {
        self.calls.lock().unwrap().push(BridgeCall::Stop);
        Ok(())
    }

    fn audio_seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push(BridgeCall::SeekMs(position_ms));
        Ok(())
    }

    fn audio_set_volume(&self, volume: f32) -> CoreResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push(BridgeCall::SetVolume(volume));
        Ok(())
    }

    fn audio_set_playback_speed(&self, speed: f32, preserve_pitch: bool) -> CoreResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push(BridgeCall::SetPlaybackSpeed {
                speed,
                preserve_pitch,
            });
        Ok(())
    }

    fn audio_update_metadata(&self, meta: &NowPlayingMetadata, playing: bool, position_ms: u64) {
        self.calls.lock().unwrap().push(BridgeCall::UpdateMetadata {
            title: meta.title.clone(),
            playing,
            position_ms,
        });
    }

    fn keystore_write(&self, key: &str, plaintext: &str) -> Result<bool, String> {
        self.calls.lock().unwrap().push(BridgeCall::KeystoreWrite {
            key: key.to_owned(),
        });
        if self.fail_keystore_write.load(Ordering::SeqCst) {
            return Err("simulated hardware keystore write failure".to_owned());
        }
        self.keystore
            .lock()
            .unwrap()
            .insert(key.to_owned(), plaintext.to_owned());
        Ok(true)
    }

    fn keystore_read(&self, key: &str) -> Result<Option<String>, String> {
        self.calls.lock().unwrap().push(BridgeCall::KeystoreRead {
            key: key.to_owned(),
        });
        if self.fail_keystore_read.load(Ordering::SeqCst) {
            return Err("simulated hardware keystore read failure".to_owned());
        }
        Ok(self.keystore.lock().unwrap().get(key).cloned())
    }

    fn keystore_delete(&self, key: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(BridgeCall::KeystoreDelete {
            key: key.to_owned(),
        });
        if self.fail_keystore_delete.load(Ordering::SeqCst) {
            return Err("simulated hardware keystore delete failure".to_owned());
        }
        self.keystore.lock().unwrap().remove(key);
        Ok(())
    }

    fn launch_sign_in_webview(&self) -> CoreResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push(BridgeCall::LaunchSignInWebView);
        Ok(())
    }
}

#[cfg(target_os = "android")]
struct AndroidJniHost {
    vm: jni::JavaVM,
    bridge_class: jni::objects::GlobalRef,
    _files_dir: String,
    _cache_dir: String,
}

#[cfg(target_os = "android")]
static ANDROID_JNI_HOST: OnceLock<AndroidJniHost> = OnceLock::new();

#[cfg(target_os = "android")]
pub struct JniBridgeInvoker;

#[cfg(target_os = "android")]
impl JniBridgeInvoker {
    pub fn new() -> Self {
        Self
    }

    fn with_env<F, R>(&self, f: F) -> Result<R, String>
    where
        F: FnOnce(&mut jni::JNIEnv<'_>, &jni::objects::JClass<'_>) -> Result<R, String>,
    {
        let host = ANDROID_JNI_HOST
            .get()
            .ok_or_else(|| "AndroidJniHost not initialized".to_owned())?;
        let mut env = host
            .vm
            .attach_current_thread()
            .map_err(|e| redact_secrets(&format!("JNI attach_current_thread failed: {e}")))?;
        let cls = <&jni::objects::JClass>::from(host.bridge_class.as_obj());
        let result = env
            .with_local_frame(32, |frame_env| -> jni::errors::Result<Result<R, String>> {
                Ok(f(frame_env, cls))
            })
            .map_err(|e| redact_secrets(&format!("JNI local frame failed: {e}")))?;
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
            return Err("Kotlin JNI call threw an exception".to_owned());
        }
        result
    }
}

#[cfg(target_os = "android")]
impl AndroidBridgeInvoker for JniBridgeInvoker {
    fn audio_load_and_play(
        &self,
        url: &str,
        meta: &NowPlayingMetadata,
        generation: u64,
    ) -> CoreResult<()> {
        self.with_env(|env, cls| {
            let j_url = env
                .new_string(url)
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let j_title = env
                .new_string(&meta.title)
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let j_artist = env
                .new_string(&meta.artist)
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let j_album = env
                .new_string(meta.album.as_deref().unwrap_or(""))
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let j_artwork = env
                .new_string(meta.artwork_url.as_deref().unwrap_or(""))
                .map_err(|e| redact_secrets(&e.to_string()))?;
            env.call_static_method(
                cls,
                "audioLoadAndPlay",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JJ)V",
                &[
                    jni::objects::JValue::Object(&j_url),
                    jni::objects::JValue::Object(&j_title),
                    jni::objects::JValue::Object(&j_artist),
                    jni::objects::JValue::Object(&j_album),
                    jni::objects::JValue::Object(&j_artwork),
                    jni::objects::JValue::Long(meta.duration_ms as i64),
                    jni::objects::JValue::Long(generation as i64),
                ],
            )
            .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        })
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(redact_secrets(&e))))
    }

    fn audio_play(&self) -> CoreResult<()> {
        self.with_env(|env, cls| {
            env.call_static_method(cls, "audioPlay", "()V", &[])
                .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        })
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(redact_secrets(&e))))
    }

    fn audio_pause(&self) -> CoreResult<()> {
        self.with_env(|env, cls| {
            env.call_static_method(cls, "audioPause", "()V", &[])
                .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        })
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(redact_secrets(&e))))
    }

    fn audio_stop(&self) -> CoreResult<()> {
        self.with_env(|env, cls| {
            env.call_static_method(cls, "audioStop", "()V", &[])
                .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        })
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(redact_secrets(&e))))
    }

    fn audio_seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        self.with_env(|env, cls| {
            env.call_static_method(
                cls,
                "audioSeekMs",
                "(J)V",
                &[jni::objects::JValue::Long(position_ms as i64)],
            )
            .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        })
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(redact_secrets(&e))))
    }

    fn audio_set_volume(&self, volume: f32) -> CoreResult<()> {
        self.with_env(|env, cls| {
            env.call_static_method(
                cls,
                "audioSetVolume",
                "(F)V",
                &[jni::objects::JValue::Float(volume)],
            )
            .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        })
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(redact_secrets(&e))))
    }

    fn audio_set_playback_speed(&self, speed: f32, preserve_pitch: bool) -> CoreResult<()> {
        self.with_env(|env, cls| {
            env.call_static_method(
                cls,
                "audioSetPlaybackSpeed",
                "(FZ)V",
                &[
                    jni::objects::JValue::Float(speed),
                    jni::objects::JValue::Bool(u8::from(preserve_pitch)),
                ],
            )
            .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        })
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(redact_secrets(&e))))
    }

    fn audio_update_metadata(&self, meta: &NowPlayingMetadata, playing: bool, position_ms: u64) {
        let _ = self.with_env(|env, cls| {
            let j_title = env
                .new_string(&meta.title)
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let j_artist = env
                .new_string(&meta.artist)
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let j_album = env
                .new_string(meta.album.as_deref().unwrap_or(""))
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let j_artwork = env
                .new_string(meta.artwork_url.as_deref().unwrap_or(""))
                .map_err(|e| redact_secrets(&e.to_string()))?;
            env.call_static_method(
                cls,
                "audioUpdateMetadata",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ZJ)V",
                &[
                    jni::objects::JValue::Object(&j_title),
                    jni::objects::JValue::Object(&j_artist),
                    jni::objects::JValue::Object(&j_album),
                    jni::objects::JValue::Object(&j_artwork),
                    jni::objects::JValue::Bool(u8::from(playing)),
                    jni::objects::JValue::Long(position_ms as i64),
                ],
            )
            .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        });
    }

    fn keystore_write(&self, key: &str, plaintext: &str) -> Result<bool, String> {
        self.with_env(|env, cls| {
            let j_key = env
                .new_string(key)
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let j_val = env
                .new_string(plaintext)
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let res = env
                .call_static_method(
                    cls,
                    "keystoreWrite",
                    "(Ljava/lang/String;Ljava/lang/String;)Z",
                    &[
                        jni::objects::JValue::Object(&j_key),
                        jni::objects::JValue::Object(&j_val),
                    ],
                )
                .map_err(|e| redact_secrets(&e.to_string()))?;
            res.z().map_err(|e| redact_secrets(&e.to_string()))
        })
    }

    fn keystore_read(&self, key: &str) -> Result<Option<String>, String> {
        self.with_env(|env, cls| {
            let j_key = env
                .new_string(key)
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let res = env
                .call_static_method(
                    cls,
                    "keystoreRead",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[jni::objects::JValue::Object(&j_key)],
                )
                .map_err(|e| redact_secrets(&e.to_string()))?;
            let obj = res.l().map_err(|e| redact_secrets(&e.to_string()))?;
            if obj.is_null() {
                return Ok(None);
            }
            let jstr = jni::objects::JString::from(obj);
            let rust_str: String = env
                .get_string(&jstr)
                .map_err(|e| redact_secrets(&e.to_string()))?
                .into();
            Ok(Some(rust_str))
        })
    }

    fn keystore_delete(&self, key: &str) -> Result<(), String> {
        self.with_env(|env, cls| {
            let j_key = env
                .new_string(key)
                .map_err(|e| redact_secrets(&e.to_string()))?;
            env.call_static_method(
                cls,
                "keystoreDelete",
                "(Ljava/lang/String;)V",
                &[jni::objects::JValue::Object(&j_key)],
            )
            .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        })
    }

    fn launch_sign_in_webview(&self) -> CoreResult<()> {
        self.with_env(|env, cls| {
            env.call_static_method(cls, "launchSignInWebView", "()V", &[])
                .map_err(|e| redact_secrets(&e.to_string()))?;
            Ok(())
        })
        .map_err(|e| CoreError::Auth(AuthError::Keystore(redact_secrets(&e))))
    }
}

fn default_invoker() -> Arc<dyn AndroidBridgeInvoker> {
    #[cfg(target_os = "android")]
    {
        Arc::new(JniBridgeInvoker::new())
    }
    #[cfg(not(target_os = "android"))]
    {
        Arc::new(MockAndroidBridgeInvoker::new())
    }
}

static ANDROID_CMD_TX: Mutex<Option<UnboundedSender<MediaCommand>>> = Mutex::new(None);
static ANDROID_TELEMETRY_TX: Mutex<Option<UnboundedSender<AudioTelemetry>>> = Mutex::new(None);
static ANDROID_AUTH_COOKIE_TX: Mutex<Option<UnboundedSender<String>>> = Mutex::new(None);
static ANDROID_ACTIVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static ANDROID_REPORTED_POSITION_MS: AtomicU64 = AtomicU64::new(0);
static ANDROID_REPORTED_DURATION_MS: AtomicU64 = AtomicU64::new(0);
static ANDROID_REPORTED_PLAYING: AtomicBool = AtomicBool::new(false);

/// Decodes an ExoPlayer state code from Kotlin `PlaybackForegroundService` into
/// a platform-agnostic [`AudioTelemetry`] event.
pub fn decode_android_playback_state(
    generation: u64,
    state_code: i32,
    error_msg: Option<String>,
) -> Option<AudioTelemetry> {
    match state_code {
        2 => Some(AudioTelemetry::Buffering {
            generation,
            buffering: true,
        }),
        3 => Some(AudioTelemetry::Buffering {
            generation,
            buffering: false,
        }),
        4 => Some(AudioTelemetry::Ended { generation }),
        5 => Some(AudioTelemetry::StateChanged {
            generation,
            state: PlayerState::Playing,
        }),
        6 => Some(AudioTelemetry::StateChanged {
            generation,
            state: PlayerState::Paused,
        }),
        -1 => Some(AudioTelemetry::Failed {
            generation,
            message: error_msg.unwrap_or_else(|| "Android Media3 playback error".to_owned()),
        }),
        _ => None,
    }
}

/// Registers and returns a fresh receiver for WebView-captured YouTube Music sign-in cookies.
pub fn subscribe_android_auth_cookies() -> UnboundedReceiver<String> {
    let (tx, rx) = unbounded_channel();
    *ANDROID_AUTH_COOKIE_TX.lock().unwrap() = Some(tx);
    rx
}

pub struct AndroidMediaBridge {
    invoker: Arc<dyn AndroidBridgeInvoker>,
    generation: AtomicU64,
    playing: AtomicBool,
    position_ms: AtomicU64,
    active_url: Mutex<Option<String>>,
    cmd_rx: Mutex<Option<UnboundedReceiver<MediaCommand>>>,
    telemetry_rx: Mutex<Option<UnboundedReceiver<AudioTelemetry>>>,
    auth_cookie_rx: Mutex<Option<UnboundedReceiver<String>>>,
}

impl AndroidMediaBridge {
    pub fn new() -> Self {
        Self::with_invoker(default_invoker())
    }

    pub fn with_invoker(invoker: Arc<dyn AndroidBridgeInvoker>) -> Self {
        let (tx, rx) = unbounded_channel();
        *ANDROID_CMD_TX.lock().unwrap() = Some(tx);
        let (tel_tx, tel_rx) = unbounded_channel();
        *ANDROID_TELEMETRY_TX.lock().unwrap() = Some(tel_tx);
        let auth_rx = subscribe_android_auth_cookies();
        Self {
            invoker,
            generation: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            position_ms: AtomicU64::new(0),
            active_url: Mutex::new(None),
            cmd_rx: Mutex::new(Some(rx)),
            telemetry_rx: Mutex::new(Some(tel_rx)),
            auth_cookie_rx: Mutex::new(Some(auth_rx)),
        }
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn take_auth_cookie_receiver(&self) -> Option<UnboundedReceiver<String>> {
        self.auth_cookie_rx.lock().unwrap().take()
    }

    /// Invoked from Kotlin `ResonanceNativeBridge` (MediaSession / BecomingNoisy / AudioFocus).
    pub fn dispatch_from_jni(cmd: MediaCommand) {
        if let Some(tx) = ANDROID_CMD_TX.lock().unwrap().as_ref() {
            let _ = tx.send(cmd);
        }
    }

    pub fn dispatch_audio_telemetry_from_jni(event: AudioTelemetry) {
        if let Some(tx) = ANDROID_TELEMETRY_TX.lock().unwrap().as_ref() {
            let _ = tx.send(event);
        }
    }

    pub fn dispatch_auth_cookies_from_jni(cookie_header: String) {
        if let Some(tx) = ANDROID_AUTH_COOKIE_TX.lock().unwrap().as_ref() {
            let _ = tx.send(cookie_header);
        }
    }

    pub fn dispatch_telemetry_from_jni(event: AndroidTelemetryEvent) {
        match event {
            AndroidTelemetryEvent::Progress {
                generation,
                position_ms,
                duration_ms,
            } => Self::dispatch_audio_telemetry_from_jni(AudioTelemetry::Progress {
                generation,
                position_ms,
                duration_ms: (duration_ms > 0).then_some(duration_ms),
            }),
            AndroidTelemetryEvent::PlaybackState {
                generation,
                state_code,
                error_msg,
            } => {
                if let Some(tel) = decode_android_playback_state(generation, state_code, error_msg)
                {
                    Self::dispatch_audio_telemetry_from_jni(tel);
                }
            }
            AndroidTelemetryEvent::AuthCookiesCaptured { cookie_header } => {
                Self::dispatch_auth_cookies_from_jni(cookie_header);
            }
            AndroidTelemetryEvent::AuthWebViewCancelled => {
                log::info!("android auth webview sign-in cancelled");
            }
        }
    }

    pub async fn set_playback_speed(&self, speed: f32, preserve_pitch: bool) -> CoreResult<()> {
        self.invoker.audio_set_playback_speed(speed, preserve_pitch)
    }

    pub fn launch_sign_in_webview(&self) -> CoreResult<()> {
        self.invoker.launch_sign_in_webview()
    }
}

impl Default for AndroidMediaBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AudioBackend for AndroidMediaBridge {
    async fn load_and_play(&self, proxy_url: &str, meta: NowPlayingMetadata) -> CoreResult<()> {
        if !is_loopback_http_url(proxy_url) {
            return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
                format!(
                    "rejected non-loopback Android stream URL: {}",
                    redact_secrets(proxy_url)
                ),
            )));
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        ANDROID_ACTIVE_GENERATION.store(generation, Ordering::SeqCst);
        ANDROID_REPORTED_POSITION_MS.store(0, Ordering::SeqCst);
        ANDROID_REPORTED_DURATION_MS.store(meta.duration_ms, Ordering::SeqCst);
        ANDROID_REPORTED_PLAYING.store(true, Ordering::SeqCst);

        self.invoker
            .audio_load_and_play(proxy_url, &meta, generation)?;
        *self.active_url.lock().unwrap() = Some(proxy_url.to_owned());
        self.position_ms.store(0, Ordering::SeqCst);
        self.playing.store(true, Ordering::SeqCst);
        log::info!(
            "AndroidMediaBridge: handed loopback URL {} ({}) [gen={}] to Media3 ForegroundService",
            redact_secrets(proxy_url),
            meta.title,
            generation
        );
        Ok(())
    }

    async fn play(&self) -> CoreResult<()> {
        self.invoker.audio_play()?;
        self.playing.store(true, Ordering::SeqCst);
        ANDROID_REPORTED_PLAYING.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn pause(&self) -> CoreResult<()> {
        self.invoker.audio_pause()?;
        self.playing.store(false, Ordering::SeqCst);
        ANDROID_REPORTED_PLAYING.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&self) -> CoreResult<()> {
        self.invoker.audio_stop()?;
        self.playing.store(false, Ordering::SeqCst);
        self.position_ms.store(0, Ordering::SeqCst);
        ANDROID_REPORTED_PLAYING.store(false, Ordering::SeqCst);
        ANDROID_REPORTED_POSITION_MS.store(0, Ordering::SeqCst);
        Ok(())
    }

    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        self.invoker.audio_seek_ms(position_ms)?;
        self.position_ms.store(position_ms, Ordering::SeqCst);
        ANDROID_REPORTED_POSITION_MS.store(position_ms, Ordering::SeqCst);
        Ok(())
    }

    async fn set_volume(&self, volume: f32) -> CoreResult<()> {
        self.invoker.audio_set_volume(volume)
    }

    async fn position_ms(&self) -> u64 {
        let reported = ANDROID_REPORTED_POSITION_MS.load(Ordering::SeqCst);
        if reported > 0 {
            reported
        } else {
            self.position_ms.load(Ordering::SeqCst)
        }
    }

    async fn update_now_playing(&self, meta: NowPlayingMetadata, playing: bool, position_ms: u64) {
        self.invoker
            .audio_update_metadata(&meta, playing, position_ms);
        self.playing.store(playing, Ordering::SeqCst);
        self.position_ms.store(position_ms, Ordering::SeqCst);
        ANDROID_REPORTED_PLAYING.store(playing, Ordering::SeqCst);
        ANDROID_REPORTED_POSITION_MS.store(position_ms, Ordering::SeqCst);
    }

    fn take_command_receiver(&self) -> Option<UnboundedReceiver<MediaCommand>> {
        self.cmd_rx.lock().unwrap().take()
    }

    fn take_telemetry_receiver(&self) -> Option<UnboundedReceiver<AudioTelemetry>> {
        self.telemetry_rx.lock().unwrap().take()
    }
}

static ACTIVE_ANDROID_BACKEND: AtomicU8 = AtomicU8::new(0); // 0 = OsKeyring, 1 = EncryptedFallbackFile, 2 = MemoryOnly

fn encode_backend_kind(kind: CredentialBackendKind) -> u8 {
    match kind {
        CredentialBackendKind::OsKeyring => 0,
        CredentialBackendKind::EncryptedFallbackFile => 1,
        CredentialBackendKind::MemoryOnly => 2,
    }
}

fn decode_backend_kind(val: u8) -> CredentialBackendKind {
    match val {
        0 => CredentialBackendKind::OsKeyring,
        2 => CredentialBackendKind::MemoryOnly,
        _ => CredentialBackendKind::EncryptedFallbackFile,
    }
}

pub fn active_android_backend_kind() -> CredentialBackendKind {
    decode_backend_kind(ACTIVE_ANDROID_BACKEND.load(Ordering::SeqCst))
}

/// Android `AuthStore` backed by `KeystoreBridge.kt` (`AndroidKeyStore` AES-256-GCM)
/// via `ResonanceNativeBridge`, falling back deterministically to `0o600` `FileAuthStore`
/// if hardware Keystore fails, and wiping BOTH on `clear_session()`.
pub struct AndroidKeystoreAuthStore {
    invoker: Arc<dyn AndroidBridgeInvoker>,
    fallback: FileAuthStore,
    backend: Mutex<CredentialBackendKind>,
}

impl AndroidKeystoreAuthStore {
    pub fn with_android_keystore(data_dir: &Path) -> Self {
        Self::new(default_invoker(), FileAuthStore::new(data_dir))
    }

    pub fn new(invoker: Arc<dyn AndroidBridgeInvoker>, fallback: FileAuthStore) -> Self {
        ACTIVE_ANDROID_BACKEND.store(
            encode_backend_kind(CredentialBackendKind::OsKeyring),
            Ordering::SeqCst,
        );
        Self {
            invoker,
            fallback,
            backend: Mutex::new(CredentialBackendKind::OsKeyring),
        }
    }

    fn set_backend(&self, kind: CredentialBackendKind) {
        *self.backend.lock().unwrap() = kind;
        ACTIVE_ANDROID_BACKEND.store(encode_backend_kind(kind), Ordering::SeqCst);
    }

    pub fn backend_kind(&self) -> CredentialBackendKind {
        *self.backend.lock().unwrap()
    }
}

impl AuthStore for AndroidKeystoreAuthStore {
    fn load_session(&self) -> Result<Option<AuthSession>, AuthError> {
        match self.invoker.keystore_read(ANDROID_KEYSTORE_KEY) {
            Ok(Some(json)) => {
                let session = serde_json::from_str::<AuthSession>(&json).map_err(|e| {
                    AuthError::Keystore(format!(
                        "corrupt android keystore session: {}",
                        redact_secrets(&e.to_string())
                    ))
                })?;
                self.set_backend(CredentialBackendKind::OsKeyring);
                Ok(Some(session))
            }
            Ok(None) => match self.fallback.load_session()? {
                Some(session) => {
                    self.set_backend(CredentialBackendKind::EncryptedFallbackFile);
                    Ok(Some(session))
                }
                None => {
                    self.set_backend(CredentialBackendKind::OsKeyring);
                    Ok(None)
                }
            },
            Err(err) => {
                log::warn!(
                    "Android Keystore unavailable ({}); falling back to 0600 file store",
                    redact_secrets(&err)
                );
                self.set_backend(CredentialBackendKind::EncryptedFallbackFile);
                self.fallback.load_session()
            }
        }
    }

    fn save_session(&self, session: &AuthSession) -> Result<(), AuthError> {
        let has_sapisid = extract_cookie(&session.cookie_header, "SAPISID").is_some();
        let has_3papisid = extract_cookie(&session.cookie_header, "__Secure-3PAPISID").is_some();
        if !has_sapisid && !has_3papisid {
            return Err(AuthError::MissingCookie(
                "SAPISID or __Secure-3PAPISID".to_owned(),
            ));
        }

        let json = serde_json::to_string(session)
            .map_err(|e| AuthError::Keystore(redact_secrets(&e.to_string())))?;

        match self.invoker.keystore_write(ANDROID_KEYSTORE_KEY, &json) {
            Ok(true) => {
                // Remove any stale fallback file so old credentials cannot resurrect.
                let _ = self.fallback.clear_session();
                self.set_backend(CredentialBackendKind::OsKeyring);
                Ok(())
            }
            Ok(false) => {
                log::warn!(
                    "Android Keystore write returned false; falling back to 0600 file store"
                );
                self.set_backend(CredentialBackendKind::EncryptedFallbackFile);
                self.fallback.save_session(session)
            }
            Err(err) => {
                log::warn!(
                    "Android Keystore write failed ({}); falling back to 0600 file store",
                    redact_secrets(&err)
                );
                self.set_backend(CredentialBackendKind::EncryptedFallbackFile);
                self.fallback.save_session(session)
            }
        }
    }

    fn clear_session(&self) -> Result<(), AuthError> {
        // Always wipe BOTH Keystore and fallback so stale credentials cannot resurrect.
        if let Err(err) = self.invoker.keystore_delete(ANDROID_KEYSTORE_KEY) {
            log::warn!(
                "Android Keystore delete failed during sign-out ({}); continuing fallback cleanup",
                redact_secrets(&err)
            );
        }
        self.fallback.clear_session()
    }
}

static ANDROID_BACK_HANDLER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Allows the NSH UI/store to register whether a non-root route or modal can consume back.
pub fn set_android_back_can_pop(can_pop: bool) {
    ANDROID_BACK_HANDLER_ACTIVE.store(can_pop, Ordering::SeqCst);
}

/// Handles system/predictive back from `MainActivity`; returns `true` if NSH should consume it.
pub fn handle_inbound_back_pressed() -> bool {
    ANDROID_BACK_HANDLER_ACTIVE.load(Ordering::SeqCst)
}

/// Decodes and dispatches an inbound media command opcode from `ResonanceNativeBridge`.
pub fn handle_inbound_media_command(opcode: i32, arg_ms: i64) -> Option<MediaCommand> {
    let cmd = match opcode {
        CMD_PLAY => MediaCommand::Play,
        CMD_PAUSE => MediaCommand::Pause,
        CMD_TOGGLE_PLAY_PAUSE => MediaCommand::TogglePlayPause,
        CMD_NEXT => MediaCommand::Next,
        CMD_PREVIOUS => MediaCommand::Previous,
        CMD_SEEK_MS => MediaCommand::SeekMs(arg_ms.max(0) as u64),
        CMD_BECOMING_NOISY => MediaCommand::BecomingNoisy,
        CMD_AUDIO_FOCUS_LOST_TRANSIENT => MediaCommand::AudioFocusLostTransient,
        CMD_AUDIO_FOCUS_GAINED => MediaCommand::AudioFocusGained,
        CMD_STOP => MediaCommand::Stop,
        _ => return None,
    };
    AndroidMediaBridge::dispatch_from_jni(cmd);
    Some(cmd)
}

/// Filters stale generations and records inbound playback progress from ExoPlayer.
pub fn handle_inbound_playback_progress(
    generation: u64,
    position_ms: u64,
    duration_ms: u64,
) -> bool {
    let active = ANDROID_ACTIVE_GENERATION.load(Ordering::SeqCst);
    if generation < active {
        return false;
    }
    ANDROID_REPORTED_POSITION_MS.store(position_ms, Ordering::SeqCst);
    ANDROID_REPORTED_DURATION_MS.store(duration_ms, Ordering::SeqCst);
    AndroidMediaBridge::dispatch_telemetry_from_jni(AndroidTelemetryEvent::Progress {
        generation,
        position_ms,
        duration_ms,
    });
    true
}

/// Filters stale generations and records inbound ExoPlayer state transitions.
pub fn handle_inbound_playback_state(
    generation: u64,
    state_code: i32,
    error_msg: Option<String>,
) -> bool {
    let active = ANDROID_ACTIVE_GENERATION.load(Ordering::SeqCst);
    if generation < active {
        return false;
    }
    match state_code {
        STATE_PLAYING => ANDROID_REPORTED_PLAYING.store(true, Ordering::SeqCst),
        STATE_ENDED | STATE_PAUSED | STATE_ERROR => {
            ANDROID_REPORTED_PLAYING.store(false, Ordering::SeqCst)
        }
        _ => {}
    }
    let redacted_error = error_msg.map(|m| redact_secrets(&m));
    AndroidMediaBridge::dispatch_telemetry_from_jni(AndroidTelemetryEvent::PlaybackState {
        generation,
        state_code,
        error_msg: redacted_error,
    });
    true
}

/// Validates and dispatches captured YouTube Music sign-in cookies from WebView.
pub fn handle_inbound_auth_cookies(cookie_header: &str) -> bool {
    let has_sapisid = extract_cookie(cookie_header, "SAPISID").is_some();
    let has_3papisid = extract_cookie(cookie_header, "__Secure-3PAPISID").is_some();
    if !has_sapisid && !has_3papisid {
        return false;
    }
    AndroidMediaBridge::dispatch_telemetry_from_jni(AndroidTelemetryEvent::AuthCookiesCaptured {
        cookie_header: cookie_header.to_owned(),
    });
    true
}

/// Dispatches WebView sign-in cancellation from `WebViewSignInActivity`.
pub fn handle_inbound_auth_cancelled() {
    AndroidMediaBridge::dispatch_telemetry_from_jni(AndroidTelemetryEvent::AuthWebViewCancelled);
}

/// C-ABI JNI callbacks invoked by `dev.resonance.ytmusic.ResonanceNativeBridge`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeInitHost(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    bridge_class: jni::objects::JClass<'_>,
    files_dir: jni::objects::JString<'_>,
    cache_dir: jni::objects::JString<'_>,
) {
    let Ok(vm) = env.get_java_vm() else {
        return;
    };
    let Ok(global_bridge) = env.new_global_ref(&bridge_class) else {
        return;
    };
    let files_str: String = env
        .get_string(&files_dir)
        .map(Into::into)
        .unwrap_or_default();
    let cache_str: String = env
        .get_string(&cache_dir)
        .map(Into::into)
        .unwrap_or_default();
    if !files_str.is_empty() && !cache_str.is_empty() {
        resonance_core::storage::set_android_app_paths(
            PathBuf::from(&files_str),
            PathBuf::from(&cache_str),
        );
    }
    let _ = ANDROID_JNI_HOST.set(AndroidJniHost {
        vm,
        bridge_class: global_bridge,
        _files_dir: files_str,
        _cache_dir: cache_str,
    });
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeOnMediaCommand(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    opcode: jni::sys::jint,
    arg_ms: jni::sys::jlong,
) {
    let _ = handle_inbound_media_command(opcode, arg_ms);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeOnPlaybackProgress(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    generation: jni::sys::jlong,
    position_ms: jni::sys::jlong,
    duration_ms: jni::sys::jlong,
) {
    let _ = handle_inbound_playback_progress(
        generation.max(0) as u64,
        position_ms.max(0) as u64,
        duration_ms.max(0) as u64,
    );
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeOnPlaybackState(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    generation: jni::sys::jlong,
    state_code: jni::sys::jint,
    error_msg: jni::objects::JString<'_>,
) {
    let msg = if error_msg.is_null() {
        None
    } else {
        env.get_string(&error_msg).ok().map(Into::into)
    };
    let _ = handle_inbound_playback_state(generation.max(0) as u64, state_code, msg);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeInitPaths(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    files_dir: jni::objects::JString<'_>,
    cache_dir: jni::objects::JString<'_>,
) {
    let files_str: String = env
        .get_string(&files_dir)
        .map(Into::into)
        .unwrap_or_default();
    let cache_str: String = env
        .get_string(&cache_dir)
        .map(Into::into)
        .unwrap_or_default();
    if !files_str.is_empty() && !cache_str.is_empty() {
        resonance_core::storage::set_android_app_paths(
            PathBuf::from(files_str),
            PathBuf::from(cache_str),
        );
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeOnBackPressed(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
) -> jni::sys::jboolean {
    u8::from(handle_inbound_back_pressed())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeOnAuthCookiesCaptured(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    cookie_header: jni::objects::JString<'_>,
) {
    if cookie_header.is_null() {
        return;
    }
    if let Ok(cookies) = env.get_string(&cookie_header) {
        let cookies_str: String = cookies.into();
        let _ = handle_inbound_auth_cookies(&cookies_str);
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeOnWebViewCancelled(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
) {
    handle_inbound_auth_cancelled();
}

/// Non-blocking in-memory snapshot served immediately to Android Auto Binder queries.
#[derive(Debug, Clone, Default)]
pub struct AndroidAutoBrowseCache {
    pub downloads: Vec<resonance_contract::Track>,
    pub home: Vec<resonance_contract::Track>,
    pub library: Vec<resonance_contract::Track>,
    pub queue: Vec<resonance_contract::Track>,
}

static ANDROID_AUTO_BROWSE_CACHE: std::sync::OnceLock<Mutex<AndroidAutoBrowseCache>> =
    std::sync::OnceLock::new();

fn auto_browse_cache() -> &'static Mutex<AndroidAutoBrowseCache> {
    ANDROID_AUTO_BROWSE_CACHE.get_or_init(|| Mutex::new(AndroidAutoBrowseCache::default()))
}

pub fn update_android_auto_tab(tab_id: &str, tracks: Vec<resonance_contract::Track>) {
    if let Ok(mut cache) = auto_browse_cache().lock() {
        match tab_id {
            "tab_downloads" => cache.downloads = tracks,
            "tab_home" => cache.home = tracks,
            "tab_library" => cache.library = tracks,
            "tab_queue" => cache.queue = tracks,
            _ => {}
        }
    }
}

pub fn serialize_android_auto_children_json(parent_id: &str) -> String {
    let tracks = auto_browse_cache()
        .lock()
        .ok()
        .map(|c| match parent_id {
            "tab_downloads" => c.downloads.clone(),
            "tab_home" => c.home.clone(),
            "tab_library" => c.library.clone(),
            "tab_queue" => c.queue.clone(),
            _ => Vec::new(),
        })
        .unwrap_or_default();

    let items: Vec<serde_json::Value> = tracks
        .into_iter()
        .take(100)
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "artist": t.artist_display(),
                "album": t.album.as_ref().map(|a| a.title.as_str()).unwrap_or(""),
                "artwork_url": t.artwork_url.unwrap_or_default(),
            })
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_owned())
}

static LAST_ANDROID_DSP_CONFIG: std::sync::OnceLock<Mutex<resonance_core::audio_dsp::DspConfig>> =
    std::sync::OnceLock::new();

fn last_android_dsp_config() -> &'static Mutex<resonance_core::audio_dsp::DspConfig> {
    LAST_ANDROID_DSP_CONFIG
        .get_or_init(|| Mutex::new(resonance_core::audio_dsp::DspConfig::default()))
}

/// Thread-safe handle owned by Android's `ResonanceDspAudioProcessor` across JNI calls.
///
/// Reuses a single `f32` scratch buffer so real-time audio callbacks perform zero heap
/// allocations after warmup.
pub struct AndroidDspHandle {
    pipeline: Mutex<resonance_core::audio_dsp::DspPipeline>,
    scratch: Mutex<Vec<f32>>,
}

impl AndroidDspHandle {
    pub fn new(sample_rate: u32, channel_count: usize) -> Self {
        let cfg = last_android_dsp_config()
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default();
        Self {
            pipeline: Mutex::new(resonance_core::audio_dsp::DspPipeline::new(
                sample_rate.max(8_000),
                channel_count.clamp(1, 8),
                &cfg,
            )),
            scratch: Mutex::new(Vec::with_capacity(4096)),
        }
    }

    pub fn update_config(
        &self,
        loudness_enabled: bool,
        silence_skip_enabled: bool,
        eq_gains_db: [f32; 10],
        track_loudness_db: Option<f32>,
    ) {
        let has_eq = eq_gains_db.iter().any(|g| g.abs() >= 0.01);
        let cfg = resonance_core::audio_dsp::DspConfig {
            loudness_normalization: loudness_enabled,
            eq_preset: if has_eq {
                resonance_core::audio_dsp::EqPreset::Custom
            } else {
                resonance_core::audio_dsp::EqPreset::Flat
            },
            eq_gains_db,
            silence_skip_enabled,
            crossfade_ms: 0,
        };
        if let Ok(mut guard) = last_android_dsp_config().lock() {
            *guard = cfg.clone();
        }
        if let Ok(mut pipeline) = self.pipeline.lock() {
            pipeline.apply_config(&cfg);
            if let Some(db) = track_loudness_db {
                pipeline.set_track_loudness_db(Some(db));
            }
        }
    }

    /// Processes little-endian PCM-16 or PCM-F32 bytes from `input` into `output`.
    /// Returns `true` to keep the processed audio block, or `false` if skipped as silence.
    pub fn process_pcm_bytes(&self, input: &[u8], output: &mut [u8], is_float: bool) -> bool {
        if input.is_empty() || output.len() < input.len() {
            return true;
        }
        let Ok(mut pipeline) = self.pipeline.lock() else {
            output[..input.len()].copy_from_slice(input);
            return true;
        };
        let Ok(mut scratch) = self.scratch.lock() else {
            output[..input.len()].copy_from_slice(input);
            return true;
        };

        if is_float {
            if input.len() % 4 != 0 {
                output[..input.len()].copy_from_slice(input);
                return true;
            }
            let samples = input.len() / 4;
            scratch.clear();
            scratch.reserve(samples.saturating_sub(scratch.capacity()));
            for chunk in input.chunks_exact(4) {
                scratch.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
            let keep = pipeline.process_interleaved(&mut scratch[..samples]);
            if keep {
                for (i, sample) in scratch.iter().copied().enumerate() {
                    let bytes = sample.to_le_bytes();
                    let off = i * 4;
                    output[off..off + 4].copy_from_slice(&bytes);
                }
            }
            keep
        } else {
            if input.len() % 2 != 0 {
                output[..input.len()].copy_from_slice(input);
                return true;
            }
            let samples = input.len() / 2;
            scratch.clear();
            scratch.reserve(samples.saturating_sub(scratch.capacity()));
            for chunk in input.chunks_exact(2) {
                let s = i16::from_le_bytes([chunk[0], chunk[1]]);
                scratch.push(s as f32 / 32768.0);
            }
            let keep = pipeline.process_interleaved(&mut scratch[..samples]);
            if keep {
                for (i, sample) in scratch.iter().copied().enumerate() {
                    let pcm = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                    let bytes = pcm.to_le_bytes();
                    let off = i * 2;
                    output[off..off + 2].copy_from_slice(&bytes);
                }
            }
            keep
        }
    }
}

/// Pushes live `DspConfig` changes to Android's Media3 `ResonanceDspAudioProcessor`.
pub fn update_android_dsp(cfg: &resonance_core::audio_dsp::DspConfig) {
    if let Ok(mut guard) = last_android_dsp_config().lock() {
        *guard = cfg.clone();
    }
    #[cfg(target_os = "android")]
    {
        if let Some(host) = ANDROID_JNI_HOST.get()
            && let Ok(mut env) = host.vm.attach_current_thread()
            && let Ok(gains_arr) = env.new_float_array(10)
            && env
                .set_float_array_region(&gains_arr, 0, &cfg.eq_gains_db)
                .is_ok()
        {
            let class = <&jni::objects::JClass>::from(host.bridge_class.as_obj());
            let _ = env.call_static_method(
                class,
                "audioUpdateDsp",
                "(ZZ[FF)V",
                &[
                    jni::objects::JValue::Bool(u8::from(cfg.loudness_normalization)),
                    jni::objects::JValue::Bool(u8::from(cfg.silence_skip_enabled)),
                    jni::objects::JValue::Object(&gains_arr),
                    jni::objects::JValue::Float(f32::NAN),
                ],
            );
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = cfg;
    }
}

/// Copies UTF-8 text to the Android system clipboard via `ResonanceNativeBridge.copyToClipboard`.
#[cfg(target_os = "android")]
pub fn copy_to_android_clipboard(text: &str) -> Result<(), String> {
    let host = ANDROID_JNI_HOST
        .get()
        .ok_or_else(|| "Android JNI host not initialized".to_owned())?;
    let mut env = host
        .vm
        .attach_current_thread()
        .map_err(|e| format!("JNI attach failed: {e}"))?;
    let jtext = env
        .new_string(text)
        .map_err(|e| format!("JNI string alloc failed: {e}"))?;
    let class = <&jni::objects::JClass>::from(host.bridge_class.as_obj());
    let ok = env
        .call_static_method(
            class,
            "copyToClipboard",
            "(Ljava/lang/String;)Z",
            &[jni::objects::JValue::Object(&jtext)],
        )
        .and_then(|v| v.z())
        .map_err(|e| format!("Android clipboard error: {e}"))?;
    if ok {
        Ok(())
    } else {
        Err("Android clipboard unavailable".to_owned())
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceDspAudioProcessor_nativeCreateDspHandle(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    sample_rate: jni::sys::jint,
    channel_count: jni::sys::jint,
) -> jni::sys::jlong {
    let sr = sample_rate.max(8_000) as u32;
    let ch = channel_count.clamp(1, 8) as usize;
    Box::into_raw(Box::new(AndroidDspHandle::new(sr, ch))) as jni::sys::jlong
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceDspAudioProcessor_nativeProcessPcm(
    env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    handle: jni::sys::jlong,
    input_buffer: jni::objects::JByteBuffer<'_>,
    input_offset: jni::sys::jint,
    byte_length: jni::sys::jint,
    output_buffer: jni::objects::JByteBuffer<'_>,
    is_float: jni::sys::jboolean,
) -> jni::sys::jboolean {
    if handle == 0 || input_offset < 0 || byte_length <= 0 {
        return 1;
    }
    let Ok(in_ptr) = env.get_direct_buffer_address(&input_buffer) else {
        return 1;
    };
    let Ok(in_cap) = env.get_direct_buffer_capacity(&input_buffer) else {
        return 1;
    };
    let Ok(out_ptr) = env.get_direct_buffer_address(&output_buffer) else {
        return 1;
    };
    let Ok(out_cap) = env.get_direct_buffer_capacity(&output_buffer) else {
        return 1;
    };

    let off = input_offset as usize;
    let len = byte_length as usize;
    let Some(end) = off.checked_add(len) else {
        return 1;
    };
    if in_ptr.is_null() || out_ptr.is_null() || end > in_cap || len > out_cap {
        return 1;
    }

    // SAFETY: Pointers and capacities were verified against JNI direct buffer bounds above,
    // and `handle` was created by `nativeCreateDspHandle` and remains valid until `nativeDestroyDspHandle`.
    let dsp = unsafe { &*(handle as *const AndroidDspHandle) };
    let in_slice = unsafe { std::slice::from_raw_parts(in_ptr.add(off), len) };
    let out_slice = unsafe { std::slice::from_raw_parts_mut(out_ptr, len) };
    u8::from(dsp.process_pcm_bytes(in_slice, out_slice, is_float != 0))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceDspAudioProcessor_nativeUpdateDspConfig(
    env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    handle: jni::sys::jlong,
    loudness_enabled: jni::sys::jboolean,
    silence_skip_enabled: jni::sys::jboolean,
    eq_gains_db: jni::objects::JFloatArray<'_>,
    track_loudness_db: jni::sys::jfloat,
) {
    if handle == 0 {
        return;
    }
    let mut gains = [0.0_f32; 10];
    if let Ok(arr_len) = env.get_array_length(&eq_gains_db) {
        let count = (arr_len.max(0) as usize).min(10);
        if count > 0 {
            let _ = env.get_float_array_region(&eq_gains_db, 0, &mut gains[..count]);
        }
    }
    let track_db = if track_loudness_db.is_finite() {
        Some(track_loudness_db)
    } else {
        None
    };
    // SAFETY: `handle` is owned by `ResonanceDspAudioProcessor` until `nativeDestroyDspHandle`.
    let dsp = unsafe { &*(handle as *const AndroidDspHandle) };
    dsp.update_config(
        loudness_enabled != 0,
        silence_skip_enabled != 0,
        gains,
        track_db,
    );
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceDspAudioProcessor_nativeDestroyDspHandle(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    handle: jni::sys::jlong,
) {
    if handle != 0 {
        // SAFETY: Releases the `Box<AndroidDspHandle>` created in `nativeCreateDspHandle`.
        unsafe {
            drop(Box::from_raw(handle as *mut AndroidDspHandle));
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeGetAutoBrowseChildren(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    parent_id: jni::objects::JString<'_>,
) -> jni::sys::jstring {
    let parent: String = env
        .get_string(&parent_id)
        .map(Into::into)
        .unwrap_or_default();
    let json = serialize_android_auto_children_json(&parent);
    env.new_string(json)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeOnAutoPlayTrack(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    video_id: jni::objects::JString<'_>,
) {
    let Ok(vid) = env.get_string(&video_id) else {
        return;
    };
    let vid_str: String = vid.into();
    let found = auto_browse_cache().lock().ok().and_then(|c| {
        c.downloads
            .iter()
            .chain(c.home.iter())
            .chain(c.library.iter())
            .chain(c.queue.iter())
            .find(|t| t.id == vid_str)
            .cloned()
    });
    if let Some(track) = found {
        crate::live_engine::dispatch(crate::live_engine::EngineCommand::PlayTrack(track));
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_resonance_ytmusic_ResonanceNativeBridge_nativeOnLocalFolderPicked(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    tree_uri: jni::objects::JString<'_>,
) {
    if let Ok(uri) = env.get_string(&tree_uri) {
        let uri_str: String = uri.into();
        if !uri_str.trim().is_empty() {
            crate::live_engine::dispatch(crate::live_engine::EngineCommand::AddLocalFolder(
                PathBuf::from(uri_str),
            ));
        }
    }
}
