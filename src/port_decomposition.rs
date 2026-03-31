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
    /// Primary VLAN number (for vars.json), if determinable.
    pub vlan: Option<u32>,
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
            let normalized_no_desc: Vec<String> = normalized
                .iter()
                .filter(|l| !l.starts_with("description "))
                .cloned()
                .collect();
            let original_no_desc: Vec<String> = lines
                .iter()
                .filter(|l| !l.trim().starts_with("description "))
                .cloned()
                .collect();
            let description_lines: Vec<String> = lines
                .iter()
                .filter(|l| l.trim().starts_with("description "))
                .cloned()
                .collect();
            PortData {
                interface_name: name.clone(),
                normalized_lines: normalized,
                original_lines: lines.clone(),
                normalized_no_desc,
                original_no_desc,
                description_lines,
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

    let debug = std::env::var("AYCFG_DEBUG").is_ok();

    for (gi, group) in groups.iter().enumerate() {
        if debug {
            eprintln!("[decompose] group {}: {} members, structural key from first member",
                gi, group.members.len());
            for pd in &group.members {
                eprintln!("  {} (no_desc: {} lines, desc: {} lines)",
                    pd.interface_name, pd.normalized_no_desc.len(), pd.description_lines.len());
            }
        }
        process_group(
            group,
            existing_services,
            port_id_map,
            &mut result,
            &mut content_to_service,
            &mut service_counter,
            debug,
        );
    }

    if debug {
        eprintln!("[decompose] result: {} services, {} ports", result.services.len(), result.ports.len());
        for svc in &result.services {
            eprintln!("  service '{}': {} lines", svc.name, svc.port_config.lines().count());
        }
        for p in &result.ports {
            eprintln!("  {} ({}) -> svc={}, prologue={}, epilogue={}",
                p.port_id, p.interface_name, p.service_name,
                p.prologue.is_some(), p.epilogue.is_some());
        }
    }

    result
}

// ─── Internal types ───────────────────────────────────────────────────────────

