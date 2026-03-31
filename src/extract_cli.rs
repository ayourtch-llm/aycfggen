use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::Result;

// ─── ExtractArgs ──────────────────────────────────────────────────────────────

#[derive(clap::Parser, Debug)]
#[command(name = "aycfgextract", about = "Network device configuration extractor")]
pub struct ExtractArgs {
    /// Target devices: IPv4/IPv6 addresses or file paths for offline mode
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Root directory for all subdirectories
    #[arg(long, default_value = ".")]
    pub config_root: PathBuf,

    /// Override hardware templates directory
    #[arg(long)]
    pub hardware_templates_dir: Option<PathBuf>,

    /// Override logical devices directory
    #[arg(long)]
    pub logical_devices_dir: Option<PathBuf>,

    /// Override services directory
    #[arg(long)]
    pub services_dir: Option<PathBuf>,

    /// Override config templates directory
    #[arg(long)]
    pub config_templates_dir: Option<PathBuf>,

    /// Override config elements directory
    #[arg(long)]
    pub config_elements_dir: Option<PathBuf>,

    /// Override configs (output) directory
    #[arg(long)]
    pub configs_dir: Option<PathBuf>,

    /// Force recreation of hardware profiles
    #[arg(long)]
    pub recreate_hardware_profiles: bool,

    /// Override save location for collected command output
    #[arg(long)]
    pub save_commands: Option<PathBuf>,
}

// ─── ResolvedExtractDirs ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedExtractDirs {
    pub hardware_templates: PathBuf,
    pub logical_devices: PathBuf,
    pub services: PathBuf,
    pub config_templates: PathBuf,
    pub config_elements: PathBuf,
    pub configs: PathBuf,
}

impl ResolvedExtractDirs {
    pub fn from_args(args: &ExtractArgs) -> Self {
        let root = &args.config_root;
        ResolvedExtractDirs {
            hardware_templates: args.hardware_templates_dir.clone()
                .unwrap_or_else(|| root.join("hardware-templates")),
            logical_devices: args.logical_devices_dir.clone()
                .unwrap_or_else(|| root.join("logical-devices")),
            services: args.services_dir.clone()
                .unwrap_or_else(|| root.join("services")),
            config_templates: args.config_templates_dir.clone()
                .unwrap_or_else(|| root.join("config-templates")),
            config_elements: args.config_elements_dir.clone()
                .unwrap_or_else(|| root.join("config-elements")),
            configs: args.configs_dir.clone()
                .unwrap_or_else(|| root.join("configs")),
        }
    }
}

// ─── Target ───────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum Target {
    LiveDevice(std::net::IpAddr),
    OfflineFile(PathBuf),
}

pub fn classify_target(target: &str) -> Target {
    if let Ok(addr) = target.parse::<std::net::IpAddr>() {
        Target::LiveDevice(addr)
    } else {
        Target::OfflineFile(PathBuf::from(target))
    }
}

// ─── run_extract_offline ──────────────────────────────────────────────────────

