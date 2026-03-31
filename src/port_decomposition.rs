/// Port Configuration Decomposition (Stage 2 of aycfgextract pipeline).
///
/// Groups interface config blocks into services and produces port assignments.

use std::collections::HashMap;

// ─── Public data structures ───────────────────────────────────────────────────

/// A derived service with its port-config content.
pub struct DerivedService {
    /// Service name, e.g., "access-vlan10".
    pub name: String,
    /// The service template (port-config.txt content).
    pub port_config: String,
}

/// A port assignment produced by the decomposition.
pub struct DecomposedPort {
    /// Port identifier, e.g., "Port0" or "Port0.100".
    pub port_id: String,
    /// Full IOS interface name, e.g., "GigabitEthernet0/0".
    pub interface_name: String,
    /// Which service this port uses.
    pub service_name: String,
    /// Lines prepended before service config in the compiled output, or `None`.
    pub prologue: Option<String>,
    /// Lines appended after service config in the compiled output, or `None`.
    pub epilogue: Option<String>,
}

/// The result of decomposing a set of interface blocks.
pub struct DecompositionResult {
    /// New (or matched) services to create/reuse.
    pub services: Vec<DerivedService>,
    /// Port assignments for the device config.
    pub ports: Vec<DecomposedPort>,
}

// ─── Public entry point ───────────────────────────────────────────────────────

/// Decompose parsed interface blocks into services and port assignments.
///
/// # Parameters
/// - `port_blocks` — `(interface_name, config_lines)` pairs for physical ports and sub-interfaces.
///   Lines are as captured by the parser (with leading whitespace).
/// - `existing_services` — service_name → port-config.txt content from the data store.
/// - `port_id_map` — interface_name → port_id (e.g., `"GigabitEthernet0/0"` → `"Port0"`).
pub fn decompose_ports(
    port_blocks: &[(String, Vec<String>)],
    existing_services: &HashMap<String, String>,
    port_id_map: &HashMap<String, String>,
) -> DecompositionResult {
    // ── Step 1: normalize config lines ────────────────────────────────────────
    // Build (interface_name, normalized_lines, original_lines) triples.
    let ports_data: Vec<PortData> = port_blocks
        .iter()
        .map(|(name, lines)| {
            let normalized: Vec<String> = lines.iter().map(|l| l.trim().to_string()).collect();
            PortData {
                interface_name: name.clone(),
                normalized_lines: normalized,
                original_lines: lines.clone(),
            }
        })
        .collect();

    // ── Step 2: group by structural identity ─────────────────────────────────
    let groups = group_by_structural_identity(&ports_data);

    // ── Steps 3-7: build services and port assignments ────────────────────────
    let mut result = DecompositionResult {
        services: Vec::new(),
        ports: Vec::new(),
    };
    let mut service_counter = 0usize;
    // Map from normalized port-config content → service name (for dedup across groups).
    // Keys are always normalized (trimmed lines) for consistent matching regardless
    // of indentation differences between existing and extracted services.
    let mut content_to_service: HashMap<String, String> = HashMap::new();

    // Seed with existing services (normalize their content for the key)
    for (svc_name, svc_content) in existing_services {
        let key = normalize_port_config(svc_content);
        content_to_service
            .entry(key)
            .or_insert_with(|| svc_name.clone());
    }

    for group in groups {
        process_group(
            &group,
            existing_services,
            port_id_map,
            &mut result,
            &mut content_to_service,
            &mut service_counter,
        );
    }

    result
}

// ─── Internal types ───────────────────────────────────────────────────────────

struct PortData {
    interface_name: String,
    normalized_lines: Vec<String>,
    original_lines: Vec<String>,
}

struct PortGroup {
    ports: Vec<usize>, // indices into the original ports_data slice
    // We carry clones of the per-port data for processing
    members: Vec<PortData>,
}

// ─── Structural grouping ──────────────────────────────────────────────────────

