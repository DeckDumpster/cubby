//! The parent API: allow-list, review list, children, budgets, devices, history and the
//! requests inbox. Mounted behind the Access guard; the allow-list invariant lives in the
//! `playable` view, these handlers only write the rows it reads.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use cubby_core::budget::{parse_windows, valid_tz};
use cubby_core::policy_url::{ChannelLookup, resolve_url};
use cubby_db::DbError;
use cubby_db::parent::{self as store, Budget};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::error::ApiError;
use crate::identity::{ParentOf, open_own};
use crate::play::unix_now;

type Result<T> = std::result::Result<T, ApiError>;

pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/parent/parents", get(parents))
        .route("/api/parent/children", get(children).post(add_child))
        .route("/api/parent/progress", get(progress))
        .route("/api/parent/children/{id}", put(edit_child))
        .route("/api/parent/children/{id}/budget", put(set_budget))
        .route("/api/parent/children/{id}/history", get(child_history))
        .route("/api/parent/policies", get(policies).post(add_policy))
        .route("/api/parent/policies/preview", post(preview))
        .route("/api/parent/policies/{id}", delete(remove_policy))
        .route("/api/parent/review", get(review))
        .route("/api/parent/library", get(library))
        .route("/api/parent/library/videos", get(library_videos))
        .route("/api/parent/videos/{id}/block", post(block))
        .route("/api/parent/videos/{id}/unblock", post(unblock))
        .route("/api/parent/videos/{id}/approve", post(approve))
        .route("/api/parent/requests", get(requests))
        .route("/api/parent/requests/{id}/resolve", post(resolve))
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> ApiError {
        match e {
            DbError::Invalid(m) => ApiError::new(StatusCode::BAD_REQUEST, m),
            e => ApiError::internal(e),
        }
    }
}

fn not_found(what: &str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, format!("no such {what}"))
}

fn bad(msg: &str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, msg)
}

fn conn(state: &AppState, family: &ParentOf) -> Result<rusqlite::Connection> {
    open_own(state, &family.0)
}

#[derive(Deserialize)]
struct ChildBody {
    name: String,
    #[serde(default)]
    avatar: String,
    tz: String,
}

fn check_child(b: &ChildBody) -> Result<()> {
    if b.name.trim().is_empty() || b.name.chars().count() > 64 {
        return Err(bad("a child needs a name of up to 64 characters"));
    }
    if b.avatar.chars().count() > 16 {
        return Err(bad("the avatar is one emoji or a few characters"));
    }
    if !valid_tz(&b.tz) {
        return Err(bad("that is not a time zone name"));
    }
    Ok(())
}

async fn children(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
) -> Result<Json<Value>> {
    let c = conn(&s, &f)?;
    let devices = store::list_devices(&c)?;
    let mut out = vec![];
    for k in store::list_children(&c)? {
        let b = store::get_budget(&c, &k.id)?;
        let windows: Value = b
            .daily_windows
            .as_deref()
            .and_then(|w| serde_json::from_str(w).ok())
            .unwrap_or(json!([]));
        let devs: Vec<Value> = devices
            .iter()
            .filter(|d| d.child_id == k.id)
            .map(|d| {
                json!({"id": d.id, "label": d.label, "paired_at": d.paired_at,
                    "last_seen": d.last_seen, "revoked_at": d.revoked_at})
            })
            .collect();
        out.push(json!({
            "id": k.id, "name": k.name, "avatar": k.avatar, "tz": k.tz,
            "budget": {"video_equivalents": b.video_equivalents, "window_s": b.window_s,
                "minutes_cap": b.minutes_cap, "cutoff_mid_play": b.cutoff_mid_play,
                "daily_windows": windows},
            "devices": devs,
        }));
    }
    Ok(Json(Value::Array(out)))
}

/// Each child's usage and what is left, computed by the play path's own budget code.
async fn progress(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
) -> Result<Json<Value>> {
    let c = conn(&s, &f)?;
    let now = unix_now();
    let mut out = vec![];
    for k in store::list_children(&c)? {
        out.push(crate::play::progress_json(&c, &k.id, now).map_err(ApiError::internal)?);
    }
    Ok(Json(Value::Array(out)))
}

