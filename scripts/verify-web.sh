#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
binary="${1:-$repo_root/target/debug/terminal-rpg}"
port="${TERMINAL_RPG_WEB_TEST_PORT:-22240}"
work_dir="$(mktemp -d)"
server_pid=""
item_server_pid=""

cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [ -n "$item_server_pid" ]; then
    kill "$item_server_pid" 2>/dev/null || true
    wait "$item_server_pid" 2>/dev/null || true
  fi
  rm -rf "$work_dir"
}
trap cleanup EXIT

if [ ! -x "$binary" ]; then
  echo "Build the binary first: cargo build" >&2
  exit 1
fi

"$binary" web --listen "127.0.0.1:$port" --seed 29 >"$work_dir/server.log" 2>&1 &
server_pid="$!"

curl --fail --silent --show-error --retry 50 --retry-all-errors \
  --retry-delay 0 --max-time 5 "http://127.0.0.1:$port/" >"$work_dir/index.html"
grep -Fq 'id="terminal"' "$work_dir/index.html"

python3 "$repo_root/scripts/verify-web.py" --port "$port"

item_port="$((port + 1))"
"$binary" web --listen "127.0.0.1:$item_port" --seed 14 >"$work_dir/item-server.log" 2>&1 &
item_server_pid="$!"
curl --fail --silent --show-error --retry 50 --retry-all-errors \
  --retry-delay 0 --max-time 5 "http://127.0.0.1:$item_port/" >/dev/null
python3 "$repo_root/scripts/verify-web.py" --port "$item_port" --item-flow

kill -0 "$server_pid"
! grep -Fq "panicked" "$work_dir/server.log"
! grep -Fq "panicked" "$work_dir/item-server.log"
