use anyhow::Result;
use regex::Regex;
use crate::model::LogicalDeviceConfig;
use crate::sources::{
    ConfigElementSource, ConfigTemplateSource, HardwareTemplateSource,
    LogicalDeviceSource, ServiceSource, SoftwareImageSource,
};
use crate::validate::validate_device;
use crate::interface_name::{derive_interface_name, resolve_slot_index_base};

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

/// Build the port configuration block for a logical device.
///
/// Returns `(port_block_content, warnings)`.
/// The content does NOT include `! PORTS-START` / `! PORTS-END` markers.
pub fn build_port_block(
    config: &LogicalDeviceConfig,
    hw_source: &dyn HardwareTemplateSource,
    service_source: &dyn ServiceSource,
) -> Result<(String, Vec<String>)> {
    let mut output = String::new();
    let mut warnings = Vec::new();

    for (slot_position, module_opt) in config.modules.iter().enumerate() {
        let module = match module_opt {
            Some(m) => m,
            None => continue,
        };

        let hw_template = hw_source.load_hardware_template(&module.sku)?;
        let slot_index_base =
            resolve_slot_index_base(config.slot_index_base, hw_template.slot_index_base);

        if module.ports.is_empty() {
            warnings.push(format!(
                "module at slot {} (SKU: {}) has zero ports",
                slot_position, module.sku
            ));
            continue;
        }

        for port_assignment in &module.ports {
            let port_def = hw_template.ports.get(&port_assignment.name)
                .ok_or_else(|| anyhow::anyhow!(
                    "port {:?} not found in hardware template for SKU {:?}",
                    port_assignment.name, module.sku
                ))?;

            let iface_name = derive_interface_name(
                port_def,
                slot_position,
                slot_index_base,
                config.omit_slot_prefix,
            );

            let port_config = service_source.load_port_config(&port_assignment.service)?;

            // interface line
            output.push_str(&format!("interface {}\n", iface_name));

            // prologue lines (split on \n, skip trailing empty line)
            if let Some(prologue) = &port_assignment.prologue {
                for line in prologue.split('\n') {
                    if !line.is_empty() {
                        output.push_str(line);
                        output.push('\n');
                    }
                }
            }

            // port-config.txt content (already normalized to end with \n)
            output.push_str(&port_config);

            // epilogue lines (split on \n, skip trailing empty line)
            if let Some(epilogue) = &port_assignment.epilogue {
                for line in epilogue.split('\n') {
                    if !line.is_empty() {
                        output.push_str(line);
                        output.push('\n');
                    }
                }
            }
        }
    }

    Ok((output, warnings))
}

/// Build the SVI configuration block for a logical device.
///
/// Collects unique service names (first-occurrence order) across all ports,
/// then for each service that has an svi-config.txt, includes its content.
/// The content does NOT include `! SVI-START` / `! SVI-END` markers.
pub fn build_svi_block(
    config: &LogicalDeviceConfig,
    service_source: &dyn ServiceSource,
) -> Result<String> {
    // Collect unique service names in first-occurrence order
    let mut seen = std::collections::HashSet::new();
    let mut unique_services: Vec<String> = Vec::new();

    for module_opt in &config.modules {
        let module = match module_opt {
            Some(m) => m,
            None => continue,
        };
        for port_assignment in &module.ports {
            if seen.insert(port_assignment.service.clone()) {
                unique_services.push(port_assignment.service.clone());
            }
        }
    }

    let mut output = String::new();
    for service_name in &unique_services {
        if let Some(svi_content) = service_source.load_svi_config(service_name)? {
            output.push_str(&svi_content);
        }
    }

    Ok(output)
}

