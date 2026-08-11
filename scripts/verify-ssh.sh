#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
binary="${1:-$repo_root/target/debug/terminal-rpg}"
pty_client="$repo_root/scripts/pty-ssh-client.py"
port="${TERMINAL_RPG_TEST_PORT:-22239}"
work_dir="$(mktemp -d)"
server_pid=""

cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$work_dir"
}
trap cleanup EXIT

if [ ! -x "$binary" ]; then
  echo "Build the binary first: cargo build" >&2
  exit 1
fi

"$binary" serve \
  --listen "127.0.0.1:$port" \
  --host-key "$work_dir/host-key" \
  --ascii --no-color >"$work_dir/server.log" 2>&1 &
server_pid="$!"

for _ in $(seq 1 50); do
  if ssh-keyscan -T 1 -p "$port" 127.0.0.1 >"$work_dir/keyscan" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
test -s "$work_dir/keyscan"

ssh_options=(
  -p "$port"
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
  -o PreferredAuthentications=none
  -o LogLevel=ERROR
)

# A protocol shell without a PTY and arbitrary exec requests must fail.
if timeout 5 ssh -T "${ssh_options[@]}" no-pty@127.0.0.1 </dev/null; then
  echo "shell request without PTY unexpectedly succeeded" >&2
  exit 1
fi
if timeout 5 ssh "${ssh_options[@]}" exec@127.0.0.1 id; then
  echo "exec request unexpectedly succeeded" >&2
  exit 1
fi

# A subsystem request must fail rather than expose an OS subsystem.
if timeout 5 sftp -q -b /dev/null -P "$port" \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=/dev/null \
  -o PreferredAuthentications=none subsystem@127.0.0.1; then
  echo "subsystem request unexpectedly succeeded" >&2
  exit 1
fi

# X11 is explicitly rejected while an otherwise valid game shell still works.
{ sleep 0.2; printf Q; } | env DISPLAY=:0 timeout 5 ssh -X -tt "${ssh_options[@]}" x11@127.0.0.1 >"$work_dir/x11.out" 2>&1
grep -aFq "X11 forwarding request failed" "$work_dir/x11.out"

# Two concurrent, known-size PTY sessions receive only their own game state.
python3 "$pty_client" --output "$work_dir/wait.out" --action s --resize 81x24 -- \
  ssh -tt "${ssh_options[@]}" wait-client@127.0.0.1 &
wait_pid="$!"
python3 "$pty_client" --output "$work_dir/help.out" --action '?' --resize 81x24 -- \
  ssh -tt "${ssh_options[@]}" help-client@127.0.0.1 &
help_pid="$!"
wait "$wait_pid"
wait "$help_pid"
perl -pe 's/\e\[[0-?]*[ -\/]*[@-~]//g' "$work_dir/wait.out" | tr -d '[:space:]' >"$work_dir/wait.text"
perl -pe 's/\e\[[0-?]*[ -\/]*[@-~]//g' "$work_dir/help.out" | tr -d '[:space:]' >"$work_dir/help.text"
grep -aFq "Turn1" "$work_dir/wait.text"
! grep -aFq "?:close|r:restart|Q:quit" "$work_dir/wait.text"
grep -aFq "?:close|r:restart|Q:quit" "$work_dir/help.text"
! grep -aFq "Turn1" "$work_dir/help.text"
wait_seed="$(grep -aoE 'Seed[0-9]+' "$work_dir/wait.text" | head -1)"
help_seed="$(grep -aoE 'Seed[0-9]+' "$work_dir/help.text" | head -1)"
test -n "$wait_seed"
test -n "$help_seed"
test "$wait_seed" != "$help_seed"

# An SSH restart creates a fresh run and remains playable.
python3 "$pty_client" --output "$work_dir/restart.out" --action r --resize 81x24 -- \
  ssh -tt "${ssh_options[@]}" restart-client@127.0.0.1
perl -pe 's/\e\[[0-?]*[ -\/]*[@-~]//g' "$work_dir/restart.out" | tr -d '[:space:]' >"$work_dir/restart.text"
grep -aFq "Thedungeonreformsaroundyou." "$work_dir/restart.text"
test "$(grep -aoE 'Seed[0-9]+' "$work_dir/restart.text" | sort -u | wc -l)" -ge 2

# Unsupported input is ignored without advancing the run.
python3 "$pty_client" --output "$work_dir/malformed.out" --action h --resize 81x24 -- \
  ssh -tt "${ssh_options[@]}" malformed-client@127.0.0.1
perl -pe 's/\e\[[0-?]*[ -\/]*[@-~]//g' "$work_dir/malformed.out" | tr -d '[:space:]' >"$work_dir/malformed.text"
grep -aFq "Turn0" "$work_dir/malformed.text"

# A flooding client that stops reading cannot delay an independent healthy one.
python3 "$pty_client" --output "$work_dir/slow.out" --spam 200 --stall -- \
  ssh -tt "${ssh_options[@]}" slow-client@127.0.0.1 &
slow_pid="$!"
python3 "$pty_client" --output "$work_dir/healthy.out" --action '?' --resize 81x24 -- \
  ssh -tt "${ssh_options[@]}" healthy-client@127.0.0.1
wait "$slow_pid"
perl -pe 's/\e\[[0-?]*[ -\/]*[@-~]//g' "$work_dir/healthy.out" | tr -d '[:space:]' >"$work_dir/healthy.text"
grep -aFq "?:close|r:restart|Q:quit" "$work_dir/healthy.text"

# Real window-change requests cross the minimum-size threshold both ways.
python3 "$pty_client" --output "$work_dir/resize.typescript" \
  --columns 80 --rows 24 --resize 60x20 --resize 80x24 -- \
  ssh -tt "${ssh_options[@]}" resize-client@127.0.0.1
perl -pe 's/\e\[[0-?]*[ -\/]*[@-~]//g' "$work_dir/resize.typescript" | tr -d '[:space:]' >"$work_dir/resize.text"
first_game_offset="$(grep -abo 'GRAVEKNIGHT' "$work_dir/resize.text" | head -1 | cut -d: -f1)"
small_offset="$(grep -abo 'Terminaltoosmall:' "$work_dir/resize.text" | head -1 | cut -d: -f1)"
last_game_offset="$(grep -abo 'GRAVEKNIGHT' "$work_dir/resize.text" | tail -1 | cut -d: -f1)"
test "$first_game_offset" -lt "$small_offset"
test "$small_offset" -lt "$last_game_offset"

# EOF without Q releases the game channel while the listener stays healthy.
timeout 5 ssh -tt "${ssh_options[@]}" eof-client@127.0.0.1 </dev/null >"$work_dir/eof.out" 2>&1 || true
{ sleep 0.2; printf Q; } | timeout 5 ssh -tt "${ssh_options[@]}" reconnect-client@127.0.0.1 >"$work_dir/reconnect.out" 2>&1
test -s "$work_dir/reconnect.out"

test "$(stat -c '%a' "$work_dir/host-key")" = "600"
echo "SSH transport verification passed"
