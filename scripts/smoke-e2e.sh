#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Usage: scripts/smoke-e2e.sh

Runs same-machine end-to-end smoke checks for:
  - TCP listener mapping
  - UDP listener mapping
  - TCP stdio mapping

Environment:
  PUNCH_BIN   Path to the punch binary to test (default: target/debug/punch)
EOF
  exit 0
fi

PUNCH_BIN="${PUNCH_BIN:-target/debug/punch}"

if [[ ! -x "$PUNCH_BIN" ]]; then
  echo "missing executable: $PUNCH_BIN" >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TCP_ECHO_PORT=43111
TCP_STDIO_PORT=43112
UDP_ECHO_PORT=43113
TCP_LOCAL_PORT=44111
UDP_LOCAL_PORT=44113

TMP_HOME_OUT=""
TMP_HOME_IN=""
OUT_LOG=""
TCP_IN_LOG=""
UDP_IN_LOG=""
STDIO_LOG=""
SERVER_PID=""
OUT_PID=""
TCP_IN_PID=""
UDP_IN_PID=""

cleanup() {
  for pid in "$TCP_IN_PID" "$UDP_IN_PID" "$OUT_PID" "$SERVER_PID"; do
    if [[ -n "$pid" ]]; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done

  rm -rf "$TMP_HOME_OUT" "$TMP_HOME_IN"
  rm -f "$OUT_LOG" "$TCP_IN_LOG" "$UDP_IN_LOG" "$STDIO_LOG"
}

trap cleanup EXIT

TMP_HOME_OUT="$(mktemp -d)"
TMP_HOME_IN="$(mktemp -d)"
OUT_LOG="$(mktemp)"
TCP_IN_LOG="$(mktemp)"
UDP_IN_LOG="$(mktemp)"
STDIO_LOG="$(mktemp)"

python3 -u - <<'PY' >/dev/null 2>&1 &
import socket
import threading

TCP_PORTS = [43111, 43112]
UDP_PORT = 43113

def handle_tcp(conn):
    with conn:
        while True:
            data = conn.recv(65536)
            if not data:
                return
            conn.sendall(data)

def tcp_server(port):
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("127.0.0.1", port))
    sock.listen()
    while True:
        conn, _ = sock.accept()
        threading.Thread(target=handle_tcp, args=(conn,), daemon=True).start()

def udp_server(port):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("127.0.0.1", port))
    while True:
        data, addr = sock.recvfrom(65536)
        sock.sendto(data, addr)

for port in TCP_PORTS:
    threading.Thread(target=tcp_server, args=(port,), daemon=True).start()
threading.Thread(target=udp_server, args=(UDP_PORT,), daemon=True).start()
threading.Event().wait()
PY
SERVER_PID=$!

HOME="$TMP_HOME_OUT" "$PUNCH_BIN" out "$TCP_ECHO_PORT" "$TCP_STDIO_PORT" "$UDP_ECHO_PORT/udp" \
  >/dev/null 2>"$OUT_LOG" &
OUT_PID=$!

PUBKEY=""
for _ in $(seq 1 100); do
  PUBKEY="$(sed -n 's/^public key: //p' "$OUT_LOG" | tail -n 1)"
  if [[ -n "$PUBKEY" ]]; then
    break
  fi
  sleep 0.1
done

if [[ -z "$PUBKEY" ]]; then
  echo "failed to read public key from punch out" >&2
  cat "$OUT_LOG" >&2
  exit 1
fi

HOME="$TMP_HOME_IN" "$PUNCH_BIN" in "$PUBKEY" "$TCP_LOCAL_PORT:$TCP_ECHO_PORT" \
  >/dev/null 2>"$TCP_IN_LOG" &
TCP_IN_PID=$!
sleep 2

TCP_RESULT="$(python3 - <<'PY'
import socket

sock = socket.create_connection(("127.0.0.1", 44111), timeout=5)
sock.sendall(b"tcp-e2e")
sock.shutdown(socket.SHUT_WR)
data = bytearray()
while True:
    chunk = sock.recv(65536)
    if not chunk:
        break
    data.extend(chunk)
print(data.decode())
PY
)"

if [[ "$TCP_RESULT" != "tcp-e2e" ]]; then
  echo "tcp smoke test failed: $TCP_RESULT" >&2
  cat "$TCP_IN_LOG" >&2
  exit 1
fi

kill "$TCP_IN_PID" 2>/dev/null || true
wait "$TCP_IN_PID" 2>/dev/null || true
TCP_IN_PID=""

HOME="$TMP_HOME_IN" "$PUNCH_BIN" in "$PUBKEY" "$UDP_LOCAL_PORT:$UDP_ECHO_PORT/udp" \
  >/dev/null 2>"$UDP_IN_LOG" &
UDP_IN_PID=$!
sleep 2

UDP_RESULT="$(python3 - <<'PY'
import socket
import sys
import time

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.bind(("127.0.0.1", 0))
sock.settimeout(2)
for i in range(5):
    payload = f"udp-e2e-{i}".encode()
    sock.sendto(payload, ("127.0.0.1", 44113))
    try:
        data, _ = sock.recvfrom(65536)
        print(data.decode())
        sys.exit(0)
    except TimeoutError:
        time.sleep(1)
sys.exit(1)
PY
)"

if [[ "$UDP_RESULT" != "udp-e2e-0" ]]; then
  echo "udp smoke test failed: $UDP_RESULT" >&2
  cat "$UDP_IN_LOG" >&2
  exit 1
fi

kill "$UDP_IN_PID" 2>/dev/null || true
wait "$UDP_IN_PID" 2>/dev/null || true
UDP_IN_PID=""

STDIO_RESULT="$(printf 'stdio-e2e' | HOME="$TMP_HOME_IN" "$PUNCH_BIN" in "$PUBKEY" -:43112 2>"$STDIO_LOG")"

if [[ "$STDIO_RESULT" != "stdio-e2e" ]]; then
  echo "stdio smoke test failed: $STDIO_RESULT" >&2
  cat "$STDIO_LOG" >&2
  exit 1
fi

echo "smoke e2e passed"
