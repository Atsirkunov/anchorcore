use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

pub async fn not_implemented() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"detail": "Not Implemented (Rust stub R1.3)"})),
    )
}
