//! Resonance Desktop Entrypoint.
//!
//! Supports:
//! - `--fixture`: seeds deterministic QA dataset (74-char title, CJK+emoji, missing-artwork
//!   monogram fallback, synced karaoke lyrics, SponsorBlock segment)
//! - `--self-check`: starts real Tokio runtime, SQLite WAL cache, 256 KB ChunkCache,
//!   LoopbackProxy (`127.0.0.1`), `PlaybackController`, and `NshAudioBackend`, logs
//!   cold-launch latency, and exits cleanly.

use std::sync::Arc;
use std::time::Instant;

use resonance::platform::create_platform_audio_backend;
use resonance::store;
use resonance_contract::EventBus;
use resonance_core::cache::{ChunkCache, DEFAULT_MAX_AUDIO_CACHE_BYTES, SqliteCacheStore};
use resonance_core::innertube::{FileAuthStore, InnerTubeClient};
use resonance_core::lyrics::CascadeLyricsProvider;
use resonance_core::playback::PlaybackController;
use resonance_core::proxy::LoopbackProxy;
use resonance_core::solver::QuickJsSolver;
use resonance_core::sponsorblock::HttpSponsorBlockProvider;
use resonance_core::storage::resolve_app_paths;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let launch_start = Instant::now();

    let args: Vec<String> = std::env::args().collect();
    let fixture_mode = args.iter().any(|a| a == "--fixture");
    let self_check = args.iter().any(|a| a == "--self-check");

    if fixture_mode {
        store::enable_fixture_mode(true);
    }

    let paths = resolve_app_paths()?;
    let cache = Arc::new(SqliteCacheStore::open(&paths.data_dir)?);
    let chunks = Arc::new(ChunkCache::new(
        &paths.cache_dir.join("chunks"),
        cache.clone(),
        DEFAULT_MAX_AUDIO_CACHE_BYTES,
    )?);
    let auth = Arc::new(FileAuthStore::new(&paths.data_dir));
    let solver = Arc::new(QuickJsSolver::new(&paths.cache_dir.join("js_solver")));
    let provider = Arc::new(InnerTubeClient::new(cache.clone(), auth, solver)?);
    let proxy = Arc::new(LoopbackProxy::start(provider.clone(), chunks).await?);
    store::set_proxy_base_url(format!("http://{}", proxy.addr()));
    let audio = create_platform_audio_backend();
    let lyrics = Arc::new(CascadeLyricsProvider::new(provider.clone(), cache.clone()));
    let sponsorblock = Arc::new(HttpSponsorBlockProvider::new());
    let bus = EventBus::new(128);

    let controller = PlaybackController::new(
        provider,
        audio,
        cache,
        proxy.clone(),
        lyrics,
        sponsorblock,
        bus,
    );

    let snap = controller.snapshot().await;
    let elapsed_ms = launch_start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "Raag ready in {:.2} ms | proxy=http://{} | fixture={} | state={}",
        elapsed_ms,
        proxy.addr(),
        fixture_mode,
        snap.state.name()
    );

    if self_check {
        return Ok(());
    }

    println!("Press Ctrl+C to stop Raag engine.");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
