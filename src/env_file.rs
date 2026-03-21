//! Parser for `.env` files.
//!
//! Supports `KEY=value` lines, `# comment` lines, blank lines,
//! quoted values (double and single), inline comments, and
//! optional `export` prefix (`export KEY=value`).
//!
//! **Not supported** (intentionally, to match uv's behavior):
//! - Variable expansion (`$VAR` or `${VAR}`)
//! - Escape sequences in quoted values (`\"`, `\n`, etc.)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur when parsing an env file.
#[derive(Debug, Error)]
pub enum EnvFileError {
    /// The env file could not be read.
    #[error("failed to read env file '{}': {source}", path.display())]
    ReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    /// A value has an opening quote with no matching closing quote.
    #[error("{path}:{line}: unclosed quote in value for '{key}'")]
    UnclosedQuote {
        path: PathBuf,
        line: usize,
        key: String,
    },
}

/// Parse a `.env` file into key-value pairs.
///
/// Returns an error if the file cannot be read or contains syntax errors.
pub fn parse(path: &Path) -> Result<BTreeMap<String, String>, EnvFileError> {
    let contents = std::fs::read_to_string(path).map_err(|source| EnvFileError::ReadFailed {
        path: path.to_path_buf(),
        source,
    })?;
    parse_str(&contents, Some(path))
}

/// Parse `.env` content from a string.
///
/// The optional `path` is used only for error messages.
fn parse_str(
    contents: &str,
    path: Option<&Path>,
) -> Result<BTreeMap<String, String>, EnvFileError> {
    let mut vars = BTreeMap::new();
    let display_path = path.unwrap_or_else(|| Path::new("<inline>"));

    for (line_num, line) in contents.lines().enumerate() {
        let trimmed = line.trim();

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Strip optional `export` prefix.
        let trimmed = trimmed
            .strip_prefix("export ")
            .or_else(|| trimmed.strip_prefix("export\t"))
            .unwrap_or(trimmed);

        // Split on first '='.
        let Some((key, raw_value)) = trimmed.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if key.is_empty() {
            continue;
        }

        let value = parse_value(raw_value, display_path, line_num + 1, key)?;
        vars.insert(key.to_string(), value);
    }

    Ok(vars)
}

