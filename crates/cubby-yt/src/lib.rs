//! YouTube Data API v3 client with a quota meter. No call is made yet.

/// Quota units a `videos.list` call costs.
pub const VIDEOS_LIST_COST: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn videos_list_costs_one_unit() {
        assert_eq!(VIDEOS_LIST_COST, 1);
    }
}
