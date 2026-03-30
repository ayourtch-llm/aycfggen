# Data Model Specification

All hashmaps referenced in this specification are order-preserving.
All JSON deserialization must be lenient by default — unknown fields are ignored.
When `--strict` mode is enabled, unknown fields cause an error.
Port identifiers (e.g., `Port0`) are case-sensitive.

All data interfaces (hardware templates, logical devices, services, config templates) must be accessed through trait abstractions that allow future substitution of alternative data sources (e.g., databases, APIs). The initial implementation provides filesystem-based backends.

## 1. Hardware Templates

**Location:** `<config-root>/hardware-templates/<SKU>/`

Each SKU directory contains platform-specific mappings for a given hardware part number.

### ports.json

A JSON object with the following top-level fields:

| Field              | Type    | Required | Description |
|--------------------|---------|----------|-------------|
| `vendor`           | String  | No       | Vendor identifier, e.g., `"cisco-ios"`, `"juniper-junos"`. Reserved for future multi-vendor support. Not used by the compiler yet. |
| `slot-index-base`  | Integer | No       | The starting slot index for this hardware platform (e.g., `0` or `1`). Default: `0`. Used in interface name derivation when the logical device does not override it. |
| `ports`            | OrderedMap<String, PortDefinition> | Yes | Port definitions, keyed by identifiers of the form `Port<N>` (e.g., `Port0`, `Port1`, ...). |

### PortDefinition record

| Field   | Type   | Required | Description |
|---------|--------|----------|-------------|
| `name`  | String | Yes      | Human-readable interface name prefix, e.g., `"GigabitEthernet"`, `"Ethernet"` |
| `index` | String | Yes      | Device-local interface index, e.g., `"0/0"`, `"1"` |

Additional fields may be present and must be preserved (but not cause errors unless `--strict`).

---

## 2. Logical Devices

**Location:** `<config-root>/logical-devices/<device-name>/`

Each logical device directory is named after the device and contains its configuration.

### config.json

A JSON object with the following fields:

| Field              | Type                        | Required | Description |
|--------------------|-----------------------------|----------|-------------|
| `config-template`  | String                      | Yes      | Filename of the configuration template. If relative, resolved from `<config-templates-dir>`. |
| `software`         | String                      | No       | Filename of software image. If relative, resolved from `<software-images-dir>`. Stored but not used during compilation. |
| `role`             | String                      | No       | Free-form short string denoting the role of this device (e.g., `"access"`, `"core"`). |
| `vendor`           | String                      | No       | Vendor identifier. Reserved for future multi-vendor support. Not used by the compiler yet. |
| `omit-slot-prefix` | Boolean                     | No       | Default: `false`. When `true`, `modules` must contain exactly one element and that element must not be `null`. The slot index is not used in interface name construction. |
| `slot-index-base`  | Integer                     | No       | Override the starting slot index for this device. If not set, the value from the hardware template's `ports.json` is used. If neither is set, defaults to `0`. |
| `vars`             | OrderedMap<String, String>  | No       | Device-level variables, keyed by name. Default: empty. |
| `modules`          | Vec<Option<Module>>         | Yes      | Ordered list of module slots. `null` entries represent empty slots. An empty list is valid and produces no port or SVI configuration. |

### Module record

| Field    | Type            | Required | Description |
|----------|-----------------|----------|-------------|
| `SKU`    | String          | Yes      | Part number, must match a directory name in `<hardware-templates-dir>`. |
| `serial` | String          | No       | Serial number of the module. |
| `ports`  | Vec<PortAssignment> | Yes  | Ordered list of port assignments for this module. An empty list is valid and produces no port configuration for this module; a warning is emitted. |

### PortAssignment record

| Field     | Type                       | Required | Description |
|-----------|----------------------------|----------|-------------|
| `name`    | String                     | Yes      | Port identifier, e.g., `"Port0"`. Used to look up the physical port definition in the module's hardware template `ports.json`. Must exist in the hardware template (always validated, not just in strict mode). |
| `service` | String                     | Yes      | Short service name. Used to look up the service configuration in the services directory. The service directory must exist and contain `port-config.txt` (always validated). |
| `prologue`| String                     | No       | Newline-separated commands inserted before the service config for this port. Default: empty. |
| `epilogue`| String                     | No       | Newline-separated commands inserted after the service config for this port. Default: empty. |
| `vars`    | OrderedMap<String, String> | No       | Port-level variables. Merged with device-level `vars` (port-level wins). Scoped to this port only. Default: empty. |

---

## 3. Service Templates

**Location:** `<config-root>/services/<service-name>/`

Each service directory is named after the short service name referenced in port assignments.

### port-config.txt

**Required.** Contains the interface-level configuration lines to apply to a physical port using this service.

### svi-config.txt

**Optional.** Contains the SVI (routed virtual interface) configuration associated with this service. If present, it is included once per logical device (deduplicated across ports).

---

## 4. Configuration Templates

**Location:** `<config-root>/config-templates/`

Whole-device configuration templates, minus physical port and SVI configuration. Referenced by the `config-template` field in each logical device's `config.json`.

### Marker substitution

Templates may contain the following markers that are replaced during compilation:

| Marker                   | Replaced with |
|--------------------------|---------------|
| `<PORTS-CONFIGURATION>`  | The generated port configuration block |
| `<SVI-CONFIGURATION>`    | The generated SVI configuration block |

The canonical comment character is `#`. Vendor-specific output may use a different character (e.g., `!` for Cisco IOS). The comment character used in generated output is determined by the vendor context; for the initial Cisco IOS implementation, `!` is used.

**If `<PORTS-CONFIGURATION>` is absent:** the port configuration block is appended at the end of the file, preceded by the vendor comment `! use <PORTS-CONFIGURATION> marker to place this configuration`.

**If `<SVI-CONFIGURATION>` is absent:** the SVI configuration block is appended at the end of the file, preceded by the vendor comment `! use <SVI-CONFIGURATION> marker to place this configuration block`.

The port configuration block is enclosed in `! PORTS-START` and `! PORTS-END` marker lines.

The SVI configuration block is enclosed in `! SVI-START` and `! SVI-END` marker lines.

Note: Markers in the template are replaced *before* port/SVI content is inserted, so marker strings appearing inside service config files will not be substituted.
