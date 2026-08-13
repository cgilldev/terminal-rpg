#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/redeploy.sh --host HOST [options]
       scripts/redeploy.sh --local [options]

Build and safely redeploy terminal-rpg-web to a remote systemd host.

Options:
  --host HOST              SSH destination (required unless --dry-run)
  --local                  Deploy to this machine (same target as the original deployment)
  --user USER              SSH user (default: dev)
  --port PORT              SSH port (default: 22)
  --binary PATH            Remote binary (default: /usr/local/bin/terminal-rpg)
  --service NAME           Web service (default: terminal-rpg-web.service)
  --config-dir PATH        Remote deployment config directory (default: /etc/terminal-rpg)
  --health-url URL         Remote health URL (default: http://127.0.0.1:8081/)
  --ssh-option VALUE       Extra option passed to ssh/scp (repeatable)
  --sudo                   Run remote install/service commands via sudo -n
  --dry-run                Print the plan without building, copying, or changing state
  --skip-checks            Skip local cargo fmt/check before building
  --help                   Show this help
EOF
}

die() { printf 'redeploy: %s\n' "$*" >&2; exit 1; }
log() { printf '[redeploy] %s\n' "$*"; }

host= user=dev port=22 remote_binary=/usr/local/bin/terminal-rpg
service=terminal-rpg-web.service config_dir=/etc/terminal-rpg
health_url=http://127.0.0.1:8081/ dry_run=false skip_checks=false use_sudo=false local_mode=false
ssh_options=()

while (($#)); do
  case "$1" in
    --host) [[ $# -ge 2 ]] || die "--host needs a value"; host=$2; shift 2 ;;
    --local) local_mode=true; shift ;;
    --user) [[ $# -ge 2 ]] || die "--user needs a value"; user=$2; shift 2 ;;
    --port) [[ $# -ge 2 ]] || die "--port needs a value"; port=$2; shift 2 ;;
    --binary) [[ $# -ge 2 ]] || die "--binary needs a value"; remote_binary=$2; shift 2 ;;
    --service) [[ $# -ge 2 ]] || die "--service needs a value"; service=$2; shift 2 ;;
    --config-dir) [[ $# -ge 2 ]] || die "--config-dir needs a value"; config_dir=$2; shift 2 ;;
    --health-url) [[ $# -ge 2 ]] || die "--health-url needs a value"; health_url=$2; shift 2 ;;
    --ssh-option) [[ $# -ge 2 ]] || die "--ssh-option needs a value"; ssh_options+=("$2"); shift 2 ;;
    --dry-run) dry_run=true; shift ;;
    --sudo) use_sudo=true; shift ;;
    --skip-checks) skip_checks=true; shift ;;
    --help|-h) usage; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
binary="$repo_root/target/release/terminal-rpg"
[[ -f "$repo_root/Cargo.toml" ]] || die "Cargo.toml not found"
[[ -f "$repo_root/deploy/terminal-rpg-web.service" ]] || die "web service template missing"
[[ -f "$repo_root/deploy/cloudflared-rpg.service" ]] || die "tunnel service template missing"
[[ -f "$repo_root/deploy/cloudflared-rpg.yml" ]] || die "tunnel config template missing"

if ! $dry_run; then
  if ! $local_mode; then [[ -n "$host" ]] || die "--host is required unless --dry-run is used"; fi
  command -v cargo >/dev/null || die "cargo is required"
  if ! $local_mode; then
    command -v ssh >/dev/null || die "ssh is required"
    command -v scp >/dev/null || die "scp is required"
  fi
  if ! $skip_checks; then
    log "running cargo fmt/check"
    cargo fmt --check
    cargo check --all-targets --all-features
  fi
  log "building release binary"
  cargo build --release --all-features
  [[ -x "$binary" ]] || die "release binary was not produced: $binary"
else
  log "dry run: no build, SSH, copy, service, or health-check side effects"
fi

destination="${user}@${host}"
ssh_cmd=(ssh -p "$port" "${ssh_options[@]}" "$destination")
scp_cmd=(scp -P "$port" "${ssh_options[@]}" )
stage="/tmp/terminal-rpg-stage"
backup="${remote_binary}.previous"

if $local_mode; then log "destination: local machine"; else log "destination: $destination"; fi
log "stage binary at: $stage/terminal-rpg"
log "install service: $service (sudo: $use_sudo)"
log "health check: $health_url"
log "transfer only: release binary and deploy/*.service/yml (no secrets or repository files)"

if $dry_run; then
  log "would run: cargo build --release --all-features"
  log "would copy binary and deployment templates with scp"
  log "would stage, backup, install, restart, health-check, and rollback on failure"
  exit 0
fi

remote_exec() {
  local command_text=$1
  if $local_mode; then
    if $use_sudo; then sudo -n bash -c "$command_text"; else bash -c "$command_text"; fi
  elif $use_sudo; then
    "${ssh_cmd[@]}" "sudo -n bash -c $(printf %q "$command_text")"
  else
    "${ssh_cmd[@]}" "$command_text"
  fi
}
remote_exec "set -Eeuo pipefail; mkdir -p '$stage' '$config_dir'"
if $local_mode; then
  if $use_sudo; then
    sudo -n cp "$binary" "$stage/terminal-rpg"
    sudo -n cp "$repo_root/deploy/terminal-rpg-web.service" "$repo_root/deploy/cloudflared-rpg.service" "$repo_root/deploy/cloudflared-rpg.yml" "$stage/"
  else
    cp "$binary" "$stage/terminal-rpg"
    cp "$repo_root/deploy/terminal-rpg-web.service" "$repo_root/deploy/cloudflared-rpg.service" "$repo_root/deploy/cloudflared-rpg.yml" "$stage/"
  fi
else
  "${scp_cmd[@]}" "$binary" "$destination:$stage/terminal-rpg"
  "${scp_cmd[@]}" "$repo_root/deploy/terminal-rpg-web.service" "$repo_root/deploy/cloudflared-rpg.service" "$repo_root/deploy/cloudflared-rpg.yml" "$destination:$stage/"
fi

remote_exec "set -Eeuo pipefail
  test -s '$stage/terminal-rpg'
  chmod 0755 '$stage/terminal-rpg'
  if test -e '$remote_binary'; then cp -p '$remote_binary' '$backup'; fi
  install -m 0755 '$stage/terminal-rpg' '$remote_binary'
  install -m 0644 '$stage/terminal-rpg-web.service' /etc/systemd/system/terminal-rpg-web.service
  install -m 0644 '$stage/cloudflared-rpg.service' /etc/systemd/system/cloudflared-rpg.service
  install -m 0644 '$stage/cloudflared-rpg.yml' '$config_dir/config.yml'
  systemctl daemon-reload
  systemctl restart '$service'
  sleep 1
  systemctl is-active --quiet '$service'
  command -v curl >/dev/null && curl --fail --silent --show-error --max-time 5 '$health_url' >/dev/null
  rm -rf '$stage'
  printf 'redeploy healthy: %s\n' '$service'" || {
    log "health check failed; restoring previous binary when available"
    remote_exec "if test -e '$backup'; then install -m 0755 '$backup' '$remote_binary'; systemctl restart '$service'; fi" || true
    die "remote deployment failed; previous binary restoration was attempted"
  }
log "deployment healthy"
