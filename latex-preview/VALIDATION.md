# Validation of the local 0.163.0 integration

## Upgrade from 0.162.0 (2026-10-16)

Replayed all eleven local commits onto `rust-v0.163.0` without conflicts and restored the pre-existing uncommitted LaTeX and conversation-tree changes. Adapted the tree's `ThreadListParams` initializer to the new `excluded_thread_ids` field. The previous source is preserved on `backup/latex-0.162.0-20261016`; the named Git stash, local-change patch, and original untracked conversation-tree source are retained in the ignored build logs.

- All 20 Python helper tests and all 58 selected Rust tests passed, including LaTeX rendering, conversation-tree navigation, and `/branch` and `/tree` dispatch.
- The full TUI suite ran 5,801 tests: 5,755 passed, 45 failed, and one timed out; eight tests were skipped. The 45 failure names and IDE IPC timeout match those recorded during the previous 0.162.0 upgrade. Most generated snapshot differences are release-version text and wrapping; the remaining differences include daemon version indicators and terminal cursor escapes. This update did not repeat the unmodified-upstream comparison. No upstream snapshots were accepted; generated snapshots are archived in the ignored build logs.
- Repository formatting and formatting checks passed, as did shell syntax and whitespace checks. Pre-existing Python edits received formatting only. Build-generated local-package changes in `Cargo.lock` were restored.
- The installer completed successfully. The installed CLI reports `codex-cli 0.163.0` and matches the build output. Its code-mode helper and all 39 bundled resource files match the official 0.163.0 package. Previous 0.162.0 binaries remain in `runtime/bin/*.previous`; the previous bundled resources are also retained.
- Interactive streaming, transcript navigation, resizing, and visual appearance were not verified during this upgrade.
- Logs are stored in the ignored `build-logs/upgrade-0.163.0-*` files.

# Previous 0.162.0 validation

## Upgrade from 0.158.0 (2026-10-09)

Replayed the ten local commits onto `rust-v0.162.0`, matching the installed official CLI. Resolved one Markdown adapter conflict by applying LaTeX rewriting before upstream's table-aware fence normalization while retaining its code-source tracking. The previous source is preserved on `backup/latex-0.158.0-20261009`. The five pre-existing edited files were restored byte-for-byte; their named Git stash and `build-logs/upgrade-0.162.0-local-changes.patch` remain available.

- All 18 Python helper tests and all eight Rust LaTeX tests passed.
- With the live LaTeX environment cleared, the full TUI suite ran 5,718 tests: 5,672 passed, 45 failed, and one timed out; eight were skipped. All 46 unsuccessful cases reproduced against the unmodified upstream TUI source in the same checkout and environment. The unsuccessful test-name sets match exactly. No upstream snapshots were accepted; generated snapshots are archived in the ignored build logs.
- Scoped `just fix -p codex-tui --profile dev-small -j 4` and repository `just fmt` passed. The new formatter required `uv`, installed in the local helper virtual environment. Build-generated package-version changes in `Cargo.lock` were restored. Shell syntax checks passed and bundled resources match the official package.
- The installer completed successfully. The installed CLI reports `codex-cli 0.162.0` and matches the build output. Its code-mode helper matches the official 0.162.0 package. Previous 0.158.0 binaries remain as `runtime/bin/*.previous`; the installer also preserved the previous bundled resources.
- Interactive streaming, transcript navigation, resizing, and visual appearance were not verified during this upgrade. The window demo command returned without producing its ready marker, so it is not counted as a successful visual smoke test.
- Logs and JUnit results are stored in the ignored `build-logs/upgrade-0.162.0-*` files.

## Previous 0.158.0 validation

## Upgrade from 0.155.1 (2026-09-28)

Replayed the nine local commits onto `rust-v0.158.0`. Resolved the two Markdown adapter conflicts by applying LaTeX rewriting before the new upstream `normalize_markdown_for_rendering` function. The previous source is preserved on `backup/latex-0.155.1-20260928`; the five pre-existing uncommitted resize-fix files were restored unchanged. Their backup remains in the named pre-upgrade Git stash and `build-logs/upgrade-0.158.0-local-changes.patch`.

- All 18 Python helper tests and eight focused Rust LaTeX tests passed.
- The full TUI suite ran 5,558 tests: 5,507 passed, 50 failed, and one timed out; eight tests were skipped. Running the 51 unsuccessful cases against the unmodified upstream TUI reproduced 48 failures and the IDE IPC timeout. The other two cases inherited this live session's image-helper environment and received image markers instead of the plain text expected by upstream assertions.
- After clearing `CODEX_LATEX_HELPER`, `CODEX_LATEX_CACHE`, and `CODEX_LATEX_ZELLIJ`, all 436 Markdown, streaming, and LaTeX tests passed, including both environment-sensitive cases. The README now documents this test isolation. No upstream snapshots were accepted or changed; generated snapshots are archived under the ignored build logs.
- Scoped `just fix -p codex-tui --profile dev-small -j 4` and repository `just fmt` completed successfully. The release build's automatic local-package version changes in `Cargo.lock` were restored.
- The offline helper demo exited successfully. Interactive streaming, transcript navigation, resizing, and a visual check of the new CLI were not repeated during this update.
- The installed CLI reports `codex-cli 0.158.0` and matches the build output; its code-mode helper and all bundled resource files match the official 0.158.0 package. The previous CLI reports `codex-cli 0.155.1` and remains in `runtime/bin/codex.previous`, alongside its previous helper. The dependency install reported zero npm vulnerabilities.
- The initial install encountered read-only files when overwriting bundled resources. The installer now stages a complete fresh resource directory before swapping it into place; the repaired resource step, shell syntax check, and final formatting completed successfully. The original resource directory remains in `runtime/.codex-resources.*.previous`.
- Logs and JUnit results are stored in the ignored `build-logs/upgrade-0.158.0-*` files.