async fn add_child(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Json(b): Json<ChildBody>,
) -> Result<Json<Value>> {
    check_child(&b)?;
    let id = store::create_child(&conn(&s, &f)?, b.name.trim(), &b.avatar, &b.tz)?;
    Ok(Json(json!({"id": id})))
}

async fn edit_child(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Path(id): Path<String>,
    Json(b): Json<ChildBody>,
) -> Result<Json<Value>> {
    check_child(&b)?;
    if !store::update_child(&conn(&s, &f)?, &id, b.name.trim(), &b.avatar, &b.tz)? {
        return Err(not_found("child"));
    }
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct BudgetBody {
    video_equivalents: Option<f64>,
    window_s: Option<i64>,
    minutes_cap: Option<i64>,
    #[serde(default)]
    cutoff_mid_play: bool,
    #[serde(default)]
    daily_windows: Vec<(String, String)>,
}

async fn set_budget(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Path(id): Path<String>,
    Json(b): Json<BudgetBody>,
) -> Result<Json<Value>> {
    if b.video_equivalents
        .is_some_and(|v| !v.is_finite() || v < 0.0)
        || b.minutes_cap.is_some_and(|m| m < 0)
        || b.window_s.is_some_and(|w| w <= 0)
    {
        return Err(bad("budget numbers must be positive"));
    }
    if !b.daily_windows.is_empty() && parse_windows(&b.daily_windows).is_none() {
        return Err(bad(
            "daily windows are pairs of HH:MM times, start before end",
        ));
    }
    let c = conn(&s, &f)?;
    if cubby_db::identity::child(&c, &id)?.is_none() {
        return Err(not_found("child"));
    }
    let windows = (!b.daily_windows.is_empty())
        .then(|| serde_json::to_string(&b.daily_windows))
        .transpose()
        .map_err(ApiError::internal)?;
    store::put_budget(
        &c,
        &id,
        &Budget {
            video_equivalents: b.video_equivalents,
            window_s: b.window_s,
            minutes_cap: b.minutes_cap,
            cutoff_mid_play: b.cutoff_mid_play,
            daily_windows: windows,
        },
    )?;
    Ok(Json(json!({"ok": true})))
}

async fn child_history(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let rows = store::history(&conn(&s, &f)?, &id, 200)?;
    Ok(Json(Value::Array(
        rows.into_iter()
            .map(|g| {
                json!({"id": g.id, "youtube_id": g.youtube_id, "title": g.title,
                    "granted_at": g.granted_at, "watched_s": g.watched_s, "charge": g.charge,
                    "ended_at": g.ended_at, "end_reason": g.end_reason, "thumb": g.thumb})
            })
            .collect(),
    )))
}

async fn policies(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
) -> Result<Json<Value>> {
    let rows = store::list_policies(&conn(&s, &f)?)?;
    Ok(Json(Value::Array(
        rows.into_iter()
            .map(|p| {
                json!({"id": p.id, "kind": p.kind, "youtube_id": p.youtube_id,
                    "child_id": p.child_id, "added_at": p.added_at, "title": p.title,
                    "pending": p.pending})
            })
            .collect(),
    )))
}

#[derive(Deserialize)]
struct UrlBody {
    url: String,
    child_id: Option<String>,
}

/// A pasted link's policy with a channel named by handle or legacy name turned into its `UC` id.
struct Target {
    kind: &'static str,
    youtube_id: String,
    /// What the parent typed for a channel that was looked up, kept for display.
    label: Option<String>,
    title: Option<String>,
}

async fn resolve_target(s: &Arc<AppState>, f: &ParentOf, url: &str) -> Result<Target> {
    let r = resolve_url(url)
        .ok_or_else(|| bad("that is not a YouTube video, channel or playlist link"))?;
    let kind = r.kind.as_str();
    let Some(lookup) = r.lookup else {
        return Ok(Target {
            kind,
            youtube_id: r.youtube_id,
            label: None,
            title: None,
        });
    };
    let Some(resolver) = s.resolver.clone() else {
        // No API key: a handle is kept as typed and the worker resolves it on its next pass.
        return match lookup {
            ChannelLookup::Handle(_) => Ok(Target {
                kind,
                youtube_id: r.youtube_id,
                label: None,
                title: None,
            }),
            ChannelLookup::Username(n) => Err(bad(&format!(
                "cannot look up the channel {n} right now; paste its /channel/ or @handle link"
            ))),
        };
    };
    let name = r.youtube_id.clone();
    let (state, family) = (s.clone(), f.0.clone());
    let found = tokio::task::spawn_blocking(move || {
        let c = open_own(&state, &family).map_err(|e| e.1)?;
        resolver(&c, &lookup)
    })
    .await
    .map_err(ApiError::internal)?;
    match found {
        Ok(Some(ch)) => Ok(Target {
            kind,
            youtube_id: ch.id,
            label: Some(name),
            title: Some(ch.title),
        }),
        Ok(None) => Err(bad(&format!("no YouTube channel was found for {name}"))),
        Err(e) => Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("could not look up {name} on YouTube: {e}"),
        )),
    }
}

