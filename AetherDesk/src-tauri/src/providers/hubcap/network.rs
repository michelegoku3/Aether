//! Diagnosis for Hubcap transport failures.
//!
//! ISP and DNS filters surface as transport errors that look like outages, so
//! the message has to name the likely cause instead of echoing reqwest. The
//! only signal available is whether Steam answered: the store search queries
//! both sources in parallel, so reachable Steam with unreachable Hubcap is the
//! fingerprint of a filter that blocks one host.

use std::error::Error as StdError;
use std::io;
use std::time::Duration;

use crate::providers::health::{self, StoreSource};

/// How recent a Steam answer must be to count as "Steam responds".
const STEAM_ANSWER_WINDOW: Duration = Duration::from_secs(300);

const STEAM_RESPONDS_MESSAGE: &str = "Hubcap is unreachable while Steam responds. An ISP, or a DNS filter, or a firewall, or a VPN is probably blocking hubcapmanifest.com. Connecting through Cloudflare WARP or another VPN should fix it.";
const UNREACHABLE_MESSAGE: &str = "Hubcap is unreachable. An ISP, or a DNS filter, or a firewall, or a VPN is probably blocking hubcapmanifest.com. Connecting through Cloudflare WARP or another VPN should fix it.";
const GENERIC_PREFIX: &str = "Hubcap request failed";

const DNS_MARKERS: &[&str] = &[
    "dns error",
    "failed to lookup address",
    "nodename nor servname",
];
const TLS_MARKERS: &[&str] = &["tls", "ssl", "certificate", "handshake"];

/// Why a Hubcap request failed, at the level the user-facing message needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubcapFault {
    Dns,
    Reset,
    Timeout,
    Tls,
    BodyCut,
    HtmlPage,
    Other,
}

pub const HTML_INSTEAD_OF_ZIP: &str = "Hubcap answered with a web page instead of a ZIP: a proxy or filter intercepted the request. This is typical of an ISP block page or of a captive portal.";
pub const BODY_CUT_MESSAGE: &str = "Hubcap's response was cut off before the ZIP was complete. A filter, a proxy or an unstable connection interrupted it: retry, and if it keeps happening connect through Cloudflare WARP or another VPN.";

/// Walks `error` and its causes, outermost first.
fn chain(error: &reqwest::Error) -> impl Iterator<Item = &(dyn StdError + 'static)> {
    std::iter::successors(Some(error as &(dyn StdError + 'static)), |cause| {
        (*cause).source()
    })
}

fn chain_contains(error: &reqwest::Error, markers: &[&str]) -> bool {
    chain(error).any(|cause| {
        let text = cause.to_string().to_lowercase();
        markers.iter().any(|marker| text.contains(marker))
    })
}

fn io_error_kind(error: &reqwest::Error) -> Option<io::ErrorKind> {
    chain(error).find_map(|cause| cause.downcast_ref::<io::Error>().map(io::Error::kind))
}

/// Classify a reqwest failure by what it says about the connection.
pub fn classify(error: &reqwest::Error) -> HubcapFault {
    if error.is_timeout() {
        return HubcapFault::Timeout;
    }
    if chain_contains(error, DNS_MARKERS) {
        return HubcapFault::Dns;
    }
    match io_error_kind(error) {
        Some(io::ErrorKind::TimedOut) => return HubcapFault::Timeout,
        Some(io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted) => {
            return HubcapFault::Reset;
        }
        _ => {}
    }
    if chain_contains(error, TLS_MARKERS) {
        return HubcapFault::Tls;
    }
    if error.is_body() {
        return HubcapFault::BodyCut;
    }
    if error.is_decode() {
        return HubcapFault::HtmlPage;
    }
    if error.is_connect() {
        // Connect-level failure with no narrower cause: still a transport
        // block, which the message layer treats like the faults above.
        return HubcapFault::Other;
    }
    HubcapFault::Other
}

/// The user-facing diagnosis for a transport fault.
///
/// `steam_answered` is the discriminating signal: when Steam responds, the
/// failure is specific to hubcapmanifest.com rather than a general outage.
pub fn message_for(fault: HubcapFault, steam_answered: bool) -> &'static str {
    match fault {
        HubcapFault::Dns
        | HubcapFault::Reset
        | HubcapFault::Timeout
        | HubcapFault::Tls
        | HubcapFault::Other => {
            if steam_answered {
                STEAM_RESPONDS_MESSAGE
            } else {
                UNREACHABLE_MESSAGE
            }
        }
        HubcapFault::HtmlPage => HTML_INSTEAD_OF_ZIP,
        HubcapFault::BodyCut => BODY_CUT_MESSAGE,
    }
}

