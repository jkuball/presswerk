//! Credentials that publishers need, such as API tokens and registry passwords.
//!
//! A declaration gives each credential in one shape, whatever its publisher: exactly one of the
//! value itself, `{ "value": "…" }`, for what is no secret, or a secret source to read it from at
//! run time, such as `{ "env": "NAME" }`. The core finds every secret of a declaration, resolves them
//! all before a publisher runs, and redacts their values from everything it prints.

use crate::declaration::DeclarationError;
use crate::publication::PublicationError;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::sync::{Mutex, PoisonError};

/// A credential of a publisher: given in the declaration, or read from a secret source. Each
/// source is a sibling of `value`, such as `{ "env": "NAME" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Credential {
    /// The value itself, written into the declaration, such as a registry username that is no
    /// secret.
    Value(String),
    /// Where to read the value at run time.
    #[serde(untagged)]
    Secret(SecretSource),
}

impl<'de> Deserialize<'de> for Credential {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Every shape a credential may have, flat, so that an unknown one is named.
        #[derive(Deserialize)]
        #[serde(rename_all = "lowercase", deny_unknown_fields)]
        enum Shape {
            Value(String),
            Env(String),
        }
        match Shape::deserialize(deserializer) {
            Ok(Shape::Value(value)) => Ok(Self::Value(value)),
            Ok(Shape::Env(name)) => Ok(Self::env(name)),
            Err(error) => Err(D::Error::custom(format!(
                "a credential is exactly one of {{ \"value\": \"…\" }} or a secret source such as {{ \"env\": \"NAME\" }}: {error}"
            ))),
        }
    }
}

impl Credential {
    /// A secret read from the environment variable `name`.
    #[must_use]
    pub fn env(name: impl Into<String>) -> Self {
        Self::Secret(SecretSource::Env(name.into()))
    }

    /// Validates a credential that is always a secret, such as a token or password, which must
    /// never be written into the declaration.
    pub fn validate_secret(&self, what: &str) -> Result<(), DeclarationError> {
        if let Self::Value(_) = self {
            return Err(DeclarationError::Invalid(format!(
                "the {what} must be a secret, such as {{ \"env\": \"NAME\" }}, never a value in the declaration"
            )));
        }
        self.validate()
    }

    pub fn validate(&self) -> Result<(), DeclarationError> {
        match self {
            Self::Value(value) if value.is_empty() => Err(DeclarationError::Invalid(
                "a credential value must not be empty".into(),
            )),
            Self::Secret(SecretSource::Env(name)) if name.is_empty() => Err(
                DeclarationError::Invalid("secret name must not be empty".into()),
            ),
            _ => Ok(()),
        }
    }
}

/// Where a secret is read at run time. More sources may follow.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum SecretSource {
    /// An environment variable.
    Env(String),
}

impl fmt::Display for SecretSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env(name) => write!(formatter, "environment variable {name:?}"),
        }
    }
}

