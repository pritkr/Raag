//! Phase 1–3 Unit & Protocol Snapshot Verification Suite:
//! - InnerTube `WEB_REMIX` search/home/next/lyrics & `VISIONOS`/`ANDROID_VR` player parsers
//! - Strict per-client auth isolation (`ANDROID_VR`/`TV_SAMSUNG` reject browser cookies)
//! - `visitorData` SQLite persistence
//! - `SAPISIDHASH` deterministic hashing & `Set-Cookie` rotation merge
//! - Android WebView `; wv` User-Agent sanitization
//! - Native QuickJS-NG (`rquickjs >= 0.12.0`) `solveN` + `solveSig` + `solve_cipher_url` + disk cache
//! - LRC synced lyrics parser + `active_line_index` + SponsorBlock SHA-256 prefix & duration fit
//! - `PlayerState` state machine valid/invalid transitions

use resonance_contract::{
    AuthError, AuthSession, AuthStore, CacheStore, HomeItem, LyricSyncKind, LyricsDocument,
    PlayerState,
};
use resonance_core::cache::SqliteCacheStore;
use resonance_core::innertube::auth::{
    apply_cookie_rotation, build_auth_headers, compute_sapisidhash, sanitize_android_webview_ua,
};
use resonance_core::innertube::clients::{AuthMode, ClientConfig, ClientKind};
use resonance_core::innertube::parser::{
    parse_home_response, parse_next_queue_response, parse_player_response, parse_search_response,
    parse_ytm_lyrics_response,
};
use resonance_core::lyrics::parse_lrc;
use resonance_core::solver::{ChallengeInput, QuickJsSolver};
use resonance_core::sponsorblock::{HttpSponsorBlockProvider, parse_sponsorblock_json};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn strict_per_client_auth_isolation_blocks_android_vr_and_tv_samsung() {
    let session = AuthSession {
        cookie_header: "SAPISID=test_sid_123; __Secure-3PAPISID=test_3p_456".into(),
        auth_user: 0,
        page_id: None,
        account_name: Some("Tester".into()),
        is_premium: false,
        visitor_data: None,
    };

    // WEB_REMIX and VISIONOS allow browser cookie auth
    let web_cfg = ClientConfig::for_kind(ClientKind::WebRemix);
    assert_eq!(web_cfg.auth_mode, AuthMode::CookiesOrAnonymous);
    let headers = build_auth_headers(web_cfg, &session, 1_700_000_000).unwrap();
    assert!(
        headers
            .iter()
            .any(|(k, v)| *k == "Authorization" && v.contains("SAPISIDHASH"))
    );

    let vision_cfg = ClientConfig::for_kind(ClientKind::VisionOs);
    assert!(build_auth_headers(vision_cfg, &session, 1_700_000_000).is_ok());

    // ANDROID_VR MUST reject browser cookies
    let vr_cfg = ClientConfig::for_kind(ClientKind::AndroidVr);
    assert_eq!(vr_cfg.auth_mode, AuthMode::AnonymousOnly);
    let vr_err = build_auth_headers(vr_cfg, &session, 1_700_000_000).unwrap_err();
    assert!(matches!(vr_err, AuthError::IsolationViolation(_)));

    // TV_SAMSUNG MUST reject browser cookies (TV OAuth only)
    let tv_cfg = ClientConfig::for_kind(ClientKind::TvSamsung);
    assert_eq!(tv_cfg.auth_mode, AuthMode::TvOAuthOnly);
    let tv_err = build_auth_headers(tv_cfg, &session, 1_700_000_000).unwrap_err();
    assert!(matches!(tv_err, AuthError::IsolationViolation(_)));
}

