use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;
use roxmltree::Document;

use crate::providers::http;
use crate::versioning::error::VersionError;
use crate::versioning::model::BuildInfo;
use crate::versioning::sources::{BoxFuture, BuildHistorySource};

const RSS_TIMEOUT_SECONDS: u64 = 25;

/// Build IDs live in the patchnotes links (`/patchnotes/1234567/`)…
static BUILD_LINK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"/patchnotes/(\d+)").expect("static regex"));
/// …and fall back to the title ("Build 1234567 – …" / "SteamDB Build 1234567").
static BUILD_TITLE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)build\s*#?\s*(\d+)").expect("static regex"));
/// "Mon, 08 May 2023 01:01:00 +0000" → day / month / year.
static PUB_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(\d{1,2})\s+(\w{3})\s+(\d{4})").expect("static regex"));

const MONTHS: &[(&str, &str)] = &[
    ("jan", "01"),
    ("feb", "02"),
    ("mar", "03"),
    ("apr", "04"),
    ("may", "05"),
    ("jun", "06"),
    ("jul", "07"),
    ("aug", "08"),
    ("sep", "09"),
    ("oct", "10"),
    ("nov", "11"),
    ("dec", "12"),
];

/// SteamDB PatchnotesRSS feed (`https://steamdb.info/api/PatchnotesRSS/?appid=…`).
/// Public XML, no authentication: each `<item>` is one published build.
pub struct SteamDbPatchnotesSource {
    client: reqwest::Client,
}

impl SteamDbPatchnotesSource {
    pub fn new() -> Self {
        Self {
            client: http::build_client(RSS_TIMEOUT_SECONDS),
        }
    }
}

impl BuildHistorySource for SteamDbPatchnotesSource {
    fn build_history(&self, app_id: u32) -> BoxFuture<'_, Result<Vec<BuildInfo>, VersionError>> {
        Box::pin(async move {
            let url = format!("https://steamdb.info/api/PatchnotesRSS/?appid={}", app_id);
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| VersionError::source("SteamDB", e))?;
            let status = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| VersionError::source("SteamDB", e))?;
            if !status.is_success() {
                return Err(VersionError::SourceUnavailable {
                    source: "SteamDB",
                    detail: format!("HTTP {}", status.as_u16()),
                });
            }
            parse_patchnotes_rss(&text)
        })
    }
}

/// Parses the RSS payload into a newest-first build list (one entry per
/// published build; duplicates collapsed by build id).
pub fn parse_patchnotes_rss(xml_text: &str) -> Result<Vec<BuildInfo>, VersionError> {
    let doc = Document::parse(xml_text)
        .map_err(|e| VersionError::parse("SteamDB build feed", e))?;

    let mut builds: Vec<BuildInfo> = Vec::new();
    let mut seen: HashSet<u64> = HashSet::new();

    for item in doc.descendants().filter(|n| n.has_tag_name("item")) {
        let mut title = String::new();
        let mut link = String::new();
        let mut pub_date = String::new();

        for child in item.children().filter(|n| n.is_element()) {
            let text = child.text().unwrap_or("").trim().to_string();
            match child.tag_name().name() {
                "title" => title = text,
                "link" => link = text,
                "pubDate" => pub_date = text,
                _ => {}
            }
        }

        let mut build_id: u64 = 0;
        if let Some(caps) = BUILD_LINK_RE.captures(&link) {
            build_id = caps[1].parse().unwrap_or(0);
        }
        if build_id == 0 {
            if let Some(caps) = BUILD_TITLE_RE.captures(&title) {
                build_id = caps[1].parse().unwrap_or(0);
            }
        }
        if build_id == 0 || !seen.insert(build_id) {
            continue;
        }

        builds.push(BuildInfo {
            build_id,
            date: parse_pub_date(&pub_date),
            title,
        });
    }

    // Newest first; same-day builds ordered by descending build id.
    builds.sort_by(|a, b| b.date.cmp(&a.date).then(b.build_id.cmp(&a.build_id)));
    Ok(builds)
}

/// "Mon, 08 May 2023 01:01:00 +0000" → "2023-05-08" ("" when unrecognizable).
fn parse_pub_date(pub_date: &str) -> String {
    let Some(caps) = PUB_DATE_RE.captures(pub_date) else {
        return String::new();
    };
    let day = &caps[1];
    let month = &caps[2];
    let year = &caps[3];
    let Some((_, month_num)) = MONTHS.iter().find(|(abbr, _)| abbr.eq_ignore_ascii_case(month))
    else {
        return String::new();
    };
    format!("{}-{}-{:0>2}", year, month_num, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>SteamDB Builds for Counter-Strike 2</title>
    <item>
      <title>Counter-Strike 2 update for 12 August 2026</title>
      <link>https://steamdb.info/patchnotes/24701871/?utm_source=rss</link>
      <pubDate>Wed, 12 Aug 2026 22:48:59 +0000</pubDate>
    </item>
    <item>
      <title>SteamDB Build 24662694</title>
      <link>https://steamdb.info/patchnotes/24662694/</link>
      <pubDate>Mon, 10 Aug 2026 23:48:17 +0000</pubDate>
    </item>
    <item>
      <title>Build 24661132 – Bug fix</title>
      <link>https://steamdb.info/patchnotes/24661132/</link>
      <pubDate>Mon, 10 Aug 2026 21:54:54 +0000</pubDate>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn parses_feed_newest_first() {
        let builds = parse_patchnotes_rss(FEED).unwrap();
        assert_eq!(builds.len(), 3);
        assert_eq!(builds[0].build_id, 24701871);
        assert_eq!(builds[0].date, "2026-08-12");
        assert_eq!(builds[1].build_id, 24662694);
        assert_eq!(builds[2].build_id, 24661132);
        // Same-day ordering: higher build id first.
        assert!(builds[1].build_id > builds[2].build_id);
        assert_eq!(builds[1].date, "2026-08-10");
    }

    #[test]
    fn dedupes_and_ignores_garbage() {
        let xml = r#"<rss><channel>
            <item><title>Build 1111111</title><link>x</link><pubDate>Tue, 05 May 2020 00:00:00 +0000</pubDate></item>
            <item><title>Build 1111111</title><link>x</link><pubDate>Tue, 05 May 2020 00:00:00 +0000</pubDate></item>
            <item><title>no build here</title><link>https://example.com/</link><pubDate>nonsense</pubDate></item>
        </channel></rss>"#;
        let builds = parse_patchnotes_rss(xml).unwrap();
        assert_eq!(builds.len(), 1);
        assert_eq!(builds[0].build_id, 1111111);
        assert_eq!(builds[0].date, "2020-05-05");
    }

    #[test]
    fn rejects_html_challenge() {
        assert!(parse_patchnotes_rss("<!doctype html><html><body>cf challenge</body></html>").is_err());
    }
}
