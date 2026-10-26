//! Syntax gate for a Lua package before AetherDesk publishes it.
//!
//! Hubcap generates `.lua` files from remote data; a malformed one publishes
//! cleanly and then fails inside the DLL's Lua VM, where the failure surfaces
//! as a game with no depots instead of an install error. This gate is
//! deliberately conservative and line-based: every statement in a Hubcap file
//! is one call to a known function on a single line, comments run from `--` to
//! the end of the line, and strings are single or double quoted with `\`
//! escapes. Anything else is rejected with a 1-based line number.
//!
//! It is not a Lua parser: it never executes or rewrites the file, and the
//! pin-level validation (`document`) stays the authority for what may be
//! written back to an already-installed file.

/// Functions a managed Lua file may call.
const ALLOWED_CALLS: [&str; 4] = ["addappid", "setManifestid", "addtoken", "seteticket"];

/// Removes a trailing `--` comment, keeping quoted strings intact.
fn strip_comment(line: &str, line_number: usize) -> Result<String, String> {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                out.push(c);
                if c == '\\' {
                    match chars.next() {
                        Some(escaped) => out.push(escaped),
                        None => return Err(format!("line {line_number}: unterminated escape")),
                    }
                } else if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                    out.push(c);
                } else if c == '-' && chars.peek() == Some(&'-') {
                    break;
                } else {
                    out.push(c);
                }
            }
        }
    }

    if quote.is_some() {
        return Err(format!("line {line_number}: unterminated string"));
    }
    Ok(out)
}

/// True when every `(` in `args` has a matching `)` outside quoted strings.
fn parentheses_balanced(args: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut chars = args.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == '\\' {
                    chars.next();
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                _ => {}
            },
        }
    }
    depth == 0 && quote.is_none()
}

/// Rejects a Lua body the DLL's VM would refuse to run.
pub fn validate_syntax(content: &str) -> Result<(), String> {
    for (index, raw) in content.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw, line_number)?;
        let statement = line.trim().trim_end_matches(';').trim();
        if statement.is_empty() {
            continue;
        }

        let Some(open) = statement.find('(') else {
            return Err(format!("line {line_number}: expected a function call"));
        };
        let name = statement[..open].trim();
        if !ALLOWED_CALLS.contains(&name) {
            return Err(format!(
                "line {line_number}: unknown call `{name}` (expected one of {})",
                ALLOWED_CALLS.join(", ")
            ));
        }
        if !statement.ends_with(')') || !parentheses_balanced(&statement[open..]) {
            return Err(format!(
                "line {line_number}: unbalanced parentheses in `{name}` call"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_syntax;

    #[test]
    fn accepts_hubcap_file() {
        let content = "-- 2369390's Lua and Manifest Created by Hubcap Manifest\n\
                       -- Far Cry 6\n\
                       addappid(2369390)\n\
                       addappid(2369391, 1, \"cc9d1e21a4e5b0a6f6e8a1c3d9e2b7f4c1a5d8e3b6f9c2a7d4e1b8c5f3a6d9e2\")\n\
                       setManifestid(2369391, \"5783041166523199015\", 118932477611)\n\
                       addtoken(2369390, \"1234567890\")\n\
                       seteticket(2369390, \"https://example.invalid/ticket\")\n";
        assert!(validate_syntax(content).is_ok());
    }

    #[test]
    fn rejects_dashx_broken_lua() {
        let content = "-- 2821390's Lua and Manifest Created by Hubcap Manifest\n\
                       -- Far Cry\n\
                       addappid(2821390, 1, \"x\" Hubcap\n";
        let error = validate_syntax(content).unwrap_err();
        assert!(error.starts_with("line 3:"), "{error}");
    }

    #[test]
    fn rejects_unterminated_string() {
        let error = validate_syntax("addappid(1, 1, \"abc)\n").unwrap_err();
        assert!(error.starts_with("line 1:"), "{error}");
        assert!(error.contains("string"), "{error}");
    }

    #[test]
    fn rejects_unknown_call() {
        let error = validate_syntax("addappidX(1)\n").unwrap_err();
        assert!(error.starts_with("line 1:"), "{error}");
        assert!(error.contains("unknown call"), "{error}");
    }

    #[test]
    fn ignores_commented_calls() {
        assert!(validate_syntax("-- addappid(1)\n   -- setManifestid(1, \"2\")\n").is_ok());
    }

    #[test]
    fn reports_the_offending_line() {
        let content = "addappid(1)\n\n  addtoken(1, \"2\"\n";
        let error = validate_syntax(content).unwrap_err();
        assert!(error.starts_with("line 3:"), "{error}");
    }
}
