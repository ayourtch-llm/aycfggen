/// Hardware Profile Discovery (Stage 1 of aycfgextract pipeline).
///
/// Builds hardware profiles (HardwareTemplate / ports.json) and device metadata
/// from `show version`, `show inventory`, and `show ip interface brief` output.

use anyhow::{anyhow, Result};
use indexmap::IndexMap;

use crate::model::{HardwareTemplate, PortDefinition};
use crate::show_parsers::{InterfaceBriefEntry, InventoryItem, ShowVersionInfo};

// ─── Known prefixes (longest first to avoid ambiguous matching) ───────────────

/// All known interface name prefixes, ordered longest-first to avoid ambiguous prefix matches.
const KNOWN_PREFIXES: &[&str] = &[
    "TwentyFiveGigE",
    "FortyGigabitEthernet",
    "TenGigabitEthernet",
    "HundredGigE",
    "GigabitEthernet",
    "FastEthernet",
    "Ethernet",
    "Serial",
    "Loopback",
    "Port-channel",
    "Tunnel",
    "Vlan",
];

/// Prefixes that indicate virtual (non-physical) interfaces — excluded from ports.json.
const VIRTUAL_PREFIXES: &[&str] = &["Loopback", "Port-channel", "Tunnel", "Vlan"];

// ─── ParsedInterfaceName ──────────────────────────────────────────────────────

/// A parsed Cisco IOS interface name broken into its components.
#[derive(Debug, PartialEq)]
pub struct ParsedInterfaceName {
    /// The long-form interface type prefix (e.g., `"GigabitEthernet"`).
    pub prefix: String,
    /// Slot number for multi-module devices (e.g., `Some(1)` for `GigabitEthernet1/0/3`).
    /// `None` for single-module devices.
    pub slot: Option<u32>,
    /// The port index portion (e.g., `"0/3"` or `"0/0"`).
    pub port_index: String,
    /// Sub-interface number if present (e.g., `Some(100)` for `.100`), otherwise `None`.
    pub sub_interface: Option<u32>,
}

/// Parse a full interface name like `"GigabitEthernet1/0/3"` or `"GigabitEthernet0/0.100"`
/// into its components.
///
/// `is_multi_module` controls whether the first numeric segment after the prefix is treated
/// as a slot number (`true`) or as part of the port index (`false`).
///
/// Returns `None` if the name does not match any known prefix or has an unexpected format.
pub fn parse_interface_name(name: &str, is_multi_module: bool) -> Option<ParsedInterfaceName> {
    // Find which known prefix matches
    let prefix = KNOWN_PREFIXES.iter().find(|&&p| name.starts_with(p))?;
    let after_prefix = &name[prefix.len()..];

    // Strip sub-interface suffix first: look for a dot that is followed by digits at the end
    let (numeric_part, sub_interface) = if let Some(dot_pos) = after_prefix.rfind('.') {
        let after_dot = &after_prefix[dot_pos + 1..];
        // Only treat as sub-interface if the part after the last dot is all digits
        if !after_dot.is_empty() && after_dot.chars().all(|c| c.is_ascii_digit()) {
            let sub: u32 = after_dot.parse().ok()?;
            (&after_prefix[..dot_pos], Some(sub))
        } else {
            (after_prefix, None)
        }
    } else {
        (after_prefix, None)
    };

    // numeric_part is like "1/0/3" (multi-module) or "0/3" (single-module) or "0" (loopback)
    if is_multi_module {
        // First segment is the slot, the rest is the port index
        if let Some(slash_pos) = numeric_part.find('/') {
            let slot_str = &numeric_part[..slash_pos];
            let port_index = &numeric_part[slash_pos + 1..];
            let slot: u32 = slot_str.parse().ok()?;
            Some(ParsedInterfaceName {
                prefix: prefix.to_string(),
                slot: Some(slot),
                port_index: port_index.to_string(),
                sub_interface,
            })
        } else {
            // No slash: treat as single number with no port index (e.g., Loopback0 on multi-module)
            Some(ParsedInterfaceName {
                prefix: prefix.to_string(),
                slot: None,
                port_index: numeric_part.to_string(),
                sub_interface,
            })
        }
    } else {
        // Single-module: entire numeric portion is the port index
        Some(ParsedInterfaceName {
            prefix: prefix.to_string(),
            slot: None,
            port_index: numeric_part.to_string(),
            sub_interface,
        })
    }
}

