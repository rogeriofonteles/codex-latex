#!/usr/bin/env bash
# Rebuild the checked-out revision; never fetch or rebase implicitly.
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_dir"
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p latex-preview/runtime/bin
python3 - <<'PY'
import os, shutil, subprocess
from pathlib import Path
candidates = [shutil.which('node')]
candidates += sorted(Path.home().glob('.nvm/versions/node/*/bin/node'), reverse=True)
for candidate in candidates:
    if not candidate:
        continue
    try:
        major = int(subprocess.check_output([str(candidate), '-p', 'process.versions.node.split(".")[0]'], text=True))
        if major >= 22:
            resolved = Path(candidate).resolve()
            target = Path('latex-preview/runtime/bin/node')
            target.unlink(missing_ok=True)
            target.symlink_to(resolved)
            break
    except (OSError, ValueError, subprocess.CalledProcessError):
        continue
else:
    raise SystemExit('Install Node.js 22 or newer, then run install.sh again.')
PY
export PATH="$project_dir/latex-preview/runtime/bin:$PATH"
python3 -m venv latex-preview/.venv
latex-preview/.venv/bin/pip install -r latex-preview/requirements.txt
npm ci --prefix latex-preview --include=optional --ignore-scripts
python3 - <<'BUILD'
import os, subprocess
from pathlib import Path
lock = Path('codex-rs/Cargo.lock')
original = lock.read_bytes()
try:
    build = ['cargo', 'build', '--profile', 'dev-small', '-j', os.environ.get('CARGO_BUILD_JOBS', '4')]
    subprocess.run(build + ['-p', 'codex-cli'], cwd='codex-rs', check=True)
    stock = Path.home() / '.codex/packages/standalone/current/bin'
    cli = Path('codex-rs/target/dev-small/codex')
    stock_host = stock / 'codex-code-mode-host'
    same_version = (stock_host.is_file() and (stock / 'codex').is_file()
                    and subprocess.check_output([str(stock / 'codex'), '--version'])
                    == subprocess.check_output([str(cli), '--version']))
    installed = Path('latex-preview/runtime/bin')
    reusable = ((installed / 'codex-code-mode-host').is_file() and (installed / 'codex').is_file()
                and subprocess.check_output([str(installed / 'codex'), '--version'])
                == subprocess.check_output([str(cli), '--version']))
    if same_version:
        host = stock_host
    elif reusable:
        host = (installed / 'codex-code-mode-host').resolve()
    else:
        release_host = Path('latex-preview/runtime/bin/codex-code-mode-host.release')
        if release_host.is_file():
            host = release_host.resolve()
        else:
            subprocess.run(build + ['-p', 'codex-code-mode-host'], cwd='codex-rs', check=True)
            host = Path('codex-rs/target/dev-small/codex-code-mode-host').resolve()
    Path('latex-preview/runtime/host-source').write_text(str(host))
finally:
    # Release tags normalize local package versions during a build. Keep the source tree clean.
    lock.write_bytes(original)
BUILD
for program in codex codex-code-mode-host; do
  destination="latex-preview/runtime/bin/$program"
  if [[ -f "$destination" ]]; then cp -- "$destination" "$destination.previous"; fi
  source="codex-rs/target/dev-small/$program"
  if [[ "$program" == codex-code-mode-host ]]; then source="$(cat latex-preview/runtime/host-source)"; fi
  install -m 755 "$source" "$destination.new"
  mv -- "$destination.new" "$destination"
done
# Preserve the bundled sandbox and shell helpers from the stock installation.
stock_resources="$HOME/.codex/packages/standalone/current/codex-resources"
if [[ -d "$stock_resources" ]]; then
  resources="latex-preview/runtime/codex-resources"
  staged_resources="$(mktemp -d latex-preview/runtime/.codex-resources.XXXXXX)"
  cp -aL "$stock_resources/." "$staged_resources/"
  if [[ -e "$resources" ]]; then
    mv -- "$resources" "$staged_resources.previous"
  fi
  mv -- "$staged_resources" "$resources"
fi
mkdir -p "$HOME/.local/bin"
launcher="$HOME/.local/bin/codex-latex"
if [[ -e "$launcher" || -L "$launcher" ]]; then
  if [[ "$(readlink -f -- "$launcher")" != "$project_dir/latex-preview/codex-latex" ]]; then
    echo "Refusing to replace an unrelated $launcher" >&2
    exit 1
  fi
else
  ln -s "$project_dir/latex-preview/codex-latex" "$launcher"
fi
printf 'Installed codex-latex. Run: codex-latex resume\n'
