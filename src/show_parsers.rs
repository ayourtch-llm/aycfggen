/// Parsers for Cisco IOS `show` command output.

// ─── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub struct ShowVersionInfo {
    pub hostname: String,
    pub software_image: String,
    pub platform: String,
    pub serial_number: String,
}

#[derive(Debug, PartialEq)]
pub struct InventoryItem {
    pub name: String,
    pub description: String,
    pub pid: String,
    pub serial: String,
    pub slot: Option<u32>,
}

#[derive(Debug, PartialEq)]
pub struct InterfaceBriefEntry {
    pub name: String,
    pub ip_address: String,
    pub status: String,
    pub protocol: String,
}

#[derive(Debug, PartialEq)]
pub struct InterfaceStatusEntry {
    pub name: String,
    pub description: String,
    pub status: String,
    pub vlan: String,
    pub duplex: String,
    pub speed: String,
    pub media_type: String,
}

// ─── parse_show_version ───────────────────────────────────────────────────────

/// Parse `show version` output and return a [`ShowVersionInfo`].
///
/// Returns `None` if the output cannot be parsed.
pub fn parse_show_version(output: &str) -> Option<ShowVersionInfo> {
    let mut hostname = String::new();
    let mut software_image = String::new();
    let mut platform = String::new();
    let mut serial_number = String::new();

    for line in output.lines() {
        let line = line.trim_end_matches('\r');

        // Hostname: "<hostname> uptime is ..."
        if hostname.is_empty() {
            if let Some(pos) = line.find(" uptime is ") {
                hostname = line[..pos].trim().to_string();
            }
        }

        // Software image: System image file is "flash:/<image>"
        if software_image.is_empty() && line.trim_start().starts_with("System image file is") {
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if end > start {
                        let path = &line[start + 1..end];
                        // Extract just the filename: strip any path separators and device prefixes
                        // e.g. "flash:/dir/file.bin" → "file.bin", "flash:file.bin" → "file.bin"
                        let filename = path
                            .rsplit('/')
                            .next()
                            .unwrap_or(path)
                            .rsplit(':')
                            .next()
                            .unwrap_or(path);
                        software_image = filename.to_string();
                    }
                }
            }
        }

        // Platform / Model number: "Model number                    : <model>"
        if platform.is_empty() {
            if let Some(rest) = line.strip_prefix("Model number") {
                if let Some(val) = rest.split(':').nth(1) {
                    platform = val.trim().to_string();
                }
            }
        }

        // Serial number: "System serial number            : <serial>"
        //            or: "Processor board ID <serial>"
        if serial_number.is_empty() {
            if let Some(rest) = line.strip_prefix("System serial number") {
                if let Some(val) = rest.split(':').nth(1) {
                    serial_number = val.trim().to_string();
                }
            } else if let Some(rest) = line.strip_prefix("Processor board ID ") {
                let id = rest.trim();
                if !id.is_empty() {
                    serial_number = id.to_string();
                }
            }
        }
    }

    if hostname.is_empty() && software_image.is_empty() && platform.is_empty() && serial_number.is_empty() {
        return None;
    }

    Some(ShowVersionInfo {
        hostname,
        software_image,
        platform,
        serial_number,
    })
}

// ─── parse_show_inventory ─────────────────────────────────────────────────────

