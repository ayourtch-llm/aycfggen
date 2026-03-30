# Compilation Pipeline Specification

## Overview

The compiler processes one or more logical devices and produces a complete configuration file for each.

## Interface Name Derivation

The full interface name for a port is constructed from:
- The **port definition** in the hardware template (`name`, `index`)
- The **slot number** (vector position of the module + `slot-index-base`)
- The **`omit-slot-prefix`** flag on the logical device

### Slot numbering

The slot number for a module is: `vector_position + slot_index_base`.

The `slot-index-base` is resolved as follows (first match wins):
1. `slot-index-base` on the logical device's `config.json`
2. `slot-index-base` on the hardware template's `ports.json`
3. Default: `0`

### Rules

- **`omit-slot-prefix: true`** — exactly one module must be present and non-null. Interface name is `name` + `index`.
  Example: `GigabitEthernet` + `0/0` → `GigabitEthernet0/0`

- **`omit-slot-prefix: false`** (default) — slot number is prepended to the index, separated by `/`.
  Example: slot 2, `GigabitEthernet` + `0/0` → `GigabitEthernet2/0/0`

This logic is encapsulated in a dedicated function to allow future vendor-specific customization.

## Per-Device Compilation Steps

For each logical device:

1. **Load device config** — read `config.json` from the logical device directory.

2. **Validate modules:**
   - If `omit-slot-prefix` is `true`, verify that `modules` has exactly one element and that element is not `null`.
   - An empty `modules` list is valid — it produces no port or SVI configuration.

3. **Load config template** — read the file specified by `config-template`. This is always a hard error if the file does not exist.

4. **Build port configuration block:**
   For each module slot (in order):
   - If the slot is `null`, skip it.
   - Load `ports.json` from `<hardware-templates-dir>/<module.SKU>/`.
   - If the module has zero ports, emit a warning and skip it.
   - For each port assignment in the module (in order):
     a. Look up the port definition in `ports.json` using the port assignment's `name` field. This is always a hard error if not found.
     b. Derive the full interface name (see rules above).
     c. Load `port-config.txt` from `<services-dir>/<service>/`. This is always a hard error if not found.
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
   - Note: markers are replaced first, then the content is inserted, so marker strings in service configs are not re-processed.

7. **Write output:**
   - Normal mode: save to `<configs-dir>/<device-name>.txt`.
   - `--dry-run`: perform all compilation steps but do not write output files.
   - `--preview <PATH>`: write output to `<PATH>` instead of the default configs directory. If `<PATH>` is `-`, write to stdout.

## Variable Handling

Device-level `vars` and port-level `vars` are defined in the data model. Port-level vars are merged on top of device-level vars (port wins on conflict), scoped to that port only.

Variable expansion into templates and service configs is **out of scope** for the initial implementation. The var data is loaded and merged but not applied.

Future syntax (reserved, not yet implemented):
- `{{variable}}` — Mustache-style template expansion from vars.
- `{{{expression}}}` — aycalc expression expansion.

## Validation

The following validations are **always** performed (regardless of `--strict`):

- Every port assignment's `name` must exist in the corresponding hardware template's `ports.json`.
- Every port assignment's `service` must correspond to an existing service directory with `port-config.txt`.
- Every `config-template` reference must resolve to an existing file.
- If `omit-slot-prefix` is `true`, `modules` must have exactly one element which is not `null`.

### Warnings

The following conditions emit a warning (may become errors in a future version):

- A module with zero ports in its `ports` list.

### Strict mode (`--strict`)

When `--strict` is enabled, the compiler performs additional validation:

- Unknown fields in any JSON file cause an error.
