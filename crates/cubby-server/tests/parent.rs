#![allow(dead_code)]
//! The parent API over HTTP, end to end: what a parent allows or blocks decides what a paired
//! child device may play.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cubby_db::Registry;
use cubby_server::Config;
use http_body_util::BodyExt;
use rusqlite::Connection;
use serde_json::{Value, json};
use tower::ServiceExt;

mod support;
use support::Provider;

const PARENT: &str = "mum@example.com";

struct Env {
    router: axum::Router,
    dir: PathBuf,
    family_id: String,
    provider: Provider,
    parent_cookie: std::sync::Mutex<Option<String>>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "cubby-id-{}-{:?}-{}",
        std::process::id(),
        std::thread::current().id(),
        now()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("index.html"), "<html>shell</html>").unwrap();
    d
}

async fn env_with(access: bool, otlp: &str) -> Env {
    env_full(access, otlp, None).await
}

async fn env_full(access: bool, otlp: &str, ingest: Option<cubby_server::Ingest>) -> Env {
    env_hooks(access, otlp, ingest, None).await
}

async fn env_hooks(
    access: bool,
    otlp: &str,
    ingest: Option<cubby_server::Ingest>,
    resolver: Option<cubby_server::ChannelResolver>,
) -> Env {
    let dir = temp_dir();
    let provider = Provider::start().await;
    let cfg = Config::from_lookup(|k| match k {
        "CUBBY_WEB_DIR" => Some(dir.display().to_string()),
        "CUBBY_DATA_DIR" => Some(dir.join("data").display().to_string()),
        "CUBBY_OTLP_ENDPOINT" => Some(otlp.into()),
        k if access => provider.env(k),
        _ => None,
    })
    .unwrap();
    let data = dir.join("data");
    let mut registry = Registry::open(&data).unwrap();
    let family_id = registry.create_family(PARENT).unwrap().id;
    drop(registry);
    let router = cubby_server::app_with_hooks(&cfg, ingest, resolver).unwrap();
    Env {
        router,
        dir,
        family_id,
        provider,
        parent_cookie: Default::default(),
    }
}

async fn env() -> Env {
    env_with(true, "").await
}

impl Env {
    fn family(&self) -> Connection {
        Registry::open(&self.dir.join("data"))
            .unwrap()
            .open_family(&self.family_id)
            .unwrap()
    }

    /// Signs `email` in through the fake provider and returns the session cookie.
    async fn cookie(&self, email: &str) -> String {
        support::session_cookie(&self.router, &self.provider, email).await
    }

    async fn parent_session(&self) -> String {
        let cached = self.parent_cookie.lock().unwrap().clone();
        match cached {
            Some(c) => c,
            None => {
                let c = self.cookie(PARENT).await;
                *self.parent_cookie.lock().unwrap() = Some(c.clone());
                c
            }
        }
    }