async fn preview(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Json(b): Json<UrlBody>,
) -> Result<Json<Value>> {
    let t = resolve_target(&s, &f, &b.url).await?;
    Ok(Json(
        json!({"kind": t.kind, "youtube_id": t.youtube_id, "title": t.title}),
    ))
}

async fn add_policy(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Json(b): Json<UrlBody>,
) -> Result<Json<Value>> {
    let r = resolve_target(&s, &f, &b.url).await?;
    let c = conn(&s, &f)?;
    if let Some(child) = &b.child_id
        && cubby_db::identity::child(&c, child)?.is_none()
    {
        return Err(not_found("child"));
    }
    let id = store::add_policy_labelled(
        &c,
        r.kind,
        &r.youtube_id,
        b.child_id.as_deref(),
        unix_now(),
        r.label.as_deref(),
    )?;
    // Ingest before answering so the child sees the video at once; a failure here leaves the
    // policy pending for the worker's next pass.
    if let Some(ingest) = s.ingest.clone() {
        let state = s.clone();
        let family = f.0.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(c) = open_own(&state, &family) {
                ingest(&c);
            }
        })
        .await;
    }
    Ok(Json(
        json!({"id": id, "kind": r.kind, "youtube_id": r.youtube_id, "title": r.title}),
    ))
}