/// Parse `show inventory` output into a list of [`InventoryItem`]s.
pub fn parse_show_inventory(output: &str) -> Vec<InventoryItem> {
    let mut items: Vec<InventoryItem> = Vec::new();

    let mut current_name: Option<String> = None;
    let mut current_descr: Option<String> = None;

    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim();

        // NAME: "...", DESCR: "..."
        if trimmed.starts_with("NAME:") {
            // Extract NAME value
            let name_val = extract_quoted_after(trimmed, "NAME:").unwrap_or_default();
            // Extract DESCR value if on the same line
            let descr_val = extract_quoted_after(trimmed, "DESCR:").unwrap_or_default();
            current_name = Some(name_val);
            if !descr_val.is_empty() {
                current_descr = Some(descr_val);
            }
        } else if trimmed.starts_with("DESCR:") && current_name.is_some() && current_descr.is_none() {
            let descr_val = extract_quoted_after(trimmed, "DESCR:").unwrap_or_default();
            current_descr = Some(descr_val);
        } else if trimmed.starts_with("PID:") {
            if let Some(name) = current_name.take() {
                let descr = current_descr.take().unwrap_or_default();
                let pid = extract_field(trimmed, "PID:").unwrap_or_default();
                let serial = extract_field(trimmed, "SN:").unwrap_or_default();
                let slot = parse_slot_from_name(&name);
                items.push(InventoryItem { name, description: descr, pid, serial, slot });
            }
            current_descr = None;
        }
    }

    items
}

/// Extract the value for a field like `PID: WS-C3560 , VID: V02 , SN: FOC123`
fn extract_field<'a>(line: &'a str, key: &str) -> Option<String> {
    let pos = line.find(key)?;
    let after = line[pos + key.len()..].trim_start();
    // Value ends at the next comma
    let val = if let Some(comma) = after.find(',') {
        after[..comma].trim()
    } else {
        after.trim()
    };
    Some(val.to_string())
}

/// Extract a double-quoted value after the given key prefix.
fn extract_quoted_after(line: &str, key: &str) -> Option<String> {
    let pos = line.find(key)?;
    let after = &line[pos + key.len()..];
    let start = after.find('"')? + 1;
    let rest = &after[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Try to parse a slot number from an inventory item name like "Switch 1", "module 1", "Gi1/1".
fn parse_slot_from_name(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    // "Switch 1" → slot 1 (1-based → 0-based)
    if let Some(rest) = lower.strip_prefix("switch ") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            return Some(n.saturating_sub(1));
        }
    }
    // "module 0" → slot 0
    if let Some(rest) = lower.strip_prefix("module ") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            return Some(n);
        }
    }
    // "slot 0" → slot 0
    if let Some(rest) = lower.strip_prefix("slot ") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            return Some(n);
        }
    }
    None
}

// ─── parse_show_ip_interface_brief ────────────────────────────────────────────

/// Parse `show ip interface brief` output.
pub fn parse_show_ip_interface_brief(output: &str) -> Vec<InterfaceBriefEntry> {
    let mut entries = Vec::new();
    let mut past_header = false;

    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim();

        if trimmed.starts_with("Interface") && trimmed.contains("IP-Address") {
            past_header = true;
            continue;
        }
        if !past_header {
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }

        // Columns:  Interface(23) IP-Address(16) OK?(4) Method(7) Status(22) Protocol
        // We split on whitespace but must reconstruct "administratively down" (2 words)
        let tokens: Vec<&str> = trimmed.splitn(6, ' ').filter(|s| !s.is_empty()).collect();

        // Use the raw line for positional parsing — the header tells us columns:
        // Interface(0..23), IP-Address(23..39), OK?(39..43), Method(43..50), Status(50..72), Protocol(72..)
        if line.len() < 10 {
            continue;
        }

        let name = trimmed.split_whitespace().next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }

        // After the name, re-parse the rest of the line using the token approach.
        // Columns after name: ip, ok, method, status, protocol
        // "status" can be "administratively down" (2 tokens) or "up"/"down" (1 token)
        let rest = trimmed[name.len()..].trim();
        let rest_tokens: Vec<&str> = rest.split_whitespace().collect();

        if rest_tokens.len() < 4 {
            continue;
        }

        let ip_address = rest_tokens[0].to_string();
        // rest_tokens[1] = OK?, rest_tokens[2] = Method
        // rest_tokens[3..] = status [protocol]
        let status_and_protocol = &rest_tokens[3..];
        let (status, protocol) = if status_and_protocol.len() >= 3
            && status_and_protocol[0] == "administratively"
            && status_and_protocol[1] == "down"
        {
            ("administratively down".to_string(), status_and_protocol[2].to_string())
        } else if status_and_protocol.len() >= 2 {
            (status_and_protocol[0].to_string(), status_and_protocol[1].to_string())
        } else if status_and_protocol.len() == 1 {
            (status_and_protocol[0].to_string(), String::new())
        } else {
            continue;
        };

        entries.push(InterfaceBriefEntry { name, ip_address, status, protocol });
        let _ = tokens; // suppress unused warning
    }

    entries
}

