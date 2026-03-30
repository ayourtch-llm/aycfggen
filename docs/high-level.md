**Note:** This document contains the original design notes. It has been superseded by the formal specifications in `docs/specs/`. Field names, comment characters, and constraints in the formal specs take precedence over this document.

There are several input sources for the configuration generator. All the hashmaps in the descriptions below must be order-preserving.

1) Hardware templates directory

In this directory there are multiple subdirectories, each named after an SKU, that hold 
files with various platform-specific
mappings and information for a given platform.

ports.json:

Primary mapping for the time being is Port assignment, which is an order-preserving hashmap, with keys being "Port{numer}", and the values being as follows:

- "name": human-readable name portion of the interface, generally at the start, like "Ethernet" or "GigabitEthernet".
- "index": a string, describing the index of this interface inside this device, could be something like "0/0", or "1",
   or something along these lines. The full interface name on a standalone device would generally be thus a 
   concatenation of the name and index.

This format is supposed to extensible, so deserialization needs to take it in mind - extra fields should not be the cause for failure, unless the "strict" mode is used.

2) Logical Devices directory

In this directory, there are subdirectories, each named after a logical device name (can be the same 
or different from hostname), holding the various files with settings specific to the logical devices.

So far only one file is defined: config.json

This file is struct with the following data:

"config-template": holds the name of the configuration template; this is essentially just a filename, that, if relative
is taken from <AYCFGGEN-CFG-TEMPLATES> directory.

"software": this is a filename of software, if relative - relative to <AYCFGGEN-SOFTWARE-IMAGES> directory.
"role": a free-form very short string to denote the role of this logical device

"vars": this is a hashmap indexed by variable name (string) and the values are variable values (strings)

"singlemodule": boolean, if true - then "modules" must be exactly one element only.

"modules": a vector Option<> values, with each inner value being the record with the following fields:
- "SKU": a part number, maps to a part number in hardware templates directory
- "serial": an optional string with the serial number 
- "ports": a vector of records:
   - "name": "Port<N>" - N typically going from 0 to X, but not necessarily! used for lookup into o
     hardware templates ports.
   - "service": maps to "<short-service-name>" that will be used to look up the precise service config for this port.
   - "prologue": newline-separated list of commands to add before the service config
   - "epilogue": newline-separated list of commands to add after the service config
   - "vars": an ordered hashmap of name:value pairs, may be empty; merged with the "vars" of the logical device,
     overwrite those variables *for a specific port* (so, the upper hash needs to be cloned+augmented and then
     discarded before building config for the next port.

3) Service templates directory

This directory contains individual services configuration, again with subdirectories being the short names
of the services, as seen in "service" field on the port above; Contains the files:

port-config.txt:

A configuration of the given service that would be placed onto a physical port. In the future will
require variable expansion from device/port levels.

svi-config.txt:

if exists, the configuration of the SVI (a routable interface) that relates to this service.

4) Config templates directory (<AYCFGGEN-CFG-TEMPLATES>)

These are whole-box "first shot" templates minus the physical port configuration.

Has a collection of files that are used to generate the configurations, being the whole-box configurations,
minus the physical ports.

The configuration variables inside the templates will be expanded in the future from variables, by to-be-specified mechanisms; 

the SVI configuration should go into a place where "<SVI-CONFIGURATION>" marker is - and if not present, then at the end of file, in that case a comment "# use <SVI-CONFIGURATION> to place this configuration block" needs to be placed
as the very first line of it. The configuration block needs to be enclosed into "# SVI-START" and "# SVI-END" marker lines.

the ports configuration should go into a place where "<PORTS-CONFIGURATION>" marker is - if not present, then at the end of the file (in that case, a comment "# use <PORTS-CONFIGURATION> marker to place this configuration" needs to be placed right before the start of it.) The ports configuration section should be enclosed into "# PORTS-START" and "# PORTS-END" marker lines. 

