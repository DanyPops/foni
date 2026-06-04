//! GET /ssml-params — canonical SSML break durations (ms) per punctuation type.
//!
//! TypeScript prosody.ts reads this endpoint at first use so both sides share
//! the same values. The constants below must stay in sync with ssml.rs.
use axum::Json;
use serde_json::{json, Value};

pub async fn get_ssml_params() -> Json<Value> {
    Json(json!({
        "comma":       super::ssml::BREAK_COMMA_MS,
        "semicolon":   super::ssml::BREAK_SEMICOLON_MS,
        "colon":       super::ssml::BREAK_COLON_MS,
        "dash":        super::ssml::BREAK_DASH_MS,
        "ellipsis":    super::ssml::BREAK_ELLIPSIS_MS,
        "period":      super::ssml::BREAK_PERIOD_MS,
        "exclamation": super::ssml::BREAK_EXCLAIM_MS,
        "question":    super::ssml::BREAK_QUESTION_MS,
    }))
}
