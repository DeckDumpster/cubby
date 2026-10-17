use std::io::{Read, Write};
use std::net::TcpListener;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cubby_server::{Config, app};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn config(web_dir: &std::path::Path, otlp: &str) -> Config {
    Config::from_lookup(|k| match k {
        "CUBBY_WEB_DIR" => Some(web_dir.display().to_string()),
        "CUBBY_OTLP_ENDPOINT" => Some(otlp.to_string()),
        _ => None,
    })
    .unwrap()
}

fn web_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "cubby-web-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("index.html"), "<html>cubby shell</html>").unwrap();
    d
}

async fn get(app: axum::Router, path: &str) -> (StatusCode, String) {
    let res = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn healthz_returns_200_ok_true() {
    let (status, body) = get(app(&config(&web_dir(), "")), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"ok":true}"#);
}

#[tokio::test]
async fn spa_shell_is_served_for_the_root_and_client_routes() {
    for path in ["/", "/parent", "/watch/deep/link"] {
        let (status, body) = get(app(&config(&web_dir(), "")), path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains("cubby shell"), "{path}");
    }
}

#[test]
fn defaults_port_8790_and_rejects_a_bad_port() {
    assert_eq!(Config::from_lookup(|_| None).unwrap().port, 8790);
    assert!(Config::from_lookup(|k| (k == "CUBBY_PORT").then(|| "x".into())).is_err());
}

#[tokio::test]
async fn a_request_sends_one_sample_to_the_otlp_sink() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let sink = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        while !got.windows(4).any(|w| w == b"\r\n\r\n") || got.len() < 200 {
            let n = s.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&got).into_owned()
    });
    let (status, _) = get(app(&config(&web_dir(), &addr)), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    let got = tokio::task::spawn_blocking(move || sink.join().unwrap())
        .await
        .unwrap();
    assert!(got.starts_with("POST /opentelemetry/v1/metrics"), "{got}");
    assert!(got.contains("cubby_http_request_seconds"), "{got}");
    assert!(got.contains("/healthz"), "{got}");
}

#[tokio::test]
async fn an_empty_endpoint_sends_nothing() {
    let (status, _) = get(app(&config(&web_dir(), "")), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
}
