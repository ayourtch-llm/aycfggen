# Implementation Plan

This document describes the incremental TDD implementation plan for aycfggen.
Each phase produces one or more commits, each at a green (all tests pass) state.

Implementation is done by Sonnet models using red-green TDD.
Review is done by a fresh Opus model after each commit, then by the supervising Opus.

## Dependencies

The following Rust crates are needed:

- `serde` + `serde_json` — JSON deserialization
- `indexmap` — order-preserving hashmaps (`IndexMap`)
- `clap` (with `derive` feature) — CLI argument parsing
- `anyhow` — error handling with context

## Phase 1: Data Model Types

**Goal:** Define all structs for JSON deserialization, tested against the example files.

### Structs to define (in `src/model.rs`):

- `HardwareTemplate` — top-level of `ports.json`: `vendor`, `slot_index_base`, `ports` (IndexMap)
- `PortDefinition` — inner record: `name`, `index`
- `LogicalDeviceConfig` — top-level of `config.json`: `config_template`, `software_image`, `role`, `vendor`, `omit_slot_prefix`, `slot_index_base`, `vars` (IndexMap), `modules`
- `Module` — inner record: `sku`, `serial`, `ports` (Vec)
- `PortAssignment` — inner record: `name`, `service`, `prologue`, `epilogue`, `vars` (IndexMap)

### Key details:
- Use `#[serde(rename = "field-name")]` for kebab-case JSON field names
- Use `#[serde(default)]` for optional fields with defaults
- Use `IndexMap<String, String>` for ordered hashmaps
- Do NOT use `#[serde(deny_unknown_fields)]` by default (only in strict mode)
- All structs derive `Debug, Clone, Serialize, Deserialize`
- `modules` field type: `Vec<Option<Module>>`

### Tests:
- Deserialize `set1/hardware-templates/WS-C3560-24TS/ports.json`
- Deserialize `set1/logical-devices/switch1/config.json`
- Deserialize `set2/logical-devices/router1/config.json` (has null module slot)
- Deserialize `set2/hardware-templates/NIM-4GE/ports.json`
- Test that unknown fields are ignored (add extra field to JSON string, verify no error)
- Test default values (omit optional fields, verify defaults)

### Commit: "Add data model types with serde deserialization"

---

## Phase 2: Trait Abstractions

**Goal:** Define trait interfaces for all data sources, so backends are swappable.

### Traits to define (in `src/sources.rs`):

- `HardwareTemplateSource` — `fn load_hardware_template(&self, sku: &str) -> Result<HardwareTemplate>`
- `LogicalDeviceSource` — `fn load_device_config(&self, device_name: &str) -> Result<LogicalDeviceConfig>` + `fn list_devices(&self) -> Result<Vec<String>>`
- `ServiceSource` — `fn load_port_config(&self, service_name: &str) -> Result<String>` + `fn load_svi_config(&self, service_name: &str) -> Result<Option<String>>`
- `ConfigTemplateSource` — `fn load_template(&self, template_name: &str) -> Result<String>`
- `ConfigElementSource` — `fn load_apply(&self, element_name: &str) -> Result<String>`
- `SoftwareImageSource` — `fn validate_exists(&self, image_name: &str) -> Result<()>`

All return `anyhow::Result`. String content returned by service/template/element sources should have trailing newline normalized (strip trailing whitespace/newlines, ensure single trailing `\n`).

### Tests:
- Trait definitions compile (no runtime tests yet, just struct/trait coherence)

### Commit: "Add trait abstractions for data sources"

---

## Phase 3: Filesystem Backends

**Goal:** Implement the traits for filesystem-based data sources.

### Implementation (in `src/fs_sources.rs`):

- `FsHardwareTemplateSource { dir: PathBuf }` — reads `<dir>/<sku>/ports.json`
- `FsLogicalDeviceSource { dir: PathBuf }` — reads `<dir>/<device>/config.json`, lists subdirectories
- `FsServiceSource { dir: PathBuf }` — reads `<dir>/<service>/port-config.txt` and `svi-config.txt`
- `FsConfigTemplateSource { dir: PathBuf }` — reads `<dir>/<template>`
- `FsConfigElementSource { dir: PathBuf }` — reads `<dir>/<element>/apply.txt`
- `FsSoftwareImageSource { dir: PathBuf }` — checks file existence at `<dir>/<image>`

Each takes a `PathBuf` for the directory root.

### Trailing newline normalization:
Apply to all text content returned by `FsServiceSource`, `FsConfigTemplateSource`, `FsConfigElementSource`: strip trailing newlines, then ensure exactly one trailing `\n`.

