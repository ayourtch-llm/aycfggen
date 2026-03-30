# CLI Specification

## Usage

```
aycfggen [OPTIONS] [DEVICE_NAMES...]
```

If no device names are given, all logical devices found in the logical devices directory are compiled.
If one or more device names are given, only those devices are compiled. If a named device directory does not exist, it is a hard error.

## Options

| Flag / Option              | Description |
|----------------------------|-------------|
| `--config-root <PATH>`     | Root directory containing all subdirectories. Default: current directory. |
| `--hardware-templates-dir <PATH>` | Override hardware templates directory. Default: `<config-root>/hardware-templates/` |
| `--logical-devices-dir <PATH>`    | Override logical devices directory. Default: `<config-root>/logical-devices/` |
| `--services-dir <PATH>`           | Override services directory. Default: `<config-root>/services/` |
| `--config-templates-dir <PATH>`   | Override config templates directory. Default: `<config-root>/config-templates/` |
| `--config-elements-dir <PATH>`    | Override config elements directory. Default: `<config-root>/config-elements/` |
| `--software-images-dir <PATH>`    | Override software images directory. Default: `<config-root>/software-images/` |
| `--configs-dir <PATH>`            | Override output configs directory. Default: `<config-root>/configs/` |
| `--strict`                 | Enable strict validation mode (unknown JSON fields become errors). |
| `--dry-run`                | Perform all compilation and validation steps but do not write output files. |
| `--preview <BANNER>`       | Write output to stdout instead of files. When compiling multiple devices, each device's output is preceded by a banner line. The `<BANNER>` parameter is a format string that may contain interpolatable variables (e.g., `"=== {{device-name}} ==="`). |

`--dry-run` and `--preview` are mutually exclusive.

## Exit Codes

| Code | Meaning |
|------|---------|
| `0`  | Success |
| `1`  | Error   |

## Directory Resolution

Each directory is resolved as follows:
1. If the per-class override is given, use it (absolute or relative to CWD).
2. Otherwise, use `<config-root>/<default-subdir-name>/`.

All directory options are additive. It is valid to have no `--config-root` as long as all required per-class directories are specified or discoverable. Similarly, per-class overrides are additive with each other — specifying one does not require specifying the rest.

The output directory (`configs/`) is created automatically if it does not exist.

## Default Directory Layout

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
│       └── svi-config.txt        (optional)
├── config-templates/
│   └── <template-name>
├── config-elements/
│   └── <element-name>/
│       ├── apply.txt
│       └── unapply.txt           (reserved for future use)
├── software-images/              (metadata, not used in compilation)
│   └── <image-file>
└── configs/                      (output, auto-created)
    └── <device-name>.txt
```
