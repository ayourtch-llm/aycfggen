use anyhow::Result;
use regex::Regex;
use crate::sources::ConfigElementSource;

/// Expand !!!###<element-name> markers in a template.
/// Each marker must be the entire content of a line (trimmed).
/// Element names must match [a-zA-Z0-9_-]+.
/// Replaces the marker line with "! config-element: <name>\n" followed by apply.txt content.
pub fn expand_config_elements(
    template: &str,
    element_source: &dyn ConfigElementSource,
) -> Result<String> {
    let re = Regex::new(r"^!!!###([a-zA-Z0-9_-]+)$").expect("valid regex");
    let mut output = String::new();

    for line in template.lines() {
        let trimmed = line.trim();
        if let Some(caps) = re.captures(trimmed) {
            let name = &caps[1];
            let apply_content = element_source.load_apply(name)?;
            output.push_str(&format!("! config-element: {}\n", name));
            output.push_str(&apply_content);
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use crate::fs_sources::FsConfigElementSource;

    struct MockElementSource {
        elements: HashMap<String, String>,
    }

    impl ConfigElementSource for MockElementSource {
        fn load_apply(&self, name: &str) -> Result<String> {
            self.elements.get(name).cloned()
                .ok_or_else(|| anyhow::anyhow!("element not found: {}", name))
        }
    }

    fn mock_source_with(name: &str, content: &str) -> MockElementSource {
        let mut elements = HashMap::new();
        elements.insert(name.to_string(), content.to_string());
        MockElementSource { elements }
    }

    #[test]
    fn test_expand_single_element() {
        let source = mock_source_with("test-element", "line1\nline2\n");
        let template = "before\n!!!###test-element\nafter\n";
        let result = expand_config_elements(template, &source).unwrap();
        assert_eq!(result, "before\n! config-element: test-element\nline1\nline2\nafter\n");
    }

    #[test]
    fn test_expand_no_elements() {
        let source = MockElementSource { elements: HashMap::new() };
        let template = "line1\nline2\nline3\n";
        let result = expand_config_elements(template, &source).unwrap();
        assert_eq!(result, template);
    }

    #[test]
    fn test_expand_unknown_element() {
        let source = MockElementSource { elements: HashMap::new() };
        let template = "!!!###nonexistent\n";
        let result = expand_config_elements(template, &source);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent"), "error should mention element name, got: {err}");
    }

    #[test]
    fn test_expand_element_preserves_comment() {
        let source = mock_source_with("my-element", "config content\n");
        let template = "!!!###my-element\n";
        let result = expand_config_elements(template, &source).unwrap();
        // The "! config-element: <name>" line must appear before the content
        let comment_pos = result.find("! config-element: my-element").unwrap();
        let content_pos = result.find("config content").unwrap();
        assert!(comment_pos < content_pos, "comment line must appear before element content");
    }

    #[test]
    fn test_expand_with_leading_whitespace() {
        let source = mock_source_with("test-element", "apply content\n");
        // Line has surrounding whitespace — trim should allow matching
        let template = "  !!!###test-element  \n";
        let result = expand_config_elements(template, &source).unwrap();
        assert!(result.contains("! config-element: test-element"));
        assert!(result.contains("apply content"));
    }

    #[test]
    fn test_expand_partial_line_not_matched() {
        let source = mock_source_with("test-element", "apply content\n");
        // Marker is not the whole line — must pass through unchanged
        let template = "some text !!!###test-element\n";
        let result = expand_config_elements(template, &source).unwrap();
        assert_eq!(result, template);
    }

    #[test]
    fn test_expand_set1_template() {
        let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples");
        let set1 = examples.join("set1");
        let template_content = std::fs::read_to_string(
            set1.join("config-templates/access-switch.conf")
        ).expect("read access-switch.conf");
        let element_source = FsConfigElementSource::new(set1.join("config-elements"));
        let result = expand_config_elements(&template_content, &element_source).unwrap();
        // The marker line should be gone
        assert!(!result.contains("!!!###logging-standard"), "marker should be replaced");
        // The comment line should be present
        assert!(result.contains("! config-element: logging-standard"), "comment line should appear");
        // The apply.txt content should be present
        assert!(result.contains("logging buffered"), "apply.txt content should appear");
        // Non-marker lines should be preserved
        assert!(result.contains("hostname switch1"), "template content should be preserved");
    }
}
