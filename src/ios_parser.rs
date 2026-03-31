/// IOS running-config parser.
///
/// Parses a `show running-config` output into structured `ConfigBlock` variants.
/// Preserves the order blocks appear in the original config.

/// Represents a parsed block from an IOS running-config.
#[derive(Debug, PartialEq)]
pub enum ConfigBlock {
    /// Physical port interface block, e.g., `GigabitEthernet0/0`
    PhysicalPort { name: String, lines: Vec<String> },
    /// Sub-interface block, e.g., `GigabitEthernet0/0.100`
    SubInterface { name: String, lines: Vec<String> },
    /// SVI (Switch Virtual Interface) block, e.g., `Vlan10`
    Svi { name: String, vlan: u16, lines: Vec<String> },
    /// Virtual interface block, e.g., `Loopback0`, `Tunnel1`, `Port-channel1`
    VirtualInterface { name: String, lines: Vec<String> },
    /// Non-interface global configuration lines
    GlobalConfig { lines: Vec<String> },
    /// Multi-line construct like banner, crypto pki certificate chain
    MultiLineConstruct { keyword: String, content: String },
}

/// Physical interface name prefixes (without sub-interface dot suffix).
const PHYSICAL_PREFIXES: &[&str] = &[
    "GigabitEthernet",
    "FastEthernet",
    "TenGigabitEthernet",
    "TwentyFiveGigE",
    "FortyGigabitEthernet",
    "HundredGigE",
    "Serial",
    "Ethernet",
];

/// Virtual interface name prefixes.
const VIRTUAL_PREFIXES: &[&str] = &["Loopback", "Tunnel", "Port-channel"];

/// Classify an interface name into the appropriate block type.
fn classify_interface(name: &str, lines: Vec<String>) -> ConfigBlock {
    // SVI: starts with "Vlan" followed by digits
    if name.starts_with("Vlan") {
        let vlan_str = &name["Vlan".len()..];
        if let Ok(vlan) = vlan_str.parse::<u16>() {
            return ConfigBlock::Svi {
                name: name.to_string(),
                vlan,
                lines,
            };
        }
    }

    // Physical + sub-interface check
    for prefix in PHYSICAL_PREFIXES {
        if name.starts_with(prefix) {
            // Sub-interface: contains a dot in the numeric portion after the prefix
            let suffix = &name[prefix.len()..];
            if suffix.contains('.') {
                return ConfigBlock::SubInterface {
                    name: name.to_string(),
                    lines,
                };
            } else {
                return ConfigBlock::PhysicalPort {
                    name: name.to_string(),
                    lines,
                };
            }
        }
    }

    // Virtual interfaces
    for prefix in VIRTUAL_PREFIXES {
        if name.starts_with(prefix) {
            return ConfigBlock::VirtualInterface {
                name: name.to_string(),
                lines,
            };
        }
    }

    // Unknown interface type — treat as virtual by default
    ConfigBlock::VirtualInterface {
        name: name.to_string(),
        lines,
    }
}

