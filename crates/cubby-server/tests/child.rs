use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cubby_core::rank::{ChildCtx, Ranker, VideoId, VideoMeta};
use cubby_db::browse::fixture::{add_video, allow, open};
use cubby_server::child::{ChildApi, router};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Leaky;
impl Ranker for Leaky {
    fn rank(&self, _: &ChildCtx, _: Option<&VideoMeta>, c: &[VideoMeta]) -> Vec<VideoId> {
        let mut out = vec!["blocked".to_string(), "sibling".into(), "ghost".into()];
        out.extend(c.iter().map(|v| v.id.clone()));
        out
    }
}

fn api(leaky: bool) -> axum::Router {
    let c = open();
    add_video(&c, "a", "Blue train", "Trains", "[]", 0);
    add_video(&c, "b", "Red train", "Trains", "[]", 5);
    add_video(&c, "blocked", "Blue blocked train", "Trains", "[]", 0);
    add_video(&c, "sibling", "Blue sibling train", "Trains", "[]", 0);
    allow(&c, "a", "kid", "approved");
    allow(&c, "b", "kid", "auto-allowed");
    allow(&c, "blocked", "kid", "approved");
    allow(&c, "blocked", "kid", "blocked");
    allow(&c, "sibling", "sib", "approved");
    let mut api = ChildApi::new(
        c,
        Arc::new(|h| {
            h.get("authorization")
                .and_then(|v| v.to_str().ok())
                .filter(|v| *v == "Bearer kid")
                .map(|_| "kid".to_string())
        }),
        Arc::new(|_, _| json!({"videos_left": 3})),
    );
    if leaky {
        api.ranker = Arc::new(Leaky);
    }
    router(api)
}

async fn get(app: axum::Router, path: &str, token: bool) -> (StatusCode, Value) {
    let mut req = Request::get(path);
    if token {
        req = req.header("authorization", "Bearer kid");
    }
    let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn ids(tiles: &Value) -> Vec<String> {
    tiles
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["youtube_id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn child_routes_refuse_an_unauthenticated_call() {
    for p in [
        "/api/child/home",
        "/api/child/search?q=x",
        "/api/child/next?after=a",
    ] {
        assert_eq!(
            get(api(false), p, false).await.0,
            StatusCode::UNAUTHORIZED,
            "{p}"
        );
    }
}

#[tokio::test]
async fn next_names_only_playable_videos_even_for_a_leaky_ranker() {
    let (st, body) = get(api(true), "/api/child/next?after=a", true).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ids(&body["tiles"]), ["b"]);
    assert_eq!(body["remaining"]["videos_left"], 3);
}

#[tokio::test]
async fn home_sections_hold_only_playable_videos() {
    let (st, body) = get(api(true), "/api/child/home", true).await;
    assert_eq!(st, StatusCode::OK);
    let sections = body["sections"].as_array().unwrap();
    let keys: Vec<&str> = sections
        .iter()
        .map(|s| s["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["continue", "liked_channels", "new", "all"]);
    for s in sections {
        for id in ids(&s["tiles"]) {
            assert!(id == "a" || id == "b", "{id} leaked into {}", s["key"]);
        }
    }
    assert_eq!(ids(&sections[3]["tiles"]), ["a", "b"]);
    assert_eq!(ids(&sections[2]["tiles"]), ["a", "b"]);
}

#[tokio::test]
async fn search_returns_only_playable_videos() {
    let (st, body) = get(api(true), "/api/child/search?q=blue", true).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ids(&body["tiles"]), ["a"]);
    let (_, none) = get(api(false), "/api/child/search?q=", true).await;
    assert!(ids(&none["tiles"]).is_empty());
}

fn shape(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), shape(v))).collect()),
        Value::Array(a) => Value::Array(a.first().map(shape).into_iter().collect()),
        _ => Value::Null,
    }
}

fn fixture(name: &str) -> Value {
    let p = format!("{}/../../web/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

// The web page's parsers are tested against these same fixtures (web/tests/child.test.js),
// so renaming a field on either side fails one of the two.
#[tokio::test]
async fn home_and_search_json_match_the_fixtures_the_page_parses() {
    let (_, home) = get(api(false), "/api/child/home", true).await;
    let new = home["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == "new")
        .unwrap();
    assert_eq!(shape(new), shape(&fixture("child-home.json")["sections"][0]));
    let (_, search) = get(api(false), "/api/child/search?q=blue", true).await;
    assert_eq!(shape(&search), shape(&fixture("child-search.json")));
}
