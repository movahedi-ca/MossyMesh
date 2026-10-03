#!/usr/bin/env python3
"""Phase 4: slop detectors over the code graph JSON from index.py.

Reads the graph, runs five heuristic detectors, and emits a Markdown report.

Usage:
    python slop.py [GRAPH_JSON] [--out REPORT_MD]

Defaults: graph at <repo>/devops/scan-graph/output/graph.json;
report at <repo>/devops/scan-graph/output/slop-report.md.
"""

import argparse
import json
import sys
import tomllib
from datetime import datetime, timezone
from pathlib import Path

CONF_ORDER = {"low": 0, "medium": 1, "medium-high": 2, "high": 3}


def load_graph(path):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def loc(ent):
    return f"{ent['file']}:{ent['line']}"


# -- detectors -----------------------------------------------------------

def dead_functions(graph):
    """Functions with no incoming call edges and no incoming resolved edges."""
    funcs = [e for e in graph["entities"] if e["kind"] == "function"]
    ids = {e["id"] for e in funcs}
    names = {e["name"] for e in funcs}
    called_names = set()
    called_ids = set()
    for edge in graph["edges"]:
        if edge["kind"] != "call":
            continue
        if edge["to"] in names:
            called_names.add(edge["to"])
        if edge.get("resolved_to") in ids:
            called_ids.add(edge["resolved_to"])
    findings = []
    for f in funcs:
        if f.get("is_pub"):
            continue
        if f["name"] == "main":
            continue
        if f.get("is_test"):
            continue
        if Path(f["file"]).name == "main.rs":
            continue
        if f["name"] in called_names or f["id"] in called_ids:
            continue
        findings.append({
            "finding": f"Unreferenced function `{f['name']}`",
            "location": loc(f),
            "confidence": "medium",
            "note": "No incoming call edges. Dynamic dispatch, trait impls, "
                    "and macro-generated callers can hide real callers, so confirm manually.",
        })
    return findings


def wrappers(graph):
    """Single-call passthrough functions that add no visible logic."""
    findings = []
    for f in graph["entities"]:
        if f["kind"] != "function":
            continue
        if not f.get("body_is_single_call"):
            continue
        if f.get("is_pub") or f.get("is_test"):
            continue
        calls = f.get("calls", [])
        if len(calls) != 1 or calls[0] == f["name"]:
            continue
        callee = calls[0]
        if callee[:1].isupper():
            continue  # constructor / enum variant call (Ok, Some, Sitf), not a function
        findings.append({
            "finding": f"`{f['name']}` only forwards to `{callee}`",
            "location": loc(f),
            "confidence": "medium-high",
            "note": "Body holds a single call expression and nothing else. "
                    "Check whether the indirection still earns its keep.",
        })
    return findings


def noop_deprecations(graph):
    """Deprecated functions that are already empty or single-call shells."""
    findings = []
    for f in graph["entities"]:
        if f["kind"] != "function":
            continue
        if "deprecated" not in f.get("attrs", []):
            continue
        if not (f.get("line_count", 99) <= 3 or f.get("body_is_single_call")):
            continue
        findings.append({
            "finding": f"Deprecated `{f['name']}` is an empty or passthrough shell",
            "location": loc(f),
            "confidence": "high",
            "note": "Carries #[deprecated] and does no real work; removal candidate.",
        })
    return findings


def oversized_test_files(graph):
    """Test files with more than 500 lines."""
    test_files = {}
    for e in graph["entities"]:
        if e["kind"] == "file":
            path = e["file"]
            is_test = ("tests/" in path or "__tests__" in path
                       or "test" in Path(path).name.lower())
            test_files[e["id"]] = {"ent": e, "is_test": is_test, "lines": e.get("line_count", 0)}
    for e in graph["entities"]:
        if e["kind"] == "function" and e.get("is_test"):
            fid = next((k for k, v in test_files.items() if v["ent"]["file"] == e["file"]), None)
            if fid:
                test_files[fid]["is_test"] = True
    findings = []
    for v in test_files.values():
        if v["is_test"] and v["lines"] > 500:
            findings.append({
                "finding": f"Test file has {v['lines']} lines",
                "location": v["ent"]["file"],
                "confidence": "high",
                "note": "Plain metric: over the 500-line budget. Consider splitting.",
            })
    return findings