/// Parse a `show running-config` string into an ordered list of `ConfigBlock`s.
///
/// `!` separator lines are not included in any block.
pub fn parse_running_config(input: &str) -> Vec<ConfigBlock> {
    let mut blocks: Vec<ConfigBlock> = Vec::new();
    let mut global_lines: Vec<String> = Vec::new();
    let mut iter = input.lines().peekable();

    while let Some(line) = iter.next() {
        // Skip bare `!` separator lines
        if line.trim() == "!" {
            continue;
        }

        // Check for multi-line banner construct
        if let Some(rest) = line.strip_prefix("banner ") {
            // rest is like "motd ^C..." or "login #"
            // The keyword is the word after "banner " and before the delimiter
            let mut parts = rest.splitn(2, ' ');
            let banner_type = parts.next().unwrap_or("").trim();
            let after_type = parts.next().unwrap_or("").trim();

            if !after_type.is_empty() {
                let keyword = format!("banner {}", banner_type);
                let mut content = line.to_string();
                content.push('\n');

                // Determine the delimiter string.
                // IOS uses the first character after the space as the delimiter.
                // Special case: ^C (caret + C) is a common two-char delimiter.
                let delim = if after_type.starts_with("^C") {
                    "^C"
                } else if after_type.starts_with("\x03") {
                    "\x03"
                } else {
                    &after_type[..after_type.chars().next().unwrap().len_utf8()]
                };

                // Check if the closing delimiter is already on this line (after the opening)
                let after_opening = &after_type[delim.len()..];
                if after_opening.contains(delim) {
                    // Single-line banner
                    flush_global(&mut global_lines, &mut blocks);
                    blocks.push(ConfigBlock::MultiLineConstruct { keyword, content });
                    continue;
                }

                // Multi-line: collect until we see a line that is exactly the delimiter
                // or contains the delimiter string
                loop {
                    match iter.next() {
                        None => break,
                        Some(next_line) => {
                            content.push_str(next_line);
                            content.push('\n');
                            if next_line.trim() == delim || next_line.ends_with(delim) {
                                break;
                            }
                        }
                    }
                }

                flush_global(&mut global_lines, &mut blocks);
                blocks.push(ConfigBlock::MultiLineConstruct { keyword, content });
                continue;
            }
            // Fall through if no delimiter found — treat as global
        }

        // Check for crypto pki certificate chain blocks.
        // Collect the entire chain including all sub-certificates until the
        // final un-indented line or end of the chain section.
        if line.starts_with("crypto pki certificate chain ") {
            let keyword = line.trim().to_string();
            let mut content = line.to_string();
            content.push('\n');

            // Collect all indented lines (certificates, data, quit lines)
            // The chain ends when we hit a non-indented line.
            loop {
                match iter.peek() {
                    None => break,
                    Some(next_line) => {
                        if next_line.starts_with(' ') || next_line.trim() == "quit" {
                            content.push_str(iter.next().unwrap());
                            content.push('\n');
                        } else {
                            break;
                        }
                    }
                }
            }

            flush_global(&mut global_lines, &mut blocks);
            blocks.push(ConfigBlock::MultiLineConstruct { keyword, content });
            continue;
        }

        // Check for interface block
        if let Some(iface_name) = line.strip_prefix("interface ") {
            let name = iface_name.trim().to_string();
            let mut iface_lines: Vec<String> = Vec::new();

            // Collect all indented lines belonging to this interface
            while let Some(next_line) = iter.peek() {
                if next_line.starts_with(' ') {
                    iface_lines.push(iter.next().unwrap().to_string());
                } else {
                    break;
                }
            }

            flush_global(&mut global_lines, &mut blocks);
            blocks.push(classify_interface(&name, iface_lines));
            continue;
        }

        // Everything else is global config
        global_lines.push(line.to_string());
    }

    // Flush any remaining global lines
    flush_global(&mut global_lines, &mut blocks);

    blocks
}

