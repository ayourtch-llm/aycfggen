/// Stage 4: Global Configuration & Config Template Builder.
///
/// Builds a config template from global config sections, matching existing config
/// elements with longest-match-first priority. Bare `!` separator lines are
/// ignored during matching. PORTS and SVI markers are inserted at the correct
/// positions. Unmatched lines are kept as literal text.

use std::collections::HashMap;

// ─── Public types ─────────────────────────────────────────────────────────────

/// A section of global configuration passed to build_template.
#[derive(Debug, Clone, PartialEq)]
pub enum GlobalSection {
    /// Global config lines (non-interface, non-multiline).
    Config(Vec<String>),
    /// A virtual interface block: `(name, lines)`.
    VirtualInterface(String, Vec<String>),
    /// A multi-line construct: `(keyword, raw_content)`.
    MultiLine(String, String),
    /// Marker for where `<PORTS-CONFIGURATION>` should appear.
    PortsMarker,
    /// Marker for where `<SVI-CONFIGURATION>` should appear.
    SviMarker,
}

/// A new config element that should be created (was not previously in the store).
#[derive(Debug, Clone, PartialEq)]
pub struct NewConfigElement {
    /// Element name (directory name).
    pub name: String,
    /// Content for `apply.txt`.
    pub apply_content: String,
}

/// The result of building a config template.
#[derive(Debug, Clone)]
pub struct TemplateResult {
    /// The rendered template content: a mix of `!!!###<name>` markers and literal lines.
    pub template_content: String,
    /// New config element directories to create (were not already in the store).
    pub new_elements: Vec<NewConfigElement>,
}

// ─── Public entry point ───────────────────────────────────────────────────────

/// Build a config template from global config sections.
///
/// # Parameters
/// - `global_sections` — ordered sections of global configuration.
/// - `existing_elements` — `element_name -> apply.txt content` map from the data store.
/// - `first_port_position` — reserved for future use (marker placement driven by
///   `GlobalSection::PortsMarker` in the section list).
/// - `first_svi_position` — reserved for future use (marker placement driven by
///   `GlobalSection::SviMarker` in the section list).
pub fn build_template(
    global_sections: &[GlobalSection],
    existing_elements: &HashMap<String, String>,
    first_port_position: Option<usize>,
    first_svi_position: Option<usize>,
) -> TemplateResult {
    let _ = first_port_position; // driven via PortsMarker sections
    let _ = first_svi_position;  // driven via SviMarker sections

    // ── Step 1: Build (element_name, apply_lines) sorted by non-bang line count
    //    descending (longest-match-first).
    let mut sorted_elements: Vec<(&str, Vec<String>)> = existing_elements
        .iter()
        .map(|(name, content)| {
            let apply_lines: Vec<String> = content
                .lines()
                .filter(|l| !is_bare_bang(l))
                .map(|l| l.to_string())
                .collect();
            (name.as_str(), apply_lines)
        })
        .collect();
    sorted_elements.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then(a.0.cmp(b.0))
    });

    // ── Step 2: Build a flat annotated line pool from Config sections only.
    //    Non-Config sections insert a "SectionBoundary" sentinel in the flat pool
    //    so that element matching cannot span across them.
    let mut flat_lines: Vec<FlatLine> = Vec::new();
    let mut section_map: Vec<SectionEntry> = Vec::new();

    for section in global_sections {
        match section {
            GlobalSection::Config(lines) => {
                let start = flat_lines.len();
                for line in lines {
                    flat_lines.push(FlatLine {
                        text: line.clone(),
                        consumed_by: None,
                        is_boundary: false,
                    });
                }
                section_map.push(SectionEntry::Config {
                    flat_start: start,
                    flat_end: flat_lines.len(),
                });
            }
            GlobalSection::VirtualInterface(name, lines) => {
                // Insert a boundary sentinel to block cross-section matches.
                flat_lines.push(FlatLine {
                    text: String::new(),
                    consumed_by: None,
                    is_boundary: true,
                });
                section_map.push(SectionEntry::VirtualInterface(name.clone(), lines.clone()));
            }
            GlobalSection::MultiLine(kw, content) => {
                flat_lines.push(FlatLine {
                    text: String::new(),
                    consumed_by: None,
                    is_boundary: true,
                });
                section_map.push(SectionEntry::MultiLine(kw.clone(), content.clone()));
            }
            GlobalSection::PortsMarker => {
                flat_lines.push(FlatLine {
                    text: String::new(),
                    consumed_by: None,
                    is_boundary: true,
                });
                section_map.push(SectionEntry::PortsMarker);
            }
            GlobalSection::SviMarker => {
                flat_lines.push(FlatLine {
                    text: String::new(),
                    consumed_by: None,
                    is_boundary: true,
                });
                section_map.push(SectionEntry::SviMarker);
            }
        }
    }

    // ── Step 3: Match elements against the flat pool (longest-match-first).
    for (elem_name, apply_lines) in &sorted_elements {
        if apply_lines.is_empty() {
            continue;
        }
        if let Some((start, end)) = find_match_lines(&flat_lines, apply_lines) {
            for idx in start..=end {
                flat_lines[idx].consumed_by = Some(elem_name.to_string());
            }
        }
    }

    // ── Step 4: Render the template by walking the section map.
    let mut output = String::new();

    for entry in &section_map {
        match entry {
            SectionEntry::Config { flat_start, flat_end } => {
                let lines = &flat_lines[*flat_start..*flat_end];
                render_config_lines(lines, &mut output);
            }
            SectionEntry::VirtualInterface(name, iface_lines) => {
                output.push_str(&format!("interface {}\n", name));
                for l in iface_lines {
                    output.push_str(l);
                    output.push('\n');
                }
            }
            SectionEntry::MultiLine(_kw, content) => {
                output.push_str(content);
                // content already ends with \n from the parser
            }
            SectionEntry::PortsMarker => {
                output.push_str("<PORTS-CONFIGURATION>\n");
            }
            SectionEntry::SviMarker => {
                output.push_str("<SVI-CONFIGURATION>\n");
            }
        }
    }

    TemplateResult {
        template_content: output,
        new_elements: vec![],
    }
}

