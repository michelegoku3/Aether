use std::fmt;

/// Errors of the `versioning` domain. Convertible to the `String` payloads
/// every Tauri command returns, so the IPC boundary stays typed everywhere
/// else and only the final conversion produces user-facing text.
#[derive(Debug, Clone)]
pub enum VersionError {
    /// Bad input rejected before any I/O or network work.
    InvalidInput(&'static str),
    /// The build id does not resolve to any known build.
    BuildNotFound,
    /// The build history feed returned no builds for this app.
    BuildHistoryEmpty,
    /// The available history ended before every Lua depot could be resolved.
    /// Applying a partial snapshot would mix manifests from different eras.
    IncompleteSnapshot(Vec<u32>),
    /// The Depotbox token was rejected (401).
    TokenInvalid,
    /// A remote source failed or refused the request (network, HTTP, VPN block, ...).
    SourceUnavailable {
        source: &'static str,
        detail: String,
    },
    /// The game has no `stplug-in` Lua yet — version changes need it.
    LuaMissing(String),
    /// The Lua could not be parsed or edited.
    Lua(String),
    /// Local file I/O failure with machine context.
    Io {
        context: &'static str,
        detail: String,
    },
    /// Remote payload could not be parsed/validated.
    Parse {
        context: &'static str,
        detail: String,
    },
}

impl VersionError {
    pub fn io(context: &'static str, err: impl fmt::Display) -> Self {
        Self::Io {
            context,
            detail: err.to_string(),
        }
    }

    pub fn parse(context: &'static str, err: impl fmt::Display) -> Self {
        Self::Parse {
            context,
            detail: err.to_string(),
        }
    }

    pub fn source(source: &'static str, err: impl fmt::Display) -> Self {
        Self::SourceUnavailable {
            source,
            detail: err.to_string(),
        }
    }
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(msg) => write!(f, "{msg}"),
            Self::BuildNotFound => write!(f, "Build not found. Check the Build ID and try again."),
            Self::BuildHistoryEmpty => write!(
                f,
                "No builds found for this game. The SteamDB build history feed may be unreachable."
            ),
            Self::IncompleteSnapshot(depots) => write!(
                f,
                "Could not reconstruct a complete build snapshot: no manifest at or before the target build was found for depot(s) {}. Nothing was changed; use the Manual editor for this build.",
                depots.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
            ),
            Self::TokenInvalid => write!(
                f,
                "The build lookup service rejected the access token. Set a valid token in Settings or via the AETHERDESK_BUILD_TOKEN environment variable."
            ),
            Self::SourceUnavailable { source, detail } => {
                write!(f, "{source} lookup unavailable: {detail}")
            }
            Self::LuaMissing(path) => write!(
                f,
                "This game has no stplug-in Lua yet ({path}). Add the game first, then change its version."
            ),
            Self::Lua(detail) => write!(f, "Lua error: {detail}"),
            Self::Io { context, detail } => write!(f, "{context}: {detail}"),
            Self::Parse { context, detail } => write!(f, "Could not parse {context}: {detail}"),
        }
    }
}

impl From<VersionError> for String {
    fn from(err: VersionError) -> Self {
        err.to_string()
    }
}
