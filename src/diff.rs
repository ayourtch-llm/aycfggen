/// Unified diff utilities using the `similar` crate.

/// Compute a unified diff between two strings, line by line.
///
/// Returns the unified diff as a `String`.  When both inputs are identical
/// (or both empty) an empty string is returned.
pub fn unified_diff(a: &str, b: &str) -> String {
    let diff = similar::TextDiff::from_lines(a, b);
    let result = diff.unified_diff().context_radius(3).to_string();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unified_diff_identical() {
        let text = "line1\nline2\nline3\n";
        assert_eq!(unified_diff(text, text), "");
    }

    #[test]
    fn test_unified_diff_added_line() {
        let a = "line1\nline2\n";
        let b = "line1\nline2\nline3\n";
        let diff = unified_diff(a, b);
        assert!(diff.contains("+line3"));
    }

    #[test]
    fn test_unified_diff_removed_line() {
        let a = "line1\nline2\nline3\n";
        let b = "line1\nline2\n";
        let diff = unified_diff(a, b);
        assert!(diff.contains("-line3"));
    }

    #[test]
    fn test_unified_diff_mixed() {
        let a = "line1\nline2\nline3\n";
        let b = "line1\nmodified\nline3\n";
        let diff = unified_diff(a, b);
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+modified"));
        assert!(diff.contains("@@")); // hunk header present
    }

    #[test]
    fn test_unified_diff_both_empty() {
        assert_eq!(unified_diff("", ""), "");
    }
}
