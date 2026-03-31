/// Extraction orchestrator: ties all pipeline stages together.
///
/// Runs the full extraction pipeline on parsed command output and returns
/// all artifacts needed to write to disk.

use std::collections::HashMap;
use anyhow::{anyhow, Result};
use indexmap::IndexMap;

use crate::hardware_discovery::{discover_hardware, DiscoveredDevice};
use crate::ios_parser::{parse_running_config, ConfigBlock};
use crate::model::{LogicalDeviceConfig, Module, PortAssignment};
use crate::port_decomposition::{decompose_ports, DerivedService};
use crate::show_parsers::{parse_show_inventory, parse_show_ip_interface_brief, parse_show_version};
use crate::svi_extraction::{extract_svis, SviAssignment};
use crate::template_builder::{build_template, GlobalSection, NewConfigElement};
use crate::variable_extraction::DefaultExtractor;

// ─── ExtractionOutput ─────────────────────────────────────────────────────────

/// All artifacts produced by the extraction pipeline for a single device.
pub struct ExtractionOutput {
    /// Stage 1 result: discovered device metadata and modules.
    pub device: DiscoveredDevice,
    /// Stage 1 result: per-SKU hardware templates (unique SKUs only).
    pub hardware_templates: HashMap<String, crate::model::HardwareTemplate>,
    /// Stage 2 result: derived services (new or matched).
    pub services: Vec<DerivedService>,
    /// Stage 3 result: SVI assignments to services.
    pub svi_assignments: Vec<SviAssignment>,
    /// Stage 4 result: template file name (`<hostname>-<serial>.conf`).
    pub template_name: String,
    /// Stage 4 result: template file content.
    pub template_content: String,
    /// Stage 4 result: new config elements that should be created.
    pub new_elements: Vec<NewConfigElement>,
    /// The assembled LogicalDeviceConfig ready to write.
    pub device_config: LogicalDeviceConfig,
}

// ─── extract_device ───────────────────────────────────────────────────────────

