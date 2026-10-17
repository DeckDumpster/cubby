//! Fire-and-forget OTLP metrics pushed directly to VictoriaMetrics. Each sample is a timestamped
//! Gauge (the store drops delta sums and histograms), protobuf over plain HTTP, every phase under
//! one deadline. Adapted from the Spira harness's spira-otel; not a dependency on it.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::{
    AnyValue, InstrumentationScope, KeyValue, any_value::Value,
};
use opentelemetry_proto::tonic::metrics::v1::{
    Gauge, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, metric::Data,
    number_data_point::Value as Num,
};
use opentelemetry_proto::tonic::resource::v1::Resource;
use prost::Message;

#[cfg(any(test, feature = "testing"))]
pub mod fake;

pub const PATH: &str = "/opentelemetry/v1/metrics";

pub struct MetricsSink {
    endpoint: Option<String>,
    service: String,
    deadline: Duration,
}

impl MetricsSink {
    /// `endpoint` is `http://host:port` (or `host:port`); empty or `None` disables the sink.
    pub fn new(endpoint: Option<&str>, service: &str) -> MetricsSink {
        let endpoint = endpoint.map(str::trim).filter(|e| !e.is_empty()).map(|e| {
            e.trim_start_matches("http://")
                .trim_end_matches('/')
                .to_string()
        });
        MetricsSink {
            endpoint,
            service: service.to_string(),
            deadline: Duration::from_secs(2),
        }
    }

    pub fn enabled(&self) -> bool {
        self.endpoint.is_some()
    }

    /// Pushes one duration sample, in seconds, of metric `name` carrying `labels`. A failure is
    /// swallowed: telemetry never changes the producer's behaviour.
    pub fn push_seconds(&self, name: &str, seconds: f64, labels: &[(&str, &str)]) {
        let _ = self.try_push_seconds(name, seconds, labels);
    }

    /// Pushes one event of counter-style metric `name` carrying `labels` (a gauge of 1).
    pub fn push_count(&self, name: &str, labels: &[(&str, &str)]) {
        let Some(addr) = &self.endpoint else {
            return;
        };
        let body = self
            .encode_unit(name, 1.0, "1", labels, SystemTime::now())
            .encode_to_vec();
        let _ = post(addr, &body, self.deadline);
    }

    /// Pushes one sample of metric `name` in `unit` carrying `labels`; failures are swallowed.
    pub fn push_value(&self, name: &str, value: f64, unit: &str, labels: &[(&str, &str)]) {
        let Some(addr) = &self.endpoint else {
            return;
        };
        let body = self
            .encode_unit(name, value, unit, labels, SystemTime::now())
            .encode_to_vec();
        let _ = post(addr, &body, self.deadline);
    }

    /// `cubby_requests{kind,resolution}`: a child's ask and how it was resolved.
    pub fn push_request(&self, kind: &str, resolution: &str) {
        self.push_count(
            "cubby_requests",
            &[("kind", kind), ("resolution", resolution)],
        );
    }

    /// `cubby_yt_quota_units{call}`: YouTube Data API quota spent by one call.
    pub fn push_yt_quota(&self, call: &str, units: u32) {
        self.push_value(
            "cubby_yt_quota_units",
            f64::from(units),
            "1",
            &[("call", call)],
        );
    }

    pub fn try_push_seconds(
        &self,
        name: &str,
        seconds: f64,
        labels: &[(&str, &str)],
    ) -> std::io::Result<()> {
        let Some(addr) = &self.endpoint else {
            return Ok(());
        };
        let body = self
            .encode(name, seconds, labels, SystemTime::now())
            .encode_to_vec();
        post(addr, &body, self.deadline)
    }

    pub fn encode(
        &self,
        name: &str,
        seconds: f64,
        labels: &[(&str, &str)],
        now: SystemTime,
    ) -> ExportMetricsServiceRequest {
        self.encode_unit(name, seconds, "s", labels, now)
    }

    pub fn encode_unit(
        &self,
        name: &str,
        value: f64,
        unit: &str,
        labels: &[(&str, &str)],
        now: SystemTime,
    ) -> ExportMetricsServiceRequest {
        let t = now
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos() as u64;
        let point = NumberDataPoint {
            attributes: labels.iter().map(|(k, v)| kv(k, v)).collect(),
            time_unix_nano: t,
            value: Some(Num::AsDouble(value)),
            ..Default::default()
        };
        let metric = Metric {
            name: name.into(),
            unit: unit.into(),
            data: Some(Data::Gauge(Gauge {
                data_points: vec![point],
            })),
            ..Default::default()
        };
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![kv("service.name", &self.service)],
                    ..Default::default()
                }),
                scope_metrics: vec![ScopeMetrics {
                    scope: Some(InstrumentationScope {
                        name: "cubby-otel".into(),
                        ..Default::default()
                    }),
                    metrics: vec![metric],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }
}

fn kv(k: &str, v: &str) -> KeyValue {
    KeyValue {
        key: k.into(),
        value: Some(AnyValue {
            value: Some(Value::StringValue(v.into())),
        }),
    }
}

