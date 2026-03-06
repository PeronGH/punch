#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Usage: scripts/smoke-tier1.sh

Runs the reliable GitHub-runner smoke check:
  - TCP listener mapping only

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
TCP_LOCAL_PORT=44111

TMP_HOME_OUT=""
TMP_HOME_IN=""
OUT_LOG=""
TCP_IN_LOG=""
SERVER_PID=""
OUT_PID=""
TCP_IN_PID=""

announce() {
  echo "tier1: $*" >&2
}

dump_logs() {
  local label="$1"
  local path="$2"
  if [[ -f "$path" && -s "$path" ]]; then
    echo "--- ${label} ---" >&2
    cat "$path" >&2
  fi
}

fail() {
  local message="$1"
  echo "$message" >&2
  dump_logs "punch out" "$OUT_LOG"
  dump_logs "punch in (tcp)" "$TCP_IN_LOG"
  exit 1
}

stop_process() {
  local pid_var="$1"
  local pid="${!pid_var}"
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    printf -v "$pid_var" '%s' ""
  fi
}

process_is_alive() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null
}

cleanup() {
  stop_process TCP_IN_PID
  stop_process OUT_PID
  stop_process SERVER_PID
  rm -rf "$TMP_HOME_OUT" "$TMP_HOME_IN"
  rm -f "$OUT_LOG" "$TCP_IN_LOG"
}

trap cleanup EXIT

TMP_HOME_OUT="$(mktemp -d)"
TMP_HOME_IN="$(mktemp -d)"
OUT_LOG="$(mktemp)"
TCP_IN_LOG="$(mktemp)"

announce "start tcp echo server"
python3 -u - <<'PY' >/dev/null 2>&1 &
import socket
import threading

PORT = 43111

def handle(conn):
    with conn:
        while True:
            data = conn.recv(65536)
            if not data:
                return
            conn.sendall(data)

sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
sock.bind(("127.0.0.1", PORT))
sock.listen()

while True:
    conn, _ = sock.accept()
    threading.Thread(target=handle, args=(conn,), daemon=True).start()
PY
SERVER_PID=$!

announce "start punch out"
HOME="$TMP_HOME_OUT" "$PUNCH_BIN" out "$TCP_ECHO_PORT" >/dev/null 2>"$OUT_LOG" &
OUT_PID=$!

PUBKEY=""
for attempt in $(seq 1 100); do
  PUBKEY="$(sed -n 's/^public key: //p' "$OUT_LOG" | tail -n 1)"
  if [[ -n "$PUBKEY" ]]; then
    announce "got pubkey on attempt ${attempt}"
    break
  fi
  if ! kill -0 "$OUT_PID" 2>/dev/null; then
    wait "$OUT_PID" 2>/dev/null || true
    fail "tier1: punch out exited before publishing a key"
  fi
  sleep 0.1
done

if [[ -z "$PUBKEY" ]]; then
  fail "tier1: timed out waiting for punch out public key"
fi

start_tcp_mapping() {
  local pubkey="$1"
  local log_path="$2"

  HOME="$TMP_HOME_IN" "$PUNCH_BIN" in "$pubkey" "$TCP_LOCAL_PORT:$TCP_ECHO_PORT" \
    >/dev/null 2>"$log_path" &
  TCP_IN_PID=$!
}

wait_for_tcp_listener() {
  local pid="$1"

  for attempt in $(seq 1 100); do
    if ! process_is_alive "$pid"; then
      return 1
    fi

    announce "wait for tcp listener attempt ${attempt}"
    if python3 - <<'PY' >/dev/null 2>&1
import socket

sock = socket.socket()
sock.settimeout(0.2)
try:
    sock.connect(("127.0.0.1", 44111))
except OSError:
    raise SystemExit(1)
else:
    raise SystemExit(0)
finally:
    sock.close()
PY
    then
      return 0
    fi

    sleep 0.1
  done

  return 1
}

announce "start punch in tcp mapping"
for attempt in $(seq 1 10); do
  : >"$TCP_IN_LOG"
  announce "tcp launch attempt ${attempt}"
  start_tcp_mapping "$PUBKEY" "$TCP_IN_LOG"
  if wait_for_tcp_listener "$TCP_IN_PID"; then
    announce "tcp listener ready"
    break
  fi
  stop_process TCP_IN_PID
  sleep 1
done

if [[ -z "$TCP_IN_PID" ]]; then
  fail "tier1: tcp mapping never became ready"
fi

TCP_RESULT="$(python3 - <<'PY'
import socket

sock = socket.create_connection(("127.0.0.1", 44111), timeout=5)
sock.sendall(b"tcp-tier1")
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

if [[ "$TCP_RESULT" != "tcp-tier1" ]]; then
  fail "tier1: tcp probe failed: ${TCP_RESULT}"
fi

announce "tcp probe passed"
echo "tier1: success"