/// Run the full extraction pipeline on parsed command output.
///
/// # Parameters
///
/// - `show_version_output` — output of `show version`
/// - `show_inventory_output` — output of `show inventory`
/// - `show_ip_brief_output` — output of `show ip interface brief`
/// - `show_running_config` — output of `show running-config`
/// - `existing_services` — service_name → port-config.txt content (from data store)
/// - `existing_elements` — element_name → apply.txt content (from data store)
pub fn extract_device(
    show_version_output: &str,
    show_inventory_output: &str,
    show_ip_brief_output: &str,
    show_running_config: &str,
    existing_services: &HashMap<String, String>,
    existing_elements: &HashMap<String, String>,
) -> Result<ExtractionOutput> {
    // ── Stage 0: Parse show commands ──────────────────────────────────────────
    let version = parse_show_version(show_version_output)
        .ok_or_else(|| anyhow!("failed to parse show version output"))?;
    let inventory = parse_show_inventory(show_inventory_output);
    let interfaces = parse_show_ip_interface_brief(show_ip_brief_output);

    // ── Stage 1: Hardware discovery ───────────────────────────────────────────
    let device = discover_hardware(&version, &inventory, &interfaces)?;

    // Collect unique hardware templates by SKU
    let mut hardware_templates: HashMap<String, crate::model::HardwareTemplate> = HashMap::new();
    for module in &device.modules {
        hardware_templates
            .entry(module.sku.clone())
            .or_insert_with(|| module.hardware_template.clone());
    }

    // Build interface_name → port_id map from Stage 1 results.
    // For each module, derive the full interface name for each port using the hardware template.
    let port_id_map = build_port_id_map(&device);

    // ── Stage 0b: Parse running config ────────────────────────────────────────
    let config_blocks = parse_running_config(show_running_config);

    // Separate blocks into categories
    let mut port_blocks: Vec<(String, Vec<String>)> = Vec::new();
    let mut svi_blocks: Vec<(String, u16, Vec<String>)> = Vec::new();
    let mut global_sections: Vec<GlobalSection> = Vec::new();

    let mut first_port_seen = false;
    let mut first_svi_seen = false;

    // We accumulate global config lines until a non-global block is encountered.
    let mut pending_global: Vec<String> = Vec::new();

    let flush_pending = |pending: &mut Vec<String>, sections: &mut Vec<GlobalSection>| {
        if !pending.is_empty() {
            sections.push(GlobalSection::Config(pending.drain(..).collect()));
        }
    };

    for block in config_blocks {
        match block {
            ConfigBlock::PhysicalPort { name, lines } | ConfigBlock::SubInterface { name, lines } => {
                if !first_port_seen {
                    flush_pending(&mut pending_global, &mut global_sections);
                    global_sections.push(GlobalSection::PortsMarker);
                    first_port_seen = true;
                }
                port_blocks.push((name, lines));
            }
            ConfigBlock::Svi { name, vlan, lines } => {
                if !first_svi_seen {
                    flush_pending(&mut pending_global, &mut global_sections);
                    global_sections.push(GlobalSection::SviMarker);
                    first_svi_seen = true;
                }
                svi_blocks.push((name, vlan, lines));
            }
            ConfigBlock::VirtualInterface { name, lines } => {
                flush_pending(&mut pending_global, &mut global_sections);
                global_sections.push(GlobalSection::VirtualInterface(name, lines));
            }
            ConfigBlock::GlobalConfig { lines } => {
                pending_global.extend(lines);
            }
            ConfigBlock::MultiLineConstruct { keyword, content } => {
                flush_pending(&mut pending_global, &mut global_sections);
                global_sections.push(GlobalSection::MultiLine(keyword, content));
            }
        }
    }
    // Flush any remaining pending global config
    flush_pending(&mut pending_global, &mut global_sections);

    // ── Stage 2: Port decomposition ───────────────────────────────────────────
    let decomp = decompose_ports(&port_blocks, existing_services, &port_id_map);
    // Destructure decomp early so we can move services and ports independently.
    let (decomp_services, decomp_ports) = (decomp.services, decomp.ports);

    // ── Stage 3: SVI extraction ───────────────────────────────────────────────
    // Build service_vlans: include BOTH new services and existing services that were matched.
    // We need the existing services' VLAN info too, or SVIs won't be assigned to reused services.
    let mut all_service_configs: Vec<(&str, &str)> = Vec::new();
    for svc in &decomp_services {
        all_service_configs.push((&svc.name, &svc.port_config));
    }
    // Add existing services that were actually used by ports
    let used_existing: std::collections::HashSet<&str> = decomp_ports.iter()
        .map(|p| p.service_name.as_str())
        .filter(|name| !decomp_services.iter().any(|s| s.name == *name))
        .collect();
    for name in &used_existing {
        if let Some(content) = existing_services.get(*name) {
            all_service_configs.push((name, content));
        }
    }

    let service_vlans = build_service_vlans_from_pairs(&all_service_configs);

    // Service creation order: new services first (in decomp order), then existing services
    let mut service_creation_order: Vec<String> = decomp_services.iter().map(|s| s.name.clone()).collect();
    for name in &used_existing {
        service_creation_order.push(name.to_string());
    }

    let svi_result = extract_svis(&svi_blocks, &service_vlans, &service_creation_order);

    // Add unmatched SVIs back as literal text into global sections,
    // inserted immediately after the SVI marker to preserve original ordering.
    if !svi_result.unmatched.is_empty() {
        let svi_marker_pos = global_sections.iter().position(|s| matches!(s, GlobalSection::SviMarker));
        let insert_pos = svi_marker_pos.map(|p| p + 1).unwrap_or(global_sections.len());

        let mut offset = 0;
        for unmatched in &svi_result.unmatched {
            let lines: Vec<String> = unmatched.literal_text.lines().map(|l| l.to_string()).collect();
            if !lines.is_empty() {
                global_sections.insert(insert_pos + offset, GlobalSection::Config(lines));
                offset += 1;
            }
        }
    }

    // ── Stage 4: Template builder ──────────────────────────────────────────────
    let template_result = build_template(&global_sections, existing_elements, None, None);

    let template_name = format!("{}-{}.conf", device.hostname, device.serial_number);

    // ── Stage 5: Variable extraction ─────────────────────────────────────────
    use crate::variable_extraction::{ExtractionArtifacts, ServiceArtifact, VariableExtractor};
    let extractor = DefaultExtractor;
    let artifacts = extractor.extract(ExtractionArtifacts {
        template_content: template_result.template_content.clone(),
        services: decomp_services.iter().map(|s| ServiceArtifact {
            name: s.name.clone(),
            port_config: s.port_config.clone(),
            svi_config: svi_result.assignments.iter()
                .find(|a| a.service_name == s.name)
                .map(|a| a.svi_config.clone()),
            vars: HashMap::new(),
        }).collect(),
        device_vars: HashMap::new(),
    });

    // Collect device vars (e.g., hostname) for LogicalDeviceConfig.
    let device_vars: IndexMap<String, String> = artifacts.device_vars.into_iter().collect();

    // Build a map from service name to per-service vars (e.g., vlan_id).
    // These will be applied to port assignments for ports using each service.
    let service_vars_map: HashMap<String, HashMap<String, String>> = artifacts
        .services
        .iter()
        .filter(|s| !s.vars.is_empty())
        .map(|s| (s.name.clone(), s.vars.clone()))
        .collect();

    // ── Build LogicalDeviceConfig ──────────────────────────────────────────────
    // Note: service files written to disk use the original (unparameterized) port_config
    // from decomp_services, to ensure existing service matching works correctly on
    // subsequent extraction passes. The per-service vars (e.g., vlan_id) are stored in
    // PortAssignment.vars instead, where they can be used for variable expansion.
    let device_config = build_device_config(
        &device,
        &decomp_ports,
        &template_name,
        &device_vars,
        &service_vars_map,
    );

    Ok(ExtractionOutput {
        device,
        hardware_templates,
        services: decomp_services,
        svi_assignments: svi_result.assignments,
        template_name,
        template_content: artifacts.template_content,
        new_elements: template_result.new_elements,
        device_config,
    })
}