/// Turn a reqwest failure into the message shown to the user.
pub fn request_failed_message(error: &reqwest::Error) -> String {
    let fault = classify(error);
    let transport_block = matches!(
        fault,
        HubcapFault::Dns | HubcapFault::Reset | HubcapFault::Timeout | HubcapFault::Tls
    ) || (fault == HubcapFault::Other && error.is_connect());
    if !transport_block {
        return format!("{GENERIC_PREFIX}: {error}");
    }
    let steam_answered = health::answered_recently(StoreSource::Steam, STEAM_ANSWER_WINDOW);
    message_for(fault, steam_answered).to_string()
}

/// True when a response body is a web page rather than the expected payload.
///
/// A block page can arrive with an HTML content type or with no usable type at
/// all, so the leading markup is inspected too. Handing such a body to the ZIP
/// parser only yields a misleading archive error.
pub fn looks_like_html(content_type: Option<&str>, bytes: &[u8]) -> bool {
    if content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
    }) {
        return true;
    }

    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let head = &bytes[start..];
    (head.len() >= 9 && head[..9].eq_ignore_ascii_case(b"<!doctype"))
        || (head.len() >= 5 && head[..5].eq_ignore_ascii_case(b"<html"))
}

#[cfg(test)]
mod tests {
    use super::{looks_like_html, message_for, HubcapFault, BODY_CUT_MESSAGE, HTML_INSTEAD_OF_ZIP};

    const STEAM_RESPONDS: &str = "Hubcap is unreachable while Steam responds. An ISP, or a DNS filter, or a firewall, or a VPN is probably blocking hubcapmanifest.com. Connecting through Cloudflare WARP or another VPN should fix it.";
    const UNREACHABLE: &str = "Hubcap is unreachable. An ISP, or a DNS filter, or a firewall, or a VPN is probably blocking hubcapmanifest.com. Connecting through Cloudflare WARP or another VPN should fix it.";

    #[test]
    fn transport_faults_name_the_steam_contrast_when_it_is_known() {
        for fault in [
            HubcapFault::Dns,
            HubcapFault::Reset,
            HubcapFault::Timeout,
            HubcapFault::Tls,
            HubcapFault::Other,
        ] {
            assert_eq!(message_for(fault, true), STEAM_RESPONDS);
            assert_eq!(message_for(fault, false), UNREACHABLE);
        }
    }

    #[test]
    fn body_faults_describe_the_interception_instead_of_the_connection() {
        assert_eq!(message_for(HubcapFault::BodyCut, true), BODY_CUT_MESSAGE);
        assert_eq!(
            message_for(HubcapFault::HtmlPage, true),
            HTML_INSTEAD_OF_ZIP
        );
        assert_eq!(
            message_for(HubcapFault::HtmlPage, false),
            HTML_INSTEAD_OF_ZIP
        );
    }

    #[test]
    fn html_is_detected_by_content_type() {
        assert!(looks_like_html(Some("text/html; charset=utf-8"), b""));
        assert!(looks_like_html(Some("TEXT/HTML"), b"PK\x03\x04"));
    }

    #[test]
    fn html_is_detected_by_body_after_whitespace() {
        assert!(looks_like_html(None, b"\r\n  <!DOCTYPE html><html>"));
        assert!(looks_like_html(None, b"<html><body>blocked</body></html>"));
    }

    #[test]
    fn zip_and_json_bodies_are_not_html() {
        assert!(!looks_like_html(Some("application/zip"), b"PK\x03\x04"));
        assert!(!looks_like_html(None, b"PK\x03\x04"));
        assert!(!looks_like_html(
            Some("application/json"),
            br#"{"error":"nope"}"#
        ));
        assert!(!looks_like_html(None, br#"{"error":"nope"}"#));
    }
}