fn post(addr: &str, body: &[u8], deadline: Duration) -> std::io::Result<()> {
    let end = Instant::now() + deadline;
    let left = || {
        end.checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "deadline"))
    };
    let target = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::other("no address"))?;
    let mut s = TcpStream::connect_timeout(&target, left()?)?;
    s.set_write_timeout(Some(left()?))?;
    let head = format!(
        "POST {PATH} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/x-protobuf\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    s.set_read_timeout(Some(left()?))?;
    let mut status = [0u8; 12];
    s.read_exact(&mut status)?;
    match &status[9..12] {
        b"200" | b"202" | b"204" => Ok(()),
        other => Err(std::io::Error::other(format!(
            "status {}",
            String::from_utf8_lossy(other)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A fake OTLP sink: answers 200 to one request and returns what it received.
    fn sink() -> (String, std::thread::JoinHandle<Vec<u8>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                if let Some(i) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&got[..i]).to_ascii_lowercase();
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if got.len() >= i + 4 + len {
                        break;
                    }
                }
            }
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            got
        });
        (addr, h)
    }

    #[test]
    fn an_empty_endpoint_is_disabled_and_pushes_nothing() {
        for e in [None, Some(""), Some("  ")] {
            let s = MetricsSink::new(e, "cubby-server");
            assert!(!s.enabled());
            s.try_push_seconds("cubby_x", 1.0, &[]).unwrap();
        }
    }

    #[test]
    fn a_push_sends_one_labelled_gauge_to_the_otlp_path() {
        let (addr, h) = sink();
        let s = MetricsSink::new(Some(&format!("http://{addr}")), "cubby-server");
        s.try_push_seconds(
            "cubby_http_request_seconds",
            0.25,
            &[("route", "/healthz"), ("status", "200")],
        )
        .unwrap();
        let got = h.join().unwrap();
        let i = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        assert!(
            String::from_utf8_lossy(&got[..i])
                .starts_with("POST /opentelemetry/v1/metrics HTTP/1.1")
        );
        let req = ExportMetricsServiceRequest::decode(&got[i + 4..]).unwrap();
        let rm = &req.resource_metrics[0];
        let m = &rm.scope_metrics[0].metrics[0];
        assert_eq!(m.name, "cubby_http_request_seconds");
        let Some(Data::Gauge(g)) = &m.data else {
            panic!("gauge")
        };
        assert_eq!(g.data_points.len(), 1);
        let keys: Vec<&str> = g.data_points[0]
            .attributes
            .iter()
            .map(|a| a.key.as_str())
            .collect();
        assert_eq!(keys, ["route", "status"]);
    }

    #[test]
    fn a_count_is_a_gauge_of_one_with_unit_one() {
        let s = MetricsSink::new(Some("x:1"), "cubby-server");
        let req = s.encode_unit(
            "cubby_device_auth",
            1.0,
            "1",
            &[("outcome", "ok")],
            SystemTime::UNIX_EPOCH,
        );
        let m = &req.resource_metrics[0].scope_metrics[0].metrics[0];
        assert_eq!(
            (m.name.as_str(), m.unit.as_str()),
            ("cubby_device_auth", "1")
        );
    }

    #[test]
    fn a_refused_connection_is_an_error_but_push_swallows_it() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        drop(l);
        let s = MetricsSink::new(Some(&addr), "cubby-worker");
        assert!(s.try_push_seconds("cubby_x", 1.0, &[]).is_err());
        s.push_seconds("cubby_x", 1.0, &[]);
    }
}

#[cfg(test)]
mod metric_tests {
    use super::fake::FakeSink;
    use super::*;

    #[test]
    fn a_request_pushes_cubby_requests_with_kind_and_resolution() {
        let f = FakeSink::start();
        MetricsSink::new(Some(&f.addr), "cubby-server").push_request("video", "approved");
        let got = f.wait_for("cubby_requests");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].label("kind"), Some("video"));
        assert_eq!(got[0].label("resolution"), Some("approved"));
    }

    #[test]
    fn a_quota_charge_pushes_the_units_by_call() {
        let f = FakeSink::start();
        MetricsSink::new(Some(&f.addr), "cubby-worker").push_yt_quota("videos.list", 3);
        let got = f.wait_for("cubby_yt_quota_units");
        assert_eq!(
            (got[0].value, got[0].label("call")),
            (3.0, Some("videos.list"))
        );
    }
}

#[cfg(test)]
mod dashboard_tests {
    /// The metrics the LLD's Telemetry section names; the dashboard must chart each.
    const METRICS: [&str; 8] = [
        "cubby_plays",
        "cubby_watch_seconds",
        "cubby_charge",
        "cubby_requests",
        "cubby_yt_quota_units",
        "cubby_worker_run_seconds",
        "cubby_device_auth",
        "cubby_http_request_seconds",
    ];

    #[test]
    fn the_grafana_dashboard_charts_every_metric() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/grafana/cubby.json"
        );
        let text = std::fs::read_to_string(path).unwrap();
        for m in METRICS {
            assert!(text.contains(m), "dashboard lacks {m}");
        }
        assert!(text.trim_start().starts_with('{') && text.contains("\"panels\""));
    }
}
