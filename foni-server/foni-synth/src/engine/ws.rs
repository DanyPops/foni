use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};

use super::stream::{drain_chunks, feed_delta, fresh_state, strip_markdown};
use crate::state::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(_state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let mut stream_state = fresh_state();

    while let Some(Ok(msg)) = rx.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };

        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };

        let msg_type = msg["type"].as_str().unwrap_or("");

        match msg_type {
            "delta" => {
                if let Some(delta) = msg["text"].as_str() {
                    feed_delta(&mut stream_state, delta);
                    let result = drain_chunks(&stream_state.buffer);
                    stream_state.buffer = result.remainder;
                    for chunk in result.chunks {
                        let clean = strip_markdown(&chunk);
                        if clean.len() > 2 {
                            let reply = serde_json::json!({"type": "speak", "text": clean});
                            if tx
                                .send(Message::Text(reply.to_string().into()))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
            }
            "message_end" => {
                let leftover = stream_state.buffer.trim().to_string();
                stream_state = fresh_state();
                if leftover.len() > 2 {
                    let clean = strip_markdown(&leftover);
                    if clean.len() > 2 {
                        let reply = serde_json::json!({"type": "speak", "text": clean});
                        if tx
                            .send(Message::Text(reply.to_string().into()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            "reset" => {
                stream_state = fresh_state();
            }
            _ => {}
        }
    }
}
