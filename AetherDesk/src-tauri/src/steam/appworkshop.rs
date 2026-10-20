//! Scoped parser for Steam's `appworkshop_*.acf` files.
//!
//! The ACF mixes the real installed items with per-branch `manifest_history`
//! blocks whose keys are small indices. Treating every quoted numeric key as a
//! Workshop ID turns those indices into phantom subscriptions, so this parser
//! reads only direct children of `WorkshopItemsInstalled` and
//! `WorkshopItemDetails`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkshopAcfItem {
    pub workshop_id: u64,
    pub manifest_gid: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorkshopAcf {
    /// Direct children of `WorkshopItemsInstalled` with a positive manifest.
    pub installed: Vec<WorkshopAcfItem>,
    /// Direct children of `WorkshopItemDetails` with a positive manifest.
    /// These are subscriptions Steam has not installed yet.
    pub subscribed: Vec<WorkshopAcfItem>,
    /// Installed entries with a `-1`/zero/missing manifest (legacy UGC items).
    pub legacy_installed: usize,
    /// Top-level `NeedsDownload "1"`.
    pub needs_download: bool,
}

/// Parses the two Workshop sections of an `appworkshop_*.acf`.
pub fn parse_workshop_acf(content: &str) -> Result<WorkshopAcf, String> {
    let installed_lines = section_lines(content, "WorkshopItemsInstalled")?;
    let detail_lines = section_lines(content, "WorkshopItemDetails")?;
    if installed_lines.is_none() && detail_lines.is_none() {
        return Err("no Workshop sections".to_string());
    }

    let mut result = WorkshopAcf::default();
    if let Some(lines) = installed_lines {
        for raw in parse_item_blocks(&lines)? {
            match raw.manifest {
                Some(manifest_gid) => result.installed.push(WorkshopAcfItem {
                    workshop_id: raw.workshop_id,
                    manifest_gid,
                }),
                None => result.legacy_installed += 1,
            }
        }
    }
    if let Some(lines) = detail_lines {
        for raw in parse_item_blocks(&lines)? {
            if let Some(manifest_gid) = raw.manifest {
                result.subscribed.push(WorkshopAcfItem {
                    workshop_id: raw.workshop_id,
                    manifest_gid,
                });
            }
        }
    }

    result.needs_download = top_level_value(content, "NeedsDownload").as_deref() == Some("1");
    Ok(result)
}

/// Lines inside the top-level `"<name>" { … }` block, excluding its braces.
fn section_lines<'a>(content: &'a str, name: &str) -> Result<Option<Vec<&'a str>>, String> {
    let lines: Vec<&str> = content.lines().collect();
    let needle = format!("\"{name}\"");
    let Some(start) = lines
        .iter()
        .position(|line| line.trim().eq_ignore_ascii_case(&needle))
    else {
        return Ok(None);
    };

    let mut index = start + 1;
    while index < lines.len() && lines[index].trim().is_empty() {
        index += 1;
    }
    if index >= lines.len() || lines[index].trim() != "{" {
        return Err(format!("Workshop section {name} has no opening brace"));
    }
    index += 1;

    let mut depth = 1i32;
    let mut body = Vec::new();
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed == "{" {
            depth += 1;
        } else if trimmed == "}" {
            depth -= 1;
            if depth == 0 {
                return Ok(Some(body));
            }
        }
        body.push(lines[index]);
        index += 1;
    }
    Err(format!("Workshop section {name} is not closed"))
}

