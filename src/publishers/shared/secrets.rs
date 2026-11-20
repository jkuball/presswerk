//! The secrets publishers need, such as API tokens and registry passwords.

use crate::declaration::DeclarationError;
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;

/// Where publishers read their secrets.
#[derive(Clone, Default)]
pub struct Secrets;

impl fmt::Debug for Secrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secrets([redacted])")
    }
}

impl Secrets {
    /// The value of an environment variable, or `None` if it is unset or empty.
    #[must_use]
    pub fn env(&self, name: &str) -> Option<String> {
        env::var(name).ok().filter(|value| !value.is_empty())
    }
}

/// A secret read from the environment variable `secret` at run time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Token {
    pub secret: String,
}

impl Token {
    pub fn validate(&self) -> Result<(), DeclarationError> {
        if self.secret.is_empty() {
            return Err(DeclarationError::Invalid(
                "secret name must not be empty".into(),
            ));
        }
        Ok(())
    }
}

/// The registry username: the name of an environment variable read at run time, or the username
/// itself, which is no secret for registries such as Google Artifact Registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Username {
    Secret(String),
    Value(String),
}
