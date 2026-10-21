#!/usr/bin/env bash
set -euo pipefail
codex_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$codex_root/codex-rs"
"${CARGO:-$HOME/.cargo/bin/cargo}" build --profile dev-small -p codex-cli --bin codex
