use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Per-service metadata stored in `vars.json`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServiceVars {
    /// Primary VLAN number for this service (access VLAN, SVI VLAN, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareTemplate {
    pub vendor: Option<String>,
    #[serde(rename = "slot-index-base")]
    pub slot_index_base: Option<u32>,
    pub ports: IndexMap<String, PortDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortDefinition {
    pub name: String,
    pub index: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogicalDeviceConfig {
    #[serde(rename = "config-template")]
    pub config_template: String,
    #[serde(rename = "software-image")]
    pub software_image: Option<String>,
    pub role: Option<String>,
    pub vendor: Option<String>,
    #[serde(rename = "omit-slot-prefix", default)]
    pub omit_slot_prefix: bool,
    #[serde(rename = "slot-index-base")]
    pub slot_index_base: Option<u32>,
    #[serde(default)]
    pub vars: IndexMap<String, String>,
    /// Additional services whose SVIs should be included in this device's SVI block.
    /// Covers standalone SVI services (VLANs not associated with any port service).
    #[serde(rename = "svi-services", default, skip_serializing_if = "Vec::is_empty")]
    pub svi_services: Vec<String>,
    pub modules: Vec<Option<Module>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
    #[serde(rename = "SKU")]
    pub sku: String,
    pub serial: Option<String>,
    pub ports: Vec<PortAssignment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortAssignment {
    pub name: String,
    pub service: String,
    pub prologue: Option<String>,
    pub epilogue: Option<String>,
    #[serde(default)]
    pub vars: IndexMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn example_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/examples")
    }

    #[test]
    fn test_deserialize_hardware_template_set1() {
        let path = example_dir()
            .join("set1/hardware-templates/WS-C3560-24TS/ports.json");
        let data = std::fs::read_to_string(&path).expect("read ports.json");
        let tmpl: HardwareTemplate =
            serde_json::from_str(&data).expect("deserialize HardwareTemplate");

        assert_eq!(tmpl.ports.len(), 4);
        let port0 = tmpl.ports.get("Port0").expect("Port0 exists");
        assert_eq!(port0.name, "GigabitEthernet");
        assert_eq!(port0.index, "0/0");
    }

    #[test]
    fn test_deserialize_device_config_set1() {
        let path = example_dir()
            .join("set1/logical-devices/switch1/config.json");
        let data = std::fs::read_to_string(&path).expect("read config.json");
        let cfg: LogicalDeviceConfig =
            serde_json::from_str(&data).expect("deserialize LogicalDeviceConfig");

        assert_eq!(cfg.config_template, "access-switch.conf");
        assert!(cfg.omit_slot_prefix);
        assert_eq!(cfg.modules.len(), 1);

        let module = cfg.modules[0].as_ref().expect("first module is Some");
        assert_eq!(module.ports.len(), 4);
    }

    #[test]
    fn test_deserialize_device_config_set2() {
        let path = example_dir()
            .join("set2/logical-devices/router1/config.json");
        let data = std::fs::read_to_string(&path).expect("read config.json");
        let cfg: LogicalDeviceConfig =
            serde_json::from_str(&data).expect("deserialize LogicalDeviceConfig");

        assert_eq!(cfg.modules.len(), 3);
        assert!(cfg.modules[0].is_none(), "first module slot is null");
        assert_eq!(cfg.slot_index_base, Some(0));
    }

    #[test]
    fn test_deserialize_hardware_template_set2() {
        let path = example_dir()
            .join("set2/hardware-templates/NIM-4GE/ports.json");
        let data = std::fs::read_to_string(&path).expect("read ports.json");
        let tmpl: HardwareTemplate =
            serde_json::from_str(&data).expect("deserialize HardwareTemplate");

        assert_eq!(tmpl.ports.len(), 4);
        assert_eq!(tmpl.vendor.as_deref(), Some("cisco-ios"));
    }

    #[test]
    fn test_unknown_fields_ignored() {
        let json = r#"{
            "config-template": "test.conf",
            "modules": [],
            "unknown-field": "should be ignored",
            "another-unknown": 42
        }"#;
        let result: Result<LogicalDeviceConfig, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "unknown fields should not cause an error");
    }

    #[test]
    fn test_default_values() {
        let json = r#"{
            "config-template": "test.conf",
            "modules": []
        }"#;
        let cfg: LogicalDeviceConfig =
            serde_json::from_str(json).expect("deserialize minimal config");

        assert!(!cfg.omit_slot_prefix, "omit_slot_prefix defaults to false");
        assert!(cfg.vars.is_empty(), "vars defaults to empty");
        assert!(cfg.software_image.is_none());
        assert!(cfg.role.is_none());
        assert!(cfg.vendor.is_none());
        assert!(cfg.slot_index_base.is_none());
    }
}
