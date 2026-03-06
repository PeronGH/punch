#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Usage: scripts/smoke-e2e.sh

Runs the full same-machine end-to-end smoke checks:
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

announce() {
  echo "e2e: $*" >&2
}

dump_logs() {
  local label="$1"
  local path="$2"
  if [[ -f "$path" && -s "$path" ]]; then
    echo "--- ${label} ---" >&2
    cat "$path" >&2
  fi
}

dump_logs_and_fail() {
  local message="$1"
  shift
  echo "$message" >&2
  while (($# > 1)); do
    dump_logs "$1" "$2"
    shift 2
  done
  exit 1
}

process_is_alive() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null
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

wait_for_public_key() {
  local pid="$1"
  local log_path="$2"
  local pubkey=""

  for _ in $(seq 1 100); do
    announce "wait for punch out pubkey"
    pubkey="$(sed -n 's/^public key: //p' "$log_path" | tail -n 1)"
    if [[ -n "$pubkey" ]]; then
      printf '%s\n' "$pubkey"
      return 0
    fi
    if ! process_is_alive "$pid"; then
      wait "$pid" 2>/dev/null || true
      dump_logs_and_fail "punch out exited before becoming ready" "punch out" "$log_path"
    fi
    sleep 0.1
  done

  dump_logs_and_fail "failed to read public key from punch out" "punch out" "$log_path"
}

wait_for_tcp_listener() {
  local pid="$1"
  local port="$2"

  for _ in $(seq 1 100); do
    announce "wait for tcp listener"
    if ! process_is_alive "$pid"; then
      return 1
    fi
    if python3 - "$port" <<'PY' >/dev/null 2>&1
import socket
import sys

sock = socket.socket()
sock.settimeout(0.2)
try:
    sock.connect(("127.0.0.1", int(sys.argv[1])))
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

start_tcp_mapping() {
  local pubkey="$1"
  local log_path="$2"

  announce "start tcp listener mapping"
  HOME="$TMP_HOME_IN" "$PUNCH_BIN" in "$pubkey" "$TCP_LOCAL_PORT:$TCP_ECHO_PORT" \
    >/dev/null 2>"$log_path" &
  TCP_IN_PID=$!
}

run_udp_probe() {
  python3 - <<'PY'
import socket
import sys
import time

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.bind(("127.0.0.1", 0))
sock.settimeout(0.5)
for i in range(20):
    payload = f"udp-e2e-{i}".encode()
    sock.sendto(payload, ("127.0.0.1", 44113))
    try:
        data, _ = sock.recvfrom(65536)
        print(data.decode())
        sys.exit(0)
    except TimeoutError:
        time.sleep(0.25)
sys.exit(1)
PY
}

wait_for_udp_mapping() {
  local pid="$1"

  for _ in $(seq 1 5); do
    announce "wait for udp mapping"
    if ! process_is_alive "$pid"; then
      return 1
    fi
    if UDP_RESULT="$(run_udp_probe)"; then
      if [[ "$UDP_RESULT" == udp-e2e-* ]]; then
        printf '%s\n' "$UDP_RESULT"
        return 0
      fi
      return 1
    fi
    sleep 0.5
  done

  return 1
}

start_udp_mapping() {
  local pubkey="$1"
  local log_path="$2"

  announce "start udp listener mapping"
  HOME="$TMP_HOME_IN" "$PUNCH_BIN" in "$pubkey" "$UDP_LOCAL_PORT:$UDP_ECHO_PORT/udp" \
    >/dev/null 2>"$log_path" &
  UDP_IN_PID=$!
}

launch_tcp_until_ready() {
  local pubkey="$1"
  local log_path="$2"

  for attempt in $(seq 1 10); do
    announce "tcp launch attempt ${attempt}"
    : >"$log_path"
    start_tcp_mapping "$pubkey" "$log_path"
    if wait_for_tcp_listener "$TCP_IN_PID" "$TCP_LOCAL_PORT"; then
      return 0
    fi
    stop_process TCP_IN_PID
    sleep 1
  done

  dump_logs_and_fail "tcp mapping never became ready" "punch in (tcp)" "$log_path"
}

launch_udp_until_ready() {
  local pubkey="$1"
  local log_path="$2"

  for attempt in $(seq 1 10); do
    announce "udp launch attempt ${attempt}"
    : >"$log_path"
    start_udp_mapping "$pubkey" "$log_path"
    if UDP_RESULT="$(wait_for_udp_mapping "$UDP_IN_PID")"; then
      printf '%s\n' "$UDP_RESULT"
      return 0
    fi
    stop_process UDP_IN_PID
    sleep 1
  done

  dump_logs_and_fail "udp mapping never became ready" "punch in (udp)" "$log_path"
}

run_stdio_probe() {
  local pubkey="$1"
  local log_path="$2"
  local output=""
  local status=0

  for attempt in $(seq 1 10); do
    announce "stdio attempt ${attempt}"
    : >"$log_path"
    set +e
    output="$(printf 'stdio-e2e' | HOME="$TMP_HOME_IN" "$PUNCH_BIN" in "$pubkey" -:43112 2>"$log_path")"
    status=$?
    set -e

    if (( status == 0 )); then
      printf '%s\n' "$output"
      return 0
    fi

    sleep 1
  done

  dump_logs_and_fail "stdio smoke test failed with exit code $status" "stdio" "$log_path"
}

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

announce "start local tcp and udp echo servers"
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

announce "start punch out"
HOME="$TMP_HOME_OUT" "$PUNCH_BIN" out "$TCP_ECHO_PORT" "$TCP_STDIO_PORT" "$UDP_ECHO_PORT/udp" \
  >/dev/null 2>"$OUT_LOG" &
OUT_PID=$!

PUBKEY="$(wait_for_public_key "$OUT_PID" "$OUT_LOG")"

announce "run tcp phase"
launch_tcp_until_ready "$PUBKEY" "$TCP_IN_LOG"

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
  dump_logs_and_fail "tcp smoke test failed: $TCP_RESULT" "punch in (tcp)" "$TCP_IN_LOG"
fi

announce "tcp phase passed"
stop_process TCP_IN_PID

announce "run udp phase"
UDP_RESULT="$(launch_udp_until_ready "$PUBKEY" "$UDP_IN_LOG")"

if [[ "$UDP_RESULT" != udp-e2e-* ]]; then
  dump_logs_and_fail "udp smoke test failed: $UDP_RESULT" "punch in (udp)" "$UDP_IN_LOG"
fi

announce "udp phase passed"
stop_process UDP_IN_PID

announce "run stdio phase"
STDIO_RESULT="$(run_stdio_probe "$PUBKEY" "$STDIO_LOG")"

if [[ "$STDIO_RESULT" != "stdio-e2e" ]]; then
  dump_logs_and_fail "stdio smoke test failed: $STDIO_RESULT" "stdio" "$STDIO_LOG"
fi

announce "stdio phase passed"
echo "e2e: success"
