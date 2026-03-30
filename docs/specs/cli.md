# CLI Specification

## Usage

```
aycfggen [OPTIONS] [DEVICE_NAMES...]
```

If no device names are given, all logical devices found in the logical devices directory are compiled.
If one or more device names are given, only those devices are compiled.

## Options

| Flag / Option              | Description |
|----------------------------|-------------|
| `--config-root <PATH>`     | Root directory containing all subdirectories. Default: current directory. |
| `--hardware-templates-dir <PATH>` | Override hardware templates directory. Default: `<config-root>/hardware-templates/` |
| `--logical-devices-dir <PATH>`    | Override logical devices directory. Default: `<config-root>/logical-devices/` |
| `--services-dir <PATH>`           | Override services directory. Default: `<config-root>/services/` |
| `--config-templates-dir <PATH>`   | Override config templates directory. Default: `<config-root>/config-templates/` |
| `--configs-dir <PATH>`            | Override output configs directory. Default: `<config-root>/configs/` |
| `--strict`                 | Enable strict validation mode. |

## Directory Resolution

Each directory is resolved as follows:
1. If the per-class override is given, use it (absolute or relative to CWD).
2. Otherwise, use `<config-root>/<default-subdir-name>/`.

All directory options are additive. It is valid to have no `--config-root` as long as all required per-class directories are specified or discoverable. Similarly, per-class overrides are additive with each other — specifying one does not require specifying the rest.

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
└── configs/                      (output)
    └── <device-name>.txt
```
