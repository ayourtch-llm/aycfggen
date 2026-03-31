use std::path::PathBuf;
use anyhow::{Context, Result};
use crate::model::{HardwareTemplate, LogicalDeviceConfig};
use crate::sinks::{
    HardwareTemplateSink, ServiceSink, ConfigTemplateSink, ConfigElementSink, LogicalDeviceSink,
};

const UNAPPLY_PLACEHOLDER: &str = "! FIXME - needs to be generated\n";

// ---------------------------------------------------------------------------
// FsHardwareTemplateSink
// ---------------------------------------------------------------------------

pub struct FsHardwareTemplateSink {
    pub dir: PathBuf,
}

impl FsHardwareTemplateSink {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl HardwareTemplateSink for FsHardwareTemplateSink {
    fn write_hardware_template(&self, sku: &str, template: &HardwareTemplate) -> Result<()> {
        let dir = self.dir.join(sku);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create hardware template directory for SKU {:?}: {}", sku, dir.display()))?;
        let path = dir.join("ports.json");
        let json = serde_json::to_string_pretty(template)
            .with_context(|| format!("failed to serialize hardware template for SKU {:?}", sku))?;
        std::fs::write(&path, json)
            .with_context(|| format!("failed to write ports.json for SKU {:?}: {}", sku, path.display()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FsServiceSink
// ---------------------------------------------------------------------------

pub struct FsServiceSink {
    pub dir: PathBuf,
}

impl FsServiceSink {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl ServiceSink for FsServiceSink {
    fn write_port_config(&self, service_name: &str, content: &str) -> Result<()> {
        let dir = self.dir.join(service_name);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create service directory for {:?}: {}", service_name, dir.display()))?;
        let path = dir.join("port-config.txt");
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write port-config.txt for service {:?}: {}", service_name, path.display()))?;
        Ok(())
    }

    fn write_svi_config(&self, service_name: &str, content: &str) -> Result<()> {
        let dir = self.dir.join(service_name);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create service directory for {:?}: {}", service_name, dir.display()))?;
        let path = dir.join("svi-config.txt");
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write svi-config.txt for service {:?}: {}", service_name, path.display()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FsConfigTemplateSink
// ---------------------------------------------------------------------------

pub struct FsConfigTemplateSink {
    pub dir: PathBuf,
}

impl FsConfigTemplateSink {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl ConfigTemplateSink for FsConfigTemplateSink {
    fn write_template(&self, name: &str, content: &str) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("failed to create config templates directory: {}", self.dir.display()))?;
        let path = self.dir.join(name);
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write config template {:?}: {}", name, path.display()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FsConfigElementSink
// ---------------------------------------------------------------------------

pub struct FsConfigElementSink {
    pub dir: PathBuf,
}

impl FsConfigElementSink {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl ConfigElementSink for FsConfigElementSink {
    fn write_element(&self, name: &str, apply_content: &str) -> Result<()> {
        let dir = self.dir.join(name);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create config element directory for {:?}: {}", name, dir.display()))?;

        let apply_path = dir.join("apply.txt");
        std::fs::write(&apply_path, apply_content)
            .with_context(|| format!("failed to write apply.txt for element {:?}: {}", name, apply_path.display()))?;

        let unapply_path = dir.join("unapply.txt");
        std::fs::write(&unapply_path, UNAPPLY_PLACEHOLDER)
            .with_context(|| format!("failed to write unapply.txt for element {:?}: {}", name, unapply_path.display()))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FsLogicalDeviceSink
// ---------------------------------------------------------------------------

pub struct FsLogicalDeviceSink {
    pub dir: PathBuf,
}

impl FsLogicalDeviceSink {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl LogicalDeviceSink for FsLogicalDeviceSink {
    fn write_device_config(&self, device_name: &str, config: &LogicalDeviceConfig) -> Result<()> {
        let dir = self.dir.join(device_name);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create logical device directory for {:?}: {}", device_name, dir.display()))?;
        let path = dir.join("config.json");
        let json = serde_json::to_string_pretty(config)
            .with_context(|| format!("failed to serialize device config for {:?}", device_name))?;
        std::fs::write(&path, json)
            .with_context(|| format!("failed to write config.json for device {:?}: {}", device_name, path.display()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HardwareTemplate, LogicalDeviceConfig, Module, PortAssignment, PortDefinition};
    use indexmap::IndexMap;
    use std::path::PathBuf;

    /// Create a unique temporary directory for a test, returns the path.
    fn make_temp_dir(suffix: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("aycfggen_sinks_test_{}", suffix));
        std::fs::create_dir_all(&base).expect("create temp dir");
        base
    }

    fn cleanup(dir: &PathBuf) {
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── FsHardwareTemplateSink ────────────────────────────────────────────────

    #[test]
    fn test_fs_hardware_template_sink_writes_ports_json() {
        let tmp = make_temp_dir("hw_template");
        let sink = FsHardwareTemplateSink::new(tmp.clone());

        let mut ports = IndexMap::new();
        ports.insert("Port0".to_string(), PortDefinition {
            name: "GigabitEthernet".to_string(),
            index: "0/0".to_string(),
        });
        ports.insert("Port1".to_string(), PortDefinition {
            name: "GigabitEthernet".to_string(),
            index: "0/1".to_string(),
        });
        let template = HardwareTemplate {
            vendor: Some("cisco-ios".to_string()),
            slot_index_base: None,
            ports,
        };

        sink.write_hardware_template("WS-C3560-24TS", &template).expect("write hardware template");

        let path = tmp.join("WS-C3560-24TS").join("ports.json");
        assert!(path.exists(), "ports.json should exist at {}", path.display());

        let data = std::fs::read_to_string(&path).expect("read ports.json");
        let parsed: HardwareTemplate = serde_json::from_str(&data).expect("parse ports.json");
        assert_eq!(parsed.ports.len(), 2);
        assert!(parsed.ports.contains_key("Port0"));
        assert_eq!(parsed.ports["Port0"].name, "GigabitEthernet");
        assert_eq!(parsed.ports["Port0"].index, "0/0");

        cleanup(&tmp);
    }

    // ── FsServiceSink ─────────────────────────────────────────────────────────

    #[test]
    fn test_fs_service_sink_writes_port_config() {
        let tmp = make_temp_dir("service_sink_pc");
        let sink = FsServiceSink::new(tmp.clone());

        let content = "switchport mode access\nswitchport access vlan 10\n";
        sink.write_port_config("access-vlan10", content).expect("write port config");

        let path = tmp.join("access-vlan10").join("port-config.txt");
        assert!(path.exists(), "port-config.txt should exist");
        let data = std::fs::read_to_string(&path).expect("read port-config.txt");
        assert_eq!(data, content);

        cleanup(&tmp);
    }

    #[test]
    fn test_fs_service_sink_writes_svi_config() {
        let tmp = make_temp_dir("service_sink_svi");
        let sink = FsServiceSink::new(tmp.clone());

        let content = "interface Vlan10\n ip address 10.0.0.1 255.255.255.0\n";
        sink.write_svi_config("access-vlan10", content).expect("write svi config");

        let path = tmp.join("access-vlan10").join("svi-config.txt");
        assert!(path.exists(), "svi-config.txt should exist");
        let data = std::fs::read_to_string(&path).expect("read svi-config.txt");
        assert_eq!(data, content);

        cleanup(&tmp);
    }

    // ── FsConfigTemplateSink ──────────────────────────────────────────────────

    #[test]
    fn test_fs_config_template_sink_writes_template() {
        let tmp = make_temp_dir("cfg_template_sink");
        let sink = FsConfigTemplateSink::new(tmp.clone());

        let content = "hostname switch1\n!!!###logging-standard\n<PORTS-CONFIGURATION>\n";
        sink.write_template("switch1-FOC123.conf", content).expect("write template");

        let path = tmp.join("switch1-FOC123.conf");
        assert!(path.exists(), "template file should exist");
        let data = std::fs::read_to_string(&path).expect("read template");
        assert_eq!(data, content);

        cleanup(&tmp);
    }

    // ── FsConfigElementSink ───────────────────────────────────────────────────

    #[test]
    fn test_fs_config_element_sink_creates_apply_and_unapply() {
        let tmp = make_temp_dir("cfg_element_sink");
        let sink = FsConfigElementSink::new(tmp.clone());

        let apply_content = "logging buffered 16384\nlogging console informational\n";
        sink.write_element("logging-standard", apply_content).expect("write element");

        let apply_path = tmp.join("logging-standard").join("apply.txt");
        let unapply_path = tmp.join("logging-standard").join("unapply.txt");

        assert!(apply_path.exists(), "apply.txt should exist");
        assert!(unapply_path.exists(), "unapply.txt should exist");

        let apply_data = std::fs::read_to_string(&apply_path).expect("read apply.txt");
        assert_eq!(apply_data, apply_content);

        let unapply_data = std::fs::read_to_string(&unapply_path).expect("read unapply.txt");
        assert_eq!(unapply_data, UNAPPLY_PLACEHOLDER);

        cleanup(&tmp);
    }

    // ── FsLogicalDeviceSink ───────────────────────────────────────────────────

    #[test]
    fn test_fs_logical_device_sink_writes_config_json() {
        let tmp = make_temp_dir("logical_device_sink");
        let sink = FsLogicalDeviceSink::new(tmp.clone());

        let config = LogicalDeviceConfig {
            config_template: "switch1-FOC1234X0AB.conf".to_string(),
            software_image: Some("c3560-ipbasek9-mz.150-2.SE11.bin".to_string()),
            role: Some("discovered".to_string()),
            vendor: None,
            omit_slot_prefix: true,
            slot_index_base: None,
            vars: IndexMap::new(),
            modules: vec![Some(Module {
                sku: "WS-C3560-24TS".to_string(),
                serial: Some("FOC1234X0AB".to_string()),
                ports: vec![
                    PortAssignment {
                        name: "Port0".to_string(),
                        service: "access-vlan10".to_string(),
                        prologue: None,
                        epilogue: None,
                        vars: IndexMap::new(),
                    },
                ],
            })],
        };

        sink.write_device_config("FOC1234X0AB", &config).expect("write device config");

        let path = tmp.join("FOC1234X0AB").join("config.json");
        assert!(path.exists(), "config.json should exist at {}", path.display());

        let data = std::fs::read_to_string(&path).expect("read config.json");
        let parsed: LogicalDeviceConfig = serde_json::from_str(&data).expect("parse config.json");
        assert_eq!(parsed.config_template, "switch1-FOC1234X0AB.conf");
        assert_eq!(parsed.role.as_deref(), Some("discovered"));
        assert_eq!(parsed.software_image.as_deref(), Some("c3560-ipbasek9-mz.150-2.SE11.bin"));
        assert!(parsed.omit_slot_prefix);
        assert_eq!(parsed.modules.len(), 1);
        let m = parsed.modules[0].as_ref().expect("module");
        assert_eq!(m.sku, "WS-C3560-24TS");
        assert_eq!(m.serial.as_deref(), Some("FOC1234X0AB"));
        assert_eq!(m.ports.len(), 1);
        assert_eq!(m.ports[0].name, "Port0");
        assert_eq!(m.ports[0].service, "access-vlan10");

        cleanup(&tmp);
    }

    #[test]
    fn test_fs_logical_device_sink_writes_config_json_with_null_module() {
        let tmp = make_temp_dir("logical_device_sink_null");
        let sink = FsLogicalDeviceSink::new(tmp.clone());

        let config = LogicalDeviceConfig {
            config_template: "router1.conf".to_string(),
            software_image: None,
            role: Some("discovered".to_string()),
            vendor: None,
            omit_slot_prefix: false,
            slot_index_base: Some(0),
            vars: IndexMap::new(),
            modules: vec![
                None,
                Some(Module {
                    sku: "NIM-4GE".to_string(),
                    serial: Some("FOC9876Y0CD".to_string()),
                    ports: vec![],
                }),
            ],
        };

        sink.write_device_config("FOC9876Y0CD", &config).expect("write device config");

        let path = tmp.join("FOC9876Y0CD").join("config.json");
        let data = std::fs::read_to_string(&path).expect("read config.json");
        let parsed: LogicalDeviceConfig = serde_json::from_str(&data).expect("parse config.json");
        assert_eq!(parsed.modules.len(), 2);
        assert!(parsed.modules[0].is_none(), "first module should be null");
        assert_eq!(parsed.slot_index_base, Some(0));

        cleanup(&tmp);
    }
}