/// Parse a value, handling quoting and inline comments.
///
/// Double-quoted values: `KEY="value with spaces"` — content between quotes is
/// taken literally (no escape processing).
///
/// Single-quoted values: `KEY='literal $value'` — same, fully literal.
///
/// Unquoted values: trailing ` # comment` is stripped as an inline comment.
fn parse_value(
    raw: &str,
    path: &Path,
    line: usize,
    key: &str,
) -> Result<String, EnvFileError> {
    let raw = raw.trim_start();

    // Double-quoted value.
    if raw.starts_with('"') {
        return match raw[1..].find('"') {
            Some(end) => Ok(raw[1..1 + end].to_string()),
            None => Err(EnvFileError::UnclosedQuote {
                path: path.to_path_buf(),
                line,
                key: key.to_string(),
            }),
        };
    }

    // Single-quoted value (literal, no escapes).
    if raw.starts_with('\'') {
        return match raw[1..].find('\'') {
            Some(end) => Ok(raw[1..1 + end].to_string()),
            None => Err(EnvFileError::UnclosedQuote {
                path: path.to_path_buf(),
                line,
                key: key.to_string(),
            }),
        };
    }

    // Unquoted: strip inline comments (` #`) and trailing whitespace.
    let value = match raw.find(" #") {
        Some(idx) => &raw[..idx],
        None => raw,
    };

    Ok(value.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: parse a string with no file path context.
    fn p(input: &str) -> BTreeMap<String, String> {
        parse_str(input, None).unwrap()
    }

    // --- Basic parsing ---

    #[test]
    fn basic_key_value() {
        let vars = p("KEY=value");
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn multiple_lines() {
        let vars = p("A=1\nB=2\nC=3");
        assert_eq!(vars.len(), 3);
        assert_eq!(vars["A"], "1");
        assert_eq!(vars["B"], "2");
        assert_eq!(vars["C"], "3");
    }

    #[test]
    fn empty_value() {
        let vars = p("KEY=");
        assert_eq!(vars["KEY"], "");
    }

    #[test]
    fn value_with_equals() {
        let vars = p("KEY=a=b=c");
        assert_eq!(vars["KEY"], "a=b=c");
    }

    #[test]
    fn whitespace_around_key() {
        let vars = p("  KEY  =value");
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn trailing_whitespace_stripped() {
        let vars = p("KEY=value   ");
        assert_eq!(vars["KEY"], "value");
    }

    // --- Comments and blanks ---

    #[test]
    fn comments_and_blanks_skipped() {
        let input = "# this is a comment\n\nKEY=value\n\n# another comment\n";
        let vars = p(input);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn inline_comment() {
        let vars = p("KEY=value # this is a comment");
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn hash_without_space_not_a_comment() {
        let vars = p("KEY=value#notacomment");
        assert_eq!(vars["KEY"], "value#notacomment");
    }

    // --- Quoting ---

    #[test]
    fn double_quoted_value() {
        let vars = p("KEY=\"value with spaces\"");
        assert_eq!(vars["KEY"], "value with spaces");
    }

    #[test]
    fn single_quoted_value() {
        let vars = p("KEY='literal $value'");
        assert_eq!(vars["KEY"], "literal $value");
    }

    #[test]
    fn inline_comment_not_stripped_in_double_quotes() {
        let vars = p("KEY=\"value # not a comment\"");
        assert_eq!(vars["KEY"], "value # not a comment");
    }

    #[test]
    fn inline_comment_not_stripped_in_single_quotes() {
        let vars = p("KEY='value # not a comment'");
        assert_eq!(vars["KEY"], "value # not a comment");
    }

    #[test]
    fn double_quoted_empty() {
        let vars = p("KEY=\"\"");
        assert_eq!(vars["KEY"], "");
    }

    #[test]
    fn single_quoted_empty() {
        let vars = p("KEY=''");
        assert_eq!(vars["KEY"], "");
    }

    // --- Unclosed quotes ---

    #[test]
    fn unclosed_double_quote_errors() {
        let result = parse_str("KEY=\"unterminated", None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("unclosed quote"));
        assert!(msg.contains("KEY"));
    }

    #[test]
    fn unclosed_single_quote_errors() {
        let result = parse_str("KEY='unterminated", None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("unclosed quote"));
    }

    #[test]
    fn unclosed_quote_reports_line_number() {
        let result = parse_str("A=1\nB=2\nKEY=\"bad", None);
        match result.unwrap_err() {
            EnvFileError::UnclosedQuote { line, key, .. } => {
                assert_eq!(line, 3);
                assert_eq!(key, "KEY");
            }
            other => panic!("expected UnclosedQuote, got {:?}", other),
        }
    }

    // --- export prefix ---

    #[test]
    fn export_prefix_stripped() {
        let vars = p("export KEY=value");
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn export_prefix_with_tab() {
        let vars = p("export\tKEY=value");
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn export_without_space_is_a_key() {
        // "exportKEY=value" — "export" is not a prefix here
        let vars = p("exportKEY=value");
        assert_eq!(vars["exportKEY"], "value");
    }

    // --- Duplicate keys ---

    #[test]
    fn duplicate_keys_last_wins() {
        let vars = p("KEY=first\nKEY=second");
        assert_eq!(vars["KEY"], "second");
        assert_eq!(vars.len(), 1);
    }

    // --- Line endings ---

    #[test]
    fn windows_line_endings() {
        let vars = p("A=1\r\nB=2\r\nC=3\r\n");
        assert_eq!(vars.len(), 3);
        assert_eq!(vars["A"], "1");
        assert_eq!(vars["B"], "2");
        assert_eq!(vars["C"], "3");
    }

    // --- UTF-8 ---

    #[test]
    fn utf8_values() {
        let vars = p("GREETING=こんにちは\nEMOJI=🚀");
        assert_eq!(vars["GREETING"], "こんにちは");
        assert_eq!(vars["EMOJI"], "🚀");
    }

    // --- Skipped lines ---

    #[test]
    fn line_without_equals_skipped() {
        let vars = p("not_a_valid_line\nKEY=value");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars["KEY"], "value");
    }

    #[test]
    fn indented_comment_skipped() {
        let vars = p("  # indented comment\nKEY=value");
        assert_eq!(vars.len(), 1);
    }

    // --- File I/O ---

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

    #[test]
    fn parse_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "").unwrap();

        let vars = parse(&path).unwrap();
        assert!(vars.is_empty());
    }
}
