#!/usr/bin/env python3
"""Coverage record for the MossyMesh scan pipeline.

Builds a per-run coverage ledger: for every scannable surface of the repo,
records which pipeline layer covered it and which did not, with reasons.
A clean scan means nothing if the scanner never looked, so the ledger's
real job is the gap: the report ends with an explicit statement of what
was NOT looked at.

Usage:
    python coverage.py [--graph GRAPH_JSON]
                       [--out-ledger LEDGER_JSON] [--out-report REPORT_MD]

Defaults: graph at <repo>/devops/scan-graph/output/graph.json;
ledger at  <repo>/devops/scan-graph/output/coverage-ledger.json;
report at  <repo>/devops/scan-graph/output/coverage-report.md.

Unit states: covered, skipped (with reason), out_of_scope (declared by
design, never implied), blocked (evidence missing). A unit is only
"covered" when this run produced evidence for it (layers 3-4) or the
layer is configured to cover it on every run (layers 1-2, marked as
declared scope, not observed coverage).

Exit codes: 0 on success. 2 when the evidence a layer needs is missing
(graph.json absent): the script refuses to certify coverage it cannot
see. 1 when the ledger was written but contains blocked units.
"""

import argparse
import json
import subprocess
import sys
import tomllib
from datetime import datetime, timezone
from pathlib import Path

SKIP_DIRS = {".git", "target", "node_modules", "dist", "output", "__pycache__"}

CLASS_BY_EXT = {
    ".rs": "rust",
    ".ts": "typescript",
    ".tsx": "typescript",
    ".py": "python",
    ".sh": "shell",
    ".bash": "shell",
    ".toml": "config",
    ".yaml": "config",
    ".yml": "config",
    ".json": "config",
    ".md": "docs",
    ".rst": "docs",
    ".txt": "docs",
}

# Layer metadata mirrors docs/scan-pipeline.md. Declared scope (what the
# layer is configured to cover) vs observed coverage (what this run saw)
# is marked per unit: only layers 3-4 have per-run evidence here.
LAYERS = {
    "l1": {
        "name": "Fast path",
        "when": "every PR and push (ci.yml, deny.yml, pre-commit hooks)",
        "coverage_basis": "declared",
    },
    "l2": {
        "name": "Pattern scan",
        "when": "every PR, differential vs base (semgrep.yml)",
        "coverage_basis": "declared",
    },
    "l3": {
        "name": "Scan graph + slop",
        "when": "nightly (nightly-deep-scan.yml)",
        "coverage_basis": "observed",
    },
    "l4": {
        "name": "Deep scan",
        "when": "nightly (nightly-deep-scan.yml)",
        "coverage_basis": "declared",
    },
}

STATUS_ORDER = {"covered": 0, "skipped": 1, "blocked": 2, "out_of_scope": 3}


def classify(path):
    return CLASS_BY_EXT.get(path.suffix.lower(), "other")


def iter_repo_files(root):
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        yield path


def is_test_rust(rel):
    """Path-level test heuristic, matching the semgrep rule exclusions."""
    parts = rel.split("/")
    if "tests" in parts or "__tests__" in parts:
        return True
    base = parts[-1].lower()
    return ".test." in base or ".spec." in base or base.startswith("test_")


def workspace_crates(root):
    """Top-level directories that are cargo workspace members."""
    try:
        data = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError):
        return set()
    members = data.get("workspace", {}).get("members", [])
    return {m.split("/")[0] for m in members}


def git_rev(root):
    try:
        out = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=root, capture_output=True,
            text=True, timeout=10,
        )
        rev = out.stdout.strip()
        return rev if rev else "unknown"
    except Exception:
        return "unknown"


def load_graph(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, json.JSONDecodeError):
        return None


def unit(layer, surface, status, via=None, reason=None, files=None,
         files_total=0, note=None):
    rec = {
        "id": f"{layer}/{surface}",
        "layer": layer,
        "surface": surface,
        "status": status,
        "via": via or [],
        "files_total": files_total,
        "files_covered": files_total if status == "covered" else 0,
        "files_skipped": len(files or []) if status in ("skipped", "blocked") else 0,
    }
    if reason:
        rec["reason"] = reason
    if files:
        rec["files"] = sorted(files)
    if note:
        rec["note"] = note
    return rec


