/// Stage 6: Verification / Round-Trip Comparison.
///
/// Normalizes config text and compares original `show running-config` against
/// aycfggen-compiled output. The normalization removes aycfggen-injected markers
/// and bare `!` separator lines so that semantically equivalent configs compare equal.

use regex::Regex;
use std::sync::LazyLock;

static RE_BARE_BANG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^!\s*$").expect("valid regex"));

static RE_CONFIG_ELEMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^! config-element: .+$").expect("valid regex"));

/// Normalize config text for round-trip comparison.
///
/// Removes:
/// 1. Bare `!` separator lines (lines that are just `!` optionally with whitespace)
/// 2. Specific aycfggen-generated comment lines (exact patterns)
/// 3. Trailing whitespace from remaining lines
/// 4. Trailing blank lines
pub fn normalize_for_comparison(text: &str) -> String {
    let aycfggen_markers: &[&str] = &[
        "! PORTS-START",
        "! PORTS-END",
        "! SVI-START",
        "! SVI-END",
    ];

    let aycfggen_prefixes: &[&str] = &[
        "! use <PORTS-CONFIGURATION>",
        "! use <SVI-CONFIGURATION>",
    ];

    let mut lines: Vec<&str> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim_end();

        // 1. Remove bare `!` lines (just `!` optionally followed by whitespace)
        if RE_BARE_BANG.is_match(line) {
            continue;
        }

        // 2a. Remove `! config-element: <name>` lines
        if RE_CONFIG_ELEMENT.is_match(trimmed) {
            continue;
        }

        // 2b. Remove exact aycfggen marker lines
        if aycfggen_markers.contains(&trimmed) {
            continue;
        }

        // 2c. Remove lines that start with specific prefixes
        if aycfggen_prefixes.iter().any(|p| trimmed.starts_with(p)) {
            continue;
        }

        // 3. Strip trailing whitespace
        lines.push(trimmed);
    }

    // 4. Remove trailing blank lines
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }

    let mut result = lines.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

