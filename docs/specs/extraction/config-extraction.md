# aycfgextract — Configuration Extraction Specification

## Overview

aycfgextract is a sub-library (with a thin CLI wrapper) that performs the **reverse** of aycfggen: given a live Cisco IOS/IOS XE device (or a saved command dump), it decomposes the running configuration into the modular parts defined by the aycfggen data model — hardware profiles, services, config elements, config templates, and logical device configurations.

The authoritative correctness criterion is **byte-for-byte round-trip**: running aycfgextract on a device's `show running-config` output, then compiling the result with aycfggen, must produce output identical to the original `show running-config`. See "Round-Trip Comparison" below for normalization details.

Initially focused on Cisco IOS/IOS XE. The architecture must accommodate future multi-vendor support.

## Prerequisites: aycfggen Changes

The following changes to aycfggen are required before or alongside the extraction implementation:

### Sub-interface support in `derive_interface_name`

The `derive_interface_name` function must be extended to handle sub-interface suffixes. When a port assignment references a port identifier with a `.N` suffix (e.g., `Port0.100`):

1. Strip the `.N` suffix to find the parent port (`Port0`)
2. Look up the parent port in the hardware profile's `ports.json`
3. Derive the base interface name as usual (e.g., `GigabitEthernet0/0` or `GigabitEthernet1/0/0`)
4. Append the `.N` suffix to produce the final name (e.g., `GigabitEthernet0/0.100`)

Hardware profiles (`ports.json`) contain **only physical ports**. Sub-interfaces are purely a property of port assignments. This means `ports.json` has `Port0` with `{"name": "GigabitEthernet", "index": "0/0"}`, and the port assignment `Port0.100` resolves through the parent.

### Enumeration methods on source traits

The existing `sources.rs` read-side traits need enumeration methods for extraction to discover available data:

- `ConfigElementSource::list_elements() -> Vec<String>` — enumerate all config element names
- `ServiceSource::list_services() -> Vec<String>` — enumerate all service names

## CLI Interface

### Arguments

aycfgextract accepts the same directory arguments as aycfggen — root folder + per-class overrides — but each directory is specified at most once (since they are also write targets):

```
aycfgextract [OPTIONS] <TARGET>...
```

Where `<TARGET>` is one or more of:
- IPv4 addresses of live devices
- IPv6 addresses of live devices
- Paths to text files containing saved command output (offline mode)

The extractor distinguishes targets by format: valid IPv4/IPv6 addresses are treated as live devices; everything else is treated as a file path.

### Directory Options

- `--config-root <PATH>` — root for all subdirectories (default: current directory)
- `--hardware-templates-dir <PATH>` — override for hardware profiles
- `--logical-devices-dir <PATH>` — override for logical device configs
- `--services-dir <PATH>` — override for service definitions
- `--config-templates-dir <PATH>` — override for config templates
- `--config-elements-dir <PATH>` — override for config elements
- `--configs-dir <PATH>` — override for compiled output directory

### Extraction Options

- `--recreate-hardware-profiles` — force recreation of hardware profiles even if they already exist for the discovered SKU
- `--save-commands <PATH>` — override the default save location for collected command output (default: `/tmp/<hostname>-<serial>-import.txt`)

### Credentials

SSH username and password are read from environment variables:

- `AYCFGEXTRACT_SSH_USERNAME`
- `AYCFGEXTRACT_SSH_PASSWORD`

This is intentionally modular — per-device credentials and alternative authentication methods (keys, TACACS, etc.) may be introduced later.

## Device Discovery

### Command Set

For each target device, the following commands are executed (or parsed from an offline dump). The command list is intentionally extensible — commands may be added or changed as needed:

- `show version` — platform identification, software version, software image filename
- `show inventory` — PIDs (SKUs), serial numbers, slot positions
- `show ip interface brief` — enumerate all interface names (primary source for interface enumeration)
- `show interfaces status` — additional interface data (speed, duplex, media type); also run for future use
- `show running-config` — full device configuration

In live mode, all commands are executed in a single session. The full command output is **always saved** to `/tmp/<hostname>-<serial>-import.txt` (or the path specified by `--save-commands`) so that extraction can be re-run offline. This file can be shared with others for offline import.

### Offline Mode

