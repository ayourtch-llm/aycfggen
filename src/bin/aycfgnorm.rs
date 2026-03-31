/// aycfgnorm — Normalize a network device configuration for comparison.
///
/// Applies the same normalization used by aycfgextract's round-trip verification:
/// strips bare `!` separator lines, aycfggen markers, trailing whitespace, and
/// trailing blank lines.
///
/// Usage:
///   aycfgnorm <src> <dst>
///
/// Use `-` for stdin or stdout respectively.

use std::io::{self, Read, Write};
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <src> <dst>", args[0]);
        eprintln!("  Use \"-\" for stdin/stdout respectively.");
        process::exit(2);
    }

    let src = &args[1];
    let dst = &args[2];

    // Read input
    let input = if src == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).unwrap_or_else(|e| {
            eprintln!("Error reading stdin: {}", e);
            process::exit(1);
        });
        buf
    } else {
        std::fs::read_to_string(src).unwrap_or_else(|e| {
            eprintln!("Error reading {}: {}", src, e);
            process::exit(1);
        })
    };

    // Normalize
    let output = aycfggen::round_trip::normalize_for_comparison(&input);

    // Write output
    if dst == "-" {
        io::stdout().write_all(output.as_bytes()).unwrap_or_else(|e| {
            eprintln!("Error writing stdout: {}", e);
            process::exit(1);
        });
    } else {
        std::fs::write(dst, &output).unwrap_or_else(|e| {
            eprintln!("Error writing {}: {}", dst, e);
            process::exit(1);
        });
    }
}
