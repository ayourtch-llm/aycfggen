# aycfggen

A modular network device configuration generator/compiler written in Rust. Currently targets Cisco IOS, with multi-vendor extensibility in mind.

## Overview

aycfggen compiles complete device configurations from modular, reusable inputs:

- **Hardware templates** define port mappings for each hardware SKU
- **Logical device configs** describe how a device is assembled (modules, ports, services)
- **Service templates** provide per-port and per-SVI configuration snippets
- **Config elements** are reusable configuration blocks shared across devices
- **Config templates** are whole-device base configurations with marker-based insertion points

A single change to a service template or config element propagates to every device that uses it.

## Installation

Requires Rust 1.85+ (edition 2024).

```sh
cargo build --release
```

The binary is at `target/release/aycfggen`. The crate is also usable as a library.

## Usage

```
aycfggen [OPTIONS] [DEVICE_NAMES...]
```

### Basic examples

Compile all devices under a config root:

```sh
aycfggen --config-root /path/to/configs
```

Compile specific devices:

```sh
aycfggen --config-root /path/to/configs switch1 router1
```

Preview output on stdout with a banner:

```sh
aycfggen --config-root /path/to/configs --preview "=== {{device-name}} ({{role}}) ==="
```

Validate without writing files:

```sh
aycfggen --config-root /path/to/configs --dry-run
```

### Options

| Flag | Description |
|------|-------------|
| `--config-root <PATH>` | Root directory containing all subdirectories. Default: current directory. |
| `--hardware-templates-dir <PATH>` | Override hardware templates directory. |
| `--logical-devices-dir <PATH>` | Override logical devices directory. |
| `--services-dir <PATH>` | Override services directory. |
| `--config-templates-dir <PATH>` | Override config templates directory. |
| `--config-elements-dir <PATH>` | Override config elements directory. |
| `--software-images-dir <PATH>` | Override software images directory. |
| `--configs-dir <PATH>` | Override output directory. |
| `--strict` | Enable strict validation (reserved, not yet implemented). |
| `--dry-run` | Run compilation and validation without writing output files. |
| `--preview <BANNER>` | Write output to stdout. Banner is a Mustache template with `{{device-name}}`, `{{role}}`, `{{config-template}}`. |

`--dry-run` and `--preview` are mutually exclusive. All directory options are additive.

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (compilation failure, missing files, validation error) |

## Directory Layout

```
<config-root>/
├── hardware-templates/
│   └── <SKU>/
│       └── ports.json
├── logical-devices/
│   └── <device-name>/
│       └── config.json
├── services/
│   └── <service-name>/
│       ├── port-config.txt
│       └── svi-config.txt          (optional)
├── config-templates/
│   └── <template-name>
├── config-elements/
│   └── <element-name>/
│       ├── apply.txt
│       └── unapply.txt             (reserved for future use)
├── software-images/                (metadata, validated but not used in output)
│   └── <image-file>
└── configs/                        (output, auto-created)
    └── <device-name>.txt
```

## Input Formats

### Hardware Template (`ports.json`)

Defines the physical ports for a hardware SKU.

```json
{
  "vendor": "cisco-ios",
  "slot-index-base": 0,
  "ports": {
    "Port0": { "name": "GigabitEthernet", "index": "0/0" },
    "Port1": { "name": "GigabitEthernet", "index": "0/1" }
  }
}
```

### Logical Device Config (`config.json`)

Describes how a device is assembled from modules and which services run on each port.

```json
{
  "config-template": "access-switch.conf",
  "software-image": "ios-16.12.bin",
  "role": "access",
  "omit-slot-prefix": true,
  "slot-index-base": 0,
  "vars": {
    "hostname": "switch1",
    "location": "Building-A"
  },
  "modules": [
    {
      "SKU": "WS-C3560-24TS",
      "serial": "FOC1234X0AB",
      "ports": [
        { "name": "Port0", "service": "access-vlan10", "prologue": "description Workstation" },
        { "name": "Port1", "service": "access-vlan10" },
        { "name": "Port2", "service": "trunk", "epilogue": "no cdp enable" }
      ]
    }
  ]
}
```

Key fields:

- **`omit-slot-prefix`** (default: `false`): When `true`, the slot number is not included in interface names. Requires exactly one non-null module.
- **`slot-index-base`**: Starting slot number. Overrides the hardware template value.
- **`modules`**: Ordered list of module slots. `null` entries represent empty slots.
- **`vars`**: Device-level variables (port-level vars in each port assignment override these).
- **`prologue`/`epilogue`**: Commands inserted before/after the service config on a port.

### Service Templates

**`port-config.txt`** (required): Interface-level configuration applied to each port using this service.

