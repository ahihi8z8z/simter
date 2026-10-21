//! Line tokenizer following OpenSSH's `readconf.c` / `argv_split()` rules.

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Parses one config line into a lowercased keyword and its arguments.
///
/// Returns `Ok(None)` for blank and comment lines. The keyword ends at
/// whitespace or `=`; a single `=` between keyword and arguments is allowed.
pub(crate) fn parse_line(line: &str) -> Result<Option<(String, Vec<String>)>, String> {
    let s = line.trim_start_matches(is_ws);
    if s.is_empty() || s.starts_with('#') {
        return Ok(None);
    }
    let kw_end = s.find(|c: char| is_ws(c) || c == '=').unwrap_or(s.len());
    let keyword = s[..kw_end].to_ascii_lowercase();
    let mut rest = s[kw_end..].trim_start_matches(is_ws);
    if let Some(r) = rest.strip_prefix('=') {
        rest = r.trim_start_matches(is_ws);
    }
    Ok(Some((keyword, split_args(rest)?)))
}

/// Splits arguments with single/double quoting and backslash escapes.
/// An unquoted token starting with `#` begins a trailing comment.
fn split_args(s: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = s.chars().collect();
    let mut args = Vec::new();
    let mut i = 0;
    loop {
        while i < chars.len() && is_ws(chars[i]) {
            i += 1;
        }
        if i >= chars.len() || chars[i] == '#' {
            break;
        }
        let mut arg = String::new();
        let mut quote: Option<char> = None;
        while i < chars.len() {
            let c = chars[i];
            if quote.is_none() && is_ws(c) {
                break;
            }
            if c == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                if matches!(next, '\'' | '"' | '\\') || (quote.is_none() && next == ' ') {
                    arg.push(next);
                    i += 2;
                    continue;
                }
            }
            match quote {
                None if c == '"' || c == '\'' => quote = Some(c),
                Some(q) if c == q => quote = None,
                _ => arg.push(c),
            }
            i += 1;
        }
        if quote.is_some() {
            return Err("unterminated quoted string".into());
        }
        args.push(arg);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> (String, Vec<String>) {
        parse_line(line).unwrap().unwrap()
    }

    #[test]
    fn skips_blank_and_comment_lines() {
        assert_eq!(parse_line("").unwrap(), None);
        assert_eq!(parse_line("   \t").unwrap(), None);
        assert_eq!(parse_line("  # Host commented").unwrap(), None);
    }

    #[test]
    fn keyword_is_case_insensitive_and_accepts_equals() {
        assert_eq!(parse("HOST alpha"), ("host".into(), vec!["alpha".into()]));
        assert_eq!(parse("Host=alpha"), ("host".into(), vec!["alpha".into()]));
        assert_eq!(
            parse("Host = alpha beta"),
            ("host".into(), vec!["alpha".into(), "beta".into()])
        );
    }

    #[test]
    fn handles_quotes_escapes_and_trailing_comments() {
        assert_eq!(
            parse(r#"Host "my server" b # note"#).1,
            vec!["my server", "b"]
        );
        assert_eq!(parse(r"Host a\ b").1, vec!["a b"]);
        assert_eq!(parse(r#"Host 'x"y'"#).1, vec![r#"x"y"#]);
        assert_eq!(parse(r#"Host "a#b""#).1, vec!["a#b"]);
    }

    #[test]
    fn keeps_windows_backslashes_literal() {
        assert_eq!(
            parse(r"Include C:\Users\me\ssh\extra").1,
            vec![r"C:\Users\me\ssh\extra"]
        );
    }

    #[test]
    fn strips_crlf() {
        assert_eq!(parse("Host alpha\r"), ("host".into(), vec!["alpha".into()]));
    }

    #[test]
    fn rejects_unterminated_quote() {
        assert!(parse_line(r#"Host "broken"#).is_err());
    }
}
