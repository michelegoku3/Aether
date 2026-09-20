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
    Execution {
        tool: &'static str,
        detail: String,
    },
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
            Self::Network { endpoint, detail } => write!(f, "Network error on {endpoint}: {detail}"),
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

#[allow(dead_code)]
pub type DeskResult<T> = Result<T, DeskError>;
