//! Standalone self-hostable `resonance-room-relay` binary (`H2.3`).

use std::net::SocketAddr;

use resonance_room_relay::{RelayConfig, spawn_relay};

#[tokio::main]
async fn main() {
    env_logger::init();
    let addr: SocketAddr = std::env::var("RESONANCE_RELAY_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8787".to_owned())
        .parse()
        .expect("valid RESONANCE_RELAY_ADDR");

    let handle = spawn_relay(addr, RelayConfig::default())
        .await
        .expect("start resonance-room-relay");
    log::info!("resonance-room-relay listening on {}", handle.addr());
    let _ = tokio::signal::ctrl_c().await;
    handle.shutdown();
}
