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
    let mut config = FoniConfig::default();
    config.dry_run = std::env::var("FONI_DRY_RUN")
        .map(|v| v == "1")
        .unwrap_or(false);
    if let Ok(url) = std::env::var("FONI_OLLAMA_URL") {
        config.ollama_url = url;
    }
    if let Ok(model) = std::env::var("FONI_OLLAMA_MODEL") {
        config.ollama_model = model;
    }
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

    // Translate (skip Ollama in dry_run or same-lang mode)
    let translated = if config.dry_run || config.input_lang == config.output_lang {
        translator::apply_glossary(&clean)
    } else {
        let glossed = translator::apply_glossary(&clean);
        translator::ollama_translate(
            &glossed,
            &config.ollama_url,
            &config.ollama_model,
            "en",
            "ru",
        )
        .await
        .unwrap_or(glossed)
    };

    // In dry_run mode: skip synthesis and playback, just report what would be spoken
    if config.dry_run {
        let reply = serde_json::json!({"type": "speak", "text": translated});
        let _ = tx.send(Message::Text(reply.to_string().into())).await;
        return;
    }

    let key = cache_key(&translated, &config.rvc_model);

    if let Some(cached) = cache.get(&key).await {
        play_queue.enqueue(cached).await;
        let reply = serde_json::json!({"type": "playing", "text": translated});
        let _ = tx.send(Message::Text(reply.to_string().into())).await;
        return;
    }

    let addr = std::env::var("FONI_SYNTH_ADDR").unwrap_or_else(|_| "0.0.0.0:5050".into());
    let synth_url = format!(
        "http://localhost:{}",
        addr.rsplit(':').next().unwrap_or("5050")
    );
    match synthesize_local(&synth_url, &translated, &config.rvc_model).await {
        Ok(wav) => {
            cache.put(key, wav.clone()).await;
            play_queue.enqueue(wav).await;
            let reply = serde_json::json!({"type": "playing", "text": translated});
            let _ = tx.send(Message::Text(reply.to_string().into())).await;
        }
        Err(e) => {
            tracing::warn!("synthesis failed: {e}");
            let reply = serde_json::json!({"type": "error", "msg": e});
            let _ = tx.send(Message::Text(reply.to_string().into())).await;
        }
    }
}

async fn synthesize_local(base_url: &str, text: &str, model: &str) -> Result<Vec<u8>, String> {
    let body = serde_json::json!({
        "text": text,
        "model": model,
        "voice": "ru",
        "speed": 150,
        "dsp": true,
        "prosody": true,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/synthesize"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("synthesize request: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| e.to_string())
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}