// ─── Internal types ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct FlatLine {
    text: String,
    /// None = unconsumed; Some(name) = consumed by this element.
    consumed_by: Option<String>,
    /// True for sentinel lines inserted at section boundaries.
    is_boundary: bool,
}

#[derive(Debug)]
enum SectionEntry {
    Config { flat_start: usize, flat_end: usize },
    VirtualInterface(String, Vec<String>),
    MultiLine(String, String),
    PortsMarker,
    SviMarker,
}

// ─── Matching helpers ─────────────────────────────────────────────────────────

/// Find the first contiguous match of `pattern` (sequence of non-bang lines)
/// in `flat_lines`.
///
/// Rules:
/// - Bare `!` lines (non-boundary) are transparent: skipped during matching.
/// - Boundary sentinels block matching (a match cannot cross a boundary).
/// - Already-consumed lines block matching.
///
/// Returns `Some((start_idx, end_idx))` — inclusive range in `flat_lines` covering
/// the matched span (including any bare `!` lines between matched lines).
fn find_match_lines(flat_lines: &[FlatLine], pattern: &[String]) -> Option<(usize, usize)> {
    if pattern.is_empty() {
        return None;
    }
    let n = flat_lines.len();
    let pat_len = pattern.len();

    'outer: for start in 0..n {
        let line = &flat_lines[start];
        // The starting line must be an unconsumed, non-boundary, non-bang line
        // that matches pattern[0].
        if line.is_boundary || is_bare_bang(&line.text) || line.consumed_by.is_some() {
            continue;
        }
        if line.text != pattern[0] {
            continue;
        }

        // Found a match for pattern[0]. Now try to match the rest.
        let mut pat_idx = 1;
        let mut pos = start + 1;
        let mut end = start;

        while pat_idx < pat_len {
            if pos >= n {
                continue 'outer;
            }
            let cur = &flat_lines[pos];
            if cur.is_boundary {
                // Cannot cross a section boundary.
                continue 'outer;
            }
            if is_bare_bang(&cur.text) {
                // Transparent: skip but continue.
                pos += 1;
                continue;
            }
            if cur.consumed_by.is_some() {
                // Consumed line blocks this match.
                continue 'outer;
            }
            if cur.text != pattern[pat_idx] {
                continue 'outer;
            }
            end = pos;
            pat_idx += 1;
            pos += 1;
        }

        if pat_idx == pat_len {
            if pat_len == 1 {
                end = start;
            }
            return Some((start, end));
        }
    }

    None
}

