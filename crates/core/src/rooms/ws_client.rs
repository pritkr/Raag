//! WebSocket primary transport for Listen Together (`tokio-tungstenite` + `rustls`).

use futures_util::{SinkExt, StreamExt};
use resonance_contract::RoomWireMessage;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{self, Message},
};
use url::Url;

use crate::rooms::protocol::redact_room_secrets;

#[derive(Debug, Clone)]
pub enum WsTransportError {
    /// Server rejected the WebSocket upgrade with an HTTP status (e.g. 400, 403, 502).
    HttpRejected(u16),
    Connection(String),
    Protocol(String),
}

impl WsTransportError {
    pub fn should_fallback_to_long_poll(&self) -> bool {
        matches!(self, Self::HttpRejected(400 | 403 | 502 | 503))
    }
}

pub struct WsRoomConnection {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl WsRoomConnection {
    pub async fn connect(ws_url: &Url) -> Result<Self, WsTransportError> {
        let (stream, _resp) = connect_async(ws_url.as_str())
            .await
            .map_err(|err| match err {
                tungstenite::Error::Http(resp) => {
                    WsTransportError::HttpRejected(resp.status().as_u16())
                }
                other => WsTransportError::Connection(redact_room_secrets(&other.to_string())),
            })?;
        Ok(Self { stream })
    }

    pub async fn send(&mut self, msg: &RoomWireMessage) -> Result<(), WsTransportError> {
        let payload =
            serde_json::to_string(msg).map_err(|e| WsTransportError::Protocol(e.to_string()))?;
        self.stream
            .send(Message::Text(payload.into()))
            .await
            .map_err(|e| WsTransportError::Connection(redact_room_secrets(&e.to_string())))
    }

    pub async fn recv(&mut self) -> Option<Result<RoomWireMessage, WsTransportError>> {
        while let Some(frame) = self.stream.next().await {
            match frame {
                Ok(Message::Text(txt)) => {
                    let parsed = serde_json::from_str::<RoomWireMessage>(&txt)
                        .map_err(|e| WsTransportError::Protocol(e.to_string()));
                    return Some(parsed);
                }
                Ok(Message::Close(_)) => return None,
                Ok(
                    Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_),
                ) => {
                    continue;
                }
                Err(e) => {
                    return Some(Err(WsTransportError::Connection(redact_room_secrets(
                        &e.to_string(),
                    ))));
                }
            }
        }
        None
    }

    pub async fn close(&mut self) {
        let _ = self.stream.close(None).await;
    }
}
