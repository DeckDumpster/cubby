//! YouTube Data API v3 client with a quota meter. No call is made yet.

use cubby_otel::MetricsSink;

/// Quota units a `videos.list` call costs.
pub const VIDEOS_LIST_COST: u32 = 1;

/// Counts the quota a run has spent and reports each charge as `cubby_yt_quota_units{call}`.
pub struct QuotaMeter<'a> {
    sink: &'a MetricsSink,
    used: u32,
}

impl<'a> QuotaMeter<'a> {
    pub fn new(sink: &'a MetricsSink) -> Self {
        QuotaMeter { sink, used: 0 }
    }

    pub fn charge(&mut self, call: &str, units: u32) {
        self.used += units;
        self.sink.push_yt_quota(call, units);
    }

    pub fn used(&self) -> u32 {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubby_otel::fake::FakeSink;

    #[test]
    fn videos_list_costs_one_unit() {
        assert_eq!(VIDEOS_LIST_COST, 1);
    }

    #[test]
    fn a_charge_is_counted_and_emitted() {
        let f = FakeSink::start();
        let sink = MetricsSink::new(Some(&f.addr), "cubby-worker");
        let mut m = QuotaMeter::new(&sink);
        m.charge("videos.list", VIDEOS_LIST_COST);
        assert_eq!(m.used(), 1);
        let got = f.wait_for("cubby_yt_quota_units");
        assert_eq!(
            (got[0].value, got[0].label("call")),
            (1.0, Some("videos.list"))
        );
    }
}
