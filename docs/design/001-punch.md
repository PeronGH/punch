# punch

Peer-to-peer TCP and UDP port forwarding over [iroh](https://github.com/n0-computer/iroh). Peers identify each other by public key; connections are direct and end-to-end encrypted.

iroh is a product requirement: the project's purpose is to provide port forwarding on top of iroh's peer-to-peer transport.

## Identity and Security

Each peer has a persistent identity derived from a secret key stored on disk.
The default storage location is `~/.local/share/punch/secret.key`; this path is overridable via the `PUNCH_SECRET_KEY` environment variable.
If no key file exists at the resolved path, the peer must generate a new key and persist it before proceeding.

The public key derived from the secret key is the peer's sole identifier.
All connections between peers are end-to-end encrypted by iroh's transport layer; punch adds no additional encryption or authentication beyond what iroh provides.

## CLI

punch exposes two subcommands: `out` (expose) and `in` (connect).

### `punch out` — expose local ports

```
punch out <port-spec>...
```

Each `<port-spec>` is `<port>` or `<port>/<proto>`, where `<proto>` is `tcp` (default) or `udp`.

Behavior:

- Parse and validate all port specs. Reject duplicates and invalid port numbers.
- Print the peer's public key to stderr on startup.
- Accept incoming connections from peers and proxy traffic to `127.0.0.1:<port>` for each exposed port.
- A connecting peer may only access ports that appear in the expose list; attempts to reach other ports must be refused.

### `punch in` — connect to a remote peer

```
punch in <pubkey> <mapping>...
```

Each `<mapping>` is `<local>:<remote-port>[/<proto>]`, where:

- `<local>` is a port number or `-` (stdio).
- `<remote-port>` is the port exposed by the remote peer.
- `<proto>` is `tcp` (default) or `udp`.

Behavior:

- **Port mapping** (`<local-port>:<remote-port>`): Open a local listener on `<local-port>`. For each accepted connection (TCP) or received packet (UDP), forward traffic to the remote peer's `<remote-port>`.
- **Stdio mapping** (`-:<remote-port>`): Do not open a local listener. Instead, bridge stdin/stdout directly to a single connection to the remote peer's `<remote-port>`. Only one stdio mapping is allowed per invocation. Stdio mode is TCP only.

Multiple mappings are allowed in a single invocation and share the same underlying connection to the remote peer.

## Forwarding Behavior

All traffic between two peers flows over a single connection.

### TCP

Each proxied TCP connection is an independent bidirectional byte stream. The server connects to `127.0.0.1:<port>` and copies bytes in both directions. TCP FIN in either direction closes the corresponding write side.

If the port is not in the expose list or the local TCP connection fails, the stream is terminated.

### UDP

Each proxied UDP packet is delivered independently. UDP semantics (unreliable, unordered) are preserved. Replies from the local service are routed back to the correct originating sender on the client side.

If the port is not in the expose list, the packet is silently dropped.

## Access Control

By default, any peer that knows the server's public key may connect.

The `PUNCH_ALLOW` environment variable accepts a comma-separated list of public keys. When set, only peers whose public key appears in the list are permitted to connect; all others must be rejected before any port proxying occurs.

## Configuration

All configuration is via environment variables. There are no config files.

| Variable           | Default                           | Description                                                                                            |
| ------------------ | --------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `PUNCH_SECRET_KEY` | `~/.local/share/punch/secret.key` | Path to the secret key file.                                                                           |
| `PUNCH_RELAY_MODE` | `default`                         | Relay behavior: `default` (use iroh's public relays), `disabled` (direct only), or a relay server URL. |
| `PUNCH_ALLOW`      | _(unset — allow all)_             | Comma-separated list of allowed peer public keys.                                                      |

## Non-Goals

The following are explicitly out of scope:

- **HTTP/TLS termination or any L7 awareness.** punch operates at L4 only.
- **Browser clients.** Only the punch CLI is a supported client.
- **Gateway for non-iroh clients.** Both peers must run punch.
- **Key management or PKI.** There is no certificate authority, key rotation, or revocation mechanism.

## Acceptance Criteria

1. `punch out` exposes specified TCP and UDP ports and prints the public key to stderr.
2. `punch in` with a port mapping opens a local listener and forwards traffic to the remote peer.
3. `punch in` with `-` as the local address bridges stdio to a single remote TCP connection.
4. TCP connections are proxied bidirectionally with correct FIN handling.
5. UDP packets are proxied with replies routed to the correct originating sender.
6. Connections to ports not in the expose list are refused (TCP) or silently dropped (UDP).
7. When `PUNCH_ALLOW` is set, only listed public keys may connect.
8. A new secret key is generated and persisted if none exists at the configured path.
9. Relay behavior respects `PUNCH_RELAY_MODE`.
10. Multiple mappings in a single `punch in` invocation share one connection.
