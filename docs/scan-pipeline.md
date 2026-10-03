# MossyMesh scan pipeline

Four layers, each with a different speed and depth. Fast layers run on
every change; slow layers run nightly. Nothing here is meant to be run
by hand except the pre-commit hooks.

## The layers

| Layer | What | When | Fails the build? |
| --- | --- | --- | --- |
| 1. Fast path | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo-deny check`, pre-commit hooks | Every PR, every push (`ci.yml`, `deny.yml`) | Yes |
| 2. Pattern scan | Semgrep custom rules, differential vs PR base | Every PR (`semgrep.yml`); full scan on push is informational | Only new findings |
| 3. Scan graph + slop | tree-sitter index of the repo, heuristic queries for AI-slop markers | Nightly (`nightly-deep-scan.yml`), artifacts kept 30 days | No, produces a report |
| 4. Deep scan | CodeQL over the Rust workspace | Nightly (`nightly-deep-scan.yml`) | Via code scanning alerts |

## Layer 1: fast path

- `.pre-commit-config.yaml`: trailing whitespace, EOF fixer, YAML/TOML
  checks, merge-conflict markers, large files, plus `cargo fmt`.
  Install with `pip install pre-commit && pre-commit install`.
- `ci.yml` already ran `cargo fmt --check` and `cargo clippy
  --workspace --all-targets -- -D warnings` before this pipeline; they
  are unchanged.
- `Cargo.toml` now has a `[workspace.lints]` table and `clippy.toml`
  tunes thresholds. The aggressive lints (unwrap_used, expect_used,
  panic, redundant_clone, needless_pass_by_value, uninlined_format_args,
  missing_errors_doc, missing_panics_doc, missing_debug_implementations)
  are staged at `allow` on purpose: CI denies warnings, so promoting any
  of them to `warn` would fail CI on existing code. Promotion path: run
  `cargo clippy` locally, fix the findings, flip the level in the table.
  The `sandbox` crate keeps its own `[lints.rust]` table (cargo forbids
  mixing it with workspace inheritance) and does not inherit.
- `deny.toml` plus the `cargo-deny` workflow: RustSec advisories,
  license allowlist, duplicate-version and wildcard bans. Runs on push,
  PR, and weekly on a schedule so advisories stay fresh.

## Layer 2: Semgrep

Six custom rules in `semgrep/rules/`, each targeting a failure mode this
codebase actually has: `panic!` in library code, `unwrap()`/`expect()`
outside tests, `unsafe` blocks needing review, `std::thread::sleep`
stalling the async executor, hardcoded secrets, `todo!`/`unimplemented!`
in shipped code. Test code is excluded by path and by `#[cfg(test)]`
module matching.

The PR job runs differential: `--baseline-commit <base-sha> --error`
reports only findings the PR introduces and fails the build when any
exist. The existing backlog (33 unwrap/expect sites, 2 unsafe blocks, 1
blocking sleep as of 2026-10-03) is grandfathered, not ignored: it shows
up in the full scan on push and in SARIF. Results upload as SARIF so
findings annotate the PR diff.

Rule discipline: the set stays small on purpose. Thirty rules developers
trust beat three hundred they suppress. If a rule fires where it should
not, fix the rule.

## Layer 3: scan graph and slop queries

`devops/scan-graph/index.py` builds a JSON graph of the repo with
tree-sitter (Rust and TypeScript): entities (files, functions, structs,
enums, traits, impls, classes, modules) and edges (use/import, calls
with best-effort name resolution). `devops/scan-graph/slop.py` queries
that graph for AI-slop markers:

- dead/unreferenced functions (medium confidence)
- single-call wrapper/passthrough functions (medium-high)
- no-op deprecations (high)
- oversized test files over 500 lines (high, plain metric)
- possibly-unused dependencies from Cargo.toml (low, heuristic)

Every finding carries a confidence label. Low-confidence items are
leads, not verdicts. A real sample report from 2026-10-03 is checked in
at `devops/scan-graph/sample-report.md` so the format is visible; nightly
runs write fresh reports to `devops/scan-graph/output/` and upload them
as artifacts.

## Layer 4: CodeQL

Nightly CodeQL analysis over the Rust workspace via the standard
init/autobuild/analyze actions. This is the layer for taint tracking and
cross-crate data-flow questions the pattern rules cannot express.

## Triage

1. New findings fail the PR. Fix them in the PR or suppress with a
   comment explaining why the rule is wrong (then fix the rule).
2. Nightly reports are triaged by the repo maintainers. The queue has
   one owner at a time; an unowned queue rots. Decide who that is and
   write the name here.
3. False positives get fixed at the rule or query level, never by
   ignoring the report. A silenced scanner is worse than none.

## What was verified where

- Locally (2026-10-03): Semgrep rules run against the real repo (36
  findings, spot-checked true positives); differential mode verified
  with `--baseline-commit` (new finding fails, clean tree passes);
  `cargo metadata` and `cargo fmt --check` pass with the new lint tables.
- In CI: clippy with the new tables, cargo-deny, the Semgrep workflow,
  the scan-graph build, and CodeQL. The scan graph and slop queries were
  run locally during development; the nightly workflow re-runs them.
