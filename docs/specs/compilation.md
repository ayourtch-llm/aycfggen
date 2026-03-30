# Compilation Pipeline Specification

## Overview

The compiler processes one or more logical devices and produces a complete configuration file for each.

## Interface Name Derivation

The full interface name for a port is constructed from:
- The **port definition** in the hardware template (`name`, `index`)
- The **slot number** (position of the module in the `modules` vector)
- The **`singlemodule`** flag on the logical device

### Rules

- **`singlemodule: true`** — exactly one module must be present. Interface name is `name` + `index`.
  Example: `GigabitEthernet` + `0/0` → `GigabitEthernet0/0`

- **`singlemodule: false`** (or not set) — slot number is prepended to the index, separated by `/`.
  Example: slot 2, `GigabitEthernet` + `0/0` → `GigabitEthernet2/0/0`

This logic will be encapsulated in a dedicated function to allow future vendor-specific customization.

## Per-Device Compilation Steps

For each logical device:

1. **Load device config** — read `config.json` from the logical device directory.

2. **Validate modules** — if `singlemodule` is `true`, verify that `modules` contains exactly one non-null entry.

3. **Load config template** — read the file specified by `config-template`.

4. **Build port configuration block:**
   For each module slot (in order):
   - If the slot is `null`, skip it.
   - Load `ports.json` from `<hardware-templates-dir>/<module.SKU>/`.
   - For each port assignment in the module (in order):
     a. Look up the port definition in `ports.json` using the port assignment's `name` field.
     b. Derive the full interface name (see rules above).
     c. Load `port-config.txt` from `<services-dir>/<service>/`.
     d. Emit the port configuration:
        ```
        interface <full-interface-name>
        <prologue lines, if any>
        <service port-config.txt content>
        <epilogue lines, if any>
        ```

5. **Build SVI configuration block:**
   - Collect all unique service names across all ports on the device (preserving first-occurrence order).
   - For each unique service, check if `<services-dir>/<service>/svi-config.txt` exists.
   - If it exists, include its content in the SVI block.

6. **Assemble final configuration:**
   - Replace `<PORTS-CONFIGURATION>` in the template with the port configuration block (wrapped in `! PORTS-START` / `! PORTS-END`).
   - Replace `<SVI-CONFIGURATION>` in the template with the SVI configuration block (wrapped in `! SVI-START` / `! SVI-END`).
   - If either marker is missing, append at the end with the appropriate comment (see data-model.md).

7. **Write output** — save to `<configs-dir>/<device-name>.txt`.

## Variable Handling

Device-level `vars` and port-level `vars` are defined in the data model. Port-level vars are merged on top of device-level vars (port wins on conflict), scoped to that port only.

Variable expansion into templates and service configs is **out of scope** for the initial implementation. The var data is loaded and merged but not applied.

## Strict Mode

When `--strict` is enabled, the compiler performs additional validation:

- Unknown fields in any JSON file cause an error.
- Every port assignment's `name` must exist in the corresponding hardware template's `ports.json`.
- Every port assignment's `service` must correspond to an existing service directory with at least `port-config.txt`.
- Every `config-template` reference must resolve to an existing file.
- If `singlemodule` is `true`, `modules` must have exactly one entry.