The extractor accepts a text file containing the concatenated output of the above commands as a `<TARGET>` argument. This enables:
- Testing and CI without live devices
- Re-running extraction with improved heuristics
- Sharing device data for remote analysis
- Round-trip unit tests (the primary correctness validation)

## Extraction Pipeline

The extraction proceeds in six stages:

### Stage 1: Hardware Profile Discovery

**Input:** `show version`, `show inventory`, `show ip interface brief`

**Process:**

1. Determine module count from `show inventory` first — this controls interface name parsing
2. Parse `show inventory` to identify PIDs (SKUs) and serial numbers per slot
3. Parse interface names from `show ip interface brief`, using the module count to determine whether interface names include a slot prefix
4. Extract slot numbers and group **physical** interfaces by slot/module (sub-interfaces are not included in `ports.json`)
5. Match each slot's interfaces to its PID from inventory
6. Determine slot configuration:
   - `omit-slot-prefix: true` — only when there is exactly one module and interface names contain no slot prefix
   - `slot-index-base` — the lowest slot number observed in the interface names
7. For stacked switches (multiple slots with the same PID but different serials): one shared hardware profile, multiple modules in the device config each with its own serial
8. For mixed stacks (different slots with different PIDs): each slot gets its own hardware profile
9. Extract the software image filename from `show version` output for the `software-image` field in the device config

**Interface name reverse-parsing:** Given an interface name like `GigabitEthernet1/0/3`, the parser splits it by matching known Cisco interface name prefixes (`GigabitEthernet`, `FastEthernet`, `TenGigabitEthernet`, `TwentyFiveGigE`, `FortyGigabitEthernet`, `HundredGigE`, `Serial`, `Loopback`, `Vlan`, `Port-channel`, `Tunnel`, etc.). After stripping the prefix, the module count from `show inventory` determines whether the first number is a slot or part of the port index. For multi-module devices, the first number segment is the slot and the remainder is the port index. For single-module devices, the entire numeric portion is the port index. This prefix list must be extensible for future platforms.

For sub-interfaces (e.g., `GigabitEthernet0/0.100`), the `.100` suffix is stripped before the above parsing. The sub-interface number is not part of the hardware profile — it appears only in port assignments as `Port0.100`.

**Output:** For each unique SKU, a `ports.json` file in the hardware templates directory (if one does not already exist, or if `--recreate-hardware-profiles` is set).

The `ports.json` structure maps port identifiers to interface name components, derived directly from the parsed **physical** interface names on the device. Sub-interfaces do not appear in `ports.json`.

### Stage 2: Port Configuration Decomposition

**Input:** `show running-config` (interface sections), existing services in the data store

**Process:**

1. **Parse** all `interface <name>` blocks from the running config. Classify each as:
   - **Physical port**: `GigabitEthernet0/0`, `FastEthernet0/1`, etc. — enters port grouping
   - **Sub-interface**: `GigabitEthernet0/0.100`, etc. — modeled as `Port0.100` in port assignments, enters port grouping
   - **SVI**: `interface Vlan*` — handled in Stage 3
   - **Virtual**: `Loopback*`, `Tunnel*`, `Port-channel*`, etc. — handled in Stage 4 (literal text in config template)
2. **Group by structural identity** — ports are in different groups if they differ in:
   - `switchport mode` (access vs. trunk vs. routed)
   - VLAN assignment (`switchport access vlan`, `switchport trunk allowed vlan`, etc.)
   - `channel-group` membership (ports in a port-channel always form a separate service)
   - These structural differences **always** result in separate services
   - All other line differences are treated as deviations within the group
3. **Within each group**, identify the most common configuration — this becomes the **service template** (`port-config.txt`)
4. **Handle deviations within a group:**
   - If a deviation is shared by **3 or more ports**, promote it to a **new service** (split the group)
   - If a deviation is shared by **fewer than 3 ports**, express it as **prologue/epilogue** on the base service
   - Epilogue can overwrite service config lines, so order matters: prologue comes before service config, epilogue after
   - A "deviation" is defined as the exact set of differing lines. Ports must share the identical deviation set to count toward the 3-port threshold
