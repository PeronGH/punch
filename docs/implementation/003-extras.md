# Extras

Adds stdio mode to the existing core.

Depends on: [punch design spec](../design/001-punch.md), [core TCP](001-core-tcp.md), [core UDP](002-core-udp.md)

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

## Error Contracts

- Stdio I/O errors → exit with non-zero status.

## Tests

- **Stdio mapping parsing**: `-:22` accepted, `-:22/udp` rejected, multiple stdio mappings rejected.