/// Extract the "structural key" for a port — the combination of fields that
/// forces separate groups regardless of deviations.
fn structural_key(normalized: &[String]) -> String {
    let mut mode = String::new();
    let mut vlan = String::new();
    let mut channel = String::new();

    for line in normalized {
        if line.starts_with("switchport mode ") {
            mode = line.clone();
        } else if line.starts_with("switchport access vlan ") {
            vlan = line.clone();
        } else if line.starts_with("switchport trunk allowed vlan ") {
            vlan = line.clone();
        } else if line.starts_with("channel-group ") {
            channel = line.clone();
        }
    }

    format!("mode={};vlan={};channel={}", mode, vlan, channel)
}

/// Group ports by structural identity. Returns one `PortGroup` per distinct
/// structural key, preserving the order in which groups are first seen.
fn group_by_structural_identity(ports_data: &[PortData]) -> Vec<PortGroup> {
    let mut key_to_group_idx: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<PortGroup> = Vec::new();

    for (idx, pd) in ports_data.iter().enumerate() {
        let key = structural_key(&pd.normalized_lines);
        let group_idx = key_to_group_idx.entry(key).or_insert_with(|| {
            groups.push(PortGroup {
                ports: Vec::new(),
                members: Vec::new(),
            });
            groups.len() - 1
        });
        groups[*group_idx].ports.push(idx);
        groups[*group_idx].members.push(PortData {
            interface_name: pd.interface_name.clone(),
            normalized_lines: pd.normalized_lines.clone(),
            original_lines: pd.original_lines.clone(),
        });
    }

    groups
}

// ─── Group processing ─────────────────────────────────────────────────────────

fn process_group(
    group: &PortGroup,
    existing_services: &HashMap<String, String>,
    port_id_map: &HashMap<String, String>,
    result: &mut DecompositionResult,
    content_to_service: &mut HashMap<String, String>,
    service_counter: &mut usize,
) {
    // ── Step 3: find the most common config within the group ──────────────────
    // Count how many ports share each normalized config signature.
    let mut config_counts: HashMap<Vec<String>, usize> = HashMap::new();
    for pd in &group.members {
        *config_counts.entry(pd.normalized_lines.clone()).or_insert(0) += 1;
    }

    // The template is the config with the highest count (break ties by first seen).
    // Track both normalized (for comparison) and original (for output) lines.
    let (template_lines, template_original_lines): (Vec<String>, Vec<String>) = {
        let mut best_count = 0usize;
        let mut best_norm: Option<Vec<String>> = None;
        let mut best_orig: Option<Vec<String>> = None;
        // Iterate in member order for stable first-seen tie-breaking.
        for pd in &group.members {
            let count = *config_counts.get(&pd.normalized_lines).unwrap_or(&0);
            if count > best_count {
                best_count = count;
                best_norm = Some(pd.normalized_lines.clone());
                best_orig = Some(pd.original_lines.clone());
            }
        }
        (best_norm.unwrap_or_default(), best_orig.unwrap_or_default())
    };

    // ── Step 4: detect deviations and handle shutdown ─────────────────────────
    // Partition members into "matches template" and "deviations".
    let mut template_members: Vec<&PortData> = Vec::new();
    let mut deviation_members: Vec<&PortData> = Vec::new();

    for pd in &group.members {
        if pd.normalized_lines == template_lines {
            template_members.push(pd);
        } else {
            deviation_members.push(pd);
        }
    }

    // Count deviating ports that share the identical deviation set.
    // deviation_set = lines in port but not in template (by sorted multiset diff)
    let mut deviation_groups: HashMap<Vec<String>, Vec<&PortData>> = HashMap::new();
    for pd in &deviation_members {
        let dev_set = compute_deviation(&template_lines, &pd.normalized_lines);
        deviation_groups.entry(dev_set).or_default().push(pd);
    }

    // Get or create the base service for the template.
    // Use original lines (with indentation) for the port-config content.
    let template_port_config = lines_to_port_config(&template_original_lines);
    let base_service_name =
        get_or_create_service(&template_port_config, &template_lines, existing_services, content_to_service, result, service_counter);

    // ── Assign template-matching ports ────────────────────────────────────────
    for pd in &template_members {
        assign_port(pd, &base_service_name, None, None, port_id_map, result);
    }

    // ── Handle deviation groups ────────────────────────────────────────────────
    for (_dev_set, dev_ports) in &deviation_groups {
        if dev_ports.len() >= 3 {
            // Promote deviation to new service
            // Use first member's original lines (with indentation) for the service content.
            let full_lines = &dev_ports[0].normalized_lines;
            let port_config = lines_to_port_config(&dev_ports[0].original_lines);
            let svc_name = get_or_create_service(
                &port_config,
                full_lines,
                existing_services,
                content_to_service,
                result,
                service_counter,
            );
            for pd in dev_ports {
                assign_port(pd, &svc_name, None, None, port_id_map, result);
            }
        } else {
            // Express as prologue/epilogue on the base service.
            for pd in dev_ports {
                // Shutdown handling step 3: port with only "shutdown"
                if pd.normalized_lines == vec!["shutdown".to_string()] {
                    let shutdown_svc = get_or_create_shutdown_service(existing_services, content_to_service, result);
                    assign_port(pd, &shutdown_svc, None, None, port_id_map, result);
                    continue;
                }

                let (prologue, epilogue) =
                    compute_prologue_epilogue(&template_lines, &pd.normalized_lines, &pd.original_lines);

                if prologue.is_none() && epilogue.is_none() && pd.normalized_lines != template_lines {
                    // Unclean split — can't decompose into prologue+service+epilogue.
                    // Create a new service for this port's full config (with original indentation).
                    let full_config = lines_to_port_config(&pd.original_lines);
                    let svc_name = get_or_create_service(
                        &full_config,
                        &pd.normalized_lines,
                        existing_services,
                        content_to_service,
                        result,
                        service_counter,
                    );
                    assign_port(pd, &svc_name, None, None, port_id_map, result);
                } else {
                    assign_port(pd, &base_service_name, prologue, epilogue, port_id_map, result);
                }
            }
        }
    }
}

