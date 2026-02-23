# Extras

Adds stdio mode, access control, and environment-variable configuration to the existing core.

Depends on: [punch design spec](../design/punch.md), [core TCP](core-tcp.md), [core UDP](core-udp.md)

## Stdio Mode

### CLI

`punch in` accepts `-` as the local address in a mapping: `-:<remote-port>` or `-:<remote-port>/tcp`.

Constraints:

- At most one stdio mapping per invocation. If more than one is specified, exit with an error.
- Stdio is TCP only. `-:<port>/udp` is rejected with an error.

### Behavior

Instead of binding a local listener:

1. Open a bidi stream on the QUIC connection and write the 2-byte remote port.
2. Bridge stdin to the QUIC send stream and the QUIC recv stream to stdout.
3. When stdin reaches EOF, finish the send stream.
4. When the recv stream reaches EOF, exit with status 0.
5. If the QUIC stream is reset by the server (port not exposed or connection refused), exit with a non-zero status and print the error to stderr.

Stdio mapping must coexist with port mappings in the same invocation. The stdio bridge runs concurrently alongside any local listeners.

### Raw I/O

Stdin and stdout must be used in raw byte mode. No line buffering, no newline translation. This is required for `ProxyCommand` compatibility with SSH.

## Access Control (`PUNCH_ALLOW`)

### Behavior

On the `out` side, if `PUNCH_ALLOW` is set:

1. Parse it as a comma-separated list of base32 node IDs. Whitespace around each entry is trimmed.
2. If any entry is malformed, exit with an error at startup (fail-fast, not at connection time).
3. On each accepted connection, check the remote peer's node ID against the allowlist. If not present, close the connection immediately and log the rejection to stderr.

If `PUNCH_ALLOW` is unset or empty, all peers are allowed (current default behavior).

### Scope

Access control applies per-connection, not per-stream. Once a peer is accepted, it may open streams/send datagrams to any exposed port. The port-level validation defined in the core specs still applies independently.

## Environment Variables

### `PUNCH_SECRET_KEY`

Overrides the default key path `~/.local/share/punch/secret.key`. When set, the key load/generate logic uses this path instead. The value must be a valid file path; no further validation is performed (I/O errors are reported as usual).

### `PUNCH_RELAY_MODE`

Controls the iroh endpoint's relay configuration:

- `default` or unset: use iroh's default relay servers.
- `disabled`: set relay mode to disabled (direct connections only).
- Any other value: treat as a relay server URL and configure a custom relay map with that single URL.

If the URL is malformed, exit with an error at startup.

### Parsing

Environment variables are read once at startup before the endpoint is created. They are not re-read during the lifetime of the process.

## Error Contracts

- Malformed `PUNCH_ALLOW` entries → exit with error at startup.
- Malformed `PUNCH_RELAY_MODE` URL → exit with error at startup.
- Rejected peer (allowlist) → connection closed, event logged to stderr, server continues.
- Stdio I/O errors → exit with non-zero status.

## Tests

- **Stdio mapping parsing**: `-:22` accepted, `-:22/udp` rejected, multiple stdio mappings rejected.
- **`PUNCH_ALLOW` parsing**: valid list, whitespace trimming, malformed entry detection.
- **`PUNCH_RELAY_MODE` parsing**: `default`, `disabled`, valid URL, malformed URL.
- **`PUNCH_SECRET_KEY` override**: custom path is used for key load/generate.
