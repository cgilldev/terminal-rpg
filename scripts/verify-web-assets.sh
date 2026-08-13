#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root/web/vendor"
sha256sum --check manifest.sha256
grep -Fq "MIT" xterm-6.0.0.LICENSE
grep -Fq "MIT" addon-fit-0.11.0.LICENSE
echo "Browser asset verification passed"
