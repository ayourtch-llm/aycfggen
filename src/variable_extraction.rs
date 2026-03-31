/// Stage 5: Variable Extraction.
///
/// This stage processes all artifacts produced by Stages 1–4, potentially
/// replacing literal values with `{{{variable}}}` references and storing
/// concrete values in the device vars. The default implementation chains
/// HostnameExtractor and VlanIdExtractor together.

use std::collections::HashMap;

/// All artifacts produced by stages 1-4 that may be modified by variable extraction.
pub struct ExtractionArtifacts {
    pub template_content: String,
    pub services: Vec<ServiceArtifact>,
    pub device_vars: HashMap<String, String>,
}

/// A service artifact: per-port config and optional SVI config for one service.
pub struct ServiceArtifact {
    pub name: String,
    pub port_config: String,
    pub svi_config: Option<String>,
    /// Per-service variables extracted from port_config (e.g., vlan_id).
    pub vars: HashMap<String, String>,
}

/// Trait for variable extraction.
///
/// Each implementation identifies parameterizable values and replaces them
/// with `{{variable}}` references, storing concrete values in device vars.
/// For now, the default no-op implementation passes everything through unchanged.
pub trait VariableExtractor {
    /// Process all artifacts and return them (possibly modified).
    fn extract(&self, artifacts: ExtractionArtifacts) -> ExtractionArtifacts;
}

/// No-op implementation that passes everything through unchanged.
pub struct NoOpExtractor;

impl VariableExtractor for NoOpExtractor {
    fn extract(&self, artifacts: ExtractionArtifacts) -> ExtractionArtifacts {
        artifacts
    }
}

/// Extracts the hostname from `hostname <name>` in the template content.
///
/// Replaces the literal hostname line with `hostname {{{hostname}}}` and
/// stores the concrete hostname in `device_vars`.
pub struct HostnameExtractor;

