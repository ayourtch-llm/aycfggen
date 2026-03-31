pub mod cli;
pub mod model;
pub mod sources;
pub mod fs_sources;
pub mod sinks;
pub mod fs_sinks;
pub mod extract;
pub mod interface_name;
pub mod compile;
pub mod validate;
pub mod output;
pub mod ios_parser;
pub mod show_parsers;
pub mod hardware_discovery;
pub mod port_decomposition;
pub mod svi_extraction;
pub mod template_builder;
pub mod variable_extraction;
pub mod round_trip;
pub mod extract_cli;

/// Regex pattern for config element markers: `!!!###<element-name>`
/// Element names must match `[a-zA-Z0-9_-]+`.
pub const CONFIG_ELEMENT_MARKER_PATTERN: &str = r"^!!!###([a-zA-Z0-9_-]+)$";
