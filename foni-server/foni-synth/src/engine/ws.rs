use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use super::emotion::{
    current_intensity, detect_emotion, effective_weights, emotion_emoji, neutral_state,
    update_emotion_state, EmotionState,
};
use super::engine_config::FoniConfig;
use super::facade::{cache_key, new_shared_cache, PlayQueue, SharedCache};

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

// ── Internal types ────────────────────────────────────────────────────────────

/// A text chunk ready for synthesis, queued while a previous synthesis is in flight.
struct ChunkJob {
    text: String,
    cache_key: String,
    chunk_idx: usize,
    /// Generation at the time the chunk entered the queue.
    snap_gen: u64,
    /// Optional in-flight LLM commentary task started in parallel with synthesis.
    /// If it resolves to a Suffix before synthesis completes, the suffix text
    /// is synthesised as a mini audio clip appended after the main chunk.
    commentary_task: Option<tokio::task::JoinHandle<Result<translator::CommentaryResult, String>>>,
}

/// Result returned by a spawned synthesis task.
struct SynthResult {
    wav: Vec<u8>,
    /// Optional suffix clip synthesised from LLM commentary (plays after main wav).
    suffix_wav: Option<Vec<u8>>,
    chunk_idx: usize,
    text: String,
    snap_gen: u64,
    cache_key: String,
}

// ── Session state ────────────────────────────────────────────────────────────

/// Per-connection mutable state bundled into one struct.
///
/// Passed as `&mut SessionCtx` to `handle_delta` and `prepare_and_enqueue`
/// instead of 14 individual `&mut` parameters. Infrastructure handles
/// (`tx`, `cache`, `play_queue`, `app_state`, `synth_tx`) are kept separate
/// because they are used directly in the `handle_socket` main loop as well.
struct SessionCtx {
    stream_state: StreamState,
    emotion_state: EmotionState,
    config: FoniConfig,
    mat_diversifier: WordDiversifier,
    interject_diversifier: WordDiversifier,
    last_injection_ms: f64,
    buffer: PlaybackBuffer,
    chunk_counter: usize,
    annotator: Box<dyn StressAnnotator>,
    chunk_queue: VecDeque<ChunkJob>,
    stream_log: VecDeque<(usize, String)>,
}

// ── Socket handler ────────────────────────────────────────────────────────────

