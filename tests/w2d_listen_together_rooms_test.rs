use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use nsh_core::prelude::{IntoElement, component, signal};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use resonance::screens::listen_together::{ListenTogetherCallbacks, listen_together_screen};
use resonance_contract::{
    AuthoritativePlaybackState, QueuePermission, RoomParticipant, RoomRole, RoomTrackRef,
    RoomWireMessage,
};
use resonance_core::rooms::{
    ClockSyncFilter, RoomCommandRequest, RoomController, RoomStateSnapshot, RoomTransportKind,
    ThreeZoneDriftAction, ThreeZoneDriftController,
};
use resonance_room_relay::{RelayConfig, spawn_relay};

#[test]
fn clock_sync_estimator_rejects_jitter_spikes_and_converges_offset() {
    let mut filter = ClockSyncFilter::new();
    let true_offset = 420_i64; // server is 420 ms ahead of local clock

    // 6 clean samples (~20 ms RTT) + 2 cellular jitter spikes (900 ms RTT)
    let samples = [
        (1_000_u64, 20_u64),
        (1_200, 18),
        (1_400, 22),
        (1_600, 900), // spike 1
        (1_800, 19),
        (2_000, 21),
        (2_200, 920), // spike 2
        (2_400, 20),
    ];

    for (i, (t1, rtt)) in samples.into_iter().enumerate() {
        let half = rtt / 2;
        let t2 = (t1 + half) as i64 + true_offset;
        let t3 = t2 + 2;
        let t4 = t1 + rtt + 2;
        let res = filter.record_pong(t1, t2 as u64, t3 as u64, t4);
        if i == 3 || i == 6 {
            assert!(
                res.is_none(),
                "expected 900ms jitter spike #{i} to be rejected"
            );
        } else {
            assert!(res.is_some(), "expected clean sample #{i} to be accepted");
        }
    }

    let err = (filter.offset_ms() - true_offset).abs();
    assert!(
        err <= 5,
        "expected converged offset within ±5ms of {true_offset}, got {} (err={err})",
        filter.offset_ms()
    );
}

#[test]
fn three_zone_hysteresis_controller_respects_deadband_slew_and_seek_cooldown() {
    let mut ctrl = ThreeZoneDriftController::new();
    let playback = AuthoritativePlaybackState {
        revision: 1,
        current_track: None,
        queue: Vec::new(),
        is_playing: true,
        anchor_position_ms: 10_000,
        anchor_server_ms: 50_000,
    };

    // Initial revision bump with 30 ms drift -> Nominal (Zone 1 deadband)
    assert_eq!(
        ctrl.evaluate(&playback, 10_030, 50_000, 0, true),
        ThreeZoneDriftAction::Nominal
    );

    // 180 ms drift for 3 consecutive ticks -> ticks 1 & 2 Nominal, tick 3 enters Slew (~0.988)
    assert_eq!(
        ctrl.evaluate(&playback, 10_680, 50_500, 0, true),
        ThreeZoneDriftAction::Nominal
    );
    assert_eq!(
        ctrl.evaluate(&playback, 11_180, 51_000, 0, true),
        ThreeZoneDriftAction::Nominal
    );
    match ctrl.evaluate(&playback, 11_680, 51_500, 0, true) {
        ThreeZoneDriftAction::Slew(ratio) => {
            assert!(
                (ratio - 0.988).abs() < 0.002,
                "expected slew ~0.988 for +180ms drift, got {ratio}"
            );
        }
        other => panic!("expected Slew on 3rd tick, got {other:?}"),
    }
    assert!(ctrl.is_slewing());

    // Drops to 15 ms (< 20 ms exit threshold) -> exits slew cleanly
    assert_eq!(
        ctrl.evaluate(&playback, 12_015, 52_000, 0, true),
        ThreeZoneDriftAction::Nominal
    );
    assert!(!ctrl.is_slewing());

    // 800 ms drift -> HardSeek once, then 3s cooldown suppresses repeat seek
    assert_eq!(
        ctrl.evaluate(&playback, 13_800, 53_000, 0, true),
        ThreeZoneDriftAction::HardSeek(13_000)
    );
    assert_eq!(
        ctrl.evaluate(&playback, 14_800, 54_000, 0, true),
        ThreeZoneDriftAction::Nominal
    );
    // After 3s cooldown expires -> HardSeek fires again
    assert_eq!(
        ctrl.evaluate(&playback, 17_800, 56_500, 0, true),
        ThreeZoneDriftAction::HardSeek(16_500)
    );
}

