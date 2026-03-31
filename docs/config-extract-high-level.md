aycfgextract: this is a sub-library with the thin wrapper binary on top of it for retrieving the Cisco IOS/IOS XE configurations from the live devices and updating the templates/configurations information stores that are defined by aycfggen, and using the ssh/telnet connection library from ../ayclic. Initially focused on IOS/IOS XE, this may add more OSes and become multivendor in the future, so this needs to be taken into consideration.

It should accept the command line parameters in the same fashion as aycfggen compiler binary does - with root folder + overrides, additive. However, it will accept only one copy of each - because those locations will be also used to write data.

The arguments should include one or more IPv4/IPv6 addresses which are the target devices, and the username/password for the ssh should be taken from the environment variables (let's try to keep this modular as we might need to introduce per-device credentials and/or different methods of access and authentication later).

For each address, a discovery procedure must be performed.

Upon connection to the device, we first need to discover the hardware profile of the device. It needs to start with "show version" and 
"show inventory" to discover the SKU, and check if a given hardware profile for this SKU exists.

If it does not (or if the flag is set requested to recreate the hardware profiles):

Create the hardware profile, based on the specification in aycfggen crate, and write it to the specified location.

With the hardware profile in place, split the configurations:

1) SVIs - see if you can match existing service configurations, if not - create services named "VLAN-SERVICE-<N>" where N is the vlan number of the SVI. If there is a comment, you can extract unique part of the comment (short) and convert to acceptable value for service SVI configuration.

2) Ports - port configuration is also extracted the same way and compared with the existing service configurations. If there is 1-2 line difference
and it is on a single port only, that can go into prologue/epilogue, if there are many ports with this change - a new service must be created
(correspondingly, in this case the SVI config will land in a different place).

3) config elements. This is is a best-effort: if there are config elements that match the parts of the configuration, then 
tweak the template so it uses them, however, this is not required.

The final result of the extraction should be compilable by ayccfggen, and result in creating an identical configuration.
The unit tests should include using aycfgextract + aycfggen to perform round-trip configuration -> parts -> configuration.

The discovered logical device configuration template should be placed into a file with the name containing the serial number - this would be
unique for the time of the discovery and would need to be renamed by the user later.

The connection to the device should always perform the same set of commands that is enough to perform the full set of actions, and the executable
should also have a mode where one supplies the text file with this full set of commands, and the extraction functions entirely offline.

The list of commands to be run is intentionally left open-ended, such that they can be added/changed in a way that is the most suitable.


