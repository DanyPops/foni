use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;

use super::emotion::{
    current_intensity, detect_emotion, emotion_emoji, neutral_state, update_emotion_state,
    EmotionState,
};
use super::engine_config::FoniConfig;
use super::facade::{cache_key, new_shared_cache, PlayQueue, SharedCache};
use super::stream::{drain_chunks, feed_delta, fresh_state, strip_markdown, StreamState};
use super::translator;
use crate::state::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, app_state: AppState) {
    let (mut tx, mut rx) = socket.split();
    let mut stream_state = fresh_state();
    let mut emotion_state = neutral_state();
    let config = FoniConfig::default();
    let cache = new_shared_cache();
    let (play_queue, _play_handle) = PlayQueue::new();

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
                    handle_delta(
                        delta,
                        &mut stream_state,
                        &config,
                        &cache,
                        &play_queue,
                        &app_state,
                        &mut tx,
                    )
                    .await;
                }
            }
            "message_end" => {
                let leftover = stream_state.buffer.trim().to_string();
                stream_state = fresh_state();
                if leftover.len() > 2 {
                    process_chunk(&leftover, &config, &cache, &play_queue, &app_state, &mut tx)
                        .await;
                }
            }
            "user_message" => {
                if let Some(text) = msg["text"].as_str() {
                    let now = now_ms();
                    let reading = detect_emotion(text);
                    emotion_state = update_emotion_state(&emotion_state, &reading, now);
                    let intensity = current_intensity(&emotion_state, now);
                    let reply = serde_json::json!({
                        "type": "emotion",
                        "emotion": emotion_state.emotion,
                        "emoji": emotion_emoji(emotion_state.emotion),
                        "intensity": intensity,
                        "signals": reading.signals,
                    });
                    let _ = tx.send(Message::Text(reply.to_string().into())).await;
                }
            }
            "reset" => {
                stream_state = fresh_state();
                emotion_state = neutral_state();
            }
            _ => {}
        }
    }
}

async fn handle_delta(
    delta: &str,
    stream_state: &mut StreamState,
    config: &FoniConfig,
    cache: &SharedCache,
    play_queue: &PlayQueue,
    app_state: &AppState,
    tx: &mut (impl SinkExt<Message> + Unpin),
) {
    feed_delta(stream_state, delta);
    let result = drain_chunks(&stream_state.buffer);
    stream_state.buffer = result.remainder;
    for chunk in result.chunks {
        process_chunk(&chunk, config, cache, play_queue, app_state, tx).await;
    }
}

async fn process_chunk(
    chunk: &str,
    config: &FoniConfig,
    cache: &SharedCache,
    play_queue: &PlayQueue,
    _app_state: &AppState,
    tx: &mut (impl SinkExt<Message> + Unpin),
) {
    let clean = strip_markdown(chunk);
    if clean.len() <= 2 {
        return;
    }

    let key = cache_key(&clean, &config.rvc_model);

    if let Some(cached) = cache.get(&key).await {
        play_queue.enqueue(cached).await;
        let reply = serde_json::json!({"type": "playing", "text": clean});
        let _ = tx.send(Message::Text(reply.to_string().into())).await;
        return;
    }

    let translated = if config.input_lang != config.output_lang {
        let glossed = translator::apply_glossary(&clean);
        match translator::ollama_translate(
            &glossed,
            &config.ollama_url,
            &config.ollama_model,
            "en",
            "ru",
        )
        .await
        {
            Ok(t) => t,
            Err(_) => glossed,
        }
    } else {
        clean.clone()
    };

    let reply = serde_json::json!({"type": "speak", "text": translated});
    let _ = tx.send(Message::Text(reply.to_string().into())).await;
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}
