//! Native `nsh/audio` End-to-End Integration Test Suite.
//!
//! Injects a recording `nsh_core::audio::AudioBackend` via `EngineConfig::audio`
//! into a live Pure Rust NSH `Headless` app and verifies that UI interactions
//! drive real `nsh_core::prelude::audio_player` commands (`Open`, `Resume`, `Pause`,
//! `Seek`, `Set`, `Close`) against Resonance's Loopback HTTP Proxy URLs, and that
//! native `AudioEvent`s (`Progress`, `Ended`, `Load`) flow back into the UI.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nsh_core::audio::{AudioBackend as NshAudioTrait, AudioCommand, AudioEvent, AudioSink};
use nsh_engine::{EngineConfig, Points, Root, Size};
use nsh_testing::Headless;
use resonance::platform::nsh_audio::{NshAudioBackend, NshAudioControl};
use resonance_contract::{AudioBackend, NowPlayingMetadata};

#[derive(Default)]
struct RecordingNshAudio {
    commands: Mutex<Vec<AudioCommand>>,
    sinks: Mutex<HashMap<u32, AudioSink>>,
    active_id: Mutex<Option<u32>>,
}

impl RecordingNshAudio {
    fn commands(&self) -> Vec<AudioCommand> {
        self.commands.lock().unwrap().clone()
    }

    fn clear_commands(&self) {
        self.commands.lock().unwrap().clear();
    }

    fn emit_to_active(&self, event: AudioEvent) -> bool {
        let id = *self.active_id.lock().unwrap();
        if let Some(id) = id
            && let Some(sink) = self.sinks.lock().unwrap().get(&id)
        {
            return sink.send(event);
        }
        false
    }
}

impl NshAudioTrait for RecordingNshAudio {
    fn available(&self) -> bool {
        true
    }

    fn command(&self, command: AudioCommand, events: &AudioSink) {
        match &command {
            AudioCommand::Open { id, .. } => {
                self.sinks.lock().unwrap().insert(*id, events.clone());
                *self.active_id.lock().unwrap() = Some(*id);
            }
            AudioCommand::Resume { .. } => {
                let _ = events.send(AudioEvent::Playing);
            }
            AudioCommand::Pause { .. } => {
                let _ = events.send(AudioEvent::Paused);
            }
            AudioCommand::Close { id } => {
                self.sinks.lock().unwrap().remove(id);
                let mut cur = self.active_id.lock().unwrap();
                if *cur == Some(*id) {
                    *cur = None;
                }
            }
            _ => {}
        }
        self.commands.lock().unwrap().push(command);
    }
}

