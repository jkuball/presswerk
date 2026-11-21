//! SHA-256 digests in the `sha256:<hex>` form that registries and GitHub report.

use sha2::{Digest, Sha256};
use std::io::{self, Read};

#[must_use]
pub fn sha256_digest(bytes: &[u8]) -> String {
    hex_digest(&Sha256::digest(bytes))
}

pub fn sha256_digest_reader(mut reader: impl Read) -> io::Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(hex_digest(&hash.finalize()));
        }
        hash.update(&buffer[..read]);
    }
}

#[must_use]
pub fn hex_digest(digest: &[u8]) -> String {
    digest
        .iter()
        .fold(String::from("sha256:"), |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        })
}
