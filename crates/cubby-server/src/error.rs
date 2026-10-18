//! The JSON error every identity refusal is rendered as.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// The message of a refusal, kept on the response so the request log can say why.
#[derive(Clone)]
pub(crate) struct ErrorCause(pub(crate) String);

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
        let cause = ErrorCause(self.1.clone());
        let mut res = (self.0, Json(json!({"error": self.1}))).into_response();
        res.extensions_mut().insert(cause);
        res
    }
}