```
 switchport mode access
 switchport access vlan 10
 no shutdown
```

**`svi-config.txt`** (optional): SVI configuration included once per device (deduplicated across ports).

```
interface Vlan10
 ip address 10.1.10.1 255.255.255.0
 no shutdown
```

### Config Elements

Reusable snippets referenced in config templates via `!!!###<element-name>` markers.

**`apply.txt`**: Inserted during compilation.

```
logging buffered 16384
logging console warnings
```

**`unapply.txt`**: Reserved for future change-set generation.

### Config Templates

Whole-device base configurations with markers for generated content:

```
hostname switch1
!
!!!###logging-standard
!
<SVI-CONFIGURATION>
!
<PORTS-CONFIGURATION>
!
end
```

Markers:
- `<PORTS-CONFIGURATION>` — replaced with the generated port configuration block
- `<SVI-CONFIGURATION>` — replaced with the generated SVI configuration block
- `!!!###<element-name>` — replaced with the config element's `apply.txt` content

If a marker is absent, the block is appended at the end with a guidance comment.

## Compilation Pipeline

For each device:

1. Load `config.json`
2. Validate (port/service existence, duplicate ports, module constraints, markers)
3. Load config template
4. Expand config elements (`!!!###<name>` markers)
5. Build port configuration block (interface name + prologue + service config + epilogue per port)
6. Build SVI configuration block (deduplicated by service, first-occurrence order)
7. Assemble: substitute markers in template, wrap blocks in `! PORTS-START`/`! PORTS-END` and `! SVI-START`/`! SVI-END`
8. Write output

### Interface Name Derivation

- **Single module** (`omit-slot-prefix: true`): `GigabitEthernet` + `0/0` = `GigabitEthernet0/0`
- **Multi module**: `GigabitEthernet` + slot 2 + `0/0` = `GigabitEthernet2/0/0`

Slot number = vector position + `slot-index-base` (device overrides hardware template overrides default 0).

## Library Usage

The crate exposes all functionality as a library:

```rust
use aycfggen::compile::compile_device;
use aycfggen::fs_sources::*;

let hw = FsHardwareTemplateSource::new("hardware-templates".into());
let devices = FsLogicalDeviceSource::new("logical-devices".into());
let services = FsServiceSource::new("services".into());
let templates = FsConfigTemplateSource::new("config-templates".into());
let elements = FsConfigElementSource::new("config-elements".into());
let images = FsSoftwareImageSource::new("software-images".into());

let config = compile_device(
    "switch1", &devices, &hw, &services, &templates, &elements, &images
)?;
```

All data access goes through traits (`HardwareTemplateSource`, `LogicalDeviceSource`, etc.), making it straightforward to implement alternative backends (databases, APIs, etc.).

## Examples

Two complete example sets are included in `docs/examples/`:

- **set1**: Single-module access switch with 4 ports, prologue/epilogue, config element, both template markers present
- **set2**: Multi-module branch router with null slot, two SKUs, config element, missing ports marker (append-at-end behavior)

Each set includes `expected-output/` with the exact compiler output for integration testing.

Run against the examples:

```sh
# Preview set1
aycfggen --config-root docs/examples/set1 --preview "--- {{device-name}} ---"

# Preview set2
aycfggen --config-root docs/examples/set2 --preview "--- {{device-name}} ---"

# Write to files
aycfggen --config-root docs/examples/set1
cat docs/examples/set1/configs/switch1.txt
```

## Testing

```sh
cargo test
```

73 tests covering:
- Data model deserialization (6 tests)
- Filesystem backends (12 tests)
- CLI parsing (6 tests)
- Interface name derivation (7 tests)
- Validation (13 tests)
- Config element expansion (8 tests)
- Port and SVI block building (8 tests)
- Template assembly (7 tests)
- Full pipeline integration (2 tests, byte-for-byte against expected output)
- Output writing and banner interpolation (4 tests)

## Specifications

Detailed specifications are in `docs/specs/`:

- `data-model.md` — JSON schemas, field definitions, all data structures
- `compilation.md` — pipeline steps, interface naming, validation rules
- `cli.md` — CLI interface, directory resolution, exit codes
- `process.md` — development methodology (TDD, review cycle)
- `implementation-plan.md` — 12-phase implementation plan

## Future Work

- **`--strict` mode**: Reject unknown fields in JSON files
- **Variable expansion**: `{{variable}}` (Mustache) and `{{{expression}}}` (aycalc) in templates and service configs
- **Multi-vendor support**: Pluggable comment characters, interface name derivation, and output formats
- **Config element unapply**: Generate change sets using `unapply.txt` for configuration rollback

## License

TBD