struct PortData {
    interface_name: String,
    normalized_lines: Vec<String>,
    original_lines: Vec<String>,
    /// Normalized lines with description lines removed (for template comparison).
    normalized_no_desc: Vec<String>,
    /// Original lines with description lines removed (for service content).
    original_no_desc: Vec<String>,
    /// Original description lines (to be used as prologue).
    description_lines: Vec<String>,
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
            normalized_no_desc: pd.normalized_no_desc.clone(),
            original_no_desc: pd.original_no_desc.clone(),
            description_lines: pd.description_lines.clone(),
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
    debug: bool,
) {
    // ── Step 3: find the most common config within the group ──────────────────
    // Use description-stripped lines for counting so that per-port descriptions
    // don't fragment the template selection.
    let mut config_counts: HashMap<Vec<String>, usize> = HashMap::new();
    for pd in &group.members {
        *config_counts
            .entry(pd.normalized_no_desc.clone())
            .or_insert(0) += 1;
    }

    // The template is the config with the highest count (break ties by first seen).
    // Track both normalized (for comparison) and original (for output) lines —
    // always using description-stripped versions.
    let (template_lines, template_original_lines): (Vec<String>, Vec<String>) = {
        let mut best_count = 0usize;
        let mut best_norm: Option<Vec<String>> = None;
        let mut best_orig: Option<Vec<String>> = None;
        for pd in &group.members {
            let count = *config_counts.get(&pd.normalized_no_desc).unwrap_or(&0);
            if count > best_count {
                best_count = count;
                best_norm = Some(pd.normalized_no_desc.clone());
                best_orig = Some(pd.original_no_desc.clone());
            }
        }
        (best_norm.unwrap_or_default(), best_orig.unwrap_or_default())
    };

    if debug {
        eprintln!("[process_group] template ({} lines, count={}):",
            template_lines.len(),
            config_counts.get(&template_lines).unwrap_or(&0));
        for l in &template_lines {
            eprintln!("    {}", l);
        }
    }

    // ── Step 4: detect deviations and handle shutdown ─────────────────────────
    // Partition members into "matches template" and "deviations",
    // comparing description-stripped lines.
    let mut template_members: Vec<&PortData> = Vec::new();
    let mut deviation_members: Vec<&PortData> = Vec::new();

    for pd in &group.members {
        if pd.normalized_no_desc == template_lines {
            template_members.push(pd);
        } else {
            deviation_members.push(pd);
        }
    }

    // Count deviating ports that share the identical deviation set.
    let mut deviation_groups: HashMap<Vec<String>, Vec<&PortData>> = HashMap::new();
    for pd in &deviation_members {
        let dev_set = compute_deviation(&template_lines, &pd.normalized_no_desc);
        deviation_groups.entry(dev_set).or_default().push(pd);
    }

    // Get or create the base service for the template.
    // Use original description-stripped lines for the port-config content.
    let template_port_config = lines_to_port_config(&template_original_lines);
    let base_service_name = get_or_create_service(
        &template_port_config,
        &template_lines,
        existing_services,
        content_to_service,
        result,
        service_counter,
    );

    // ── Assign template-matching ports ────────────────────────────────────────
    for pd in &template_members {
        let desc_prologue = description_prologue(pd);
        assign_port(pd, &base_service_name, desc_prologue, None, port_id_map, result);
    }

    // ── Handle deviation groups ────────────────────────────────────────────────
    for (_dev_set, dev_ports) in &deviation_groups {
        if dev_ports.len() >= 3 {
            // Promote deviation to new service (using description-stripped content).
            let full_lines = &dev_ports[0].normalized_no_desc;
            let port_config = lines_to_port_config(&dev_ports[0].original_no_desc);
            let svc_name = get_or_create_service(
                &port_config,
                full_lines,
                existing_services,
                content_to_service,
                result,
                service_counter,
            );
            for pd in dev_ports {
                let desc_prologue = description_prologue(pd);
                assign_port(pd, &svc_name, desc_prologue, None, port_id_map, result);
            }
        } else {
            // Express as prologue/epilogue on the base service.
            for pd in dev_ports {
                // Shutdown handling step 3: port with only "shutdown"
                if pd.normalized_no_desc == vec!["shutdown".to_string()] {
                    let shutdown_svc =
                        get_or_create_shutdown_service(existing_services, content_to_service, result);
                    let desc_prologue = description_prologue(pd);
                    assign_port(pd, &shutdown_svc, desc_prologue, None, port_id_map, result);
                    continue;
                }

                let (prologue, epilogue) = compute_prologue_epilogue(
                    &template_lines,
                    &pd.normalized_no_desc,
                    &pd.original_no_desc,
                );

                if prologue.is_none() && epilogue.is_none() && pd.normalized_no_desc != template_lines
                {
                    // Unclean split — create a new service (description-stripped).
                    let full_config = lines_to_port_config(&pd.original_no_desc);
                    let svc_name = get_or_create_service(
                        &full_config,
                        &pd.normalized_no_desc,
                        existing_services,
                        content_to_service,
                        result,
                        service_counter,
                    );
                    let desc_prologue = description_prologue(pd);
                    assign_port(pd, &svc_name, desc_prologue, None, port_id_map, result);
                } else {
                    // Merge description lines into prologue.
                    let merged_prologue = merge_description_prologue(pd, prologue);
                    assign_port(
                        pd,
                        &base_service_name,
                        merged_prologue,
                        epilogue,
                        port_id_map,
                        result,
                    );
                }
            }
        }
    }
}

// ─── Description-as-prologue helpers ─────────────────────────────────────────

/// Build a prologue string from the port's description lines, or None if empty.
fn description_prologue(pd: &PortData) -> Option<String> {
    if pd.description_lines.is_empty() {
        None
    } else {
        Some(pd.description_lines.join("\n"))
    }
}

