# punch

`punch` forwards TCP and UDP ports between two machines over iroh.

One side runs `punch out` to expose local services. The other side runs `punch in` to open local listeners and forward them to the remote peer.

## Current behavior

- TCP and UDP port mappings are implemented.
- TCP stdio mode such as `-:22` is implemented.
- Both peers must run `punch`.

## Build

```bash
cargo build --release
```

## Identity

- The secret key is stored at `~/.local/share/punch/secret.key`.
- On first run, `punch` creates the key and prints the path to stderr.
- `punch out` prints the public key to stderr. Use that key with `punch in`.

## Commands

Expose local ports on the remote machine:

```bash
punch out <port-spec>...
```

Connect to a remote peer and open local listeners:

```bash
punch in <pubkey> <mapping>...
```

Port format:

- `<port>` or `<port>/<proto>`
- `<proto>` is `tcp` or `udp`
- bare ports default to `tcp`

Mapping format:

- `<local>:<remote>` or `<local>:<remote>/<proto>`
- `-:<remote>` or `-:<remote>/tcp` for stdio mode
- `local` is the port opened on the machine running `punch in`
- `-` means use stdin/stdout instead of opening a local listener
- `remote` is the port reached on `127.0.0.1` on the machine running `punch out`
- bare mappings default to `tcp`

## Examples

Expose a remote HTTP service on port `8080`:

```bash
punch out 8080
```

Connect to it locally on port `3000`:

```bash
punch in <pubkey> 3000:8080
```

Then use:

```bash
curl http://127.0.0.1:3000
```

Expose a remote UDP service on port `53`:

```bash
punch out 53/udp
```

Connect to it locally on port `5300`:

```bash
punch in <pubkey> 5300:53/udp
```

Use stdio to reach a remote SSH service on port `22`:

```bash
punch out 22
```

```bash
punch in <pubkey> -:22
```

This connects stdin/stdout directly to the remote peer's `127.0.0.1:22`.

Multiple mappings in one process:

```bash
punch out 8080 53/udp
```

```bash
punch in <pubkey> 3000:8080 5300:53/udp
```