### Tests:
- Load hardware template from set1 via `FsHardwareTemplateSource`
- Load device config from set1 via `FsLogicalDeviceSource`
- List devices from set1 (should return `["switch1"]`)
- List devices from set2 (should return `["router1"]`)
- Load port-config.txt from set1 services via `FsServiceSource`
- Load svi-config.txt (present and absent cases)
- Load config template via `FsConfigTemplateSource`
- Load config element apply.txt via `FsConfigElementSource`
- Validate software image exists / doesn't exist
- Verify trailing newline normalization

### Commit: "Add filesystem backends for data source traits"

---

## Phase 4: CLI Parsing

**Goal:** Parse command-line arguments with clap.

### Implementation (in `src/cli.rs`):

Define a `CliArgs` struct with clap derive:
- `config_root: Option<PathBuf>`
- `hardware_templates_dir: Option<PathBuf>`
- `logical_devices_dir: Option<PathBuf>`
- `services_dir: Option<PathBuf>`
- `config_templates_dir: Option<PathBuf>`
- `config_elements_dir: Option<PathBuf>`
- `software_images_dir: Option<PathBuf>`
- `configs_dir: Option<PathBuf>`
- `strict: bool`
- `dry_run: bool`
- `preview: Option<String>`
- `device_names: Vec<String>`

Add a `ResolvedDirs` struct with a method `resolve(cli: &CliArgs) -> ResolvedDirs` that applies the directory resolution logic (per-class override → config-root + default subdir → CWD + default subdir).

Validate `--dry-run` and `--preview` are mutually exclusive.

### Tests:
- Parse `--config-root /tmp/test` → resolves all default subdirs
- Parse with per-class overrides
- Parse with device names
- `--dry-run` and `--preview` together → error
- Default: config-root is CWD
- Additive: only `--services-dir` given, no root

### Commit: "Add CLI argument parsing and directory resolution"

---

## Phase 5: Interface Name Derivation

**Goal:** Implement the function that constructs full interface names.

### Implementation (in `src/interface_name.rs`):

```rust
pub fn derive_interface_name(
    port_def: &PortDefinition,
    slot_position: usize,
    slot_index_base: u32,
    omit_slot_prefix: bool,
) -> String
```

Rules:
- `omit_slot_prefix: true` → `name + index`
- `omit_slot_prefix: false` → `name + (slot_position + slot_index_base) + "/" + index`

### Slot index base resolution (in caller, not this function):
1. Logical device `slot_index_base` if set
2. Hardware template `slot_index_base` if set
3. Default: `0`

### Tests:
- `omit_slot_prefix=true`: `GigabitEthernet` + `0/0` → `GigabitEthernet0/0`
- `omit_slot_prefix=false`, slot 2, base 0: `GigabitEthernet` + `0/0` → `GigabitEthernet2/0/0`
- `omit_slot_prefix=false`, slot 0, base 0: `Ethernet` + `1` → `Ethernet0/1`
- `omit_slot_prefix=false`, slot 1, base 1: `GigabitEthernet` + `0/0` → `GigabitEthernet2/0/0`
- Slot index base resolution: device overrides template overrides default

### Commit: "Add interface name derivation function"

---

## Phase 6: Validation

**Goal:** Implement all always-on and strict-mode validations.

### Implementation (in `src/validate.rs`):

Validation functions that take the loaded data model + sources and check:
- `omit_slot_prefix=true` → modules has exactly one non-null element
- Duplicate port assignments within a module → error
- Port name exists in hardware template
- Service directory exists with `port-config.txt`
- Config template file exists
- Config element references exist with `apply.txt`
- Software image exists (if specified)
- Markers appear at most once in template

Warning:
- Module with zero ports

### Tests:
- Valid set1 config passes validation
- Valid set2 config passes validation
- `omit_slot_prefix=true` with 2 modules → error
- `omit_slot_prefix=true` with `[null]` → error
- Duplicate port name in module → error
- Missing port in hardware template → error
- Missing service → error
- Missing config template → error
- Missing config element → error
- Duplicate marker in template → error
- Zero-port module → warning (not error)

### Commit: "Add validation logic"

---

## Phase 7: Config Element Expansion

**Goal:** Expand `!!!###<element-name>` markers in config templates.

### Implementation (in `src/compile.rs` or `src/elements.rs`):

```rust
pub fn expand_config_elements(
    template: &str,
    element_source: &dyn ConfigElementSource,
) -> Result<String>
```

- Scan line by line
- If trimmed line matches `^!!!###([a-zA-Z0-9_-]+)$`, replace with `! config-element: <name>\n` + apply.txt content
- Otherwise, pass through unchanged

### Tests:
- Single element expansion (set1 template with `!!!###logging-standard`)
- Single element expansion (set2 template with `!!!###ntp-config`)
- No elements in template → unchanged
- Unknown element → error
- Element name with invalid chars → not matched (passed through)

### Commit: "Add config element expansion"

---

## Phase 8: Port Configuration Block Building

**Goal:** Build the port configuration block from modules.

### Implementation (in `src/compile.rs`):

