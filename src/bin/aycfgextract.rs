use aycfggen::extract_cli::{ExtractArgs, ResolvedExtractDirs, Target, classify_target, run_extract_offline};
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
            Target::LiveDevice(addr) => {
                eprintln!("error: live device connection to {} is not yet implemented", addr);
                eprintln!("hint: collect command output to a file and use it as an offline target");
                any_error = true;
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