async fn remove_policy(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    if !store::remove_policy(&conn(&s, &f)?, &id, unix_now())? {
        return Err(not_found("policy"));
    }
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct ReviewQuery {
    channel: Option<String>,
}

async fn review(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Query(q): Query<ReviewQuery>,
) -> Result<Json<Value>> {
    let rows = store::auto_allowed(&conn(&s, &f)?, q.channel.as_deref())?;
    Ok(Json(Value::Array(
        rows.into_iter()
            .map(|i| {
                json!({"youtube_id": i.youtube_id, "title": i.title, "channel_id": i.channel_id,
                    "channel_title": i.channel_title, "policy_id": i.policy_id,
                    "policy_kind": i.policy_kind, "policy_youtube_id": i.policy_youtube_id,
                    "child_id": i.child_id})
            })
            .collect(),
    )))
}

#[derive(Deserialize)]
struct LibraryQuery {
    #[serde(default)]
    view: String,
    q: Option<String>,
    child: Option<String>,
    group: Option<String>,
    policy: Option<String>,
    #[serde(default)]
    sort: String,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
}

impl LibraryQuery {
    fn filter(&self) -> store::LibraryFilter {
        store::LibraryFilter {
            review: self.view == "review",
            q: self.q.clone().filter(|q| !q.trim().is_empty()),
            child: self.child.clone().filter(|c| !c.is_empty()),
        }
    }
}

/// The collapsed tree for the allow-list and review screens: groups and counts only.
async fn library(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Query(q): Query<LibraryQuery>,
) -> Result<Json<Value>> {
    let groups = store::library_groups(&conn(&s, &f)?, &q.filter())?;
    Ok(Json(json!({"groups": groups.into_iter().map(|g| json!({
        "key": g.key, "title": g.title, "count": g.count,
        "playlists": g.playlists.into_iter().map(|p| json!({
            "policy_id": p.policy_id, "kind": p.kind, "youtube_id": p.youtube_id,
            "count": p.count})).collect::<Vec<_>>(),
    })).collect::<Vec<_>>()})))
}

/// One page of a group's videos, with play count and watched seconds per video.
async fn library_videos(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Query(q): Query<LibraryQuery>,
) -> Result<Json<Value>> {
    let group = q.group.clone().ok_or_else(|| bad("a group is required"))?;
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let (total, rows) = store::library_videos(
        &conn(&s, &f)?,
        &q.filter(),
        &group,
        q.policy.as_deref(),
        &q.sort,
        q.offset,
        limit,
    )?;
    Ok(Json(
        json!({"total": total, "videos": rows.into_iter().map(|v| json!({
        "youtube_id": v.youtube_id, "title": v.title, "channel_title": v.channel_title,
        "policy_id": v.policy_id, "policy_kind": v.policy_kind,
        "policy_youtube_id": v.policy_youtube_id, "child_id": v.child_id,
        "plays": v.plays, "watched_s": v.watched_s})).collect::<Vec<_>>()}),
    ))
}

#[derive(Deserialize, Default)]
struct ScopeBody {
    child_id: Option<String>,
}

async fn block(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Path(id): Path<String>,
    Json(b): Json<ScopeBody>,
) -> Result<Json<Value>> {
    store::block_video(&conn(&s, &f)?, &id, b.child_id.as_deref(), unix_now())?;
    Ok(Json(json!({"ok": true})))
}

async fn unblock(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Path(id): Path<String>,
    Json(b): Json<ScopeBody>,
) -> Result<Json<Value>> {
    store::unblock_video(&conn(&s, &f)?, &id, b.child_id.as_deref())?;
    Ok(Json(json!({"ok": true})))
}

async fn approve(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    store::approve_video(&conn(&s, &f)?, &id, unix_now())?;
    Ok(Json(json!({"ok": true})))
}

async fn parents(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
) -> Result<Json<Value>> {
    let emails = s
        .registry
        .lock()
        .map_err(ApiError::internal)?
        .parents(&f.0)?;
    Ok(Json(json!(emails)))
}

async fn requests(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
) -> Result<Json<Value>> {
    let rows = store::open_requests(&conn(&s, &f)?)?;
    Ok(Json(Value::Array(
        rows.into_iter()
            .map(|r| {
                json!({"id": r.id, "child_id": r.child_id, "child_name": r.child_name,
                    "kind": r.kind, "youtube_id": r.youtube_id, "title": r.title,
                    "channel_id": r.channel_id, "channel_title": r.channel_title,
                    "created_at": r.created_at})
            })
            .collect(),
    )))
}

#[derive(Deserialize)]
struct ResolveBody {
    action: String,
    #[serde(default)]
    n: usize,
}

async fn resolve(
    State(s): State<Arc<AppState>>,
    Extension(f): Extension<ParentOf>,
    Path(id): Path<String>,
    Json(b): Json<ResolveBody>,
) -> Result<Json<Value>> {
    if b.n > 50 {
        return Err(bad("allow at most 50 at a time"));
    }
    let kind = store::resolve_request(&conn(&s, &f)?, &id, &b.action, b.n, unix_now())?
        .ok_or_else(|| not_found("open request"))?;
    if s.sink.enabled() {
        let state = s.clone();
        let resolution = match b.action.as_str() {
            "allow-channel" => "allowed-channel",
            "allow-more" => "allowed-more",
            _ => "dismissed",
        };
        tokio::task::spawn_blocking(move || {
            state.sink.push_count(
                "cubby_requests",
                &[("kind", &kind), ("resolution", resolution)],
            )
        });
    }
    Ok(Json(json!({"ok": true})))
}