// ─── Helper: build interface_name → port_id map ────────────────────────────────

/// Build a map from full IOS interface name → port identifier (`Port0`, `Port1`, etc.)
/// using the discovered hardware profiles and slot configuration.
fn build_port_id_map(device: &DiscoveredDevice) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();

    for module in &device.modules {
        for (port_id, port_def) in &module.hardware_template.ports {
            // Reconstruct the full interface name
            let iface_name = if device.omit_slot_prefix {
                // Single-module, no slot prefix: e.g., GigabitEthernet0/0
                format!("{}{}", port_def.name, port_def.index)
            } else {
                // Multi-module: e.g., GigabitEthernet1/0/0
                format!("{}{}/{}", port_def.name, module.slot, port_def.index)
            };
            map.insert(iface_name, port_id.clone());
        }
    }

    map
}

// ─── Helper: build service → VLANs map ────────────────────────────────────────

/// Extract VLANs from port-config content for a list of (name, content) pairs.
fn build_service_vlans_from_pairs(services: &[(&str, &str)]) -> HashMap<String, Vec<u16>> {
    let mut result: HashMap<String, Vec<u16>> = HashMap::new();

    for &(name, content) in services {
        let mut vlans: Vec<u16> = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            // Access port VLAN
            if let Some(rest) = trimmed.strip_prefix("switchport access vlan ") {
                if let Ok(v) = rest.trim().parse::<u16>() {
                    if !vlans.contains(&v) {
                        vlans.push(v);
                    }
                }
            }
            // Trunk allowed VLANs (also matches "switchport trunk allowed vlan add ...")
            if let Some(rest) = trimmed.strip_prefix("switchport trunk allowed vlan ") {
                // Strip optional "add " prefix
                let vlan_list = rest.strip_prefix("add ").unwrap_or(rest);
                for part in vlan_list.trim().split(',') {
                    let part = part.trim();
                    if let Some((start, end)) = part.split_once('-') {
                        if let (Ok(s), Ok(e)) = (start.parse::<u16>(), end.parse::<u16>()) {
                            for v in s..=e {
                                if !vlans.contains(&v) {
                                    vlans.push(v);
                                }
                            }
                        }
                    } else if let Ok(v) = part.parse::<u16>() {
                        if !vlans.contains(&v) {
                            vlans.push(v);
                        }
                    }
                }
            }
            // Native VLAN on trunk
            if let Some(rest) = trimmed.strip_prefix("switchport trunk native vlan ") {
                if let Ok(v) = rest.trim().parse::<u16>() {
                    if !vlans.contains(&v) {
                        vlans.push(v);
                    }
                }
            }
        }
        if !vlans.is_empty() {
            result.insert(name.to_string(), vlans);
        }
    }

    result
}