/// Run extraction for a single offline target (file path).
/// For live devices, this would connect via SSH — not implemented yet.
pub fn run_extract_offline(
    file_path: &std::path::Path,
    dirs: &ResolvedExtractDirs,
    save_commands_path: Option<&std::path::Path>,
    recreate_hw: bool,
) -> Result<()> {
    use crate::extract::extract_device;
    use crate::fs_sinks::{
        FsHardwareTemplateSink, FsServiceSink, FsConfigTemplateSink,
        FsConfigElementSink, FsLogicalDeviceSink,
    };
    use crate::fs_sources::{FsServiceSource, FsConfigElementSource};
    use crate::sinks::{
        HardwareTemplateSink, ServiceSink, ConfigTemplateSink,
        ConfigElementSink, LogicalDeviceSink,
    };
    use crate::sources::{ServiceSource, ConfigElementSource};
    use std::collections::HashMap;

    // Read the entire command dump
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| anyhow::anyhow!("failed to read command dump {:?}: {}", file_path, e))?;

    // Optionally save the command dump to the specified path
    if let Some(save_path) = save_commands_path {
        if let Some(parent) = save_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("failed to create save directory: {}", e))?;
        }
        std::fs::write(save_path, &content)
            .map_err(|e| anyhow::anyhow!("failed to save command dump to {:?}: {}", save_path, e))?;
    }

    // Split the dump into sections by our markers, or fall back to passing everything.
    let sections = split_command_dump(&content);
    let show_version_output: &str = sections.get("show version").map(String::as_str).unwrap_or(&content);
    let show_inventory_output: &str = sections.get("show inventory").map(String::as_str).unwrap_or(&content);
    let show_ip_brief_output: &str = sections.get("show ip interface brief").map(String::as_str).unwrap_or(&content);
    let show_running_config: &str = sections.get("show running-config").map(String::as_str).unwrap_or(&content);

    // Load existing services from the data store
    let service_source = FsServiceSource::new(dirs.services.clone());
    let mut existing_services: HashMap<String, String> = HashMap::new();
    if dirs.services.exists() {
        let service_names = service_source.list_services()
            .unwrap_or_default();
        for name in &service_names {
            if let Ok(config) = service_source.load_port_config(name) {
                existing_services.insert(name.clone(), config);
            }
        }
    }

    // Load existing config elements from the data store
    let element_source = FsConfigElementSource::new(dirs.config_elements.clone());
    let mut existing_elements: HashMap<String, String> = HashMap::new();
    if dirs.config_elements.exists() {
        let element_names = element_source.list_elements()
            .unwrap_or_default();
        for name in &element_names {
            if let Ok(apply) = element_source.load_apply(name) {
                existing_elements.insert(name.clone(), apply);
            }
        }
    }

    // Run the extraction pipeline
    let output = extract_device(
        show_version_output,
        show_inventory_output,
        show_ip_brief_output,
        show_running_config,
        &existing_services,
        &existing_elements,
    )?;

    // Write hardware templates (skip if already exists and not recreating)
    let hw_sink = FsHardwareTemplateSink::new(dirs.hardware_templates.clone());
    for (sku, template) in &output.hardware_templates {
        let template_dir = dirs.hardware_templates.join(sku);
        if recreate_hw || !template_dir.join("ports.json").exists() {
            hw_sink.write_hardware_template(sku, template)?;
        }
    }

    // Write new services
    let service_sink = FsServiceSink::new(dirs.services.clone());
    for svc in &output.services {
        service_sink.write_port_config(&svc.name, &svc.port_config)?;
    }

    // Write SVI configs for services that have them
    for svi_assignment in &output.svi_assignments {
        service_sink.write_svi_config(&svi_assignment.service_name, &svi_assignment.svi_config)?;
    }

    // Write new config elements
    let element_sink = FsConfigElementSink::new(dirs.config_elements.clone());
    for elem in &output.new_elements {
        element_sink.write_element(&elem.name, &elem.apply_content)?;
    }

    // Write config template
    let template_sink = FsConfigTemplateSink::new(dirs.config_templates.clone());
    template_sink.write_template(&output.template_name, &output.template_content)?;

    // Write logical device config (keyed by serial number)
    let device_sink = FsLogicalDeviceSink::new(dirs.logical_devices.clone());
    device_sink.write_device_config(&output.device.serial_number, &output.device_config)?;

    // Print results
    println!("Extracted device: {} ({})", output.device.hostname, output.device.serial_number);
    println!("  Hardware templates: {}", output.hardware_templates.len());
    println!("  New services: {}", output.services.len());
    println!("  SVI assignments: {}", output.svi_assignments.len());
    println!("  New config elements: {}", output.new_elements.len());
    println!("  Template: {}", output.template_name);
    println!("  Logical device written to: {}/config.json", output.device.serial_number);

    Ok(())
}

// ─── Command dump splitting ──────────────────────────────────────────────────

/// Known show commands we look for when splitting a command dump.
const KNOWN_COMMANDS: &[&str] = &[
    "show version",
    "show inventory",
    "show ip interface brief",
    "show interfaces status",
    "show running-config",
];

