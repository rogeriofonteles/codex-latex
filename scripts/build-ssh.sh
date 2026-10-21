#!/usr/bin/env bash
set -euo pipefail
codex_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$codex_root/codex-rs"
lock_backup="$(mktemp)"
cp Cargo.lock "$lock_backup"
trap 'cp "$lock_backup" Cargo.lock; rm -f "$lock_backup"' EXIT
"${CARGO:-$HOME/.cargo/bin/cargo}" build --profile dev-small -j "${CARGO_BUILD_JOBS:-4}" -p codex-cli -p codex-code-mode-host
