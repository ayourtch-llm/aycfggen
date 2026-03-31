# aycfgextract — Configuration Extraction Specification

## Overview

aycfgextract is a sub-library (with a thin CLI wrapper) that performs the **reverse** of aycfggen: given a live Cisco IOS/IOS XE device (or a saved command dump), it decomposes the running configuration into the modular parts defined by the aycfggen data model — hardware profiles, services, config elements, config templates, and logical device configurations.

The authoritative correctness criterion is **byte-for-byte round-trip**: running aycfgextract on a device's `show running-config` output, then compiling the result with aycfggen, must produce output identical to the original `show running-config`. See "Round-Trip Comparison" below for normalization details.

Initially focused on Cisco IOS/IOS XE. The architecture must accommodate future multi-vendor support.

## CLI Interface

### Arguments

aycfgextract accepts the same directory arguments as aycfggen — root folder + per-class overrides — but each directory is specified at most once (since they are also write targets):

```
aycfgextract [OPTIONS] <TARGET>...
```

Where `<TARGET>` is one or more of:
- IPv4/IPv6 addresses of live devices
- Paths to text files containing saved command output (offline mode)

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
- `--offline <FILE>` — read command output from a text file instead of connecting to a live device

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
- `show ip interface brief` (or `show interfaces status`) — enumerate all interface names
- `show running-config` — full device configuration

In live mode, all commands are executed in a single session. The full command output is always saved so that extraction can be re-run offline.

### Offline Mode

The extractor accepts a text file containing the concatenated output of the above commands. This enables:
- Testing and CI without live devices
- Re-running extraction with improved heuristics
- Round-trip unit tests (the primary correctness validation)

## Extraction Pipeline

The extraction proceeds in six stages:

### Stage 1: Hardware Profile Discovery

**Input:** `show version`, `show inventory`, `show ip interface brief`

**Process:**

1. Parse `show inventory` to identify PIDs (SKUs) and serial numbers per slot
2. Parse interface names from `show ip interface brief`, extract slot numbers, and group interfaces by slot/module
3. Match each slot's interfaces to its PID from inventory
4. Determine slot configuration:
   - `omit-slot-prefix: true` — only when there is exactly one module and interface names contain no slot prefix
   - `slot-index-base` — the lowest slot number observed in the interface names
5. For stacked switches (multiple slots with the same PID but different serials): one shared hardware profile, multiple modules in the device config each with its own serial
6. Extract the software image filename from `show version` output for the `software-image` field in the device config

**Interface name reverse-parsing:** Given an interface name like `GigabitEthernet1/0/3`, the parser splits it by matching known Cisco interface name prefixes (`GigabitEthernet`, `FastEthernet`, `TenGigabitEthernet`, `TwentyFiveGigE`, `FortyGigabitEthernet`, `HundredGigE`, `Serial`, `Loopback`, `Vlan`, `Port-channel`, `Tunnel`, etc.). After stripping the prefix, the first number segment is the slot (if multi-module), and the remainder is the port index. This prefix list must be extensible for future platforms.

**Output:** For each unique SKU, a `ports.json` file in the hardware templates directory (if one does not already exist, or if `--recreate-hardware-profiles` is set).

The `ports.json` structure maps port identifiers to interface name components, derived directly from the parsed interface names on the device.

### Stage 2: Port Configuration Decomposition

**Input:** `show running-config` (physical interface sections), existing services in the data store

**Process:**

1. **Parse** all physical `interface <name>` blocks from the running config, excluding:
   - SVIs (`interface Vlan*`)
   - Loopback interfaces (`interface Loopback*`)
   - Tunnel interfaces (`interface Tunnel*`)
   - Port-channel interfaces (`interface Port-channel*`)
   - Sub-interfaces (`interface <name>.<number>`) — see "Sub-interfaces" below
2. **Group by structural identity** — ports are in different groups if they differ in:
   - `switchport mode` (access vs. trunk vs. routed)
   - VLAN assignment (`switchport access vlan`, `switchport trunk allowed vlan`, etc.)
   - These structural differences **always** result in separate services
   - All other line differences are treated as deviations within the group
3. **Within each group**, identify the most common configuration — this becomes the **service template** (`port-config.txt`)
4. **Handle deviations within a group:**
   - If a deviation is shared by **3 or more ports**, promote it to a **new service** (split the group)
   - If a deviation is shared by **fewer than 3 ports**, express it as **prologue/epilogue** on the base service
   - Epilogue can overwrite service config lines, so order matters: prologue comes before service config, epilogue after
   - To determine prologue vs. epilogue: diff each port's interface block against the service template. Lines that appear before the first matching service line are prologue; lines that appear after the last matching service line are epilogue
   - A "deviation" is defined as the exact set of differing lines. Ports must share the identical deviation set to count toward the 3-port threshold
