//! Family and registry SQLite storage: schema, migrations, the `playable` view and the registry.

mod error;
pub mod family;
pub mod registry;
mod schema;

pub use error::{DbError, Result};
pub use family::{FAMILY_VERSION, open_family};
pub use registry::{Family, Registry};

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
