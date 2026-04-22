use std::env;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretError {
    #[error("required env var {0} is not set")]
    Missing(String),
}

pub fn require(key: &str) -> Result<String, SecretError> {
    env::var(key).map_err(|_| SecretError::Missing(key.to_string()))
}

pub fn optional(key: &str) -> Option<String> {
    env::var(key).ok().filter(|s| !s.is_empty())
}