/// Split a command dump file into sections.
///
/// Recognizes section boundaries in multiple formats:
/// - `!!! aycfgextract: <command> !!!` (our own marker format)
/// - `hostname#<command>` (IOS enable-mode prompt)
/// - `hostname><command>` (IOS user-mode prompt)
/// - Just `<command>` on a line by itself
///
/// Returns a map of command name → section content.
/// If no markers are found, returns an empty map (caller falls back to full content).
fn split_command_dump(content: &str) -> HashMap<String, String> {
    let mut sections: HashMap<String, String> = HashMap::new();
    let mut current_cmd: Option<String> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        // Try to detect a command line
        if let Some(cmd) = detect_command_line(line) {
            // Flush previous section — only if it has non-empty content.
            // Empty sections occur when the user types partial/cancelled commands
            // (tab completion, typos) that produce consecutive prompt lines.
            if let Some(prev_cmd) = current_cmd.take() {
                let content_text = current_lines.join("\n");
                let has_content = content_text.lines().any(|l| !l.trim().is_empty());
                if has_content {
                    sections.insert(prev_cmd, content_text);
                }
                current_lines.clear();
            }
            current_cmd = Some(cmd);
        } else if current_cmd.is_some() {
            current_lines.push(line);
        }
    }

    // Flush last section
    if let Some(cmd) = current_cmd {
        let content_text = current_lines.join("\n");
        let has_content = content_text.lines().any(|l| !l.trim().is_empty());
        if has_content {
            sections.insert(cmd, content_text);
        }
    }

    sections
}

/// Detect if a line is a command marker/prompt, returning the normalized command name.
fn detect_command_line(line: &str) -> Option<String> {
    let trimmed = line.trim();

    // Format 1: !!! aycfgextract: <command> !!!
    if let Some(rest) = trimmed.strip_prefix("!!! aycfgextract: ") {
        let cmd = rest.strip_suffix(" !!!").unwrap_or(rest);
        return Some(cmd.to_string());
    }

    // Format 2: hostname#command or hostname>command (IOS prompt)
    // Look for a known command after # or >
    for sep in ['#', '>'] {
        if let Some(pos) = trimmed.find(sep) {
            let after_prompt = trimmed[pos + 1..].trim();
            if let Some(cmd) = match_command(after_prompt) {
                return Some(cmd);
            }
        }
    }

    // Format 3: bare command on a line by itself
    if let Some(cmd) = match_command(trimmed) {
        return Some(cmd);
    }

    None
}

/// Match a command string against known commands.
/// Supports both exact match and IOS-style abbreviation (each word is a prefix of the
/// corresponding word in the full command, e.g. "sh ip int bri" matches "show ip interface brief").
fn match_command(input: &str) -> Option<String> {
    let input_lower = input.to_ascii_lowercase();

    // Try exact match first
    for &known in KNOWN_COMMANDS {
        if input_lower == known {
            return Some(known.to_string());
        }
    }

    // Try abbreviation matching: each input word must be a prefix of the corresponding
    // known command word, and all known command words must be accounted for.
    let input_words: Vec<&str> = input_lower.split_whitespace().collect();
    if input_words.is_empty() {
        return None;
    }

    let mut best_match: Option<&str> = None;
    for &known in KNOWN_COMMANDS {
        let known_words: Vec<&str> = known.split_whitespace().collect();
        if input_words.len() != known_words.len() {
            continue;
        }
        let all_match = input_words
            .iter()
            .zip(known_words.iter())
            .all(|(inp, kw)| kw.starts_with(*inp));
        if all_match {
            // Prefer exact match over abbreviation, but accept first abbreviation match
            if best_match.is_none() {
                best_match = Some(known);
            }
        }
    }

    best_match.map(|s| s.to_string())
}

// ─── run_extract_live ────────────────────────────────────────────────────────

/// Commands to execute on the device for full extraction.
const EXTRACTION_COMMANDS: &[&str] = &[
    "show version",
    "show inventory",
    "show ip interface brief",
    "show interfaces status",
    "show running-config",
];

