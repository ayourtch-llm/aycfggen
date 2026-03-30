# Compilation Pipeline Specification

## Overview

The compiler processes one or more logical devices and produces a complete configuration file for each.

## Interface Name Derivation

The full interface name for a port is constructed as: `name + slot_number + "/" + index` (when slot prefix is used) or `name + index` (when omitted).

Components:
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

- **`omit-slot-prefix: false`** (default) — interface name is `name` + `slot_number` + `"/"` + `index`.
  Example: slot 2, `GigabitEthernet` + `0/0` → `GigabitEthernet2/0/0`
  Example: slot 0, `Ethernet` + `1` → `Ethernet0/1`

This logic is encapsulated in a dedicated function to allow future vendor-specific customization.

## Per-Device Compilation Steps

For each logical device:

1. **Load device config** — read `config.json` from the logical device directory. A missing device directory is always a hard error.

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
     b. Verify that no other port assignment in the same module references the same port identifier. Duplicate port assignments within a module are a hard error.
     c. Derive the full interface name (see rules above).
     d. Load `port-config.txt` from `<services-dir>/<service>/`. This is always a hard error if not found.
     e. Emit the port configuration:
        ```
        interface <full-interface-name>
        <prologue lines, if any>
        <service port-config.txt content>
        <epilogue lines, if any>
        ```

5. **Build SVI configuration block:**
   - Collect all unique service names across all ports on the device, traversing modules in slot order, then ports in list order within each module. First-occurrence order is preserved; duplicates are skipped.
   - Null module slots are skipped during this traversal.
   - For each unique service, check if `<services-dir>/<service>/svi-config.txt` exists.
   - If it exists, include its content in the SVI block.

6. **Expand config elements:**
   - Scan the template for lines matching `!!!###<element-name>`.
   - For each match, load `apply.txt` from `<config-elements-dir>/<element-name>/`. This is always a hard error if not found.
   - Replace the marker line with `! config-element: <element-name>` followed by the contents of `apply.txt`.
   - Config element expansion happens before ports/SVI marker substitution, so config element content may contain `<PORTS-CONFIGURATION>` or `<SVI-CONFIGURATION>` markers (though this is not recommended).

7. **Assemble final configuration:**
   - Each marker (`<PORTS-CONFIGURATION>`, `<SVI-CONFIGURATION>`) must appear at most once in the template. If a marker appears more than once, it is a hard error.
   - Replace `<PORTS-CONFIGURATION>` in the template with the port configuration block (wrapped in `! PORTS-START` / `! PORTS-END`).
   - Replace `<SVI-CONFIGURATION>` in the template with the SVI configuration block (wrapped in `! SVI-START` / `! SVI-END`).
   - If a block is empty (no ports or no SVIs), emit only the marker lines (e.g., `! PORTS-START` followed by `! PORTS-END`).
   - If either marker is missing from the template, append at the end with the appropriate comment (see data-model.md).
   - Note: markers are replaced first, then the content is inserted, so marker strings in service configs are not re-processed.

8. **Write output:**
   - Normal mode: save to `<configs-dir>/<device-name>.txt`. The output directory is created automatically if it does not exist.
   - `--dry-run`: perform all compilation steps but do not write output files.
   - `--preview <BANNER>`: write output to stdout. When compiling multiple devices, each device's output is preceded by a banner line generated from the `<BANNER>` format string.

## Variable Handling

Device-level `vars` and port-level `vars` are defined in the data model. Port-level vars are merged on top of device-level vars (port wins on conflict), scoped to that port only. The merged vars are loaded and stored for data model correctness and future use, but have no observable effect on output in the initial implementation.

Future syntax (reserved, not yet implemented):
- `{{variable}}` — Mustache-style template expansion from vars.
- `{{{expression}}}` — aycalc expression expansion.

## Validation

The following validations are **always** performed (regardless of `--strict`):

- Every device name given on the CLI must correspond to an existing logical device directory.
- Every port assignment's `name` must exist in the corresponding hardware template's `ports.json`.
- Duplicate port assignments (same port identifier) within a single module are not allowed.
- Every port assignment's `service` must correspond to an existing service directory with `port-config.txt`.
- Every `config-template` reference must resolve to an existing file.
- Every `!!!###<element-name>` reference in a config template must correspond to an existing config element directory with `apply.txt`.
- If `software-image` is specified, the referenced file must exist (resolved from `<software-images-dir>` if relative).
- If `omit-slot-prefix` is `true`, `modules` must have exactly one element which is not `null`.
- Each marker (`<PORTS-CONFIGURATION>`, `<SVI-CONFIGURATION>`) must appear at most once in a template.

### Warnings

The following conditions emit a warning (may become errors in a future version):

- A module with zero ports in its `ports` list.

### Strict mode (`--strict`)

When `--strict` is enabled, the compiler performs additional validation:

- Unknown fields in any JSON file cause an error.
