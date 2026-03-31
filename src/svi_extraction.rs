/// Stage 3: SVI Extraction.
///
/// Assigns SVI config blocks to services based on VLAN references.
/// "First service wins" — the first service in creation order that references a VLAN
/// gets the SVI block as its svi-config.txt.

use std::collections::HashMap;

/// An SVI assignment: which service owns this SVI and what config content to write.
pub struct SviAssignment {
    /// Which service gets this SVI (directory name).
    pub service_name: String,
    /// The svi-config.txt content (the interface block lines, normalized).
    pub svi_config: String,
    /// The VLAN number.
    pub vlan: u16,
}

/// Result of SVI extraction.
pub struct SviExtractionResult {
    /// SVIs matched to a service (get written to svi-config.txt).
    pub assignments: Vec<SviAssignment>,
    /// SVIs with no matching service — must be preserved as literal text
    /// in the config template to maintain round-trip correctness.
    pub unmatched: Vec<UnmatchedSvi>,
}

/// An SVI that could not be matched to any service.
pub struct UnmatchedSvi {
    /// The full interface block as literal text (e.g., "interface Vlan99\n ip address ...\n").
    pub literal_text: String,
    /// The VLAN number.
    pub vlan: u16,
}

/// Assign SVIs to services based on VLAN references.
///
/// # Parameters
/// - `svi_blocks` — parsed SVI blocks: `(name, vlan, lines)` where lines include leading whitespace.
/// - `service_vlans` — map of `service_name -> list of VLAN numbers it references`.
/// - `service_creation_order` — services in the order they were created in Stage 2.
///
/// Returns matched assignments and unmatched SVIs separately.
pub fn extract_svis(
    svi_blocks: &[(String, u16, Vec<String>)],
    service_vlans: &HashMap<String, Vec<u16>>,
    service_creation_order: &[String],
) -> SviExtractionResult {
    let mut assignments = Vec::new();
    let mut unmatched = Vec::new();

    for (svi_name, vlan, lines) in svi_blocks {
        // Find the first service (by creation order) that references this VLAN.
        let owner = service_creation_order.iter().find(|svc| {
            service_vlans
                .get(*svc)
                .map(|vlans| vlans.contains(vlan))
                .unwrap_or(false)
        });

        // Build the interface block text
        let mut config = format!("interface {}\n", svi_name);
        for line in lines {
            config.push_str(line);
            config.push('\n');
        }

        if let Some(service_name) = owner {
            assignments.push(SviAssignment {
                service_name: service_name.clone(),
                svi_config: config,
                vlan: *vlan,
            });
        } else {
            unmatched.push(UnmatchedSvi {
                literal_text: config,
                vlan: *vlan,
            });
        }
    }

    SviExtractionResult { assignments, unmatched }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_service_vlans(pairs: &[(&str, &[u16])]) -> HashMap<String, Vec<u16>> {
        pairs
            .iter()
            .map(|(s, v)| (s.to_string(), v.to_vec()))
            .collect()
    }

    // -------------------------------------------------------------------------
    // Test 1: single SVI, one service references its VLAN
    // -------------------------------------------------------------------------

    #[test]
    fn test_single_svi_single_service() {
        let svi_blocks = vec![(
            "Vlan10".to_string(),
            10u16,
            vec![
                " ip address 10.10.0.1 255.255.255.0".to_string(),
                " no shutdown".to_string(),
            ],
        )];
        let service_vlans = make_service_vlans(&[("access-vlan10", &[10])]);
        let order = vec!["access-vlan10".to_string()];

        let result = extract_svis(&svi_blocks, &service_vlans, &order);

        assert_eq!(result.assignments.len(), 1);
        assert_eq!(result.assignments[0].service_name, "access-vlan10");
        assert_eq!(result.assignments[0].vlan, 10);
        assert!(result.assignments[0].svi_config.contains("interface Vlan10"));
        assert!(result.assignments[0].svi_config.contains("ip address 10.10.0.1"));
        assert!(result.unmatched.is_empty());
    }

    // -------------------------------------------------------------------------
    // Test 2: two services reference same VLAN — first service wins
    // -------------------------------------------------------------------------

    #[test]
    fn test_first_service_wins_for_same_vlan() {
        let svi_blocks = vec![(
            "Vlan20".to_string(),
            20u16,
            vec![" ip address 10.20.0.1 255.255.255.0".to_string()],
        )];
        let service_vlans = make_service_vlans(&[
            ("service-a", &[20]),
            ("service-b", &[20]),
        ]);
        // service-a was created first
        let order = vec!["service-a".to_string(), "service-b".to_string()];

        let result = extract_svis(&svi_blocks, &service_vlans, &order);

        assert_eq!(result.assignments.len(), 1);
        assert_eq!(result.assignments[0].service_name, "service-a");
        assert_eq!(result.assignments[0].vlan, 20);
    }

    // -------------------------------------------------------------------------
    // Test 3: SVI with no matching service — not assigned
    // -------------------------------------------------------------------------

    #[test]
    fn test_svi_no_matching_service_not_assigned() {
        let svi_blocks = vec![(
            "Vlan99".to_string(),
            99u16,
            vec![" ip address 10.99.0.1 255.255.255.0".to_string()],
        )];
        // No service references VLAN 99
        let service_vlans = make_service_vlans(&[("access-vlan10", &[10])]);
        let order = vec!["access-vlan10".to_string()];

        let result = extract_svis(&svi_blocks, &service_vlans, &order);

        assert!(result.assignments.is_empty(), "unmatched SVI should not be assigned");
        assert_eq!(result.unmatched.len(), 1, "unmatched SVI should be returned");
        assert_eq!(result.unmatched[0].vlan, 99);
        assert!(result.unmatched[0].literal_text.contains("interface Vlan99"));
    }

    // -------------------------------------------------------------------------
    // Test 4: multiple SVIs, multiple services
    // -------------------------------------------------------------------------

    #[test]
    fn test_multiple_svis_multiple_services() {
        let svi_blocks = vec![
            (
                "Vlan10".to_string(),
                10u16,
                vec![" ip address 10.10.0.1 255.255.255.0".to_string()],
            ),
            (
                "Vlan20".to_string(),
                20u16,
                vec![" ip address 10.20.0.1 255.255.255.0".to_string()],
            ),
            (
                "Vlan30".to_string(),
                30u16,
                vec![" ip address 10.30.0.1 255.255.255.0".to_string()],
            ),
        ];
        let service_vlans = make_service_vlans(&[
            ("svc-a", &[10, 30]),
            ("svc-b", &[20]),
        ]);
        // svc-a was created first
        let order = vec!["svc-a".to_string(), "svc-b".to_string()];

        let result = extract_svis(&svi_blocks, &service_vlans, &order);

        assert_eq!(result.assignments.len(), 3);

        let find_by_vlan = |v: u16| result.assignments.iter().find(|a| a.vlan == v);

        let a10 = find_by_vlan(10).expect("Vlan10 should be assigned");
        assert_eq!(a10.service_name, "svc-a");

        let a20 = find_by_vlan(20).expect("Vlan20 should be assigned");
        assert_eq!(a20.service_name, "svc-b");

        let a30 = find_by_vlan(30).expect("Vlan30 should be assigned");
        assert_eq!(a30.service_name, "svc-a");
    }

    // -------------------------------------------------------------------------
    // Test 5: order of service_creation_order determines winner even when
    //         service_vlans lists the second service first lexicographically
    // -------------------------------------------------------------------------

    #[test]
    fn test_creation_order_not_alphabetical() {
        let svi_blocks = vec![(
            "Vlan5".to_string(),
            5u16,
            vec![" ip address 10.5.0.1 255.255.255.0".to_string()],
        )];
        let service_vlans = make_service_vlans(&[
            ("alpha", &[5]),
            ("zeta", &[5]),
        ]);
        // zeta was created before alpha
        let order = vec!["zeta".to_string(), "alpha".to_string()];

        let result = extract_svis(&svi_blocks, &service_vlans, &order);

        assert_eq!(result.assignments.len(), 1);
        assert_eq!(result.assignments[0].service_name, "zeta");
    }

    // -------------------------------------------------------------------------
    // Test 6: svi_config content is correctly formatted
    // -------------------------------------------------------------------------

    #[test]
    fn test_svi_config_format() {
        let svi_blocks = vec![(
            "Vlan42".to_string(),
            42u16,
            vec![
                " description Management VLAN".to_string(),
                " ip address 192.168.42.1 255.255.255.0".to_string(),
            ],
        )];
        let service_vlans = make_service_vlans(&[("mgmt-service", &[42])]);
        let order = vec!["mgmt-service".to_string()];

        let result = extract_svis(&svi_blocks, &service_vlans, &order);

        assert_eq!(result.assignments.len(), 1);
        let config = &result.assignments[0].svi_config;
        // Must start with "interface Vlan42\n"
        assert!(config.starts_with("interface Vlan42\n"), "config: {:?}", config);
        assert!(config.contains(" description Management VLAN\n"));
        assert!(config.contains(" ip address 192.168.42.1 255.255.255.0\n"));
    }

    // -------------------------------------------------------------------------
    // Test 7: empty svi_blocks yields empty result
    // -------------------------------------------------------------------------

    #[test]
    fn test_no_svi_blocks() {
        let service_vlans = make_service_vlans(&[("access-vlan10", &[10])]);
        let order = vec!["access-vlan10".to_string()];

        let result = extract_svis(&[], &service_vlans, &order);

        assert!(result.assignments.is_empty());
        assert!(result.unmatched.is_empty());
    }
}
