use anyhow::Result;
use crate::model::{HardwareTemplate, LogicalDeviceConfig, ServiceVars};

pub trait HardwareTemplateSink {
    fn write_hardware_template(&self, sku: &str, template: &HardwareTemplate) -> Result<()>;
}

pub trait ServiceSink {
    fn write_port_config(&self, service_name: &str, content: &str) -> Result<()>;
    fn write_svi_config(&self, service_name: &str, content: &str) -> Result<()>;
    fn write_service_vars(&self, service_name: &str, vars: &ServiceVars) -> Result<()>;
}

pub trait ConfigTemplateSink {
    fn write_template(&self, name: &str, content: &str) -> Result<()>;
}

pub trait ConfigElementSink {
    /// Creates `apply.txt` with `apply_content` and a placeholder `unapply.txt`.
    fn write_element(&self, name: &str, apply_content: &str) -> Result<()>;
}

pub trait LogicalDeviceSink {
    fn write_device_config(&self, device_name: &str, config: &LogicalDeviceConfig) -> Result<()>;
}
