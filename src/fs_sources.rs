use std::path::PathBuf;
use anyhow::{Context, Result};
use crate::model::{HardwareTemplate, LogicalDeviceConfig};
use crate::sources::{
    HardwareTemplateSource, LogicalDeviceSource, ServiceSource,
    ConfigTemplateSource, ConfigElementSource, SoftwareImageSource,
};

/// Strip trailing newline/carriage-return characters and append exactly one `\n`.
/// Preserves trailing spaces on content lines — only strips `\n` and `\r`.
fn normalize_trailing_newline(s: &str) -> String {
    let trimmed = s.trim_end_matches(|c: char| c == '\n' || c == '\r');
    let mut result = trimmed.to_string();
    result.push('\n');
    result
}

// ---------------------------------------------------------------------------
// FsHardwareTemplateSource
// ---------------------------------------------------------------------------

pub struct FsHardwareTemplateSource {
    pub dir: PathBuf,
}

impl FsHardwareTemplateSource {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl HardwareTemplateSource for FsHardwareTemplateSource {
    fn load_hardware_template(&self, sku: &str) -> Result<HardwareTemplate> {
        let path = self.dir.join(sku).join("ports.json");
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read hardware template for SKU {:?}: {}", sku, path.display()))?;
        let tmpl: HardwareTemplate = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse hardware template for SKU {:?}", sku))?;
        Ok(tmpl)
    }
}

// ---------------------------------------------------------------------------
// FsLogicalDeviceSource
// ---------------------------------------------------------------------------

pub struct FsLogicalDeviceSource {
    pub dir: PathBuf,
}

impl FsLogicalDeviceSource {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl LogicalDeviceSource for FsLogicalDeviceSource {
    fn load_device_config(&self, device_name: &str) -> Result<LogicalDeviceConfig> {
        let path = self.dir.join(device_name).join("config.json");
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read device config for {:?}: {}", device_name, path.display()))?;
        let cfg: LogicalDeviceConfig = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse device config for {:?}", device_name))?;
        Ok(cfg)
    }

    fn list_devices(&self) -> Result<Vec<String>> {
        let entries = std::fs::read_dir(&self.dir)
            .with_context(|| format!("failed to read logical devices directory: {}", self.dir.display()))?;
        let mut names: Vec<String> = Vec::new();
        for entry in entries {
            let entry = entry
                .with_context(|| format!("failed to read entry in {}", self.dir.display()))?;
            if entry.file_type()
                .with_context(|| format!("failed to get file type for {:?}", entry.path()))?
                .is_dir()
            {
                if let Ok(name) = entry.file_name().into_string() {
                    names.push(name);
                }
            }
        }
        names.sort();
        Ok(names)
    }
}

// ---------------------------------------------------------------------------
// FsServiceSource
// ---------------------------------------------------------------------------

pub struct FsServiceSource {
    pub dir: PathBuf,
}

impl FsServiceSource {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl ServiceSource for FsServiceSource {
    fn load_port_config(&self, service_name: &str) -> Result<String> {
        let path = self.dir.join(service_name).join("port-config.txt");
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read port-config.txt for service {:?}: {}", service_name, path.display()))?;
        Ok(normalize_trailing_newline(&data))
    }