// ─── parse_show_interfaces_status ────────────────────────────────────────────

/// Parse `show interfaces status` output.
///
/// The column layout (from the header) is:
/// - Port:   0..col_name
/// - Name:   col_name..col_status
/// - Status: col_status..col_vlan
/// - Vlan:   col_vlan..col_duplex
/// - Duplex, Speed, Type: parsed as whitespace-separated tokens after col_duplex
///   (the Duplex/Speed columns may overflow on long values like "a-1000").
pub fn parse_show_interfaces_status(output: &str) -> Vec<InterfaceStatusEntry> {
    let mut entries = Vec::new();
    let mut past_header = false;

    let mut col_name: usize = 10;
    let mut col_status: usize = 29;
    let mut col_vlan: usize = 42;
    let mut col_duplex: usize = 53;

    for line in output.lines() {
        let line = line.trim_end_matches('\r');

        if line.trim_start().starts_with("Port") && line.contains("Name") && line.contains("Status") {
            if let Some(p) = line.find("Name")   { col_name   = p; }
            if let Some(p) = line.find("Status") { col_status = p; }
            if let Some(p) = line.find("Vlan")   { col_vlan   = p; }
            if let Some(p) = line.find("Duplex") { col_duplex = p; }
            past_header = true;
            continue;
        }

        if !past_header || line.trim().is_empty() {
            continue;
        }

        if line.len() < col_name {
            continue;
        }

        // Fixed columns: Port, Name, Status, Vlan
        let pad_len = col_duplex + 20;
        let padded = format!("{:<width$}", line, width = pad_len);

        let name = padded[..col_name].trim().to_string();
        if name.is_empty() {
            continue;
        }

        let col_status_end = col_status.min(padded.len());
        let col_vlan_end   = col_vlan.min(padded.len());
        let col_duplex_end = col_duplex.min(padded.len());

        let description = padded[col_name..col_status_end].trim().to_string();
        let status      = padded[col_status_end..col_vlan_end].trim().to_string();
        let vlan        = padded[col_vlan_end..col_duplex_end].trim().to_string();

        // The remaining tokens (duplex, speed, type) are whitespace-separated
        let tail = padded[col_duplex..].trim_end();
        let tail_tokens: Vec<&str> = tail.split_whitespace().collect();

        let duplex     = tail_tokens.first().copied().unwrap_or("").to_string();
        let speed      = tail_tokens.get(1).copied().unwrap_or("").to_string();
        let media_type = tail_tokens[2..].join(" ");

        entries.push(InterfaceStatusEntry {
            name,
            description,
            status,
            vlan,
            duplex,
            speed,
            media_type,
        });
    }

    entries
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── show version ──────────────────────────────────────────────────────────

    const SHOW_VERSION_SINGLE: &str = "\
Cisco IOS Software, C3560 Software (C3560-IPBASEK9-M), Version 15.0(2)SE11, RELEASE SOFTWARE (fc3)
Technical Support: http://www.cisco.com/techsupport
Copyright (c) 1986-2025 by Cisco Systems, Inc.
Compiled Mon 15-Sep-25 13:05 by mcpre

ROM: Bootstrap program is C3560 boot loader
BOOTLDR: C3560 Boot Loader (C3560-HBOOT-M) Version 15.2(7r)E, RELEASE SOFTWARE (fc2)

switch1 uptime is 2 weeks, 3 days, 4 hours, 5 minutes
System returned to ROM by power-on
System restarted at 12:00:00 UTC Mon Jan 1 2024
System image file is \"flash:c3560-ipbasek9-mz.150-2.SE11.bin\"
Last reload reason: power-on


This product contains cryptographic features and is subject to United
States and local country laws governing import, export, transfer and
use.

License Level: ipservices
License Type: Evaluation
Next reload license Level: ipservices

cisco WS-C3560-24TS (APM86XXX) processor (revision D0) with 524288K bytes of memory.
Processor board ID FOC1234X0AB
Last reset from power-on
1 Virtual Ethernet interfaces
24 Gigabit Ethernet interfaces
2 Ten Gigabit Ethernet interfaces
The password-recovery mechanism is disabled.

512K bytes of flash-simulated non-volatile configuration memory.
Base ethernet MAC Address       : AA:BB:CC:DD:EE:FF
Motherboard assembly number     : 73-16573-05
Power supply part number        : 341-0675-02
Motherboard serial number       : FOC234X0AB
Power supply serial number      : LIT19381A8A
Model revision number           : D0
Motherboard revision number     : A0
Model number                    : WS-C3560-24TS
System serial number            : FOC1234X0AB
Top Assembly Part Number        : 68-5409-02
Top Assembly Revision Number    : B0
Version ID                      : V02
CLEI Code Number                : CMM1Z00DRB
Hardware Board Revision Number  : 0x02


Switch Ports Model                     SW Version            SW Image
------ ----- -----                     ----------            ----------
*    1 26    WS-C3560-24TS             15.0(2)SE11           C3560-IPBASEK9-M


Configuration register is 0xF";

    #[test]
    fn test_parse_show_version_single_switch() {
        let info = parse_show_version(SHOW_VERSION_SINGLE).expect("should parse");
        assert_eq!(info.hostname, "switch1");
        assert_eq!(info.software_image, "c3560-ipbasek9-mz.150-2.SE11.bin");
        assert_eq!(info.platform, "WS-C3560-24TS");
        assert_eq!(info.serial_number, "FOC1234X0AB");
    }

    // Stack show version: "Processor board ID" gives *first* serial.
    // System serial number line gives chassis (switch 1) serial.
    const SHOW_VERSION_STACK: &str = "\
Cisco IOS Software, CAT3K_CAA Software (CAT3K_CAA-UNIVERSALK9-M), Version 16.12.5, RELEASE SOFTWARE (fc2)

switch-stack uptime is 1 week, 2 days, 3 hours, 10 minutes
System returned to ROM by power-on
System image file is \"flash:packages.conf\"
Last reload reason: power-on

cisco WS-C3850-24T (MIPS) processor (revision V01) with 4194304K bytes of memory.
Processor board ID FOC2001A1BB
3 Virtual Ethernet interfaces
28 Gigabit Ethernet interfaces
The password-recovery mechanism is disabled.

Model number                    : WS-C3850-24T
System serial number            : FOC2001A1BB

Switch/Stack Mac Address        : aa:bb:cc:00:11:22 - Local Mac Address

Switch  Ports    Model                Serial No.   MAC address     Hw Ver.       Sw Ver.
------  -----   ---------             -----------  --------------  -------       --------
 1      28     WS-C3850-24T          FOC2001A1BB  aa:bb:cc:00:11:22  V01         16.12.5
 2      28     WS-C3850-24T          FOC2001A1CC  aa:bb:cc:00:22:33  V01         16.12.5

Configuration register is 0x102";

    #[test]
    fn test_parse_show_version_stack() {
        let info = parse_show_version(SHOW_VERSION_STACK).expect("should parse");
        assert_eq!(info.hostname, "switch-stack");
        // packages.conf is a valid image filename
        assert_eq!(info.software_image, "packages.conf");
        assert_eq!(info.platform, "WS-C3850-24T");
        // System serial number takes priority over Processor board ID
        assert_eq!(info.serial_number, "FOC2001A1BB");
    }

    // ── show inventory ────────────────────────────────────────────────────────

    const SHOW_INVENTORY_SINGLE: &str = "\
NAME: \"1\", DESCR: \"WS-C3560-24TS\"
PID: WS-C3560-24TS , VID: V02  , SN: FOC1234X0AB


";

    #[test]
    fn test_parse_show_inventory_single_chassis() {
        let items = parse_show_inventory(SHOW_INVENTORY_SINGLE);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "1");
        assert_eq!(items[0].description, "WS-C3560-24TS");
        assert_eq!(items[0].pid, "WS-C3560-24TS");
        assert_eq!(items[0].serial, "FOC1234X0AB");
    }

    const SHOW_INVENTORY_MULTI: &str = "\
NAME: \"Switch 1\", DESCR: \"WS-C3850-24T\"
PID: WS-C3850-24T  , VID: V01  , SN: FOC2001A1BB