5. **Prologue/epilogue determination:** Use sorted versions of config lines for comparison purposes, but track the original line order. If the difference can be expressed as a clean prepend (prologue) and/or append (epilogue) in the original unsorted order, do that. If the lines are reordered or interleaved in a way that does not decompose cleanly into prologue + service + epilogue, create a new service instead.
6. **Shutdown port handling:** Ports with `shutdown` in their config are matched using a three-step priority:
   - **Step 1: Exact match** — try matching the full config (including `shutdown`) against existing services. If it matches, use it as-is
   - **Step 2: Strip and re-match** — if no exact match, strip the `shutdown` line, match against services normally, and add `shutdown` as epilogue
   - **Step 3: Shutdown-only** — ports whose only config line is `shutdown` match or create a `shutdown` service
7. **Match against existing services** — if an existing service's `port-config.txt` matches a derived service template exactly, reuse it instead of creating a new one

**Output:** Service directories (new or matched) and port assignments for the device config.

### Stage 3: SVI Extraction

**Input:** `show running-config` (SVI sections), services derived in Stage 2

**Process:**

1. Parse all `interface Vlan<N>` blocks from the running config
2. For each SVI, determine which service references VLAN N (from Stage 2)
3. **First service wins** — the first service *created* during Stage 2 processing (which follows the interface block order in `show running-config`) that references a VLAN gets the SVI block as its `svi-config.txt`
4. Other services referencing the same VLAN do not carry the SVI config — aycfggen's deduplication during compilation handles this correctly
5. If the user later wants a different service to own the SVI, they can manually duplicate the definition

**Output:** `svi-config.txt` files added to the appropriate service directories.

### Stage 4: Global Configuration & Config Elements

**Input:** `show running-config` (all non-port, non-sub-interface, non-SVI sections), existing config elements in the data store

**Process:**

1. Extract all global configuration lines — everything outside of physical port, sub-interface, and SVI `interface` blocks
2. This includes virtual interface blocks (Loopback, Tunnel, Port-channel), which are kept as literal text in the config template (see "Virtual Interfaces" below)
3. **Config element matching** against existing config elements (enumerated via `list_elements()`), processed in **longest-match-first** order: for each config element in the data store (sorted by `apply.txt` length, descending), check if its `apply.txt` content appears as a contiguous block in the global config. Bare `!` separator lines between candidate lines are **ignored** during matching (since IOS inserts them unpredictably). Matching is **whitespace-sensitive** (indentation matters). Once lines are consumed by a match, they are unavailable to subsequent elements. This prevents a smaller element from "stealing" lines that belong to a larger one.
4. Matched blocks are replaced with `!!!###<element-name>` markers in the config template
5. **Unmatched global config lines remain as literal text** in the config template — this guarantees round-trip correctness regardless of config element recognition
6. Place `<PORTS-CONFIGURATION>` marker where the first physical port or sub-interface block appeared in the original running config. If no physical ports or sub-interfaces exist, **omit the marker** — aycfggen's fallback behavior (appending at the end) handles this case.
7. Place `<SVI-CONFIGURATION>` marker where the first `interface Vlan*` block appeared in the original running config. If no SVIs exist, **omit the marker**.
8. When creating new config element directories, include a placeholder `unapply.txt` with the content `! FIXME - needs to be generated`

**Multi-line constructs:** The parser must correctly handle multi-line blocks with non-standard delimiters:
- `banner motd <delim>...<delim>` — banner text between delimiter characters
- `banner login <delim>...<delim>`, `banner exec <delim>...<delim>`
- `crypto pki certificate chain` blocks (terminated by specific end markers)
- `certificate` blocks within PKI configuration

These are treated as opaque text blobs and included verbatim in the config template. The parser recognizes known multi-line construct starters and captures everything through the terminator. See the mockios project (`../ayclic/mockios/`) for reference patterns on IOS output formatting.

**Output:** A config template file named `<hostname>-<serial>.conf` with a mix of `!!!###<name>` markers and literal config lines, plus `<PORTS-CONFIGURATION>` and `<SVI-CONFIGURATION>` markers at the appropriate positions.

### Stage 5: Variable Extraction

**Input:** All outputs from Stages 1–4

**Process:**

A pluggable `VariableExtractor` trait processes the generated services, templates, and device config. Each extractor identifies parameterizable values and replaces them with `{{variable}}` references, storing the concrete values in the device's `config.json` vars.

