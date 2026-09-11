//! One error type carrying the process exit code, so every leaf can fail
//! with the right code and one message.

use gearbox_api::{Status, code};

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NO_INSTANCE: i32 = 3;
pub const EXIT_REFUSED: i32 = 4;
pub const EXIT_UNSUPPORTED: i32 = 5;
pub const EXIT_BUSY: i32 = 6;
pub const EXIT_TIMEOUT: i32 = 7;

#[derive(Debug, Clone)]
pub struct CliError {
    pub code: i32,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, CliError>;

impl CliError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(EXIT_ERROR, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(EXIT_USAGE, message)
    }

    pub fn no_instance(message: impl Into<String>) -> Self {
        Self::new(EXIT_NO_INSTANCE, message)
    }

    pub fn refused(message: impl Into<String>) -> Self {
        Self::new(EXIT_REFUSED, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(EXIT_UNSUPPORTED, message)
    }

    pub fn busy(message: impl Into<String>) -> Self {
        Self::new(EXIT_BUSY, message)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(EXIT_TIMEOUT, message)
    }

    /// A non-ok wire status becomes the matching exit code.
    pub fn from_status(status: &Status, what: &str) -> Self {
        let message = status.message();
        let detail = if message.is_empty() {
            format!("{what}: status code {}", status.code)
        } else {
            format!("{what}: {message}")
        };
        Self::new(exit_for_wire_code(status.code), detail)
    }
}

pub fn exit_for_wire_code(wire: u32) -> i32 {
    match wire {
        code::OK => EXIT_OK,
        code::USAGE => EXIT_USAGE,
        code::NO_INSTANCE => EXIT_NO_INSTANCE,
        code::REFUSED => EXIT_REFUSED,
        code::UNSUPPORTED => EXIT_UNSUPPORTED,
        code::BUSY => EXIT_BUSY,
        code::TIMEOUT => EXIT_TIMEOUT,
        _ => EXIT_ERROR,
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<agentio::Error> for CliError {
    fn from(err: agentio::Error) -> Self {
        let text = err.to_string();
        let code = match &err {
            agentio::Error::ControlTimeout(_) => EXIT_TIMEOUT,
            agentio::Error::ResolutionFailed(_) => EXIT_NO_INSTANCE,
            _ if text.to_lowercase().contains("timeout")
                || text.to_lowercase().contains("timed out") =>
            {
                EXIT_TIMEOUT
            }
            _ => EXIT_ERROR,
        };
        Self::new(code, text)
    }
}

impl From<std::io::Error> for CliError {
    fn from(err: std::io::Error) -> Self {
        Self::error(err.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(err: serde_json::Error) -> Self {
        Self::usage(format!("bad JSON: {err}"))
    }
}

impl From<toml::de::Error> for CliError {
    fn from(err: toml::de::Error) -> Self {
        Self::error(format!("bad TOML: {err}"))
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::error(message)
    }
}
