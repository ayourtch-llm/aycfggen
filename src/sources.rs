use anyhow::Result;
use crate::model::*;

pub trait HardwareTemplateSource {
    fn load_hardware_template(&self, sku: &str) -> Result<HardwareTemplate>;
}

pub trait LogicalDeviceSource {
    fn load_device_config(&self, device_name: &str) -> Result<LogicalDeviceConfig>;
    fn list_devices(&self) -> Result<Vec<String>>;
}

pub trait ServiceSource {
    fn load_port_config(&self, service_name: &str) -> Result<String>;
    fn load_svi_config(&self, service_name: &str) -> Result<Option<String>>;
}

pub trait ConfigTemplateSource {
    fn load_template(&self, template_name: &str) -> Result<String>;
}

pub trait ConfigElementSource {
    fn load_apply(&self, element_name: &str) -> Result<String>;
}

pub trait SoftwareImageSource {
    fn validate_exists(&self, image_name: &str) -> Result<()>;
}
