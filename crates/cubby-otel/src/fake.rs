//! A fake OTLP receiver: accepts pushes on a loopback port and keeps every decoded sample.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::metrics::v1::{metric::Data, number_data_point::Value as Num};
use prost::Message;

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub name: String,
    pub value: f64,
    pub labels: Vec<(String, String)>,
}

impl Sample {
    pub fn label(&self, key: &str) -> Option<&str> {
        self.labels
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

pub struct FakeSink {
    pub addr: String,
    samples: Arc<Mutex<Vec<Sample>>>,
}

impl FakeSink {
    pub fn start() -> FakeSink {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let keep = samples.clone();
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(mut s) = conn else { continue };
                if let Some(body) = read_body(&mut s)
                    && let Ok(req) = ExportMetricsServiceRequest::decode(body.as_slice())
                {
                    keep.lock().unwrap().extend(decode(&req));
                }
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        FakeSink { addr, samples }
    }

    pub fn samples(&self) -> Vec<Sample> {
        self.samples.lock().unwrap().clone()
    }

    /// Waits up to five seconds for a sample named `name` and returns every one of that name.
    pub fn wait_for(&self, name: &str) -> Vec<Sample> {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let got: Vec<Sample> = self
                .samples()
                .into_iter()
                .filter(|s| s.name == name)
                .collect();
            if !got.is_empty() || Instant::now() > end {
                return got;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn read_body(s: &mut std::net::TcpStream) -> Option<Vec<u8>> {
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = s.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        got.extend_from_slice(&buf[..n]);
        if let Some(i) = got.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&got[..i]).to_ascii_lowercase();
            let len: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))?
                .trim()
                .parse()
                .ok()?;
            if got.len() >= i + 4 + len {
                return Some(got[i + 4..i + 4 + len].to_vec());
            }
        }
    }
}

fn decode(req: &ExportMetricsServiceRequest) -> Vec<Sample> {
    let mut out = Vec::new();
    for rm in &req.resource_metrics {
        for sm in &rm.scope_metrics {
            for m in &sm.metrics {
                let Some(Data::Gauge(g)) = &m.data else {
                    continue;
                };
                for p in &g.data_points {
                    let value = match p.value {
                        Some(Num::AsDouble(v)) => v,
                        Some(Num::AsInt(v)) => v as f64,
                        None => continue,
                    };
                    let labels = p
                        .attributes
                        .iter()
                        .filter_map(|a| match a.value.as_ref()?.value.as_ref()? {
                            Value::StringValue(v) => Some((a.key.clone(), v.clone())),
                            _ => None,
                        })
                        .collect();
                    out.push(Sample {
                        name: m.name.clone(),
                        value,
                        labels,
                    });
                }
            }
        }
    }
    out
}