**Important:** Variable extraction is deferred until aycfggen implements `{{variable}}` expansion in its compilation pipeline. The trait and pipeline stage are architected and present in the code, but the default implementation is a **no-op** that passes all artifacts through unchanged. This ensures the round-trip verification (Stage 6) is not broken by variable references that the compiler cannot yet expand.

**Planned extractors (to be activated when aycfggen supports expansion):**

- **Hostname** — the `hostname <name>` line in the config template is replaced with `hostname {{hostname}}`, and the actual hostname is stored in device vars
- **VLAN ID** — VLAN numbers in service configs (e.g., `switchport access vlan 10`) are replaced with `{{vlan_id}}` (or similar), with the concrete value in device or port vars

**Architecture for future extractors:**

The trait is designed for incremental addition of new extractors (IP addresses, descriptions, community strings, etc.) without modifying the pipeline. Each extractor is independently testable.

**Output:** (Currently) Unchanged artifacts passed through. (Future) Updated services, templates, and device `config.json` with vars populated.

### Stage 6: Verification

**Input:** All generated artifacts, original `show running-config`

**Process:**

1. Compile the generated device configuration using aycfggen's library API
2. Normalize both the compiled output and the original `show running-config` (see "Round-Trip Comparison" below)
3. Compare the normalized outputs byte-for-byte
4. Report any differences as errors

This is a built-in self-check. If verification fails, the extraction is considered incomplete — the generated artifacts are still written (for debugging), but the tool exits with an error.

**Output:** Pass/fail status with diff on failure.

## Round-Trip Comparison

Byte-for-byte comparison between the original `show running-config` and the aycfggen-compiled output requires normalization of both sides, because:

- aycfggen injects `! config-element: <name>` comment lines before each config element's content
- aycfggen wraps port blocks with `! PORTS-START` / `! PORTS-END` markers
- aycfggen wraps SVI blocks with `! SVI-START` / `! SVI-END` markers
- IOS running configs contain `!` separator lines that may not be preserved exactly through the round-trip

**Normalization procedure (applied to both sides before comparison):**

1. Remove all lines that consist solely of `!` optionally followed by whitespace (bare separator lines)
2. Remove lines matching these specific aycfggen-generated patterns (matched at the start of line with no leading whitespace):
   - `^! config-element: .+$` (config element comment headers)
   - `^! PORTS-START$`
   - `^! PORTS-END$`
   - `^! SVI-START$`
   - `^! SVI-END$`
   - `^! use <PORTS-CONFIGURATION>` (fallback guidance comments, prefix match)
   - `^! use <SVI-CONFIGURATION>` (fallback guidance comments, prefix match)
3. Strip trailing whitespace from all remaining lines
4. Remove trailing blank lines

All other `!`-prefixed comment lines (e.g., `! Access Switch Configuration`) are **preserved** on both sides — they are legitimate config content.

After normalization, the two outputs must be identical.

## Write-Side Trait Abstractions

The existing `sources.rs` traits (`HardwareTemplateSource`, `ServiceSource`, etc.) are read-only. The extraction tool requires write-side counterpart traits to maintain the abstraction pattern:

- `HardwareTemplateSink` — write hardware profiles
- `ServiceSink` — write service directories (port-config.txt, svi-config.txt)
- `ConfigTemplateSink` — write config templates
- `ConfigElementSink` — write config element directories (apply.txt, unapply.txt)
- `LogicalDeviceSink` — write logical device configs

These mirror the read-side traits and have filesystem implementations. This keeps the extraction pipeline testable with mock backends and consistent with the existing architecture.

## Logical Device Output

The discovered logical device configuration is written to:

```
<logical-devices-dir>/<serial-number>/config.json
```

Where `<serial-number>` is the device's chassis serial number from `show inventory`. This ensures uniqueness at discovery time. The user renames the directory to a meaningful device name afterward.

The `config.json` follows the existing aycfggen schema:

```json
{
  "config-template": "switch1-FOC1234X0AB.conf",
  "role": "discovered",
  "software-image": "c3560-ipbasek9-mz.150-2.SE11.bin",
  "omit-slot-prefix": true,
  "slot-index-base": 0,
  "vars": {
    "hostname": "switch1"
  },
  "modules": [
    {
      "SKU": "WS-C3560-24TS",
      "serial": "FOC1234X0AB",
      "ports": [
        {
          "name": "Port0",
          "service": "access-vlan10",
          "epilogue": "no cdp enable"
        }
      ]
    }
  ]
}
```

