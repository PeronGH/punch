# Core TCP

Implements the minimum viable punch: secret key management, iroh endpoint, CLI, and TCP-only proxying.

Depends on: [punch design spec](../design/001-punch.md)

## Dependencies

- **iroh** — peer-to-peer QUIC transport (endpoint, connections, bidi streams).
- **clap** — CLI argument parsing with derive macros.
- **tokio** — async runtime. Required by iroh; also used for TCP listeners and I/O.
- **anyhow** — error handling.

## Secret Key

On startup, both `out` and `in` load the secret key from the configured path (see design spec).

- If the file exists, read 32 bytes and construct the key.
- If the file does not exist, generate a new key, create parent directories, write it with mode `0600`, then proceed.
- If the file exists but is unreadable or malformed, exit with an error.

## Endpoint

Both subcommands create an iroh `Endpoint`:

- `out`: configured with ALPN `punch/0` (to accept connections) and the loaded secret key.
- `in`: configured with the loaded secret key only (no ALPN needed — it only connects, never accepts).

The endpoint uses iroh's default relay mode.

## CLI

Single binary with two subcommands.

### `punch out <port>...`

Each argument is a bare port number (e.g. `8080`). Each port must be 1–65535, no duplicates.

On startup, print the node ID (public key) to stderr in iroh's standard base32 format.

### `punch in <pubkey> <mapping>...`

`<pubkey>` is the remote peer's node ID in base32.

Each `<mapping>` is `<local-port>:<remote-port>`. Both ports must be 1–65535.

## Server (`out`) Loop

1. Call `endpoint.accept()` in a loop.
2. For each accepted connection, spawn a task that runs an inner loop calling `connection.accept_bi()`.
3. For each accepted bidi stream:
   a. Read exactly 2 bytes (big-endian port number).
   b. If the port is not in the expose list, reset the stream and continue.
   c. Open a TCP connection to `127.0.0.1:<port>`.
   d. If the TCP connection fails, reset the stream and continue.
   e. Spawn a task that copies bytes bidirectionally between the QUIC stream pair and the TCP stream. When either read side reaches EOF, finish/shutdown the corresponding write side.

## Client (`in`) Loop

1. Connect to the remote peer: `endpoint.connect(NodeAddr::new(pubkey), b"punch/0")`.
2. For each TCP mapping, bind a `TcpListener` on `127.0.0.1:<local-port>`.
3. For each accepted TCP connection:
   a. Open a bidi stream on the QUIC connection.
   b. Write the 2-byte remote port (big-endian).
   c. Copy bytes bidirectionally between the TCP stream and the QUIC stream pair, with the same EOF/finish handling as the server side.

All listeners run concurrently via `tokio::select!` or equivalent; they share the single QUIC connection.

## Bidirectional Copy

The copy logic between a QUIC stream pair `(SendStream, RecvStream)` and a `TcpStream` (split into read/write halves) runs two concurrent tasks:

- **QUIC → TCP**: read from `RecvStream`, write to TCP write half. On EOF, shutdown the TCP write half.
- **TCP → QUIC**: read from TCP read half, write to `SendStream`. On EOF, call `finish()` on the `SendStream`.

When both directions complete, the task exits. If either direction errors, both are cancelled.

## Error Contracts

- Malformed CLI arguments → exit with a non-zero status and a human-readable message.
- Secret key I/O failure → exit with error.
- Individual stream/connection failures are logged to stderr and do not terminate the process.
- If the QUIC connection to the remote peer is lost, `in` must exit with an error.

## Tests

- **Port parsing**: valid (`8080`, `22`), invalid (`0`, `70000`, `abc`), duplicate detection.
- **Mapping parsing**: valid (`4000:8080`), invalid (`0:80`, `80`, `abc:80`).
- **Bidirectional copy**: spin up a real iroh endpoint pair and a TCP listener, proxy a stream end-to-end, verify data flows correctly in both directions and EOF propagation works.
