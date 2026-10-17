//! Identity routes and the two guards: the Access JWT for `/api/parent/*`, the device bearer
//! token for `/api/child/*` (except `/api/child/pair`, which is how a device gets one).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::to_bytes;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use cubby_db::identity::{self as ident, Device, PairError};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::access::{extract_token, missing_token};
use crate::error::ApiError;

type Result<T> = std::result::Result<T, ApiError>;

pub(crate) fn routes(state: Arc<AppState>) -> Router {
    let parent = Router::new()
        .route("/api/parent/children/{id}/pairing", post(new_pairing))
        .route("/api/parent/devices/{id}/revoke", post(revoke))
        .route_layer(middleware::from_fn_with_state(state.clone(), parent_guard));
    let child = Router::new()
        .route("/api/child/me", get(me))
        .merge(crate::play::routes())
        .route_layer(middleware::from_fn_with_state(state.clone(), device_guard));
    Router::new()
        .route("/api/child/pair", post(pair))
        .merge(parent)
        .merge(child)
        .with_state(state)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn db<T, E: std::fmt::Display>(r: std::result::Result<T, E>) -> Result<T> {
    r.map_err(ApiError::internal)
}

/// The family a verified parent belongs to, put on the request by [`parent_guard`].
#[derive(Clone)]
pub(crate) struct ParentOf(pub(crate) String);

impl AppState {
    /// Opens `family_id`'s database, but only if the registry knows the family: a made-up id
    /// must not make a stray file.
    fn open_known_family(&self, family_id: &str) -> Result<Option<Connection>> {
        let registry = self.registry.lock().map_err(ApiError::internal)?;
        if !db(registry.list())?.iter().any(|f| f.id == family_id) {
            return Ok(None);
        }
        Ok(Some(db(registry.open_family(family_id))?))
    }
}

/// Verifies the Access JWT and that its email is one of the family's parents. Fails closed: with
/// no Access configuration every parent route is a 401.
async fn parent_guard(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Result<Response> {
    let access = state
        .access
        .as_ref()
        .ok_or_else(|| ApiError::unauthorized("Cloudflare Access is not configured"))?;
    let (parts, body) = req.into_parts();
    let token = extract_token(&parts).ok_or_else(missing_token)?;
    let identity = access.verify(&token).await?;
    let family = {
        let registry = state.registry.lock().map_err(ApiError::internal)?;
        db(registry.lookup(identity.email()))?
    };
    let family =
        family.ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "not a parent of any family"))?;
    let mut req = Request::from_parts(parts, body);
    req.extensions_mut().insert(ParentOf(family.id));
    Ok(next.run(req).await)
}

/// Looks the bearer token up on every request, so a revoked device stops at once.
async fn device_guard(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Result<Response> {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let Some(token) = token else {
        state.count("cubby_device_auth", "missing");
        return Err(ApiError::unauthorized("missing device token"));
    };
    let device = match ident::token_family(token) {
        Some(fid) => match state.open_known_family(fid)? {
            Some(conn) => {
                db(ident::authenticate_device(&conn, token, now()))?.map(|d| (fid.to_string(), d))
            }
            None => None,
        },
        None => None,
    };
    let Some(device) = device else {
        state.count("cubby_device_auth", "rejected");
        return Err(ApiError::unauthorized("unknown or revoked device"));
    };
    state.count("cubby_device_auth", "ok");
    let (family_id, device) = device;
    req.extensions_mut().insert(ParentOf(family_id));
    req.extensions_mut().insert(device);
    Ok(next.run(req).await)
}

pub(crate) fn open_own(state: &AppState, family_id: &str) -> Result<Connection> {
    state
        .open_known_family(family_id)?
        .ok_or_else(|| ApiError::unauthorized("unknown family"))
}

async fn new_pairing(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Path(child_id): Path<String>,
) -> Result<Json<Value>> {
    let conn = open_own(&state, &family_id)?;
    let code = db(ident::new_pairing_code(&conn, &child_id, now()))?;
    let code = code.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no such child"))?;
    Ok(Json(
        json!({"code": code.code, "expires_at": code.expires_at}),
    ))
}

async fn revoke(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Path(device_id): Path<String>,
) -> Result<Json<Value>> {
    let conn = open_own(&state, &family_id)?;
    let revoked = db(ident::revoke_device(&conn, &device_id, now()))?;
    if !revoked {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "no such active device",
        ));
    }
    Ok(Json(json!({"revoked": true})))
}

#[derive(Deserialize)]
struct PairBody {
    code: String,
    label: Option<String>,
}

/// The client address: Cloudflare's header (the tunnel is the only way in, and it overwrites
/// it), else the socket peer.
fn client_ip(req: &Request) -> String {
    req.headers()
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip().to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

async fn pair(State(state): State<Arc<AppState>>, req: Request) -> Result<Json<Value>> {
    let ip = client_ip(&req);
    if !state.pair_limiter.allow(&ip, now().max(0) as u64) {
        state.count("cubby_device_auth", "rate_limited");
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many pairing attempts; wait a minute",
        ));
    }
    let body = to_bytes(req.into_body(), 4096)
        .await
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "bad body"))?;
    let body: PairBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "expected {\"code\": ...}"))?;
    let label = body
        .label
        .as_deref()
        .map(|l| l.chars().take(64).collect::<String>());
    let families = {
        let registry = state.registry.lock().map_err(ApiError::internal)?;
        db(registry.list())?
    };
    // A code names no family, so the first family that holds it as live claims it.
    let mut paired = Err(PairError::InvalidCode);
    for f in families {
        let Some(conn) = state.open_known_family(&f.id)? else {
            continue;
        };
        paired = ident::redeem_pairing_code(&conn, &f.id, &body.code, label.as_deref(), now());
        if !matches!(paired, Err(PairError::InvalidCode)) {
            break;
        }
    }
    match paired {
        Ok(p) => {
            state.count("cubby_device_auth", "paired");
            Ok(Json(json!({
                "token": p.token, "device_id": p.device_id, "child_id": p.child_id
            })))
        }
        Err(PairError::InvalidCode) => {
            state.count("cubby_device_auth", "invalid_code");
            Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "that code is wrong, used or expired",
            ))
        }
        Err(PairError::Db(e)) => Err(ApiError::internal(e)),
    }
}

async fn me(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(device): Extension<Device>,
) -> Result<Json<Value>> {
    let conn = open_own(&state, &family_id)?;
    let child = db(ident::child(&conn, &device.child_id))?;
    let child = child.ok_or_else(|| ApiError::unauthorized("device has no child"))?;
    Ok(Json(json!({
        "child_id": child.id, "name": child.name, "tz": child.tz, "device_id": device.id
    })))
}
