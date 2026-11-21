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

/// Standard Base64 with padding, as HTTP basic authentication and npm uploads use it.
#[must_use]
pub fn base64(input: impl AsRef<[u8]>) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = input.as_ref();
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let value = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                output.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                output.push('=');
            }
        }
    }
    output
}