#[test]
fn native_nsh_audio_player_drives_open_pause_resume_seek_volume_loop_and_auto_advance()
-> Result<(), String> {
    resonance::store::enable_fixture_mode(true);
    resonance::store::set_proxy_base_url("http://127.0.0.1:41234");

    let recorder = Arc::new(RecordingNshAudio::default());
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(Points(1200.0), Points(800.0)),
        audio: Some(recorder.clone()),
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .expect("headless NSH engine with native audio backend starts");

    app.wait_for("Raag")?;

    // 1. Initial mount opens & resumes the first fixture track via Loopback Proxy URL
    let initial_cmds = recorder.commands();
    let first_open = initial_cmds.iter().find_map(|c| match c {
        AudioCommand::Open { id, src, settings } => Some((*id, src.clone(), *settings)),
        _ => None,
    });
    let (first_id, first_src, first_settings) =
        first_open.expect("native nsh/audio must emit AudioCommand::Open on startup");
    assert_eq!(
        first_src,
        "http://127.0.0.1:41234/stream/fix-long-70?itag=251"
    );
    assert!((first_settings.volume - 1.0).abs() < 0.01);
    assert!(!first_settings.looped);
    assert!(
        initial_cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::Resume { id } if *id == first_id)),
        "startup playing track must emit AudioCommand::Resume"
    );

    // Navigate to Now Playing to drive transport, seek, volume, and repeat controls
    app.press("id:nav-now-playing")?;
    app.wait_for("Now Playing")?;

    // 2. Pause -> Resume
    recorder.clear_commands();
    app.press("label:Play or Pause")?;
    app.wait_for("Paused")?;
    assert!(
        recorder
            .commands()
            .iter()
            .any(|c| matches!(c, AudioCommand::Pause { id } if *id == first_id)),
        "Pause button must emit native AudioCommand::Pause"
    );

    recorder.clear_commands();
    app.press("label:Play or Pause")?;
    app.wait_for("Playing")?;
    assert!(
        recorder
            .commands()
            .iter()
            .any(|c| matches!(c, AudioCommand::Resume { id } if *id == first_id)),
        "Resume button must emit native AudioCommand::Resume"
    );

    // 3. Seek +15s (26s -> 41s) emits AudioCommand::Seek
    recorder.clear_commands();
    app.press("+15s")?;
    app.wait_for("0:41")?;
    assert!(
        recorder.commands().iter().any(|c| matches!(
            c,
            AudioCommand::Seek { id, position }
                if *id == first_id && *position == Duration::from_secs(41)
        )),
        "+15s button must emit native AudioCommand::Seek(41s)"
    );

    // 4. Volume -10% emits AudioCommand::Set with volume ~= 0.90
    recorder.clear_commands();
    app.press("label:Decrease Volume")?;
    app.wait_for("Volume 90%")?;
    assert!(
        recorder.commands().iter().any(|c| matches!(
            c,
            AudioCommand::Set { id, settings }
                if *id == first_id && (settings.volume - 0.9).abs() < 0.02
        )),
        "Decrease Volume must emit native AudioCommand::Set(volume=0.9)"
    );

    // 5. Cycle Repeat twice (None -> All -> One) emits AudioCommand::Set(looped=true)
    recorder.clear_commands();
    app.press("label:Cycle Repeat Mode")?;
    app.press("label:Cycle Repeat Mode")?;
    assert!(
        recorder.commands().iter().any(|c| matches!(
            c,
            AudioCommand::Set { id, settings } if *id == first_id && settings.looped
        )),
        "Repeat: One must emit native AudioCommand::Set(looped=true)"
    );
    // Return Repeat to None before track advance
    app.press("label:Cycle Repeat Mode")?;

    // 6. Native AudioEvent::Progress updates UI position WITHOUT echoing AudioCommand::Seek
    recorder.clear_commands();
    assert!(recorder.emit_to_active(AudioEvent::Progress {
        position: Duration::from_secs(65),
        duration: Some(Duration::from_secs(252)),
    }));
    app.wait_for("1:05")?;
    assert!(
        !recorder
            .commands()
            .iter()
            .any(|c| matches!(c, AudioCommand::Seek { .. })),
        "AudioEvent::Progress must NOT trigger a feedback AudioCommand::Seek"
    );

    // 7. Next Track closes player #1 and opens + resumes track #2
    recorder.clear_commands();
    app.press("label:Next Track")?;
    app.wait_for("夜に駆ける 🌙✨ — Tokyo Midnight Synthwave (2026 Remaster)")?;
    let next_cmds = recorder.commands();
    assert!(
        next_cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::Close { id } if *id == first_id)),
        "Switching tracks must emit AudioCommand::Close on previous player"
    );
    let second_id = next_cmds
        .iter()
        .find_map(|c| match c {
            AudioCommand::Open { id, src, .. }
                if src == "http://127.0.0.1:41234/stream/fix-cjk-emoji?itag=251" =>
            {
                Some(*id)
            }
            _ => None,
        })
        .expect("Next Track must emit AudioCommand::Open for fix-cjk-emoji");
    assert!(
        next_cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::Resume { id } if *id == second_id)),
        "Next Track must emit AudioCommand::Resume for track #2"
    );

    // 8. Native AudioEvent::Ended auto-advances to track #3 and opens it in native nsh/audio!
    recorder.clear_commands();
    assert!(recorder.emit_to_active(AudioEvent::Ended));
    let mut opened = false;
    for _ in 0..50 {
        if recorder.commands().iter().any(|c| {
            matches!(
                c,
                AudioCommand::Open { src, .. }
                    if src == "http://127.0.0.1:41234/stream/fix-missing-art?itag=251"
            )
        }) {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        opened,
        "AudioEvent::Ended must auto-advance and open track #3 in native nsh/audio"
    );

    Ok(())
}

#[tokio::test]
async fn nsh_audio_backend_bridge_forwards_playback_controller_commands() {
    let backend = NshAudioBackend::new();
    let mut rx = backend
        .take_control_receiver()
        .expect("control receiver available once");
    let meta = NowPlayingMetadata {
        track_id: "abc".into(),
        title: "Track One".into(),
        artist: "Artist".into(),
        album: None,
        artwork_url: None,
        duration_ms: 180_000,
    };

    backend
        .load_and_play("http://127.0.0.1:9999/stream/abc?itag=140", meta)
        .await
        .unwrap();
    backend.pause().await.unwrap();
    backend.play().await.unwrap();
    backend.seek_ms(12_000).await.unwrap();
    backend.set_volume(0.65).await.unwrap();
    backend.stop().await.unwrap();

    assert!(matches!(
        rx.recv().await,
        Some(NshAudioControl::LoadAndPlay { proxy_url, .. })
            if proxy_url == "http://127.0.0.1:9999/stream/abc?itag=140"
    ));
    assert_eq!(rx.recv().await, Some(NshAudioControl::Pause));
    assert_eq!(rx.recv().await, Some(NshAudioControl::Play));
    assert_eq!(rx.recv().await, Some(NshAudioControl::SeekMs(12_000)));
    assert_eq!(rx.recv().await, Some(NshAudioControl::SetVolume(0.65)));
    assert_eq!(rx.recv().await, Some(NshAudioControl::Stop));
}
