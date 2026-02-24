# Core UDP

Adds UDP port forwarding to the existing TCP core.

Depends on: [punch design spec](../design/001-punch.md), [core TCP](001-core-tcp.md)

## Dependencies

No new dependencies beyond those in core-tcp.

## CLI Changes

This spec introduces the `/<proto>` suffix to the CLI. In core-tcp, arguments were bare numbers only.

### `punch out`

Port specs now accept an optional `/<proto>` suffix: `<port>/tcp` or `<port>/udp`. A bare port (no suffix) remains TCP. A port may be exposed as TCP, UDP, or both via separate specs (e.g. `53/tcp 53/udp`).

### `punch in`

Mappings now accept an optional `/<proto>` suffix: `<local>:<remote>/tcp` or `<local>:<remote>/udp`. A bare mapping (no suffix) remains TCP.

## Datagram Wire Format

All UDP traffic uses QUIC datagrams. Each datagram has a header containing a 2-byte flow ID and optionally a 2-byte destination port, all big-endian.

**Client → Server** (4-byte header): `[flow_id: u16][dest_port: u16][payload]`

- `flow_id` identifies a unique local sender (source address) on the client side.
- `dest_port` tells the server which exposed port to forward to.

**Server → Client** (2-byte header): `[flow_id: u16][payload]`

- The client assigned the flow ID, so it already knows which mapping and local sender it belongs to. No port is needed.

## Client (`in`) — Datagram Handling

For each UDP mapping, bind a `UdpSocket` on `127.0.0.1:<local-port>`.

### Sending (local → remote)

On each received local packet, the client:

1. Looks up the source address in a flow table. If no entry exists, assigns a new flow ID (monotonically incrementing `u16`, wrapping).
2. Sends a QUIC datagram: `[flow_id][remote_port][payload]`.

### Receiving (remote → local)

On each received QUIC datagram:

1. Read the 2-byte flow ID.
2. Look up the flow ID to find the local source address and mapping.
3. Send the payload to that address via the mapping's local socket.

If the flow ID is unknown, drop the datagram.

### Flow cleanup

Flows are evicted after 5 minutes of inactivity (no packets in either direction). The timeout resets on every sent or received packet for that flow.

## Server (`out`) — Datagram Handling

In addition to the existing `accept_bi` loop, the server runs a concurrent loop receiving QUIC datagrams from the connection.

### Receiving (client → local)

1. Read the 4-byte header: `[flow_id][dest_port]`.
2. If the port is not in the UDP expose list, drop silently.
3. Look up the flow ID in a per-connection flow table. If no entry exists, create one: bind a new `UdpSocket` to an ephemeral local port.
4. Send the payload to `127.0.0.1:<dest_port>` via that flow's socket.

Each flow gets its own ephemeral socket so that replies can be attributed to the correct flow ID.

### Sending (local → client)

Each flow's socket is polled for incoming packets. When a reply arrives:

1. Send a QUIC datagram: `[flow_id][payload]`.

### Flow cleanup

Flows are evicted after 5 minutes of inactivity. On eviction, the flow's UDP socket is closed.

## Demultiplexing

Both the `accept_bi` loop (TCP) and the datagram receive loop (UDP) read from the same QUIC connection. These run as concurrent tasks. iroh's `Connection` supports concurrent `accept_bi` and datagram reads without additional synchronization.

On the client side, incoming datagrams are dispatched by flow ID to the correct mapping handler.

## Error Contracts

- Local UDP send/receive errors are logged to stderr and do not terminate the process.
- A missing or unreachable local UDP target is logged on first occurrence per flow and the datagram is dropped.

## Tests

- **Port-spec parsing with UDP**: `53/udp`, `53/tcp 53/udp`, mixed specs.
- **Mapping parsing with UDP**: `5300:53/udp`, rejection of `-:53/udp`.
- **Datagram framing**: client→server 4-byte header and server→client 2-byte header are correctly constructed and parsed.
- **Flow routing**: two distinct local senders to the same mapping receive their own replies correctly.
- **Flow timeout**: inactive flows are evicted after 5 minutes.
