//! The JSON error every identity refusal is rendered as.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn new(status: StatusCode, msg: impl Into<String>) -> ApiError {
        ApiError(status, msg.into())
    }

    pub fn unauthorized(msg: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, msg)
    }

    pub fn internal(msg: impl std::fmt::Display) -> ApiError {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, msg.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