Note: prologue and epilogue fields use `null`/absent (not empty string) when there is no prologue or epilogue. This is consistent with the `Option<String>` representation in the data model.

## Virtual Interfaces

Non-port, non-SVI virtual interfaces — `Loopback`, `Tunnel`, `Port-channel`, and similar — are placed as **literal text in the config template** or matched against existing **config elements**. They are not part of the port/service model.

This is a pragmatic choice for the initial implementation. These interfaces do not map cleanly to the port assignment model (they have no physical slot/module) and are often unique per device.

## Sub-interfaces

Sub-interfaces (`GigabitEthernet0/0.100`, etc.) are modeled as ports with extended naming in port assignments: `Port0.100`. The `.N` suffix is the sub-interface number, and the prefix (`Port0`) refers to the parent physical port in the hardware profile.

**Hardware profiles (`ports.json`) contain only physical ports.** Sub-interfaces do not appear in `ports.json`. The `derive_interface_name` function resolves `Port0.100` by looking up the parent `Port0`, deriving the base interface name, and appending `.100`.

For example, `GigabitEthernet1/0/0.100` on a multi-module device has slot=1, port index=`0/0`, sub-interface=100. In the hardware profile: `Port0: {"name": "GigabitEthernet", "index": "0/0"}`. In the port assignment: `"name": "Port0.100"`.

This allows sub-interfaces to participate in the service model and port grouping (Stage 2), which is natural for router-on-a-stick deployments where multiple sub-interfaces share identical encapsulation + IP assignment patterns.

## Service Naming Convention

Services created by the extractor follow a naming scheme derived from their structural properties:

- Access ports: `access-vlan<N>` (e.g., `access-vlan10`)
- Trunk ports: `trunk-vlan<N>-<N>` or `trunk-all` depending on allowed VLANs
- Routed ports: `routed-<brief-description>`
- Port-channel members: `channel-group-<N>` (e.g., `channel-group-1`)
- Shutdown-only ports: `shutdown`
- SVI-derived names: `VLAN-SERVICE-<N>` as fallback, or a short identifier extracted from the interface description/comment if one exists

Existing services in the data store are always preferred over creating new ones with these generated names.

## Connection Library

Live device connections use the ayclic library (`../ayclic`), which provides:

- SSH/Telnet connectivity
- Template-driven CLI interaction
- Cisco IOS-specific helpers

The mockios simulator in `../ayclic/mockios/` generates realistic IOS command output and can serve as a source of test fixtures for offline extraction testing.

## Testing Strategy

### Unit Tests

- Parser tests for each `show` command output format (using realistic fixtures, potentially generated via mockios)
- Interface name reverse-parsing tests (including sub-interfaces)
- Port grouping/clustering algorithm tests
- Prologue/epilogue derivation tests
- Shutdown port matching tests
- Config element matching tests (including overlap/priority and bare `!` separator handling)
- Round-trip comparison normalization tests
- Sub-interface port assignment tests

### Integration Tests

Round-trip tests are the primary correctness validation:

1. Provide a known `show running-config` + supporting command output
2. Run aycfgextract to produce modular artifacts
3. Run aycfggen to compile the artifacts
4. Normalize both outputs (see "Round-Trip Comparison")
5. Assert byte-for-byte equality between normalized outputs

Test fixtures live alongside the existing aycfggen example sets in `docs/examples/`.

## Future Considerations

- **Multi-vendor support:** The extraction pipeline is structured around traits that can be implemented for different vendors (NX-OS, EOS, JunOS, etc.)
- **Variable extraction activation:** Enable hostname, VLAN ID, and additional variable extractors once aycfggen implements `{{variable}}` expansion
- **Additional variable extractors:** IP addresses, subnet masks, descriptions, SNMP communities, NTP servers, etc.
- **Incremental extraction:** Re-run extraction on a device that already has a config in the data store, updating only what changed
- **Config element unapply:** Leverage `unapply.txt` for generating change sets
- **Multi-device processing order:** When extracting multiple devices, services created by earlier devices are available for matching by later devices (processed sequentially, written to disk after each device)
