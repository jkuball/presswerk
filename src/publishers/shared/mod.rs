//! What several publisher types share: decoding, secrets, digests, version templates, and aliases.

pub mod alias;
pub mod digest;
pub mod secrets;
pub mod template;

use crate::declaration::DeclarationError;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A declaration that does not decode was written for another Presswerk version, as long as the
/// schema is unstable.
#[must_use]
pub fn incompatible(message: impl std::fmt::Display) -> DeclarationError {
    DeclarationError::IncompatibleUnstableSchema(<serde_json::Error as serde::de::Error>::custom(
        message,
    ))
}

/// Decodes one publisher declaration into its type's shape.
pub fn decode<T: DeserializeOwned>(declaration: &Value) -> Result<T, DeclarationError> {
    T::deserialize(declaration).map_err(DeclarationError::IncompatibleUnstableSchema)
}
