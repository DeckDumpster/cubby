use std::fmt;

#[derive(Debug)]
pub enum DbError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    /// The file carries a schema version newer than this build understands.
    NewerSchema {
        label: &'static str,
        found: i64,
        supported: i64,
    },
    /// The email is already registered.
    Taken(String),
    /// The id is not a registry-minted family id.
    BadId(String),
}

pub type Result<T> = std::result::Result<T, DbError>;

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Sqlite(e) => write!(f, "sqlite: {e}"),
            DbError::Io(e) => write!(f, "io: {e}"),
            DbError::NewerSchema {
                label,
                found,
                supported,
            } => write!(
                f,
                "{label} database is at schema version {found}, newer than the {supported} this build understands; refusing to open it"
            ),
            DbError::Taken(e) => write!(f, "email {e:?} is already registered"),
            DbError::BadId(i) => write!(f, "{i:?} is not a family id"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sqlite(e)
    }
}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::Io(e)
    }
}