#[tokio::test]
async fn room_relay_websocket_and_long_poll_end_to_end_sync() {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let relay = spawn_relay(addr, RelayConfig::default()).await.unwrap();
    let base_url = relay.base_url();

    // 1. Host creates room via WebSocket
    let mut host = RoomController::create_room(&base_url, "Host", QueuePermission::GuestsCanAdd)
        .await
        .unwrap();
    assert_eq!(host.transport(), RoomTransportKind::WebSocket);
    let code = host.snapshot().room_code;

    // 2. Guest joins via WebSocket
    let mut guest_ws = RoomController::join_room(&base_url, &code, "GuestWS")
        .await
        .unwrap();
    assert_eq!(guest_ws.transport(), RoomTransportKind::WebSocket);

    // 3. Force 502 on WS upgrade so third peer falls back to HTTP Long-Poll
    relay.set_reject_ws_502(true);
    let guest_lp = RoomController::join_room(&base_url, &code, "GuestLP")
        .await
        .unwrap();
    assert_eq!(guest_lp.transport(), RoomTransportKind::HttpLongPoll);
    relay.set_reject_ws_502(false);

    // 4. Local-file broadcast must be blocked on host & guest
    let local_err = guest_ws
        .guest_suggest_track(RoomTrackRef {
            video_id: "local:song-1".to_owned(),
            title: "Private".to_owned(),
            artist: "Local".to_owned(),
            duration_ms: 120_000,
            artwork_url: None,
        })
        .await
        .unwrap_err();
    assert!(local_err.contains("Local track"));

    // 5. Host broadcasts track + queued next track
    host.host_sync_state(AuthoritativePlaybackState {
        revision: 2,
        current_track: Some(RoomTrackRef {
            video_id: "yt_track_1".to_owned(),
            title: "Track One".to_owned(),
            artist: "Artist A".to_owned(),
            duration_ms: 200_000,
            artwork_url: None,
        }),
        queue: vec![RoomTrackRef {
            video_id: "yt_track_2".to_owned(),
            title: "Track Two".to_owned(),
            artist: "Artist B".to_owned(),
            duration_ms: 210_000,
            artwork_url: None,
        }],
        is_playing: true,
        anchor_position_ms: 0,
        anchor_server_ms: 0,
    })
    .await
    .unwrap();

    // 6. Unauthorized host command returns 403 Forbidden
    let client = reqwest::Client::new();
    let unauth = client
        .post(format!("{base_url}/v1/rooms/{code}/cmd"))
        .json(&RoomCommandRequest {
            peer_id: guest_ws.snapshot().peer_id,
            host_secret: Some("wrong-secret".to_owned()),
            message: RoomWireMessage::HostSyncState {
                playback: host.snapshot().playback,
            },
        })
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::FORBIDDEN);

    // 7. Vote-skip threshold (3 participants -> threshold 2): 1st vote doesn't skip, 2nd skips to Track Two
    let v1 = guest_ws.cast_vote_skip().await.unwrap();
    assert_eq!(v1.votes, 1);
    assert_eq!(v1.threshold, 2);
    assert!(!v1.skipped);

    let v2 = guest_lp.cast_vote_skip().await.unwrap();
    assert_eq!(v2.votes, 2);
    assert!(v2.skipped);

    relay.shutdown();
}

#[test]
fn listen_together_screen_renders_create_join_and_active_room_headless() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = fs::read_to_string(root.join("src/screens/listen_together.rs")).unwrap();
    assert!(!src.contains("rgb("));
    assert!(!src.contains("rgba("));
    assert!(!src.contains("0xFF"));

    #[component]
    fn EmptyRoomRoot() -> impl IntoElement {
        let snap = signal(None);
        let warn = signal(None);
        listen_together_screen(snap, warn, ListenTogetherCallbacks::default())
    }

    let mut app = Headless::start(EngineConfig::new(Root::of::<EmptyRoomRoot>())).unwrap();
    app.wait_for("Start or Join a Room").unwrap();
    app.wait_for("Create Room").unwrap();
    app.wait_for("Join Room").unwrap();

    #[component]
    fn ActiveRoomRoot() -> impl IntoElement {
        let snap = signal(Some(RoomStateSnapshot {
            room_code: "RAAG-7K4M".to_owned(),
            role: RoomRole::Host,
            peer_id: "peer-1".to_owned(),
            host_peer_id: "peer-1".to_owned(),
            queue_permission: QueuePermission::GuestsCanAdd,
            participants: vec![
                RoomParticipant {
                    peer_id: "peer-1".to_owned(),
                    display_name: "Host".to_owned(),
                    role: RoomRole::Host,
                },
                RoomParticipant {
                    peer_id: "peer-2".to_owned(),
                    display_name: "Listener".to_owned(),
                    role: RoomRole::Guest,
                },
            ],
            playback: AuthoritativePlaybackState {
                revision: 1,
                current_track: Some(RoomTrackRef {
                    video_id: "abc".to_owned(),
                    title: "Raag Yaman".to_owned(),
                    artist: "Ensemble".to_owned(),
                    duration_ms: 200_000,
                    artwork_url: None,
                }),
                queue: Vec::new(),
                is_playing: true,
                anchor_position_ms: 0,
                anchor_server_ms: 0,
            },
            transport: RoomTransportKind::WebSocket,
            clock_offset_ms: 12,
            local_track_blocked: false,
            connected: true,
        }));
        let warn = signal(None);
        listen_together_screen(snap, warn, ListenTogetherCallbacks::default())
    }

    let mut active_app = Headless::start(EngineConfig::new(Root::of::<ActiveRoomRoot>())).unwrap();
    active_app.wait_for("Room RAAG-7K4M").unwrap();
    active_app.wait_for("Participants (2)").unwrap();
    active_app.wait_for("Vote Skip").unwrap();
}