// ─── Shutdown-only service ────────────────────────────────────────────────────

fn get_or_create_shutdown_service(
    existing_services: &HashMap<String, String>,
    content_to_service: &mut HashMap<String, String>,
    result: &mut DecompositionResult,
) -> String {
    let content = " shutdown\n".to_string();
    let key = normalize_port_config(&content);
    if let Some(name) = content_to_service.get(&key) {
        return name.clone();
    }
    // Check existing services by name "shutdown"
    if let Some(existing_content) = existing_services.get("shutdown") {
        if normalize_port_config(existing_content) == key {
            content_to_service.insert(key, "shutdown".to_string());
            return "shutdown".to_string();
        }
    }
    content_to_service.insert(key, "shutdown".to_string());
    result.services.push(DerivedService {
        name: "shutdown".to_string(),
        port_config: content,
    });
    "shutdown".to_string()
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Normalize port-config content for comparison: trim each line, join with newline.
fn normalize_port_config(content: &str) -> String {
    let mut s = String::new();
    for line in content.lines() {
        s.push_str(line.trim());
        s.push('\n');
    }
    s
}

/// Convert lines to port-config.txt string content, preserving original formatting.
fn lines_to_port_config(lines: &[String]) -> String {
    let mut s = String::new();
    for line in lines {
        s.push_str(line);
        s.push('\n');
    }
    s
}


/// Get an existing service for this content, or create a new one.
fn get_or_create_service(
    port_config: &str,
    normalized_lines: &[String],
    _existing_services: &HashMap<String, String>,
    content_to_service: &mut HashMap<String, String>,
    result: &mut DecompositionResult,
    service_counter: &mut usize,
) -> String {
    // Use normalized key for matching (indentation-insensitive)
    let key = normalize_port_config(port_config);
    if let Some(name) = content_to_service.get(&key) {
        return name.clone();
    }
    // Generate a name from structural properties.
    let name = derive_service_name(normalized_lines, service_counter);
    content_to_service.insert(key, name.clone());
    result.services.push(DerivedService {
        name: name.clone(),
        port_config: port_config.to_string(),
    });
    name
}

/// Derive a service name from normalized config lines.
fn derive_service_name(lines: &[String], counter: &mut usize) -> String {
    // Access vlan
    for line in lines {
        if let Some(rest) = line.strip_prefix("switchport access vlan ") {
            return format!("access-vlan{}", rest.trim());
        }
    }
    // Channel-group (check before trunk since channel-group ports may also be trunk)
    for line in lines {
        if let Some(rest) = line.strip_prefix("channel-group ") {
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            return format!("channel-group-{}", num);
        }
    }
    // Trunk: check for allowed vlan or just trunk mode
    let is_trunk = lines.iter().any(|l| l == "switchport mode trunk");
    for line in lines {
        if let Some(rest) = line.strip_prefix("switchport trunk allowed vlan ") {
            let vlan_part = rest.trim().replace(',', "-");
            if vlan_part == "all" {
                return "trunk-all".to_string();
            }
            return format!("trunk-vlan{}", vlan_part);
        }
    }
    if is_trunk {
        return "trunk-all".to_string();
    }
    // Shutdown-only
    if lines == &["shutdown"] {
        return "shutdown".to_string();
    }
    // Routed (no switchport mode line)
    let has_switchport_mode = lines.iter().any(|l| l.starts_with("switchport mode "));
    if !has_switchport_mode && lines.iter().any(|l| l.starts_with("ip address ") || l.starts_with("no switchport")) {
        return "routed-default".to_string();
    }
    // Fallback
    let n = *counter;
    *counter += 1;
    format!("service-{}", n)
}

/// Compute the symmetric difference between template and port config.
/// Returns lines added in port AND lines removed from template (sorted).
/// This ensures two ports with different missing/added lines get different deviation sets.
fn compute_deviation(template_lines: &[String], port_lines: &[String]) -> Vec<String> {
    let mut template_sorted: Vec<String> = template_lines.to_vec();
    template_sorted.sort();
    let mut port_sorted: Vec<String> = port_lines.to_vec();
    port_sorted.sort();

    let mut diff: Vec<String> = Vec::new();
    let mut t_idx = 0;
    let mut p_idx = 0;

    while t_idx < template_sorted.len() && p_idx < port_sorted.len() {
        match template_sorted[t_idx].cmp(&port_sorted[p_idx]) {
            std::cmp::Ordering::Equal => {
                t_idx += 1;
                p_idx += 1;
            }
            std::cmp::Ordering::Less => {
                // In template but not port (removal)
                diff.push(format!("-{}", template_sorted[t_idx]));
                t_idx += 1;
            }
            std::cmp::Ordering::Greater => {
                // In port but not template (addition)
                diff.push(format!("+{}", port_sorted[p_idx]));
                p_idx += 1;
            }
        }
    }
    while t_idx < template_sorted.len() {
        diff.push(format!("-{}", template_sorted[t_idx]));
        t_idx += 1;
    }
    while p_idx < port_sorted.len() {
        diff.push(format!("+{}", port_sorted[p_idx]));
        p_idx += 1;
    }
    diff
}

/// Compute prologue and epilogue for a deviating port.
///
/// Uses positional matching: walk through the port's lines and try to match
/// each template line in order. Extra lines before the first template match
/// are prologue, extra lines after the last template match are epilogue.
///
/// Returns `(None, None)` if the split is "unclean" — extra lines appear
/// between matched template lines, meaning a clean prologue+service+epilogue
/// decomposition is not possible. The caller should create a new service instead.
fn compute_prologue_epilogue(
    template_lines: &[String],
    _normalized_port_lines: &[String],
    original_lines: &[String],
) -> (Option<String>, Option<String>) {
    let normalized_original: Vec<String> = original_lines.iter().map(|l| l.trim().to_string()).collect();

    // Find the range of positions in the port's lines that correspond to template lines.
    // Use greedy positional matching: for each template line (in order), find the next
    // matching line in the port.
    let mut template_match_positions: Vec<usize> = Vec::new();
    let mut search_from = 0;
    for tline in template_lines {
        let mut found = false;
        for pos in search_from..normalized_original.len() {
            if &normalized_original[pos] == tline {
                template_match_positions.push(pos);
                search_from = pos + 1;
                found = true;
                break;
            }
        }
        if !found {
            // Template line not found in port — this is a removal, can't do clean split
            return (None, None);
        }
    }

    if template_match_positions.is_empty() {
        // No template lines matched — entire port config is deviation
        return (None, None);
    }

    let first_match = *template_match_positions.first().unwrap();
    let last_match = *template_match_positions.last().unwrap();

    // Check for interleaved extra lines: any non-template line between first_match and last_match?
    let matched_set: std::collections::HashSet<usize> = template_match_positions.iter().copied().collect();
    for pos in first_match..=last_match {
        if !matched_set.contains(&pos) {
            // Extra line interleaved between template lines — unclean split
            return (None, None);
        }
    }

    // Clean split: lines before first_match are prologue, lines after last_match are epilogue.
    // Preserve original formatting (including indentation) for round-trip fidelity.
    let mut prologue_lines: Vec<&str> = Vec::new();
    for pos in 0..first_match {
        prologue_lines.push(&original_lines[pos]);
    }

    let mut epilogue_lines: Vec<&str> = Vec::new();
    for pos in (last_match + 1)..original_lines.len() {
        epilogue_lines.push(&original_lines[pos]);
    }

    let prologue = if prologue_lines.is_empty() {
        None
    } else {
        Some(prologue_lines.join("\n"))
    };
    let epilogue = if epilogue_lines.is_empty() {
        None
    } else {
        Some(epilogue_lines.join("\n"))
    };

    (prologue, epilogue)
}

/// Add a `DecomposedPort` to the result.
fn assign_port(
    pd: &PortData,
    service_name: &str,
    prologue: Option<String>,
    epilogue: Option<String>,
    port_id_map: &HashMap<String, String>,
    result: &mut DecompositionResult,
) {
    let port_id = port_id_map
        .get(&pd.interface_name)
        .cloned()
        .unwrap_or_else(|| pd.interface_name.clone());

    result.ports.push(DecomposedPort {
        port_id,
        interface_name: pd.interface_name.clone(),
        service_name: service_name.to_string(),
        prologue,
        epilogue,
    });
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build a port_id_map from a list of (interface_name, port_id) pairs.
    fn pid_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // Helper: build port_blocks from (interface_name, indented_lines) pairs.
    fn port_blocks(pairs: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        pairs
            .iter()
            .map(|(name, lines)| {
                (
                    name.to_string(),
                    lines.iter().map(|l| format!(" {}", l)).collect(),
                )
            })
            .collect()
    }

    // ── Test 1: Simple case — 4 ports with identical config → 1 service ───────

    #[test]
    fn test_simple_identical_ports_one_service() {
        let config_lines = &[
            "switchport mode access",
            "switchport access vlan 10",
        ];
        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", config_lines),
            ("GigabitEthernet0/1", config_lines),
            ("GigabitEthernet0/2", config_lines),
            ("GigabitEthernet0/3", config_lines),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
            ("GigabitEthernet0/2", "Port2"),
            ("GigabitEthernet0/3", "Port3"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        assert_eq!(result.services.len(), 1, "should produce exactly one service");
        assert_eq!(result.services[0].name, "access-vlan10");
        assert_eq!(result.ports.len(), 4);
        for port in &result.ports {
            assert_eq!(port.service_name, "access-vlan10");
            assert!(port.prologue.is_none());
            assert!(port.epilogue.is_none());
        }
    }

    // ── Test 2: Two groups — 3 access-vlan10 + 2 access-vlan20 → 2 services ──

    #[test]
    fn test_two_vlan_groups_two_services() {
        let vlan10_lines = &["switchport mode access", "switchport access vlan 10"];
        let vlan20_lines = &["switchport mode access", "switchport access vlan 20"];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", vlan10_lines),
            ("GigabitEthernet0/1", vlan10_lines),
            ("GigabitEthernet0/2", vlan10_lines),
            ("GigabitEthernet0/3", vlan20_lines),
            ("GigabitEthernet0/4", vlan20_lines),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
            ("GigabitEthernet0/2", "Port2"),
            ("GigabitEthernet0/3", "Port3"),
            ("GigabitEthernet0/4", "Port4"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        assert_eq!(result.services.len(), 2, "should produce two services");
        let svc_names: Vec<&str> = result.services.iter().map(|s| s.name.as_str()).collect();
        assert!(svc_names.contains(&"access-vlan10"));
        assert!(svc_names.contains(&"access-vlan20"));

        let vlan10_ports: Vec<_> = result.ports.iter().filter(|p| p.service_name == "access-vlan10").collect();
        let vlan20_ports: Vec<_> = result.ports.iter().filter(|p| p.service_name == "access-vlan20").collect();
        assert_eq!(vlan10_ports.len(), 3);
        assert_eq!(vlan20_ports.len(), 2);
    }

    // ── Test 3: Deviation < 3 ports → 1 service, outlier gets epilogue ────────

    #[test]
    fn test_deviation_less_than_3_gets_epilogue() {
        let base_lines = &["switchport mode access", "switchport access vlan 10"];
        let extra_lines = &["switchport mode access", "switchport access vlan 10", "no cdp enable"];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", base_lines),
            ("GigabitEthernet0/1", base_lines),
            ("GigabitEthernet0/2", base_lines),
            ("GigabitEthernet0/3", extra_lines), // 1 outlier
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
            ("GigabitEthernet0/2", "Port2"),
            ("GigabitEthernet0/3", "Port3"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // Only 1 service since the deviation is < 3 ports
        assert_eq!(result.services.len(), 1, "should have 1 service for base config");

        // The outlier port should use the base service with an epilogue
        let outlier = result.ports.iter().find(|p| p.port_id == "Port3").unwrap();
        assert_eq!(outlier.service_name, "access-vlan10");
        assert!(outlier.epilogue.is_some(), "outlier should have epilogue");
        assert!(outlier.epilogue.as_deref().unwrap().contains("no cdp enable"));
        assert!(outlier.prologue.is_none());
    }

    // ── Test 4: Deviation >= 3 ports → 2 services ─────────────────────────────

    #[test]
    fn test_deviation_3_or_more_promotes_to_new_service() {
        let base_lines = &["switchport mode access", "switchport access vlan 10"];
        let extra_lines = &["switchport mode access", "switchport access vlan 10", "storm-control broadcast level 10"];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", base_lines),
            ("GigabitEthernet0/1", base_lines),
            ("GigabitEthernet0/2", base_lines),
            ("GigabitEthernet0/3", extra_lines),
            ("GigabitEthernet0/4", extra_lines),
            ("GigabitEthernet0/5", extra_lines),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
            ("GigabitEthernet0/2", "Port2"),
            ("GigabitEthernet0/3", "Port3"),
            ("GigabitEthernet0/4", "Port4"),
            ("GigabitEthernet0/5", "Port5"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // 2 services: base access-vlan10, and a second for the promoted deviation
        assert_eq!(result.services.len(), 2, "deviation >=3 should create new service");

        // Ports 0-2 have no prologue/epilogue
        for i in 0..3 {
            let p = result.ports.iter().find(|p| p.port_id == format!("Port{}", i)).unwrap();
            assert!(p.prologue.is_none());
            assert!(p.epilogue.is_none());
        }
        // Ports 3-5 also have no prologue/epilogue (they use a dedicated service)
        for i in 3..6 {
            let p = result.ports.iter().find(|p| p.port_id == format!("Port{}", i)).unwrap();
            assert!(p.prologue.is_none());
            assert!(p.epilogue.is_none());
        }
    }

    // ── Test 5: Prologue and epilogue ─────────────────────────────────────────

    #[test]
    fn test_prologue_and_epilogue() {
        let base_lines = &["switchport mode access", "switchport access vlan 10"];
        // Port with extra line before and after base config
        let extra_lines = &[
            "description SPECIAL",      // prologue
            "switchport mode access",
            "switchport access vlan 10",
            "no cdp enable",            // epilogue
        ];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", base_lines),
            ("GigabitEthernet0/1", base_lines),
            ("GigabitEthernet0/2", base_lines),
            ("GigabitEthernet0/3", extra_lines),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
            ("GigabitEthernet0/2", "Port2"),
            ("GigabitEthernet0/3", "Port3"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        let outlier = result.ports.iter().find(|p| p.port_id == "Port3").unwrap();
        assert_eq!(outlier.service_name, "access-vlan10");
        assert!(outlier.prologue.is_some(), "should have prologue");
        assert!(outlier.prologue.as_deref().unwrap().contains("description SPECIAL"));
        assert!(outlier.epilogue.is_some(), "should have epilogue");
        assert!(outlier.epilogue.as_deref().unwrap().contains("no cdp enable"));
    }

    // ── Test 6: Shutdown handling step 1 — exact match against existing service

    #[test]
    fn test_shutdown_exact_match_existing_service() {
        // Existing service includes shutdown
        let mut existing: HashMap<String, String> = HashMap::new();
        existing.insert(
            "access-vlan10-shutdown".to_string(),
            "switchport mode access\nswitchport access vlan 10\nshutdown\n".to_string(),
        );

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", &["switchport mode access", "switchport access vlan 10", "shutdown"]),
        ]);
        let pid = pid_map(&[("GigabitEthernet0/0", "Port0")]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // Should reuse existing service, not create new one
        assert_eq!(result.services.len(), 0, "no new services created, reuse existing");
        assert_eq!(result.ports[0].service_name, "access-vlan10-shutdown");
        assert!(result.ports[0].epilogue.is_none());
    }

    // ── Test 7: Shutdown handling step 2 — strip shutdown, match, add epilogue ─

    #[test]
    fn test_shutdown_strip_and_add_epilogue() {
        // Existing service without shutdown
        let mut existing: HashMap<String, String> = HashMap::new();
        existing.insert(
            "access-vlan10".to_string(),
            "switchport mode access\nswitchport access vlan 10\n".to_string(),
        );

        // Port has the base config + shutdown, but shutdown is the only deviation
        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", &["switchport mode access", "switchport access vlan 10"]),
            ("GigabitEthernet0/1", &["switchport mode access", "switchport access vlan 10", "shutdown"]),
        ]);
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        let shut_port = result.ports.iter().find(|p| p.port_id == "Port1").unwrap();
        assert_eq!(shut_port.service_name, "access-vlan10");
        assert!(shut_port.epilogue.is_some());
        assert!(shut_port.epilogue.as_deref().unwrap().contains("shutdown"));
    }

    // ── Test 8: Shutdown handling step 3 — port with only shutdown → shutdown svc

    #[test]
    fn test_shutdown_only_port_uses_shutdown_service() {
        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", &["switchport mode access", "switchport access vlan 10"]),
            ("GigabitEthernet0/1", &["shutdown"]),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        let shut_port = result.ports.iter().find(|p| p.port_id == "Port1").unwrap();
        assert_eq!(shut_port.service_name, "shutdown");

        // A "shutdown" service should have been created
        let svc = result.services.iter().find(|s| s.name == "shutdown");
        assert!(svc.is_some(), "shutdown service should be created");
    }

    // ── Test 9: Existing service match ────────────────────────────────────────

    #[test]
    fn test_existing_service_reused() {
        let mut existing: HashMap<String, String> = HashMap::new();
        existing.insert(
            "access-vlan10".to_string(),
            "switchport mode access\nswitchport access vlan 10\n".to_string(),
        );

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", &["switchport mode access", "switchport access vlan 10"]),
            ("GigabitEthernet0/1", &["switchport mode access", "switchport access vlan 10"]),
        ]);
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // No new service should be created since it matches existing
        assert_eq!(result.services.len(), 0, "existing service should be reused");
        for port in &result.ports {
            assert_eq!(port.service_name, "access-vlan10");
        }
    }

    // ── Test 10: Sub-interface ports treated same as physical ─────────────────

    #[test]
    fn test_sub_interface_ports() {
        let config_lines = &["encapsulation dot1Q 100", "ip address 10.1.0.1 255.255.255.0"];
        let blocks = port_blocks(&[
            ("GigabitEthernet0/0.100", config_lines),
            ("GigabitEthernet0/0.200", &["encapsulation dot1Q 200", "ip address 10.2.0.1 255.255.255.0"]),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0.100", "Port0.100"),
            ("GigabitEthernet0/0.200", "Port0.200"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // Two services (different VLAN encapsulations = different structural key via vlan or fallback)
        // Both are sub-interface ports and get correct port_ids
        let p100 = result.ports.iter().find(|p| p.port_id == "Port0.100").unwrap();
        let p200 = result.ports.iter().find(|p| p.port_id == "Port0.200").unwrap();
        assert_eq!(p100.interface_name, "GigabitEthernet0/0.100");
        assert_eq!(p200.interface_name, "GigabitEthernet0/0.200");
    }

    // ── Test 11: Channel-group → separate service ─────────────────────────────

    #[test]
    fn test_channel_group_separate_service() {
        let access_lines = &["switchport mode access", "switchport access vlan 10"];
        let channel_lines = &["channel-group 1 mode active", "switchport mode trunk"];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", access_lines),
            ("GigabitEthernet0/1", access_lines),
            ("GigabitEthernet0/2", channel_lines),
            ("GigabitEthernet0/3", channel_lines),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
            ("GigabitEthernet0/2", "Port2"),
            ("GigabitEthernet0/3", "Port3"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // Should have at least 2 services: one for access vlan, one for channel-group
        assert!(result.services.len() >= 2, "channel-group ports must form separate service");

        let channel_ports: Vec<_> = result.ports.iter()
            .filter(|p| p.port_id == "Port2" || p.port_id == "Port3")
            .collect();
        assert_eq!(channel_ports.len(), 2);
        // Both channel-group ports should use the same service
        assert_eq!(channel_ports[0].service_name, channel_ports[1].service_name);

        // Channel-group service name should reflect channel-group
        let cg_svc_name = &channel_ports[0].service_name;
        assert!(cg_svc_name.contains("channel-group"), "service name should contain channel-group");
    }

    // ── Test 12: Unclean split — interleaved lines → new service ─────────────

    #[test]
    fn test_unclean_split_creates_new_service() {
        // Base: lines A, B, C
        // Port: lines A, X, B, C  → X is interleaved (between A and B)
        // Per spec: if lines can't be cleanly split into prologue+service+epilogue,
        // create a new service instead.
        let base_lines = &["switchport mode access", "switchport access vlan 10", "spanning-tree portfast"];
        let interleaved = &[
            "switchport mode access",
            "description INTERLEAVED",   // inserted between line 1 and 2
            "switchport access vlan 10",
            "spanning-tree portfast",
        ];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", base_lines),
            ("GigabitEthernet0/1", base_lines),
            ("GigabitEthernet0/2", base_lines),
            ("GigabitEthernet0/3", interleaved),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
            ("GigabitEthernet0/2", "Port2"),
            ("GigabitEthernet0/3", "Port3"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // The interleaved port should get its own service (unclean split)
        let outlier = result.ports.iter().find(|p| p.port_id == "Port3").unwrap();
        // Should NOT use the base service with prologue/epilogue
        assert!(outlier.prologue.is_none(), "unclean split should not produce prologue");
        assert!(outlier.epilogue.is_none(), "unclean split should not produce epilogue");
        // Should have its own service (2 services total: base + interleaved)
        assert_eq!(result.services.len(), 2, "unclean split creates a new service");
    }
}