    async fn call(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn parent_post(&self, path: &str, jwt: Option<&str>) -> (StatusCode, Value) {
        let mut r = Request::post(path);
        if let Some(j) = jwt {
            r = r.header("cookie", j);
        }
        self.call(r.body(Body::empty()).unwrap()).await
    }

    async fn pair(&self, code: &str, ip: &str) -> (StatusCode, Value) {
        let r = Request::post("/api/child/pair")
            .header("content-type", "application/json")
            .header("cf-connecting-ip", ip)
            .body(Body::from(
                json!({"code": code, "label": "iPad"}).to_string(),
            ))
            .unwrap();
        self.call(r).await
    }

    async fn me(&self, token: Option<&str>) -> (StatusCode, Value) {
        let mut r = Request::get("/api/child/me");
        if let Some(t) = token {
            r = r.header("authorization", format!("Bearer {t}"));
        }
        self.call(r.body(Body::empty()).unwrap()).await
    }
}

impl Env {
    async fn parent(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut r = Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", self.parent_session().await);
        let b = match body {
            Some(b) => {
                r = r.header("content-type", "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        self.call(r.body(b).unwrap()).await
    }

    async fn child_play(&self, token: &str, vid: &str) -> Value {
        let r = Request::post("/api/child/play")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({"youtube_id": vid}).to_string()))
            .unwrap();
        self.call(r).await.1
    }

    /// Makes a child and pairs a device for it; returns (child id, device token).
    async fn child_with_device(&self) -> (String, String) {
        let (s, c) = self
            .parent(
                "POST",
                "/api/parent/children",
                Some(json!({"name": "Ada", "tz": "UTC"})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{c}");
        let id = c["id"].as_str().unwrap().to_string();
        let (_, code) = self
            .parent("POST", &format!("/api/parent/children/{id}/pairing"), None)
            .await;
        let (s, paired) = self.pair(code["code"].as_str().unwrap(), "9.9.9.9").await;
        assert_eq!(s, StatusCode::OK, "{paired}");
        (id, paired["token"].as_str().unwrap().to_string())
    }

    fn videos(&self) {
        self.family()
            .execute_batch(
                "INSERT INTO videos(youtube_id,title,channel_id,channel_title,duration_s,fetched_at) VALUES
                 ('v1','one','ch1','Chan',600,1),('v2','two','ch1','Chan',600,2),
                 ('w1','other','ch2','Other',600,3);",
            )
            .unwrap();
    }
}

fn granted(v: &Value) -> bool {
    v.get("grant_id").is_some()
}

#[tokio::test]
async fn allow_a_channel_block_one_video_and_the_channel_stays_allowed() {
    let e = env().await;
    e.videos();
    let (child, tok) = e.child_with_device().await;
    assert!(
        !granted(&e.child_play(&tok, "v1").await),
        "nothing is allowed yet"
    );

    let (s, p) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "https://www.youtube.com/channel/ch1"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert!(granted(&e.child_play(&tok, "v1").await));
    assert!(granted(&e.child_play(&tok, "v2").await));
    assert!(!granted(&e.child_play(&tok, "w1").await));

    let (s, _) = e
        .parent("POST", "/api/parent/videos/v1/block", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(e.child_play(&tok, "v1").await["refused"], "not-allowed");
    assert!(
        granted(&e.child_play(&tok, "v2").await),
        "the channel stays allowed"
    );
    let (_, pols) = e.parent("GET", "/api/parent/policies", None).await;
    assert_eq!(pols.as_array().unwrap().len(), 1);

    let (s, _) = e
        .parent("POST", "/api/parent/videos/v1/unblock", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(granted(&e.child_play(&tok, "v1").await));
    let _ = child;
}

#[tokio::test]
async fn the_library_lists_groups_then_a_groups_videos_and_searches_them() {
    let e = env().await;
    e.videos();
    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "youtube.com/channel/ch1"})),
    )
    .await;
    let (s, lib) = e.parent("GET", "/api/parent/library", None).await;
    assert_eq!(s, StatusCode::OK, "{lib}");
    assert_eq!(lib["groups"].as_array().unwrap().len(), 1);
    assert_eq!(lib["groups"][0]["key"], "ch1");
    assert_eq!(lib["groups"][0]["count"], 2);
    let (_, vids) = e
        .parent(
            "GET",
            "/api/parent/library/videos?group=ch1&sort=plays",
            None,
        )
        .await;
    assert_eq!(vids["total"], 2);
    assert_eq!(vids["videos"][0]["plays"], 0);
    let (s, by_title) = e
        .parent(
            "GET",
            "/api/parent/library/videos?group=ch1&sort=title",
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(by_title["total"], 2);
    let (s, _) = e
        .parent("GET", "/api/parent/library/videos?group=ch1&sort=zz", None)
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, none) = e
        .parent("GET", "/api/parent/library?q=nomatchatall", None)
        .await;
    assert!(none["groups"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_library_videos_carry_their_duration_or_null() {
    let e = env().await;
    e.videos();
    e.family()
        .execute("UPDATE videos SET duration_s=423 WHERE youtube_id='v1'", [])
        .unwrap();
    e.family()
        .execute(
            "UPDATE videos SET duration_s=NULL WHERE youtube_id='v2'",
            [],
        )
        .unwrap();
    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "youtube.com/channel/ch1"})),
    )
    .await;
    let (_, vids) = e
        .parent(
            "GET",
            "/api/parent/library/videos?group=ch1&sort=title",
            None,
        )
        .await;
    assert_eq!(vids["videos"][0]["duration_s"], 423);
    assert!(vids["videos"][1]["duration_s"].is_null());
}

#[tokio::test]
async fn approving_a_more_from_channel_request_adds_a_policy_and_opens_the_channel() {
    let e = env().await;
    e.videos();
    let (child, tok) = e.child_with_device().await;
    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "https://youtu.be/v1", "child_id": child})),
    )
    .await;
    assert!(!granted(&e.child_play(&tok, "v2").await));
    e.family()
        .execute(
            "INSERT INTO requests(id,child_id,kind,youtube_id,created_at) VALUES ('r1',?1,'more-from-channel','v1',1)",
            [&child],
        )
        .unwrap();

    let (_, inbox) = e.parent("GET", "/api/parent/requests", None).await;
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    assert_eq!(inbox[0]["channel_title"], "Chan");

    let (s, b) = e
        .parent(
            "POST",
            "/api/parent/requests/r1/resolve",
            Some(json!({"action": "allow-channel"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(granted(&e.child_play(&tok, "v2").await));
    let (_, pols) = e.parent("GET", "/api/parent/policies", None).await;
    assert_eq!(pols.as_array().unwrap().len(), 2);
    let (_, inbox) = e.parent("GET", "/api/parent/requests", None).await;
    assert!(inbox.as_array().unwrap().is_empty());
    let (s, _) = e
        .parent(
            "POST",
            "/api/parent/requests/r1/resolve",
            Some(json!({"action": "dismiss"})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a resolved request is gone");
}

#[tokio::test]
async fn the_review_list_shows_what_policies_added_and_approving_clears_it() {
    let e = env().await;
    e.videos();
    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "youtube.com/channel/ch1"})),
    )
    .await;
    let (_, all) = e.parent("GET", "/api/parent/review", None).await;
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert_eq!(all[0]["policy_youtube_id"], "ch1");
    let (_, none) = e
        .parent("GET", "/api/parent/review?channel=ch2", None)
        .await;
    assert!(none.as_array().unwrap().is_empty());
    e.parent("POST", "/api/parent/videos/v1/approve", Some(json!({})))
        .await;
    let (_, left) = e.parent("GET", "/api/parent/review", None).await;
    assert_eq!(left.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_url_that_is_not_youtube_is_refused_and_a_preview_names_the_policy() {
    let e = env().await;
    let (s, _) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "https://example.com/x"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, p) = e
        .parent(
            "POST",
            "/api/parent/policies/preview",
            Some(json!({"url": "https://www.youtube.com/@Bluey"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (p["kind"].as_str(), p["youtube_id"].as_str()),
        (Some("channel"), Some("@Bluey"))
    );
}

#[tokio::test]
async fn removing_a_policy_stops_play() {
    let e = env().await;
    e.videos();
    let (_, tok) = e.child_with_device().await;
    let (_, p) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "youtube.com/channel/ch1"})),
        )
        .await;
    let id = p["id"].as_str().unwrap();
    assert!(granted(&e.child_play(&tok, "v1").await));
    let (s, _) = e
        .parent("DELETE", &format!("/api/parent/policies/{id}"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(!granted(&e.child_play(&tok, "v1").await));
    let (s, _) = e
        .parent("DELETE", &format!("/api/parent/policies/{id}"), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn children_budgets_devices_and_history_are_reachable() {
    let e = env().await;
    e.videos();
    let (child, tok) = e.child_with_device().await;
    let (s, b) = e
        .parent(
            "PUT",
            &format!("/api/parent/children/{child}/budget"),
            Some(json!({"video_equivalents": 2.0, "minutes_cap": 45,
                "cutoff_mid_play": false, "daily_windows": [["00:00", "23:59"]]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, _) = e
        .parent(
            "PUT",
            &format!("/api/parent/children/{child}/budget"),
            Some(json!({"daily_windows": [["25:00", "19:00"]]})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "bad windows are refused");
    let (s, _) = e
        .parent(
            "PUT",
            &format!("/api/parent/children/{child}"),
            Some(json!({"name": "Ada", "avatar": "x", "tz": "Not/AZone"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "bad tz is refused");

    let (_, kids) = e.parent("GET", "/api/parent/children", None).await;
    assert_eq!(kids[0]["budget"]["minutes_cap"], 45);
    assert_eq!(kids[0]["budget"]["daily_windows"][0][0], "00:00");
    assert_eq!(kids[0]["devices"].as_array().unwrap().len(), 1);

    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "youtube.com/channel/ch1"})),
    )
    .await;
    assert!(granted(&e.child_play(&tok, "v1").await));
    let (_, h) = e
        .parent(
            "GET",
            &format!("/api/parent/children/{child}/history"),
            None,
        )
        .await;
    assert_eq!(h[0]["title"], "one");

    let dev = kids[0]["devices"][0]["id"].as_str().unwrap();
    e.parent("POST", &format!("/api/parent/devices/{dev}/revoke"), None)
        .await;
    let (_, kids) = e.parent("GET", "/api/parent/children", None).await;
    assert!(kids[0]["devices"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_parent_api_needs_a_parent() {
    let e = env().await;
    let r = Request::get("/api/parent/policies")
        .body(Body::empty())
        .unwrap();
    assert_eq!(e.call(r).await.0, StatusCode::UNAUTHORIZED);
}

fn fixture_ingest() -> cubby_server::Ingest {
    std::sync::Arc::new(|conn: &Connection| {
        let mut t = cubby_yt::FixtureTransport::default();
        t.add(
            "videos",
            &[
                ("part", "snippet,contentDetails,status"),
                ("id", "yebNIHKAC4A"),
            ],
            json!({"items": [{"id": "yebNIHKAC4A",
                "snippet": {"title": "Allowed now", "channelId": "UCx", "channelTitle": "Chan",
                    "tags": [], "thumbnails": {}},
                "contentDetails": {"duration": "PT4M13S"},
                "status": {"privacyStatus": "public", "madeForKids": true}}]}),
        );
        cubby_worker::run_pass(conn, &t, now(), None);
    })
}

/// What the child's home and search read from: the `playable` set for `child`.
fn playable(e: &Env, child: &str) -> usize {
    cubby_db::browse::playable_videos(&e.family(), child)
        .unwrap()
        .len()
}

#[tokio::test]
async fn an_allowed_video_reaches_the_child_without_a_worker_tick() {
    let e = env_full(true, "", Some(fixture_ingest())).await;
    let (child, _) = e.child_with_device().await;
    assert_eq!(playable(&e, &child), 0);
    let (s, p) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "https://youtu.be/yebNIHKAC4A"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(playable(&e, &child), 1, "the child sees it at once");
    let (_, pols) = e.parent("GET", "/api/parent/policies", None).await;
    assert_eq!(pols[0]["pending"], false);
    assert_eq!(pols[0]["title"], "Allowed now");
}

#[tokio::test]
async fn without_ingest_an_allowed_video_is_pending_until_the_worker_runs() {
    let e = env().await;
    let (s, _) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "https://youtu.be/yebNIHKAC4A"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, pols) = e.parent("GET", "/api/parent/policies", None).await;
    assert_eq!(pols[0]["pending"], true);
}

#[tokio::test]
async fn the_parents_endpoint_lists_every_parent_of_the_family() {
    let e = env().await;
    Registry::open(&e.dir.join("data"))
        .unwrap()
        .add_parent(&e.family_id, "kim@gmail.com")
        .unwrap();
    let (s, body) = e.parent("GET", "/api/parent/parents", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, json!(["kim@gmail.com", PARENT]));
}

/// A resolver backed by a fake Data API that knows one channel, by handle and by username.
fn fixture_resolver() -> cubby_server::ChannelResolver {
    std::sync::Arc::new(
        |conn: &Connection, l: &cubby_core::policy_url::ChannelLookup| {
            let mut t = cubby_yt::FixtureTransport::default();
            let found = json!({"items": [{"id": "UCxgpotnLEd1OeAo5t8f2XLQ",
            "snippet": {"title": "Skye's Pup Squad"}}]});
            t.add(
                "channels",
                &[("part", "snippet"), ("forHandle", "@SkyesPupSquad")],
                found.clone(),
            );
            t.add(
                "channels",
                &[("part", "snippet"), ("forUsername", "skyeold")],
                found,
            );
            t.add(
                "channels",
                &[("part", "snippet"), ("forHandle", "@nobodyhere")],
                json!({"pageInfo": {"totalResults": 0}}),
            );
            cubby_worker::lookup_channel(conn, &t, now(), l).map_err(|e| e.to_string())
        },
    )
}

#[tokio::test]
async fn a_channel_added_by_handle_is_stored_under_its_uc_id() {
    let e = env_hooks(true, "", None, Some(fixture_resolver())).await;
    for url in [
        "https://www.youtube.com/@SkyesPupSquad",
        "https://www.youtube.com/c/SkyesPupSquad",
        "https://www.youtube.com/user/skyeold",
    ] {
        let (s, p) = e
            .parent("POST", "/api/parent/policies", Some(json!({"url": url})))
            .await;
        assert_eq!(s, StatusCode::OK, "{url}: {p}");
        assert_eq!(p["youtube_id"], "UCxgpotnLEd1OeAo5t8f2XLQ", "{url}");
        assert_eq!(p["title"], "Skye's Pup Squad", "{url}");
    }
    let (_, pols) = e.parent("GET", "/api/parent/policies", None).await;
    assert_eq!(pols.as_array().unwrap().len(), 1, "one channel, one policy");
    assert_eq!(pols[0]["youtube_id"], "UCxgpotnLEd1OeAo5t8f2XLQ");
    assert_eq!(pols[0]["title"], "@SkyesPupSquad");
}

#[tokio::test]
async fn the_preview_shows_the_channel_title() {
    let e = env_hooks(true, "", None, Some(fixture_resolver())).await;
    let (s, p) = e
        .parent(
            "POST",
            "/api/parent/policies/preview",
            Some(json!({"url": "https://www.youtube.com/@SkyesPupSquad"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["title"], "Skye's Pup Squad");
    assert_eq!(p["youtube_id"], "UCxgpotnLEd1OeAo5t8f2XLQ");
}

#[tokio::test]
async fn an_unknown_handle_is_a_400_with_a_message_and_stores_nothing() {
    let e = env_hooks(true, "", None, Some(fixture_resolver())).await;
    for path in ["/api/parent/policies", "/api/parent/policies/preview"] {
        let (s, p) = e
            .parent(
                "POST",
                path,
                Some(json!({"url": "https://www.youtube.com/@nobodyhere"})),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{path}: {p}");
        assert_eq!(p["error"], "no YouTube channel was found for @nobodyhere");
    }
    let (_, pols) = e.parent("GET", "/api/parent/policies", None).await;
    assert_eq!(pols.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn progress_reports_used_and_left_against_the_budget_and_who_is_watching() {
    let e = env().await;
    e.videos();
    let (child, _tok) = e.child_with_device().await;
    let (s, _) = e
        .parent(
            "PUT",
            &format!("/api/parent/children/{child}/budget"),
            Some(json!({"video_equivalents": 3.0, "minutes_cap": 60})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let t = now();
    let tz: String = e
        .family()
        .query_row("SELECT tz FROM children LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let today = cubby_core::budget::day_start(&tz, t);
    let dev: String = e
        .family()
        .query_row("SELECT id FROM devices LIMIT 1", [], |r| r.get(0))
        .unwrap();
    e.family()
        .execute(
            "INSERT INTO play_grants(id,child_id,device_id,youtube_id,granted_at,last_beat_at,watched_s,charge,ended_at,end_reason) VALUES
             ('g1',?1,?2,'v1',?3,?3,300,0.5,?3,'ended'),
             ('g2',?1,?2,'v2',?4,?4,600,1.0,?4,'ended')",
            rusqlite::params![child, dev, today + 1, today + 2],
        )
        .unwrap();

    let (s, p) = e.parent("GET", "/api/parent/progress", None).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let k = &p[0];
    assert_eq!(k["child_id"], child.as_str());
    assert_eq!(k["videos"]["limit"], 3.0);
    assert_eq!(k["videos"]["used"], 1.5);
    assert_eq!(k["videos"]["left"], 1.5);
    assert_eq!(k["minutes"]["limit"], 60.0);
    assert_eq!(k["minutes"]["used"], 15.0);
    assert_eq!(k["minutes"]["left"], 45.0);
    assert_eq!(
        k["resets_at"],
        cubby_core::budget::next_midnight(&tz, t),
        "the reset is the next local midnight, not a rolling 24 h"
    );
    assert_eq!(k["window"]["restricted"], false);
    assert!(k["watching"].is_null());

    e.family()
        .execute(
            "INSERT INTO play_grants(id,child_id,device_id,youtube_id,granted_at,last_beat_at,watched_s,charge)
             VALUES ('g3',?1,?2,'v1',?3,?4,120,0.2)",
            rusqlite::params![child, dev, t - 130, t - 5],
        )
        .unwrap();
    let (_, p) = e.parent("GET", "/api/parent/progress", None).await;
    assert_eq!(p[0]["watching"]["title"], "one");
    assert_eq!(p[0]["watching"]["minutes"], 2.0);
}

#[tokio::test]
async fn progress_says_no_limit_for_unlimited_budgets() {
    let e = env().await;
    let (_child, _tok) = e.child_with_device().await;
    let (_, p) = e.parent("GET", "/api/parent/progress", None).await;
    assert!(p[0]["videos"]["limit"].is_null());
    assert!(p[0]["videos"]["left"].is_null());
    assert!(p[0]["minutes"]["limit"].is_null());
    assert!(p[0]["resets_at"].is_null());
}

fn tile_ids(v: &Value) -> Vec<String> {
    let mut out = vec![];
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(id) = m.get("youtube_id").or(m.get("id")).and_then(Value::as_str) {
                    out.push(id.to_string());
                }
                m.values().for_each(|x| walk(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(v, &mut out);
    out.sort();
    out.dedup();
    out
}

#[tokio::test]
async fn channel_rules_allow_only_the_episode_count_live_and_judge_new_uploads() {
    let e = env().await;
    e.family()
        .execute_batch(
            "INSERT INTO videos(youtube_id,title,channel_id,channel_title,duration_s,fetched_at) VALUES
             ('short','Clip train','chr','Rails',45,1),('ep','Episode train','chr','Rails',1500,2),
             ('comp','Train compilation','chr','Rails',5400,3);",
        )
        .unwrap();
    let (_, tok) = e.child_with_device().await;
    let (s, p) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "https://www.youtube.com/channel/chr"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let pid = p["id"].as_str().unwrap().to_string();
    let rules = json!({"exclude_shorts": true, "max_minutes": 30});

    // The live count before saving, with the excluded list; nothing changes for the child yet.
    let (s, n) = e
        .parent(
            "POST",
            &format!("/api/parent/policies/{pid}/rules/preview"),
            Some(rules.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{n}");
    assert_eq!(
        (n["allowed"].clone(), n["excluded"].clone()),
        (json!(1), json!(2))
    );
    assert_eq!(tile_ids(&n["excluded_videos"]), ["comp", "short"]);
    assert!(granted(&e.child_play(&tok, "short").await));

    let (s, saved) = e
        .parent(
            "PUT",
            &format!("/api/parent/policies/{pid}/rules"),
            Some(rules),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    assert_eq!(saved["counts"]["excluded"], 2);
    let (_, pols) = e.parent("GET", "/api/parent/policies", None).await;
    assert_eq!(pols[0]["rules"]["max_minutes"], 30.0);
    assert_eq!(pols[0]["excluded_count"], 2);

    // The child no longer sees or plays the excluded videos, in the grid or in search.
    let get = |path: &'static str| {
        let r = Request::get(path)
            .header("authorization", format!("Bearer {tok}"))
            .body(Body::empty())
            .unwrap();
        e.call(r)
    };
    assert_eq!(tile_ids(&get("/api/child/home").await.1), ["ep"]);
    assert_eq!(tile_ids(&get("/api/child/search?q=train").await.1), ["ep"]);
    assert_eq!(e.child_play(&tok, "comp").await["refused"], "not-allowed");
    assert!(granted(&e.child_play(&tok, "ep").await));

    // A later upload is judged when it arrives: the matching one shows, the other stays hidden.
    e.family()
        .execute_batch(&format!(
            "INSERT INTO videos(youtube_id,title,channel_id,channel_title,duration_s,fetched_at) VALUES
             ('new-ok','Train 5','chr','Rails',1200,9),('new-long','Train movie','chr','Rails',7200,9);
             INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at) VALUES
             ('new-ok',NULL,'{pid}','auto-allowed',9),('new-long',NULL,'{pid}','auto-allowed',9);"
        ))
        .unwrap();
    assert_eq!(tile_ids(&get("/api/child/home").await.1), ["ep", "new-ok"]);

    // Allowing one excluded video outright overrides the rules.
    let (s, _) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "https://www.youtube.com/watch?v=comp"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(granted(&e.child_play(&tok, "comp").await));

    let (s, _) = e
        .parent("PUT", "/api/parent/policies/nope/rules", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = e
        .parent(
            "PUT",
            &format!("/api/parent/policies/{pid}/rules"),
            Some(json!({"max_minutes": -1})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_extra_grant_raises_the_limits_and_is_listed_with_who_gave_it() {
    let e = env().await;
    let (child, _tok) = e.child_with_device().await;
    e.parent(
        "PUT",
        &format!("/api/parent/children/{child}/budget"),
        Some(json!({"video_equivalents": 3.0, "minutes_cap": 60})),
    )
    .await;
    let path = format!("/api/parent/children/{child}/extra");
    let (s, _) = e.parent("POST", &path, Some(json!({"minutes": 15}))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = e
        .parent("POST", &path, Some(json!({"videos": 1, "note": "chores"})))
        .await;
    assert_eq!(s, StatusCode::OK);

    let (_, p) = e.parent("GET", "/api/parent/progress", None).await;
    let k = &p[0];
    assert_eq!(k["videos"]["limit"], 4.0);
    assert_eq!(k["minutes"]["limit"], 75.0);
    assert_eq!(k["grants"].as_array().unwrap().len(), 2);
    assert_eq!(k["grants"][0]["note"], "chores");
    assert_eq!(k["grants"][0]["by"], PARENT);
    assert!(k["grants"][0]["expires_at"].as_i64().unwrap() > now());
}

#[tokio::test]
async fn an_empty_or_absurd_extra_grant_is_refused() {
    let e = env().await;
    let (child, _tok) = e.child_with_device().await;
    let path = format!("/api/parent/children/{child}/extra");
    for body in [json!({}), json!({"minutes": -5}), json!({"videos": 5000})] {
        let (s, _) = e.parent("POST", &path, Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
    let (s, _) = e
        .parent(
            "POST",
            "/api/parent/children/nobody/extra",
            Some(json!({"videos": 1})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_child_request_reaches_the_parent_inbox_once() {
    let e = env().await;
    e.videos();
    let (child, tok) = e.child_with_device().await;
    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "https://youtu.be/v1", "child_id": child})),
    )
    .await;
    let post = |tok: String, body: Value| {
        let e = &e;
        async move {
            let r = Request::post("/api/child/request")
                .header("authorization", format!("Bearer {tok}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            e.call(r).await
        }
    };
    let (s, a) = post(tok.clone(), json!({"kind": "like", "youtube_id": "v1"})).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let (s, b) = post(tok.clone(), json!({"kind": "like", "youtube_id": "v1"})).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(a["id"], b["id"], "a duplicate open request is the same row");
    let (_, inbox) = e.parent("GET", "/api/parent/requests", None).await;
    assert_eq!(inbox.as_array().unwrap().len(), 1, "{inbox}");
    assert_eq!(inbox[0]["kind"], "like");
    assert_eq!(inbox[0]["youtube_id"], "v1");
    assert_eq!(inbox[0]["child_name"], "Ada");

    let (s, _) = post(tok.clone(), json!({"kind": "like", "youtube_id": "w1"})).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a video the child may not watch");
    let (s, _) = post(tok.clone(), json!({"kind": "nope", "youtube_id": "v1"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = post("bad".into(), json!({"kind": "like", "youtube_id": "v1"})).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (_, inbox) = e.parent("GET", "/api/parent/requests", None).await;
    assert_eq!(inbox.as_array().unwrap().len(), 1);
}

impl Env {
    async fn child_get(&self, token: &str, path: &str) -> (StatusCode, Value) {
        let r = Request::get(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        self.call(r).await
    }
}

#[tokio::test]
async fn once_the_grace_is_over_the_child_api_refuses_browsing_until_a_grant() {
    let e = env().await;
    e.videos();
    let (child, tok) = e.child_with_device().await;
    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "https://www.youtube.com/channel/ch1"})),
    )
    .await;
    let (s, _) = e
        .parent(
            "PUT",
            &format!("/api/parent/children/{child}/budget"),
            Some(json!({"video_equivalents": 0.0})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    // Budget spent: browsing works for the grace, with the countdown, and play is refused.
    let (s, home) = e.child_get(&tok, "/api/child/home").await;
    assert_eq!(s, StatusCode::OK, "{home}");
    assert_eq!(home["limit"]["state"], "grace");
    assert!(home["limit"]["grace_left_s"].as_i64().unwrap() > 100);
    assert_eq!(e.child_play(&tok, "v1").await["refused"], "out-of-videos");
    assert_eq!(
        e.child_get(&tok, "/api/child/search?q=one").await.0,
        StatusCode::OK
    );

    // The grace is a family setting.
    let (s, _) = e
        .parent(
            "PUT",
            "/api/parent/settings",
            Some(json!({"grace_minutes": 5})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, st) = e.parent("GET", "/api/parent/settings", None).await;
    assert_eq!(st["grace_minutes"], 5.0);
    let (_, home) = e.child_get(&tok, "/api/child/home").await;
    assert!(home["limit"]["grace_left_s"].as_i64().unwrap() > 200);
    let (s, _) = e
        .parent(
            "PUT",
            "/api/parent/settings",
            Some(json!({"grace_minutes": 99})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Ten minutes on, the grace is over: every browse call is refused, on any device.
    e.family()
        .execute("UPDATE limit_reached SET reached_at = reached_at - 600", [])
        .unwrap();
    for p in [
        "/api/child/home",
        "/api/child/search?q=one",
        "/api/child/next?after=v1",
        "/api/child/recent",
    ] {
        let (s, v) = e.child_get(&tok, p).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{p}");
        assert_eq!(v["error"], "locked");
        assert_eq!(v["limit"]["state"], "locked");
    }

    // A parent grant lifts it at once.
    let (s, _) = e
        .parent(
            "POST",
            &format!("/api/parent/children/{child}/extra"),
            Some(json!({"videos": 1})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, home) = e.child_get(&tok, "/api/child/home").await;
    assert_eq!(s, StatusCode::OK, "{home}");
    assert_eq!(home["limit"]["state"], "open");
}

#[tokio::test]
async fn removing_a_grant_appends_a_reversal_and_lowers_the_limits() {
    let e = env().await;
    let (child, _tok) = e.child_with_device().await;
    e.parent(
        "PUT",
        &format!("/api/parent/children/{child}/budget"),
        Some(json!({"video_equivalents": 3.0, "minutes_cap": 60})),
    )
    .await;
    let path = format!("/api/parent/children/{child}/extra");
    e.parent("POST", &path, Some(json!({"minutes": 15}))).await;
    let (_, g) = e.parent("POST", &path, Some(json!({"videos": 1}))).await;
    let gid = g["id"].as_str().unwrap().to_string();

    let (s, _) = e.parent("DELETE", &format!("{path}/{gid}"), None).await;
    assert_eq!(s, StatusCode::OK);
    // Removing twice is a no-op.
    let (s, _) = e.parent("DELETE", &format!("{path}/{gid}"), None).await;
    assert_eq!(s, StatusCode::OK);

    let (_, p) = e.parent("GET", "/api/parent/progress", None).await;
    let k = &p[0];
    assert_eq!(k["videos"]["limit"], 3.0);
    assert_eq!(k["minutes"]["limit"], 75.0);
    let grants = k["grants"].as_array().unwrap();
    assert_eq!(grants.len(), 2);
    let removed: Vec<_> = grants.iter().filter(|g| g["removed_at"].is_i64()).collect();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0]["id"], gid);
    assert_eq!(removed[0]["removed_by"], PARENT);
    assert_eq!(removed[0]["videos"], 1.0);
}

#[tokio::test]
async fn a_grant_that_is_not_this_childs_cannot_be_removed() {
    let e = env().await;
    let (child, _tok) = e.child_with_device().await;
    let path = format!("/api/parent/children/{child}/extra");
    let (s, _) = e.parent("DELETE", &format!("{path}/nonesuch"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = e
        .parent("DELETE", "/api/parent/children/nobody/extra/nonesuch", None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
