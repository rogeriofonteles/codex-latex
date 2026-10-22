# Inline LaTeX for Codex in Alacritty

This local Codex CLI fork renders equations in assistant messages as sharp MathJax images, positioned directly among the chat paragraphs. It uses your normal Codex account and saved conversations. The ordinary `codex` command remains available.

## Use

```sh
codex-latex
codex-latex resume
codex-latex --plain resume  # ordinary Codex, no graphics or polling
codex-latex --window resume  # optional separate Alacritty window
```

Resume the conversation where this was built:

```sh
codex-latex resume 01a091c5-6797-72e0-93f7-5686f2e53b38
```

Run that after the existing turn finishes. Avoid sending turns into the same conversation from two windows at once. Inside **Zellij**, the launcher runs Codex in the **current pane**. If the current terminal has no usable X11 window (including native Wayland terminals), or you are outside Zellij, it stays in the current terminal without graphical formulas. Use `--window` as the first argument to explicitly request a separate window with graphical formulas.

For formulas inside Zellij on a Wayland desktop, start Alacritty with `env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET alacritty`, then create a **new** Zellij session and run `codex-latex resume`. Existing sessions retain their original window environment; attaching them to a different window does not fix it. This machine’s local Alacritty desktop entry uses that XWayland launch command automatically.

Use `codex-latex --demo` for the offline equation display demo. The earlier window with directory `/tmp/codex-latex-offline` was an offline sample, not your actual conversation.

Supported delimiters: `\[...\]`, `$$...$$`, and `\(...\)`. Equations occupy their own rows, including expressions written with inline delimiters. Single-dollar expressions, math inside code fences, and literal code remain text. MathJax supports common LaTeX mathematics, including fractions, roots, integrals, AMS environments and matrices; this is not a full LaTeX document compiler.

## Requirements and limits

Tested with Linux X11, Alacritty 0.13.2, Zellij 0.44.3 and Codex 0.163.0. Node.js 22+ (this installation uses your existing Node 24), Python 3.12, and the repository's Rust toolchain are required to rebuild. Python and JavaScript dependencies are pinned in this directory.

- Zellij mode requires the JSON `list-panes`/`list-tabs` and targeted `dump-screen` APIs available in the tested 0.44.3 build. It uses the local X11 window identified by `WINDOWID`; use one attached Alacritty client. Multiple attached clients, reattachment to a different window, nested multiplexers, tmux and SSH are not supported. Native Wayland windows cannot host overlays; the launcher stays in the current terminal without graphical formulas. A separate X11/XWayland window requires `--window`.
- Both modes preserve your existing foreground and background. Startup reads terminal colors using OSC 10/11 queries without changing them; Zellij pane-specific colors are respected as well. Formulas are transparent masks composited with those colors. Zellij colors are never set or reset by the launcher. Restart after changing the inherited terminal theme; explicit Zellij pane color changes are picked up automatically. Zero terminal padding remains the supported geometry.
- In Zellij, the helper reads the pane viewport, so it follows pane positions and Zellij scrollback. It hides images when the tab or pane is hidden, or floating panes obscure a tiled pane. Viewport reads run on a background thread with a 250 ms pause between polls, with pane/tab metadata refreshed after 500 ms. Query time adds to those intervals; each query allows up to three seconds for busy sessions. Polling starts only after a formula appears; typing and terminal output never wait for these CLI queries. In separate-window mode, Alacritty scrollback is disabled; use Codex's **Ctrl+T transcript view** for history. Alacritty's own scrollback is not tracked.
- Images are click-through and track terminal rows. Selection may expose internal position markers; use Codex's source-copy/export features when you need original LaTeX.
- Invalid, overly large, or timed-out formulas fall back to text. Try resizing again or resuming if a temporary failure was cached. A persistent local worker renders formulas; requests time out after 250 ms instead of waiting several seconds. Successful renders are cached. This remains an experimental local integration.
- The helper changes presentation only. Original messages and model context retain their LaTeX. Rendering happens locally; no external rendering service or TeX shell execution is used. Temporary images live in a private session directory and are removed on normal exit.

## Rebuild this version

```sh
cd ~/Projects/codex-latex
./latex-preview/install.sh
```

The script installs pinned dependencies in local directories, builds Codex, reuses the unmodified code-mode helper from a matching official installation (or builds it if no matching installation exists), copies the binaries into `latex-preview/runtime/bin`, and installs `~/.local/bin/codex-latex`. It does not download a newer Codex revision. It preserves the previous binaries as `*.previous` when rebuilding. Keep this project directory: the launcher refers to it.

The installer selects a suitable `node` from PATH or `~/.nvm/versions/node`. After removing or upgrading your Node installation, rerun the installer to refresh its local symlink. Builds use four jobs by default; set `CARGO_BUILD_JOBS=2` if memory is tight. The build cache can occupy tens of gigabytes. Once installed, it can be cleaned without removing the copied runtime binaries, but the next rebuild will take longer.

## Update to a newer Codex release

The stock `codex` updater does **not** update this fork. Keep the working version until a new one is verified.

1. Confirm `git status --short` is clean and make a backup branch, for example `git branch backup/latex-working` (choose a fresh name on later updates).
2. Ensure `upstream` points to `https://github.com/openai/codex.git` (add it with `git remote add upstream https://github.com/openai/codex.git` if needed). Prefer a release matching your installed official `codex --version`, so its code-mode helper can be reused. Choose an explicit upstream release tag from `https://github.com/openai/codex/releases` and fetch it: `git fetch upstream tag rust-vVERSION`.
3. Replay the local commits on that release:

   ```sh
   git rebase --onto rust-vVERSION "$(cat latex-preview/UPSTREAM_BASE)"
   ```

   Replace `VERSION` with the actual version. Resolve any conflicts in `codex-rs/tui/src/markdown.rs` and `lib.rs`; these connect the separate `latex_render.rs` module to Codex. If the renderer has changed substantially, stop and adapt the integration. `git rebase --abort` returns to the previous source.