    fn load_svi_config(&self, service_name: &str) -> Result<Option<String>> {
        let path = self.dir.join(service_name).join("svi-config.txt");
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read svi-config.txt for service {:?}: {}", service_name, path.display()))?;
        Ok(Some(normalize_trailing_newline(&data)))
    }
}

// ---------------------------------------------------------------------------
// FsConfigTemplateSource
// ---------------------------------------------------------------------------

pub struct FsConfigTemplateSource {
    pub dir: PathBuf,
}

impl FsConfigTemplateSource {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl ConfigTemplateSource for FsConfigTemplateSource {
    fn load_template(&self, template_name: &str) -> Result<String> {
        let path = self.dir.join(template_name);
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config template {:?}: {}", template_name, path.display()))?;
        Ok(normalize_trailing_newline(&data))
    }
}

// ---------------------------------------------------------------------------
// FsConfigElementSource
// ---------------------------------------------------------------------------

pub struct FsConfigElementSource {
    pub dir: PathBuf,
}

impl FsConfigElementSource {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl ConfigElementSource for FsConfigElementSource {
    fn load_apply(&self, element_name: &str) -> Result<String> {
        let path = self.dir.join(element_name).join("apply.txt");
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read apply.txt for element {:?}: {}", element_name, path.display()))?;
        Ok(normalize_trailing_newline(&data))
    }
}

// ---------------------------------------------------------------------------
// FsSoftwareImageSource
// ---------------------------------------------------------------------------

pub struct FsSoftwareImageSource {
    pub dir: PathBuf,
}

impl FsSoftwareImageSource {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl SoftwareImageSource for FsSoftwareImageSource {
    fn validate_exists(&self, image_name: &str) -> Result<()> {
        let path = self.dir.join(image_name);
        if path.exists() {
            Ok(())
        } else {
            anyhow::bail!(
                "software image {:?} not found at {}",
                image_name,
                path.display()
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn examples_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples")
    }

    fn set1_dir() -> PathBuf {
        examples_dir().join("set1")
    }

    fn set2_dir() -> PathBuf {
        examples_dir().join("set2")
    }

    // --- normalize_trailing_newline ---

    #[test]
    fn test_trailing_newline_normalization() {
        // No trailing newline: one is added
        assert_eq!(normalize_trailing_newline("hello"), "hello\n");

        // Single trailing newline: unchanged (effectively)
        assert_eq!(normalize_trailing_newline("hello\n"), "hello\n");

        // Multiple trailing newlines: collapsed to one
        assert_eq!(normalize_trailing_newline("hello\n\n\n"), "hello\n");

        // Trailing whitespace only (no newlines): preserved, newline added
        assert_eq!(normalize_trailing_newline("hello   "), "hello   \n");

        // Trailing whitespace then newlines: spaces on content line preserved
        assert_eq!(normalize_trailing_newline("hello   \n\n"), "hello   \n");

        // Multi-line content, last content line has trailing spaces
        assert_eq!(
            normalize_trailing_newline("line1\nline2   \n\n"),
            "line1\nline2   \n"
        );

        // Empty string: only a newline
        assert_eq!(normalize_trailing_newline(""), "\n");
    }

    // --- FsHardwareTemplateSource ---

    #[test]
    fn test_load_hardware_template_set1() {
        let source = FsHardwareTemplateSource::new(set1_dir().join("hardware-templates"));
        let tmpl = source.load_hardware_template("WS-C3560-24TS").expect("load hardware template");
        assert_eq!(tmpl.ports.len(), 4);
        let port0 = tmpl.ports.get("Port0").expect("Port0 exists");
        assert_eq!(port0.name, "GigabitEthernet");
        assert_eq!(port0.index, "0/0");
    }

    // --- FsLogicalDeviceSource ---

    #[test]
    fn test_load_device_config_set1() {
        let source = FsLogicalDeviceSource::new(set1_dir().join("logical-devices"));
        let cfg = source.load_device_config("switch1").expect("load device config");
        assert_eq!(cfg.config_template, "access-switch.conf");
        assert!(cfg.omit_slot_prefix);
    }

    #[test]
    fn test_list_devices_set1() {
        let source = FsLogicalDeviceSource::new(set1_dir().join("logical-devices"));
        let devices = source.list_devices().expect("list devices");
        assert_eq!(devices, vec!["switch1"]);
    }

    #[test]
    fn test_list_devices_set2() {
        let source = FsLogicalDeviceSource::new(set2_dir().join("logical-devices"));
        let devices = source.list_devices().expect("list devices");
        assert_eq!(devices, vec!["router1"]);
    }

    // --- FsServiceSource ---

    #[test]
    fn test_load_port_config() {
        let source = FsServiceSource::new(set1_dir().join("services"));
        let content = source.load_port_config("access-vlan10").expect("load port config");
        // Must end with exactly one \n
        assert!(content.ends_with('\n'), "content must end with newline");
        assert!(!content.ends_with("\n\n"), "content must not end with double newline");
    }

    #[test]
    fn test_load_svi_config_present() {
        let source = FsServiceSource::new(set1_dir().join("services"));
        let result = source.load_svi_config("access-vlan10").expect("load svi config");
        assert!(result.is_some(), "access-vlan10 has svi-config.txt");
        let content = result.unwrap();
        assert!(content.ends_with('\n'));
        assert!(!content.ends_with("\n\n"));
    }

    #[test]
    fn test_load_svi_config_absent() {
        let source = FsServiceSource::new(set1_dir().join("services"));
        let result = source.load_svi_config("trunk").expect("load svi config");
        assert!(result.is_none(), "trunk has no svi-config.txt");
    }

    // --- FsConfigTemplateSource ---

    #[test]
    fn test_load_config_template() {
        let source = FsConfigTemplateSource::new(set1_dir().join("config-templates"));
        let content = source.load_template("access-switch.conf").expect("load template");
        assert!(content.contains("hostname switch1"));
        assert!(content.ends_with('\n'));
        assert!(!content.ends_with("\n\n"));
    }

    // --- FsConfigElementSource ---

    #[test]
    fn test_load_config_element() {
        let source = FsConfigElementSource::new(set1_dir().join("config-elements"));
        let content = source.load_apply("logging-standard").expect("load element");
        assert!(content.contains("logging buffered"));
        assert!(content.ends_with('\n'));
        assert!(!content.ends_with("\n\n"));
    }

    // --- FsSoftwareImageSource ---

    #[test]
    fn test_validate_software_image_exists() {
        let tmp = std::env::temp_dir().join("aycfggen_test_sw_images");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let image_path = tmp.join("test-image.bin");
        std::fs::write(&image_path, b"fake image").expect("write test image");
        let source = FsSoftwareImageSource::new(tmp.clone());
        let result = source.validate_exists("test-image.bin");
        assert!(result.is_ok(), "existing image should return Ok");
        // cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_validate_software_image_missing() {
        let source = FsSoftwareImageSource::new(set1_dir().join("software-images"));
        let result = source.validate_exists("nonexistent-image.bin");
        assert!(result.is_err(), "missing image should return an error");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("nonexistent-image.bin"),
            "error message should contain image name, got: {msg}"
        );
    }
}