/// Assemble final configuration by substituting markers in the template.
///
/// `port_block` and `svi_block` are raw content without the surrounding marker lines.
/// Each marker must appear at most once — returns an error if duplicate markers are found.
///
/// Substitution rules:
/// - If `<PORTS-CONFIGURATION>` line is present, replace that entire line with the wrapped ports section.
/// - If `<SVI-CONFIGURATION>` line is present, replace that entire line with the wrapped SVI section.
/// - If a marker is absent, append the block at the end with a guidance comment.
///   When BOTH markers are absent, the SVI block is appended first, then the ports block.
///
/// Wrapped block format (non-empty):
///   `! PORTS-START\n` + port_block + `! PORTS-END\n`
///   `! SVI-START\n` + svi_block + `! SVI-END\n`
///
/// Wrapped block format (empty):
///   `! PORTS-START\n! PORTS-END\n`
///   `! SVI-START\n! SVI-END\n`
pub fn assemble_config(
    template: &str,
    port_block: &str,
    svi_block: &str,
) -> Result<String> {
    // Validate: each marker appears at most once.
    for marker in &["<PORTS-CONFIGURATION>", "<SVI-CONFIGURATION>"] {
        let count = template.matches(marker).count();
        if count > 1 {
            anyhow::bail!(
                "marker '{}' appears {} times in template (must appear at most once)",
                marker,
                count
            );
        }
    }

    // Build the wrapped sections.
    let ports_section = if port_block.is_empty() {
        "! PORTS-START\n! PORTS-END\n".to_string()
    } else {
        format!("! PORTS-START\n{}! PORTS-END\n", port_block)
    };

    let svi_section = if svi_block.is_empty() {
        "! SVI-START\n! SVI-END\n".to_string()
    } else {
        format!("! SVI-START\n{}! SVI-END\n", svi_block)
    };

    let has_ports_marker = template.contains("<PORTS-CONFIGURATION>");
    let has_svi_marker = template.contains("<SVI-CONFIGURATION>");

    // Replace marker lines in the template.
    let mut output = String::new();
    for line in template.lines() {
        if has_ports_marker && line.contains("<PORTS-CONFIGURATION>") {
            output.push_str(&ports_section);
        } else if has_svi_marker && line.contains("<SVI-CONFIGURATION>") {
            output.push_str(&svi_section);
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    // Append missing sections at the end.
    if !has_svi_marker && !has_ports_marker {
        // Both missing: SVI first, then ports.
        output.push_str("! use <SVI-CONFIGURATION> marker to place this configuration block\n");
        output.push_str(&svi_section);
        output.push_str("! use <PORTS-CONFIGURATION> marker to place this configuration\n");
        output.push_str(&ports_section);
    } else if !has_svi_marker {
        output.push_str("! use <SVI-CONFIGURATION> marker to place this configuration block\n");
        output.push_str(&svi_section);
    } else if !has_ports_marker {
        output.push_str("! use <PORTS-CONFIGURATION> marker to place this configuration\n");
        output.push_str(&ports_section);
    }

    Ok(output)
}

/// Compile a single device configuration end-to-end.
///
/// Steps:
/// 1. Load device config from `device_source`.
/// 2. Validate the config (hard errors propagated; warnings printed to stderr).
/// 3. Load the config template.
/// 4. Expand config elements in the template.
/// 5. Build the port block.
/// 6. Build the SVI block.
/// 7. Assemble the final configuration.
/// 8. Return the assembled string.
pub fn compile_device(
    device_name: &str,
    device_source: &dyn LogicalDeviceSource,
    hw_source: &dyn HardwareTemplateSource,
    service_source: &dyn ServiceSource,
    template_source: &dyn ConfigTemplateSource,
    element_source: &dyn ConfigElementSource,
    image_source: &dyn SoftwareImageSource,
) -> Result<String> {
    // Step 1: Load device config.
    let config = device_source.load_device_config(device_name)?;

    // Step 2: Validate.
    let warnings = validate_device(
        device_name,
        &config,
        hw_source,
        service_source,
        template_source,
        image_source,
    )?;
    for w in &warnings {
        eprintln!("WARNING [{}]: {}", device_name, w);
    }

    // Step 3: Load config template.
    let raw_template = template_source.load_template(&config.config_template)?;

    // Step 4: Expand config elements.
    let expanded_template = expand_config_elements(&raw_template, element_source)?;

    // Step 5: Build port block.
    let (port_block, port_warnings) = build_port_block(&config, hw_source, service_source)?;
    for w in &port_warnings {
        eprintln!("WARNING [{}]: {}", device_name, w);
    }

    // Step 6: Build SVI block.
    let svi_block = build_svi_block(&config, service_source)?;

    // Step 7: Assemble.
    let result = assemble_config(&expanded_template, &port_block, &svi_block)?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use crate::fs_sources::{FsConfigElementSource, FsHardwareTemplateSource, FsServiceSource};
    use crate::model::{HardwareTemplate, PortDefinition, LogicalDeviceConfig, Module, PortAssignment};
    use indexmap::IndexMap;

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

    // -------------------------------------------------------------------------
    // Mock sources for port/SVI block tests
    // -------------------------------------------------------------------------

    struct MockHwSource {
        templates: HashMap<String, HardwareTemplate>,
    }

    impl HardwareTemplateSource for MockHwSource {
        fn load_hardware_template(&self, sku: &str) -> Result<HardwareTemplate> {
            self.templates.get(sku).cloned()
                .ok_or_else(|| anyhow::anyhow!("SKU not found: {}", sku))
        }
    }

    struct MockServiceSource {
        port_configs: HashMap<String, String>,
        svi_configs: HashMap<String, String>,
    }

    impl ServiceSource for MockServiceSource {
        fn load_port_config(&self, name: &str) -> Result<String> {
            self.port_configs.get(name).cloned()
                .ok_or_else(|| anyhow::anyhow!("service not found: {}", name))
        }
        fn load_svi_config(&self, name: &str) -> Result<Option<String>> {
            Ok(self.svi_configs.get(name).cloned())
        }
    }

    fn examples_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples")
    }

    /// Extract the lines between two marker lines (exclusive of the markers themselves).
    fn extract_between_markers<'a>(text: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
        let start = text.find(start_marker)
            .map(|i| i + start_marker.len())
            .unwrap_or(0);
        // skip the newline after start_marker
        let start = if text[start..].starts_with('\n') { start + 1 } else { start };
        let end = text[start..].find(end_marker)
            .map(|i| start + i)
            .unwrap_or(text.len());
        &text[start..end]
    }

    // -------------------------------------------------------------------------
    // Phase 8: Port block tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_port_block_set1() {
        let set1 = examples_dir().join("set1");
        let hw_source = FsHardwareTemplateSource::new(set1.join("hardware-templates"));
        let service_source = FsServiceSource::new(set1.join("services"));

        let config_json = std::fs::read_to_string(
            set1.join("logical-devices/switch1/config.json")
        ).expect("read config.json");
        let config: LogicalDeviceConfig = serde_json::from_str(&config_json).expect("parse config");

        let (port_block, warnings) = build_port_block(&config, &hw_source, &service_source)
            .expect("build_port_block");

        assert!(warnings.is_empty(), "no warnings expected: {:?}", warnings);

        // All 4 interfaces must be present
        assert!(port_block.contains("interface GigabitEthernet0/0\n"), "missing GigabitEthernet0/0");
        assert!(port_block.contains("interface GigabitEthernet0/1\n"), "missing GigabitEthernet0/1");
        assert!(port_block.contains("interface GigabitEthernet0/2\n"), "missing GigabitEthernet0/2");
        assert!(port_block.contains("interface GigabitEthernet0/3\n"), "missing GigabitEthernet0/3");

        // Prologue on port0
        assert!(port_block.contains("description Workstation Port\n"), "missing prologue line 1");
        assert!(port_block.contains("spanning-tree portfast\n"), "missing prologue line 2");

        // Epilogue on port3
        assert!(port_block.contains("no cdp enable\n"), "missing epilogue");

        // Compare against the expected port section from switch1.txt
        let expected_output = std::fs::read_to_string(
            set1.join("expected-output/switch1.txt")
        ).expect("read switch1.txt");
        let expected_port_section = extract_between_markers(
            &expected_output, "! PORTS-START", "! PORTS-END"
        );
        assert_eq!(port_block, expected_port_section,
            "port block does not match expected output");
    }

    #[test]
    fn test_build_port_block_set2() {
        let set2 = examples_dir().join("set2");
        let hw_source = FsHardwareTemplateSource::new(set2.join("hardware-templates"));
        let service_source = FsServiceSource::new(set2.join("services"));

        let config_json = std::fs::read_to_string(
            set2.join("logical-devices/router1/config.json")
        ).expect("read config.json");
        let config: LogicalDeviceConfig = serde_json::from_str(&config_json).expect("parse config");

        let (port_block, warnings) = build_port_block(&config, &hw_source, &service_source)
            .expect("build_port_block");

        assert!(warnings.is_empty(), "no warnings expected: {:?}", warnings);

        // NIM-4GE at slot 1 → GigabitEthernet1/0/0 through 1/0/3
        assert!(port_block.contains("interface GigabitEthernet1/0/0\n"), "missing GigabitEthernet1/0/0");
        assert!(port_block.contains("interface GigabitEthernet1/0/1\n"), "missing GigabitEthernet1/0/1");
        assert!(port_block.contains("interface GigabitEthernet1/0/2\n"), "missing GigabitEthernet1/0/2");
        assert!(port_block.contains("interface GigabitEthernet1/0/3\n"), "missing GigabitEthernet1/0/3");

        // NIM-2FXS at slot 2 → FastEthernet2/0/0 through 2/0/1
        assert!(port_block.contains("interface FastEthernet2/0/0\n"), "missing FastEthernet2/0/0");
        assert!(port_block.contains("interface FastEthernet2/0/1\n"), "missing FastEthernet2/0/1");

        // Compare against the expected port section from router1.txt
        let expected_output = std::fs::read_to_string(
            set2.join("expected-output/router1.txt")
        ).expect("read router1.txt");
        let expected_port_section = extract_between_markers(
            &expected_output, "! PORTS-START", "! PORTS-END"
        );
        assert_eq!(port_block, expected_port_section,
            "port block does not match expected output");
    }

    #[test]
    fn test_build_port_block_empty_modules() {
        let config = LogicalDeviceConfig {
            config_template: "test.conf".to_string(),
            software_image: None,
            role: None,
            vendor: None,
            omit_slot_prefix: false,
            slot_index_base: None,
            vars: IndexMap::new(),
            modules: vec![],
        };
        let hw_source = MockHwSource { templates: HashMap::new() };
        let service_source = MockServiceSource {
            port_configs: HashMap::new(),
            svi_configs: HashMap::new(),
        };
        let (port_block, warnings) = build_port_block(&config, &hw_source, &service_source)
            .expect("build_port_block");
        assert_eq!(port_block, "", "empty modules should produce empty string");
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_build_port_block_zero_ports_warning() {
        // Build a hardware template with no ports
        let hw_tmpl = HardwareTemplate {
            vendor: None,
            slot_index_base: None,
            ports: IndexMap::new(),
        };
        let mut templates = HashMap::new();
        templates.insert("EMPTY-SKU".to_string(), hw_tmpl);

        let module = Module {
            sku: "EMPTY-SKU".to_string(),
            serial: None,
            ports: vec![],
        };
        let config = LogicalDeviceConfig {
            config_template: "test.conf".to_string(),
            software_image: None,
            role: None,
            vendor: None,
            omit_slot_prefix: false,
            slot_index_base: None,
            vars: IndexMap::new(),
            modules: vec![Some(module)],
        };
        let hw_source = MockHwSource { templates };
        let service_source = MockServiceSource {
            port_configs: HashMap::new(),
            svi_configs: HashMap::new(),
        };
        let (port_block, warnings) = build_port_block(&config, &hw_source, &service_source)
            .expect("build_port_block");
        assert_eq!(port_block, "", "zero-port module should produce empty output");
        assert!(!warnings.is_empty(), "should have at least one warning");
        assert!(warnings[0].contains("zero ports") || warnings[0].contains("EMPTY-SKU"),
            "warning should mention zero ports or SKU: {}", warnings[0]);
    }

    // -------------------------------------------------------------------------
    // Phase 9: SVI block tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_svi_block_set1() {
        let set1 = examples_dir().join("set1");
        let service_source = FsServiceSource::new(set1.join("services"));

        let config_json = std::fs::read_to_string(
            set1.join("logical-devices/switch1/config.json")
        ).expect("read config.json");
        let config: LogicalDeviceConfig = serde_json::from_str(&config_json).expect("parse config");

        let svi_block = build_svi_block(&config, &service_source).expect("build_svi_block");

        // access-vlan10 has SVI → must be present
        assert!(svi_block.contains("Vlan10"), "access-vlan10 SVI should be included");
        // trunk has no SVI → no trunk-specific content
        // (trunk has no svi-config.txt so nothing trunk-specific would be in the block)

        // Compare against expected SVI section from switch1.txt
        let expected_output = std::fs::read_to_string(
            set1.join("expected-output/switch1.txt")
        ).expect("read switch1.txt");
        let expected_svi_section = extract_between_markers(
            &expected_output, "! SVI-START", "! SVI-END"
        );
        assert_eq!(svi_block, expected_svi_section,
            "SVI block does not match expected output");
    }

    #[test]
    fn test_build_svi_block_set2() {
        let set2 = examples_dir().join("set2");
        let service_source = FsServiceSource::new(set2.join("services"));

        let config_json = std::fs::read_to_string(
            set2.join("logical-devices/router1/config.json")
        ).expect("read config.json");
        let config: LogicalDeviceConfig = serde_json::from_str(&config_json).expect("parse config");

        let svi_block = build_svi_block(&config, &service_source).expect("build_svi_block");

        // wan-link SVI content
        assert!(svi_block.contains("Loopback0"), "wan-link SVI should be included");
        // voice SVI content
        assert!(svi_block.contains("Vlan100"), "voice SVI should be included");
        assert!(svi_block.contains("Vlan200"), "voice SVI should be included");

        // wan-link appears first (Port0 on slot 1 is first encountered)
        let loopback_pos = svi_block.find("Loopback0").expect("Loopback0 in output");
        let vlan100_pos = svi_block.find("Vlan100").expect("Vlan100 in output");
        assert!(loopback_pos < vlan100_pos, "wan-link SVI should appear before voice SVI");

        // Compare against expected SVI section from router1.txt
        let expected_output = std::fs::read_to_string(
            set2.join("expected-output/router1.txt")
        ).expect("read router1.txt");
        let expected_svi_section = extract_between_markers(
            &expected_output, "! SVI-START", "! SVI-END"
        );
        assert_eq!(svi_block, expected_svi_section,
            "SVI block does not match expected output");
    }

    #[test]
    fn test_build_svi_block_dedup() {
        // Same service on multiple ports → SVI should appear only once
        let port0 = PortAssignment {
            name: "Port0".to_string(),
            service: "my-service".to_string(),
            prologue: None,
            epilogue: None,
            vars: IndexMap::new(),
        };
        let port1 = PortAssignment {
            name: "Port1".to_string(),
            service: "my-service".to_string(),
            prologue: None,
            epilogue: None,
            vars: IndexMap::new(),
        };
        let hw_tmpl = HardwareTemplate {
            vendor: None,
            slot_index_base: None,
            ports: {
                let mut m = IndexMap::new();
                m.insert("Port0".to_string(), PortDefinition { name: "Eth".to_string(), index: "0".to_string() });
                m.insert("Port1".to_string(), PortDefinition { name: "Eth".to_string(), index: "1".to_string() });
                m
            },
        };
        let _ = hw_tmpl; // not needed for SVI block

        let module = Module {
            sku: "TEST-SKU".to_string(),
            serial: None,
            ports: vec![port0, port1],
        };
        let config = LogicalDeviceConfig {
            config_template: "test.conf".to_string(),
            software_image: None,
            role: None,
            vendor: None,
            omit_slot_prefix: false,
            slot_index_base: None,
            vars: IndexMap::new(),
            modules: vec![Some(module)],
        };

        let mut svi_configs = HashMap::new();
        svi_configs.insert("my-service".to_string(), "interface Vlan999\n no shutdown\n".to_string());
        let service_source = MockServiceSource {
            port_configs: HashMap::new(),
            svi_configs,
        };

        let svi_block = build_svi_block(&config, &service_source).expect("build_svi_block");

        // SVI content should appear exactly once
        let count = svi_block.matches("interface Vlan999").count();
        assert_eq!(count, 1, "SVI should appear exactly once, got {} occurrences", count);
    }

    #[test]
    fn test_build_svi_block_no_svis() {
        // Services exist but none have svi-config.txt
        let port0 = PortAssignment {
            name: "Port0".to_string(),
            service: "no-svi-service".to_string(),
            prologue: None,
            epilogue: None,
            vars: IndexMap::new(),
        };
        let module = Module {
            sku: "TEST-SKU".to_string(),
            serial: None,
            ports: vec![port0],
        };
        let config = LogicalDeviceConfig {
            config_template: "test.conf".to_string(),
            software_image: None,
            role: None,
            vendor: None,
            omit_slot_prefix: false,
            slot_index_base: None,
            vars: IndexMap::new(),
            modules: vec![Some(module)],
        };

        // MockServiceSource returns None for all svi_configs (empty map)
        let service_source = MockServiceSource {
            port_configs: HashMap::new(),
            svi_configs: HashMap::new(),
        };

        let svi_block = build_svi_block(&config, &service_source).expect("build_svi_block");
        assert_eq!(svi_block, "", "no SVIs should produce empty string");
    }

    // -------------------------------------------------------------------------
    // Phase 10: Template Assembly tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_assemble_both_markers_present() {
        let template = "header\n<PORTS-CONFIGURATION>\n<SVI-CONFIGURATION>\nfooter\n";
        let port_block = "interface Eth0\n no shutdown\n";
        let svi_block = "interface Vlan10\n ip address 1.2.3.4/24\n";
        let result = assemble_config(template, port_block, svi_block).unwrap();
        assert!(result.contains("! PORTS-START\n"), "should contain PORTS-START marker");
        assert!(result.contains("interface Eth0\n"), "should contain port content");
        assert!(result.contains("! PORTS-END\n"), "should contain PORTS-END marker");
        assert!(result.contains("! SVI-START\n"), "should contain SVI-START marker");
        assert!(result.contains("interface Vlan10\n"), "should contain SVI content");
        assert!(result.contains("! SVI-END\n"), "should contain SVI-END marker");
        assert!(result.contains("header\n"), "should preserve header");
        assert!(result.contains("footer\n"), "should preserve footer");
        // The marker lines themselves should not appear literally
        assert!(!result.contains("<PORTS-CONFIGURATION>"), "marker line should be replaced");
        assert!(!result.contains("<SVI-CONFIGURATION>"), "marker line should be replaced");
    }

    #[test]
    fn test_assemble_empty_port_block() {
        let template = "before\n<PORTS-CONFIGURATION>\nafter\n";
        let result = assemble_config(template, "", "").unwrap();
        assert!(result.contains("! PORTS-START\n! PORTS-END\n"),
            "empty port block should emit only marker lines, got:\n{}", result);
    }

    #[test]
    fn test_assemble_empty_svi_block() {
        let template = "before\n<SVI-CONFIGURATION>\nafter\n";
        let result = assemble_config(template, "", "").unwrap();
        assert!(result.contains("! SVI-START\n! SVI-END\n"),
            "empty SVI block should emit only marker lines, got:\n{}", result);
    }

    #[test]
    fn test_assemble_missing_ports_marker() {
        // Only SVI marker present — ports must be appended at the end with a comment.
        let template = "header\n<SVI-CONFIGURATION>\nfooter\n";
        let port_block = "interface Eth0\n";
        let result = assemble_config(template, port_block, "svi content\n").unwrap();
        // SVI section should be in-place
        assert!(result.contains("! SVI-START\n"), "SVI section should be present");
        // Ports guidance comment and section must appear at the end
        assert!(result.contains("! use <PORTS-CONFIGURATION> marker to place this configuration\n"),
            "missing ports marker should emit guidance comment, got:\n{}", result);
        assert!(result.contains("! PORTS-START\n"), "PORTS-START should appear in appended block");
        // The appended block must come after the template body
        let footer_pos = result.find("footer").expect("footer in output");
        let ports_comment_pos = result.find("! use <PORTS-CONFIGURATION>").expect("ports comment in output");
        assert!(ports_comment_pos > footer_pos, "ports block must be after template body");
        // SVI marker in template should not appear literally
        assert!(!result.contains("<SVI-CONFIGURATION>"), "SVI marker line should be replaced");
    }

    #[test]
    fn test_assemble_missing_svi_marker() {
        // Only ports marker present — SVI must be appended at the end with a comment.
        let template = "header\n<PORTS-CONFIGURATION>\nfooter\n";
        let svi_block = "interface Vlan10\n";
        let result = assemble_config(template, "eth content\n", svi_block).unwrap();
        // Ports section should be in-place
        assert!(result.contains("! PORTS-START\n"), "PORTS section should be present");
        // SVI guidance comment and section must appear at the end
        assert!(result.contains("! use <SVI-CONFIGURATION> marker to place this configuration block\n"),
            "missing SVI marker should emit guidance comment, got:\n{}", result);
        assert!(result.contains("! SVI-START\n"), "SVI-START should appear in appended block");
        let footer_pos = result.find("footer").expect("footer in output");
        let svi_comment_pos = result.find("! use <SVI-CONFIGURATION>").expect("svi comment in output");
        assert!(svi_comment_pos > footer_pos, "SVI block must be after template body");
        assert!(!result.contains("<PORTS-CONFIGURATION>"), "PORTS marker line should be replaced");
    }

    #[test]
    fn test_assemble_both_markers_missing() {
        let template = "header\nfooter\n";
        let result = assemble_config(template, "port content\n", "svi content\n").unwrap();
        // SVI should appear before ports
        let svi_comment_pos = result.find("! use <SVI-CONFIGURATION>").expect("SVI comment not found");
        let ports_comment_pos = result.find("! use <PORTS-CONFIGURATION>").expect("PORTS comment not found");
        assert!(svi_comment_pos < ports_comment_pos,
            "SVI block must appear before ports block when both markers are missing");
        // Both sections should be present
        assert!(result.contains("! SVI-START\n"));
        assert!(result.contains("! PORTS-START\n"));
    }

    #[test]
    fn test_assemble_duplicate_marker() {
        let template = "header\n<PORTS-CONFIGURATION>\nmiddle\n<PORTS-CONFIGURATION>\nfooter\n";
        let result = assemble_config(template, "port content\n", "");
        assert!(result.is_err(), "duplicate marker should return an error");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("PORTS-CONFIGURATION"), "error should mention the duplicate marker: {}", err);
    }

    // -------------------------------------------------------------------------
    // Phase 11: Integration tests
    // -------------------------------------------------------------------------

    fn make_fs_sources(example_dir: &std::path::Path) -> (
        crate::fs_sources::FsLogicalDeviceSource,
        crate::fs_sources::FsHardwareTemplateSource,
        crate::fs_sources::FsServiceSource,
        crate::fs_sources::FsConfigTemplateSource,
        FsConfigElementSource,
        crate::fs_sources::FsSoftwareImageSource,
    ) {
        use crate::fs_sources::{
            FsConfigTemplateSource, FsHardwareTemplateSource, FsLogicalDeviceSource,
            FsServiceSource, FsSoftwareImageSource,
        };
        (
            FsLogicalDeviceSource::new(example_dir.join("logical-devices")),
            FsHardwareTemplateSource::new(example_dir.join("hardware-templates")),
            FsServiceSource::new(example_dir.join("services")),
            FsConfigTemplateSource::new(example_dir.join("config-templates")),
            FsConfigElementSource::new(example_dir.join("config-elements")),
            FsSoftwareImageSource::new(example_dir.join("software-images")),
        )
    }

    #[test]
    fn test_compile_device_set1() {
        let example_dir = examples_dir().join("set1");
        let (device_src, hw_src, svc_src, tmpl_src, elem_src, img_src) =
            make_fs_sources(&example_dir);

        let result = compile_device(
            "switch1",
            &device_src,
            &hw_src,
            &svc_src,
            &tmpl_src,
            &elem_src,
            &img_src,
        ).expect("compile_device set1 should succeed");

        let expected = std::fs::read_to_string(
            example_dir.join("expected-output/switch1.txt")
        ).expect("read switch1.txt");

        if result != expected {
            // Show first differing line for diagnosis
            for (i, (got, exp)) in result.lines().zip(expected.lines()).enumerate() {
                if got != exp {
                    panic!(
                        "set1 output differs at line {}:\n  got: {:?}\n  exp: {:?}\n\nFull got:\n{}\n\nFull expected:\n{}",
                        i + 1, got, exp, result, expected
                    );
                }
            }
            let got_lines = result.lines().count();
            let exp_lines = expected.lines().count();
            panic!(
                "set1 output differs (got {} lines, expected {} lines)\n\nFull got:\n{}\n\nFull expected:\n{}",
                got_lines, exp_lines, result, expected
            );
        }
    }

    #[test]
    fn test_compile_device_set2() {
        let example_dir = examples_dir().join("set2");
        let (device_src, hw_src, svc_src, tmpl_src, elem_src, img_src) =
            make_fs_sources(&example_dir);

        let result = compile_device(
            "router1",
            &device_src,
            &hw_src,
            &svc_src,
            &tmpl_src,
            &elem_src,
            &img_src,
        ).expect("compile_device set2 should succeed");

        let expected = std::fs::read_to_string(
            example_dir.join("expected-output/router1.txt")
        ).expect("read router1.txt");

        if result != expected {
            for (i, (got, exp)) in result.lines().zip(expected.lines()).enumerate() {
                if got != exp {
                    panic!(
                        "set2 output differs at line {}:\n  got: {:?}\n  exp: {:?}\n\nFull got:\n{}\n\nFull expected:\n{}",
                        i + 1, got, exp, result, expected
                    );
                }
            }
            let got_lines = result.lines().count();
            let exp_lines = expected.lines().count();
            panic!(
                "set2 output differs (got {} lines, expected {} lines)\n\nFull got:\n{}\n\nFull expected:\n{}",
                got_lines, exp_lines, result, expected
            );
        }
    }
}
