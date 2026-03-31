/// Stage 5: Variable Extraction (no-op implementation).
///
/// This stage processes all artifacts produced by Stages 1–4, potentially
/// replacing literal values with `{{variable}}` references and storing
/// concrete values in the device vars. The default implementation is a
/// no-op that passes everything through unchanged until aycfggen implements
/// `{{variable}}` expansion.

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
                },
                ServiceArtifact {
                    name: "shutdown".to_string(),
                    port_config: "shutdown\n".to_string(),
                    svi_config: None,
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
}