#[test]
fn sapisidhash_and_cookie_rotation_and_webview_ua_stripping() {
    let cookie = "SID=abc; SAPISID=my_secret_sid; __Secure-3PAPISID=my_3p_sid";
    let h1 = compute_sapisidhash(cookie, "https://music.youtube.com", 1_700_000_000).unwrap();
    let h2 = compute_sapisidhash(cookie, "https://music.youtube.com", 1_700_000_000).unwrap();
    assert_eq!(h1, h2);
    assert!(h1.contains("SAPISIDHASH 1700000000_"));
    assert!(h1.contains("SAPISID3PHASH 1700000000_"));

    let session = AuthSession {
        cookie_header: cookie.into(),
        auth_user: 0,
        page_id: None,
        account_name: None,
        is_premium: false,
        visitor_data: None,
    };
    let rotated = apply_cookie_rotation(
        &session,
        [
            "SID=rotated_val; Path=/; Secure; HttpOnly",
            "SAPISID=deleted; Max-Age=0",
        ],
    )
    .expect("rotation should merge new SID while ignoring deletion cookie");
    assert!(rotated.cookie_header.contains("SID=rotated_val"));
    assert!(rotated.cookie_header.contains("SAPISID=my_secret_sid"));

    // Android WebView `; wv` stripping prevents Google's 403 disallowed_useragent
    let raw_wv = "Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/AP1A; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/131.0.6778.100 Mobile Safari/537.36";
    let clean = sanitize_android_webview_ua(raw_wv);
    assert!(!clean.contains("; wv"), "must strip '; wv': {clean}");
    assert!(!clean.contains("Version/4.0"), "must strip Version/4.0: {clean}");
    assert!(clean.contains("Chrome/131.0.6778.100"));
}

#[test]
fn visitor_data_and_auth_store_persist_across_restarts() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    {
        let db = SqliteCacheStore::open(&db_path).unwrap();
        assert!(db.get_visitor_data().unwrap().is_none());
        db.set_visitor_data("Cgt2aXNpdG9yX3Rlc3Q%3D").unwrap();
    }
    {
        let db = SqliteCacheStore::open(&db_path).unwrap();
        assert_eq!(
            db.get_visitor_data().unwrap().as_deref(),
            Some("Cgt2aXNpdG9yX3Rlc3Q%3D")
        );
    }

    let store = resonance_core::innertube::FileAuthStore::new(dir.path());
    assert!(store.load_session().unwrap().is_none());
    let sess = AuthSession {
        cookie_header: "SAPISID=xyz".into(),
        auth_user: 0,
        page_id: Some("102938".into()),
        account_name: Some("Resonance User".into()),
        is_premium: true,
        visitor_data: Some("Cgt2aXNpdG9y".into()),
    };
    store.save_session(&sess).unwrap();
    let loaded = store.load_session().unwrap().unwrap();
    assert_eq!(loaded.cookie_header, "SAPISID=xyz");
    assert_eq!(loaded.page_id.as_deref(), Some("102938"));
    store.clear_session().unwrap();
    assert!(store.load_session().unwrap().is_none());
}