## Previous 0.155.1 validation

## Upgrade from 0.154.0

Rebased all eight local commits onto `rust-v0.155.1` without conflicts. The previous source is preserved on `backup/latex-0.154.0-20260922`.

- All 16 Python helper tests and all six Rust LaTeX tests passed.
- The full TUI suite ran 4,589 tests: 4,545 passed, 43 failed, one timed out, and six were skipped. All 44 unsuccessful cases reproduced on the unmodified 0.155.1 TUI in the same environment; the sets of unsuccessful test names match exactly. No upstream snapshots were accepted or changed.
- Repository `just fmt` and scoped `just fix -p codex-tui --profile dev-small -j 4` completed successfully.
- The offline Alacritty demo visibly rendered the integral, matrix, and quadratic formula between prose and exited successfully. This upgrade did not repeat interactive streaming, transcript navigation, or resizing checks in a live conversation.
- The installer completed successfully. The installed CLI reports `codex-cli 0.155.1` and is byte-identical to the build output; its code-mode helper is byte-identical to the official 0.155.1 package. The previous CLI reports `codex-cli 0.154.0` and remains available as `runtime/bin/codex.previous`, alongside the previous helper. The npm install audit reported zero vulnerabilities.
- Logs, JUnit results, upstream comparison, and the demo screenshot are stored in the ignored `build-logs/upgrade-0.155.1-*` files.

## Previous 0.154.0 validation


## Zellij query timeout correction

Live measurements in the user's session found `list-panes --json --all` taking 1.216–1.804 seconds, while tab and viewport queries generally took 13–18 ms. The 400 ms subprocess timeout introduced with background polling repeatedly discarded the visible state and removed formula windows. Raised the query timeout to three seconds; polling remains on the background worker. Sixteen helper tests pass, including a real subprocess taking 600 ms through the polling path. This addresses an observed query failure; it does not establish that every earlier visual artifact had the same cause. The periodic X11 repaint experiment remains reverted.

## Responsiveness fix

Removed synchronous Zellij subprocess queries from the input-forwarding loop and stopped rebuilding a complete pyte screen for every viewport poll. No ongoing polling starts before a formula is displayed. Unchanged image frames skip X11 work; unchanged cell geometry is not rewritten to disk. Replaced per-formula Node subprocesses and eight-second UI waits with a persistent local worker and 250 ms bounded IPC.

Measured locally: 12 uncached formulas, first/max 32.95 ms and median 9.55 ms. A 180×87 viewport processing microbenchmark improved from a 16.85 ms median to 0.07 ms. These are component measurements, not an end-to-end typing latency guarantee. Fifteen helper tests and six focused Rust tests pass, including slow-polling nonblocking behavior, no pre-math polling, worker IPC, and stalled-worker fallback. `--plain` bypasses the integration entirely. Repository formatting and scoped Clippy completed successfully. The installer rebuilt and installed the CLI; the installed binary matches the build output, and both custom and plain CLI version checks pass.

## Inherited colors

The launcher no longer sets pane or Alacritty colors. A live Zellij test showed `default_fg` and `default_bg` both null, while the formula background matched the terminal’s `#181818` (`artifacts/zellij-inherited-colors.png`). Twelve helper tests pass, including terminal RGB reply parsing, preserving typed input, and transparent PNG margins.

## Zellij pane mode

The launcher was tested inside a temporary pane in the existing Zellij 0.44.3 session, without opening an Alacritty window. Integrals, matrices and fractions appeared at the correct pane offset (`artifacts/zellij-pane.png`). The temporary pane closed after the demo and all 15 image strips were removed. All ten helper tests pass, including tab visibility, fullscreen/suppression, floating-pane occlusion and terminal/plugin ID disambiguation. No Rust renderer or installed Codex binary changed for this addition.

## Original renderer

- The user confirmed that equations displayed correctly in the new Alacritty window.
- Visually checked MathJax integrals, matrices and fractions between normal paragraphs in both the offline helper demo and a real Codex CLI rendering an offline saved conversation. See `artifacts/codex-chat.png` and `artifacts/inline-demo.png`.
- Verified X11 image windows have empty input regions, so mouse events pass through to Codex.
- Six Python checks pass: terminal scrolling/clearing, alternate-screen restoration, wide-Unicode coordinates, PNG generation/cache reuse, matrices, and invalid/oversized/disallowed input.
- Four focused Codex LaTeX tests pass, including the reviewed snapshot for the reserved formula rows.
- Repository formatting and the scoped `just fix -p codex-tui --profile dev-small` Clippy pass completed successfully.
- The full TUI suite ran 4,313 tests: 4,273 passed and 40 failed. One failure was the newly generated LaTeX snapshot, subsequently reviewed, accepted, and passed in the focused run. All other **39 failures were reproduced on the unmodified upstream source** with the same environment. They concern existing status/update/startup/cursor snapshots and IDE IPC tests, including a timeout. They have not been changed as part of this patch.
- The npm dependency audit reported zero vulnerabilities for the pinned dependency tree when checked.

The initial implementation used per-formula subprocesses with a timeout eventually increased to eight seconds. That design caused responsiveness problems and has been replaced by the persistent worker described above. Streamed and restored messages share the adapter; original message data is not rewritten.

The code-mode helper's upstream V8 archive returned HTTP 404 during a source build. The installer therefore reuses the unchanged helper from the matching official Codex 0.154.0 package. It checks version equality before reuse, and can also reuse an already installed matching local helper.

Detailed logs and JUnit results are kept locally in the ignored `build-logs/` directory. The broad suite is not being claimed as fully passing. This is an experimental X11 integration with the limits described in `README.md`.