```rust
pub fn build_port_block(
    device: &LogicalDeviceConfig,
    hw_source: &dyn HardwareTemplateSource,
    service_source: &dyn ServiceSource,
) -> Result<String>
```

- Iterate modules in order, skip nulls
- For each module, load hardware template by SKU
- Resolve slot_index_base
- For each port assignment, derive interface name, load port-config.txt
- Emit: `interface <name>\n` + prologue + port-config + epilogue

### Tests:
- Build port block for set1 (4 ports, omit-slot-prefix, prologue on port0, epilogue on port3)
- Build port block for set2 (null slot, 2 modules with slot numbering)
- Empty modules list → empty string
- Module with zero ports → warning, empty contribution

### Commit: "Add port configuration block building"

---

## Phase 9: SVI Configuration Block Building

**Goal:** Build the SVI configuration block from unique services.

### Implementation (in `src/compile.rs`):

```rust
pub fn build_svi_block(
    device: &LogicalDeviceConfig,
    service_source: &dyn ServiceSource,
) -> Result<String>
```

- Traverse modules in slot order, ports in list order
- Collect unique service names (first-occurrence, dedup)
- For each, try to load svi-config.txt; include if present
- Skip services without svi-config.txt

### Tests:
- Set1: access-vlan10 has SVI, trunk doesn't → only access-vlan10 SVI
- Set2: wan-link and voice both have SVIs → both included in order
- Service appearing on multiple ports → SVI included once
- No services have SVIs → empty string

### Commit: "Add SVI configuration block building"

---

## Phase 10: Template Assembly

**Goal:** Assemble the final configuration by substituting markers in the template.

### Implementation (in `src/compile.rs`):

```rust
pub fn assemble_config(
    template: &str,
    port_block: &str,
    svi_block: &str,
) -> Result<String>
```

- Validate each marker appears at most once
- Replace `<PORTS-CONFIGURATION>` with `! PORTS-START\n` + port_block + `! PORTS-END\n`
- Replace `<SVI-CONFIGURATION>` with `! SVI-START\n` + svi_block + `! SVI-END\n`
- If marker missing: append at end (SVI first, then ports) with comment
- Empty blocks: emit only marker lines

### Tests:
- Both markers present (set1 template)
- SVI marker present, ports missing (set2 template) → ports appended with comment
- Both markers missing → both appended (SVI first, then ports)
- Empty port block → `! PORTS-START\n! PORTS-END\n`
- Empty SVI block → `! SVI-START\n! SVI-END\n`
- Duplicate marker → error

### Commit: "Add template assembly with marker substitution"

---

## Phase 11: Full Compilation Pipeline

**Goal:** Wire everything together into a `compile_device` function.

### Implementation (in `src/compile.rs`):

```rust
pub fn compile_device(
    device_name: &str,
    device_source: &dyn LogicalDeviceSource,
    hw_source: &dyn HardwareTemplateSource,
    service_source: &dyn ServiceSource,
    template_source: &dyn ConfigTemplateSource,
    element_source: &dyn ConfigElementSource,
    image_source: &dyn SoftwareImageSource,
) -> Result<String>
```

Steps: load config → validate → load template → expand elements → build port block → build SVI block → assemble → return string.

### Tests (integration, using example sets):
- Compile set1/switch1 → compare byte-for-byte with `set1/expected-output/switch1.txt`
- Compile set2/router1 → compare byte-for-byte with `set2/expected-output/router1.txt`

### Commit: "Add full compilation pipeline with integration tests"

---

## Phase 12: Output and Main

**Goal:** Implement output writing, dry-run, preview, and wire into `main()`.

### Implementation:
- `src/output.rs`: write to file (create dir if needed), dry-run (no-op), preview (stdout with banner)
- `src/main.rs`: parse CLI → resolve dirs → create fs sources → list/filter devices → compile each → output
- Preview banner: simple Mustache expansion of `{{device-name}}`, `{{role}}`, `{{config-template}}`
- Exit code: 0 on success, 1 on any error

### Tests:
- Write output to temp directory, verify file exists and content matches
- Dry-run: verify no file created
- Preview to captured stdout with banner
- Banner interpolation: `"=== {{device-name}} ==="` → `"=== switch1 ==="`
- Missing output dir → auto-created
- Alphabetical device ordering when no names given

### Commit: "Add output modes and main entry point"

---

## Module Structure Summary

```
src/
├── main.rs          — entry point, CLI → compile → output
├── cli.rs           — CliArgs, ResolvedDirs
├── model.rs         — all data model structs (serde)
├── sources.rs       — trait definitions
├── fs_sources.rs    — filesystem implementations
├── interface_name.rs — derive_interface_name()
├── validate.rs      — validation functions
├── compile.rs       — element expansion, port/SVI blocks, assembly, compile_device
└── output.rs        — file writing, dry-run, preview
```