#[test]
fn innertube_json_snapshot_parsers_cover_search_home_next_player_and_lyrics() {
    // 1. WEB_REMIX search snapshot
    let search_json = json!({
        "contents": {
            "tabbedSearchResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "contents": [{
                                    "musicShelfRenderer": {
                                        "contents": [{
                                            "musicResponsiveListItemRenderer": {
                                                "playlistItemData": { "videoId": "dQw4w9WgXcQ" },
                                                "thumbnail": {
                                                    "musicThumbnailRenderer": {
                                                        "thumbnail": {
                                                            "thumbnails": [{ "url": "https://lh3.googleusercontent.com/art=w544-h544" }]
                                                        }
                                                    }
                                                },
                                                "flexColumns": [
                                                    {
                                                        "musicResponsiveListItemFlexColumnRenderer": {
                                                            "text": { "runs": [{ "text": "Midnight Synthesis" }] }
                                                        }
                                                    },
                                                    {
                                                        "musicResponsiveListItemFlexColumnRenderer": {
                                                            "text": {
                                                                "runs": [
                                                                    { "text": "Song" },
                                                                    { "text": " • " },
                                                                    { "text": "Aether Waves", "navigationEndpoint": { "browseEndpoint": { "browseId": "UC123" } } },
                                                                    { "text": " • " },
                                                                    { "text": "3:45" }
                                                                ]
                                                            }
                                                        }
                                                    }
                                                ]
                                            }
                                        }]
                                    }
                                }]
                            }
                        }
                    }
                }]
            }
        }
    });
    let res = parse_search_response("midnight", &search_json).unwrap();
    assert_eq!(res.tracks.len(), 1);
    assert_eq!(res.tracks[0].id, "dQw4w9WgXcQ");
    assert_eq!(res.tracks[0].title, "Midnight Synthesis");
    assert_eq!(res.tracks[0].artist_display(), "Aether Waves");
    assert_eq!(res.tracks[0].duration_ms, 225_000);

    // 2. Player response prefers highest quality Opus itag 251 over lower bitrate MP4
    let player_json = json!({
        "playabilityStatus": { "status": "OK" },
        "streamingData": {
            "expiresInSeconds": "21540",
            "adaptiveFormats": [
                {
                    "itag": 140,
                    "url": "https://rr1---sn.googlevideo.com/videoplayback?itag=140",
                    "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"",
                    "bitrate": 129000,
                    "contentLength": "3200000",
                    "approxDurationMs": "200000"
                },
                {
                    "itag": 251,
                    "url": "https://rr1---sn.googlevideo.com/videoplayback?itag=251",
                    "mimeType": "audio/webm; codecs=\"opus\"",
                    "bitrate": 160000,
                    "contentLength": "4100000",
                    "approxDurationMs": "200000"
                }
            ]
        }
    });
    let raw = parse_player_response("dQw4w9WgXcQ", "VISIONOS", &player_json, 1_700_000_000).unwrap();
    assert!(raw.signature_cipher.is_none());
    assert!(raw.url.is_some());
    assert_eq!(raw.itag, 251);
    assert_eq!(raw.content_length, 4_100_000);

    // 3. Home browse snapshot
    let home_json = json!({
        "contents": {
            "singleColumnBrowseResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "contents": [{
                                    "musicCarouselShelfRenderer": {
                                        "header": {
                                            "musicCarouselShelfBasicHeaderRenderer": {
                                                "title": { "runs": [{ "text": "Quick picks" }] }
                                            }
                                        },
                                        "contents": [{
                                            "musicTwoRowItemRenderer": {
                                                "title": { "runs": [{ "text": "Tokyo Night" }] },
                                                "subtitle": { "runs": [{ "text": "Haruka" }] },
                                                "navigationEndpoint": { "watchEndpoint": { "videoId": "tk01" } }
                                            }
                                        }]
                                    }
                                }]
                            }
                        }
                    }
                }]
            }
        }
    });
    let sections = parse_home_response(&home_json).unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].title, "Quick picks");
    assert!(matches!(
        &sections[0].items[0],
        HomeItem::Track(t) if t.id == "tk01"
    ));

    // 4. Autoplay next snapshot
    let next_json = json!({
        "contents": {
            "singleColumnMusicWatchNextResultsRenderer": {
                "tabbedRenderer": {
                    "watchNextTabbedResultsRenderer": {
                        "tabs": [{
                            "tabRenderer": {
                                "content": {
                                    "musicQueueRenderer": {
                                        "content": {
                                            "playlistPanelRenderer": {
                                                "contents": [{
                                                    "playlistPanelVideoRenderer": {
                                                        "videoId": "rad01",
                                                        "title": { "runs": [{ "text": "Radio Cut" }] },
                                                        "shortBylineText": { "runs": [{ "text": "Signal FM" }] },
                                                        "lengthText": { "runs": [{ "text": "4:02" }] }
                                                    }
                                                }]
                                            }
                                        }
                                    }
                                }
                            }
                        }]
                    }
                }
            }
        }
    });
    let radio = parse_next_queue_response(&next_json).unwrap();
    assert_eq!(radio.len(), 1);
    assert_eq!(radio[0].id, "rad01");
    assert_eq!(radio[0].duration_ms, 242_000);

    // 5. YTM internal lyrics snapshot
    let lyrics_json = json!({
        "contents": {
            "sectionListRenderer": {
                "contents": [{
                    "musicDescriptionShelfRenderer": {
                        "description": { "runs": [{ "text": "Line one\nLine two" }] }
                    }
                }]
            }
        }
    });
    let doc = parse_ytm_lyrics_response("rad01", &lyrics_json).unwrap();
    assert_eq!(doc.lines.len(), 2);
    assert_eq!(doc.sync_kind, LyricSyncKind::Plain);
}