fn is_bare_bang(line: &str) -> bool {
    line.trim() == "!"
}

/// Render a slice of flat lines (from a Config section) into the template output.
///
/// - Unconsumed lines → emitted verbatim.
/// - Bare `!` lines → dropped (they were transparent during matching).
/// - Runs of lines consumed by the same element → single `!!!###name` marker.
fn render_config_lines(lines: &[FlatLine], output: &mut String) {
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];

        // Drop bare `!` separators.
        if is_bare_bang(&line.text) {
            i += 1;
            continue;
        }

        match &line.consumed_by {
            None => {
                output.push_str(&line.text);
                output.push('\n');
                i += 1;
            }
            Some(elem_name) => {
                // Emit a single marker for this element.
                output.push_str(&format!("!!!###{}\n", elem_name));
                let name_clone = elem_name.clone();
                // Skip all subsequent lines consumed by the same element
                // (and any bare `!` lines in between).
                i += 1;
                while i < lines.len() {
                    if is_bare_bang(&lines[i].text) {
                        i += 1;
                        continue;
                    }
                    if lines[i].consumed_by.as_deref() == Some(&name_clone) {
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn no_elements() -> HashMap<String, String> {
        HashMap::new()
    }

    fn elements(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // -------------------------------------------------------------------------
    // Test 1: simple global config, no existing elements — all literal text
    // -------------------------------------------------------------------------

    #[test]
    fn test_no_elements_all_literal() {
        let sections = vec![GlobalSection::Config(vec![
            "hostname switch1".to_string(),
            "ip routing".to_string(),
        ])];

        let result = build_template(&sections, &no_elements(), None, None);

        assert!(result.template_content.contains("hostname switch1"));
        assert!(result.template_content.contains("ip routing"));
        assert!(!result.template_content.contains("!!!###"));
    }

    // -------------------------------------------------------------------------
    // Test 2: global config matching one existing element → marker replaces it
    // -------------------------------------------------------------------------

    #[test]
    fn test_one_element_matched() {
        let sections = vec![GlobalSection::Config(vec![
            "hostname switch1".to_string(),
            "ntp server 1.2.3.4".to_string(),
            "ntp server 5.6.7.8".to_string(),
            "ip routing".to_string(),
        ])];

        let elems = elements(&[("ntp-config", "ntp server 1.2.3.4\nntp server 5.6.7.8\n")]);

        let result = build_template(&sections, &elems, None, None);

        assert!(result.template_content.contains("hostname switch1"));
        assert!(result.template_content.contains("!!!###ntp-config"));
        assert!(result.template_content.contains("ip routing"));
        // The matched lines should not appear as literal text
        assert!(!result.template_content.contains("ntp server 1.2.3.4"));
    }

    // -------------------------------------------------------------------------
    // Test 3: two elements, longest-match-first priority
    // -------------------------------------------------------------------------

    #[test]
    fn test_longest_match_wins() {
        let sections = vec![GlobalSection::Config(vec![
            "snmp-server community public RO".to_string(),
            "snmp-server community secret RW".to_string(),
            "snmp-server location DC1".to_string(),
        ])];

        // "snmp-full" matches all 3 lines; "snmp-basic" matches only 2.
        let elems = elements(&[
            (
                "snmp-full",
                "snmp-server community public RO\nsnmp-server community secret RW\nsnmp-server location DC1\n",
            ),
            (
                "snmp-basic",
                "snmp-server community public RO\nsnmp-server community secret RW\n",
            ),
        ]);

        let result = build_template(&sections, &elems, None, None);

        assert!(result.template_content.contains("!!!###snmp-full"));
        assert!(!result.template_content.contains("!!!###snmp-basic"));
        assert!(!result.template_content.contains("snmp-server community public RO"));
    }

    // -------------------------------------------------------------------------
    // Test 4: element content with bare `!` separators in between (ignored)
    // -------------------------------------------------------------------------

    #[test]
    fn test_bang_lines_ignored_during_matching() {
        let sections = vec![GlobalSection::Config(vec![
            "hostname router1".to_string(),
            "ntp server 10.0.0.1".to_string(),
            "!".to_string(),
            "ntp server 10.0.0.2".to_string(),
            "ip routing".to_string(),
        ])];

        // The apply.txt does NOT have the `!` separator.
        let elems = elements(&[("ntp-servers", "ntp server 10.0.0.1\nntp server 10.0.0.2\n")]);

        let result = build_template(&sections, &elems, None, None);

        assert!(result.template_content.contains("!!!###ntp-servers"));
        assert!(!result.template_content.contains("ntp server 10.0.0.1"));
        assert!(!result.template_content.contains("ntp server 10.0.0.2"));
        assert!(result.template_content.contains("hostname router1"));
        assert!(result.template_content.contains("ip routing"));
    }

    // -------------------------------------------------------------------------
    // Test 5: PORTS and SVI markers at correct positions
    // -------------------------------------------------------------------------

    #[test]
    fn test_ports_and_svi_markers_present() {
        let sections = vec![
            GlobalSection::Config(vec!["hostname switch1".to_string()]),
            GlobalSection::PortsMarker,
            GlobalSection::SviMarker,
            GlobalSection::Config(vec!["ip routing".to_string()]),
        ];

        let result = build_template(&sections, &no_elements(), Some(1), Some(2));

        assert!(result.template_content.contains("<PORTS-CONFIGURATION>"));
        assert!(result.template_content.contains("<SVI-CONFIGURATION>"));
        let content = &result.template_content;
        let hostname_pos = content.find("hostname switch1").unwrap();
        let ports_pos = content.find("<PORTS-CONFIGURATION>").unwrap();
        let svi_pos = content.find("<SVI-CONFIGURATION>").unwrap();
        let routing_pos = content.find("ip routing").unwrap();
        assert!(hostname_pos < ports_pos);
        assert!(ports_pos < svi_pos);
        assert!(svi_pos < routing_pos);
    }

    // -------------------------------------------------------------------------
    // Test 6: no ports/SVIs → markers omitted
    // -------------------------------------------------------------------------

    #[test]
    fn test_no_markers_when_no_ports_or_svis() {
        let sections = vec![GlobalSection::Config(vec![
            "hostname switch1".to_string(),
            "ip routing".to_string(),
        ])];

        let result = build_template(&sections, &no_elements(), None, None);

        assert!(!result.template_content.contains("<PORTS-CONFIGURATION>"));
        assert!(!result.template_content.contains("<SVI-CONFIGURATION>"));
    }

    // -------------------------------------------------------------------------
    // Test 7: virtual interface blocks preserved as literal text
    // -------------------------------------------------------------------------

    #[test]
    fn test_virtual_interface_preserved() {
        let sections = vec![
            GlobalSection::Config(vec!["hostname router1".to_string()]),
            GlobalSection::VirtualInterface(
                "Loopback0".to_string(),
                vec![" ip address 1.1.1.1 255.255.255.255".to_string()],
            ),
            GlobalSection::Config(vec!["ip routing".to_string()]),
        ];

        let result = build_template(&sections, &no_elements(), None, None);

        assert!(result.template_content.contains("interface Loopback0"));
        assert!(result.template_content.contains(" ip address 1.1.1.1 255.255.255.255"));
    }

    // -------------------------------------------------------------------------
    // Test 8: multi-line constructs preserved as literal text
    // -------------------------------------------------------------------------

    #[test]
    fn test_multiline_construct_preserved() {
        let sections = vec![
            GlobalSection::Config(vec!["hostname router1".to_string()]),
            GlobalSection::MultiLine(
                "banner motd".to_string(),
                "banner motd ^\nWelcome.\n^\n".to_string(),
            ),
            GlobalSection::Config(vec!["ip routing".to_string()]),
        ];

        let result = build_template(&sections, &no_elements(), None, None);

        assert!(result.template_content.contains("banner motd ^"));
        assert!(result.template_content.contains("Welcome."));
    }

    // -------------------------------------------------------------------------
    // Test 9: unmatched element lines stay as literal text
    // -------------------------------------------------------------------------

    #[test]
    fn test_unmatched_lines_stay_literal() {
        let sections = vec![GlobalSection::Config(vec![
            "hostname switch1".to_string(),
            "some-unique-config line".to_string(),
        ])];

        let elems = elements(&[("some-element", "ntp server 1.2.3.4\n")]);

        let result = build_template(&sections, &elems, None, None);

        assert!(result.template_content.contains("hostname switch1"));
        assert!(result.template_content.contains("some-unique-config line"));
        assert!(!result.template_content.contains("!!!###some-element"));
    }

    // -------------------------------------------------------------------------
    // Test 10: element with single line matched
    // -------------------------------------------------------------------------

    #[test]
    fn test_single_line_element_matched() {
        let sections = vec![GlobalSection::Config(vec![
            "hostname switch1".to_string(),
            "ip routing".to_string(),
        ])];

        let elems = elements(&[("routing-enable", "ip routing\n")]);

        let result = build_template(&sections, &elems, None, None);

        assert!(result.template_content.contains("!!!###routing-enable"));
        assert!(!result.template_content.contains("\nip routing\n"));
        assert!(result.template_content.contains("hostname switch1"));
    }

    // -------------------------------------------------------------------------
    // Test 11: element in the middle of a Config section
    // -------------------------------------------------------------------------

    #[test]
    fn test_element_in_middle_of_config() {
        let sections = vec![GlobalSection::Config(vec![
            "hostname switch1".to_string(),
            "logging host 10.0.0.5".to_string(),
            "logging trap informational".to_string(),
            "ip routing".to_string(),
        ])];

        let elems = elements(&[(
            "logging-config",
            "logging host 10.0.0.5\nlogging trap informational\n",
        )]);

        let result = build_template(&sections, &elems, None, None);

        assert!(result.template_content.contains("hostname switch1"));
        assert!(result.template_content.contains("!!!###logging-config"));
        assert!(result.template_content.contains("ip routing"));
        assert!(!result.template_content.contains("logging host 10.0.0.5"));
    }

    // -------------------------------------------------------------------------
    // Test 12: element does NOT span across different Config sections
    // -------------------------------------------------------------------------

    #[test]
    fn test_element_does_not_span_sections() {
        // The element lines are split across two GlobalSection::Config blocks,
        // separated by a VirtualInterface. Matching must not cross sections.
        let sections = vec![
            GlobalSection::Config(vec!["ntp server 1.2.3.4".to_string()]),
            GlobalSection::VirtualInterface("Loopback0".to_string(), vec![]),
            GlobalSection::Config(vec!["ntp server 5.6.7.8".to_string()]),
        ];

        let elems = elements(&[("ntp-pair", "ntp server 1.2.3.4\nntp server 5.6.7.8\n")]);

        let result = build_template(&sections, &elems, None, None);

        assert!(!result.template_content.contains("!!!###ntp-pair"));
        assert!(result.template_content.contains("ntp server 1.2.3.4"));
        assert!(result.template_content.contains("ntp server 5.6.7.8"));
    }

    // -------------------------------------------------------------------------
    // Test 13: empty global sections → empty template
    // -------------------------------------------------------------------------

    #[test]
    fn test_empty_sections() {
        let result = build_template(&[], &no_elements(), None, None);
        assert!(result.template_content.is_empty());
    }

    // -------------------------------------------------------------------------
    // Test 14: new_elements is empty (unmatched lines stay as literal text only)
    // -------------------------------------------------------------------------

    #[test]
    fn test_new_elements_empty() {
        let sections = vec![GlobalSection::Config(vec!["hostname switch1".to_string()])];
        let result = build_template(&sections, &no_elements(), None, None);
        assert!(result.new_elements.is_empty());
    }
}
