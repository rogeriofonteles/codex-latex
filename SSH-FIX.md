# Codex for slow SSH terminals

This checkout uses `https://github.com/rogeriofonteles/codex-latex.git` on
`main`, with the SSH terminal timeout fix integrated into the LaTeX fork.
The OpenAI repository is retained as the `upstream` remote. The original
0.162.1 fix and its build lockfile remain on `fix/ssh-terminal-timeout`.

The upstream Unix startup probe has a 250 ms deadline. If the cursor-position
response arrives later, startup uses `(0, 0)` instead of the actual cursor position.
That can place the input box incorrectly. Late color replies also cause the
terminal palette to use fallback colors.

The custom launcher, `bin/codex`, sets `CODEX_TERMINAL_TIMEOUT_MS=5000` unless an
override is already set. Startup and palette probes use that deadline and return
as soon as the requested replies arrive. Unsupported terminals can take up to
five seconds to finish probing. The deadline is not a reduction in network latency.

When running the compiled binary directly, it uses five seconds if `SSH_TTY`,
`SSH_CONNECTION`, or `SSH_CLIENT` is present; otherwise it keeps the upstream
250 ms default. Overrides accept integer milliseconds from 1 through 60000.
Invalid values fall back to the default for the detected connection.

For a slower connection:

```sh
CODEX_TERMINAL_TIMEOUT_MS=10000 codex
```

Build again:

```sh
~/code/codex/scripts/build-ssh.sh
```

Run the terminal probe tests, including a simulated 1.2-second response delay:

```sh
cd ~/code/codex/codex-rs
~/.cargo/bin/cargo test --profile dev-small -p codex-tui --lib terminal_probe::
```

The build uses `dev-small` to keep local rebuilds practical and builds both the
CLI and its matching code-mode helper. It uses the repository's pinned,
checksum-verified Codex V8 artifacts for the helper build and preserves the
checked-in lockfile. `~/.local/bin/codex-real` points to
`~/code/codex/bin/codex`; the Zellij wrapper and fish functions continue to use
that launcher. `codex-stock` remains available for the official installation.

This changes the terminal UI in new CLI sessions. Restart an already-running
Codex CLI to use the rebuilt version. The shared app-server daemon does not
render the terminal UI and can continue using its existing binary.

The SSH fix changes terminal response deadlines; it does not add graphical
LaTeX support over SSH. For the fork's optional graphical launcher and its
separate dependencies, follow [latex-preview/README.md](latex-preview/README.md).
