//! Child device identity over a family database: pairing codes, device tokens and revocation.
//!
//! A device token is `<family id>.<64 hex>`: the family id is not secret, it only says which
//! family file holds the device row. Only the SHA-256 of the whole token is stored.

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::Result;

/// Lifetime of a pairing code, in seconds.
pub const CODE_TTL_S: i64 = 600;
pub const CODE_LEN: usize = 6;
/// No 0/O, 1/I/L: a code is read off a parent's screen and typed on a child's device.
pub const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

#[derive(Debug, Clone, PartialEq)]
pub struct Child {
    pub id: String,
    pub name: String,
    pub tz: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairingCode {
    pub code: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paired {
    pub device_id: String,
    pub child_id: String,
    /// The bearer token, shown once.
    pub token: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: String,
    pub child_id: String,
}

#[derive(Debug, PartialEq)]
pub enum PairError {
    /// Unknown, expired or already-used code; deliberately indistinguishable.
    InvalidCode,
    Db(String),
}

impl From<rusqlite::Error> for PairError {
    fn from(e: rusqlite::Error) -> PairError {
        PairError::Db(e.to_string())
    }
}

pub fn add_child(conn: &Connection, id: &str, name: &str, tz: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO children(id, name, tz) VALUES (?1, ?2, ?3)",
        (id, name, tz),
    )?;
    Ok(())
}

pub fn child(conn: &Connection, id: &str) -> Result<Option<Child>> {
    Ok(conn
        .query_row(
            "SELECT id, name, tz FROM children WHERE id = ?1",
            [id],
            |r| {
                Ok(Child {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    tz: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// Issues a fresh code for `child_id`; `None` if there is no such child.
pub fn new_pairing_code(
    conn: &Connection,
    child_id: &str,
    now: i64,
) -> Result<Option<PairingCode>> {
    if child(conn, child_id)?.is_none() {
        return Ok(None);
    }
    let expires_at = now + CODE_TTL_S;
    loop {
        let code = random_code();
        // A collision with a live code inserts nothing and draws again; a spent or expired
        // code's row is reused.
        let inserted = conn.execute(
            "INSERT INTO pairing_codes(code, child_id, expires_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(code) DO UPDATE SET child_id = ?2, expires_at = ?3, used_at = NULL
             WHERE used_at IS NOT NULL OR expires_at <= ?4",
            (&code, child_id, expires_at, now),
        )?;
        if inserted == 1 {
            return Ok(Some(PairingCode { code, expires_at }));
        }
    }
}

/// Spends `code` once and creates the device, returning its token.
pub fn redeem_pairing_code(
    conn: &Connection,
    family_id: &str,
    code: &str,
    label: Option<&str>,
    now: i64,
) -> std::result::Result<Paired, PairError> {
    let code = code.trim().to_ascii_uppercase();
    let tx = conn.unchecked_transaction()?;
    let child_id: Option<String> = tx
        .query_row(
            "UPDATE pairing_codes SET used_at = ?2
             WHERE code = ?1 AND used_at IS NULL AND expires_at > ?2 RETURNING child_id",
            (&code, now),
            |r| r.get(0),
        )
        .optional()?;
    let child_id = child_id.ok_or(PairError::InvalidCode)?;
    let token = format!("{family_id}.{}", random_hex(32));
    let device_id = random_hex(16);
    tx.execute(
        "INSERT INTO devices(id, child_id, token_hash, label, paired_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (
            &device_id,
            &child_id,
            hash_token(&token).as_slice(),
            label.unwrap_or(""),
            now,
        ),
    )?;
    tx.commit()?;
    Ok(Paired {
        device_id,
        child_id,
        token,
    })
}

/// The family id a token claims to belong to; the token is not yet verified.
pub fn token_family(token: &str) -> Option<&str> {
    token.split_once('.').map(|(f, _)| f)
}

/// The device behind `token`, or `None` when the token is unknown or revoked.
pub fn authenticate_device(conn: &Connection, token: &str, now: i64) -> Result<Option<Device>> {
    Ok(conn
        .query_row(
            "UPDATE devices SET last_seen = ?2 WHERE token_hash = ?1 AND revoked_at IS NULL
             RETURNING id, child_id",
            (hash_token(token).as_slice(), now),
            |r| {
                Ok(Device {
                    id: r.get(0)?,
                    child_id: r.get(1)?,
                })
            },
        )
        .optional()?)
}

/// Revokes a device; `false` if it does not exist or was already revoked.
pub fn revoke_device(conn: &Connection, device_id: &str, now: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE devices SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
        (device_id, now),
    )? == 1)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the OS random source is unavailable");
    b
}

fn random_hex(bytes: usize) -> String {
    let mut out = String::with_capacity(bytes * 2);
    let mut left = bytes;
    while left > 0 {
        let chunk = random_bytes::<16>();
        for b in &chunk[..left.min(16)] {
            out.push_str(&format!("{b:02x}"));
        }
        left = left.saturating_sub(16);
    }
    out
}

/// Uniform over the alphabet: bytes at or above the largest multiple of its length are redrawn.
fn random_code() -> String {
    let n = CODE_ALPHABET.len();
    let limit = 256 - 256 % n;
    let mut code = String::with_capacity(CODE_LEN);
    while code.len() < CODE_LEN {
        for b in random_bytes::<16>() {
            if usize::from(b) < limit && code.len() < CODE_LEN {
                code.push(CODE_ALPHABET[usize::from(b) % n] as char);
            }
        }
    }
    code
}

fn hash_token(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family::migrate_family;

    const FAM: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

    fn family() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        migrate_family(&mut c).unwrap();
        add_child(&c, "c1", "Ada", "Europe/London").unwrap();
        c
    }

    fn pair(c: &Connection) -> Paired {
        let code = new_pairing_code(c, "c1", 0).unwrap().unwrap();
        redeem_pairing_code(c, FAM, &code.code, None, 1).unwrap()
    }

    #[test]
    fn a_code_is_six_unambiguous_characters_valid_ten_minutes() {
        let c = new_pairing_code(&family(), "c1", 1000).unwrap().unwrap();
        assert_eq!(c.code.len(), CODE_LEN);
        assert!(c.code.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        assert_eq!(c.expires_at, 1000 + 600);
    }

    #[test]
    fn no_code_for_an_unknown_child() {
        assert_eq!(new_pairing_code(&family(), "nope", 0).unwrap(), None);
    }

    #[test]
    fn a_code_pairs_once_and_is_then_spent() {
        let f = family();
        let c = new_pairing_code(&f, "c1", 1000).unwrap().unwrap();
        let p = redeem_pairing_code(&f, FAM, &c.code, Some("iPad"), 1001).unwrap();
        assert_eq!(p.child_id, "c1");
        assert_eq!(token_family(&p.token), Some(FAM));
        assert_eq!(p.token.len(), FAM.len() + 1 + 64);
        assert_eq!(
            redeem_pairing_code(&f, FAM, &c.code, None, 1002),
            Err(PairError::InvalidCode)
        );
    }

    #[test]
    fn a_code_expires_after_ten_minutes() {
        let f = family();
        let c = new_pairing_code(&f, "c1", 1000).unwrap().unwrap();
        assert_eq!(
            redeem_pairing_code(&f, FAM, &c.code, None, 1600),
            Err(PairError::InvalidCode)
        );
        let c = new_pairing_code(&f, "c1", 1000).unwrap().unwrap();
        assert!(redeem_pairing_code(&f, FAM, &c.code, None, 1599).is_ok());
    }

    #[test]
    fn a_code_is_matched_case_insensitively_and_trimmed() {
        let f = family();
        let c = new_pairing_code(&f, "c1", 0).unwrap().unwrap();
        let typed = format!(" {} ", c.code.to_lowercase());
        assert!(redeem_pairing_code(&f, FAM, &typed, None, 1).is_ok());
    }

    #[test]
    fn an_unknown_code_is_invalid() {
        assert_eq!(
            redeem_pairing_code(&family(), FAM, "ZZZZZZ", None, 0),
            Err(PairError::InvalidCode)
        );
    }

    #[test]
    fn the_token_authenticates_and_only_its_hash_is_stored() {
        let f = family();
        let p = pair(&f);
        let d = authenticate_device(&f, &p.token, 5).unwrap().unwrap();
        assert_eq!((d.id, d.child_id), (p.device_id, "c1".to_string()));
        assert_eq!(authenticate_device(&f, "bogus", 5).unwrap(), None);
        let stored: Vec<u8> = f
            .query_row("SELECT token_hash FROM devices", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored.len(), 32);
        assert_ne!(stored, p.token.as_bytes());
    }

    #[test]
    fn a_revoked_token_stops_authenticating_at_once() {
        let f = family();
        let p = pair(&f);
        assert!(authenticate_device(&f, &p.token, 2).unwrap().is_some());
        assert!(revoke_device(&f, &p.device_id, 3).unwrap());
        assert_eq!(authenticate_device(&f, &p.token, 3).unwrap(), None);
        assert!(!revoke_device(&f, &p.device_id, 4).unwrap());
        assert!(!revoke_device(&f, "nope", 4).unwrap());
    }

    #[test]
    fn two_pairings_get_distinct_tokens() {
        let f = family();
        let tokens: std::collections::HashSet<_> = (0..5).map(|_| pair(&f).token).collect();
        assert_eq!(tokens.len(), 5);
    }
}
