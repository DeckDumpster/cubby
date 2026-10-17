//! Family and registry SQLite storage: schema, migrations and queries land here.

/// Name of the per-family database file for family `id`.
pub fn family_file_name(id: &str) -> String {
    format!("family-{id}.sqlite")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_file_is_named_after_its_id() {
        assert_eq!(family_file_name("01ABC"), "family-01ABC.sqlite");
    }
}
