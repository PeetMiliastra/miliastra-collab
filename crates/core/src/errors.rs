//! Error codes for code to check, separate from the wording shown to creators.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Unauthorized,
    Removed,
    ReservationLost,
    Conflict,
    Validation,
    Unavailable,
    NotFound,
    RateLimited,
    Internal,
    ProtocolMismatch,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceError {
    pub code: ErrorCode,
    pub error: String,
}
impl ServiceError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            error: message.into(),
        }
    }
}
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error)
    }
}
impl std::error::Error for ServiceError {}

impl ErrorCode {
    pub fn http_status(self) -> u16 {
        match self {
            Self::Unauthorized | Self::Removed => 401,
            Self::ReservationLost | Self::Conflict | Self::ProtocolMismatch => 409,
            Self::Validation => 400,
            Self::Unavailable => 503,
            Self::NotFound => 404,
            Self::RateLimited => 429,
            Self::Internal => 500,
        }
    }
}