/// Compare original config against compiled output after normalization.
///
/// Returns `Ok(())` if they match after normalization, or `Err` with a unified
/// diff showing the differences.
pub fn verify_round_trip(original: &str, compiled: &str) -> Result<(), String> {
    let norm_original = normalize_for_comparison(original);
    let norm_compiled = normalize_for_comparison(compiled);

    if norm_original == norm_compiled {
        return Ok(());
    }

    let diff = similar::TextDiff::from_lines(&norm_original, &norm_compiled);
    let unified = diff
        .unified_diff()
        .context_radius(3)
        .header("original", "compiled")
        .to_string();

    Err(unified)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test 1: Identical configs ──────────────────────────────────────────────

    #[test]
    fn test_identical_configs_ok() {
        let config = "hostname switch1\nno ip domain-lookup\n";
        assert!(verify_round_trip(config, config).is_ok());
    }

    // ── Test 2: Bare `!` lines stripped ───────────────────────────────────────

    #[test]
    fn test_bare_bang_stripped() {
        let original = "hostname switch1\n!\nno ip domain-lookup\n";
        let compiled = "hostname switch1\nno ip domain-lookup\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    #[test]
    fn test_bare_bang_with_whitespace_stripped() {
        // `!` followed by spaces still counts as a bare separator
        let original = "hostname switch1\n!  \nno ip domain-lookup\n";
        let compiled = "hostname switch1\nno ip domain-lookup\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    // ── Test 3: `! config-element:` line stripped ─────────────────────────────

    #[test]
    fn test_config_element_comment_stripped() {
        let original = "hostname switch1\n";
        let compiled = "! config-element: logging\nhostname switch1\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    // ── Test 4: PORTS-START / PORTS-END / SVI-START / SVI-END stripped ────────

    #[test]
    fn test_ports_markers_stripped() {
        let original = "interface GigabitEthernet0/1\n switchport mode access\n";
        let compiled =
            "! PORTS-START\ninterface GigabitEthernet0/1\n switchport mode access\n! PORTS-END\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    #[test]
    fn test_svi_markers_stripped() {
        let original = "interface Vlan10\n ip address 10.0.0.1 255.255.255.0\n";
        let compiled =
            "! SVI-START\ninterface Vlan10\n ip address 10.0.0.1 255.255.255.0\n! SVI-END\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    // ── Test 5: `! use <PORTS-CONFIGURATION>` / `! use <SVI-CONFIGURATION>` ──

    #[test]
    fn test_use_ports_configuration_stripped() {
        let original = "hostname switch1\n";
        let compiled =
            "hostname switch1\n! use <PORTS-CONFIGURATION> marker at the end of the file\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    #[test]
    fn test_use_svi_configuration_stripped() {
        let original = "hostname switch1\n";
        let compiled =
            "hostname switch1\n! use <SVI-CONFIGURATION> marker at the end of the file\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    // ── Test 6: Regular `! comment` lines PRESERVED ───────────────────────────

    #[test]
    fn test_regular_comment_preserved() {
        let original = "! Access Switch Configuration\nhostname switch1\n";
        let compiled = "! Access Switch Configuration\nhostname switch1\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    #[test]
    fn test_mismatched_regular_comment_is_error() {
        let original = "! Access Switch Configuration\nhostname switch1\n";
        let compiled = "hostname switch1\n";
        assert!(verify_round_trip(original, compiled).is_err());
    }

    // ── Test 7: Trailing whitespace stripped ──────────────────────────────────

    #[test]
    fn test_trailing_whitespace_stripped() {
        let original = "hostname switch1   \n";
        let compiled = "hostname switch1\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    // ── Test 8: Trailing blank lines stripped ─────────────────────────────────

    #[test]
    fn test_trailing_blank_lines_stripped() {
        let original = "hostname switch1\n\n\n";
        let compiled = "hostname switch1\n";
        assert!(verify_round_trip(original, compiled).is_ok());
    }

    // ── Test 9: Different configs after normalization → Err ───────────────────

    #[test]
    fn test_different_configs_error() {
        let original = "hostname switch1\n";
        let compiled = "hostname switch2\n";
        let result = verify_round_trip(original, compiled);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("switch1") || msg.contains("switch2") || msg.contains("line"));
    }

    // ── Test 10: Real-world round-trip example ────────────────────────────────

    #[test]
    fn test_real_world_round_trip() {
        // Simulates original `show run` output
        let original = "\
! Access Switch Configuration
hostname sw-access-01
!
no ip domain-lookup
!
interface GigabitEthernet0/1
 switchport mode access
 switchport access vlan 10
!
interface Vlan10
 ip address 10.0.0.1 255.255.255.0
 no shutdown
";

        // Simulates aycfggen-compiled output with injected markers/comments
        let compiled = "\
! Access Switch Configuration
hostname sw-access-01
no ip domain-lookup
! PORTS-START
! config-element: base-config
interface GigabitEthernet0/1
 switchport mode access
 switchport access vlan 10
! PORTS-END
! SVI-START
interface Vlan10
 ip address 10.0.0.1 255.255.255.0
 no shutdown
! SVI-END
";

        assert!(
            verify_round_trip(original, compiled).is_ok(),
            "round-trip failed: {:?}",
            verify_round_trip(original, compiled)
        );
    }

    // ── normalize_for_comparison unit tests ──────────────────────────────────

    #[test]
    fn test_normalize_empty_string() {
        assert_eq!(normalize_for_comparison(""), "");
    }

    #[test]
    fn test_normalize_only_bare_bangs() {
        assert_eq!(normalize_for_comparison("!\n!\n!"), "");
    }

    #[test]
    fn test_normalize_preserves_content_comment() {
        let input = "! This is a real comment\n";
        assert_eq!(normalize_for_comparison(input), "! This is a real comment\n");
    }
}
