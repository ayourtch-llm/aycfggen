# Data Model Specification

All hashmaps referenced in this specification are order-preserving.
All JSON deserialization must be lenient by default — unknown fields are ignored.
When `--strict` mode is enabled, unknown fields cause an error.

## 1. Hardware Templates

**Location:** `<config-root>/hardware-templates/<SKU>/`

Each SKU directory contains platform-specific mappings for a given hardware part number.

### ports.json

An ordered hashmap keyed by port identifiers of the form `Port<N>` (e.g., `Port0`, `Port1`, ...).

Each value is a record with:

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
| `software`         | String                      | No       | Filename of software image. If relative, resolved from `<software-images-dir>`. Metadata only for now. |
| `role`             | String                      | No       | Free-form short string denoting the role of this device (e.g., `"access"`, `"core"`). |
| `singlemodule`     | Boolean                     | No       | Default: `false`. When `true`, `modules` must contain exactly one entry, and the slot index is not used in interface name construction. |
| `vars`             | OrderedMap<String, String>  | No       | Device-level variables, keyed by name. Default: empty. |
| `modules`          | Vec<Option<Module>>         | Yes      | Ordered list of module slots. `null` entries represent empty slots. |

### Module record

| Field    | Type            | Required | Description |
|----------|-----------------|----------|-------------|
| `SKU`    | String          | Yes      | Part number, must match a directory name in `<hardware-templates-dir>`. |
| `serial` | String          | No       | Serial number of the module. |
| `ports`  | Vec<PortAssignment> | Yes  | Ordered list of port assignments for this module. |

### PortAssignment record

| Field     | Type                       | Required | Description |
|-----------|----------------------------|----------|-------------|
| `name`    | String                     | Yes      | Port identifier, e.g., `"Port0"`. Used to look up the physical port definition in the module's hardware template `ports.json`. |
| `service` | String                     | Yes      | Short service name. Used to look up the service configuration in the services directory. |
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

**If `<PORTS-CONFIGURATION>` is absent:** the port configuration block is appended at the end of the file, preceded by the comment line `! use <PORTS-CONFIGURATION> marker to place this configuration`.

**If `<SVI-CONFIGURATION>` is absent:** the SVI configuration block is appended at the end of the file, preceded by the comment line `! use <SVI-CONFIGURATION> marker to place this configuration block`.

The port configuration block is enclosed in `! PORTS-START` and `! PORTS-END` marker lines.

The SVI configuration block is enclosed in `! SVI-START` and `! SVI-END` marker lines.