#[tokio::test]
async fn native_rquickjs_solver_solves_n_and_sig_and_caches_by_player_hash() {
    let dir = tempdir().unwrap();
    let solver = QuickJsSolver::new(dir.path());

    let solver_js = r#"
        function solveN(n) { return n.split('').reverse().join('') + '_ok'; }
        function solveSig(s) { return 'SIG_' + s.toUpperCase(); }
    "#;

    let out = solver
        .solve(ChallengeInput {
            player_hash: "player_9a8b7c".into(),
            solver_js: solver_js.into(),
            n_param: Some("abc123".into()),
            sig_param: Some("xyz".into()),
        })
        .await
        .unwrap();

    assert_eq!(out.n_param.as_deref(), Some("321cba_ok"));
    assert_eq!(out.sig_param.as_deref(), Some("SIG_XYZ"));
    assert!(dir.path().join("js_solver/player_9a8b7c.js").exists());

    let cipher = "url=https%3A%2F%2Frr1.googlevideo.com%2Fvideoplayback%3Fitag%3D251%26n%3Dhello&s=tok&sp=sig";
    let solved_url = solver
        .solve_cipher_url("player_9a8b7c", solver_js, cipher)
        .await
        .unwrap();
    assert!(solved_url.contains("n=olleh_ok"), "solved_url={solved_url}");
    assert!(solved_url.contains("sig=SIG_TOK"), "solved_url={solved_url}");
}

#[test]
fn lrc_parser_and_sponsorblock_prefix_and_player_state_machine() {
    let lrc = "[00:05.50] First line\n[00:12.00] Second line\n[00:20.250] Third line";
    let lines = parse_lrc(lrc);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].start_ms, 5_500);
    assert_eq!(lines[1].start_ms, 12_000);
    assert_eq!(lines[2].start_ms, 20_250);

    let doc = LyricsDocument {
        video_id: "vid1".into(),
        provider: "LRCLIB".into(),
        sync_kind: LyricSyncKind::Line,
        lines,
    };
    assert_eq!(doc.active_line_index(13_000), Some(1));
    assert_eq!(doc.active_line_index(25_000), Some(2));

    let prefix = HttpSponsorBlockProvider::hash_prefix("dQw4w9WgXcQ");
    assert_eq!(prefix.len(), 4);
    let sb_json = r#"[{"videoID":"dQw4w9WgXcQ","segments":[{"segment":[0.0,18.5],"actionType":"skip","videoDuration":212.0}]}]"#;
    let segs = parse_sponsorblock_json("dQw4w9WgXcQ", sb_json);
    assert_eq!(segs.len(), 1);
    assert!(segs[0].fits_duration(212.0));
    assert!(segs[0].contains_ms(10_000));
    assert!(!segs[0].contains_ms(20_000));

    // Deterministic PlayerState transitions
    let mut state = PlayerState::Idle;
    assert!(state.transition(PlayerState::Playing).is_err());
    state.transition(PlayerState::Loading).unwrap();
    state.transition(PlayerState::Buffering).unwrap();
    state.transition(PlayerState::Playing).unwrap();
    state.transition(PlayerState::Paused).unwrap();
    state.transition(PlayerState::Recovering).unwrap();
    state.transition(PlayerState::Playing).unwrap();
}
