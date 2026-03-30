use std::path::Path;
use anyhow::{Context, Result};

/// Write compiled config to a file, creating the output directory if needed.
pub fn write_config(configs_dir: &Path, device_name: &str, content: &str) -> Result<()> {
    std::fs::create_dir_all(configs_dir)
        .with_context(|| format!("failed to create output directory: {}", configs_dir.display()))?;
    let path = configs_dir.join(format!("{}.txt", device_name));
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write config for device '{}': {}", device_name, path.display()))?;
    Ok(())
}

/// Simple Mustache-style banner interpolation for --preview.
/// Replaces {{device-name}}, {{role}}, {{config-template}} with actual values.
pub fn interpolate_banner(
    banner: &str,
    device_name: &str,
    role: Option<&str>,
    config_template: &str,
) -> String {
    banner
        .replace("{{device-name}}", device_name)
        .replace("{{role}}", role.unwrap_or(""))
        .replace("{{config-template}}", config_template)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_config_creates_dir_and_file() {
        let tmp = std::env::temp_dir().join("aycfggen_test_output");
        let _ = std::fs::remove_dir_all(&tmp); // clean up from previous runs
        write_config(&tmp, "testdev", "config content\n").unwrap();
        let path = tmp.join("testdev.txt");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "config content\n");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_interpolate_banner_all_vars() {
        let result = interpolate_banner(
            "=== {{device-name}} ({{role}}) [{{config-template}}] ===",
            "switch1", Some("access"), "access-switch.conf"
        );
        assert_eq!(result, "=== switch1 (access) [access-switch.conf] ===");
    }

    #[test]
    fn test_interpolate_banner_no_role() {
        let result = interpolate_banner(
            "=== {{device-name}} ({{role}}) ===",
            "router1", None, "router.conf"
        );
        assert_eq!(result, "=== router1 () ===");
    }

    #[test]
    fn test_interpolate_banner_no_vars() {
        let result = interpolate_banner("--- separator ---", "dev", Some("role"), "tmpl");
        assert_eq!(result, "--- separator ---");
    }
}
