//! GET /ssml-params — canonical SSML break durations (ms) per punctuation type.
//!
//! TypeScript prosody.ts reads this endpoint at first use so both sides share
//! the same values. The constants below must stay in sync with ssml.rs.
use axum::Json;
use serde_json::{json, Value};

pub async fn get_ssml_params() -> Json<Value> {
    Json(json!({
        "comma":       crate::ssml::BREAK_COMMA_MS,
        "semicolon":   crate::ssml::BREAK_SEMICOLON_MS,
        "colon":       crate::ssml::BREAK_COLON_MS,
        "dash":        crate::ssml::BREAK_DASH_MS,
        "ellipsis":    crate::ssml::BREAK_ELLIPSIS_MS,
        "period":      crate::ssml::BREAK_PERIOD_MS,
        "exclamation": crate::ssml::BREAK_EXCLAIM_MS,
        "question":    crate::ssml::BREAK_QUESTION_MS,
    }))
}
