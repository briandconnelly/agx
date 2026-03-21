//! Parser for `.env` files.
//!
//! Supports `KEY=value` lines, `# comment` lines, blank lines,
//! quoted values (double and single), and inline comments.
//! No variable expansion — values are taken literally.

use std::collections::BTreeMap;
use std::path::Path;

/// Parse a `.env` file into key-value pairs.
///
/// Returns an error if the file cannot be read.
pub fn parse(path: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read env file '{}': {}", path.display(), e))?;
    Ok(parse_str(&contents))
}

/// Parse `.env` content from a string.
fn parse_str(contents: &str) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();

    for line in contents.lines() {
        let trimmed = line.trim();

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Split on first '='.
        let Some((key, raw_value)) = trimmed.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if key.is_empty() {
            continue;
        }

        let value = parse_value(raw_value);
        vars.insert(key.to_string(), value);
    }

    vars
}

/// Parse a value, handling quoting and inline comments.
fn parse_value(raw: &str) -> String {
    let raw = raw.trim_start();

    // Double-quoted value.
    if raw.starts_with('"') {
        if let Some(end) = raw[1..].find('"') {
            return raw[1..1 + end].to_string();
        }
    }

    // Single-quoted value (literal, no escapes).
    if raw.starts_with('\'') {
        if let Some(end) = raw[1..].find('\'') {
            return raw[1..1 + end].to_string();
        }
    }

    // Unquoted: strip inline comments and trailing whitespace.
    let value = if let Some(idx) = raw.find(" #") {
        &raw[..idx]
    } else {
        raw
    };

    value.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_key_value() {
        let vars = parse_str("KEY=value");
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn multiple_lines() {
        let vars = parse_str("A=1\nB=2\nC=3");
        assert_eq!(vars.len(), 3);
        assert_eq!(vars["A"], "1");
        assert_eq!(vars["B"], "2");
        assert_eq!(vars["C"], "3");
    }

    #[test]
    fn comments_and_blanks_skipped() {
        let input = "# this is a comment\n\nKEY=value\n\n# another comment\n";
        let vars = parse_str(input);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn double_quoted_value() {
        let vars = parse_str("KEY=\"value with spaces\"");
        assert_eq!(vars["KEY"], "value with spaces");
    }

    #[test]
    fn single_quoted_value() {
        let vars = parse_str("KEY='literal $value'");
        assert_eq!(vars["KEY"], "literal $value");
    }

    #[test]
    fn inline_comment() {
        let vars = parse_str("KEY=value # this is a comment");
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn inline_comment_not_stripped_in_quotes() {
        let vars = parse_str("KEY=\"value # not a comment\"");
        assert_eq!(vars["KEY"], "value # not a comment");
    }

    #[test]
    fn empty_value() {
        let vars = parse_str("KEY=");
        assert_eq!(vars["KEY"], "");
    }

    #[test]
    fn value_with_equals() {
        let vars = parse_str("KEY=a=b=c");
        assert_eq!(vars["KEY"], "a=b=c");
    }

    #[test]
    fn whitespace_around_key() {
        let vars = parse_str("  KEY  =value");
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn line_without_equals_skipped() {
        let vars = parse_str("not_a_valid_line\nKEY=value");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn file_not_found_error() {
        let result = parse(Path::new("/nonexistent/.env"));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("failed to read env file"));
    }

    #[test]
    fn parse_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "HOST=localhost\nPORT=8080\n").unwrap();

        let vars = parse(&path).unwrap();
        assert_eq!(vars["HOST"], "localhost");
        assert_eq!(vars["PORT"], "8080");
    }
}