5. **Match against existing services** — if an existing service's `port-config.txt` matches a derived service template exactly, reuse it instead of creating a new one

**Output:** Service directories (new or matched) and port assignments for the device config.

### Stage 3: SVI Extraction

**Input:** `show running-config` (SVI sections), services derived in Stage 2

**Process:**

1. Parse all `interface Vlan<N>` blocks from the running config
2. For each SVI, determine which service references VLAN N (from Stage 2)
3. **First service wins** — the first service (by discovery order) that references a VLAN gets the SVI block as its `svi-config.txt`
4. Other services referencing the same VLAN do not carry the SVI config — aycfggen's deduplication during compilation handles this correctly
5. If the user later wants a different service to own the SVI, they can manually duplicate the definition

**Output:** `svi-config.txt` files added to the appropriate service directories.

### Stage 4: Global Configuration & Config Elements

**Input:** `show running-config` (all non-physical-port sections), existing config elements in the data store

**Process:**

1. Extract all global configuration lines — everything outside of physical port `interface` blocks
2. This includes virtual interface blocks (Loopback, Tunnel, Port-channel) and sub-interfaces, which are kept as literal text in the config template (see "Virtual Interfaces" and "Sub-interfaces" below)
3. **Best-effort matching** against existing config elements: for each config element in the data store, check if its `apply.txt` content appears as a contiguous block in the global config
4. Matched blocks are replaced with `!!!###<element-name>` markers in the config template
5. **Unmatched global config lines remain as literal text** in the config template — this guarantees round-trip correctness regardless of config element recognition
6. Place `<PORTS-CONFIGURATION>` marker where the first physical port interface block appeared in the original running config
7. Place `<SVI-CONFIGURATION>` marker where the first `interface Vlan*` block appeared in the original running config
8. When creating new config element directories, include an empty `unapply.txt` with the content `! FIXME - needs to be generated`

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

1. Remove all lines that consist solely of `!` optionally followed by whitespace (separator lines)
2. Remove all lines starting with `! ` that are aycfggen-generated comments (e.g., `! config-element:`, `! PORTS-START`, `! PORTS-END`, `! SVI-START`, `! SVI-END`)
3. Strip trailing whitespace from all remaining lines
4. Remove trailing blank lines

After normalization, the two outputs must be identical.

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
          "prologue": "",
          "epilogue": "no cdp enable"
        }
      ]
    }
  ]
}
```

## Virtual Interfaces

Non-port, non-SVI virtual interfaces — `Loopback`, `Tunnel`, `Port-channel`, and similar — are placed as **literal text in the config template** or matched against existing **config elements**. They are not part of the port/service model.

This is a pragmatic choice for the initial implementation. These interfaces do not map cleanly to the port assignment model (they have no physical slot/module) and are often unique per device.

## Sub-interfaces

Sub-interfaces (`GigabitEthernet0/0.100`, etc.) are placed as **literal text in the config template** in the initial implementation.

**Future direction:** Sub-interfaces could be modeled as ports with extended naming (e.g., `Port0.100` in the hardware profile / port assignments), allowing them to participate in the service model. This would be natural for router-on-a-stick deployments where multiple sub-interfaces share identical encapsulation + IP assignment patterns. This is deferred to a future design iteration.

## Service Naming Convention

Services created by the extractor follow a naming scheme derived from their structural properties:

- Access ports: `access-vlan<N>` (e.g., `access-vlan10`)
- Trunk ports: `trunk-vlan<N>-<N>` or `trunk-all` depending on allowed VLANs
- Routed ports: `routed-<brief-description>`
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
- Interface name reverse-parsing tests
- Port grouping/clustering algorithm tests
- Prologue/epilogue derivation tests
- Config element matching tests
- Round-trip comparison normalization tests

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
- **Sub-interface modeling:** Extend port naming to `PortX.<N>` for sub-interfaces on physical ports
- **Incremental extraction:** Re-run extraction on a device that already has a config in the data store, updating only what changed
- **Config element unapply:** Leverage `unapply.txt` for generating change sets
- **Multi-device processing order:** When extracting multiple devices, services created by earlier devices are available for matching by later devices (processed sequentially, written to disk after each device)