4. Record the new base and commit it:

   ```sh
   git rev-parse 'rust-vVERSION^{}' > latex-preview/UPSTREAM_BASE
   git add latex-preview/UPSTREAM_BASE
   git commit -m 'Record updated Codex base'
   ```

5. Run the focused checks below and the TUI suite; compare unrelated upstream failures to an unmodified build. Then run `./latex-preview/install.sh`.
6. Run `codex-latex --demo`, then resume a real conversation. Check streaming, Ctrl+T history, window resizing, and closing the window before adopting the update.

The binaries are local development builds; do not use Codex's self-update command to update them. Change the source revision and rebuild instead.

## Checks

With `~/.cargo/bin` and the selected Node on PATH:

```sh
./latex-preview/.venv/bin/python -m unittest discover -s latex-preview -v
env -u CODEX_LATEX_HELPER -u CODEX_LATEX_CACHE -u CODEX_LATEX_ZELLIJ \
  just test -p codex-tui --cargo-profile dev-small -E 'test(latex_render)' -j 4
env -u CODEX_LATEX_HELPER -u CODEX_LATEX_CACHE -u CODEX_LATEX_ZELLIJ \
  just test -p codex-tui --cargo-profile dev-small -j 4
```

The Rust test commands clear the live image-helper environment so upstream text-rendering assertions do not use a running LaTeX session.

Follow the root `AGENTS.md` for formatting and Rust lint checks. See `VALIDATION.md` for this build's results and known upstream test failures. Visual evidence is in `artifacts/codex-chat.png`.

## Roll back or remove

Immediate fallback: exit the custom Codex session and use `codex-latex --plain resume` (or ordinary `codex resume`). Plain mode bypasses both the custom binary and image bridge and stays in the current pane. No conversation conversion is needed.

To restore the previous local binaries after an upgrade:

```sh
cd ~/Projects/codex-latex
cp latex-preview/runtime/bin/codex.previous latex-preview/runtime/bin/codex
cp latex-preview/runtime/bin/codex-code-mode-host.previous latex-preview/runtime/bin/codex-code-mode-host
```

To uninstall the launcher, remove only `~/.local/bin/codex-latex`. The project can then be archived or deleted. Do not remove `~/.codex`: that holds your normal settings and conversations.

## Files to maintain

`latex_render.rs` recognizes math and reserves chat rows; the small changes in `markdown.rs` apply it to streamed and restored assistant messages. `render.cjs` runs as a persistent local socket worker and produces transparent PNG masks; `terminal_colors.py` reads the existing terminal palette without altering it. `bridge.py` tracks terminal rows and draws X11 image strips. `zellij_view.py` supplies pane geometry, visibility and viewport contents in Zellij mode. `codex-latex` selects the current Zellij pane or a dedicated window. `install.sh` rebuilds and installs the chosen revision. Dependency versions belong in `package.json`, `package-lock.json`, and `requirements.txt`; do not apply automatic major-version upgrades without testing.

## Conversation branches

Use `/branch [prompt]` to create and open a saved child conversation from the
current chat. For example, `/branch Explain the cache invalidation` starts that
request immediately in the child and generates its title from the prompt. The
generated title updates the Zellij pane name while preserving its status badge.
Bare `/branch` opens a child ready for your next message. The child inherits the
history up to that point and continues independently in the same working
directory. You can branch again inside a child, or return to its parent and
create additional sibling conversations.
Use `/rename` to change the current conversation's title.

Use `/sibling [prompt]` to create a peer conversation at the same depth. From a
root chat, it creates another root in the same forest. From a child, it creates
another child of the same parent. An optional prompt starts immediately and
generates the new chat's title, just like `/branch`.

Launch this build with `codex-latex --no-daemon` to use live sibling context.
The command requires the matching local backend; it refuses older or remote
background servers rather than creating a group whose context they cannot share.
Resume saved group members in the same updated build when continuing them.

Sibling context is live: before each new turn, the model receives the completed
turns from the other chats in that sibling group alongside its own history.
In-progress turns and descendants are excluded. The shared material is not
stored in the receiving chat's history or copied into its children. For example,
root chats A and B share context, but a child of B inherits B's own history and
does not receive A's history. Creating a sibling for that child enables sharing
among B's children while keeping their context separate from the root group.
Sibling membership survives restarts, and `/tree` shows linked root chats as a
forest with each root's own descendants.

Use `/tree` to browse the current chat's family of saved, non-archived
conversations. From any child, it shows the root parent and all sibling and nested
branches, including linked sibling roots and their descendants. Unrelated chats
are hidden. The current chat is marked with
`← current` and selected automatically when the tree opens:

- **Up/Down:** select a conversation.
- **Left/Right:** collapse or expand branches; move to a parent or child.
- **Enter:** open the selected conversation and continue chatting.
- **Esc:** return without switching.

The tree marks the current conversation and includes existing `/fork` ancestry.
Branches remain available after restarting codex-latex. Conversations whose
parent was archived or deleted appear as roots; archived chats remain accessible
through `/resume`. Branches are separate saved chats, so they are not limited to
the single temporary `/side` conversation.
