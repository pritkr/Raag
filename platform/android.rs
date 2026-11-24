//! Android Media Lifecycle JNI Bridge (`PlaybackForegroundService.kt`).
//!
//! Connects Rust `AudioBackend` to the Kotlin Media3/ExoPlayer Foreground Service
//! (`MediaSession` lock-screen/AVRCP controls, `AudioFocus`, `ACTION_AUDIO_BECOMING_NOISY`).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use resonance_contract::{AudioBackend, CoreResult, MediaCommand, NowPlayingMetadata};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

static ANDROID_CMD_TX: std::sync::OnceLock<UnboundedSender<MediaCommand>> =
    std::sync::OnceLock::new();

pub struct AndroidMediaBridge {
    playing: AtomicBool,
    position_ms: AtomicU64,
    active_url: Mutex<Option<String>>,
    cmd_rx: Mutex<Option<UnboundedReceiver<MediaCommand>>>,
}

impl AndroidMediaBridge {
    pub fn new() -> Self {
        let (tx, rx) = unbounded_channel();
        let _ = ANDROID_CMD_TX.set(tx);
        Self {
            playing: AtomicBool::new(false),
            position_ms: AtomicU64::new(0),
            active_url: Mutex::new(None),
            cmd_rx: Mutex::new(Some(rx)),
        }
    }

    /// Invoked from Kotlin `PlaybackForegroundService` (MediaSession / BecomingNoisy / AudioFocus).
    pub fn dispatch_from_jni(cmd: MediaCommand) {
        if let Some(tx) = ANDROID_CMD_TX.get() {
            let _ = tx.send(cmd);
        }
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
        *self.active_url.lock().unwrap() = Some(proxy_url.to_owned());
        self.position_ms.store(0, Ordering::SeqCst);
        self.playing.store(true, Ordering::SeqCst);
        log::info!(
            "AndroidMediaBridge: handing loopback URL {proxy_url} ({}) to Media3 ForegroundService",
            meta.title
        );
        Ok(())
    }

    async fn play(&self) -> CoreResult<()> {
        self.playing.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn pause(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        self.position_ms.store(0, Ordering::SeqCst);
        Ok(())
    }

    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        self.position_ms.store(position_ms, Ordering::SeqCst);
        Ok(())
    }

    async fn set_volume(&self, _volume: f32) -> CoreResult<()> {
        Ok(())
    }

    async fn position_ms(&self) -> u64 {
        self.position_ms.load(Ordering::SeqCst)
    }

    async fn update_now_playing(&self, _meta: NowPlayingMetadata, playing: bool, position_ms: u64) {
        self.playing.store(playing, Ordering::SeqCst);
        self.position_ms.store(position_ms, Ordering::SeqCst);
    }

    fn take_command_receiver(&self) -> Option<UnboundedReceiver<MediaCommand>> {
        self.cmd_rx.lock().unwrap().take()
    }
}

/// C-ABI JNI callback symbols invoked by `dev.resonance.ytmusic.PlaybackForegroundService`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_dev_resonance_ytmusic_PlaybackForegroundService_nativeOnMediaCommand(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    opcode: i32,
    arg_ms: i64,
) {
    let cmd = match opcode {
        1 => MediaCommand::Play,
        2 => MediaCommand::Pause,
        3 => MediaCommand::TogglePlayPause,
        4 => MediaCommand::Next,
        5 => MediaCommand::Previous,
        6 => MediaCommand::SeekMs(arg_ms.max(0) as u64),
        7 => MediaCommand::BecomingNoisy,
        8 => MediaCommand::AudioFocusLostTransient,
        9 => MediaCommand::AudioFocusGained,
        _ => return,
    };
    AndroidMediaBridge::dispatch_from_jni(cmd);
}
