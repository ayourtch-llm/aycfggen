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

    // Pass the entire file content to each parser (they skip unrecognized lines)
    let show_version_output = content.as_str();
    let show_inventory_output = content.as_str();
    let show_ip_brief_output = content.as_str();
    let show_running_config = content.as_str();

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
}
