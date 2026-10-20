//! Folder-access request types and validation. The rules are in decisions.rs.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessRequest {
    pub id: String,
    /// Hash of the invitation that authorizes this request and its status checks.
    pub invite: String,
    /// Ciphertext only; the coordinator never has the key.
    pub sealed: SealedAccess,
    /// Kept so retries still match after the ciphertext is cleared.
    pub payload_hash: String,
    pub at: i64,
    /// "pending", "granted" or "failed".
    pub status: String,
    #[serde(default)]
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SealedAccess {
    pub version: u32,
    pub key_id: String,
    pub nonce: String,
    pub ciphertext: String,
}
impl SealedAccess {
    pub fn validate(&self) -> Result<()> {
        fn hex(s: &str, len: usize) -> bool {
            s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
        }
        ensure!(
            self.version == 1
                && hex(&self.key_id, 64)
                && hex(&self.nonce, 24)
                && hex(&self.ciphertext, 2080),
            "Invalid encrypted folder access request"
        );
        Ok(())
    }
}
pub const ACCESS_REQUEST_LIFETIME: i64 = 86400;
pub const REMOVED_MESSAGE: &str =
    "You were removed from this project. Your stage files stay on this PC.";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_access_framing_is_validated() {
        let mut value = SealedAccess {
            version: 1,
            key_id: "a".repeat(64),
            nonce: "b".repeat(24),
            ciphertext: "c".repeat(2080),
        };
        assert!(value.validate().is_ok());
        value.ciphertext.pop();
        assert!(value.validate().is_err());
        value.ciphertext = "c".repeat(2080);
        value.version = 2;
        assert!(value.validate().is_err());
    }
}
