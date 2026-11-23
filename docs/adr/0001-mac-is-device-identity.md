# MAC address is the device identity

A Device's identity is its MAC address; its IP is only the most recent address observed. The alternative, keying on IP, would reshuffle the Inventory whenever DHCP reassigns leases and make "first seen" meaningless.

The consequence is a hard boundary: a MAC is only visible on the same L2 segment, so devices behind a router can never be inventoried, and `Segment` is a one-subnet concept by construction. Changing identity later would require rewriting all accumulated history, so treat this as permanent.