def unused_deps(graph, repo_root):
    """Heuristic: Cargo [dependencies] never referenced by a use edge or use line."""
    findings = []
    tomls = sorted(repo_root.rglob("Cargo.toml"))
    tomls = [p for p in tomls if "target" not in p.parts and ".git" not in p.parts]
    use_targets = [e["to"] for e in graph["edges"] if e["kind"] in ("use", "import")]
    rs_text = []
    for rs in repo_root.rglob("*.rs"):
        if any(part in ("target", ".git", "__pycache__") for part in rs.parts):
            continue
        try:
            rs_text.append(rs.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
    blob = "\n".join(rs_text)
    for toml in tomls:
        try:
            data = tomllib.loads(toml.read_text(encoding="utf-8"))
        except (OSError, tomllib.TOMLDecodeError):
            continue
        for dep in data.get("dependencies", {}):
            ident = dep.replace("-", "_")
            hit = (any(ident in t for t in use_targets)
                   or f"use {ident}" in blob or f"use {ident}::" in blob
                   or f"{ident}::" in blob)
            if not hit:
                findings.append({
                    "finding": f"Possibly unused dependency `{dep}`",
                    "location": toml.relative_to(repo_root).as_posix(),
                    "confidence": "low",
                    "note": "Heuristic only: no `use <crate>` edge or line found. "
                            "Build scripts, macros, re-exports, and feature-gated code "
                            "can all hide real usage. Verify with cargo before removing.",
                })
    return findings


DETECTORS = [
    ("Dead / unreferenced functions", dead_functions),
    ("Single-call wrapper functions", wrappers),
    ("No-op deprecations", noop_deprecations),
    ("Oversized test files", oversized_test_files),
    ("Possibly unused dependencies", None),  # needs repo_root; handled below
]


# -- report ---------------------------------------------------------------

def md_table(findings):
    lines = ["| Finding | Location | Confidence | Note |",
             "| --- | --- | --- | --- |"]
    for f in findings:
        row = [f["finding"], f["location"], f["confidence"], f["note"]]
        row = [c.replace("|", "\\|").replace("\n", " ") for c in row]
        lines.append("| " + " | ".join(row) + " |")
    return "\n".join(lines)


def build_report(graph, repo_root):
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    rev = graph.get("meta", {}).get("repo_rev", "unknown")
    sections = []
    total = 0
    for title, fn in DETECTORS:
        if fn is None:
            findings = unused_deps(graph, repo_root)
        else:
            findings = fn(graph)
        findings.sort(key=lambda f: CONF_ORDER.get(f["confidence"], 0), reverse=True)
        total += len(findings)
        body = md_table(findings) if findings else "_No findings._"
        sections.append(f"## {title} ({len(findings)})\n\n{body}")
    how_to = """## How to read this

Confidence labels say how much to trust each finding before acting:

- **high**: directly measured facts (a metric or an explicit attribute). Safe to act on.
- **medium-high**: strong structural signal with a small chance of a false positive.
- **medium**: worth a look, but callers can hide behind dynamic dispatch, trait impls,
  macros, or FFI boundaries. Confirm before deleting.
- **low**: leads, not verdicts. The unused-dependency check is a plain text/edge
  heuristic; build scripts and re-exports routinely defeat it.

This report is a triage list. Every finding below medium-high confidence should get
a human glance before any code is touched."""
    return (f"# Slop report\n\nGenerated {now} from repo rev `{rev}`.\n\n"
            f"{len(graph['entities'])} entities, {len(graph['edges'])} edges indexed. "
            f"{total} findings across {len(DETECTORS)} detectors.\n\n"
            + "\n\n".join(sections) + "\n\n" + how_to + "\n")


def main(argv=None):
    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parent.parent
    default_graph = repo_root / "devops" / "scan-graph" / "output" / "graph.json"
    ap = argparse.ArgumentParser(description="Run slop detectors over the code graph.")
    ap.add_argument("graph", nargs="?", default=str(default_graph))
    ap.add_argument("--out", default=None)
    args = ap.parse_args(argv)

    graph = load_graph(args.graph)
    out = Path(args.out) if args.out else repo_root / "devops" / "scan-graph" / "output" / "slop-report.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    report = build_report(graph, repo_root)
    out.write_text(report, encoding="utf-8")
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