// ─── DiscoveredDevice / DiscoveredModule ──────────────────────────────────────

/// A module (slot) discovered within a device.
#[derive(Debug)]
pub struct DiscoveredModule {
    /// SKU / PID from inventory (e.g., `"WS-C3850-24T"`).
    pub sku: String,
    /// Serial number from inventory.
    pub serial: String,
    /// Slot number (0-based as seen in inventory).
    pub slot: u32,
    /// The hardware template (ports.json equivalent) derived for this module.
    pub hardware_template: HardwareTemplate,
}

/// Top-level result of hardware discovery for a single device.
#[derive(Debug)]
pub struct DiscoveredDevice {
    /// Hostname from `show version`.
    pub hostname: String,
    /// Chassis serial number from `show version`.
    pub serial_number: String,
    /// Software image filename from `show version`.
    pub software_image: String,
    /// Platform / model from `show version`.
    pub platform: String,
    /// `true` only when there is exactly one module and interface names contain no slot prefix.
    pub omit_slot_prefix: bool,
    /// The lowest slot number observed in the interface names (or inventory).
    pub slot_index_base: u32,
    /// One entry per physical module/slot.
    pub modules: Vec<DiscoveredModule>,
}

// ─── discover_hardware ────────────────────────────────────────────────────────

/// Build hardware profiles from show command output.
///
/// # Process
///
/// 1. Determine module count from inventory (items with a slot number).
/// 2. Determine `is_multi_module` from module count.
/// 3. Parse all physical interface names (filter out Vlan, Loopback, Tunnel, Port-channel,
///    and sub-interfaces).
/// 4. Group physical interfaces by slot.
/// 5. Match each slot's interfaces to its inventory item by slot number.
/// 6. Build a `HardwareTemplate` for each unique SKU.
/// 7. Determine `omit_slot_prefix` (true only if single module and no slot in interface names).
/// 8. Determine `slot_index_base` (lowest slot number seen in interface names for multi-module,
///    or 0 for single-module devices).
pub fn discover_hardware(
    version: &ShowVersionInfo,
    inventory: &[InventoryItem],
    interfaces: &[InterfaceBriefEntry],
) -> Result<DiscoveredDevice> {
    // ── Step 1: collect slotted inventory items ───────────────────────────────
    // Sort slotted items by their (0-based) inventory slot number so they can be
    // aligned positionally with the slot numbers seen in interface names.
    let mut slotted: Vec<&InventoryItem> = inventory.iter().filter(|i| i.slot.is_some()).collect();
    slotted.sort_by_key(|i| i.slot.unwrap());
    let module_count = slotted.len();
    let is_multi_module = module_count > 1;

    // ── Step 2: parse physical interface names ────────────────────────────────
    // Filter out virtual interfaces and sub-interfaces; keep only physical ports.
    let physical_ifaces: Vec<ParsedInterfaceName> = interfaces
        .iter()
        .filter_map(|e| {
            // Skip known virtual prefixes early
            for vp in VIRTUAL_PREFIXES {
                if e.name.starts_with(vp) {
                    return None;
                }
            }
            let parsed = parse_interface_name(&e.name, is_multi_module)?;
            // Drop sub-interfaces (ports.json has only physical ports)
            if parsed.sub_interface.is_some() {
                return None;
            }
            Some(parsed)
        })
        .collect();

    // ── Step 3: determine omit_slot_prefix and slot_index_base ───────────────
    // omit_slot_prefix: true only when single module AND interface names have no slot segment
    let has_slot_in_ifaces = physical_ifaces.iter().any(|i| i.slot.is_some());
    let omit_slot_prefix = !is_multi_module && !has_slot_in_ifaces;

    // slot_index_base: lowest slot number as seen in interface names (for multi-module),
    // or 0 for single-module devices (interface names have no slot component).
    let slot_index_base: u32 = if is_multi_module {
        physical_ifaces.iter().filter_map(|i| i.slot).min().unwrap_or(0)
    } else {
        0
    };

    // ── Step 4: group physical interfaces by slot ─────────────────────────────
    // For multi-module devices: group by the slot number parsed from the interface name.
    // For single-module devices: all interfaces go into slot `slot_index_base` (= 0).
    let mut slot_to_ifaces: IndexMap<u32, Vec<&ParsedInterfaceName>> = IndexMap::new();

    if is_multi_module {
        for iface in &physical_ifaces {
            let slot = iface.slot.unwrap_or(slot_index_base);
            slot_to_ifaces.entry(slot).or_default().push(iface);
        }
    } else {
        // Single module: all interfaces belong to the one slot
        if !physical_ifaces.is_empty() {
            for iface in &physical_ifaces {
                slot_to_ifaces.entry(0).or_default().push(iface);
            }
        }
    }

    // ── Step 5: determine the set of slots to create modules for ─────────────
    // For multi-module: union of interface-derived slots.
    // For single-module: use slot 0, whether or not there are interfaces.
    let mut all_slots: Vec<u32> = if is_multi_module {
        let mut s: Vec<u32> = slot_to_ifaces.keys().copied().collect();
        s.sort();
        s
    } else {
        vec![0]
    };

    // If there are no interface-derived slots but we have slotted inventory items,
    // add their interface-space slot numbers (inventory_slot_0_based + slot_index_base).
    if all_slots.is_empty() {
        for (inv_idx, _) in slotted.iter().enumerate() {
            all_slots.push(slot_index_base + inv_idx as u32);
        }
        all_slots.sort();
    }

    // ── Step 6: build modules ─────────────────────────────────────────────────
    // Inventory items are sorted by their 0-based slot, so we align them positionally
    // to the interface-space slots (sorted ascending).  inventory_item[i] corresponds
    // to the i-th distinct slot number seen in the interfaces.
    let fallback_inv: Option<&InventoryItem> = if slotted.is_empty() {
        inventory.first()
    } else {
        None
    };

    let mut modules: Vec<DiscoveredModule> = Vec::new();

    for (slot_idx, &slot) in all_slots.iter().enumerate() {
        // Match inventory item by positional alignment
        let inv_item: Option<&InventoryItem> =
            slotted.get(slot_idx).copied().or(fallback_inv);

        let (sku, serial) = if let Some(item) = inv_item {
            (item.pid.clone(), item.serial.clone())
        } else {
            // No inventory item for this slot: use platform from show version
            (version.platform.clone(), String::new())
        };

        // Build the ports.json for this slot
        let ifaces_for_slot = slot_to_ifaces.get(&slot).map(|v| v.as_slice()).unwrap_or(&[]);

        // Sort by port_index so Port0, Port1, ... are in a stable, meaningful order
        let mut sorted_ifaces: Vec<&&ParsedInterfaceName> = ifaces_for_slot.iter().collect();
        sorted_ifaces.sort_by(|a, b| {
            // Numeric-aware sort: split on '/' and compare segments numerically
            let a_parts: Vec<u32> = a
                .port_index
                .split('/')
                .filter_map(|s| s.parse().ok())
                .collect();
            let b_parts: Vec<u32> = b
                .port_index
                .split('/')
                .filter_map(|s| s.parse().ok())
                .collect();
            a_parts.cmp(&b_parts)
        });

        let mut ports: IndexMap<String, PortDefinition> = IndexMap::new();
        for (idx, iface) in sorted_ifaces.iter().enumerate() {
            let port_id = format!("Port{}", idx);
            ports.insert(
                port_id,
                PortDefinition {
                    name: iface.prefix.clone(),
                    index: iface.port_index.clone(),
                },
            );
        }

        let hardware_template = HardwareTemplate {
            vendor: Some("cisco-ios".to_string()),
            slot_index_base: None,
            ports,
        };

        modules.push(DiscoveredModule {
            sku,
            serial,
            slot,
            hardware_template,
        });
    }

    if modules.is_empty() {
        return Err(anyhow!(
            "no modules discovered: no inventory items and no physical interfaces"
        ));
    }

    Ok(DiscoveredDevice {
        hostname: version.hostname.clone(),
        serial_number: version.serial_number.clone(),
        software_image: version.software_image.clone(),
        platform: version.platform.clone(),
        omit_slot_prefix,
        slot_index_base,
        modules,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::show_parsers::{parse_show_inventory, parse_show_ip_interface_brief, parse_show_version};

    // ── parse_interface_name ──────────────────────────────────────────────────

    #[test]
    fn test_parse_iface_single_module_gigabit() {
        // GigabitEthernet0/1 on single-module device
        let p = parse_interface_name("GigabitEthernet0/1", false).unwrap();
        assert_eq!(p.prefix, "GigabitEthernet");
        assert_eq!(p.slot, None);
        assert_eq!(p.port_index, "0/1");
        assert_eq!(p.sub_interface, None);
    }

    #[test]
    fn test_parse_iface_multi_module_gigabit() {
        // GigabitEthernet1/0/3 on multi-module device
        let p = parse_interface_name("GigabitEthernet1/0/3", true).unwrap();
        assert_eq!(p.prefix, "GigabitEthernet");
        assert_eq!(p.slot, Some(1));
        assert_eq!(p.port_index, "0/3");
        assert_eq!(p.sub_interface, None);
    }

    #[test]
    fn test_parse_iface_multi_module_slot2() {
        let p = parse_interface_name("GigabitEthernet2/0/5", true).unwrap();
        assert_eq!(p.slot, Some(2));
        assert_eq!(p.port_index, "0/5");
    }

    #[test]
    fn test_parse_iface_sub_interface_single_module() {
        // GigabitEthernet0/0.100 on single-module router
        let p = parse_interface_name("GigabitEthernet0/0.100", false).unwrap();
        assert_eq!(p.prefix, "GigabitEthernet");
        assert_eq!(p.slot, None);
        assert_eq!(p.port_index, "0/0");
        assert_eq!(p.sub_interface, Some(100));
    }

    #[test]
    fn test_parse_iface_sub_interface_multi_module() {
        // GigabitEthernet1/0/0.100 on multi-module device
        let p = parse_interface_name("GigabitEthernet1/0/0.100", true).unwrap();
        assert_eq!(p.prefix, "GigabitEthernet");
        assert_eq!(p.slot, Some(1));
        assert_eq!(p.port_index, "0/0");
        assert_eq!(p.sub_interface, Some(100));
    }

    #[test]
    fn test_parse_iface_ten_gig() {
        let p = parse_interface_name("TenGigabitEthernet1/1/1", true).unwrap();
        assert_eq!(p.prefix, "TenGigabitEthernet");
        assert_eq!(p.slot, Some(1));
        assert_eq!(p.port_index, "1/1");
    }

    #[test]
    fn test_parse_iface_twenty_five_gig() {
        let p = parse_interface_name("TwentyFiveGigE1/0/1", true).unwrap();
        assert_eq!(p.prefix, "TwentyFiveGigE");
        assert_eq!(p.slot, Some(1));
        assert_eq!(p.port_index, "0/1");
    }

    #[test]
    fn test_parse_iface_forty_gig() {
        let p = parse_interface_name("FortyGigabitEthernet1/1/1", true).unwrap();
        assert_eq!(p.prefix, "FortyGigabitEthernet");
        assert_eq!(p.slot, Some(1));
        assert_eq!(p.port_index, "1/1");
    }

    #[test]
    fn test_parse_iface_hundred_gig() {
        let p = parse_interface_name("HundredGigE1/0/1", true).unwrap();
        assert_eq!(p.prefix, "HundredGigE");
        assert_eq!(p.slot, Some(1));
        assert_eq!(p.port_index, "0/1");
    }

    #[test]
    fn test_parse_iface_fast_ethernet_single() {
        let p = parse_interface_name("FastEthernet0/1", false).unwrap();
        assert_eq!(p.prefix, "FastEthernet");
        assert_eq!(p.slot, None);
        assert_eq!(p.port_index, "0/1");
    }

    #[test]
    fn test_parse_iface_serial_single() {
        let p = parse_interface_name("Serial0/0/0", false).unwrap();
        assert_eq!(p.prefix, "Serial");
        assert_eq!(p.slot, None);
        assert_eq!(p.port_index, "0/0/0");
    }

    #[test]
    fn test_parse_iface_loopback() {
        let p = parse_interface_name("Loopback0", false).unwrap();
        assert_eq!(p.prefix, "Loopback");
        assert_eq!(p.slot, None);
        assert_eq!(p.port_index, "0");
        assert_eq!(p.sub_interface, None);
    }

    #[test]
    fn test_parse_iface_vlan() {
        let p = parse_interface_name("Vlan1", false).unwrap();
        assert_eq!(p.prefix, "Vlan");
        assert_eq!(p.port_index, "1");
    }

    #[test]
    fn test_parse_iface_port_channel() {
        let p = parse_interface_name("Port-channel1", false).unwrap();
        assert_eq!(p.prefix, "Port-channel");
        assert_eq!(p.port_index, "1");
    }

    #[test]
    fn test_parse_iface_tunnel() {
        let p = parse_interface_name("Tunnel0", false).unwrap();
        assert_eq!(p.prefix, "Tunnel");
        assert_eq!(p.port_index, "0");
    }

    #[test]
    fn test_parse_iface_unknown_returns_none() {
        assert!(parse_interface_name("Bogus0/1", false).is_none());
    }

    // ── discover_hardware: single-module switch ───────────────────────────────

    const SHOW_VERSION_SINGLE: &str = "\
switch1 uptime is 2 weeks, 3 days, 4 hours, 5 minutes
System image file is \"flash:c3560-ipbasek9-mz.150-2.SE11.bin\"
Model number                    : WS-C3560-24TS
System serial number            : FOC1234X0AB";

    const SHOW_INVENTORY_SINGLE: &str = "\
NAME: \"1\", DESCR: \"WS-C3560-24TS\"
PID: WS-C3560-24TS , VID: V02  , SN: FOC1234X0AB

";

    /// 4 GigabitEthernet ports (0/0–0/3) on a single-module switch, plus Vlan1 and Loopback0
    const SHOW_IP_BRIEF_SINGLE: &str = "\
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet0/0     unassigned      YES unset  up                    up
GigabitEthernet0/1     unassigned      YES unset  up                    up
GigabitEthernet0/2     unassigned      YES unset  down                  down
GigabitEthernet0/3     unassigned      YES unset  administratively down down
Vlan1                  192.168.1.1     YES NVRAM  up                    up
Loopback0              10.0.0.1        YES NVRAM  up                    up
";

    #[test]
    fn test_single_module_switch_omit_slot_prefix() {
        let version = parse_show_version(SHOW_VERSION_SINGLE).unwrap();
        let inventory = parse_show_inventory(SHOW_INVENTORY_SINGLE);
        let interfaces = parse_show_ip_interface_brief(SHOW_IP_BRIEF_SINGLE);

        let device = discover_hardware(&version, &inventory, &interfaces).unwrap();

        assert_eq!(device.hostname, "switch1");
        assert_eq!(device.serial_number, "FOC1234X0AB");
        assert_eq!(device.software_image, "c3560-ipbasek9-mz.150-2.SE11.bin");
        assert!(device.omit_slot_prefix, "single-module → omit_slot_prefix=true");
        assert_eq!(device.slot_index_base, 0);
        assert_eq!(device.modules.len(), 1);

        let m = &device.modules[0];
        assert_eq!(m.sku, "WS-C3560-24TS");
        assert_eq!(m.serial, "FOC1234X0AB");
        // 4 physical ports, Vlan1 and Loopback0 excluded
        assert_eq!(m.hardware_template.ports.len(), 4);
        assert_eq!(m.hardware_template.ports["Port0"].name, "GigabitEthernet");
        assert_eq!(m.hardware_template.ports["Port0"].index, "0/0");
        assert_eq!(m.hardware_template.ports["Port3"].index, "0/3");
    }

    #[test]
    fn test_single_module_vlan_loopback_excluded() {
        let version = parse_show_version(SHOW_VERSION_SINGLE).unwrap();
        let inventory = parse_show_inventory(SHOW_INVENTORY_SINGLE);
        let interfaces = parse_show_ip_interface_brief(SHOW_IP_BRIEF_SINGLE);

        let device = discover_hardware(&version, &inventory, &interfaces).unwrap();

        // Confirm Vlan and Loopback ports are NOT in the hardware template
        let ports = &device.modules[0].hardware_template.ports;
        assert!(
            ports.values().all(|p| p.name != "Vlan" && p.name != "Loopback"),
            "Vlan and Loopback must not appear in ports.json"
        );
    }

    // ── discover_hardware: multi-module stack (same SKU) ─────────────────────

    const SHOW_VERSION_STACK: &str = "\
switch-stack uptime is 1 week
System image file is \"flash:packages.conf\"
Model number                    : WS-C3850-24T
System serial number            : FOC2001A1BB";

    const SHOW_INVENTORY_STACK: &str = "\
NAME: \"Switch 1\", DESCR: \"WS-C3850-24T\"
PID: WS-C3850-24T  , VID: V01  , SN: FOC2001A1BB

NAME: \"Switch 2\", DESCR: \"WS-C3850-24T\"
PID: WS-C3850-24T  , VID: V01  , SN: FOC2001A1CC

";

    /// 4 ports on slot 1 and 4 ports on slot 2
    const SHOW_IP_BRIEF_STACK: &str = "\
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet1/0/1   unassigned      YES unset  up                    up
GigabitEthernet1/0/2   unassigned      YES unset  up                    up
GigabitEthernet1/0/3   unassigned      YES unset  down                  down
GigabitEthernet1/0/4   unassigned      YES unset  administratively down down
GigabitEthernet2/0/1   unassigned      YES unset  up                    up
GigabitEthernet2/0/2   unassigned      YES unset  up                    up
GigabitEthernet2/0/3   unassigned      YES unset  down                  down
GigabitEthernet2/0/4   unassigned      YES unset  administratively down down
Vlan1                  192.168.1.1     YES NVRAM  up                    up
";

    #[test]
    fn test_multi_module_stack_omit_slot_prefix_false() {
        let version = parse_show_version(SHOW_VERSION_STACK).unwrap();
        let inventory = parse_show_inventory(SHOW_INVENTORY_STACK);
        let interfaces = parse_show_ip_interface_brief(SHOW_IP_BRIEF_STACK);

        let device = discover_hardware(&version, &inventory, &interfaces).unwrap();

        assert!(!device.omit_slot_prefix, "multi-module → omit_slot_prefix=false");
        assert_eq!(device.slot_index_base, 1, "lowest slot observed is 1");
        assert_eq!(device.modules.len(), 2);

        // Both modules have the same SKU
        assert_eq!(device.modules[0].sku, "WS-C3850-24T");
        assert_eq!(device.modules[1].sku, "WS-C3850-24T");
        // Different serials
        assert_eq!(device.modules[0].serial, "FOC2001A1BB");
        assert_eq!(device.modules[1].serial, "FOC2001A1CC");

        // Each module has 4 physical ports
        assert_eq!(device.modules[0].hardware_template.ports.len(), 4);
        assert_eq!(device.modules[1].hardware_template.ports.len(), 4);
        // Ports are port_index-only (no slot in the port definition)
        assert_eq!(device.modules[0].hardware_template.ports["Port0"].index, "0/1");
        assert_eq!(device.modules[1].hardware_template.ports["Port0"].index, "0/1");
    }

    // ── discover_hardware: mixed-SKU stack ────────────────────────────────────

    const SHOW_INVENTORY_MIXED: &str = "\
NAME: \"Switch 1\", DESCR: \"WS-C3850-24T\"
PID: WS-C3850-24T  , VID: V01  , SN: FOC2001A1BB

NAME: \"Switch 2\", DESCR: \"WS-C3850-48T\"
PID: WS-C3850-48T  , VID: V01  , SN: FOC2001A1CC

";

    const SHOW_IP_BRIEF_MIXED: &str = "\
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet1/0/1   unassigned      YES unset  up                    up
GigabitEthernet1/0/2   unassigned      YES unset  up                    up
GigabitEthernet2/0/1   unassigned      YES unset  up                    up
GigabitEthernet2/0/2   unassigned      YES unset  up                    up
GigabitEthernet2/0/3   unassigned      YES unset  up                    up
";

    #[test]
    fn test_mixed_sku_stack() {
        let version = parse_show_version(SHOW_VERSION_STACK).unwrap();
        let inventory = parse_show_inventory(SHOW_INVENTORY_MIXED);
        let interfaces = parse_show_ip_interface_brief(SHOW_IP_BRIEF_MIXED);

        let device = discover_hardware(&version, &inventory, &interfaces).unwrap();

        assert_eq!(device.modules.len(), 2);
        // Slot 1 → WS-C3850-24T with 2 ports
        let m0 = &device.modules[0];
        assert_eq!(m0.sku, "WS-C3850-24T");
        assert_eq!(m0.hardware_template.ports.len(), 2);
        // Slot 2 → WS-C3850-48T with 3 ports
        let m1 = &device.modules[1];
        assert_eq!(m1.sku, "WS-C3850-48T");
        assert_eq!(m1.hardware_template.ports.len(), 3);
    }

    // ── discover_hardware: sub-interfaces excluded ────────────────────────────

    const SHOW_IP_BRIEF_SUBIF: &str = "\
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet0/0     unassigned      YES unset  up                    up
GigabitEthernet0/0.100 192.168.100.1   YES NVRAM  up                    up
GigabitEthernet0/0.200 192.168.200.1   YES NVRAM  up                    up
GigabitEthernet0/1     unassigned      YES unset  up                    up
Vlan1                  192.168.1.1     YES NVRAM  up                    up
";

    #[test]
    fn test_sub_interfaces_excluded_from_hardware_template() {
        let version = parse_show_version(SHOW_VERSION_SINGLE).unwrap();
        let inventory = parse_show_inventory(SHOW_INVENTORY_SINGLE);
        let interfaces = parse_show_ip_interface_brief(SHOW_IP_BRIEF_SUBIF);

        let device = discover_hardware(&version, &inventory, &interfaces).unwrap();

        let ports = &device.modules[0].hardware_template.ports;
        // Only 2 physical ports (GE0/0 and GE0/1), sub-interfaces .100 and .200 excluded
        assert_eq!(ports.len(), 2, "sub-interfaces must not appear in ports.json");
        assert_eq!(ports["Port0"].index, "0/0");
        assert_eq!(ports["Port1"].index, "0/1");
    }

    // ── discover_hardware: only virtual interfaces (no physical ports) ─────────

    const SHOW_IP_BRIEF_VIRTUAL_ONLY: &str = "\
Interface              IP-Address      OK? Method Status                Protocol
Vlan1                  192.168.1.1     YES NVRAM  up                    up
Loopback0              10.0.0.1        YES NVRAM  up                    up
";

    #[test]
    fn test_no_physical_ports_produces_empty_template() {
        let version = parse_show_version(SHOW_VERSION_SINGLE).unwrap();
        let inventory = parse_show_inventory(SHOW_INVENTORY_SINGLE);
        let interfaces = parse_show_ip_interface_brief(SHOW_IP_BRIEF_VIRTUAL_ONLY);

        let device = discover_hardware(&version, &inventory, &interfaces).unwrap();

        // Device still discovered (inventory present), but hardware template has no ports
        assert_eq!(device.modules.len(), 1);
        assert_eq!(device.modules[0].hardware_template.ports.len(), 0);
        // omit_slot_prefix: single module, no slot in interface names → true
        assert!(device.omit_slot_prefix);
    }
}
