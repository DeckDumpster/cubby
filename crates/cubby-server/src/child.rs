//! Child API reads: home, search and next. Every tile comes from `playable`, and ranker output
//! is passed through `rank_checked` so it cannot name a video outside that set.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use cubby_core::rank::{ChildCtx, Ranker, V1Ranker, VideoMeta, rank_checked};
use cubby_db::browse::{self, History, VideoRow};
use cubby_db::identity::Device;
use cubby_db::playlists::{self, PlaylistError};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::error::ApiError;
use crate::identity::{ParentOf, open_own};

pub const NEXT_TILES: usize = 12;
const HOME_ROW: usize = 12;
const HOME_ALL: usize = 60;
const SEARCH_LIMIT: usize = 40;
const RECENT_PAGE: usize = 24;
const RECENT_MAX: usize = 60;

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

pub fn router(api: ChildApi) -> Router {
    Router::new()
        .route("/api/child/home", get(home))
        .route("/api/child/search", get(search))
        .route("/api/child/next", get(next))
        .route("/api/child/recent", get(recent))
        .route("/api/child/channel/{id}", get(channel))
        .route("/api/child/channel/{id}/request", post(channel_request))
        .with_state(api)
}

fn tile(v: &VideoRow) -> Value {
    json!({
        "youtube_id": v.youtube_id,
        "title": v.title,
        "channel_id": v.channel_id,
        "channel_title": v.channel_title,
        "duration_s": v.duration_s,
        "thumb": v.thumb_path.as_ref().map(|p| format!("/thumbs/{p}?v={}", v.published_at)),
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
    ranker: &dyn Ranker,
    child: &ChildCtx,
    current: Option<&VideoRow>,
    rows: &'a [VideoRow],
    limit: usize,
) -> Vec<&'a VideoRow> {
    let metas: Vec<VideoMeta> = rows.iter().map(meta).collect();
    let by_id: HashMap<&str, &VideoRow> = rows.iter().map(|r| (r.youtube_id.as_str(), r)).collect();
    let cur = current.map(meta);
    rank_checked(ranker, child, cur.as_ref(), &metas)
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
    home_with(
        api.ranker.as_ref(),
        (api.now)(),
        (api.budget)(conn, child_id),
        conn,
        child_id,
    )
}

fn home_with(
    ranker: &dyn Ranker,
    now: i64,
    remaining: Value,
    conn: &Connection,
    child_id: &str,
) -> rusqlite::Result<Value> {
    let rows = browse::playable_videos(conn, child_id)?;
    let h = browse::history(conn, child_id)?;
    let c = ctx(&h, now);
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
    let child = conn
        .query_row(
            "SELECT name, avatar FROM children WHERE id=?1",
            [child_id],
            |r| Ok(json!({"name": r.get::<_, String>(0)?, "avatar": r.get::<_, String>(1)?})),
        )
        .unwrap_or(Value::Null);
    let later = playlists::list(conn, child_id, playlists::WATCH_LATER).unwrap_or_default();
    let saved = playlists::saved_ids(conn, child_id, playlists::WATCH_LATER)?;
    let channels: Vec<Value> = browse::channels(conn, child_id)?
        .iter()
        .map(|c| {
            json!({
                "channel_id": c.channel_id,
                "title": c.title,
                "thumb": c.thumb_path.as_ref().map(|p| format!("/thumbs/{p}?v={}", c.published_at)),
            })
        })
        .collect();
    Ok(json!({
        "child": child,
        "remaining": remaining,
        "saved": saved.iter().collect::<Vec<_>>(),
        "watch_later": later.iter().map(tile).collect::<Vec<_>>(),
        "channels": channels,
        "sections": [
            {"key": "continue", "title": "Keep watching", "tiles": cont},
            {"key": "liked_channels", "title": "From channels you like", "tiles": ranked(ranker, &c, None, &liked, HOME_ROW).iter().map(|v| tile(v)).collect::<Vec<_>>()},
            {"key": "new", "title": "New", "tiles": newest.iter().take(HOME_ROW).map(|v| tile(v)).collect::<Vec<_>>()},
            {"key": "all", "title": "Everything", "tiles": ranked(ranker, &c, None, &rows, HOME_ALL).iter().map(|v| tile(v)).collect::<Vec<_>>()},
        ],
    }))
}

pub fn next_json(
    api: &ChildApi,
    conn: &Connection,
    child_id: &str,
    after: &str,
) -> rusqlite::Result<Value> {
    next_with(
        api.ranker.as_ref(),
        (api.now)(),
        (api.budget)(conn, child_id),
        conn,
        child_id,
        after,
    )
}

fn next_with(
    ranker: &dyn Ranker,
    now: i64,
    remaining: Value,
    conn: &Connection,
    child_id: &str,
    after: &str,
) -> rusqlite::Result<Value> {
    let rows = browse::playable_videos(conn, child_id)?;
    let h = browse::history(conn, child_id)?;
    let c = ctx(&h, now);
    let current = rows.iter().find(|r| r.youtube_id == after);
    // The current video is not a candidate, whatever the ranker would do with it.
    let others: Vec<VideoRow> = rows
        .iter()
        .filter(|r| r.youtube_id != after)
        .cloned()
        .collect();
    let tiles: Vec<Value> = ranked(ranker, &c, current, &others, NEXT_TILES)
        .iter()
        .map(|v| tile(v))
        .collect();
    Ok(json!({"tiles": tiles, "remaining": remaining}))
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

fn ask_json(a: browse::ChannelAsk) -> Value {
    json!({"can_request": !a.imported && !a.requested, "requested": a.requested && !a.imported})
}

/// One channel's allowed videos, newest first. A channel with none for this child is a 404,
/// the same answer as a channel that does not exist.
fn channel_json(conn: &Connection, child_id: &str, id: &str) -> rusqlite::Result<Option<Value>> {
    let rows = browse::channel_videos(conn, child_id, id)?;
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let ask = browse::channel_ask(conn, child_id, id)?;
    Ok(Some(json!({
        "channel": {"channel_id": id, "title": first.channel_title},
        "tiles": rows.iter().map(tile).collect::<Vec<_>>(),
        "can_request": !ask.imported && !ask.requested,
        "requested": ask.requested && !ask.imported,
    })))
}

fn channel_request_json(
    conn: &Connection,
    child_id: &str,
    id: &str,
    now: i64,
) -> rusqlite::Result<Option<Value>> {
    Ok(browse::request_more_from_channel(conn, child_id, id, now)?.map(ask_json))
}

async fn channel(
    State(api): State<ChildApi>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let child = authed(&api, &headers)?;
    let conn = api.db.lock().unwrap();
    channel_json(&conn, &child, &id)
        .map_err(db_err)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn channel_request(
    State(api): State<ChildApi>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let child = authed(&api, &headers)?;
    let conn = api.db.lock().unwrap();
    channel_request_json(&conn, &child, &id, (api.now)())
        .map_err(db_err)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Deserialize)]
struct RecentParams {
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

/// One page of the watch history; `more` says whether another page follows.
fn recent_json(conn: &Connection, child_id: &str, p: &RecentParams) -> rusqlite::Result<Value> {
    let limit = p.limit.unwrap_or(RECENT_PAGE).clamp(1, RECENT_MAX);
    let mut rows = browse::recent(conn, child_id, limit + 1, p.offset)?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    Ok(json!({"tiles": rows.iter().map(tile).collect::<Vec<_>>(), "more": more}))
}

async fn recent(
    State(api): State<ChildApi>,
    headers: HeaderMap,
    Query(p): Query<RecentParams>,
) -> Result<Json<Value>, StatusCode> {
    let child = authed(&api, &headers)?;
    let conn = api.db.lock().unwrap();
    recent_json(&conn, &child, &p).map(Json).map_err(db_err)
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

/// The same three reads, mounted in the real app behind the device guard: the child is the
/// device's own and the database is its family's, so no call can name another.
pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/child/home", get(home_route))
        .route("/api/child/search", get(search_route))
        .route("/api/child/next", get(next_route))
        .route("/api/child/recent", get(recent_route))
        .route("/api/child/request", post(request_route))
        .route(
            "/api/child/playlists/{kind}/items",
            get(playlist_list_route),
        )
        .route(
            "/api/child/playlists/{kind}/items/{id}",
            put(playlist_add_route).delete(playlist_remove_route),
        )
        .route("/api/child/channel/{id}", get(channel_route))
        .route("/api/child/channel/{id}/request", post(channel_request_route))
}

#[derive(Deserialize)]
struct RequestBody {
    kind: String,
    youtube_id: String,
}

/// A child's "I like this" / "More from this channel": one open row for the parent's inbox.
async fn request_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Json(b): Json<RequestBody>,
) -> Result<Json<Value>, ApiError> {
    if !matches!(b.kind.as_str(), "like" | "more-from-channel") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unknown request kind",
        ));
    }
    let conn = open_own(&state, &family_id)?;
    let id = cubby_db::parent::add_request(
        &conn,
        &dev.child_id,
        &b.kind,
        &b.youtube_id,
        crate::play::unix_now(),
    )
    .map_err(internal)?
    .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "that video is not available"))?;
    Ok(Json(json!({"ok": true, "id": id})))
}