/// Connect to a live device via SSH, collect command output, and run extraction.
///
/// Credentials are read from environment variables:
/// - `AYCFGEXTRACT_SSH_USERNAME`
/// - `AYCFGEXTRACT_SSH_PASSWORD`
pub fn run_extract_live(
    addr: std::net::IpAddr,
    dirs: &ResolvedExtractDirs,
    save_commands_path: Option<&std::path::Path>,
    recreate_hw: bool,
) -> Result<()> {
    let username = std::env::var("AYCFGEXTRACT_SSH_USERNAME")
        .map_err(|_| anyhow::anyhow!("AYCFGEXTRACT_SSH_USERNAME environment variable not set"))?;
    let password = std::env::var("AYCFGEXTRACT_SSH_PASSWORD")
        .map_err(|_| anyhow::anyhow!("AYCFGEXTRACT_SSH_PASSWORD environment variable not set"))?;

    // Build a tokio runtime for the async SSH operations
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| anyhow::anyhow!("failed to create tokio runtime: {}", e))?;

    let collected = rt.block_on(collect_from_device(addr, &username, &password))?;

    // Save the collected output
    let default_save_path;
    let save_path = if let Some(p) = save_commands_path {
        p
    } else {
        // Try to extract hostname from collected output for the default path.
        // Fall back to IP address if hostname can't be determined.
        let hostname = crate::show_parsers::parse_show_version(&collected)
            .map(|v| v.hostname)
            .unwrap_or_else(|| addr.to_string());
        let serial = crate::show_parsers::parse_show_version(&collected)
            .map(|v| v.serial_number)
            .unwrap_or_else(|| "unknown".to_string());
        default_save_path = std::path::PathBuf::from(format!("/tmp/{}-{}-import.txt", hostname, serial));
        &default_save_path
    };

    if let Some(parent) = save_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("failed to create save directory: {}", e))?;
    }
    std::fs::write(save_path, &collected)
        .map_err(|e| anyhow::anyhow!("failed to save command dump to {:?}: {}", save_path, e))?;
    eprintln!("Command output saved to: {}", save_path.display());

    // Now run the offline extraction on the saved file
    run_extract_offline(save_path, dirs, None, recreate_hw)
}