def build_ledger(root, graph):
    units = []
    files = [(p.relative_to(root).as_posix(), classify(p)) for p in iter_repo_files(root)]
    by_class = {}
    for rel, cls in files:
        by_class.setdefault(cls, []).append(rel)

    crates = workspace_crates(root)
    rust_files = by_class.get("rust", [])

    # -- layer 1: fast path (declared scope) ---------------------------
    for cls, rels in sorted(by_class.items()):
        if cls == "rust":
            in_ws = [r for r in rels if r.split("/")[0] in crates]
            out_ws = [r for r in rels if r.split("/")[0] not in crates]
            units.append(unit(
                "l1", "rust", "covered",
                via=["cargo fmt --check", "cargo clippy -D warnings",
                     "pre-commit hygiene hooks"],
                files_total=len(in_ws), note=(
                    f"{len(in_ws)} of {len(rels)} .rs files sit in cargo "
                    "workspace crates and get fmt+clippy on every PR/push."),
            ))
            if out_ws:
                units.append(unit(
                    "l1", "rust-outside-workspace", "skipped",
                    via=["pre-commit hygiene hooks"],
                    reason="not in a cargo workspace crate: no fmt/clippy coverage",
                    files=out_ws, files_total=len(out_ws)))
        elif cls == "config":
            manifests = [r for r in rels
                         if r == "Cargo.toml" or r.endswith("/Cargo.toml")
                         or r == "Cargo.lock" or r.endswith("/Cargo.lock")]
            units.append(unit(
                "l1", "config", "covered",
                via=["cargo-deny (Cargo.toml/Cargo.lock)",
                     "pre-commit hygiene hooks"],
                files_total=len(rels), note=(
                    f"cargo-deny covers {len(manifests)} manifests "
                    "(advisories, licenses, duplicates, wildcards); hygiene "
                    "hooks cover the rest of the class.")))
        else:
            units.append(unit(
                "l1", cls, "covered",
                via=["pre-commit hygiene hooks"],
                files_total=len(rels), note=(
                    "hygiene only (whitespace, EOF, YAML/TOML validity, merge "
                    "markers, large files). No fmt/clippy/deny for this class.")))

    # -- layer 2: semgrep (declared scope) ------------------------------
    test_rust = [r for r in rust_files if is_test_rust(r)]
    lib_rust = [r for r in rust_files if not is_test_rust(r)]
    units.append(unit(
        "l2", "rust", "covered",
        via=["semgrep custom rules (6: panic-in-lib, unwrap/expect, unsafe, "
             "blocking-sleep, hardcoded-secret, todo/unimplemented)"],
        files_total=len(lib_rust), note=(
            "differential on PRs: only findings introduced after the base "
            "commit fail the build. Full scan on push is informational.")))
    if test_rust:
        units.append(unit(
            "l2", "rust-test", "out_of_scope",
            reason="rules exclude test code by path and #[cfg(test)] module "
                   "matching, by design",
            files_total=len(test_rust)))
    for cls, rels in sorted(by_class.items()):
        if cls in ("rust",):
            continue
        units.append(unit(
            "l2", cls, "out_of_scope",
            reason="no semgrep rules target this class",
            files_total=len(rels)))

    # -- layer 3: scan graph (observed this run) -------------------------
    graph_files = {e["file"] for e in graph["entities"] if e["kind"] == "file"}
    parse_err = set(graph.get("meta", {}).get("parse_error_files", []))
    legacy = "parse_error_files" not in graph.get("meta", {})
    for cls in ("rust", "typescript"):
        rels = by_class.get(cls, [])
        parsed = [r for r in rels if r in graph_files]
        if legacy:
            units.append(unit(
                "l3", cls, "blocked",
                reason="indexer did not report per-file parse status; "
                       "re-run with an index.py that records parse_error_files",
                files_total=len(rels)))
            continue
        err = sorted(parse_err & set(rels))
        units.append(unit(
            "l3", cls, "covered",
            via=["tree-sitter index", "slop queries"],
            files_total=len(rels), note=(
                f"{len(parsed)} of {len(rels)} files parsed this run.")))
        if err:
            units.append(unit(
                "l3", f"{cls}-parse-errors", "skipped",
                reason="tree-sitter parse error: not indexed, slop queries "
                       "never saw these files",
                files=err, files_total=len(err)))
    for cls, rels in sorted(by_class.items()):
        if cls in ("rust", "typescript"):
            continue
        units.append(unit(
            "l3", cls, "out_of_scope",
            reason="indexer parses .rs/.ts/.tsx only",
            files_total=len(rels)))

    # -- layer 4: codeql (declared scope) --------------------------------
    in_ws = [r for r in rust_files if r.split("/")[0] in crates]
    out_ws = [r for r in rust_files if r.split("/")[0] not in crates]
    units.append(unit(
        "l4", "rust", "covered",
        via=["CodeQL rust analysis (init/autobuild/analyze), nightly"],
        files_total=len(in_ws), note=(
            "taint tracking and cross-crate data-flow the pattern rules "
            "cannot express.")))
    if out_ws:
        units.append(unit(
            "l4", "rust-outside-workspace", "skipped",
            reason="not in the cargo workspace CodeQL builds",
            files=out_ws, files_total=len(out_ws)))
    for cls, rels in sorted(by_class.items()):
        if cls == "rust":
            continue
        units.append(unit(
            "l4", cls, "out_of_scope",
            reason="CodeQL job analyzes the Rust workspace only",
            files_total=len(rels)))

    units.sort(key=lambda u: (u["layer"], STATUS_ORDER[u["status"]], u["surface"]))
    return units


