# lanlord

A self-hosted inventory of the devices on a single home network segment. lanlord scans the segment, remembers what it has seen, and shows it in a web UI.

## Language

### Devices

**Device**:
A host observed on the segment, identified by its MAC address.
_Avoid_: host, node, client, asset

**Inventory**:
The persisted set of every Device ever seen, including ones absent from the current scan.

**Presence**:
Whether a Device answered the most recent Scan.
_Avoid_: online, up, active

**First seen** / **Last seen**:
The timestamps of a Device's earliest and most recent observation.

### Scanning

**Segment**:
The single IPv4 subnet reachable by ARP from the scanned interface. Scanning one Segment is the whole of v1's reach.
_Avoid_: network, LAN, subnet, VLAN

**Scan**:
One complete ARP sweep of the Segment, producing the set of Devices currently present.
_Avoid_: sweep, discovery, probe

**Scan interval**:
The time between the starts of consecutive Scans.