/// Every secret a declaration names, with where it is declared, such as
/// `github-release.default.token`. It reads the JSON generically, so it finds the secrets of
/// every publisher type in the same shape.
pub fn collect(publishers: &Value) -> Vec<(String, SecretSource)> {
    fn walk(value: &Value, path: &str, found: &mut Vec<(String, SecretSource)>) {
        match value {
            Value::Object(object) => {
                if object.len() == 1
                    && let Ok(source) = SecretSource::deserialize(value.clone())
                {
                    found.push((path.to_owned(), source));
                    return;
                }
                for (key, value) in object {
                    let path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    walk(value, &path, found);
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    walk(value, &format!("{path}.{index}"), found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    walk(publishers, "", &mut found);
    found
}

/// The resolved secrets of one declaration.
#[derive(Clone, Default)]
pub struct Secrets {
    /// Each declared secret, by where it is declared, with its value if it is set.
    declared: Vec<(String, SecretSource)>,
    values: BTreeMap<SecretSource, String>,
}

impl fmt::Debug for Secrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Secrets")
            .field("declared", &self.declared)
            .finish_non_exhaustive()
    }
}

impl Secrets {
    /// Reads every secret named in `publishers` from its source. An unset or empty variable is
    /// missing.
    #[must_use]
    pub fn resolve(publishers: &Value) -> Self {
        Self::resolve_with(publishers, |name| env::var(name).ok())
    }

    /// Like [`Secrets::resolve`], with `lookup` reading environment variables.
    #[must_use]
    pub fn resolve_with(publishers: &Value, lookup: impl Fn(&str) -> Option<String>) -> Self {
        let declared = collect(publishers);
        let mut values = BTreeMap::new();
        for (_, source) in &declared {
            let SecretSource::Env(name) = source;
            register_name(name);
            if let Some(value) = lookup(name).filter(|value| !value.is_empty()) {
                register(&value);
                values.insert(source.clone(), value);
            }
        }
        Self { declared, values }
    }

    /// The declared secrets that are not set, by where they are declared.
    #[must_use]
    pub fn missing(&self) -> Vec<(&str, &SecretSource)> {
        self.declared
            .iter()
            .filter(|(_, source)| !self.values.contains_key(source))
            .map(|(path, source)| (path.as_str(), source))
            .collect()
    }

    /// The value of the environment variable `name`, if a declared secret reads it and it is set.
    #[must_use]
    pub fn env(&self, name: &str) -> Option<&str> {
        self.values
            .get(&SecretSource::Env(name.to_owned()))
            .map(String::as_str)
    }

    /// Whether a credential has a value now.
    #[must_use]
    pub fn is_set(&self, credential: &Credential) -> bool {
        match credential {
            Credential::Value(_) => true,
            Credential::Secret(source) => self.values.contains_key(source),
        }
    }

    /// The value of a credential.
    pub fn get(&self, credential: &Credential) -> Result<String, PublicationError> {
        match credential {
            Credential::Value(value) => Ok(value.clone()),
            Credential::Secret(source) => self.values.get(source).cloned().ok_or_else(|| {
                PublicationError::new(format!("secret {source} is not set or empty"))
            }),
        }
    }
}

/// Every secret value read so far in this process, longest first, so that output never shows
/// one.
static KNOWN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The environment variables that hold a declared secret, so that programs Presswerk runs for
/// other purposes do not inherit them.
static NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn register_name(name: &str) {
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    if !names.iter().any(|known| known == name) {
        names.push(name.to_owned());
    }
}

/// Every environment variable that a secret read so far in this process is declared in.
#[must_use]
pub fn secret_variables() -> Vec<String> {
    NAMES.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn register(value: &str) {
    let mut known = KNOWN.lock().unwrap_or_else(PoisonError::into_inner);
    if !known.iter().any(|known| known == value) {
        known.push(value.to_owned());
        known.sort_by_key(|value| std::cmp::Reverse(value.len()));
    }
}

/// `text` with every secret value read so far replaced by `[redacted]`.
#[must_use]
pub fn redact(text: &str) -> String {
    let known = KNOWN.lock().unwrap_or_else(PoisonError::into_inner);
    known.iter().fold(text.to_owned(), |text, value| {
        text.replace(value, "[redacted]")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_value_or_a_secret_source() {
        for (json, expected) in [
            (json!({ "value": "user" }), Credential::Value("user".into())),
            (json!({ "env": "TOKEN" }), Credential::env("TOKEN")),
        ] {
            assert_eq!(Credential::deserialize(json).unwrap(), expected);
        }
        for invalid in [
            json!({}),
            json!({ "value": "user", "env": "TOKEN" }),
            json!({ "vault": "path" }),
            // The shapes before credentials were flat; such a declaration is one this CLI cannot
            // read.
            json!({ "secret": "TOKEN" }),
            json!({ "secret": { "env": "TOKEN" } }),
        ] {
            assert!(
                Credential::deserialize(invalid.clone()).is_err(),
                "{invalid}"
            );
        }
        assert_eq!(
            serde_json::to_value(Credential::env("TOKEN")).unwrap(),
            json!({ "env": "TOKEN" })
        );
    }

    #[test]
    fn collects_every_secret_of_every_publisher_and_reports_the_missing_ones() {
        let publishers = json!({
            "github-release": { "default": { "token": { "env": "GITHUB_TOKEN" } } },
            "oci-image": {
                "default": {
                    "username": { "value": "oauth2accesstoken" },
                    "password": { "env": "REGISTRY_TOKEN" }
                }
            }
        });
        let secrets = Secrets::resolve_with(&publishers, |name| {
            (name == "GITHUB_TOKEN").then(|| "presswerk-test-github-token".to_owned())
        });

        assert_eq!(
            secrets.missing(),
            [(
                "oci-image.default.password",
                &SecretSource::Env("REGISTRY_TOKEN".into())
            )]
        );
        assert_eq!(
            secrets.get(&Credential::env("GITHUB_TOKEN")).unwrap(),
            "presswerk-test-github-token"
        );
        let error = secrets
            .get(&Credential::env("REGISTRY_TOKEN"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("\"REGISTRY_TOKEN\""), "{error}");
        assert_eq!(
            redact("token presswerk-test-github-token leaked"),
            "token [redacted] leaked"
        );
    }
}