impl VariableExtractor for HostnameExtractor {
    fn extract(&self, mut artifacts: ExtractionArtifacts) -> ExtractionArtifacts {
        let mut found_hostname: Option<String> = None;
        let new_content: String = artifacts
            .template_content
            .lines()
            .map(|line| {
                if let Some(name) = line.strip_prefix("hostname ") {
                    let name = name.trim().to_string();
                    if !name.is_empty() && found_hostname.is_none() {
                        found_hostname = Some(name);
                        return "hostname {{{hostname}}}".to_string();
                    }
                }
                line.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Preserve trailing newline if original had one
        let new_content = if artifacts.template_content.ends_with('\n') {
            new_content + "\n"
        } else {
            new_content
        };

        artifacts.template_content = new_content;
        if let Some(hostname) = found_hostname {
            artifacts.device_vars.insert("hostname".to_string(), hostname);
        }
        artifacts
    }
}

/// Extracts VLAN IDs from `switchport access vlan <N>` in each service's port_config.
///
/// Replaces the literal VLAN line with `switchport access vlan {{{vlan_id}}}` and
/// stores the concrete VLAN ID in the service's `vars` map.
pub struct VlanIdExtractor;

impl VariableExtractor for VlanIdExtractor {
    fn extract(&self, mut artifacts: ExtractionArtifacts) -> ExtractionArtifacts {
        for svc in &mut artifacts.services {
            let mut found_vlan: Option<String> = None;
            let new_config: String = svc
                .port_config
                .lines()
                .map(|line| {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("switchport access vlan ") {
                        let vlan = rest.trim().to_string();
                        if !vlan.is_empty() && found_vlan.is_none() {
                            found_vlan = Some(vlan);
                            // Preserve original indentation
                            let indent = &line[..line.len() - trimmed.len()];
                            return indent.to_string() + "switchport access vlan {{{vlan_id}}}";
                        }
                    }
                    line.to_string()
                })
                .collect::<Vec<_>>()
                .join("\n");

            let new_config = if svc.port_config.ends_with('\n') {
                new_config + "\n"
            } else {
                new_config
            };

            svc.port_config = new_config;
            if let Some(vlan) = found_vlan {
                svc.vars.insert("vlan_id".to_string(), vlan);
            }
        }
        artifacts
    }
}

/// Default extractor: chains HostnameExtractor and VlanIdExtractor in sequence.
pub struct DefaultExtractor;

impl VariableExtractor for DefaultExtractor {
    fn extract(&self, artifacts: ExtractionArtifacts) -> ExtractionArtifacts {
        let artifacts = HostnameExtractor.extract(artifacts);
        VlanIdExtractor.extract(artifacts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_artifacts() -> ExtractionArtifacts {
        let mut vars = HashMap::new();
        vars.insert("hostname".to_string(), "switch1".to_string());

        ExtractionArtifacts {
            template_content: "hostname switch1\n".to_string(),
            services: vec![
                ServiceArtifact {
                    name: "access-vlan10".to_string(),
                    port_config: "switchport mode access\nswitchport access vlan 10\n".to_string(),
                    svi_config: Some("interface Vlan10\n ip address 10.0.0.1 255.255.255.0\n".to_string()),
                    vars: HashMap::new(),
                },
                ServiceArtifact {
                    name: "shutdown".to_string(),
                    port_config: "shutdown\n".to_string(),
                    svi_config: None,
                    vars: HashMap::new(),
                },
            ],
            device_vars: vars,
        }
    }

    #[test]
    fn test_no_op_extractor_passes_template_through() {
        let extractor = NoOpExtractor;
        let artifacts = make_artifacts();
        let original_template = artifacts.template_content.clone();
        let result = extractor.extract(artifacts);
        assert_eq!(result.template_content, original_template);
    }

    #[test]
    fn test_no_op_extractor_passes_services_through() {
        let extractor = NoOpExtractor;
        let artifacts = make_artifacts();
        let result = extractor.extract(artifacts);
        assert_eq!(result.services.len(), 2);
        assert_eq!(result.services[0].name, "access-vlan10");
        assert_eq!(
            result.services[0].port_config,
            "switchport mode access\nswitchport access vlan 10\n"
        );
        assert_eq!(
            result.services[0].svi_config,
            Some("interface Vlan10\n ip address 10.0.0.1 255.255.255.0\n".to_string())
        );
        assert_eq!(result.services[1].name, "shutdown");
        assert_eq!(result.services[1].port_config, "shutdown\n");
        assert!(result.services[1].svi_config.is_none());
    }

    #[test]
    fn test_no_op_extractor_passes_device_vars_through() {
        let extractor = NoOpExtractor;
        let artifacts = make_artifacts();
        let result = extractor.extract(artifacts);
        assert_eq!(result.device_vars.get("hostname"), Some(&"switch1".to_string()));
    }

    #[test]
    fn test_variable_extractor_is_object_safe() {
        // If this compiles, the trait is object-safe.
        let extractor: Box<dyn VariableExtractor> = Box::new(NoOpExtractor);
        let artifacts = ExtractionArtifacts {
            template_content: String::new(),
            services: vec![],
            device_vars: HashMap::new(),
        };
        let result = extractor.extract(artifacts);
        assert!(result.services.is_empty());
    }

    // ── HostnameExtractor tests ───────────────────────────────────────────────

    #[test]
    fn test_hostname_extractor_replaces_hostname_line() {
        let extractor = HostnameExtractor;
        let artifacts = ExtractionArtifacts {
            template_content: "hostname switch1\nsome other line\n".to_string(),
            services: vec![],
            device_vars: HashMap::new(),
        };
        let result = extractor.extract(artifacts);
        assert_eq!(result.template_content, "hostname {{{hostname}}}\nsome other line\n");
        assert_eq!(result.device_vars.get("hostname"), Some(&"switch1".to_string()));
    }

    #[test]
    fn test_hostname_extractor_no_hostname_line_unchanged() {
        let extractor = HostnameExtractor;
        let template = "ip domain-name example.com\nsome config\n".to_string();
        let artifacts = ExtractionArtifacts {
            template_content: template.clone(),
            services: vec![],
            device_vars: HashMap::new(),
        };
        let result = extractor.extract(artifacts);
        assert_eq!(result.template_content, template);
        assert!(result.device_vars.get("hostname").is_none());
    }

    #[test]
    fn test_hostname_extractor_does_not_touch_services() {
        let extractor = HostnameExtractor;
        let artifacts = ExtractionArtifacts {
            template_content: "hostname router1\n".to_string(),
            services: vec![
                ServiceArtifact {
                    name: "access-vlan10".to_string(),
                    port_config: "switchport mode access\nswitchport access vlan 10\n".to_string(),
                    svi_config: None,
                    vars: HashMap::new(),
                },
            ],
            device_vars: HashMap::new(),
        };
        let result = extractor.extract(artifacts);
        assert_eq!(result.services[0].port_config, "switchport mode access\nswitchport access vlan 10\n");
    }

    // ── VlanIdExtractor tests ─────────────────────────────────────────────────

    #[test]
    fn test_vlan_id_extractor_replaces_access_vlan_line() {
        let extractor = VlanIdExtractor;
        let artifacts = ExtractionArtifacts {
            template_content: String::new(),
            services: vec![
                ServiceArtifact {
                    name: "access-vlan10".to_string(),
                    port_config: "switchport mode access\nswitchport access vlan 10\n".to_string(),
                    svi_config: None,
                    vars: HashMap::new(),
                },
            ],
            device_vars: HashMap::new(),
        };
        let result = extractor.extract(artifacts);
        assert_eq!(
            result.services[0].port_config,
            "switchport mode access\nswitchport access vlan {{{vlan_id}}}\n"
        );
        assert_eq!(result.services[0].vars.get("vlan_id"), Some(&"10".to_string()));
    }

    #[test]
    fn test_vlan_id_extractor_trunk_service_unchanged() {
        let extractor = VlanIdExtractor;
        let port_config = "switchport mode trunk\nswitchport trunk allowed vlan 10,20\n".to_string();
        let artifacts = ExtractionArtifacts {
            template_content: String::new(),
            services: vec![
                ServiceArtifact {
                    name: "trunk-vlan10-20".to_string(),
                    port_config: port_config.clone(),
                    svi_config: None,
                    vars: HashMap::new(),
                },
            ],
            device_vars: HashMap::new(),
        };
        let result = extractor.extract(artifacts);
        assert_eq!(result.services[0].port_config, port_config);
        assert!(result.services[0].vars.get("vlan_id").is_none());
    }

    #[test]
    fn test_vlan_id_extractor_does_not_touch_template() {
        let extractor = VlanIdExtractor;
        let template = "hostname switch1\n".to_string();
        let artifacts = ExtractionArtifacts {
            template_content: template.clone(),
            services: vec![],
            device_vars: HashMap::new(),
        };
        let result = extractor.extract(artifacts);
        assert_eq!(result.template_content, template);
    }

    // ── DefaultExtractor tests ────────────────────────────────────────────────

    #[test]
    fn test_default_extractor_combines_hostname_and_vlan() {
        let extractor = DefaultExtractor;
        let artifacts = ExtractionArtifacts {
            template_content: "hostname myswitch\nip domain-name example.com\n".to_string(),
            services: vec![
                ServiceArtifact {
                    name: "access-vlan20".to_string(),
                    port_config: "switchport mode access\nswitchport access vlan 20\n".to_string(),
                    svi_config: None,
                    vars: HashMap::new(),
                },
            ],
            device_vars: HashMap::new(),
        };
        let result = extractor.extract(artifacts);
        // Hostname extraction
        assert_eq!(result.template_content, "hostname {{{hostname}}}\nip domain-name example.com\n");
        assert_eq!(result.device_vars.get("hostname"), Some(&"myswitch".to_string()));
        // VLAN extraction
        assert_eq!(
            result.services[0].port_config,
            "switchport mode access\nswitchport access vlan {{{vlan_id}}}\n"
        );
        assert_eq!(result.services[0].vars.get("vlan_id"), Some(&"20".to_string()));
    }
}
