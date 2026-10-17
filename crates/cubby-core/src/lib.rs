//! Pure domain logic: no I/O. Budget math, policy resolution and the ranker land here.

pub mod budget;

/// Whole seconds in `minutes`.
pub fn minutes_to_seconds(minutes: u32) -> u64 {
    u64::from(minutes) * 60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_convert_to_seconds() {
        assert_eq!(minutes_to_seconds(15), 900);
    }
}
