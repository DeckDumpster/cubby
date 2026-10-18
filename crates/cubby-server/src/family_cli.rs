//! Operator path for provisioning families: `cubby-server family create <email>` and
//! `cubby-server family list`, plus the optional `CUBBY_BOOTSTRAP_PARENT` startup seed.

use std::io::Write;
use std::path::Path;

use cubby_db::{DbError, Registry};

pub const USAGE: &str = "usage: cubby-server family create <email> | cubby-server family list";

/// Runs `family <args...>` against the registry in `data_dir`, writing results to `out`.
pub fn run(data_dir: &Path, args: &[String], out: &mut impl Write) -> Result<(), String> {
    let mut registry = Registry::open(data_dir)
        .map_err(|e| format!("open registry in {}: {e}", data_dir.display()))?;
    match args {
        [cmd, email] if cmd == "create" => {
            let email = email.trim();
            if !email.contains('@') {
                return Err(format!("{email:?} is not an email address"));
            }
            let f = registry
                .create_family(email)
                .map_err(|e| format!("create family for {email}: {e}"))?;
            writeln!(out, "{}\t{}", f.id, f.email).map_err(|e| e.to_string())
        }
        [cmd] if cmd == "list" => {
            for f in registry.list().map_err(|e| e.to_string())? {
                writeln!(out, "{}\t{}", f.id, f.email).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// Creates the family for `email` when the registry has none yet. Returns whether it created one.
pub fn bootstrap(data_dir: &Path, email: &str) -> Result<bool, String> {
    let mut registry = Registry::open(data_dir)
        .map_err(|e| format!("open registry in {}: {e}", data_dir.display()))?;
    if !registry.list().map_err(|e| e.to_string())?.is_empty() {
        return Ok(false);
    }
    match registry.create_family(email) {
        Ok(_) => Ok(true),
        Err(DbError::Taken(_)) => Ok(false),
        Err(e) => Err(format!("bootstrap family for {email}: {e}")),
    }
}