NAME: \"Switch 2\", DESCR: \"WS-C3850-24T\"
PID: WS-C3850-24T  , VID: V01  , SN: FOC2001A1CC

NAME: \"Switch 1 - Power Supply A\", DESCR: \"CAB-TA-NA\"
PID: CAB-TA-NA      , VID: V01  , SN: LIT19381A00

";

    #[test]
    fn test_parse_show_inventory_multi_slot() {
        let items = parse_show_inventory(SHOW_INVENTORY_MULTI);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].name, "Switch 1");
        assert_eq!(items[0].pid, "WS-C3850-24T");
        assert_eq!(items[0].serial, "FOC2001A1BB");
        assert_eq!(items[0].slot, Some(0));

        assert_eq!(items[1].name, "Switch 2");
        assert_eq!(items[1].pid, "WS-C3850-24T");
        assert_eq!(items[1].serial, "FOC2001A1CC");
        assert_eq!(items[1].slot, Some(1));

        // Power supply: no slot parsed
        assert_eq!(items[2].name, "Switch 1 - Power Supply A");
        assert_eq!(items[2].slot, None);
    }

    // ── show ip interface brief ───────────────────────────────────────────────

    const SHOW_IP_IFACE_BRIEF: &str = "\
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet0/1     unassigned      YES unset  up                    up
GigabitEthernet0/2     unassigned      YES unset  down                  down
GigabitEthernet0/3     unassigned      YES unset  administratively down down
Vlan1                  192.168.1.1     YES NVRAM  up                    up
Loopback0              10.0.0.1        YES NVRAM  up                    up
";

    #[test]
    fn test_parse_show_ip_interface_brief_basic() {
        let entries = parse_show_ip_interface_brief(SHOW_IP_IFACE_BRIEF);
        assert_eq!(entries.len(), 5);

        assert_eq!(entries[0].name, "GigabitEthernet0/1");
        assert_eq!(entries[0].ip_address, "unassigned");
        assert_eq!(entries[0].status, "up");
        assert_eq!(entries[0].protocol, "up");

        assert_eq!(entries[1].name, "GigabitEthernet0/2");
        assert_eq!(entries[1].status, "down");
        assert_eq!(entries[1].protocol, "down");

        assert_eq!(entries[2].name, "GigabitEthernet0/3");
        assert_eq!(entries[2].status, "administratively down");
        assert_eq!(entries[2].protocol, "down");

        assert_eq!(entries[3].name, "Vlan1");
        assert_eq!(entries[3].ip_address, "192.168.1.1");
        assert_eq!(entries[3].status, "up");
        assert_eq!(entries[3].protocol, "up");

        assert_eq!(entries[4].name, "Loopback0");
        assert_eq!(entries[4].ip_address, "10.0.0.1");
    }

    const SHOW_IP_IFACE_BRIEF_TE: &str = "\
Interface              IP-Address      OK? Method Status                Protocol
Te1/0/1                unassigned      YES unset  up                    up
GigabitEthernet1/0/1   unassigned      YES unset  up                    up
GigabitEthernet1/0/2   unassigned      YES unset  down                  down
";

    #[test]
    fn test_parse_show_ip_interface_brief_abbreviated() {
        let entries = parse_show_ip_interface_brief(SHOW_IP_IFACE_BRIEF_TE);
        assert_eq!(entries.len(), 3);
        // Abbreviated names pass through as-is
        assert_eq!(entries[0].name, "Te1/0/1");
        assert_eq!(entries[0].status, "up");
        assert_eq!(entries[1].name, "GigabitEthernet1/0/1");
        assert_eq!(entries[2].name, "GigabitEthernet1/0/2");
    }

    const SHOW_IP_IFACE_BRIEF_SUBIF: &str = "\
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet0/0     unassigned      YES unset  up                    up
GigabitEthernet0/0.100 192.168.100.1   YES NVRAM  up                    up
GigabitEthernet0/0.200 192.168.200.1   YES NVRAM  up                    up
";

    #[test]
    fn test_parse_show_ip_interface_brief_subinterfaces() {
        let entries = parse_show_ip_interface_brief(SHOW_IP_IFACE_BRIEF_SUBIF);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "GigabitEthernet0/0");
        assert_eq!(entries[1].name, "GigabitEthernet0/0.100");
        assert_eq!(entries[1].ip_address, "192.168.100.1");
        assert_eq!(entries[2].name, "GigabitEthernet0/0.200");
    }

    // ── show interfaces status ────────────────────────────────────────────────

    const SHOW_INTERFACES_STATUS: &str = concat!(
        "\n",
        "Port      Name               Status       Vlan       Duplex  Speed Type\n",
        "Gi0/1                        connected    1            a-full a-1000 10/100/1000BaseTX\n",
        "Gi0/2     uplink             connected    trunk        a-full a-1000 10/100/1000BaseTX\n",
        "Gi0/3                        notconnect   1              auto   auto 10/100/1000BaseTX\n",
        "Gi0/4                        disabled     1              auto   auto 10/100/1000BaseTX\n",
        "Te0/1                        connected    trunk            full    10G Not Present\n",
    );

    #[test]
    fn test_parse_show_interfaces_status_basic() {
        let entries = parse_show_interfaces_status(SHOW_INTERFACES_STATUS);
        assert_eq!(entries.len(), 5);

        assert_eq!(entries[0].name, "Gi0/1");
        assert_eq!(entries[0].description, "");
        assert_eq!(entries[0].status, "connected");
        assert_eq!(entries[0].vlan, "1");
        assert_eq!(entries[0].duplex, "a-full");
        assert_eq!(entries[0].speed, "a-1000");
        assert_eq!(entries[0].media_type, "10/100/1000BaseTX");

        assert_eq!(entries[1].name, "Gi0/2");
        assert_eq!(entries[1].description, "uplink");
        assert_eq!(entries[1].status, "connected");
        assert_eq!(entries[1].vlan, "trunk");

        assert_eq!(entries[2].name, "Gi0/3");
        assert_eq!(entries[2].status, "notconnect");

        assert_eq!(entries[3].name, "Gi0/4");
        assert_eq!(entries[3].status, "disabled");

        assert_eq!(entries[4].name, "Te0/1");
        assert_eq!(entries[4].media_type, "Not Present");
        assert_eq!(entries[4].vlan, "trunk");
    }

    #[test]
    fn test_parse_show_interfaces_status_empty() {
        let entries = parse_show_interfaces_status("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_show_ip_interface_brief_empty() {
        let entries = parse_show_ip_interface_brief("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_show_inventory_empty() {
        let items = parse_show_inventory("");
        assert!(items.is_empty());
    }

    #[test]
    fn test_parse_show_version_empty() {
        let result = parse_show_version("");
        assert!(result.is_none());
    }
}
