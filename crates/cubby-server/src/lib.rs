//! The axum server: /healthz, identity (Access JWT for parents, bearer tokens for paired child
//! devices), the built SPA shell, and request-latency telemetry.

pub mod access;
mod error;
pub mod family_cli;
mod identity;
mod parent;
pub mod play;
mod ratelimit;

pub mod child;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
    /// YouTube Data API key; with it, allowing a link ingests its metadata before the request
    /// returns. Empty leaves that to the worker's timer.
    pub yt_api_key: String,
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
            yt_api_key: get("CUBBY_YT_API_KEY")
                .unwrap_or_default()
                .trim()
                .to_string(),
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
    pub ingest: Option<Ingest>,
}

/// Runs one metadata pass over a family's database; called when a parent allows a link.
pub type Ingest = Arc<dyn Fn(&rusqlite::Connection) + Send + Sync>;

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
    let ingest: Option<Ingest> = (!config.yt_api_key.is_empty()).then(|| {
        let key = config.yt_api_key.clone();
        let thumbs = config.data_dir.join("thumbs");
        Arc::new(move |conn: &rusqlite::Connection| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let transport = cubby_yt::HttpTransport::new(&key);
            cubby_worker::run_pass(conn, &transport, now, Some(&thumbs));
        }) as Ingest
    });
    app_with_ingest(config, ingest)
}

/// [`app`] with the allow-time metadata pass supplied by the caller (tests pass a fixture one).
pub fn app_with_ingest(config: &Config, ingest: Option<Ingest>) -> Result<Router, String> {
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
        ingest,
    });
    let spa =
        ServeDir::new(&config.web_dir).fallback(ServeFile::new(config.web_dir.join("index.html")));
    Ok(Router::new()
        .route("/healthz", get(healthz))
        .merge(identity::routes(state))
        .route("/api/{*rest}", axum::routing::any(api_not_found))
        .nest_service("/thumbs", ServeDir::new(config.data_dir.join("thumbs")))
        .fallback_service(spa)
        .layer(middleware::from_fn_with_state(sink, observe))
        .layer(middleware::from_fn(capture_log_line))
        .layer(middleware::from_fn(cache_control)))
}

/// Thumbnails and the versioned `/assets/icons` are cached for a year; no `/api/*` response is ever cached (errors and 401s included), and every HTML response (the SPA shell
/// included) is `no-store`, so a CDN that caches by extension can never keep it as a static file; the immutable `/_app` assets keep the long cache they are built with. Without
/// this a browser heuristically caches a header-less response forever.
async fn cache_control(req: Request, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api/");
    let is_thumb = req.uri().path().starts_with("/thumbs/");
    let is_icon = req.uri().path().starts_with("/assets/icons/");
    let is_manifest = req.uri().path() == "/assets/manifest.webmanifest";
    let mut res = next.run(req).await;
    let value = if is_api {
        Some("no-store")
    } else if is_thumb {
        // Thumbnails are named for their video and never rewritten; a miss must not stick.
        Some(if res.status().is_success() {
            "public, max-age=31536000, immutable"
        } else {
            "no-store"
        })
    } else if res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/html"))
    {
        Some("no-store")
    } else if (is_icon || is_manifest) && res.status().is_success() {
        // Icons carry a version in their name and are never rewritten; the manifest names them, so it may change.
        Some(if is_icon {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=86400"
        })
    } else {
        None
    };
    if let Some(v) = value {
        res.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static(v),
        );
    }
    res
}

/// An unmounted `/api/*` path is a JSON 404, never the SPA shell with a 200.
async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"ok": true})))
}

/// Who a request was made by, filled by the guards (`device:<id>` or `parent:<email>`) so the
/// request log can name it. Never a token.
#[derive(Clone, Default)]
pub(crate) struct Actor(Arc<std::sync::Mutex<Option<String>>>);

impl Actor {
    pub(crate) fn set(&self, who: String) {
        if let Ok(mut g) = self.0.lock() {
            *g = Some(who);
        }
    }

    fn get(&self) -> Option<String> {
        self.0.lock().ok().and_then(|g| g.clone())
    }
}

/// Where request log lines go: stderr, which journald collects.
pub type LogSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Writes one JSON line per request: method, route template (never the raw path, which can carry
/// ids), status, latency, the actor and, for a refusal, its message. No header, body or token.
async fn log_request(sink: Option<&LogSink>, mut req: Request, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "spa".to_string(), |m| m.as_str().to_string());
    let method = req.method().to_string();
    let actor = Actor::default();
    req.extensions_mut().insert(actor.clone());
    let start = Instant::now();
    let res = next.run(req).await;
    let mut line = json!({
        "ts": SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        "method": method,
        "route": route,
        "status": res.status().as_u16(),
        "ms": start.elapsed().as_millis() as u64,
    });
    if let Some(who) = actor.get() {
        line["actor"] = who.into();
    }
    if let Some(cause) = res.extensions().get::<error::ErrorCause>() {
        line["error"] = cause.0.clone().into();
    }
    let text = format!("cubby-server: request {line}");
    match sink {
        Some(f) => f(&text),
        None => eprintln!("{text}"),
    }
    res
}

async fn capture_log_line(req: Request, next: Next) -> Response {
    let sink = req.extensions().get::<LogSink>().cloned();
    log_request(sink.as_ref(), req, next).await
}

/// Same as [`app`], but request lines go to `log` instead of stderr (tests read them).
pub fn app_with_log(config: &Config, log: LogSink) -> Result<Router, String> {
    Ok(app(config)?.layer(axum::Extension(log)))
}

async fn observe(State(sink): State<Arc<MetricsSink>>, req: Request, next: Next) -> Response {
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
            let labels = [("route", route.as_str()), ("status", status.as_str())];
            sink.push_seconds("cubby_http_request_seconds", seconds, &labels);
            sink.push_count("cubby_http_requests", &labels);
        });
    }
    res
}
