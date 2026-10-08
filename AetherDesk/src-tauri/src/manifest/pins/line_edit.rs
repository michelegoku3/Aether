//! Lossless line-level rewrites used by the manifest editor.

use once_cell::sync::Lazy;
use regex::Regex;

static SETMANIFEST_REWRITE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"^(?P<indent>\s*)(?P<comment>--\s*)?(?P<func>(?i:setmanifestid))\s*\(\s*(?P<appid>\d+)\s*,\s*["'][^"']+["']\s*(?:,[^)]*)?\)(?P<suffix>.*)$"#,
    )
    .expect("valid regex")
});

static SETMANIFEST_DEPOT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:--\s*)?setmanifestid\s*\(\s*(?P<appid>\d+)")
        .expect("valid regex")
});

pub(super) fn rewrite_setmanifest_line(
    line: &str,
    next_manifest_id: &str,
) -> Result<String, String> {
    if let Some(captures) = SETMANIFEST_REWRITE_RE.captures(line) {
        return Ok(format!(
            "{}{}{}({}, \"{}\"){}",
            captures.name("indent").map(|value| value.as_str()).unwrap_or(""),
            captures.name("comment").map(|value| value.as_str()).unwrap_or(""),
            captures
                .name("func")
                .map(|value| value.as_str())
                .unwrap_or("setManifestid"),
            captures.name("appid").map(|value| value.as_str()).unwrap_or("0"),
            next_manifest_id,
            captures.name("suffix").map(|value| value.as_str()).unwrap_or(""),
        ));
    }

    let captures = SETMANIFEST_DEPOT_RE
        .captures(line)
        .ok_or_else(|| "Line is not a valid setManifestid call".to_string())?;
    let app_id = captures
        .name("appid")
        .map(|value| value.as_str())
        .unwrap_or("0")
        .to_string();
    let head_end = captures.get(0).map(|value| value.end()).unwrap_or(0);
    let head = line.get(..head_end).unwrap_or_default();
    let head_trimmed = head.trim_start();
    let indent = head
        .get(..head.len().saturating_sub(head_trimmed.len()))
        .unwrap_or("");
    let comment = if head_trimmed.starts_with("--") {
        "-- "
    } else {
        ""
    };
    let function = head_trimmed
        .trim_start_matches('-')
        .trim_start()
        .split(['(', ' ', '\t'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("setManifestid");
    let rest = line.get(head_end..).unwrap_or_default();
    let suffix = rest
        .find("--")
        .map(|comment_at| rest[comment_at..].trim_start())
        .unwrap_or("");

    if suffix.is_empty() {
        Ok(format!(
            "{indent}{comment}{function}({app_id}, \"{next_manifest_id}\")"
        ))
    } else {
        Ok(format!(
            "{indent}{comment}{function}({app_id}, \"{next_manifest_id}\") {suffix}"
        ))
    }
}

pub(super) fn set_commented(line: &mut String, commented: bool) {
    let is_commented = line.trim_start().starts_with("--");
    match (commented, is_commented) {
        (true, false) => *line = format!("--{}", line),
        (false, true) => {
            let leading_len = line.len() - line.trim_start().len();
            let (leading, rest) = line.split_at(leading_len);
            let rest = rest.strip_prefix("--").unwrap_or(rest).trim_start();
            *line = format!("{}{}", leading, rest);
        }
        _ => {}
    }
}

pub(super) fn join_lua_lines(lines: &[String]) -> String {
    let mut content = lines.join("\n");
    content.push('\n');
    content
}