/// Merge description lines (prepended) with a computed prologue from deviation analysis.
fn merge_description_prologue(pd: &PortData, other_prologue: Option<String>) -> Option<String> {
    let desc = description_prologue(pd);
    match (desc, other_prologue) {
        (None, None) => None,
        (Some(d), None) => Some(d),
        (None, Some(p)) => Some(p),
        (Some(d), Some(p)) => Some(format!("{}\n{}", d, p)),
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
        vlan: None,
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
    let mut name = derive_service_name(normalized_lines, service_counter);

    // Resolve name collisions: if another service with a different content
    // already uses this name, append a numeric suffix.
    let used_names: std::collections::HashSet<&str> =
        content_to_service.values().map(|n| n.as_str()).collect();
    if used_names.contains(name.as_str()) {
        let base = name.clone();
        let mut suffix = 2;
        loop {
            name = format!("{}-{}", base, suffix);
            if !used_names.contains(name.as_str()) {
                break;
            }
            suffix += 1;
        }
    }

    let vlan = extract_primary_vlan(normalized_lines);
    content_to_service.insert(key, name.clone());
    result.services.push(DerivedService {
        name: name.clone(),
        port_config: port_config.to_string(),
        vlan,
    });
    name
}

/// Extract the primary VLAN number from normalized config lines.
/// For access ports: the access VLAN.
/// For trunk ports: the native VLAN (if set).
/// For other modes: None.
fn extract_primary_vlan(lines: &[String]) -> Option<u32> {
    let is_trunk = lines.iter().any(|l| l == "switchport mode trunk");
    if is_trunk {
        // For trunk ports, use the native VLAN if set
        for line in lines {
            if let Some(rest) = line.strip_prefix("switchport trunk native vlan ") {
                return rest.trim().parse().ok();
            }
        }
        return None;
    }
    for line in lines {
        if let Some(rest) = line.strip_prefix("switchport access vlan ") {
            return rest.trim().parse().ok();
        }
    }
    None
}

/// Derive a service name from normalized config lines.
fn derive_service_name(lines: &[String], counter: &mut usize) -> String {
    // Determine switchport mode first — this drives naming strategy.
    // IOS keeps `switchport access vlan` even on trunk ports, so we must
    // check mode before using access vlan for naming.
    let is_trunk = lines.iter().any(|l| l == "switchport mode trunk");
    let is_access = lines.iter().any(|l| l == "switchport mode access");

    // Channel-group (check first since channel-group ports may also be trunk)
    for line in lines {
        if let Some(rest) = line.strip_prefix("channel-group ") {
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            return format!("channel-group-{}", num);
        }
    }
    // Trunk: check for allowed vlan or just trunk mode
    if is_trunk {
        for line in lines {
            if let Some(rest) = line.strip_prefix("switchport trunk allowed vlan ") {
                let vlan_part = rest.trim().replace(',', "-");
                if vlan_part == "all" {
                    return "trunk-all".to_string();
                }
                return format!("trunk-vlan{}", vlan_part);
            }
        }
        return "trunk-all".to_string();
    }
    // Access vlan (only if actually in access mode or no explicit mode)
    if is_access || !is_trunk {
        for line in lines {
            if let Some(rest) = line.strip_prefix("switchport access vlan ") {
                return format!("access-vlan{}", rest.trim());
            }
        }
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

    // ── Test 12: Unclean split — interleaved non-description lines → new service

    #[test]
    fn test_unclean_split_creates_new_service() {
        // Base: lines A, B, C
        // Port: lines A, X, B, C  → X is interleaved (between A and B)
        // Per spec: if lines can't be cleanly split into prologue+service+epilogue,
        // create a new service instead.
        // NOTE: "description" lines are always-prologue and don't cause unclean splits.
        // Use a non-description interleaved line to test the unclean split path.
        let base_lines = &["switchport mode access", "switchport access vlan 10", "spanning-tree portfast"];
        let interleaved = &[
            "switchport mode access",
            "logging event link-status",   // inserted between line 1 and 2
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

    // ── Test 12b: Description interleaved between template lines → prologue ──

    #[test]
    fn test_description_interleaved_becomes_prologue_not_unclean_split() {
        // Previously this would cause an unclean split; now description lines
        // are stripped before comparison so it matches the template cleanly.
        let base_lines = &["switchport mode access", "switchport access vlan 10", "spanning-tree portfast"];
        let with_desc = &[
            "switchport mode access",
            "description INTERLEAVED",   // description between template lines
            "switchport access vlan 10",
            "spanning-tree portfast",
        ];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", base_lines),
            ("GigabitEthernet0/1", base_lines),
            ("GigabitEthernet0/2", base_lines),
            ("GigabitEthernet0/3", with_desc),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
            ("GigabitEthernet0/2", "Port2"),
            ("GigabitEthernet0/3", "Port3"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // Only 1 service — description is extracted, remaining lines match template
        assert_eq!(result.services.len(), 1, "description should not cause new service");
        let outlier = result.ports.iter().find(|p| p.port_id == "Port3").unwrap();
        assert_eq!(outlier.service_name, "access-vlan10");
        assert!(outlier.prologue.as_deref().unwrap().contains("description INTERLEAVED"));
        assert!(outlier.epilogue.is_none());
    }

    // ── Test 13: Description lines excluded from template, become prologue ───

    #[test]
    fn test_description_lines_become_prologue() {
        // Trunk ports with different descriptions but same structural config.
        // The description should NOT be baked into the service template.
        let port_a = &[
            "description Living Room AP",
            "switchport trunk encapsulation dot1q",
            "switchport trunk native vlan 2",
            "switchport trunk allowed vlan 1-200",
            "switchport mode trunk",
        ];
        let port_b = &[
            "description link to office",
            "switchport trunk encapsulation dot1q",
            "switchport trunk native vlan 2",
            "switchport trunk allowed vlan 1-200",
            "switchport mode trunk",
        ];
        let port_c = &[
            "switchport trunk encapsulation dot1q",
            "switchport trunk native vlan 2",
            "switchport trunk allowed vlan 1-200",
            "switchport mode trunk",
        ];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/7", port_a),
            ("GigabitEthernet0/8", port_b),
            ("GigabitEthernet0/9", port_c),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/7", "Port6"),
            ("GigabitEthernet0/8", "Port7"),
            ("GigabitEthernet0/9", "Port8"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // All 3 ports should use ONE service (descriptions excluded from template)
        assert_eq!(result.services.len(), 1, "should have 1 service, descriptions excluded");
        let svc = &result.services[0];
        assert!(!svc.port_config.contains("description"), "service template must not contain description");

        // Port A should have description as prologue
        let pa = result.ports.iter().find(|p| p.port_id == "Port6").unwrap();
        assert_eq!(pa.service_name, svc.name);
        assert!(pa.prologue.is_some(), "Port A should have description prologue");
        assert!(pa.prologue.as_deref().unwrap().contains("description Living Room AP"));

        // Port B should have description as prologue
        let pb = result.ports.iter().find(|p| p.port_id == "Port7").unwrap();
        assert_eq!(pb.service_name, svc.name);
        assert!(pb.prologue.is_some(), "Port B should have description prologue");
        assert!(pb.prologue.as_deref().unwrap().contains("description link to office"));

        // Port C (no description) should have no prologue
        let pc = result.ports.iter().find(|p| p.port_id == "Port8").unwrap();
        assert_eq!(pc.service_name, svc.name);
        assert!(pc.prologue.is_none(), "Port C (no description) should have no prologue");
    }

    // ── Test 14: Description + other deviation → description as prologue ────

    #[test]
    fn test_description_with_other_epilogue_lines() {
        // Ports with same base config but one has a description AND an extra line.
        let base = &[
            "switchport mode access",
            "switchport access vlan 10",
        ];
        let with_desc_and_extra = &[
            "description SPECIAL",
            "switchport mode access",
            "switchport access vlan 10",
            "no cdp enable",
        ];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/0", base),
            ("GigabitEthernet0/1", base),
            ("GigabitEthernet0/2", base),
            ("GigabitEthernet0/3", with_desc_and_extra),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/0", "Port0"),
            ("GigabitEthernet0/1", "Port1"),
            ("GigabitEthernet0/2", "Port2"),
            ("GigabitEthernet0/3", "Port3"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // 1 service; Port3 has description as prologue and "no cdp enable" as epilogue
        assert_eq!(result.services.len(), 1);
        let outlier = result.ports.iter().find(|p| p.port_id == "Port3").unwrap();
        assert!(outlier.prologue.as_deref().unwrap().contains("description SPECIAL"));
        assert!(outlier.epilogue.as_deref().unwrap().contains("no cdp enable"));
    }

    // ── Test 15: Trunk ports with mixed native-vlan presence ────────────────

    #[test]
    fn test_trunk_ports_mixed_native_vlan() {
        // Some trunk ports have native vlan, some don't.
        // With descriptions stripped, the two groups should emerge.
        let with_native = &[
            "description AP",
            "switchport trunk encapsulation dot1q",
            "switchport trunk native vlan 2",
            "switchport trunk allowed vlan 1-200",
            "switchport mode trunk",
        ];
        let without_native = &[
            "description Uplink",
            "switchport trunk encapsulation dot1q",
            "switchport trunk allowed vlan 1-200",
            "switchport mode trunk",
        ];
        let without_native_no_desc = &[
            "switchport trunk encapsulation dot1q",
            "switchport trunk allowed vlan 1-200",
            "switchport mode trunk",
        ];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/3", with_native),
            ("GigabitEthernet0/7", with_native),
            ("GigabitEthernet0/8", without_native_no_desc),
            ("GigabitEthernet0/9", with_native),
            ("GigabitEthernet0/10", without_native),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/3", "Port2"),
            ("GigabitEthernet0/7", "Port6"),
            ("GigabitEthernet0/8", "Port7"),
            ("GigabitEthernet0/9", "Port8"),
            ("GigabitEthernet0/10", "Port9"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        // With descriptions excluded, with_native group has 3 ports, without has 2.
        // 3 ports with native vlan → most common → base service.
        // 2 ports without native vlan → deviation (<3 but identical deviation).
        // They should either get prologue/epilogue or a second service.
        // Since "missing a template line" can't be expressed as epilogue,
        // they should get their own service (2 identical deviations → still <3, so each
        // gets unclean split → new service via content_to_service dedup).

        // At most 2 services
        assert!(result.services.len() <= 2, "at most 2 services for trunk ports");

        // No service template should contain "description"
        for svc in &result.services {
            assert!(!svc.port_config.contains("description"),
                "service template must not contain description: {}", svc.name);
        }

        // Ports with descriptions should have them as prologue
        let p3 = result.ports.iter().find(|p| p.port_id == "Port2").unwrap();
        assert!(p3.prologue.as_deref().unwrap_or("").contains("description AP"));
        let p7 = result.ports.iter().find(|p| p.port_id == "Port6").unwrap();
        assert!(p7.prologue.as_deref().unwrap_or("").contains("description AP"));
        let p10 = result.ports.iter().find(|p| p.port_id == "Port9").unwrap();
        assert!(p10.prologue.as_deref().unwrap_or("").contains("description Uplink"));

        // Port without description should have no prologue
        let p8 = result.ports.iter().find(|p| p.port_id == "Port7").unwrap();
        assert!(p8.prologue.is_none(), "port without description should have no prologue");
    }

    // ── Test 16: Realistic trunk scenario — 5 ports, mixed descriptions + native vlan ─

    #[test]
    fn test_trunk_mixed_descriptions_native_vlan_and_residual_access() {
        // 5 trunk ports with varying descriptions and native vlan settings.
        // Tests description-as-prologue, service name dedup, and deviation handling.
        //
        // Config groups (after stripping descriptions):
        //   Config A: encap + native-vlan + allowed-vlan + mode (Gi0/7, Gi0/9) → count 2
        //   Config B: encap + allowed-vlan + mode (Gi0/8, Gi0/10) → count 2
        //   Config C: access-vlan + encap + native-vlan + allowed-vlan + mode (Gi0/3) → count 1
        // Tie at 2: first seen = Config A wins as template.
        // Gi0/3 deviates with extra "access vlan 2" line → prologue
        // Gi0/8+Gi0/10 deviate (missing native vlan) → unclean split → new service

        // Use raw lines with leading space, exactly like the IOS parser produces
        let port_blocks_raw: Vec<(String, Vec<String>)> = vec![
            ("GigabitEthernet0/3".into(), vec![
                " switchport access vlan 2".into(),
                " switchport trunk encapsulation dot1q".into(),
                " switchport trunk native vlan 2".into(),
                " switchport trunk allowed vlan 1-200".into(),
                " switchport mode trunk".into(),
            ]),
            ("GigabitEthernet0/7".into(), vec![
                " description Floor 1 AP".into(),
                " switchport trunk encapsulation dot1q".into(),
                " switchport trunk native vlan 2".into(),
                " switchport trunk allowed vlan 1-200".into(),
                " switchport mode trunk".into(),
            ]),
            ("GigabitEthernet0/8".into(), vec![
                " switchport trunk encapsulation dot1q".into(),
                " switchport trunk allowed vlan 1-200".into(),
                " switchport mode trunk".into(),
            ]),
            ("GigabitEthernet0/9".into(), vec![
                " description Floor 2 uplink".into(),
                " switchport trunk encapsulation dot1q".into(),
                " switchport trunk native vlan 2".into(),
                " switchport trunk allowed vlan 1-200".into(),
                " switchport mode trunk".into(),
            ]),
            ("GigabitEthernet0/10".into(), vec![
                " description Floor 3 uplink".into(),
                " switchport trunk encapsulation dot1q".into(),
                " switchport trunk allowed vlan 1-200".into(),
                " switchport mode trunk".into(),
            ]),
        ];
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/3", "Port2"),
            ("GigabitEthernet0/7", "Port6"),
            ("GigabitEthernet0/8", "Port7"),
            ("GigabitEthernet0/9", "Port8"),
            ("GigabitEthernet0/10", "Port9"),
        ]);

        let result = decompose_ports(&port_blocks_raw, &existing, &pid);

        // No service should contain description lines
        for svc in &result.services {
            assert!(!svc.port_config.contains("description"),
                "service '{}' must not contain description line", svc.name);
        }

        // Exactly 2 services: trunk with native vlan (template) + trunk without
        assert_eq!(result.services.len(), 2,
            "expected 2 services, got {}: {:?}",
            result.services.len(),
            result.services.iter().map(|s| &s.name).collect::<Vec<_>>());

        // Service names should be unique (dedup fix)
        assert_ne!(result.services[0].name, result.services[1].name,
            "service names must be unique");

        // Template service should have native vlan
        let template_svc = &result.services[0];
        assert!(template_svc.port_config.contains("native vlan"),
            "template service should contain native vlan");

        // Second service should NOT have native vlan
        let second_svc = &result.services[1];
        assert!(!second_svc.port_config.contains("native vlan"),
            "second service should not contain native vlan");

        // Description ports should have descriptions as prologue
        let p7 = result.ports.iter().find(|p| p.port_id == "Port6").unwrap();
        assert!(p7.prologue.as_deref().unwrap_or("").contains("description Floor 1 AP"));

        let p9 = result.ports.iter().find(|p| p.port_id == "Port8").unwrap();
        assert!(p9.prologue.as_deref().unwrap_or("").contains("description Floor 2 uplink"));

        let p10 = result.ports.iter().find(|p| p.port_id == "Port9").unwrap();
        assert!(p10.prologue.as_deref().unwrap_or("").contains("description Floor 3 uplink"));

        // Gi0/3 should have "access vlan 2" as prologue (residual line on trunk port)
        let p3 = result.ports.iter().find(|p| p.port_id == "Port2").unwrap();
        assert!(p3.prologue.as_deref().unwrap_or("").contains("switchport access vlan 2"),
            "Gi0/3 should have residual access vlan as prologue");

        // Gi0/8 has no description → no description in prologue
        let p8 = result.ports.iter().find(|p| p.port_id == "Port7").unwrap();
        if let Some(ref pro) = p8.prologue {
            assert!(!pro.contains("description"), "Gi0/8 has no description");
        }
    }

    // ── Test 17: Service name collision → unique suffix ─────────────────────

    #[test]
    fn test_service_name_dedup_on_collision() {
        // Two groups that would both derive "trunk-vlan1-100" but have different content.
        // The second should get a "-2" suffix.
        let group_a = &[
            "switchport trunk encapsulation dot1q",
            "switchport trunk native vlan 5",
            "switchport trunk allowed vlan 1-100",
            "switchport mode trunk",
        ];
        let group_b = &[
            "switchport trunk encapsulation dot1q",
            "switchport trunk allowed vlan 1-100",
            "switchport mode trunk",
        ];

        let blocks = port_blocks(&[
            ("GigabitEthernet0/1", group_a),
            ("GigabitEthernet0/2", group_a),
            ("GigabitEthernet0/3", group_a),
            ("GigabitEthernet0/4", group_b),
            ("GigabitEthernet0/5", group_b),
            ("GigabitEthernet0/6", group_b),
        ]);
        let existing: HashMap<String, String> = HashMap::new();
        let pid = pid_map(&[
            ("GigabitEthernet0/1", "Port0"),
            ("GigabitEthernet0/2", "Port1"),
            ("GigabitEthernet0/3", "Port2"),
            ("GigabitEthernet0/4", "Port3"),
            ("GigabitEthernet0/5", "Port4"),
            ("GigabitEthernet0/6", "Port5"),
        ]);

        let result = decompose_ports(&blocks, &existing, &pid);

        assert_eq!(result.services.len(), 2, "should have 2 services");
        let names: Vec<&str> = result.services.iter().map(|s| s.name.as_str()).collect();
        // Both derive "trunk-vlan1-100" but second must be uniquified
        assert!(names.contains(&"trunk-vlan1-100"), "first service should be trunk-vlan1-100");
        assert!(names.contains(&"trunk-vlan1-100-2"), "second service should be trunk-vlan1-100-2");
    }
}
