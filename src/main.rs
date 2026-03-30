use aycfggen::cli::{CliArgs, ResolvedDirs};
use aycfggen::compile::compile_device;
use aycfggen::fs_sources::{
    FsHardwareTemplateSource, FsLogicalDeviceSource, FsServiceSource,
    FsConfigTemplateSource, FsConfigElementSource, FsSoftwareImageSource,
};
use aycfggen::output::{write_config, interpolate_banner};
use aycfggen::sources::LogicalDeviceSource;
use clap::Parser;
use std::process;

fn run() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    let dirs = ResolvedDirs::from_cli(&args);

    let hw_source = FsHardwareTemplateSource::new(dirs.hardware_templates_dir.clone());
    let device_source = FsLogicalDeviceSource::new(dirs.logical_devices_dir.clone());
    let service_source = FsServiceSource::new(dirs.services_dir.clone());
    let template_source = FsConfigTemplateSource::new(dirs.config_templates_dir.clone());
    let element_source = FsConfigElementSource::new(dirs.config_elements_dir.clone());
    let image_source = FsSoftwareImageSource::new(dirs.software_images_dir.clone());

    // Determine device list
    let device_names: Vec<String> = if !args.device_names.is_empty() {
        args.device_names.clone()
    } else {
        device_source.list_devices()?
    };

    let mut any_error = false;

    for device_name in &device_names {
        let result = compile_device(
            device_name,
            &device_source,
            &hw_source,
            &service_source,
            &template_source,
            &element_source,
            &image_source,
        );

        match result {
            Err(e) => {
                eprintln!("error compiling device '{}': {:#}", device_name, e);
                any_error = true;
            }
            Ok(content) => {
                if args.dry_run {
                    // Compilation ran for validation; do nothing with the output
                } else if let Some(ref banner_template) = args.preview {
                    // Load device config separately to get role and config_template for banner
                    let device_cfg = device_source.load_device_config(device_name)?;
                    let banner = interpolate_banner(
                        banner_template,
                        device_name,
                        device_cfg.role.as_deref(),
                        &device_cfg.config_template,
                    );
                    println!("{}", banner);
                    print!("{}", content);
                } else {
                    write_config(&dirs.configs_dir, device_name, &content)?;
                }
            }
        }
    }

    if any_error {
        process::exit(1);
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {:#}", e);
        process::exit(1);
    }
}
