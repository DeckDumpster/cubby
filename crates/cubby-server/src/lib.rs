//! The axum server: /healthz, the built SPA shell, and request-latency telemetry.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use cubby_otel::MetricsSink;
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

pub const DEFAULT_PORT: u16 = 8790;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub bind: String,
    pub port: u16,
    pub web_dir: PathBuf,
    /// `host:port` of an OTLP/HTTP receiver; empty disables the push.
    pub otlp_endpoint: String,
}

impl Config {
    /// Reads `CUBBY_BIND` (127.0.0.1), `CUBBY_PORT` (8790), `CUBBY_WEB_DIR` (web/build) and
    /// `CUBBY_OTLP_ENDPOINT` (empty).
    pub fn from_env() -> Result<Config, String> {
        Config::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let port = match get("CUBBY_PORT") {
            Some(p) => p
                .trim()
                .parse()
                .map_err(|_| format!("CUBBY_PORT {p:?} is not a port"))?,
            None => DEFAULT_PORT,
        };
        Ok(Config {
            bind: get("CUBBY_BIND").unwrap_or_else(|| "127.0.0.1".into()),
            port,
            web_dir: get("CUBBY_WEB_DIR")
                .unwrap_or_else(|| "web/build".into())
                .into(),
            otlp_endpoint: get("CUBBY_OTLP_ENDPOINT").unwrap_or_default(),
        })
    }
}

pub fn app(config: &Config) -> Router {
    let sink = Arc::new(MetricsSink::new(
        Some(&config.otlp_endpoint),
        "cubby-server",
    ));
    let spa =
        ServeDir::new(&config.web_dir).fallback(ServeFile::new(config.web_dir.join("index.html")));
    Router::new()
        .route("/healthz", get(healthz))
        .fallback_service(spa)
        .layer(middleware::from_fn_with_state(sink, record_latency))
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"ok": true})))
}

async fn record_latency(
    State(sink): State<Arc<MetricsSink>>,
    req: Request,
    next: Next,
) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "spa".to_string(), |m| m.as_str().to_string());
    let start = Instant::now();
    let res = next.run(req).await;
    if sink.enabled() {
        let seconds = start.elapsed().as_secs_f64();
        let status = res.status().as_u16().to_string();
        tokio::task::spawn_blocking(move || {
            sink.push_seconds(
                "cubby_http_request_seconds",
                seconds,
                &[("route", &route), ("status", &status)],
            );
        });
    }
    res
}
