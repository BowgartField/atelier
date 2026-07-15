use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendErrorCode {
    InvalidArgument,
    Io,
    Unsupported,
    NotReady,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BackendError {
    pub code: BackendErrorCode,
    pub message: String,
}

impl BackendError {
    pub fn new(code: BackendErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn unsupported(capability: impl Into<String>) -> Self {
        let capability = capability.into();
        Self::new(
            BackendErrorCode::Unsupported,
            format!("Capability '{capability}' is not available in headless mode"),
        )
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for BackendError {}

impl From<std::io::Error> for BackendError {
    fn from(error: std::io::Error) -> Self {
        Self::new(BackendErrorCode::Io, error.to_string())
    }
}

impl From<serde_json::Error> for BackendError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(BackendErrorCode::Internal, error.to_string())
    }
}
