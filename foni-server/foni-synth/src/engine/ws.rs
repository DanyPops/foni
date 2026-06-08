use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};

use super::emotion::{
    current_intensity, detect_emotion, effective_weights, emotion_emoji, neutral_state,
    update_emotion_state, EmotionState,
};
use super::engine_config::FoniConfig;
use super::facade::{cache_key, new_shared_cache, PlayQueue, SharedCache};
use std::sync::atomic::Ordering;

use rand::Rng;

use super::lexicon;
use super::playback_buffer::PlaybackBuffer;
use super::stream::{drain_chunks, feed_delta, fresh_state, strip_markdown, StreamState};
use super::stress::{make_annotator, StressAnnotator};
use super::train_events;
use super::translator::{self, WordDiversifier};
use crate::state::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, app_state: AppState) {
    let (mut tx, mut rx) = socket.split();
    let mut stream_state = fresh_state();
    let mut emotion_state = neutral_state();
    let defaults = FoniConfig::default();
    let mut config = FoniConfig {
        dry_run: std::env::var("FONI_DRY_RUN")
            .map(|v| v == "1")
            .unwrap_or(false),
        ollama_url: std::env::var("FONI_OLLAMA_URL").unwrap_or(defaults.ollama_url.clone()),
        ollama_model: std::env::var("FONI_OLLAMA_MODEL").unwrap_or(defaults.ollama_model.clone()),
        ..defaults
    };
    if let Ok(mode) = std::env::var("FONI_STRESS") {
        use std::str::FromStr;
        config.stress_mode = super::stress::StressMode::from_str(&mode).unwrap_or_default();
    }
    if let Ok(backend) = std::env::var("FONI_TRANSLATE") {
        use std::str::FromStr;
        config.translate_backend =
            super::engine_config::TranslateBackend::from_str(&backend).unwrap_or_default();
    }
    let annotator: Box<dyn StressAnnotator> =
        make_annotator(&config.stress_mode, &config.ruaccent_url);
    let cache = new_shared_cache();
    let (play_queue, _play_handle) = PlayQueue::new();
    let mut buffer = PlaybackBuffer::new();
    let mut chunk_counter: usize = 0;
    let mut mat_diversifier = WordDiversifier::new();
    let mut interject_diversifier = WordDiversifier::new();
    // Timestamp (ms) of the last successful personality injection.
    // Shared across mat and interject — one cooldown clock for both.
    let mut last_injection_ms: f64 = 0.0;

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
                        &emotion_state,
                        &config,
                        &cache,
                        &play_queue,
                        &app_state,
                        &mut mat_diversifier,
                        &mut interject_diversifier,
                        &mut last_injection_ms,
                        &mut tx,
                        &mut buffer,
                        &mut chunk_counter,
                        annotator.as_ref(),
                    )
                    .await;
                }
            }
            "message_end" => {
                if !config.enabled {
                    stream_state = fresh_state();
                    chunk_counter = 0;
                    buffer = PlaybackBuffer::new();
                    continue;
                }
                let leftover = stream_state.buffer.trim().to_string();
                stream_state = fresh_state();
                if leftover.len() > 2 {
                    let idx = chunk_counter;
                    chunk_counter += 1;
                    process_chunk(
                        &leftover,
                        &emotion_state,
                        &config,
                        &cache,
                        &play_queue,
                        &app_state,
                        &mut mat_diversifier,
                        &mut interject_diversifier,
                        &mut last_injection_ms,
                        &mut tx,
                        &mut buffer,
                        idx,
                        annotator.as_ref(),
                    )
                    .await;
                }
                buffer.close(chunk_counter);
                emit_buffer_state(&buffer, &mut tx).await;
                chunk_counter = 0;
                buffer = PlaybackBuffer::new();
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
                    let _ = tx.send(Message::Text(reply.to_string())).await;
                }
            }
            "set_config" => {
                // dry_run can be toggled per-connection so tests don't need
                // the FONI_DRY_RUN env var (which is process-global).
                if let Some(dr) = msg["dry_run"].as_bool() {
                    config.dry_run = dr;
                }
                // lang: "en,ru" or "ru,ru" — sets input and output language.
                // Passing same code for both sides disables translation (useful in tests).
                if let Some(lang) = msg["lang"].as_str() {
                    use crate::engine::engine_config::Lang;
                    if let Some((inp, out)) = lang.split_once(',') {
                        if let Some(l) = Lang::from_code(inp) {
                            config.input_lang = l;
                        }
                        if let Some(l) = Lang::from_code(out) {
                            config.output_lang = l;
                        }
                    }
                }
                if let Some(enabled) = msg["enabled"].as_bool() {
                    let was_disabled = !config.enabled;
                    config.enabled = enabled;
                    if !enabled && !was_disabled {
                        // Muting: clear the play queue so in-flight audio stops.
                        play_queue.clear();
                        stream_state = fresh_state();
                        buffer = PlaybackBuffer::new();
                        chunk_counter = 0;
                        emit_buffer_state(&buffer, &mut tx).await;
                    }
                    if enabled && was_disabled {
                        // Drain any text that accumulated while disabled.
                        let result = drain_chunks(&stream_state.buffer);
                        stream_state.buffer = result.remainder;
                        for chunk in result.chunks {
                            let idx = chunk_counter;
                            chunk_counter += 1;
                            process_chunk(
                                &chunk,
                                &emotion_state,
                                &config,
                                &cache,
                                &play_queue,
                                &app_state,
                                &mut mat_diversifier,
                                &mut interject_diversifier,
                                &mut last_injection_ms,
                                &mut tx,
                                &mut buffer,
                                idx,
                                annotator.as_ref(),
                            )
                            .await;
                        }
                    }
                }
            }
            "prewarm" => {
                // Tell the client warming has started so it can show a status indicator.
                let _ = tx
                    .send(Message::Text(
                        serde_json::json!({"type": "prewarm_start"}).to_string(),
                    ))
                    .await;

                // Pick a short phrase and synthesize silently to heat the Modal GPU.
                // Result stored in cache — first real request of the same phrase is instant.
                let phrase = super::engine_config::PREWARM_RU
                    .first()
                    .copied()
                    .unwrap_or("Да.");
                let synth_result = app_state
                    .0
                    .synth
                    .synthesize(phrase, &config.rvc_model)
                    .await;

                match synth_result {
                    Ok(wav) => {
                        cache.put(cache_key(phrase, &config.rvc_model), wav).await;
                        tracing::info!("prewarm: complete");
                    }
                    Err(e) => tracing::warn!(error = %e, "prewarm: failed"),
                }

                let _ = tx
                    .send(Message::Text(
                        serde_json::json!({"type": "prewarm_done"}).to_string(),
                    ))
                    .await;
            }
            "reset" => {
                // Clear the play queue immediately — generation bump drops all
                // pending chunks without waiting for them to play.
                play_queue.clear();
                stream_state = fresh_state();
                emotion_state = neutral_state();
                mat_diversifier.reset();
                interject_diversifier.reset();
                buffer = PlaybackBuffer::new();
                chunk_counter = 0;
                emit_buffer_state(&buffer, &mut tx).await;
            }
            "parse_train_logs" => {
                if let Some(text) = msg["text"].as_str() {
                    let events = train_events::parse_log_batch(text);
                    for event in &events {
                        let reply = serde_json::json!({
                            "type": "train_event",
                            "data": event,
                        });
                        if tx.send(Message::Text(reply.to_string())).await.is_err() {
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn emit_buffer_state(buffer: &PlaybackBuffer, tx: &mut (impl SinkExt<Message> + Unpin)) {
    let snap = buffer.snapshot();
    let msg = serde_json::json!({
        "type": "buffer_state",
        "data": snap,
    });
    let _ = tx.send(Message::Text(msg.to_string())).await;
}

#[allow(clippy::too_many_arguments)]
async fn handle_delta(
    delta: &str,
    stream_state: &mut StreamState,
    emotion_state: &EmotionState,
    config: &FoniConfig,
    cache: &SharedCache,
    play_queue: &PlayQueue,
    app_state: &AppState,
    mat_div: &mut WordDiversifier,
    interject_div: &mut WordDiversifier,
    last_injection_ms: &mut f64,
    tx: &mut (impl SinkExt<Message> + Unpin),
    buffer: &mut PlaybackBuffer,
    chunk_counter: &mut usize,
    annotator: &dyn StressAnnotator,
) {
    feed_delta(stream_state, delta);
    // When disabled, accumulate in buffer but do not synthesize.
    if !config.enabled {
        return;
    }
    let result = drain_chunks(&stream_state.buffer);
    stream_state.buffer = result.remainder;
    for chunk in result.chunks {
        let idx = *chunk_counter;
        *chunk_counter += 1;
        process_chunk(
            &chunk,
            emotion_state,
            config,
            cache,
            play_queue,
            app_state,
            mat_div,
            interject_div,
            last_injection_ms,
            tx,
            buffer,
            idx,
            annotator,
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_chunk(
    chunk: &str,
    emotion_state: &EmotionState,
    config: &FoniConfig,
    cache: &SharedCache,
    play_queue: &PlayQueue,
    app_state: &AppState,
    mat_div: &mut WordDiversifier,
    interject_div: &mut WordDiversifier,
    last_injection_ms: &mut f64,
    tx: &mut (impl SinkExt<Message> + Unpin),
    buffer: &mut PlaybackBuffer,
    chunk_idx: usize,
    annotator: &dyn StressAnnotator,
) {
    let t_start = std::time::Instant::now();
    let clean = strip_markdown(chunk);
    if clean.len() <= 2 {
        return;
    }

    // Translate (skipped in dry_run or same-lang mode)
    let mut text = if config.dry_run || config.input_lang == config.output_lang {
        translator::apply_glossary(&clean)
    } else {
        use crate::engine::engine_config::TranslateBackend;
        let glossed = translator::apply_glossary(&clean);
        match config.translate_backend {
            TranslateBackend::Nllb => {
                translator::nllb_translate(&glossed, &config.nllb_url, "eng_Latn", "rus_Cyrl")
                    .await
                    .unwrap_or(glossed)
            }
            TranslateBackend::Ollama => translator::ollama_translate(
                &glossed,
                &config.ollama_url,
                &config.ollama_model,
                "en",
                "ru",
            )
            .await
            .unwrap_or(glossed),
        }
    };

    // Inject personality based on emotion state
    if !config.dry_run {
        let now = now_ms();
        let weights = effective_weights(emotion_state, now);
        let mat_prob = config.mat_prob * weights.mat_multiplier;
        let interject_prob = config.interject_prob * weights.interject_multiplier;
        let bias = Some(weights.word_bias);

        // Cooldown gate — shared clock for mat and interject.
        let cooled = (now - *last_injection_ms)
            >= config.mat_cooldown_ms.min(config.interject_cooldown_ms) as f64;

        let wants_mat = config.mat_enabled && mat_prob > 0.0 && cooled;
        let wants_interject = config.interject_enabled && interject_prob > 0.0 && cooled;
        let wants_personality = wants_mat || wants_interject;

        // Dice roll gates the LLM seeder — injection_dice = N means 1-in-N chance.
        let dice_pass = config.injection_dice <= 1
            || rand::thread_rng().gen_range(1..=config.injection_dice) == 1;

        let mut injected = false;
        if config.llm_commentary_enabled && wants_personality && dice_pass {
            let seed = lexicon::character_seed();
            match translator::ollama_commentary(
                &text,
                weights.word_bias,
                seed,
                &config.ollama_url,
                &config.ollama_model,
                config.llm_commentary_timeout_ms,
            )
            .await
            {
                Ok(commentary) => {
                    tracing::debug!(injection = %commentary.text, "llm_commentary: applied");
                    text = translator::apply_commentary(&text, &commentary);
                    *last_injection_ms = now;
                    injected = true;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "llm_commentary failed, using static injection");
                }
            }
        }

        if !injected {
            if wants_mat {
                let before = text.len();
                text = translator::inject_mat(&text, mat_prob, config.mat_stretch, mat_div, bias);
                if text.len() != before {
                    *last_injection_ms = now;
                    injected = true;
                }
            }
            if !injected && wants_interject {
                let before = text.len();
                text = translator::inject_interject(&text, interject_prob, interject_div, bias);
                if text.len() != before {
                    *last_injection_ms = now;
                }
            }
        }
    }

    let translated = annotator.annotate(&text);

    // In dry_run mode: skip synthesis and playback, just report what would be spoken
    if config.dry_run {
        use super::playback_buffer::AudioChunk;
        buffer.submit(AudioChunk {
            index: chunk_idx,
            samples: vec![],
            sample_rate: 24_000,
        });
        buffer.drain_next();
        emit_buffer_state(buffer, tx).await;
        let reply = serde_json::json!({"type": "speak", "text": translated});
        let _ = tx.send(Message::Text(reply.to_string())).await;
        return;
    }

    let key = cache_key(&translated, &config.rvc_model);

    if let Some(cached) = cache.get(&key).await {
        play_queue.enqueue(cached.clone()).await;
        use super::playback_buffer::AudioChunk;
        buffer.submit(AudioChunk {
            index: chunk_idx,
            samples: vec![], // placeholder — actual audio in play_queue
            sample_rate: 24_000,
        });
        buffer.drain_next();
        emit_buffer_state(buffer, tx).await;
        let reply = serde_json::json!({"type": "playing", "text": translated});
        let _ = tx.send(Message::Text(reply.to_string())).await;
        return;
    }

    let t_synth = std::time::Instant::now();
    match app_state
        .0
        .synth
        .synthesize(&translated, &config.rvc_model)
        .await
    {
        Ok(raw_wav) => {
            tracing::info!(
                synth_ms = t_synth.elapsed().as_millis() as u64,
                bytes = raw_wav.len(),
                "ws: synth done"
            );

            // Apply DSP chain (loudnorm, compression, EQ) using the server’s
            // configured defaults and reactive controller correction.
            let dsp_defaults = app_state.0.dsp_defaults.read().await.clone();
            let base_opts = crate::quality::dsp::SmoothingOptions::from(&dsp_defaults);
            let controller_enabled = app_state.0.controller_enabled.load(Ordering::Relaxed);
            let controller_cfg = app_state.0.controller_config.read().await.clone();
            let policy_arc = app_state.0.policy_engine.read().await.clone();

            let wav = tokio::task::spawn_blocking(move || {
                crate::wav::roundtrip(&raw_wav, |samples, sr| {
                    let opts = if controller_enabled {
                        let analysis = foni_analyse::analyse_fast(samples, sr);
                        if let Some(ref policy) = policy_arc {
                            if let Some((corrected, _)) =
                                policy.evaluate(&analysis, &base_opts, &controller_cfg)
                            {
                                corrected
                            } else {
                                crate::quality::dsp::controller::correct(
                                    &analysis,
                                    &base_opts,
                                    &controller_cfg,
                                )
                                .0
                            }
                        } else {
                            crate::quality::dsp::controller::correct(
                                &analysis,
                                &base_opts,
                                &controller_cfg,
                            )
                            .0
                        }
                    } else {
                        base_opts
                    };
                    *samples = crate::quality::dsp::apply(std::mem::take(samples), sr, &opts);
                })
                .unwrap_or(raw_wav)
            })
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "ws: dsp spawn_blocking failed, using raw audio");
                vec![]
            });

            cache.put(key, wav.clone()).await;
            play_queue.enqueue(wav).await;
            use super::playback_buffer::AudioChunk;
            buffer.submit(AudioChunk {
                index: chunk_idx,
                samples: vec![],
                sample_rate: 24_000,
            });
            buffer.drain_next();
            emit_buffer_state(buffer, tx).await;
            tracing::info!(
                total_ms = t_start.elapsed().as_millis() as u64,
                "ws: chunk complete"
            );
            let reply = serde_json::json!({"type": "playing", "text": translated});
            let _ = tx.send(Message::Text(reply.to_string())).await;
        }
        Err(e) => {
            tracing::warn!(synth_ms = t_synth.elapsed().as_millis() as u64, error = %e, "ws: synth failed");
            let reply = serde_json::json!({"type": "error", "msg": e});
            let _ = tx.send(Message::Text(reply.to_string())).await;
        }
    }
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}
