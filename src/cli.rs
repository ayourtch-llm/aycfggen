use std::path::PathBuf;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "aycfggen", about = "Network device configuration generator")]
pub struct CliArgs {
    /// Root directory containing all subdirectories
    #[arg(long)]
    pub config_root: Option<PathBuf>,

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

    /// Override software images directory
    #[arg(long)]
    pub software_images_dir: Option<PathBuf>,

    /// Override output configs directory
    #[arg(long)]
    pub configs_dir: Option<PathBuf>,

    /// Enable strict validation mode
    #[arg(long)]
    pub strict: bool,

    /// Perform compilation without writing output files
    #[arg(long)]
    pub dry_run: bool,

    /// Write output to stdout with banner format string
    #[arg(long, conflicts_with = "dry_run")]
    pub preview: Option<String>,

    /// Device names to compile (all if none specified)
    pub device_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedDirs {
    pub hardware_templates_dir: PathBuf,
    pub logical_devices_dir: PathBuf,
    pub services_dir: PathBuf,
    pub config_templates_dir: PathBuf,
    pub config_elements_dir: PathBuf,
    pub software_images_dir: PathBuf,
    pub configs_dir: PathBuf,
}

impl ResolvedDirs {
    pub fn from_cli(cli: &CliArgs) -> Self {
        let root = cli.config_root.clone().unwrap_or_else(|| PathBuf::from("."));
        ResolvedDirs {
            hardware_templates_dir: cli.hardware_templates_dir.clone()
                .unwrap_or_else(|| root.join("hardware-templates")),
            logical_devices_dir: cli.logical_devices_dir.clone()
                .unwrap_or_else(|| root.join("logical-devices")),
            services_dir: cli.services_dir.clone()
                .unwrap_or_else(|| root.join("services")),
            config_templates_dir: cli.config_templates_dir.clone()
                .unwrap_or_else(|| root.join("config-templates")),
            config_elements_dir: cli.config_elements_dir.clone()
                .unwrap_or_else(|| root.join("config-elements")),
            software_images_dir: cli.software_images_dir.clone()
                .unwrap_or_else(|| root.join("software-images")),
            configs_dir: cli.configs_dir.clone()
                .unwrap_or_else(|| root.join("configs")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_root() {
        let args = CliArgs::try_parse_from(["aycfggen", "--config-root", "/tmp/test"]).unwrap();
        let dirs = ResolvedDirs::from_cli(&args);
        assert_eq!(dirs.hardware_templates_dir, PathBuf::from("/tmp/test/hardware-templates"));
        assert_eq!(dirs.logical_devices_dir, PathBuf::from("/tmp/test/logical-devices"));
        assert_eq!(dirs.services_dir, PathBuf::from("/tmp/test/services"));
        assert_eq!(dirs.config_templates_dir, PathBuf::from("/tmp/test/config-templates"));
        assert_eq!(dirs.config_elements_dir, PathBuf::from("/tmp/test/config-elements"));
        assert_eq!(dirs.software_images_dir, PathBuf::from("/tmp/test/software-images"));
        assert_eq!(dirs.configs_dir, PathBuf::from("/tmp/test/configs"));
    }

    #[test]
    fn test_parse_per_class_override() {
        let args = CliArgs::try_parse_from([
            "aycfggen",
            "--config-root", "/tmp/test",
            "--services-dir", "/custom/services",
        ]).unwrap();
        let dirs = ResolvedDirs::from_cli(&args);
        assert_eq!(dirs.services_dir, PathBuf::from("/custom/services"));
        assert_eq!(dirs.hardware_templates_dir, PathBuf::from("/tmp/test/hardware-templates"));
        assert_eq!(dirs.logical_devices_dir, PathBuf::from("/tmp/test/logical-devices"));
        assert_eq!(dirs.config_templates_dir, PathBuf::from("/tmp/test/config-templates"));
        assert_eq!(dirs.config_elements_dir, PathBuf::from("/tmp/test/config-elements"));
        assert_eq!(dirs.software_images_dir, PathBuf::from("/tmp/test/software-images"));
        assert_eq!(dirs.configs_dir, PathBuf::from("/tmp/test/configs"));
    }

    #[test]
    fn test_parse_device_names() {
        let args = CliArgs::try_parse_from(["aycfggen", "device1", "device2"]).unwrap();
        assert_eq!(args.device_names, vec!["device1", "device2"]);
    }

    #[test]
    fn test_dry_run_and_preview_conflict() {
        let result = CliArgs::try_parse_from(["aycfggen", "--dry-run", "--preview", "banner"]);
        assert!(result.is_err(), "Expected error due to conflict between --dry-run and --preview");
    }

    #[test]
    fn test_default_root_is_cwd() {
        let args = CliArgs::try_parse_from(["aycfggen"]).unwrap();
        assert!(args.config_root.is_none());
        let dirs = ResolvedDirs::from_cli(&args);
        assert_eq!(dirs.services_dir, PathBuf::from("./services"));
    }

    #[test]
    fn test_additive_overrides() {
        let args = CliArgs::try_parse_from([
            "aycfggen",
            "--services-dir", "/custom/services",
        ]).unwrap();
        let dirs = ResolvedDirs::from_cli(&args);
        assert_eq!(dirs.services_dir, PathBuf::from("/custom/services"));
        assert_eq!(dirs.hardware_templates_dir, PathBuf::from("./hardware-templates"));
        assert_eq!(dirs.logical_devices_dir, PathBuf::from("./logical-devices"));
        assert_eq!(dirs.config_templates_dir, PathBuf::from("./config-templates"));
        assert_eq!(dirs.config_elements_dir, PathBuf::from("./config-elements"));
        assert_eq!(dirs.software_images_dir, PathBuf::from("./software-images"));
        assert_eq!(dirs.configs_dir, PathBuf::from("./configs"));
    }
}