/// If `lines` is non-empty, push a `GlobalConfig` block and clear the vec.
fn flush_global(lines: &mut Vec<String>, blocks: &mut Vec<ConfigBlock>) {
    if !lines.is_empty() {
        blocks.push(ConfigBlock::GlobalConfig {
            lines: std::mem::take(lines),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helper predicates
    // -----------------------------------------------------------------------

    fn physical_names(blocks: &[ConfigBlock]) -> Vec<&str> {
        blocks
            .iter()
            .filter_map(|b| match b {
                ConfigBlock::PhysicalPort { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    fn sub_interface_names(blocks: &[ConfigBlock]) -> Vec<&str> {
        blocks
            .iter()
            .filter_map(|b| match b {
                ConfigBlock::SubInterface { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    fn svi_names(blocks: &[ConfigBlock]) -> Vec<&str> {
        blocks
            .iter()
            .filter_map(|b| match b {
                ConfigBlock::Svi { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    fn virtual_names(blocks: &[ConfigBlock]) -> Vec<&str> {
        blocks
            .iter()
            .filter_map(|b| match b {
                ConfigBlock::VirtualInterface { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    fn global_lines_all(blocks: &[ConfigBlock]) -> Vec<&str> {
        blocks
            .iter()
            .flat_map(|b| match b {
                ConfigBlock::GlobalConfig { lines } => lines.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                _ => vec![],
            })
            .collect()
    }

    fn multiline_keywords(blocks: &[ConfigBlock]) -> Vec<&str> {
        blocks
            .iter()
            .filter_map(|b| match b {
                ConfigBlock::MultiLineConstruct { keyword, .. } => Some(keyword.as_str()),
                _ => None,
            })
            .collect()
    }

    fn get_physical_lines<'a>(blocks: &'a [ConfigBlock], name: &str) -> Option<&'a Vec<String>> {
        blocks.iter().find_map(|b| match b {
            ConfigBlock::PhysicalPort { name: n, lines } if n == name => Some(lines),
            _ => None,
        })
    }

    fn get_multiline_content<'a>(blocks: &'a [ConfigBlock], keyword: &str) -> Option<&'a String> {
        blocks.iter().find_map(|b| match b {
            ConfigBlock::MultiLineConstruct { keyword: k, content } if k == keyword => Some(content),
            _ => None,
        })
    }

    // -----------------------------------------------------------------------
    // Test: simple config with physical ports
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_physical_ports() {
        let config = "\
hostname router1
!
interface GigabitEthernet0/0
 ip address 192.168.1.1 255.255.255.0
 no shutdown
!
interface GigabitEthernet0/1
 shutdown
!
end
";
        let blocks = parse_running_config(config);
        let names = physical_names(&blocks);
        assert_eq!(names, vec!["GigabitEthernet0/0", "GigabitEthernet0/1"]);
    }

    #[test]
    fn test_physical_port_lines_captured() {
        let config = "\
interface GigabitEthernet0/0
 ip address 10.0.0.1 255.255.255.0
 no shutdown
!
";
        let blocks = parse_running_config(config);
        let lines = get_physical_lines(&blocks, "GigabitEthernet0/0").expect("block found");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], " ip address 10.0.0.1 255.255.255.0");
        assert_eq!(lines[1], " no shutdown");
    }

    #[test]
    fn test_empty_interface_block() {
        let config = "\
interface GigabitEthernet0/0
!
interface GigabitEthernet0/1
 shutdown
!
";
        let blocks = parse_running_config(config);
        let lines0 = get_physical_lines(&blocks, "GigabitEthernet0/0").expect("block found");
        assert!(lines0.is_empty(), "empty interface should have no lines");

        let lines1 = get_physical_lines(&blocks, "GigabitEthernet0/1").expect("block found");
        assert_eq!(lines1.len(), 1);
        assert_eq!(lines1[0], " shutdown");
    }

    // -----------------------------------------------------------------------
    // Test: SVIs
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_svi_blocks() {
        let config = "\
interface Vlan10
 ip address 10.10.0.1 255.255.255.0
 no shutdown
!
interface Vlan20
 ip address 10.20.0.1 255.255.255.0
!
";
        let blocks = parse_running_config(config);
        let names = svi_names(&blocks);
        assert_eq!(names, vec!["Vlan10", "Vlan20"]);
    }

    #[test]
    fn test_svi_vlan_number_extracted() {
        let config = "\
interface Vlan42
 ip address 192.168.42.1 255.255.255.0
!
";
        let blocks = parse_running_config(config);
        let svi = blocks.iter().find_map(|b| match b {
            ConfigBlock::Svi { vlan, .. } => Some(*vlan),
            _ => None,
        });
        assert_eq!(svi, Some(42));
    }

    // -----------------------------------------------------------------------
    // Test: sub-interfaces
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_sub_interfaces() {
        let config = "\
interface GigabitEthernet0/0.100
 encapsulation dot1Q 100
 ip address 10.1.0.1 255.255.255.0
!
interface GigabitEthernet0/0.200
 encapsulation dot1Q 200
 ip address 10.2.0.1 255.255.255.0
!
";
        let blocks = parse_running_config(config);
        let names = sub_interface_names(&blocks);
        assert_eq!(names, vec!["GigabitEthernet0/0.100", "GigabitEthernet0/0.200"]);
    }

    #[test]
    fn test_sub_interface_not_classified_as_physical() {
        let config = "\
interface GigabitEthernet0/0.100
 encapsulation dot1Q 100
!
";
        let blocks = parse_running_config(config);
        assert!(physical_names(&blocks).is_empty());
        assert_eq!(sub_interface_names(&blocks), vec!["GigabitEthernet0/0.100"]);
    }

    // -----------------------------------------------------------------------
    // Test: virtual interfaces (Loopback, Tunnel, Port-channel)
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_loopback_interface() {
        let config = "\
interface Loopback0
 ip address 1.1.1.1 255.255.255.255
!
";
        let blocks = parse_running_config(config);
        assert_eq!(virtual_names(&blocks), vec!["Loopback0"]);
    }

    #[test]
    fn test_parse_tunnel_interface() {
        let config = "\
interface Tunnel1
 ip address 172.16.0.1 255.255.255.252
 tunnel source GigabitEthernet0/0
 tunnel destination 203.0.113.1
!
";
        let blocks = parse_running_config(config);
        assert_eq!(virtual_names(&blocks), vec!["Tunnel1"]);
    }

    #[test]
    fn test_parse_port_channel_interface() {
        let config = "\
interface Port-channel1
 switchport mode trunk
!
";
        let blocks = parse_running_config(config);
        assert_eq!(virtual_names(&blocks), vec!["Port-channel1"]);
    }

    // -----------------------------------------------------------------------
    // Test: global config
    // -----------------------------------------------------------------------

    #[test]
    fn test_global_config_captured() {
        let config = "\
hostname switch1
!
ip routing
!
";
        let blocks = parse_running_config(config);
        let globals = global_lines_all(&blocks);
        assert!(globals.contains(&"hostname switch1"), "should contain hostname line");
        assert!(globals.contains(&"ip routing"), "should contain ip routing line");
    }

    #[test]
    fn test_bang_lines_not_in_global_config() {
        let config = "\
hostname switch1
!
ip routing
!
";
        let blocks = parse_running_config(config);
        let globals = global_lines_all(&blocks);
        assert!(!globals.contains(&"!"), "! separator must not appear in global config");
    }

    // -----------------------------------------------------------------------
    // Test: banner motd (delimiter-based multi-line construct)
    // -----------------------------------------------------------------------

    #[test]
    fn test_banner_motd_single_line() {
        let config = "banner motd ^Unauthorized access prohibited^\n";
        let blocks = parse_running_config(config);
        let keywords = multiline_keywords(&blocks);
        assert_eq!(keywords, vec!["banner motd"]);
    }

    #[test]
    fn test_banner_motd_multi_line() {
        let config = "\
banner motd ^
Welcome to the network device.
Unauthorized access is prohibited.
^
hostname router1
";
        let blocks = parse_running_config(config);
        let keywords = multiline_keywords(&blocks);
        assert!(keywords.contains(&"banner motd"), "should have banner motd block");
    }

    #[test]
    fn test_banner_motd_content_captured() {
        let config = "\
banner motd ^
Hello World
^
";
        let blocks = parse_running_config(config);
        let content = get_multiline_content(&blocks, "banner motd").expect("banner motd found");
        assert!(content.contains("Hello World"), "banner content should be captured");
    }

    #[test]
    fn test_banner_login_recognized() {
        let config = "\
banner login ^
Login banner text.
^
";
        let blocks = parse_running_config(config);
        let keywords = multiline_keywords(&blocks);
        assert!(keywords.contains(&"banner login"));
    }

    #[test]
    fn test_banner_exec_recognized() {
        let config = "\
banner exec ^
Exec banner text.
^
";
        let blocks = parse_running_config(config);
        let keywords = multiline_keywords(&blocks);
        assert!(keywords.contains(&"banner exec"));
    }

    // -----------------------------------------------------------------------
    // Test: crypto PKI certificate chain
    // -----------------------------------------------------------------------

    #[test]
    fn test_crypto_pki_certificate_chain() {
        let config = "\
crypto pki certificate chain SomeCA
 certificate 01
  AABBCCDDEEFF
  quit
!
hostname router1
";
        let blocks = parse_running_config(config);
        let keywords = multiline_keywords(&blocks);
        assert!(
            keywords.contains(&"crypto pki certificate chain SomeCA"),
            "should have crypto pki block; got: {:?}",
            keywords
        );
    }

    #[test]
    fn test_crypto_pki_content_captured() {
        let config = "\
crypto pki certificate chain MyCA
 certificate ca
  DEADBEEF0123
  quit
!
";
        let blocks = parse_running_config(config);
        let content = get_multiline_content(&blocks, "crypto pki certificate chain MyCA")
            .expect("crypto pki block found");
        assert!(content.contains("DEADBEEF0123"), "certificate data should be captured");
        assert!(content.contains("quit"), "quit should be in content");
    }

    // -----------------------------------------------------------------------
    // Test: ordering preserved
    // -----------------------------------------------------------------------

    #[test]
    fn test_block_ordering_preserved() {
        let config = "\
hostname router1
!
interface GigabitEthernet0/0
 ip address 10.0.0.1 255.255.255.0
!
interface Loopback0
 ip address 1.1.1.1 255.255.255.255
!
interface Vlan10
 ip address 192.168.10.1 255.255.255.0
!
ip route 0.0.0.0 0.0.0.0 10.0.0.254
!
";
        let blocks = parse_running_config(config);

        // Verify ordering: GlobalConfig (hostname), PhysicalPort, VirtualInterface, Svi, GlobalConfig
        let type_sequence: Vec<&str> = blocks
            .iter()
            .map(|b| match b {
                ConfigBlock::GlobalConfig { .. } => "global",
                ConfigBlock::PhysicalPort { .. } => "physical",
                ConfigBlock::SubInterface { .. } => "sub",
                ConfigBlock::Svi { .. } => "svi",
                ConfigBlock::VirtualInterface { .. } => "virtual",
                ConfigBlock::MultiLineConstruct { .. } => "multiline",
            })
            .collect();

        assert_eq!(
            type_sequence,
            vec!["global", "physical", "virtual", "svi", "global"],
            "block ordering must match original config: {:?}",
            type_sequence
        );
    }

    // -----------------------------------------------------------------------
    // Test: mixed config (all block types)
    // -----------------------------------------------------------------------

    #[test]
    fn test_mixed_config_all_block_types() {
        let config = "\
version 15.2
!
hostname switch1
!
banner motd ^
Welcome.
^
!
interface FastEthernet0/0
 switchport mode access
 switchport access vlan 10
!
interface GigabitEthernet0/0.100
 encapsulation dot1Q 100
!
interface Vlan10
 ip address 10.10.10.1 255.255.255.0
!
interface Loopback0
 ip address 1.1.1.1 255.255.255.255
!
interface Tunnel1
 tunnel source GigabitEthernet0/0
!
interface Port-channel1
 switchport mode trunk
!
crypto pki certificate chain TP-self-signed
 certificate self-signed 01
  CAFEBABE
  quit
!
ip route 0.0.0.0 0.0.0.0 10.0.0.1
!
end
";
        let blocks = parse_running_config(config);

        assert!(!physical_names(&blocks).is_empty(), "should have physical ports");
        assert!(!sub_interface_names(&blocks).is_empty(), "should have sub-interfaces");
        assert!(!svi_names(&blocks).is_empty(), "should have SVIs");
        assert!(!virtual_names(&blocks).is_empty(), "should have virtual interfaces");
        assert!(!global_lines_all(&blocks).is_empty(), "should have global config");
        assert!(!multiline_keywords(&blocks).is_empty(), "should have multi-line constructs");

        // Check specific items
        assert!(physical_names(&blocks).contains(&"FastEthernet0/0"));
        assert!(sub_interface_names(&blocks).contains(&"GigabitEthernet0/0.100"));
        assert!(svi_names(&blocks).contains(&"Vlan10"));

        let virtuals = virtual_names(&blocks);
        assert!(virtuals.contains(&"Loopback0"));
        assert!(virtuals.contains(&"Tunnel1"));
        assert!(virtuals.contains(&"Port-channel1"));
    }

    // -----------------------------------------------------------------------
    // Test: various physical interface types
    // -----------------------------------------------------------------------

    #[test]
    fn test_ten_gig_interface_classified_as_physical() {
        let config = "\
interface TenGigabitEthernet1/0/1
 description Uplink
!
";
        let blocks = parse_running_config(config);
        assert_eq!(physical_names(&blocks), vec!["TenGigabitEthernet1/0/1"]);
    }

    #[test]
    fn test_hundred_gig_interface_classified_as_physical() {
        let config = "\
interface HundredGigE1/0/1
 description Core Uplink
!
";
        let blocks = parse_running_config(config);
        assert_eq!(physical_names(&blocks), vec!["HundredGigE1/0/1"]);
    }

    #[test]
    fn test_serial_interface_classified_as_physical() {
        let config = "\
interface Serial0/0/0
 encapsulation hdlc
!
";
        let blocks = parse_running_config(config);
        assert_eq!(physical_names(&blocks), vec!["Serial0/0/0"]);
    }

    #[test]
    fn test_twenty_five_gig_interface_classified_as_physical() {
        let config = "\
interface TwentyFiveGigE1/0/1
 description 25G Uplink
!
";
        let blocks = parse_running_config(config);
        assert_eq!(physical_names(&blocks), vec!["TwentyFiveGigE1/0/1"]);
    }

    #[test]
    fn test_forty_gig_interface_classified_as_physical() {
        let config = "\
interface FortyGigabitEthernet1/0/1
 description 40G Uplink
!
";
        let blocks = parse_running_config(config);
        assert_eq!(physical_names(&blocks), vec!["FortyGigabitEthernet1/0/1"]);
    }

    // -----------------------------------------------------------------------
    // Test: interface with only shutdown
    // -----------------------------------------------------------------------

    #[test]
    fn test_interface_with_only_shutdown() {
        let config = "\
interface GigabitEthernet0/5
 shutdown
!
";
        let blocks = parse_running_config(config);
        let names = physical_names(&blocks);
        assert_eq!(names, vec!["GigabitEthernet0/5"]);
        let lines = get_physical_lines(&blocks, "GigabitEthernet0/5").expect("block found");
        assert_eq!(lines, &vec![" shutdown".to_string()]);
    }

    // -----------------------------------------------------------------------
    // Test: multi-module device interface names
    // -----------------------------------------------------------------------

    #[test]
    fn test_multi_module_physical_port() {
        let config = "\
interface GigabitEthernet1/0/3
 description Server Port
 switchport mode access
!
interface GigabitEthernet2/0/1
 description Uplink
 switchport mode trunk
!
";
        let blocks = parse_running_config(config);
        let names = physical_names(&blocks);
        assert_eq!(names, vec!["GigabitEthernet1/0/3", "GigabitEthernet2/0/1"]);
    }

    // -----------------------------------------------------------------------
    // Test: banner with ^C delimiter (most common IOS delimiter)
    // -----------------------------------------------------------------------

    #[test]
    fn test_banner_motd_caret_c_delimiter() {
        let config = "\
banner motd ^C
Unauthorized access prohibited.
Contact admin@example.com for access.
^C
hostname router1
";
        let blocks = parse_running_config(config);
        let keywords = multiline_keywords(&blocks);
        assert!(keywords.contains(&"banner motd"), "should have banner motd block");
        let content = get_multiline_content(&blocks, "banner motd").expect("banner motd found");
        assert!(content.contains("Unauthorized access prohibited."), "banner body captured");
        assert!(content.contains("Contact admin@example.com"), "full banner body captured");
        // Verify router1 is in global config, not in the banner
        let globals = global_lines_all(&blocks);
        assert!(globals.contains(&"hostname router1"), "hostname should be global, not in banner");
    }

    #[test]
    fn test_banner_motd_hash_delimiter() {
        let config = "\
banner motd #
Welcome to the network.
#
";
        let blocks = parse_running_config(config);
        let content = get_multiline_content(&blocks, "banner motd").expect("banner found");
        assert!(content.contains("Welcome to the network."));
    }

    #[test]
    fn test_banner_motd_caret_c_single_line() {
        let config = "banner motd ^CNo access^C\n";
        let blocks = parse_running_config(config);
        let keywords = multiline_keywords(&blocks);
        assert_eq!(keywords, vec!["banner motd"]);
    }

    // -----------------------------------------------------------------------
    // Test: multi-cert chain (multiple quit blocks)
    // -----------------------------------------------------------------------

    #[test]
    fn test_crypto_pki_multi_certificate_chain() {
        let config = "\
crypto pki certificate chain MyCA
 certificate ca 01
  AABBCCDD
  quit
 certificate 02
  EEFF0011
  quit
!
hostname router1
";
        let blocks = parse_running_config(config);
        let content = get_multiline_content(&blocks, "crypto pki certificate chain MyCA")
            .expect("crypto pki block found");
        assert!(content.contains("AABBCCDD"), "first cert data captured");
        assert!(content.contains("EEFF0011"), "second cert data captured");
        assert!(content.contains("certificate 02"), "second certificate header captured");
        // Verify hostname is not consumed by the cert block
        let globals = global_lines_all(&blocks);
        assert!(globals.contains(&"hostname router1"), "hostname should be global");
    }

    // -----------------------------------------------------------------------
    // Test: interface blocks without ! separator
    // -----------------------------------------------------------------------

    #[test]
    fn test_interfaces_without_bang_separator() {
        let config = "\
interface GigabitEthernet0/0
 ip address 10.0.0.1 255.255.255.0
interface GigabitEthernet0/1
 ip address 10.0.1.1 255.255.255.0
";
        let blocks = parse_running_config(config);
        let names = physical_names(&blocks);
        assert_eq!(names, vec!["GigabitEthernet0/0", "GigabitEthernet0/1"]);
    }

    // -----------------------------------------------------------------------
    // Test: unknown interface type falls back to VirtualInterface
    // -----------------------------------------------------------------------

    #[test]
    fn test_unknown_interface_type_fallback() {
        let config = "\
interface BDI1
 ip address 10.0.0.1 255.255.255.0
!
";
        let blocks = parse_running_config(config);
        assert_eq!(virtual_names(&blocks), vec!["BDI1"]);
    }

    // -----------------------------------------------------------------------
    // Test: Serial sub-interface
    // -----------------------------------------------------------------------

    #[test]
    fn test_serial_sub_interface() {
        let config = "\
interface Serial0/0/0.1
 encapsulation ppp
!
";
        let blocks = parse_running_config(config);
        assert_eq!(sub_interface_names(&blocks), vec!["Serial0/0/0.1"]);
    }
}