fn playlist_err(e: PlaylistError) -> ApiError {
    match e {
        PlaylistError::UnknownKind => ApiError::new(StatusCode::NOT_FOUND, "no such playlist"),
        PlaylistError::NotAllowed => ApiError::new(StatusCode::FORBIDDEN, "not allowed"),
        PlaylistError::Db(m) => internal(m),
    }
}

async fn playlist_list_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Path(kind): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let conn = open_own(&state, &family_id)?;
    let rows = playlists::list(&conn, &dev.child_id, &kind).map_err(playlist_err)?;
    Ok(Json(
        json!({"kind": kind, "tiles": rows.iter().map(tile).collect::<Vec<_>>()}),
    ))
}

async fn playlist_add_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let conn = open_own(&state, &family_id)?;
    let now = crate::play::unix_now();
    playlists::add(&conn, &dev.child_id, &kind, &id, now).map_err(playlist_err)?;
    Ok(Json(json!({"ok": true})))
}

async fn playlist_remove_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let conn = open_own(&state, &family_id)?;
    playlists::remove(&conn, &dev.child_id, &kind, &id).map_err(playlist_err)?;
    Ok(Json(json!({"ok": true})))
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::internal(e)
}

/// Browsing is refused once the grace is over; the body carries the limit so the page can say so.
fn guard_locked(conn: &Connection, child: &str, now: i64) -> Result<Value, Response> {
    let limit =
        crate::play::limit_json(conn, child, now).map_err(|e| internal(e).into_response())?;
    if limit["state"] == "locked" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "locked", "limit": limit})),
        )
            .into_response());
    }
    Ok(limit)
}

