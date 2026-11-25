use std::sync::Arc;

use resonance_contract::MusicProvider;
use resonance_core::cache::{
    ArtworkCache, ChunkCache, DEFAULT_ARTWORK_DISK_CAP, DEFAULT_MAX_AUDIO_CACHE_BYTES,
    SqliteCacheStore,
};
use resonance_core::innertube::{FileAuthStore, InnerTubeClient};
use resonance_core::proxy::LoopbackProxy;
use resonance_core::solver::QuickJsSolver;
use tempfile::tempdir;

#[tokio::test]
#[ignore]
async fn live_smoke_home_explore_search_stream_and_artwork() {
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(
        ChunkCache::new(
            &dir.path().join("chunks"),
            cache.clone(),
            DEFAULT_MAX_AUDIO_CACHE_BYTES,
        )
        .unwrap(),
    );
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 32, DEFAULT_ARTWORK_DISK_CAP).unwrap());
    let auth = Arc::new(FileAuthStore::new(dir.path()));
    let solver = Arc::new(QuickJsSolver::new(dir.path()));
    let client = Arc::new(InnerTubeClient::new(cache.clone(), auth, solver).unwrap());

    let home = client.home().await.expect("home");
    println!("HOME sections: {}", home.len());
    for s in &home {
        println!("  home sec '{}' ({} items)", s.title, s.items.len());
    }
    assert!(!home.is_empty());

    let explore = client.explore().await.expect("explore");
    println!("EXPLORE sections: {}", explore.len());
    for s in &explore {
        println!("  explore sec '{}' ({} items)", s.title, s.items.len());
    }
    assert!(!explore.is_empty());

    let s_all = client
        .search_with_params("daft punk", None)
        .await
        .expect("search all");
    println!(
        "SEARCH ALL: {} tracks, {} albums",
        s_all.tracks.len(),
        s_all.albums.len()
    );
    let s_songs = client
        .search_with_params("daft punk", Some("EgWKAQIIAWoKEAMQBBAJEAoQBQ%3D%3D"))
        .await
        .expect("search songs");
    println!(
        "SEARCH SONGS: {} tracks, {} albums",
        s_songs.tracks.len(),
        s_songs.albums.len()
    );
    let s_albums = client
        .search_with_params("daft punk", Some("EgWKAQIYAWoKEAMQBBAJEAoQBQ%3D%3D"))
        .await
        .expect("search albums");
    println!(
        "SEARCH ALBUMS: {} tracks, {} albums",
        s_albums.tracks.len(),
        s_albums.albums.len()
    );
    assert!(!s_songs.tracks.is_empty());

    let first_track = &s_songs.tracks[0];
    println!(
        "TESTING STREAM FOR: {} ({})",
        first_track.title, first_track.id
    );
    let stream = client
        .resolve_stream(&first_track.id)
        .await
        .expect("resolve_stream");
    println!(
        "RESOLVED STREAM: client={} itag={} mime={} len={}",
        stream.resolved_by_client, stream.itag, stream.mime_type, stream.content_length
    );

    let proxy =
        LoopbackProxy::start_with_caches(client.clone(), chunks, Some(cache), Some(artwork))
            .await
            .unwrap();
    proxy.register_stream(stream.clone());

    let http = reqwest::Client::new();
    let stream_url = proxy.stream_url(&first_track.id, stream.itag);
    let r = http
        .get(&stream_url)
        .header("Range", "bytes=0-1023")
        .send()
        .await
        .unwrap();
    println!(
        "PROXY STREAM 206 status={} content-type={:?} content-range={:?}",
        r.status(),
        r.headers().get("content-type"),
        r.headers().get("content-range")
    );
    assert_eq!(r.status().as_u16(), 206);
    let bytes = r.bytes().await.unwrap();
    println!("PROXY STREAM bytes={}", bytes.len());
    assert_eq!(bytes.len(), 1024);

    if let Some(art_url) = &first_track.artwork_url {
        let enc: String = art_url.bytes().map(|b| format!("%{b:02X}")).collect();
        let art_proxy = format!("http://{}/artwork?url={enc}", proxy.addr());
        let ar = http.get(&art_proxy).send().await.unwrap();
        println!(
            "PROXY ARTWORK status={} content-type={:?}",
            ar.status(),
            ar.headers().get("content-type")
        );
        assert_eq!(ar.status().as_u16(), 200);
    }
}

#[test]
#[ignore]
fn live_ui_smoke_home_search_explore_and_play() -> Result<(), String> {
    use nsh_engine::{ColorScheme, EngineConfig, Environment, Points, Root, Size};
    use nsh_testing::Headless;

    resonance::store::enable_fixture_mode(false);
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(Points(1280.0), Points(800.0)),
        environment: Environment {
            color_scheme: ColorScheme::Dark,
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .expect("headless live engine starts cleanly");

    app.wait_for("MIXES & ALBUMS")?;
    println!(
        "LIVE UI HOME OK: {:?}",
        app.screen()
            .texts()
            .into_iter()
            .take(12)
            .collect::<Vec<_>>()
    );

    app.press("id:nav-search")?;
    app.wait_for("Search")?;
    app.type_text("id:search-input", "daft punk")?;
    app.wait_for("Daft Punk")?;
    println!("LIVE UI SEARCH ALL OK");

    app.press("id:search-filter-songs")?;
    app.wait_for("Instant Crush (feat. Julian Casablancas)")?;
    println!("LIVE UI SEARCH SONGS OK");

    app.press("id:search-filter-albums")?;
    app.wait_for("Albums & Collections")?;
    println!("LIVE UI SEARCH ALBUMS OK");

    app.press("Explore")?;
    app.wait_for("New releases for you")?;
    println!("LIVE UI EXPLORE OK");
    app.press("Charts")?;
    app.wait_for("Trending")?;
    println!("LIVE UI EXPLORE CHARTS FILTER OK");
    app.press("Moods & genres")?;
    app.wait_for("Chill")?;
    println!("LIVE UI EXPLORE MOODS FILTER OK");

    app.press("id:nav-search")?;
    app.press("id:search-filter-songs")?;
    app.wait_for("Instant Crush (feat. Julian Casablancas)")?;
    app.press("Instant Crush (feat. Julian Casablancas) >> nth=0")?;
    app.wait_for("0:00 / 5:38")
        .or_else(|_| app.wait_for("5:38"))?;
    println!("LIVE UI PLAY OK");

    app.press("Explore")?;
    app.wait_for("Chill")?;
    app.press("Chill")?;
    app.wait_for("00s Chill: Tamil")
        .or_else(|_| app.wait_for("Albums & Collections"))?;
    println!("LIVE UI EXPLORE GENRE -> SEARCH OK");

    Ok(())
}
