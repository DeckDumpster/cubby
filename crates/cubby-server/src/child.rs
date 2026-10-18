//! Child API reads: home, search and next. Every tile comes from `playable`, and ranker output
//! is passed through `rank_checked` so it cannot name a video outside that set.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::error::ApiError;
use crate::identity::{ParentOf, open_own};
use axum::Extension;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use cubby_core::rank::{ChildCtx, Ranker, V1Ranker, VideoMeta, rank_checked};
use cubby_db::browse::{self, History, VideoRow};
use cubby_db::identity::Device;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

pub const NEXT_TILES: usize = 12;
const HOME_ROW: usize = 12;
const HOME_ALL: usize = 60;
const SEARCH_LIMIT: usize = 40;

/// Resolves a request to the child its device token belongs to.
pub type Auth = Arc<dyn Fn(&HeaderMap) -> Option<String> + Send + Sync>;
/// Remaining budget for a child, as the budget engine reports it.
pub type Budget = Arc<dyn Fn(&Connection, &str) -> Value + Send + Sync>;

#[derive(Clone)]
pub struct ChildApi {
    pub db: Arc<Mutex<Connection>>,
    pub auth: Auth,
    pub budget: Budget,
    pub ranker: Arc<dyn Ranker + Send + Sync>,
    pub now: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl ChildApi {
    pub fn new(db: Connection, auth: Auth, budget: Budget) -> ChildApi {
        ChildApi {
            db: Arc::new(Mutex::new(db)),
            auth,
            budget,
            ranker: Arc::new(V1Ranker),
            now: Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64)
            }),
        }
    }
}

/// Mounted behind the device guard, which supplies the family and the [`Device`].
pub(crate) fn routes() -> axum::Router<Arc<crate::AppState>> {
    Router::new()
        .route("/api/child/home", get(home_route))
        .route("/api/child/search", get(search_route))
        .route("/api/child/next", get(next_route))
}

/// A catalogue handle over the device's own family database.
fn api_for(state: &crate::AppState, family_id: &str) -> Result<(ChildApi, Connection), ApiError> {
    let conn = open_own(state, family_id)?;
    let api = ChildApi::new(
        Connection::open_in_memory().map_err(ApiError::internal)?,
        Arc::new(|_| None),
        Arc::new(|c, child| crate::play::remaining_value(c, child, crate::play::unix_now())),
    );
    Ok((api, conn))
}

async fn home_route(
    State(state): State<Arc<crate::AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
) -> Result<Json<Value>, ApiError> {
    let (api, conn) = api_for(&state, &family_id)?;
    home_json(&api, &conn, &dev.child_id)
        .map(Json)
        .map_err(ApiError::internal)
}

async fn search_route(
    State(state): State<Arc<crate::AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Query(p): Query<SearchParams>,
) -> Result<Json<Value>, ApiError> {
    let conn = open_own(&state, &family_id)?;
    let found =
        browse::search(&conn, &dev.child_id, &p.q, SEARCH_LIMIT).map_err(ApiError::internal)?;
    Ok(Json(json!({
        "q": p.q,
        "tiles": found.iter().map(tile).collect::<Vec<_>>(),
    })))
}

async fn next_route(
    State(state): State<Arc<crate::AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Query(p): Query<NextParams>,
) -> Result<Json<Value>, ApiError> {
    let (api, conn) = api_for(&state, &family_id)?;
    next_json(&api, &conn, &dev.child_id, &p.after)
        .map(Json)
        .map_err(ApiError::internal)
}

pub fn router(api: ChildApi) -> Router {
    Router::new()
        .route("/api/child/home", get(home))
        .route("/api/child/search", get(search))
        .route("/api/child/next", get(next))
        .with_state(api)
}

fn tile(v: &VideoRow) -> Value {
    json!({
        "youtube_id": v.youtube_id,
        "title": v.title,
        "channel_title": v.channel_title,
        "duration_s": v.duration_s,
        "thumb": v.thumb_path.as_ref().map(|p| format!("/thumbs/{p}")),
    })
}

fn meta(v: &VideoRow) -> VideoMeta {
    VideoMeta {
        id: v.youtube_id.clone(),
        channel_id: v.channel_id.clone(),
        tags: v.tags.clone(),
        published_at: v.published_at,
    }
}

fn ctx(h: &History, now: i64) -> ChildCtx {
    ChildCtx {
        watched: h.watched.clone(),
        liked_channels: h.liked_channels.clone(),
        now,
    }
}