async fn home_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
) -> Result<Json<Value>, Response> {
    let conn = open_own(&state, &family_id).map_err(IntoResponse::into_response)?;
    let now = crate::play::unix_now();
    let limit = guard_locked(&conn, &dev.child_id, now)?;
    let remaining = crate::play::remaining_json_for(&conn, &dev.child_id, now)
        .map_err(|e| internal(e).into_response())?;
    let mut v = home_with(&V1Ranker, now, remaining, &conn, &dev.child_id)
        .map_err(|e| internal(e).into_response())?;
    v["limit"] = limit;
    Ok(Json(v))
}

async fn search_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Query(p): Query<SearchParams>,
) -> Result<Json<Value>, Response> {
    let conn = open_own(&state, &family_id).map_err(IntoResponse::into_response)?;
    guard_locked(&conn, &dev.child_id, crate::play::unix_now())?;
    let found = browse::search(&conn, &dev.child_id, &p.q, SEARCH_LIMIT)
        .map_err(|e| internal(e).into_response())?;
    Ok(Json(json!({
        "q": p.q,
        "tiles": found.iter().map(tile).collect::<Vec<_>>(),
    })))
}

async fn recent_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Query(p): Query<RecentParams>,
) -> Result<Json<Value>, Response> {
    let conn = open_own(&state, &family_id).map_err(IntoResponse::into_response)?;
    guard_locked(&conn, &dev.child_id, crate::play::unix_now())?;
    recent_json(&conn, &dev.child_id, &p)
        .map(Json)
        .map_err(|e| internal(e).into_response())
}

async fn next_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Query(p): Query<NextParams>,
) -> Result<Json<Value>, Response> {
    let conn = open_own(&state, &family_id).map_err(IntoResponse::into_response)?;
    let now = crate::play::unix_now();
    let limit = guard_locked(&conn, &dev.child_id, now)?;
    let remaining = crate::play::remaining_json_for(&conn, &dev.child_id, now)
        .map_err(|e| internal(e).into_response())?;
    let mut v = next_with(&V1Ranker, now, remaining, &conn, &dev.child_id, &p.after)
        .map_err(|e| internal(e).into_response())?;
    v["limit"] = limit;
    Ok(Json(v))
}

async fn channel_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let conn = open_own(&state, &family_id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    channel_json(&conn, &dev.child_id, &id)
        .map_err(db_err)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn channel_request_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let conn = open_own(&state, &family_id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    channel_request_json(&conn, &dev.child_id, &id, crate::play::unix_now())
        .map_err(db_err)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
