use anyhow::Result;
use indexmap::IndexMap;
use mustache::MapBuilder;

/// Expand Mustache variables in a template string using the given vars.
/// Supports both {{var}} (HTML-escaped) and {{{var}}} (unescaped) syntax.
/// Missing variables expand to empty string (Mustache default).
/// If template parsing fails, returns the original string with a warning to stderr.
pub fn expand_vars(template: &str, vars: &IndexMap<String, String>) -> Result<String> {
    let compiled = match mustache::compile_str(template) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("WARNING: failed to compile mustache template: {}", e);
            return Ok(template.to_string());
        }
    };

    let mut builder = MapBuilder::new();
    for (key, value) in vars {
        builder = builder.insert_str(key.clone(), value.clone());
    }
    let data = builder.build();

    let mut output = Vec::new();
    compiled
        .render_data(&mut output, &data)
        .map_err(|e| anyhow::anyhow!("mustache render error: {}", e))?;

    String::from_utf8(output).map_err(|e| anyhow::anyhow!("mustache output not UTF-8: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_triple_brace_expansion() {
        let result = expand_vars("hostname {{{hostname}}}", &vars(&[("hostname", "switch1")])).unwrap();
        assert_eq!(result, "hostname switch1");
    }

    #[test]
    fn test_double_brace_expansion() {
        // For normal (non-HTML) values, {{var}} and {{{var}}} are identical
        let result = expand_vars("hostname {{hostname}}", &vars(&[("hostname", "switch1")])).unwrap();
        assert_eq!(result, "hostname switch1");
    }

    #[test]
    fn test_multiple_variables() {
        let result = expand_vars(
            "hostname {{{hostname}}} location {{{location}}}",
            &vars(&[("hostname", "switch1"), ("location", "Room-A")]),
        )
        .unwrap();
        assert_eq!(result, "hostname switch1 location Room-A");
    }

    #[test]
    fn test_missing_variable_expands_to_empty() {
        let result = expand_vars("hostname {{{missing}}}", &vars(&[])).unwrap();
        assert_eq!(result, "hostname ");
    }

    #[test]
    fn test_no_variables_unchanged() {
        let input = "no vars here\njust plain text\n";
        let result = expand_vars(input, &vars(&[("hostname", "switch1")])).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_empty_vars_map_template_unchanged() {
        let input = "plain text with no mustache\n";
        let result = expand_vars(input, &vars(&[])).unwrap();
        assert_eq!(result, input);
    }
}
