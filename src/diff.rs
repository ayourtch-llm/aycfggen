//! Unified diff utilities using the `similar` crate.

/// Compute a unified diff between two strings, line by line.
///
/// Returns the unified diff as a `String`.  When both inputs are identical
/// (or both empty) an empty string is returned.
pub fn unified_diff(a: &str, b: &str) -> String {
    similar::TextDiff::from_lines(a, b)
        .unified_diff()
        .context_radius(3)
        .to_string()
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
        assert!(diff.contains("+line3\n"));
    }

    #[test]
    fn test_unified_diff_removed_line() {
        let a = "line1\nline2\nline3\n";
        let b = "line1\nline2\n";
        let diff = unified_diff(a, b);
        assert!(diff.contains("-line3\n"));
    }

    #[test]
    fn test_unified_diff_mixed() {
        let a = "line1\nline2\nline3\n";
        let b = "line1\nmodified\nline3\n";
        let diff = unified_diff(a, b);
        assert!(diff.contains("-line2\n"));
        assert!(diff.contains("+modified\n"));
        assert!(diff.contains("@@")); // hunk header present
    }

    #[test]
    fn test_unified_diff_both_empty() {
        assert_eq!(unified_diff("", ""), "");
    }

    #[test]
    fn test_unified_diff_empty_to_nonempty() {
        let b = "some text\n";
        let diff = unified_diff("", b);
        assert!(diff.contains("+some text\n"));
        assert!(!diff.contains("-some text\n"));
    }

    #[test]
    fn test_unified_diff_nonempty_to_empty() {
        let a = "some text\n";
        let diff = unified_diff(a, "");
        assert!(diff.contains("-some text\n"));
        assert!(!diff.contains("+some text\n"));
    }
}
