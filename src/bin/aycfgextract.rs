use aycfggen::extract_cli::{
    ExtractArgs, ResolvedExtractDirs, Target, classify_target,
    run_extract_offline, run_extract_live,
};
use clap::Parser;
use std::process;

fn run() -> anyhow::Result<()> {
    let args = ExtractArgs::parse();
    let dirs = ResolvedExtractDirs::from_args(&args);

    let mut any_error = false;

    for target_str in &args.targets {
        let target = classify_target(target_str);

        match target {
            Target::OfflineFile(path) => {
                let save_path = args.save_commands.as_deref();
                match run_extract_offline(&path, &dirs, save_path, args.recreate_hardware_profiles) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("error extracting from {:?}: {:#}", path, e);
                        any_error = true;
                    }
                }
            }
            Target::OfflineDir(dir_path) => {
                // Process all files in the directory
                let mut entries: Vec<_> = match std::fs::read_dir(&dir_path) {
                    Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
                    Err(e) => {
                        eprintln!("error reading directory {:?}: {:#}", dir_path, e);
                        any_error = true;
                        continue;
                    }
                };
                entries.sort_by_key(|e| e.file_name());
                for entry in entries {
                    let path = entry.path();
                    if path.is_file() {
                        eprintln!("Processing file: {}", path.display());
                        let save_path = args.save_commands.as_deref();
                        match run_extract_offline(&path, &dirs, save_path, args.recreate_hardware_profiles) {
                            Ok(()) => {}
                            Err(e) => {
                                eprintln!("error extracting from {:?}: {:#}", path, e);
                                any_error = true;
                            }
                        }
                    }
                }
            }
            Target::LiveDevice(addr) => {
                let save_path = args.save_commands.as_deref();
                match run_extract_live(addr, &dirs, save_path, args.recreate_hardware_profiles) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("error extracting from {}: {:#}", addr, e);
                        any_error = true;
                    }
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
