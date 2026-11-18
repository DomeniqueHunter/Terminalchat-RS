use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// Login rejected by the server (wrong password, banned, ...). Retrying will not help.
    #[error("{0}")]
    Rejected(String),

    /// Network or server problem. Retrying later may help.
    #[error("{0}")]
    Temporary(String),
}

impl Error {
    pub fn is_temporary(&self) -> bool {
        matches!(self, Error::Temporary(_))
    }
}
