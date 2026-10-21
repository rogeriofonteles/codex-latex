import os
import subprocess
from pathlib import Path

from codex_package.targets import TARGET_SPECS
from codex_package.v8 import resolve_codex_v8_cargo_env


def main() -> None:
    compiler = subprocess.check_output(["rustc", "-vV"], text=True)
    host = next(
        line.removeprefix("host: ")
        for line in compiler.splitlines()
        if line.startswith("host: ")
    )
    spec = TARGET_SPECS[host]
    environment = dict(os.environ)
    if not spec.is_windows:
        environment.update(resolve_codex_v8_cargo_env(spec))
    cargo = os.environ.get("CARGO", str(Path.home() / ".cargo/bin/cargo"))
    subprocess.run(
        [
            cargo,
            "build",
            "--profile",
            "dev-small",
            "-j",
            os.environ.get("CARGO_BUILD_JOBS", "4"),
            "-p",
            "codex-cli",
            "-p",
            "codex-code-mode-host",
        ],
        env=environment,
        check=True,
    )


if __name__ == "__main__":
    main()