// ─── Helper: assemble LogicalDeviceConfig ─────────────────────────────────────

fn build_device_config(
    device: &DiscoveredDevice,
    ports: &[crate::port_decomposition::DecomposedPort],
    template_name: &str,
    device_vars: &IndexMap<String, String>,
    service_vars_map: &HashMap<String, HashMap<String, String>>,
) -> LogicalDeviceConfig {
    // Build a map from interface_name to DecomposedPort for lookup.
    // Interface names are unique across all modules, unlike port_ids which repeat.
    let iface_map: HashMap<&str, &crate::port_decomposition::DecomposedPort> =
        ports.iter().map(|p| (p.interface_name.as_str(), p)).collect();

    // Determine the slot range to decide how many module slots to emit.
    let max_slot = device.modules.iter().map(|m| m.slot).max().unwrap_or(0);
    let min_slot = device.slot_index_base;

    // Build the modules array; gaps between slot_index_base and max_slot become None.
    let modules: Vec<Option<Module>> = (min_slot..=max_slot)
        .map(|slot| {
            let discovered = device.modules.iter().find(|m| m.slot == slot)?;
            // Collect port assignments for this module by reconstructing the interface name
            // for each port and looking it up in the decomposed ports.
            let port_assignments: Vec<PortAssignment> = discovered
                .hardware_template
                .ports
                .iter()
                .filter_map(|(port_id, port_def)| {
                    // Reconstruct the interface name for this port in this module
                    let iface_name = if device.omit_slot_prefix {
                        format!("{}{}", port_def.name, port_def.index)
                    } else {
                        format!("{}{}/{}", port_def.name, slot, port_def.index)
                    };
                    let dp = iface_map.get(iface_name.as_str())?;
                    // Populate port vars from the service vars map (e.g., vlan_id from VlanIdExtractor)
                    let port_vars: IndexMap<String, String> = service_vars_map
                        .get(&dp.service_name)
                        .map(|svars| svars.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default();
                    Some(PortAssignment {
                        name: port_id.clone(),
                        service: dp.service_name.clone(),
                        prologue: dp.prologue.clone(),
                        epilogue: dp.epilogue.clone(),
                        vars: port_vars,
                    })
                })
                .collect();

            Some(Module {
                sku: discovered.sku.clone(),
                serial: if discovered.serial.is_empty() { None } else { Some(discovered.serial.clone()) },
                ports: port_assignments,
            })
        })
        .collect();

    let software_image = if device.software_image.is_empty() {
        None
    } else {
        Some(device.software_image.clone())
    };

    LogicalDeviceConfig {
        config_template: template_name.to_string(),
        software_image,
        role: Some("discovered".to_string()),
        vendor: None,
        omit_slot_prefix: device.omit_slot_prefix,
        slot_index_base: if device.slot_index_base == 0 && device.omit_slot_prefix {
            None
        } else {
            Some(device.slot_index_base)
        },
        vars: device_vars.clone(),
        modules,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal 2-port switch fixture
    const SHOW_VERSION: &str = r#"
switch1 uptime is 1 day, 2 hours, 3 minutes
System image file is "flash:c3560-ipbasek9-mz.150-2.SE11.bin"
Model number                    : WS-C3560-2TS
System serial number            : FOC9999TEST
"#;

    const SHOW_INVENTORY: &str = r#"
NAME: "1", DESCR: "WS-C3560-2TS chassis"
PID: WS-C3560-2TS     , VID: V02 , SN: FOC9999TEST
"#;

    const SHOW_IP_BRIEF: &str = r#"
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet0/0     unassigned      YES unset  up                    up
GigabitEthernet0/1     unassigned      YES unset  down                  down
"#;

    const SHOW_RUNNING_CONFIG: &str = r#"
hostname switch1
!
logging buffered 16384
!
interface GigabitEthernet0/0
 switchport mode access
 switchport access vlan 10
!
interface GigabitEthernet0/1
 switchport mode access
 switchport access vlan 10
!
interface Vlan10
 ip address 10.0.0.1 255.255.255.0
!
end
"#;

    #[test]
    fn test_extract_device_simple_2port_switch() {
        let existing_services: HashMap<String, String> = HashMap::new();
        let existing_elements: HashMap<String, String> = HashMap::new();

        let output = extract_device(
            SHOW_VERSION,
            SHOW_INVENTORY,
            SHOW_IP_BRIEF,
            SHOW_RUNNING_CONFIG,
            &existing_services,
            &existing_elements,
        ).expect("extraction should succeed");

        // Verify device metadata
        assert_eq!(output.device.hostname, "switch1");
        assert_eq!(output.device.serial_number, "FOC9999TEST");
        assert_eq!(output.device.software_image, "c3560-ipbasek9-mz.150-2.SE11.bin");
        assert!(output.device.omit_slot_prefix, "single-module device should omit slot prefix");

        // Verify hardware template
        assert!(output.hardware_templates.contains_key("WS-C3560-2TS"),
            "hardware template for WS-C3560-2TS should be present");
        let tmpl = &output.hardware_templates["WS-C3560-2TS"];
        assert_eq!(tmpl.ports.len(), 2, "should have 2 ports");

        // Verify services
        assert_eq!(output.services.len(), 1, "both ports map to same service");
        assert_eq!(output.services[0].name, "access-vlan10");

        // Verify SVI assignments
        assert_eq!(output.svi_assignments.len(), 1);
        assert_eq!(output.svi_assignments[0].service_name, "access-vlan10");
        assert_eq!(output.svi_assignments[0].vlan, 10);

        // Verify template name format
        assert_eq!(output.template_name, "switch1-FOC9999TEST.conf");

        // Verify template content contains key elements
        // The DefaultExtractor replaces the literal hostname with a {{{hostname}}} placeholder.
        assert!(output.template_content.contains("hostname {{{hostname}}}"),
            "template should contain parameterized hostname placeholder");
        assert!(output.template_content.contains("<PORTS-CONFIGURATION>"),
            "template should contain PORTS marker");
        assert!(output.template_content.contains("<SVI-CONFIGURATION>"),
            "template should contain SVI marker");

        // Verify device config
        assert_eq!(output.device_config.config_template, "switch1-FOC9999TEST.conf");
        assert_eq!(output.device_config.role.as_deref(), Some("discovered"));
        assert_eq!(output.device_config.software_image.as_deref(),
            Some("c3560-ipbasek9-mz.150-2.SE11.bin"));
        assert!(output.device_config.omit_slot_prefix);
        assert_eq!(output.device_config.modules.len(), 1);

        let module = output.device_config.modules[0].as_ref().expect("module should be present");
        assert_eq!(module.sku, "WS-C3560-2TS");
        assert_eq!(module.ports.len(), 2, "module should have 2 port assignments");
        for port in &module.ports {
            assert_eq!(port.service, "access-vlan10");
        }
    }

    #[test]
    fn test_template_name_format() {
        let existing_services: HashMap<String, String> = HashMap::new();
        let existing_elements: HashMap<String, String> = HashMap::new();

        let output = extract_device(
            SHOW_VERSION,
            SHOW_INVENTORY,
            SHOW_IP_BRIEF,
            SHOW_RUNNING_CONFIG,
            &existing_services,
            &existing_elements,
        ).expect("extraction should succeed");

        // Template name must be <hostname>-<serial>.conf
        assert_eq!(output.template_name, "switch1-FOC9999TEST.conf");
        assert!(output.template_name.ends_with(".conf"));
        assert!(output.template_name.contains('-'));
        let parts: Vec<&str> = output.template_name.splitn(2, '-').collect();
        assert_eq!(parts[0], "switch1", "first part should be hostname");
        assert!(parts[1].starts_with("FOC9999TEST"), "second part should start with serial");
    }

    #[test]
    fn test_extract_device_uses_existing_services() {
        let mut existing_services: HashMap<String, String> = HashMap::new();
        existing_services.insert(
            "my-custom-service".to_string(),
            "switchport mode access\nswitchport access vlan 10\n".to_string(),
        );
        let existing_elements: HashMap<String, String> = HashMap::new();

        let output = extract_device(
            SHOW_VERSION,
            SHOW_INVENTORY,
            SHOW_IP_BRIEF,
            SHOW_RUNNING_CONFIG,
            &existing_services,
            &existing_elements,
        ).expect("extraction should succeed");

        // The existing service matches our ports, so no new services should be created
        assert!(output.services.is_empty(),
            "no new services should be created when existing service matches");

        // Port assignments should reference the existing service
        let module = output.device_config.modules[0].as_ref().expect("module");
        for port in &module.ports {
            assert_eq!(port.service, "my-custom-service",
                "port should use existing matching service");
        }
    }

    #[test]
    fn test_extract_device_invalid_show_version_fails() {
        let existing_services: HashMap<String, String> = HashMap::new();
        let existing_elements: HashMap<String, String> = HashMap::new();

        let result = extract_device(
            "this is not valid show version output",
            SHOW_INVENTORY,
            SHOW_IP_BRIEF,
            SHOW_RUNNING_CONFIG,
            &existing_services,
            &existing_elements,
        );

        assert!(result.is_err(), "should fail with unparseable show version");
    }

    #[test]
    fn test_extract_device_vars_flow_into_device_config() {
        let existing_services: HashMap<String, String> = HashMap::new();
        let existing_elements: HashMap<String, String> = HashMap::new();

        let output = extract_device(
            SHOW_VERSION,
            SHOW_INVENTORY,
            SHOW_IP_BRIEF,
            SHOW_RUNNING_CONFIG,
            &existing_services,
            &existing_elements,
        ).expect("extraction should succeed");

        // The DefaultExtractor should have extracted `hostname` into device vars
        assert_eq!(
            output.device_config.vars.get("hostname"),
            Some(&"switch1".to_string()),
            "hostname should be in device_config.vars"
        );

        // The template should have the {{{hostname}}} placeholder
        assert!(
            output.template_content.contains("{{{hostname}}}"),
            "template should contain hostname placeholder, got: {}",
            output.template_content
        );
    }

    #[test]
    fn test_extract_device_vlan_vars_flow_into_port_assignments() {
        let existing_services: HashMap<String, String> = HashMap::new();
        let existing_elements: HashMap<String, String> = HashMap::new();

        let output = extract_device(
            SHOW_VERSION,
            SHOW_INVENTORY,
            SHOW_IP_BRIEF,
            SHOW_RUNNING_CONFIG,
            &existing_services,
            &existing_elements,
        ).expect("extraction should succeed");

        // The new access-vlan10 service should exist with literal (unparameterized) port_config.
        // Service files on disk stay literal to ensure existing-service matching on re-extraction.
        let svc = output.services.iter().find(|s| s.name == "access-vlan10")
            .expect("access-vlan10 service should exist");
        assert!(
            svc.port_config.contains("switchport access vlan 10"),
            "service port_config should contain literal VLAN, got: {}",
            svc.port_config
        );

        // Port assignments for access-vlan10 should have vlan_id in vars (from VlanIdExtractor).
        let module = output.device_config.modules[0].as_ref().expect("module");
        for port in &module.ports {
            if port.service == "access-vlan10" {
                assert_eq!(
                    port.vars.get("vlan_id"),
                    Some(&"10".to_string()),
                    "port {} should have vlan_id=10 in vars",
                    port.name
                );
            }
        }
    }
}