/// Connect to a device via SSH and collect all extraction commands.
async fn collect_from_device(
    addr: std::net::IpAddr,
    username: &str,
    password: &str,
) -> Result<String> {
    use ayclic::{CiscoIosConn, ConnectionType};

    let target = addr.to_string();
    eprintln!("Connecting to {} via SSH...", target);

    let mut conn = CiscoIosConn::new(&target, ConnectionType::Ssh, username, password)
        .await
        .map_err(|e| anyhow::anyhow!("SSH connection to {} failed: {}", target, e))?;

    eprintln!("Connected. Collecting command output...");

    let mut collected = String::new();
    for cmd in EXTRACTION_COMMANDS {
        eprintln!("  Running: {}", cmd);
        let output = conn.run_cmd(cmd)
            .await
            .map_err(|e| anyhow::anyhow!("command '{}' failed on {}: {}", cmd, target, e))?;
        // Write a section header so the file is human-readable
        collected.push_str(&format!("!!! aycfgextract: {} !!!\n", cmd));
        collected.push_str(&output);
        collected.push('\n');
    }

    eprintln!("All commands collected successfully.");
    Ok(collected)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    // ── classify_target ──────────────────────────────────────────────────────

    #[test]
    fn test_classify_target_ipv4() {
        let result = classify_target("192.168.1.1");
        match result {
            Target::LiveDevice(addr) => {
                assert_eq!(addr.to_string(), "192.168.1.1");
            }
            Target::OfflineFile(_) => panic!("Expected LiveDevice for IPv4 address"),
        }
    }

    #[test]
    fn test_classify_target_ipv6() {
        let result = classify_target("2001:db8::1");
        match result {
            Target::LiveDevice(addr) => {
                assert_eq!(addr.to_string(), "2001:db8::1");
            }
            Target::OfflineFile(_) => panic!("Expected LiveDevice for IPv6 address"),
        }
    }

    #[test]
    fn test_classify_target_file_path() {
        let result = classify_target("/tmp/device-dump.txt");
        match result {
            Target::OfflineFile(path) => {
                assert_eq!(path, PathBuf::from("/tmp/device-dump.txt"));
            }
            Target::LiveDevice(_) => panic!("Expected OfflineFile for file path"),
        }
    }

    #[test]
    fn test_classify_target_relative_path() {
        let result = classify_target("dumps/switch1.txt");
        match result {
            Target::OfflineFile(path) => {
                assert_eq!(path, PathBuf::from("dumps/switch1.txt"));
            }
            Target::LiveDevice(_) => panic!("Expected OfflineFile for relative path"),
        }
    }

    #[test]
    fn test_classify_target_hostname_is_offline() {
        // A hostname like "myswitch" is not a valid IP, so it becomes OfflineFile
        let result = classify_target("myswitch");
        match result {
            Target::OfflineFile(path) => {
                assert_eq!(path, PathBuf::from("myswitch"));
            }
            Target::LiveDevice(_) => panic!("Expected OfflineFile for hostname string"),
        }
    }

    // ── ResolvedExtractDirs ──────────────────────────────────────────────────

    #[test]
    fn test_resolved_dirs_default_from_config_root() {
        let args = ExtractArgs::try_parse_from([
            "aycfgextract",
            "--config-root", "/tmp/myroot",
            "192.168.1.1",
        ]).unwrap();
        let dirs = ResolvedExtractDirs::from_args(&args);
        assert_eq!(dirs.hardware_templates, PathBuf::from("/tmp/myroot/hardware-templates"));
        assert_eq!(dirs.logical_devices, PathBuf::from("/tmp/myroot/logical-devices"));
        assert_eq!(dirs.services, PathBuf::from("/tmp/myroot/services"));
        assert_eq!(dirs.config_templates, PathBuf::from("/tmp/myroot/config-templates"));
        assert_eq!(dirs.config_elements, PathBuf::from("/tmp/myroot/config-elements"));
        assert_eq!(dirs.configs, PathBuf::from("/tmp/myroot/configs"));
    }

    #[test]
    fn test_resolved_dirs_per_dir_overrides() {
        let args = ExtractArgs::try_parse_from([
            "aycfgextract",
            "--config-root", "/tmp/myroot",
            "--services-dir", "/custom/services",
            "--configs-dir", "/out/configs",
            "192.168.1.1",
        ]).unwrap();
        let dirs = ResolvedExtractDirs::from_args(&args);
        // Overridden dirs
        assert_eq!(dirs.services, PathBuf::from("/custom/services"));
        assert_eq!(dirs.configs, PathBuf::from("/out/configs"));
        // Non-overridden dirs fall back to config_root
        assert_eq!(dirs.hardware_templates, PathBuf::from("/tmp/myroot/hardware-templates"));
        assert_eq!(dirs.logical_devices, PathBuf::from("/tmp/myroot/logical-devices"));
        assert_eq!(dirs.config_templates, PathBuf::from("/tmp/myroot/config-templates"));
        assert_eq!(dirs.config_elements, PathBuf::from("/tmp/myroot/config-elements"));
    }

    #[test]
    fn test_resolved_dirs_default_root_is_dot() {
        let args = ExtractArgs::try_parse_from([
            "aycfgextract",
            "192.168.1.1",
        ]).unwrap();
        let dirs = ResolvedExtractDirs::from_args(&args);
        assert_eq!(dirs.services, PathBuf::from("./services"));
        assert_eq!(dirs.hardware_templates, PathBuf::from("./hardware-templates"));
    }

    // ── ExtractArgs parsing ──────────────────────────────────────────────────

    #[test]
    fn test_extract_args_targets_required() {
        let result = ExtractArgs::try_parse_from(["aycfgextract"]);
        assert!(result.is_err(), "targets are required");
    }

    #[test]
    fn test_extract_args_multiple_targets() {
        let args = ExtractArgs::try_parse_from([
            "aycfgextract",
            "192.168.1.1",
            "10.0.0.1",
            "/tmp/device.txt",
        ]).unwrap();
        assert_eq!(args.targets.len(), 3);
        assert_eq!(args.targets[0], "192.168.1.1");
        assert_eq!(args.targets[1], "10.0.0.1");
        assert_eq!(args.targets[2], "/tmp/device.txt");
    }

    #[test]
    fn test_extract_args_flags() {
        let args = ExtractArgs::try_parse_from([
            "aycfgextract",
            "--recreate-hardware-profiles",
            "--save-commands", "/tmp/dump.txt",
            "192.168.1.1",
        ]).unwrap();
        assert!(args.recreate_hardware_profiles);
        assert_eq!(args.save_commands, Some(PathBuf::from("/tmp/dump.txt")));
    }

    // ── match_command / detect_command_line ──────────────────────────────────

    #[test]
    fn test_match_command_exact() {
        assert_eq!(match_command("show version"), Some("show version".into()));
        assert_eq!(match_command("show ip interface brief"), Some("show ip interface brief".into()));
    }

    #[test]
    fn test_match_command_abbreviated() {
        assert_eq!(match_command("sh ver"), Some("show version".into()));
        assert_eq!(match_command("show ip int bri"), Some("show ip interface brief".into()));
        assert_eq!(match_command("sh ip int br"), Some("show ip interface brief".into()));
        assert_eq!(match_command("sh inven"), Some("show inventory".into()));
        assert_eq!(match_command("show run"), Some("show running-config".into()));
        assert_eq!(match_command("sh int stat"), Some("show interfaces status".into()));
    }

    #[test]
    fn test_match_command_no_match() {
        assert_eq!(match_command("show"), None);
        assert_eq!(match_command("term len 0"), None);
        assert_eq!(match_command("exit"), None);
        assert_eq!(match_command(""), None);
    }

    #[test]
    fn test_detect_command_line_with_prompt_abbreviated() {
        assert_eq!(
            detect_command_line("SWITCH-01#sh ip int bri"),
            Some("show ip interface brief".into())
        );
        assert_eq!(
            detect_command_line("SWITCH-01#show inven"),
            Some("show inventory".into())
        );
        assert_eq!(
            detect_command_line("SWITCH-01#show ver"),
            Some("show version".into())
        );
    }

    #[test]
    fn test_detect_command_line_partial_word_not_matched() {
        // "show" alone doesn't match any 2-word command
        assert_eq!(detect_command_line("SWITCH#show"), None);
        // "show in" matches "show inventory" (2 words, prefix match)
        assert_eq!(
            detect_command_line("SWITCH#show in"),
            Some("show inventory".into())
        );
    }

    #[test]
    fn test_split_command_dump_with_abbreviated_commands() {
        let dump = "\
SWITCH#term len 0
SWITCH#sh ver
Cisco IOS Software version 15.2
SWITCH#sh inven
NAME: \"1\", DESCR: \"Switch\"
PID: WS-C3560CG , SN: ABC123
SWITCH#sh ip int bri
Interface      IP-Address
Gi0/1          unassigned
SWITCH#sh int stat
Port   Name   Status
Gi0/1         connected
SWITCH#sh run
Building configuration...
hostname SWITCH
!
end
";
        let sections = split_command_dump(dump);
        assert!(sections.contains_key("show version"), "should have show version");
        assert!(sections.contains_key("show inventory"), "should have show inventory");
        assert!(sections.contains_key("show ip interface brief"), "should have show ip interface brief");
        assert!(sections.contains_key("show interfaces status"), "should have show interfaces status");
        assert!(sections.contains_key("show running-config"), "should have show running-config");

        // show ip interface brief should contain the interface table, not the status table
        let ip_brief = sections.get("show ip interface brief").unwrap();
        assert!(ip_brief.contains("IP-Address"), "should contain IP-Address header");
        assert!(!ip_brief.contains("Status"), "should not contain status table header");
    }
}