/// Ranker order over `rows`, resolved back to rows, at most `limit` of them.
fn ranked<'a>(
    api: &ChildApi,
    child: &ChildCtx,
    current: Option<&VideoRow>,
    rows: &'a [VideoRow],
    limit: usize,
) -> Vec<&'a VideoRow> {
    let metas: Vec<VideoMeta> = rows.iter().map(meta).collect();
    let by_id: HashMap<&str, &VideoRow> = rows.iter().map(|r| (r.youtube_id.as_str(), r)).collect();
    let cur = current.map(meta);
    rank_checked(api.ranker.as_ref(), child, cur.as_ref(), &metas)
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .take(limit)
        .collect()
}

fn authed(api: &ChildApi, headers: &HeaderMap) -> Result<String, StatusCode> {
    (api.auth)(headers).ok_or(StatusCode::UNAUTHORIZED)
}

fn db_err(_: rusqlite::Error) -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}

pub fn home_json(api: &ChildApi, conn: &Connection, child_id: &str) -> rusqlite::Result<Value> {
    let rows = browse::playable_videos(conn, child_id)?;
    let h = browse::history(conn, child_id)?;
    let c = ctx(&h, (api.now)());
    let by_id: HashMap<&str, &VideoRow> = rows.iter().map(|r| (r.youtube_id.as_str(), r)).collect();

    let cont: Vec<Value> = h
        .resumable
        .iter()
        .filter_map(|id| by_id.get(id.as_str()))
        .take(HOME_ROW)
        .map(|v| tile(v))
        .collect();
    let liked: Vec<VideoRow> = rows
        .iter()
        .filter(|r| h.liked_channels.contains(&r.channel_id))
        .cloned()
        .collect();
    let mut newest: Vec<&VideoRow> = rows
        .iter()
        .filter(|r| !h.watched.contains(&r.youtube_id))
        .collect();
    newest.sort_by(|a, b| {
        b.published_at
            .cmp(&a.published_at)
            .then_with(|| a.youtube_id.cmp(&b.youtube_id))
    });
    Ok(json!({
        "remaining": (api.budget)(conn, child_id),
        "sections": [
            {"key": "continue", "tiles": cont},
            {"key": "liked_channels", "tiles": ranked(api, &c, None, &liked, HOME_ROW).iter().map(|v| tile(v)).collect::<Vec<_>>()},
            {"key": "new", "tiles": newest.iter().take(HOME_ROW).map(|v| tile(v)).collect::<Vec<_>>()},
            {"key": "all", "tiles": ranked(api, &c, None, &rows, HOME_ALL).iter().map(|v| tile(v)).collect::<Vec<_>>()},
        ],
    }))
}

pub fn next_json(
    api: &ChildApi,
    conn: &Connection,
    child_id: &str,
    after: &str,
) -> rusqlite::Result<Value> {
    let rows = browse::playable_videos(conn, child_id)?;
    let h = browse::history(conn, child_id)?;
    let c = ctx(&h, (api.now)());
    let current = rows.iter().find(|r| r.youtube_id == after);
    // The current video is not a candidate, whatever the ranker would do with it.
    let others: Vec<VideoRow> = rows
        .iter()
        .filter(|r| r.youtube_id != after)
        .cloned()
        .collect();
    let tiles: Vec<Value> = ranked(api, &c, current, &others, NEXT_TILES)
        .iter()
        .map(|v| tile(v))
        .collect();
    Ok(json!({"tiles": tiles, "remaining": (api.budget)(conn, child_id)}))
}

async fn home(State(api): State<ChildApi>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let child = authed(&api, &headers)?;
    let conn = api.db.lock().unwrap();
    home_json(&api, &conn, &child).map(Json).map_err(db_err)
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
}

async fn search(
    State(api): State<ChildApi>,
    headers: HeaderMap,
    Query(p): Query<SearchParams>,
) -> Result<Json<Value>, StatusCode> {
    let child = authed(&api, &headers)?;
    let conn = api.db.lock().unwrap();
    let found = browse::search(&conn, &child, &p.q, SEARCH_LIMIT).map_err(db_err)?;
    // Relevance (bm25) is the order; the join to `playable` is what keeps the set safe.
    Ok(Json(json!({
        "q": p.q,
        "tiles": found.iter().map(tile).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct NextParams {
    after: String,
}

async fn next(
    State(api): State<ChildApi>,
    headers: HeaderMap,
    Query(p): Query<NextParams>,
) -> Result<Json<Value>, StatusCode> {
    let child = authed(&api, &headers)?;
    let conn = api.db.lock().unwrap();
    next_json(&api, &conn, &child, &p.after)
        .map(Json)
        .map_err(db_err)
}
