# CLAUDE.md

## How I expect you to write code

**No shortcuts. "Simple" never means "sloppy."** A small diff that hardcodes,
duplicates, or skips a test isn't simpler — it's deferred cost.

1. **Fix causes, not symptoms.** Find the root cause before fixing. If you're
   applying a workaround, say so explicitly and explain why. Never swallow an
   exception or silence an error to make a problem disappear.

2. **Think about consequences.** Before changing shared or widely-used code,
   trace its callers and the invariants they rely on. A fix that's locally
   correct but breaks something elsewhere — now or later — is not a fix.

3. **SOLID, sensibly.** One responsibility per class/widget/function. Separate
   pure logic from I/O so it can be tested. Inject dependencies that cross a
   boundary so they're mockable. Don't add abstractions for things that don't
   cross a boundary.

4. **DRY about knowledge, not appearance.** Don't duplicate a rule or decision.
   Code that merely looks similar but changes for different reasons stays
   separate. When unsure, prefer duplication over a premature/wrong abstraction.

5. **No hardcoded values.** No magic numbers or strings inline — give them
   names. Environment/tenant/feature-specific values go in typed config in
   application code, never scattered literals, never the database.

6. **Readable & maintainable.** Clear names, short flat functions, early
   returns over deep nesting. Comments explain *why*, not *what*. Match the
   existing style of the file you're editing.

7. **Testable, and prove it.** Ship a test for behavior you add or change. If
   something is hard to test, that's a design smell — restructure until it
   isn't. "Works but can't be tested" means it isn't done.

A change is done only when: the cause (not a symptom) is fixed, no new hardcoded
values, a test covers it, and the analyzer/formatter are clean.

## Project facts

> Keep these current as the repo evolves; only write what you've confirmed.

- **Setup command:** `cargo build` (install the binary with `cargo install --path crates/app`)
- **Analyze/lint command:** _TBD_
- **Test command (all):** `cargo test` (runs the whole workspace)
- **Test command (single file/test):** `cargo test -p <crate> <test_name>` (e.g. `cargo test -p edit --test benchmark`)
- **Format command:** _TBD_
- **Run an app:** `cargo run -p edit -- [paths]` (TUI); `cargo run -p edit-gui` (GUI)
- **Repo layout:** `crates/` Rust workspace (core-* libs: buffer/diff/theme/picker/fs/syntax/terminal/agent-protocol, ui-tui + ui-gui frontends, `app` = `edit` binary, `edit-agent` binary); `website/` static GitHub Pages site; `scripts/` benchmark + signing helpers; `prototypes/`; `.github/workflows/` release + pages CI
- **State management / data layer conventions:** No database; in-memory editor state per crate (e.g. `app/src/state.rs`, `ui-gui/src/state.rs`). Text is a ropey rope buffer (`core-buffer`); agent IPC uses serde/serde_json messages (`core-agent-protocol`)
- **Generated files NOT to hand-edit:** `/target` (build output), `Cargo.lock` (managed by Cargo), `.signing/` and `scripts/node_modules/` (gitignored)
- **Other gotchas worth recording:** Two binaries — `edit` (TUI, `crates/app`) and `edit-gui` (`crates/ui-gui`); release tags `v*` build/sign/notarize cross-platform binaries via `.github/workflows/release.yml`. GUI builds need system libs on Linux (libgtk-3, libxcb, libxkbcommon, etc.). Use `--benchmark <path>` or `scripts/benchmark.sh` to measure startup/RSS
