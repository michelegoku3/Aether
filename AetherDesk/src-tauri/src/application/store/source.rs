/// Public-source sentinel retained at the IPC boundary for compatibility with
/// the existing frontend settings model.
const PUBLIC_SOURCE_KEY: &str = "oureveryday_public";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HubcapSource {
    Hubcap,
    Oureveryday,
}

impl HubcapSource {
    pub(super) fn from_api_key(api_key: &str) -> Self {
        if api_key == PUBLIC_SOURCE_KEY {
            Self::Oureveryday
        } else {
            Self::Hubcap
        }
    }

    pub(super) fn log_name(self) -> &'static str {
        match self {
            Self::Hubcap => "hubcap",
            Self::Oureveryday => "oureveryday",
        }
    }

    pub(super) fn display_name(self) -> &'static str {
        match self {
            Self::Hubcap => "Hubcap",
            Self::Oureveryday => "MOED",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HubcapSource;

    #[test]
    fn public_sentinel_selects_oureveryday_without_leaking_into_workflows() {
        assert_eq!(
            HubcapSource::from_api_key("oureveryday_public"),
            HubcapSource::Oureveryday
        );
        assert_eq!(HubcapSource::from_api_key("secret"), HubcapSource::Hubcap);
    }
}
