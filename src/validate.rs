use anyhow::{Context, Result};
use std::collections::HashSet;

use regex::Regex;
use std::sync::LazyLock;
use crate::model::LogicalDeviceConfig;
use crate::sources::{ConfigElementSource, ConfigTemplateSource, HardwareTemplateSource, ServiceSource, SoftwareImageSource};

/// Check that each marker appears at most once in the template.
pub fn validate_template_markers(template: &str) -> Result<()> {
    let markers = ["<PORTS-CONFIGURATION>", "<SVI-CONFIGURATION>"];
    for marker in &markers {
        let count = template.lines().filter(|line| line.trim() == *marker).count();
        if count > 1 {
            anyhow::bail!(
                "marker '{}' appears {} times in template (must appear at most once)",
                marker,
                count
            );
        }
    }
    Ok(())
}

/// Validate a single device configuration against all always-on rules.
///
/// Returns a list of warning messages (non-fatal). On any hard error, returns `Err`.
pub fn validate_device(
    device_name: &str,
    config: &LogicalDeviceConfig,
    hw_source: &dyn HardwareTemplateSource,
    service_source: &dyn ServiceSource,
    template_source: &dyn ConfigTemplateSource,
    element_source: &dyn ConfigElementSource,
    image_source: &dyn SoftwareImageSource,
) -> Result<Vec<String>> {
    let mut warnings: Vec<String> = Vec::new();

    // --- omit-slot-prefix constraint ---
    if config.omit_slot_prefix {
        let non_null_count = config.modules.iter().filter(|m| m.is_some()).count();
        if config.modules.len() != 1 {
            anyhow::bail!(
                "device '{}': omit-slot-prefix is true but modules has {} elements (must have exactly 1)",
                device_name,
                config.modules.len()
            );
        }
        if non_null_count == 0 {
            anyhow::bail!(
                "device '{}': omit-slot-prefix is true but the single module is null",
                device_name
            );
        }
    }

    // --- Per-module validations ---
    for (slot_idx, module_opt) in config.modules.iter().enumerate() {
        let module = match module_opt {
            Some(m) => m,
            None => continue,
        };

        // Warning: zero ports
        if module.ports.is_empty() {
            warnings.push(format!(
                "device '{}': module at slot {} (SKU '{}') has zero ports",
                device_name, slot_idx, module.sku
            ));
            continue;
        }

        // Load hardware template for this module
        let hw_template = hw_source
            .load_hardware_template(&module.sku)
            .with_context(|| {
                format!(
                    "device '{}': failed to load hardware template for SKU '{}'",
                    device_name, module.sku
                )
            })?;

        // Check for duplicate port names within this module
        let mut seen_ports: HashSet<&str> = HashSet::new();
        for port_assignment in &module.ports {
            if !seen_ports.insert(port_assignment.name.as_str()) {
                anyhow::bail!(
                    "device '{}': duplicate port assignment '{}' in module at slot {} (SKU '{}')",
                    device_name,
                    port_assignment.name,
                    slot_idx,
                    module.sku
                );
            }
        }

        // Check each port assignment
        for port_assignment in &module.ports {
            // Port name must exist in hardware template
            if !hw_template.ports.contains_key(&port_assignment.name) {
                anyhow::bail!(
                    "device '{}': port '{}' not found in hardware template for SKU '{}'",
                    device_name,
                    port_assignment.name,
                    module.sku
                );
            }

            // Service must have a port-config.txt (try loading it)
            service_source
                .load_port_config(&port_assignment.service)
                .with_context(|| {
                    format!(
                        "device '{}': service '{}' does not have a valid port-config.txt",
                        device_name, port_assignment.service
                    )
                })?;
        }
    }

    // --- Config template must exist ---
    let template_content = template_source
        .load_template(&config.config_template)
        .with_context(|| {
            format!(
                "device '{}': config template '{}' does not exist",
                device_name, config.config_template
            )
        })?;

    // --- Markers must appear at most once ---
    validate_template_markers(&template_content).with_context(|| {
        format!(
            "device '{}': template '{}' has duplicate markers",
            device_name, config.config_template
        )
    })?;

    // --- Config element references must exist ---
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(crate::CONFIG_ELEMENT_MARKER_PATTERN).expect("valid regex")
    });
    for line in template_content.lines() {
        let trimmed = line.trim();
        if let Some(caps) = RE.captures(trimmed) {
            let element_name = &caps[1];
            element_source
                .load_apply(element_name)
                .with_context(|| {
                    format!(
                        "device '{}': config element '{}' referenced in template '{}' does not exist",
                        device_name, element_name, config.config_template
                    )
                })?;
        }
    }

    // --- Software image must exist if specified ---
    if let Some(image) = &config.software_image {
        image_source
            .validate_exists(image)
            .with_context(|| {
                format!(
                    "device '{}': software image '{}' does not exist",
                    device_name, image
                )
            })?;
    }

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HardwareTemplate, LogicalDeviceConfig, Module, PortAssignment, PortDefinition};
    use crate::sources::{ConfigElementSource, ConfigTemplateSource, HardwareTemplateSource, ServiceSource, SoftwareImageSource};
    use anyhow::Result;
    use indexmap::IndexMap;
    use std::collections::HashMap;
    use std::path::PathBuf;

    // --- Mock sources ---

    struct MockHwSource {
        templates: HashMap<String, HardwareTemplate>,
    }

    impl MockHwSource {
        fn new() -> Self {
            Self {
                templates: HashMap::new(),
            }
        }

        fn with_template(mut self, sku: &str, ports: Vec<(&str, &str, &str)>) -> Self {
            // ports: (port_key, name, index)
            let mut port_map = IndexMap::new();
            for (key, name, index) in ports {
                port_map.insert(
                    key.to_string(),
                    PortDefinition {
                        name: name.to_string(),
                        index: index.to_string(),
                    },
                );
            }
            self.templates.insert(
                sku.to_string(),
                HardwareTemplate {
                    vendor: None,
                    slot_index_base: None,
                    ports: port_map,
                },
            );
            self
        }
    }

    impl HardwareTemplateSource for MockHwSource {
        fn load_hardware_template(&self, sku: &str) -> Result<HardwareTemplate> {
            self.templates
                .get(sku)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("SKU not found: {}", sku))
        }
    }

    struct MockServiceSource {
        /// Set of service names that have a port-config.txt
        services: HashSet<String>,
    }

    impl MockServiceSource {
        fn new(services: Vec<&str>) -> Self {
            Self {
                services: services.into_iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl ServiceSource for MockServiceSource {
        fn load_port_config(&self, service_name: &str) -> Result<String> {
            if self.services.contains(service_name) {
                Ok(format!("! port config for {}\n", service_name))
            } else {
                anyhow::bail!("service '{}' not found", service_name)
            }
        }

        fn load_svi_config(&self, _service_name: &str) -> Result<Option<String>> {
            Ok(None)
        }
    }

    struct MockTemplateSource {
        /// Map from template name to content; if absent, load_template returns an error
        templates: HashMap<String, String>,
    }

    impl MockTemplateSource {
        fn new(templates: Vec<(&str, &str)>) -> Self {
            Self {
                templates: templates
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            }
        }
    }

    impl ConfigTemplateSource for MockTemplateSource {
        fn load_template(&self, template_name: &str) -> Result<String> {
            self.templates
                .get(template_name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("template '{}' not found", template_name))
        }
    }

    struct MockImageSource {
        /// Set of image names that exist
        images: HashSet<String>,
    }

    impl MockImageSource {
        fn new(images: Vec<&str>) -> Self {
            Self {
                images: images.into_iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl SoftwareImageSource for MockImageSource {
        fn validate_exists(&self, image_name: &str) -> Result<()> {
            if self.images.contains(image_name) {
                Ok(())
            } else {
                anyhow::bail!("software image '{}' not found", image_name)
            }
        }
    }

    struct MockElementSource {
        elements: HashSet<String>,
    }

    impl MockElementSource {
        fn new(elements: Vec<&str>) -> Self {
            Self {
                elements: elements.into_iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl ConfigElementSource for MockElementSource {
        fn load_apply(&self, element_name: &str) -> Result<String> {
            if self.elements.contains(element_name) {
                Ok(format!("! element {}\n", element_name))
            } else {
                anyhow::bail!("config element '{}' not found", element_name)
            }
        }
    }

    // --- Helper builders ---

    fn simple_template() -> &'static str {
        "hostname test\n<PORTS-CONFIGURATION>\n<SVI-CONFIGURATION>\n"
    }

    fn make_port_assignment(name: &str, service: &str) -> PortAssignment {
        PortAssignment {
            name: name.to_string(),
            service: service.to_string(),
            prologue: None,
            epilogue: None,
            vars: IndexMap::new(),
        }
    }

    fn make_module(sku: &str, ports: Vec<PortAssignment>) -> Module {
        Module {
            sku: sku.to_string(),
            serial: None,
            ports,
        }
    }

    fn make_config(
        template: &str,
        omit_slot_prefix: bool,
        modules: Vec<Option<Module>>,
    ) -> LogicalDeviceConfig {
        LogicalDeviceConfig {
            config_template: template.to_string(),
            software_image: None,
            role: None,
            vendor: None,
            omit_slot_prefix,
            slot_index_base: None,
            vars: IndexMap::new(),
            modules,
        }
    }

    fn example_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples")
    }

    // --- Tests ---

    #[test]
    fn test_valid_set1_config_passes() {
        // Build a config mirroring set1/switch1
        let hw = MockHwSource::new().with_template(
            "WS-C3560-24TS",
            vec![
                ("Port0", "GigabitEthernet", "0/0"),
                ("Port1", "GigabitEthernet", "0/1"),
                ("Port2", "GigabitEthernet", "0/2"),
                ("Port3", "GigabitEthernet", "0/3"),
            ],
        );
        let svc = MockServiceSource::new(vec!["access-vlan10", "trunk"]);
        let tmpl = MockTemplateSource::new(vec![("access-switch.conf", simple_template())]);
        let img = MockImageSource::new(vec![]);

        let ports = vec![
            make_port_assignment("Port0", "access-vlan10"),
            make_port_assignment("Port1", "access-vlan10"),
            make_port_assignment("Port2", "access-vlan10"),
            make_port_assignment("Port3", "trunk"),
        ];
        let module = make_module("WS-C3560-24TS", ports);
        let config = make_config("access-switch.conf", true, vec![Some(module)]);

        let elem = MockElementSource::new(vec![]);
        let result = validate_device("switch1", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_ok(), "valid set1 config should pass: {:?}", result.err());
        let warnings = result.unwrap();
        assert!(warnings.is_empty(), "no warnings expected");
    }

    #[test]
    fn test_omit_slot_prefix_with_two_modules() {
        let hw = MockHwSource::new().with_template(
            "SKU1",
            vec![("Port0", "GigabitEthernet", "0/0")],
        );
        let svc = MockServiceSource::new(vec!["svc"]);
        let tmpl = MockTemplateSource::new(vec![("tmpl.conf", simple_template())]);
        let img = MockImageSource::new(vec![]);

        let m1 = make_module("SKU1", vec![make_port_assignment("Port0", "svc")]);
        let m2 = make_module("SKU1", vec![make_port_assignment("Port0", "svc")]);
        let config = make_config("tmpl.conf", true, vec![Some(m1), Some(m2)]);

        let elem = MockElementSource::new(vec![]);
        let result = validate_device("dev", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_err(), "should error with two modules and omit-slot-prefix");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("omit-slot-prefix"), "error message should mention omit-slot-prefix: {}", msg);
    }

    #[test]
    fn test_omit_slot_prefix_with_null_module() {
        let hw = MockHwSource::new();
        let svc = MockServiceSource::new(vec![]);
        let tmpl = MockTemplateSource::new(vec![("tmpl.conf", simple_template())]);
        let img = MockImageSource::new(vec![]);

        // [null] with omit_slot_prefix=true → error
        let config = make_config("tmpl.conf", true, vec![None]);

        let elem = MockElementSource::new(vec![]);
        let result = validate_device("dev", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_err(), "should error with null module and omit-slot-prefix");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("null"), "error message should mention null: {}", msg);
    }

    #[test]
    fn test_duplicate_port_in_module() {
        let hw = MockHwSource::new().with_template(
            "SKU1",
            vec![("Port0", "GigabitEthernet", "0/0")],
        );
        let svc = MockServiceSource::new(vec!["svc"]);
        let tmpl = MockTemplateSource::new(vec![("tmpl.conf", simple_template())]);
        let img = MockImageSource::new(vec![]);

        let ports = vec![
            make_port_assignment("Port0", "svc"),
            make_port_assignment("Port0", "svc"), // duplicate
        ];
        let module = make_module("SKU1", ports);
        let config = make_config("tmpl.conf", false, vec![Some(module)]);

        let elem = MockElementSource::new(vec![]);
        let result = validate_device("dev", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_err(), "should error on duplicate port");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("duplicate"), "error should mention duplicate: {}", msg);
    }

    #[test]
    fn test_missing_port_in_hardware_template() {
        let hw = MockHwSource::new().with_template(
            "SKU1",
            vec![("Port0", "GigabitEthernet", "0/0")],
        );
        let svc = MockServiceSource::new(vec!["svc"]);
        let tmpl = MockTemplateSource::new(vec![("tmpl.conf", simple_template())]);
        let img = MockImageSource::new(vec![]);

        let ports = vec![make_port_assignment("Port99", "svc")]; // Port99 not in template
        let module = make_module("SKU1", ports);
        let config = make_config("tmpl.conf", false, vec![Some(module)]);

        let elem = MockElementSource::new(vec![]);
        let result = validate_device("dev", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_err(), "should error on missing port in hw template");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Port99"), "error should mention the missing port: {}", msg);
    }

    #[test]
    fn test_missing_service() {
        let hw = MockHwSource::new().with_template(
            "SKU1",
            vec![("Port0", "GigabitEthernet", "0/0")],
        );
        let svc = MockServiceSource::new(vec![]); // no services
        let tmpl = MockTemplateSource::new(vec![("tmpl.conf", simple_template())]);
        let img = MockImageSource::new(vec![]);

        let ports = vec![make_port_assignment("Port0", "nonexistent")];
        let module = make_module("SKU1", ports);
        let config = make_config("tmpl.conf", false, vec![Some(module)]);

        let elem = MockElementSource::new(vec![]);
        let result = validate_device("dev", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_err(), "should error on missing service");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("nonexistent") || msg.contains("port-config"),
            "error should identify the missing service: {}",
            msg
        );
    }

    #[test]
    fn test_missing_config_template() {
        let hw = MockHwSource::new().with_template(
            "SKU1",
            vec![("Port0", "GigabitEthernet", "0/0")],
        );
        let svc = MockServiceSource::new(vec!["svc"]);
        let tmpl = MockTemplateSource::new(vec![]); // no templates
        let img = MockImageSource::new(vec![]);

        let ports = vec![make_port_assignment("Port0", "svc")];
        let module = make_module("SKU1", ports);
        let config = make_config("missing.conf", false, vec![Some(module)]);

        let elem = MockElementSource::new(vec![]);
        let result = validate_device("dev", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_err(), "should error on missing config template");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("missing.conf") || msg.contains("template"),
            "error should mention the template: {}",
            msg
        );
    }

    #[test]
    fn test_duplicate_marker_in_template() {
        let double_marker = "hostname test\n<PORTS-CONFIGURATION>\n<PORTS-CONFIGURATION>\n";
        let result = validate_template_markers(double_marker);
        assert!(result.is_err(), "should error on duplicate marker");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("PORTS-CONFIGURATION"),
            "error should mention the marker: {}",
            msg
        );
    }

    #[test]
    fn test_zero_port_module_warning() {
        // A module with empty ports vec should produce a warning, not an error
        let hw = MockHwSource::new().with_template("SKU1", vec![]);
        let svc = MockServiceSource::new(vec![]);
        let tmpl = MockTemplateSource::new(vec![("tmpl.conf", simple_template())]);
        let img = MockImageSource::new(vec![]);

        let module = make_module("SKU1", vec![]); // zero ports
        let config = make_config("tmpl.conf", false, vec![Some(module)]);

        let elem = MockElementSource::new(vec![]);
        let result = validate_device("dev", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_ok(), "zero-port module should not be an error: {:?}", result.err());
        let warnings = result.unwrap();
        assert!(!warnings.is_empty(), "should have a warning for zero-port module");
        assert!(
            warnings[0].contains("zero ports"),
            "warning should mention zero ports: {}",
            warnings[0]
        );
    }

    #[test]
    fn test_missing_software_image() {
        let hw = MockHwSource::new().with_template(
            "SKU1",
            vec![("Port0", "GigabitEthernet", "0/0")],
        );
        let svc = MockServiceSource::new(vec!["svc"]);
        let tmpl = MockTemplateSource::new(vec![("tmpl.conf", simple_template())]);
        let img = MockImageSource::new(vec![]); // no images

        let ports = vec![make_port_assignment("Port0", "svc")];
        let module = make_module("SKU1", ports);
        let mut config = make_config("tmpl.conf", false, vec![Some(module)]);
        config.software_image = Some("firmware.bin".to_string());

        let elem = MockElementSource::new(vec![]);
        let result = validate_device("dev", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_err(), "should error on missing software image");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("firmware.bin") || msg.contains("software image"),
            "error should mention the image: {}",
            msg
        );
    }

    /// Integration-style test: validate set1 switch1 using filesystem sources
    #[test]
    fn test_valid_set1_config_passes_fs() {
        use crate::fs_sources::{
            FsConfigElementSource, FsConfigTemplateSource, FsHardwareTemplateSource,
            FsServiceSource, FsSoftwareImageSource,
        };

        let base = example_dir().join("set1");
        let hw = FsHardwareTemplateSource::new(base.join("hardware-templates"));
        let svc = FsServiceSource::new(base.join("services"));
        let tmpl = FsConfigTemplateSource::new(base.join("config-templates"));
        let elem = FsConfigElementSource::new(base.join("config-elements"));
        let img = FsSoftwareImageSource::new(base.join("software-images"));

        let config_path = base.join("logical-devices/switch1/config.json");
        let data = std::fs::read_to_string(&config_path).expect("read config.json");
        let config: LogicalDeviceConfig =
            serde_json::from_str(&data).expect("deserialize config");

        let result = validate_device("switch1", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_ok(), "set1 switch1 should pass validation: {:?}", result.err());
    }

    /// Integration-style test: validate set2 router1 using filesystem sources
    #[test]
    fn test_valid_set2_config_passes_fs() {
        use crate::fs_sources::{
            FsConfigElementSource, FsConfigTemplateSource, FsHardwareTemplateSource,
            FsServiceSource, FsSoftwareImageSource,
        };

        let base = example_dir().join("set2");
        let hw = FsHardwareTemplateSource::new(base.join("hardware-templates"));
        let svc = FsServiceSource::new(base.join("services"));
        let tmpl = FsConfigTemplateSource::new(base.join("config-templates"));
        let elem = FsConfigElementSource::new(base.join("config-elements"));
        let img = FsSoftwareImageSource::new(base.join("software-images"));

        let config_path = base.join("logical-devices/router1/config.json");
        let data = std::fs::read_to_string(&config_path).expect("read config.json");
        let config: LogicalDeviceConfig =
            serde_json::from_str(&data).expect("deserialize config");

        let result = validate_device("router1", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_ok(), "set2 router1 should pass validation: {:?}", result.err());
    }

    #[test]
    fn test_missing_config_element() {
        let hw = MockHwSource::new().with_template(
            "SKU1",
            vec![("Port0", "GigabitEthernet", "0/0")],
        );
        let svc = MockServiceSource::new(vec!["svc"]);
        // Template references a config element
        let tmpl = MockTemplateSource::new(vec![
            ("tmpl.conf", "hostname test\n!!!###nonexistent-element\n<PORTS-CONFIGURATION>\n<SVI-CONFIGURATION>\n"),
        ]);
        let elem = MockElementSource::new(vec![]); // no elements
        let img = MockImageSource::new(vec![]);

        let ports = vec![make_port_assignment("Port0", "svc")];
        let module = make_module("SKU1", ports);
        let config = make_config("tmpl.conf", false, vec![Some(module)]);

        let result = validate_device("dev", &config, &hw, &svc, &tmpl, &elem, &img);
        assert!(result.is_err(), "should error on missing config element");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("nonexistent-element"),
            "error should mention the element name: {}",
            msg
        );
    }
}
