use crate::model::PortDefinition;

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

    fn make_port(name: &str, index: &str) -> PortDefinition {
        PortDefinition {
            name: name.to_string(),
            index: index.to_string(),
        }
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
