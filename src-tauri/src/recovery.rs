//! Storage problem categories shown to the creator. No URLs, tokens or file paths.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageIssue {
    Reconnect,
    Access,
    Missing,
    Network,
    Busy,
    Validation,
    Unknown,
}
#[derive(Debug)]
pub struct ReconnectRequired;
impl std::fmt::Display for ReconnectRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Reconnect your storage account")
    }
}
impl std::error::Error for ReconnectRequired {}
pub fn classify(error: &anyhow::Error) -> StorageIssue {
    if error.is::<ReconnectRequired>() {
        return StorageIssue::Reconnect;
    }
    for cause in error.chain() {
        if let Some(http) = cause.downcast_ref::<reqwest::Error>() {
            if let Some(status) = http.status() {
                return status_issue(status.as_u16());
            }
            if http.is_timeout() || http.is_connect() || http.is_body() {
                return StorageIssue::Network;
            }
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return match io.kind() {
                std::io::ErrorKind::PermissionDenied => StorageIssue::Access,
                std::io::ErrorKind::NotFound => StorageIssue::Missing,
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::NotConnected => {
                    StorageIssue::Network
                }
                _ => StorageIssue::Unknown,
            };
        }
    }
    let message = error.to_string().to_lowercase();
    if message.contains("checksum")
        || message.contains("fingerprint validation")
        || message.contains("content validation")
    {
        StorageIssue::Validation
    } else {
        StorageIssue::Unknown
    }
}
fn status_issue(status: u16) -> StorageIssue {
    match status {
        401 => StorageIssue::Reconnect,
        403 => StorageIssue::Access,
        404 => StorageIssue::Missing,
        408 => StorageIssue::Network,
        429 | 500..=599 => StorageIssue::Busy,
        _ => StorageIssue::Unknown,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn categories_preserve_actionable_distinctions() {
        for (status, expected) in [
            (401, StorageIssue::Reconnect),
            (403, StorageIssue::Access),
            (404, StorageIssue::Missing),
            (429, StorageIssue::Busy),
            (503, StorageIssue::Busy),
            (400, StorageIssue::Unknown),
        ] {
            assert_eq!(status_issue(status), expected);
        }
        let permission =
            anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                .context("private path");
        assert_eq!(classify(&permission), StorageIssue::Access);
        assert_eq!(
            classify(&anyhow::Error::new(ReconnectRequired)),
            StorageIssue::Reconnect
        );
        assert_eq!(
            classify(&anyhow::anyhow!(
                "Downloaded file failed checksum validation"
            )),
            StorageIssue::Validation
        );
        assert_eq!(
            classify(&anyhow::anyhow!("Unexpected private URL")),
            StorageIssue::Unknown
        );
        assert_eq!(
            serde_json::to_string(&classify(&permission)).unwrap(),
            "\"access\""
        );
    }
}
