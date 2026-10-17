//! The metadata worker pass. The skeleton pass does nothing and succeeds.

use std::time::Instant;

use cubby_otel::MetricsSink;

/// One no-op pass; returns the outcome label.
pub fn run_pass() -> &'static str {
    "ok"
}

/// Runs one pass and pushes `cubby_worker_run_seconds{outcome}` to `endpoint` (empty disables).
pub fn run_once(endpoint: &str) -> &'static str {
    let sink = MetricsSink::new(Some(endpoint), "cubby-worker");
    let start = Instant::now();
    let outcome = run_pass();
    sink.push_seconds(
        "cubby_worker_run_seconds",
        start.elapsed().as_secs_f64(),
        &[("outcome", outcome)],
    );
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_noop_pass_succeeds() {
        assert_eq!(run_pass(), "ok");
    }

    #[test]
    fn a_pass_with_no_endpoint_still_succeeds() {
        assert_eq!(run_once(""), "ok");
    }
}
