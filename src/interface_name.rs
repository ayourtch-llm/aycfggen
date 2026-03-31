use anyhow::Result;
use crate::model::{HardwareTemplate, PortDefinition};

/// Derive the full interface name for a port.
///
/// - When `omit_slot_prefix` is true: returns `name + index` (e.g., "GigabitEthernet0/0")
/// - When `omit_slot_prefix` is false: returns `name + slot_number + "/" + index`
///   where slot_number = slot_position + slot_index_base (e.g., "GigabitEthernet2/0/0")
pub fn derive_interface_name(
    port_def: &PortDefinition,
    slot_position: usize,
    slot_index_base: u32,
    omit_slot_prefix: bool,
) -> String {
    if omit_slot_prefix {
        format!("{}{}", port_def.name, port_def.index)
    } else {
        let slot_number = slot_position as u32 + slot_index_base;
        format!("{}{}/{}", port_def.name, slot_number, port_def.index)
    }
}

/// Derive the full interface name for a port assignment identifier.
///
/// The `port_id` may optionally include a sub-interface suffix (e.g., `Port0.100`).
/// If a suffix is present, the parent port (`Port0`) is looked up in `hw_template`,
/// the base interface name is derived, and the suffix (`.100`) is appended.
///
/// Returns an error if the parent port is not found in the hardware template.
pub fn derive_interface_name_for_port_id(
    port_id: &str,
    hw_template: &HardwareTemplate,
    slot_position: usize,
    slot_index_base: u32,
    omit_slot_prefix: bool,
) -> Result<String> {
    let (parent_id, suffix) = match port_id.find('.') {
        Some(dot_pos) => (&port_id[..dot_pos], &port_id[dot_pos..]),
        None => (port_id, ""),
    };

    let port_def = hw_template.ports.get(parent_id)
        .ok_or_else(|| anyhow::anyhow!(
            "port {:?} not found in hardware template",
            parent_id
        ))?;

    let base = derive_interface_name(port_def, slot_position, slot_index_base, omit_slot_prefix);
    Ok(format!("{}{}", base, suffix))
}

/// Resolve the effective slot_index_base from device and hardware template values.
/// Priority: device override > hardware template > default (0).
pub fn resolve_slot_index_base(
    device_slot_index_base: Option<u32>,
    hw_slot_index_base: Option<u32>,
) -> u32 {
    device_slot_index_base
        .or(hw_slot_index_base)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use crate::model::HardwareTemplate;

    fn make_port(name: &str, index: &str) -> PortDefinition {
        PortDefinition {
            name: name.to_string(),
            index: index.to_string(),
        }
    }

    fn make_hw_template(ports: &[(&str, &str, &str)]) -> HardwareTemplate {
        let mut map = IndexMap::new();
        for (port_id, name, index) in ports {
            map.insert(port_id.to_string(), PortDefinition {
                name: name.to_string(),
                index: index.to_string(),
            });
        }
        HardwareTemplate {
            vendor: None,
            slot_index_base: None,
            ports: map,
        }
    }

    // --- Sub-interface tests ---

    #[test]
    fn test_sub_iface_omit_slot_prefix_no_suffix() {
        // Port0 with no suffix — same as original behavior
        let hw = make_hw_template(&[("Port0", "GigabitEthernet", "0/0")]);
        let result = derive_interface_name_for_port_id("Port0", &hw, 0, 0, true)
            .expect("no error");
        assert_eq!(result, "GigabitEthernet0/0");
    }

    #[test]
    fn test_sub_iface_omit_slot_prefix_with_suffix() {
        // Port0.100 — parent is Port0, suffix is .100
        let hw = make_hw_template(&[("Port0", "GigabitEthernet", "0/0")]);
        let result = derive_interface_name_for_port_id("Port0.100", &hw, 0, 0, true)
            .expect("no error");
        assert_eq!(result, "GigabitEthernet0/0.100");
    }

    #[test]
    fn test_sub_iface_with_slot_prefix_and_suffix() {
        // Port0.100 on multi-module device (omit_slot_prefix=false, slot_position=1, base=0)
        let hw = make_hw_template(&[("Port0", "GigabitEthernet", "0/0")]);
        let result = derive_interface_name_for_port_id("Port0.100", &hw, 1, 0, false)
            .expect("no error");
        assert_eq!(result, "GigabitEthernet1/0/0.100");
    }

    #[test]
    fn test_sub_iface_different_port_with_suffix() {
        // Port1.200 — parent is Port1
        let hw = make_hw_template(&[
            ("Port0", "GigabitEthernet", "0/0"),
            ("Port1", "GigabitEthernet", "0/1"),
        ]);
        let result = derive_interface_name_for_port_id("Port1.200", &hw, 0, 0, true)
            .expect("no error");
        assert_eq!(result, "GigabitEthernet0/1.200");
    }

    #[test]
    fn test_sub_iface_unknown_parent_returns_error() {
        let hw = make_hw_template(&[("Port0", "GigabitEthernet", "0/0")]);
        let result = derive_interface_name_for_port_id("Port99.100", &hw, 0, 0, true);
        assert!(result.is_err(), "unknown parent port should return an error");
    }

    #[test]
    fn test_omit_slot_prefix_true() {
        let port = make_port("GigabitEthernet", "0/0");
        let result = derive_interface_name(&port, 0, 0, true);
        assert_eq!(result, "GigabitEthernet0/0");
    }

    #[test]
    fn test_omit_slot_prefix_false_slot2() {
        let port = make_port("GigabitEthernet", "0/0");
        let result = derive_interface_name(&port, 2, 0, false);
        assert_eq!(result, "GigabitEthernet2/0/0");
    }

    #[test]
    fn test_slot0_with_simple_index() {
        let port = make_port("Ethernet", "1");
        let result = derive_interface_name(&port, 0, 0, false);
        assert_eq!(result, "Ethernet0/1");
    }

    #[test]
    fn test_slot_index_base_offset() {
        let port = make_port("GigabitEthernet", "0/0");
        let result = derive_interface_name(&port, 1, 1, false);
        assert_eq!(result, "GigabitEthernet2/0/0");
    }

    #[test]
    fn test_resolve_slot_index_base_device_wins() {
        assert_eq!(resolve_slot_index_base(Some(1), Some(0)), 1);
    }

    #[test]
    fn test_resolve_slot_index_base_hw_fallback() {
        assert_eq!(resolve_slot_index_base(None, Some(1)), 1);
    }

    #[test]
    fn test_resolve_slot_index_base_default() {
        assert_eq!(resolve_slot_index_base(None, None), 0);
    }
}