/// Top-level `"key" "value"` inside `AppWorkshop`, ignoring nested blocks.
fn top_level_value(content: &str, key: &str) -> Option<String> {
    let mut depth = 0i32;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "{" {
            depth += 1;
            continue;
        }
        if trimmed == "}" {
            depth -= 1;
            continue;
        }
        if depth == 1 {
            if let Some((found, value)) = parse_key_value(trimmed) {
                if found.eq_ignore_ascii_case(key) {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

#[derive(Debug)]
struct RawItem {
    workshop_id: u64,
    manifest: Option<u64>,
}

/// Direct children of a Workshop section. Nested blocks are traversed for brace
/// balance but their keys are never read as items or manifests.
fn parse_item_blocks(lines: &[&str]) -> Result<Vec<RawItem>, String> {
    let mut items = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let Some(workshop_id) = parse_standalone_numeric_key(lines[index]) else {
            index += 1;
            continue;
        };

        let mut cursor = index + 1;
        while cursor < lines.len() && lines[cursor].trim().is_empty() {
            cursor += 1;
        }
        if cursor >= lines.len() || lines[cursor].trim() != "{" {
            index += 1;
            continue;
        }
        cursor += 1;

        let mut depth = 1i32;
        let mut manifest = None;
        while cursor < lines.len() {
            let trimmed = lines[cursor].trim();
            if trimmed == "{" {
                depth += 1;
            } else if trimmed == "}" {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            } else if depth == 1 {
                if let Some((key, value)) = parse_key_value(trimmed) {
                    if key.eq_ignore_ascii_case("manifest") {
                        manifest = value.parse::<u64>().ok().filter(|gid| *gid > 0);
                    }
                }
            }
            cursor += 1;
        }
        if depth != 0 {
            return Err(format!("Workshop item {workshop_id} is not closed"));
        }

        items.push(RawItem {
            workshop_id,
            manifest,
        });
        index = cursor + 1;
    }
    Ok(items)
}

/// A standalone quoted decimal key such as `\t"3806068832"`; a key-value line
/// like `\t"manifest"\t\t"123"` is not one.
fn parse_standalone_numeric_key(line: &str) -> Option<u64> {
    let trimmed = line.trim();
    if !trimmed.starts_with('"') || !trimmed.ends_with('"') {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.is_empty() || !inner.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    inner.parse().ok()
}

/// Splits a simple `"key" "value"` VDF line, rejecting trailing content.
fn parse_key_value(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('"') {
        return None;
    }
    let key_end = trimmed[1..].find('"')? + 1;
    let key = &trimmed[1..key_end];
    let rest = trimmed[key_end + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let rest = &rest[1..];
    let value_end = rest.find('"')?;
    let value = &rest[..value_end];
    if !rest[value_end + 1..].trim().is_empty() {
        return None;
    }
    Some((key, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NESTED_HISTORY: &str = r#""AppWorkshop"
{
	"appid"		"1"
	"NeedsDownload"		"1"
	"WorkshopItemsInstalled"
	{
		"111"
		{
			"manifest"		"1001"
			"manifest_history"
			{
				"0"
				{
					"manifest"		"9001"
				}
				"19"
				{
					"manifest"		"9019"
				}
			}
		}
		"222"
		{
			"manifest"		"1002"
			"manifest_history"
			{
				"0"
				{
					"manifest"		"9000"
				}
			}
		}
		"333"
		{
			"manifest"		"-1"
			"ugchandle"		"abc"
		}
	}
	"WorkshopItemDetails"
	{
	}
}
"#;

    #[test]
    fn read_only_installed_direct_children() {
        let parsed = parse_workshop_acf(NESTED_HISTORY).unwrap();

        assert_eq!(
            parsed.installed,
            vec![
                WorkshopAcfItem {
                    workshop_id: 111,
                    manifest_gid: 1001,
                },
                WorkshopAcfItem {
                    workshop_id: 222,
                    manifest_gid: 1002,
                },
            ]
        );
        assert_eq!(parsed.legacy_installed, 1);
        assert!(!parsed
            .installed
            .iter()
            .any(|item| item.workshop_id == 0 || item.workshop_id == 19));
    }

    #[test]
    fn ignores_latest_manifest() {
        let content = r#""AppWorkshop"
{
	"WorkshopItemsInstalled"
	{
		"3747222633"
		{
			"manifest"		"634883037376787724"
		}
	}
	"WorkshopItemDetails"
	{
		"3747222633"
		{
			"manifest"		"4458352795112025122"
			"latest_manifest"		"9999999999999999999"
		}
	}
}
"#;

        let parsed = parse_workshop_acf(content).unwrap();

        assert_eq!(parsed.installed[0].manifest_gid, 634883037376787724);
        assert_eq!(parsed.subscribed[0].manifest_gid, 4458352795112025122);
    }

    #[test]
    fn collects_details_only_subscriptions() {
        let content = r#""AppWorkshop"
{
	"WorkshopItemsInstalled"
	{
	}
	"WorkshopItemDetails"
	{
		"3806068832"
		{
			"manifest"		"3748992252827399921"
			"latest_manifest"		"3748992252827399921"
		}
	}
}
"#;

        let parsed = parse_workshop_acf(content).unwrap();

        assert!(parsed.installed.is_empty());
        assert_eq!(
            parsed.subscribed,
            vec![WorkshopAcfItem {
                workshop_id: 3806068832,
                manifest_gid: 3748992252827399921,
            }]
        );
    }

    #[test]
    fn reads_needs_download() {
        let parsed = parse_workshop_acf(NESTED_HISTORY).unwrap();
        assert!(parsed.needs_download);

        let without =
            NESTED_HISTORY.replace("\"NeedsDownload\"\t\t\"1\"", "\"NeedsDownload\"\t\t\"0\"");
        assert!(!parse_workshop_acf(&without).unwrap().needs_download);
    }

    #[test]
    fn rejects_unclosed_section() {
        let broken = r#""AppWorkshop"
{
	"WorkshopItemsInstalled"
	{
		"111"
		{
			"manifest"		"1001"
		}
"#;

        assert!(parse_workshop_acf(broken).is_err());
    }

    #[test]
    fn handles_crlf() {
        let crlf = NESTED_HISTORY.replace('\n', "\r\n");
        let parsed = parse_workshop_acf(&crlf).unwrap();

        assert_eq!(parsed.installed.len(), 2);
        assert_eq!(parsed.legacy_installed, 1);
    }

    #[test]
    fn missing_both_sections_is_an_error() {
        let content = "\"AppWorkshop\"\n{\n\t\"appid\"\t\t\"1\"\n}\n";
        assert!(parse_workshop_acf(content).is_err());
    }
}
