# Semgrep custom rules

Small, high-trust rule set for MossyMesh (scan pipeline, phase 2).
Deliberately not the stock registry: every rule here targets a failure
mode this codebase actually has (daemon panics, network-driven input,
async executor stalls, unsafe review).

Run locally: `semgrep --config semgrep/rules`
Differential (CI): `semgrep scan --config semgrep/rules --baseline-commit <sha>`

| Rule | What it catches | Severity |
| --- | --- | --- |
| rust-no-panic-in-lib | `panic!` in library code | WARNING |
| rust-no-unwrap-expect | `unwrap()` / `expect()` outside tests | WARNING |
| rust-unsafe-block | `unsafe` blocks needing review | WARNING |
| rust-no-blocking-sleep | `std::thread::sleep` (stalls async executor) | WARNING |
| rust-hardcoded-secret | hardcoded passwords, keys, tokens, PEM blocks | ERROR |
| rust-no-todo-unimplemented | `todo!` / `unimplemented!` in shipped code | WARNING |

Test code is excluded by path. If a rule fires where it should not,
fix the rule, not the code: a rule nobody trusts gets suppressed.
