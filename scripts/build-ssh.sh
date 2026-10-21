#!/usr/bin/env bash
set -euo pipefail
codex_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$codex_root/codex-rs"
lock_backup="$(mktemp)"
cp Cargo.lock "$lock_backup"
trap 'cp "$lock_backup" Cargo.lock; rm -f "$lock_backup"' EXIT
export CODEX_REPO_ROOT="$codex_root"
python3 "$codex_root/scripts/build_ssh.py"