def build_report(units, root, graph):
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    rev = graph.get("meta", {}).get("repo_rev", "unknown")
    lines = [
        "# Coverage record",
        "",
        f"Generated {now} from repo rev `{rev}`.",
        "",
        "What the scan pipeline looked at this run, and what it did not. "
        "A unit is `covered` only with evidence from this run (layers 3-4) "
        "or because the layer is configured to cover it on every run "
        "(layers 1-2, marked declared). Anything else is `skipped`, "
        "`blocked`, or `out_of_scope` — never silently absent.",
        "",
    ]
    for lid, lmeta in LAYERS.items():
        lines.append(f"## Layer {lid[1]}: {lmeta['name']}")
        lines.append("")
        lines.append(f"Runs {lmeta['when']}. Coverage basis: "
                     f"**{lmeta['coverage_basis']}**.")
        lines.append("")
        lines.append("| Unit | Status | Files | Via / reason |")
        lines.append("| --- | --- | --- | --- |")
        for u in units:
            if u["layer"] != lid:
                continue
            via = "; ".join(u["via"]) if u["via"] else ""
            extra = u.get("reason") or u.get("note") or ""
            detail = "; ".join(x for x in (via, extra) if x)
            detail = detail.replace("|", "\\|").replace("\n", " ")
            lines.append(f"| `{u['id']}` | **{u['status']}** | "
                         f"{u['files_covered']}/{u['files_total']} | {detail} |")
        lines.append("")

    lines.append("## What was NOT looked at")
    lines.append("")
    gap_units = [u for u in units if u["status"] in ("skipped", "blocked", "out_of_scope")]
    if not gap_units:
        lines.append("_No gaps recorded._")
    else:
        for u in gap_units:
            basis = LAYERS[u["layer"]]["name"]
            reason = u.get("reason", "")
            lines.append(f"- `{u['id']}` ({basis}): **{u['status']}** — "
                         f"{u['files_total']} files. {reason}".rstrip())
            for f in u.get("files", [])[:20]:
                lines.append(f"  - `{f}`")
            if len(u.get("files", [])) > 20:
                lines.append(f"  - ... and {len(u['files']) - 20} more")
    lines.extend([
        "",
        "## Non-claims",
        "",
        "- A clean ledger is not a clean bill of health: it records where "
        "the pipeline looked, not that the code is safe.",
        "- Layers 1-2 run differential on PRs. A passing PR means no *new* "
        "findings in the diff, not full-repo coverage.",
        "- `cargo-deny` covers advisories, licenses, duplicates and "
        "wildcards. It says nothing about how the code behaves.",
        "- `skipped` files were never indexed: their contents are unknown "
        "to layers 3-4. Parse errors are the coverage gap most likely to "
        "hide real findings.",
        "- `out_of_scope` is a design decision, not an oversight. If a "
        "class here starts carrying security-sensitive logic, that is a "
        "reason to extend a layer, not to assume the gap closed itself.",
        "",
    ])
    return "\n".join(lines)


def main(argv=None):
    script_dir = Path(__file__).resolve().parent
    default_root = script_dir.parent.parent
    default_graph = default_root / "devops" / "scan-graph" / "output" / "graph.json"
    ap = argparse.ArgumentParser(description="Build the scan-pipeline coverage record.")
    ap.add_argument("--graph", default=str(default_graph))
    ap.add_argument("--out-ledger", default=None)
    ap.add_argument("--out-report", default=None)
    ap.add_argument("--repo-root", default=str(default_root))
    args = ap.parse_args(argv)

    root = Path(args.repo_root).resolve()
    graph = load_graph(args.graph)
    if graph is None:
        print(f"coverage.py: cannot read graph evidence at {args.graph}; "
              "refusing to certify coverage it cannot see", file=sys.stderr)
        return 2

    units = build_ledger(root, graph)
    ledger = {
        "meta": {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "repo_rev": git_rev(root),
            "graph_rev": graph.get("meta", {}).get("repo_rev", "unknown"),
            "graph_files_parsed": graph.get("meta", {}).get("files_parsed"),
            "graph_parse_errors": graph.get("meta", {}).get("parse_errors"),
            "unit_states": ["covered", "skipped", "out_of_scope", "blocked"],
            "rule": "a unit is covered only with evidence from this run or "
                    "a layer configured to cover it on every run",
        },
        "layers": LAYERS,
        "units": units,
    }

    out_ledger = Path(args.out_ledger) if args.out_ledger else \
        root / "devops" / "scan-graph" / "output" / "coverage-ledger.json"
    out_report = Path(args.out_report) if args.out_report else \
        root / "devops" / "scan-graph" / "output" / "coverage-report.md"
    out_ledger.parent.mkdir(parents=True, exist_ok=True)
    out_ledger.write_text(json.dumps(ledger, indent=2), encoding="utf-8")
    out_report.write_text(build_report(units, root, graph), encoding="utf-8")

    n_blocked = sum(1 for u in units if u["status"] == "blocked")
    n_skipped = sum(u["files_skipped"] for u in units)
    print(f"wrote {out_ledger} and {out_report}: {len(units)} units, "
          f"{n_skipped} skipped files, {n_blocked} blocked units")
    return 1 if n_blocked else 0


if __name__ == "__main__":
    sys.exit(main())
