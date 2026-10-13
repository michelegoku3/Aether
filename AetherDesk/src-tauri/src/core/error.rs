use std::fmt;

/// Unified domain error for AetherDesk backend operations and Tauri commands.
///
/// Designed to provide typed, structured error information across internal
/// subsystems while converting seamlessly to `String` at Tauri IPC boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeskError {
    /// Invalid caller or user input before performing operations.
    InvalidInput(String),
    /// The requested entity or resource was not found.
    NotFound(String),
    /// Filesystem or system I/O error with context.
    Io {
        context: &'static str,
        detail: String,
    },
    /// Data parsing or deserialization error (JSON, VDF, YAML, etc.).
    Parse {
        context: &'static str,
        detail: String,
    },
    /// Remote HTTP or network service failure.
    Network {
        endpoint: &'static str,
        detail: String,
    },
    /// Steam installation, process, or client error.
    Steam(String),
    /// Settings or configuration storage error.
    Settings(String),
    /// External provider error (Hubcap, Ryuu, LuaTools, etc.).
    Provider {
        provider: &'static str,
        detail: String,
    },
    /// Process or external tool execution failure (e.g. Steamless).
    Execution { tool: &'static str, detail: String },
    /// Internal unexpected error.
    Internal(String),
}

impl DeskError {
    #[inline]
    pub fn invalid_input(msg: impl Into<String>) -> Self {
        Self::InvalidInput(msg.into())
    }

    #[inline]
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    #[inline]
    pub fn io(context: &'static str, err: impl fmt::Display) -> Self {
        Self::Io {
            context,
            detail: err.to_string(),
        }
    }

    #[inline]
    pub fn parse(context: &'static str, err: impl fmt::Display) -> Self {
        Self::Parse {
            context,
            detail: err.to_string(),
        }
    }

    #[inline]
    pub fn network(endpoint: &'static str, err: impl fmt::Display) -> Self {
        Self::Network {
            endpoint,
            detail: err.to_string(),
        }
    }

    #[inline]
    pub fn steam(msg: impl Into<String>) -> Self {
        Self::Steam(msg.into())
    }

    #[inline]
    pub fn settings(msg: impl Into<String>) -> Self {
        Self::Settings(msg.into())
    }

    #[inline]
    pub fn provider(provider: &'static str, err: impl fmt::Display) -> Self {
        Self::Provider {
            provider,
            detail: err.to_string(),
        }
    }

    #[inline]
    pub fn execution(tool: &'static str, err: impl fmt::Display) -> Self {
        Self::Execution {
            tool,
            detail: err.to_string(),
        }
    }

    #[inline]
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl fmt::Display for DeskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(msg) => write!(f, "{msg}"),
            Self::NotFound(msg) => write!(f, "Not found: {msg}"),
            Self::Io { context, detail } => write!(f, "{context}: {detail}"),
            Self::Parse { context, detail } => write!(f, "Could not parse {context}: {detail}"),
            Self::Network { endpoint, detail } => {
                write!(f, "Network error on {endpoint}: {detail}")
            }
            Self::Steam(msg) => write!(f, "Steam error: {msg}"),
            Self::Settings(msg) => write!(f, "Settings error: {msg}"),
            Self::Provider { provider, detail } => write!(f, "{provider} error: {detail}"),
            Self::Execution { tool, detail } => write!(f, "{tool} execution failed: {detail}"),
            Self::Internal(msg) => write!(f, "Internal error: {msg}"),
        }
    }
}

impl std::error::Error for DeskError {}

impl From<DeskError> for String {
    fn from(err: DeskError) -> Self {
        err.to_string()
    }
}

impl From<std::io::Error> for DeskError {
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            context: "I/O operation",
            detail: err.to_string(),
        }
    }
}

impl DeskError {
    /// Machine-readable code for this error, from the same registry the
    /// `CODE: message` strings below use. Lets future call sites build typed
    /// errors and convert them at the IPC boundary without string matching.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => codes::INVALID_INPUT,
            Self::NotFound(_) => codes::NOT_FOUND,
            Self::Io { .. } => codes::IO_ERROR,
            Self::Parse { .. } => codes::PARSE_ERROR,
            Self::Network { .. } => codes::NETWORK_ERROR,
            Self::Steam(_) => codes::STEAM_ERROR,
            Self::Settings(_) => codes::SETTINGS_ERROR,
            Self::Provider { .. } => codes::PROVIDER_ERROR,
            Self::Execution { .. } => codes::EXECUTION_ERROR,
            Self::Internal(_) => codes::INTERNAL_ERROR,
        }
    }

    /// Prefix a body with a machine-readable code: the single constructor
    /// for `CODE: message` strings, so the format cannot drift per call site.
    /// The frontend recovers the code with `getErrorCode` (same pattern).
    pub fn coded(code: &str, body: impl fmt::Display) -> String {
        format!("{code}: {body}")
    }
}

/// Machine-readable error codes used at control-flow boundaries.
///
/// Convention: `CODE: human message`. The UI switches on the code, never on
/// substrings of the message. Codes are `SCREAMING_SNAKE_CASE` of 4+ chars
/// so `split_code` cannot misfire on ordinary text (e.g. `C:\...` or times).
pub mod codes {
    pub const SETTINGS_CONFLICT: &str = "SETTINGS_CONFLICT";
    pub const GAME_STATE_CONFLICT: &str = "GAME_STATE_CONFLICT";
    pub const HUBCAP_KEY_REQUIRED_FOR_UPDATES: &str = "HUBCAP_KEY_REQUIRED_FOR_UPDATES";
    pub const CONFIG_CONFLICT: &str = "CONFIG_CONFLICT";
    pub const GAME_BUSY: &str = "GAME_BUSY";
    pub const INVALID_INPUT: &str = "INVALID_INPUT";
    pub const NOT_FOUND: &str = "NOT_FOUND";
    pub const IO_ERROR: &str = "IO_ERROR";
    pub const PARSE_ERROR: &str = "PARSE_ERROR";
    pub const NETWORK_ERROR: &str = "NETWORK_ERROR";
    pub const STEAM_ERROR: &str = "STEAM_ERROR";
    pub const SETTINGS_ERROR: &str = "SETTINGS_ERROR";
    pub const PROVIDER_ERROR: &str = "PROVIDER_ERROR";
    pub const EXECUTION_ERROR: &str = "EXECUTION_ERROR";
    pub const INTERNAL_ERROR: &str = "INTERNAL_ERROR";

    /// Split `CODE: body` into its parts. Returns `(None, message)` when no
    /// valid code prefix is present. A prefix counts only when it is
    /// `SCREAMING_SNAKE_CASE` of 4+ characters followed by a colon.
    ///
    /// Only exercised by tests today: on the wire the parser lives in the
    /// frontend (`getErrorCode`), which is the single consumer of codes.
    #[allow(dead_code)]
    pub fn split_code(message: &str) -> (Option<&str>, &str) {
        let Some(colon) = message.find(':') else {
            return (None, message);
        };
        let (code, rest) = message.split_at(colon);
        let valid = code.len() >= 4
            && code
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit());
        if valid && !rest.is_empty() {
            (Some(code), rest[1..].trim_start())
        } else {
            (None, message)
        }
    }
}

#[allow(dead_code)]
pub type DeskResult<T> = Result<T, DeskError>;
