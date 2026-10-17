//! The axum server: /healthz, identity (Access JWT for parents, bearer tokens for paired child
//! devices), the built SPA shell, and request-latency telemetry.

pub mod access;
mod error;
mod identity;
mod parent;
pub mod play;
mod ratelimit;

pub mod child;

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

pub use access::AccessConfig;

pub const DEFAULT_PORT: u16 = 8790;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub bind: String,
    pub port: u16,
    pub web_dir: PathBuf,
    /// `host:port` of an OTLP/HTTP receiver; empty disables the push.
    pub otlp_endpoint: String,
    /// Directory holding the family database.
    pub data_dir: PathBuf,
    /// Cloudflare Access settings; without them every parent route refuses.
    pub access: Option<AccessConfig>,
}

impl Config {
    /// Reads `CUBBY_BIND` (127.0.0.1), `CUBBY_PORT` (8790), `CUBBY_WEB_DIR` (web/build) and
    /// `CUBBY_OTLP_ENDPOINT` (empty), `CUBBY_DATA_DIR` (data) and `CUBBY_ACCESS_TEAM_DOMAIN` / `CUBBY_ACCESS_AUD` / `CUBBY_ACCESS_JWKS_URL`,
    /// which are set together or not at all.
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
        let var = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let access_vars = [
            "CUBBY_ACCESS_TEAM_DOMAIN",
            "CUBBY_ACCESS_AUD",
            "CUBBY_ACCESS_JWKS_URL",
        ];
        let access = match access_vars.map(var) {
            [None, None, None] => None,
            [Some(team_domain), Some(aud), Some(jwks_url)] => Some(AccessConfig {
                team_domain,
                aud,
                jwks_url,
            }),
            vals => {
                let missing: Vec<_> = access_vars
                    .iter()
                    .zip(&vals)
                    .filter(|(_, v)| v.is_none())
                    .map(|(n, _)| *n)
                    .collect();
                return Err(format!(
                    "Cloudflare Access is partly configured; also set {}",
                    missing.join(", ")
                ));
            }
        };
        Ok(Config {
            access,
            data_dir: get("CUBBY_DATA_DIR")
                .unwrap_or_else(|| "data".into())
                .into(),
            bind: get("CUBBY_BIND").unwrap_or_else(|| "127.0.0.1".into()),
            port,
            web_dir: get("CUBBY_WEB_DIR")
                .unwrap_or_else(|| "web/build".into())
                .into(),
            otlp_endpoint: get("CUBBY_OTLP_ENDPOINT").unwrap_or_default(),
        })
    }
}

/// Shared by every handler and middleware.
pub(crate) struct AppState {
    pub registry: std::sync::Mutex<cubby_db::Registry>,
    pub access: Option<access::AccessState>,
    pub sink: Arc<MetricsSink>,
    pub pair_limiter: ratelimit::RateLimiter,
}

impl AppState {
    /// Fire-and-forget counter push; never blocks or fails the request.
    pub(crate) fn count(self: &Arc<Self>, name: &'static str, outcome: &'static str) {
        if self.sink.enabled() {
            let state = self.clone();
            tokio::task::spawn_blocking(move || {
                state.sink.push_count(name, &[("outcome", outcome)])
            });
        }
    }
}

/// Fetches the Access JWKS once so a wrong URL fails the start; a no-op without Access.
pub async fn check_access(config: &Config) -> Result<(), String> {
    match &config.access {
        Some(cfg) => access::AccessState::new(cfg).prime().await,
        None => Ok(()),
    }
}

pub fn app(config: &Config) -> Result<Router, String> {
    let registry = cubby_db::Registry::open(&config.data_dir)
        .map_err(|e| format!("open registry in {}: {e}", config.data_dir.display()))?;
    let sink = Arc::new(MetricsSink::new(
        Some(&config.otlp_endpoint),
        "cubby-server",
    ));
    let state = Arc::new(AppState {
        registry: std::sync::Mutex::new(registry),
        access: config.access.as_ref().map(access::AccessState::new),
        sink: sink.clone(),
        pair_limiter: ratelimit::RateLimiter::new(5, 60),
    });
    let spa =
        ServeDir::new(&config.web_dir).fallback(ServeFile::new(config.web_dir.join("index.html")));
    Ok(Router::new()
        .route("/healthz", get(healthz))
        .merge(identity::routes(state))
        .fallback_service(spa)
        .layer(middleware::from_fn_with_state(sink, record_latency)))
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