async fn handle_socket(socket: WebSocket, app_state: AppState) {
    let (mut tx, mut rx) = socket.split();
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
    // Build annotator before moving config into SessionCtx.
    let annotator = make_annotator(&config.stress_mode, &config.ruaccent_url);
    let cache = new_shared_cache();
    let (play_queue, _play_handle, mut played_rx) = PlayQueue::new();

    // Synthesis is offloaded to spawned tasks so the WS receive loop stays
    // responsive to control messages (reset, mute) during slow TTS calls.
    let (synth_tx, mut synth_rx) = tokio::sync::mpsc::channel::<Result<SynthResult, String>>(8);
    let mut synth_active = false;

    const STREAM_LOG_CAPACITY: usize = 20;
    let mut ctx = SessionCtx {
        stream_state: fresh_state(),
        emotion_state: neutral_state(),
        config,
        mat_diversifier: WordDiversifier::new(),
        interject_diversifier: WordDiversifier::new(),
        last_injection_ms: 0.0,
        buffer: PlaybackBuffer::new(),
        chunk_counter: 0,
        annotator,
        chunk_queue: VecDeque::new(),
        stream_log: VecDeque::with_capacity(STREAM_LOG_CAPACITY),
    };

    loop {
        tokio::select! {
            msg = rx.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => continue,
                };

                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };

                match msg["type"].as_str().unwrap_or("") {
                    "delta" => {
                        if let Some(delta) = msg["text"].as_str() {
                            handle_delta(
                                delta,
                                &mut ctx.stream_state,
                                &ctx.emotion_state,
                                &ctx.config,
                                &cache,
                                &play_queue,
                                &mut ctx.mat_diversifier,
                                &mut ctx.interject_diversifier,
                                &mut ctx.last_injection_ms,
                                &mut tx,
                                &mut ctx.buffer,
                                &mut ctx.chunk_counter,
                                ctx.annotator.as_ref(),
                                &mut ctx.chunk_queue,
                            )
                            .await;
                        }
                    }
                    "message_end" => {
                        if !ctx.config.enabled {
                            ctx.stream_state = fresh_state();
                            ctx.chunk_counter = 0;
                            ctx.buffer = PlaybackBuffer::new();
                            continue;
                        }
                        let leftover = ctx.stream_state.buffer.trim().to_string();
                        ctx.stream_state = fresh_state();
                        if leftover.len() > 2 {
                            let idx = ctx.chunk_counter;
                            ctx.chunk_counter += 1;
                            prepare_and_enqueue(
                                &leftover,
                                idx,
                                &ctx.emotion_state,
                                &ctx.config,
                                &cache,
                                &play_queue,
                                &mut ctx.mat_diversifier,
                                &mut ctx.interject_diversifier,
                                &mut ctx.last_injection_ms,
                                &mut tx,
                                &mut ctx.buffer,
                                ctx.annotator.as_ref(),
                                &mut ctx.chunk_queue,
                            )
                            .await;
                        }
                        ctx.buffer.close(ctx.chunk_counter);
                        emit_buffer_state(&ctx.buffer, &mut tx).await;
                        // Buffer is NOT reset here — it stays alive until the
                        // played_rx signal drains every submitted chunk. Only
                        // chunk_counter resets so the next message assigns
                        // fresh indices into this same buffer.
                        ctx.chunk_counter = 0;
                    }
                    "user_message" => {
                        if let Some(text) = msg["text"].as_str() {
                            let now = now_ms();
                            let reading = detect_emotion(text);
                            ctx.emotion_state =
                                update_emotion_state(&ctx.emotion_state, &reading, now);
                            let intensity = current_intensity(&ctx.emotion_state, now);
                            let reply = serde_json::json!({
                                "type": "emotion",
                                "emotion": ctx.emotion_state.emotion,
                                "emoji": emotion_emoji(ctx.emotion_state.emotion),
                                "intensity": intensity,
                                "signals": reading.signals,
                            });
                            let _ = tx.send(Message::Text(reply.to_string())).await;
                        }
                    }
                    "set_config" => {
                        if let Some(dr) = msg["dry_run"].as_bool() {
                            ctx.config.dry_run = dr;
                        }
                        if let Some(lang) = msg["lang"].as_str() {
                            use crate::engine::engine_config::Lang;
                            if let Some((inp, out)) = lang.split_once(',') {
                                if let Some(l) = Lang::from_code(inp) {
                                    ctx.config.input_lang = l;
                                }
                                if let Some(l) = Lang::from_code(out) {
                                    ctx.config.output_lang = l;
                                }
                            }
                        }
                        if let Some(enabled) = msg["enabled"].as_bool() {
                            let was_disabled = !ctx.config.enabled;
                            ctx.config.enabled = enabled;
                            if !enabled && !was_disabled {
                                play_queue.clear();
                                ctx.chunk_queue.clear();
                                synth_active = false;
                                ctx.stream_state = fresh_state();
                                ctx.buffer = PlaybackBuffer::new();
                                ctx.chunk_counter = 0;
                                emit_buffer_state(&ctx.buffer, &mut tx).await;
                            }
                            if enabled && was_disabled {
                                let result = drain_chunks(&ctx.stream_state.buffer);
                                ctx.stream_state.buffer = result.remainder;
                                for chunk in result.chunks {
                                    let idx = ctx.chunk_counter;
                                    ctx.chunk_counter += 1;
                                    prepare_and_enqueue(
                                        &chunk,
                                        idx,
                                        &ctx.emotion_state,
                                        &ctx.config,
                                        &cache,
                                        &play_queue,
                                        &mut ctx.mat_diversifier,
                                        &mut ctx.interject_diversifier,
                                        &mut ctx.last_injection_ms,
                                        &mut tx,
                                        &mut ctx.buffer,
                                        ctx.annotator.as_ref(),
                                        &mut ctx.chunk_queue,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    "prewarm" => {
                        if !ctx.config.enabled {
                            continue;
                        }
                        let _ = tx
                            .send(Message::Text(
                                serde_json::json!({"type": "prewarm_start"}).to_string(),
                            ))
                            .await;
                        let phrase = super::engine_config::PREWARM_RU
                            .first()
                            .copied()
                            .unwrap_or("Да.");
                        let synth_result = app_state
                            .0
                            .synth
                            .synthesize(phrase, &ctx.config.rvc_model)
                            .await;
                        match synth_result {
                            Ok(wav) => {
                                cache
                                    .put(cache_key(phrase, &ctx.config.rvc_model), wav)
                                    .await;
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
                    "resume" => {
                        // Client reconnected and wants to replay from a given chunk.
                        // Replay all logged chunks with chunk_id > last_seen.
                        let last_seen = msg["last_chunk_id"].as_u64().unwrap_or(0) as usize;
                        let replay: Vec<_> = ctx.stream_log
                            .iter()
                            .filter(|(idx, _)| *idx > last_seen)
                            .cloned()
                            .collect();
                        tracing::info!(
                            last_seen,
                            replaying = replay.len(),
                            "ws: resume requested"
                        );
                        for (chunk_id, text) in replay {
                            let reply = serde_json::json!({
                                "type": "playing",
                                "text": text,
                                "chunk_id": chunk_id,
                                "replayed": true,
                            });
                            let _ = tx.send(Message::Text(reply.to_string())).await;
                        }
                    }
                    "reset" => {
                        play_queue.clear();
                        ctx.chunk_queue.clear();
                        synth_active = false;
                        ctx.stream_state = fresh_state();
                        ctx.emotion_state = neutral_state();
                        ctx.mat_diversifier.reset();
                        ctx.interject_diversifier.reset();
                        ctx.buffer = PlaybackBuffer::new();
                        ctx.chunk_counter = 0;
                        ctx.stream_log.clear();
                        emit_buffer_state(&ctx.buffer, &mut tx).await;
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

                // Start synthesis for the next queued chunk if none is in flight.
                if !synth_active {
                    synth_active =
                        try_start_synthesis(&mut ctx.chunk_queue, &synth_tx, &play_queue, &app_state, &ctx.config);
                }
            }

            result = synth_rx.recv(), if synth_active => {
                synth_active = false;
                match result {
                    Some(Ok(r)) if r.snap_gen == play_queue.generation_snapshot() => {
                        cache.put(r.cache_key, r.wav.clone()).await;
                        // Enqueue with chunk_idx so the player task can signal
                        // playback completion back via played_rx.
                        play_queue
                            .enqueue_tagged(r.wav, r.snap_gen, r.chunk_idx)
                            .await;
                        // Suffix commentary clip (synthesised in parallel with main).
                        if let Some(sfx) = r.suffix_wav {
                            play_queue.enqueue_tagged(sfx, r.snap_gen, r.chunk_idx).await;
                        }
                        use super::playback_buffer::AudioChunk;
                        // submit only — drain_next is deferred to played_rx so
                        // the slot stays visible (█) until audio actually plays.
                        ctx.buffer.submit(AudioChunk {
                            index: r.chunk_idx,
                            samples: vec![],
                            sample_rate: 24_000,
                        });
                        emit_buffer_state(&ctx.buffer, &mut tx).await;
                        // Append to StreamLog ring buffer for replay on reconnect.
                        if ctx.stream_log.len() == STREAM_LOG_CAPACITY {
                            ctx.stream_log.pop_front();
                        }
                        ctx.stream_log.push_back((r.chunk_idx, r.text.clone()));

                        let reply = serde_json::json!({
                            "type": "playing",
                            "text": r.text,
                            "chunk_id": r.chunk_idx,
                        });
                        let _ = tx.send(Message::Text(reply.to_string())).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        let reply = serde_json::json!({"type": "error", "msg": e});
                        let _ = tx.send(Message::Text(reply.to_string())).await;
                    }
                    None => {}
                }
                // Advance to the next queued chunk.
                if !synth_active {
                    synth_active = try_start_synthesis(
                        &mut ctx.chunk_queue,
                        &synth_tx,
                        &play_queue,
                        &app_state,
                        &ctx.config,
                    );
                }
            }

            Some((played_gen, chunk_idx)) = played_rx.recv() => {
                // Ignore signals from a generation that has been superseded by
                // a reset — those chunks were skipped, not played.
                if played_gen == play_queue.generation_snapshot() {
                    ctx.buffer.drain_next();
                    emit_buffer_state(&ctx.buffer, &mut tx).await;
                    // Once every submitted chunk has been played the buffer is
                    // complete. Reset it so the next message starts clean.
                    if ctx.buffer.is_complete() {
                        ctx.buffer = PlaybackBuffer::new();
                        ctx.chunk_counter = 0;
                        emit_buffer_state(&ctx.buffer, &mut tx).await;
                    }
                }
                let _ = chunk_idx; // index carried for future per-chunk events
            }
        }
    }
}

// ── Synthesis task ────────────────────────────────────────────────────────────

fn try_start_synthesis(
    chunk_queue: &mut VecDeque<ChunkJob>,
    synth_tx: &tokio::sync::mpsc::Sender<Result<SynthResult, String>>,
    play_queue: &PlayQueue,
    app_state: &AppState,
    config: &FoniConfig,
) -> bool {
    while let Some(job) = chunk_queue.pop_front() {
        if job.snap_gen != play_queue.generation_snapshot() {
            continue;
        }
        let tx = synth_tx.clone();
        let app = app_state.clone();
        let cfg = config.clone();
        tokio::spawn(async move { synthesize_job(job, tx, app, cfg).await });
        return true;
    }
    false
}

async fn synthesize_job(
    job: ChunkJob,
    tx: tokio::sync::mpsc::Sender<Result<SynthResult, String>>,
    app_state: AppState,
    config: FoniConfig,
) {
    let t_synth = std::time::Instant::now();
    let raw_wav = match app_state
        .0
        .synth
        .synthesize(&job.text, &config.rvc_model)
        .await
    {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(synth_ms = t_synth.elapsed().as_millis() as u64, error = %e, "ws: synth failed");
            let _ = tx.send(Err(e)).await;
            return;
        }
    };
    tracing::info!(
        synth_ms = t_synth.elapsed().as_millis() as u64,
        bytes = raw_wav.len(),
        "ws: synth done"
    );

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
                    crate::quality::dsp::controller::correct(&analysis, &base_opts, &controller_cfg)
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

    // Await the LLM commentary task (started concurrently with synthesis).
    // By now Fish Speech has finished, so the timeout is effectively "whatever
    // arrived during synthesis" — no added latency on the main path.
    let suffix_wav = if let Some(task) = job.commentary_task {
        match tokio::time::timeout(std::time::Duration::from_millis(50), task).await {
            Ok(Ok(Ok(commentary))) => {
                // Commentary resolved to a Suffix — synthesise the short clip.
                let suffix_text = commentary.text.clone();
                tracing::debug!(suffix = %suffix_text, "commentary: synthesising suffix clip");
                match app_state
                    .0
                    .synth
                    .synthesize(&suffix_text, &config.rvc_model)
                    .await
                {
                    Ok(raw) => Some(raw),
                    Err(e) => {
                        tracing::debug!(error = %e, "commentary: suffix synth failed");
                        None
                    }
                }
            }
            _ => None, // timed out, task failed, or non-suffix — skip
        }
    } else {
        None
    };

    let _ = tx
        .send(Ok(SynthResult {
            wav,
            suffix_wav,
            chunk_idx: job.chunk_idx,
            text: job.text,
            snap_gen: job.snap_gen,
            cache_key: job.cache_key,
        }))
        .await;
}

// ── Text preparation ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn handle_delta(
    delta: &str,
    stream_state: &mut StreamState,
    emotion_state: &EmotionState,
    config: &FoniConfig,
    cache: &SharedCache,
    play_queue: &PlayQueue,
    mat_div: &mut WordDiversifier,
    interject_div: &mut WordDiversifier,
    last_injection_ms: &mut f64,
    tx: &mut (impl SinkExt<Message> + Unpin),
    buffer: &mut PlaybackBuffer,
    chunk_counter: &mut usize,
    annotator: &dyn StressAnnotator,
    chunk_queue: &mut VecDeque<ChunkJob>,
) {
    feed_delta(stream_state, delta);
    if !config.enabled {
        return;
    }
    let result = drain_chunks(&stream_state.buffer);
    stream_state.buffer = result.remainder;
    for chunk in result.chunks {
        let idx = *chunk_counter;
        *chunk_counter += 1;
        prepare_and_enqueue(
            &chunk,
            idx,
            emotion_state,
            config,
            cache,
            play_queue,
            mat_div,
            interject_div,
            last_injection_ms,
            tx,
            buffer,
            annotator,
            chunk_queue,
        )
        .await;
    }
}

/// Prepare one text chunk and either emit it immediately (dry_run / cache hit)
/// or push it to `chunk_queue` for background synthesis.
#[allow(clippy::too_many_arguments)]
async fn prepare_and_enqueue(
    chunk: &str,
    chunk_idx: usize,
    emotion_state: &EmotionState,
    config: &FoniConfig,
    cache: &SharedCache,
    play_queue: &PlayQueue,
    mat_div: &mut WordDiversifier,
    interject_div: &mut WordDiversifier,
    last_injection_ms: &mut f64,
    tx: &mut (impl SinkExt<Message> + Unpin),
    buffer: &mut PlaybackBuffer,
    annotator: &dyn StressAnnotator,
    chunk_queue: &mut VecDeque<ChunkJob>,
) {
    let clean = strip_markdown(chunk);
    if clean.len() <= 2 {
        return;
    }

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

    let commentary_dice_pass =
        config.injection_dice <= 1 || rand::thread_rng().gen_range(1..=config.injection_dice) == 1;

    if !config.dry_run {
        let now = now_ms();
        let weights = effective_weights(emotion_state, now);
        let mat_prob = config.mat_prob * weights.mat_multiplier;
        let interject_prob = config.interject_prob * weights.interject_multiplier;
        let bias = Some(weights.word_bias);
        let cooled = (now - *last_injection_ms)
            >= config.mat_cooldown_ms.min(config.interject_cooldown_ms) as f64;
        let wants_mat = config.mat_enabled && mat_prob > 0.0 && cooled;
        let wants_interject = config.interject_enabled && interject_prob > 0.0 && cooled;
        // Static injection (synchronous, no latency).
        let mut stat_injected = false;
        if wants_mat {
            let before = text.len();
            text = translator::inject_mat(&text, mat_prob, config.mat_stretch, mat_div, bias);
            if text.len() != before {
                *last_injection_ms = now;
                stat_injected = true;
            }
        }
        if !stat_injected && wants_interject {
            let before = text.len();
            text = translator::inject_interject(&text, interject_prob, interject_div, bias);
            if text.len() != before {
                *last_injection_ms = now;
            }
        }
    }

    let translated = annotator.annotate(&text);

    if config.dry_run {
        use super::playback_buffer::AudioChunk;
        buffer.submit(AudioChunk {
            index: chunk_idx,
            samples: vec![],
            sample_rate: 24_000,
        });
        // dry_run has no audio subprocess, so drain immediately.
        buffer.drain_next();
        emit_buffer_state(buffer, tx).await;
        let reply = serde_json::json!({"type": "speak", "text": translated});
        let _ = tx.send(Message::Text(reply.to_string())).await;
        return;
    }

    let key = cache_key(&translated, &config.rvc_model);

    if let Some(cached) = cache.get(&key).await {
        let snap = play_queue.generation_snapshot();
        play_queue.enqueue_tagged(cached, snap, chunk_idx).await;
        use super::playback_buffer::AudioChunk;
        // submit only — drain_next is deferred to played_rx.
        buffer.submit(AudioChunk {
            index: chunk_idx,
            samples: vec![],
            sample_rate: 24_000,
        });
        emit_buffer_state(buffer, tx).await;
        let reply = serde_json::json!({"type": "playing", "text": translated});
        let _ = tx.send(Message::Text(reply.to_string())).await;
        return;
    }

    // Spawn LLM commentary concurrently with synthesis.
    // It resolves inside synthesize_job (hidden by Fish Speech latency)
    // and appends a short suffix clip after the main audio.
    let commentary_task = if !config.dry_run
        && config.llm_commentary_enabled
        && commentary_dice_pass
        && translated.len() > 2
    {
        use foni_client::commentary::Placement;
        // `character_seed()` returns &'static — safe to move into spawn.
        let seed: &'static _ = lexicon::character_seed();
        let commentary_bias = {
            let now2 = now_ms();
            effective_weights(emotion_state, now2).word_bias
        };
        let emotion_str: &'static str = match commentary_bias {
            super::emotion::WordBias::Aggressive => "angry, aggressive",
            super::emotion::WordBias::Commiseration => "sympathetic, commiserating",
            super::emotion::WordBias::Mockery => "mocking, sarcastic",
            super::emotion::WordBias::Excitement => "excited, enthusiastic",
            super::emotion::WordBias::Neutral => "neutral",
        };
        let text_for_commentary = translated.clone();
        let url = config.ollama_url.clone();
        let model = config.ollama_model.clone();
        let timeout = config.llm_commentary_timeout_ms;
        let handle: tokio::task::JoinHandle<Result<_, String>> = tokio::spawn(async move {
            // Build refs inside the async block where seed is 'static.
            let exprs: Vec<&str> = seed.expressions.iter().map(String::as_str).collect();
            let client_seed = foni_client::commentary::CharacterSeed {
                persona: &seed.persona,
                expressions: &exprs,
            };
            let r = foni_client::commentary::ollama_commentary(
                &text_for_commentary,
                emotion_str,
                &client_seed,
                &url,
                &model,
                timeout,
            )
            .await?;
            if r.placement == Placement::Suffix {
                Ok(r)
            } else {
                Err("non-suffix commentary discarded in pipeline mode".into())
            }
        });
        Some(handle)
    } else {
        None
    };

    chunk_queue.push_back(ChunkJob {
        text: translated,
        cache_key: key,
        chunk_idx,
        snap_gen: play_queue.generation_snapshot(),
        commentary_task,
    });
}

async fn emit_buffer_state(buffer: &PlaybackBuffer, tx: &mut (impl SinkExt<Message> + Unpin)) {
    let snap = buffer.snapshot();
    let msg = serde_json::json!({
        "type": "buffer_state",
        "data": snap,
    });
    let _ = tx.send(Message::Text(msg.to_string())).await;
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}
