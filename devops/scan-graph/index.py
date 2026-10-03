#!/usr/bin/env python3
"""Phase 3: code graph indexer for the scan pipeline.

Walks the repo, parses .rs files with tree-sitter-rust and .ts/.tsx files
with tree-sitter-typescript, and emits a JSON graph of entities and edges.

Usage:
    python index.py [REPO_ROOT] [--out PATH]

Defaults: repo root is two directories above this script;
output is <repo>/devops/scan-graph/output/graph.json.

Requires tree-sitter 0.25.x (see requirements.txt): 0.26.0 segfaults
during tree walks.
"""

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

from tree_sitter import Language, Parser
import tree_sitter_rust
import tree_sitter_typescript

SKIP_DIRS = {".git", "target", "node_modules", "dist", "output", "__pycache__"}
RUST_EXTS = {".rs"}
TS_EXT = ".ts"
TSX_EXT = ".tsx"

LANG_RUST = Language(tree_sitter_rust.language())
LANG_TS = Language(tree_sitter_typescript.language_typescript())
LANG_TSX = Language(tree_sitter_typescript.language_tsx())


def node_text(node):
    return node.text.decode("utf-8", errors="replace")


def node_line(node):
    # Bind the Point to a local first: chaining .row directly on the
    # node.start_point temporary corrupts the heap in tree-sitter 0.26.0
    # and segfaults a later tree-sitter call.
    sp = node.start_point
    return sp.row + 1


def node_line_count(node):
    sp = node.start_point
    ep = node.end_point
    return ep.row - sp.row + 1


def stmt_children(block):
    """Named children of a block that act as statements (skip braces/comments)."""
    out = []
    for child in block.children:
        if not child.is_named:
            continue
        if child.type in ("line_comment", "block_comment"):
            continue
        out.append(child)
    return out


class GraphBuilder:
    def __init__(self):
        self.entities = []
        self.edges = []
        self.files_parsed = 0
        self.parse_errors = 0
        self._file_ids = {}
        # function name -> list of entity ids (for call resolution)
        self._name_index = {}
        # Keep parsers alive for the whole run: trees/nodes reference the
        # underlying parser, and a temporary Parser would be GC'd mid-walk.
        self._parsers = {
            ".rs": Parser(LANG_RUST),
            ".ts": Parser(LANG_TS),
            ".tsx": Parser(LANG_TSX),
        }

    # -- entity helpers -------------------------------------------------
    def add_entity(self, kind, name, rel, line, **extra):
        qual = extra.pop("qual", name)
        eid = f"{kind}:{rel}:{qual}:{line}".replace("\\", "/")
        ent = {"id": eid, "kind": kind, "name": name, "file": rel, "line": line}
        ent.update(extra)
        self.entities.append(ent)
        if kind == "function":
            self._name_index.setdefault(name, []).append(eid)
        return eid

    def add_edge(self, frm, to, kind, **extra):
        edge = {"from": frm, "to": to, "kind": kind}
        edge.update(extra)
        self.edges.append(edge)

    # -- top level ------------------------------------------------------
    def index_repo(self, root):
        root = Path(root)
        for path in sorted(root.rglob("*")):
            if not path.is_file():
                continue
            if any(part in SKIP_DIRS for part in path.parts):
                continue
            if path.suffix in RUST_EXTS:
                self._index_rust(path, root)
            elif path.suffix in (TS_EXT, TSX_EXT):
                self._index_ts(path, root)

    def _parser_for(self, suffix):
        return self._parsers[".tsx" if suffix == TSX_EXT else ".ts" if suffix == TS_EXT else ".rs"]

    # -- Rust -----------------------------------------------------------
    def _index_rust(self, path, root):
        rel = path.relative_to(root).as_posix()
        try:
            data = path.read_bytes()
            tree = self._parser_for(path.suffix).parse(data)
            if tree.root_node.has_error:
                raise ValueError("tree-sitter reported error nodes")
        except Exception:
            self.parse_errors += 1
            return
        self.files_parsed += 1
        file_id = self.add_entity(
            "file", rel, rel, 1, line_count=data.decode("utf-8", errors="replace").count("\n") + 1
        )
        self._file_ids[rel] = file_id
        in_test_file = self._is_test_path(rel)
        self._handle_items(
            tree.root_node.children, file_id, rel, data, parent_label=None,
            in_test=in_test_file, is_rust=True,
        )

    # -- TypeScript -----------------------------------------------------
    def _index_ts(self, path, root):
        rel = path.relative_to(root).as_posix()
        try:
            data = path.read_bytes()
            tree = self._parser_for(path.suffix).parse(data)
            if tree.root_node.has_error:
                raise ValueError("tree-sitter reported error nodes")
        except Exception:
            self.parse_errors += 1
            return
        self.files_parsed += 1
        file_id = self.add_entity(
            "file", rel, rel, 1, line_count=data.decode("utf-8", errors="replace").count("\n") + 1
        )
        self._file_ids[rel] = file_id
        in_test_file = self._is_test_path(rel)
        self._handle_items(
            tree.root_node.children, file_id, rel, data, parent_label=None,
            in_test=in_test_file, is_rust=False,
        )

    # -- shared item walker ---------------------------------------------
    @staticmethod
    def _is_test_path(rel):
        parts = rel.split("/")
        if any(p == "tests" or p == "__tests__" for p in parts):
            return True
        base = parts[-1].lower()
        return ".test." in base or ".spec." in base or base.startswith("test_")

    def _handle_items(self, children, file_id, rel, data, parent_label, in_test, is_rust,
                      exported=False):
        pending_attrs = []
        for child in children:
            if child.type == "attribute_item":
                pending_attrs.append(self._parse_rust_attr(child))
                continue
            if child.type in ("line_comment", "block_comment"):
                continue
            if child.type == "export_statement" and not is_rust:
                inner = [c for c in child.children if c.is_named and c.type not in ("export",)]
                self._handle_items(inner, file_id, rel, data, parent_label, in_test,
                                   is_rust, exported=True)
                continue
            if child.is_named:
                self._handle_item(child, file_id, rel, data, parent_label, in_test,
                                  is_rust, pending_attrs, exported)
                pending_attrs = []

    @staticmethod
    def _parse_rust_attr(node):
        """Return (attr_name, is_cfg_test) for an attribute_item."""
        name = ""
        is_cfg_test = False
        for child in node.children:
            if child.type == "attribute":
                ident = child.child_by_field_name("name")
                if ident is None:
                    named = [c for c in child.children if c.is_named]
                    ident = named[0] if named else None
                name = node_text(ident) if ident is not None else ""
                if name == "cfg":
                    tok = node_text(child)
                    is_cfg_test = "test" in tok
        return (name, is_cfg_test)

    def _handle_item(self, node, file_id, rel, data, parent_label, in_test, is_rust,
                     attrs, exported):
        t = node.type
        attr_names = [a for a, _ in attrs]
        cfg_test = any(c for _, c in attrs)
        line = node_line(node)

        if is_rust:
            if t in ("function_item", "function_signature_item"):
                name_node = next((c for c in node.children if c.type == "identifier"), None)
                name = node_text(name_node) if name_node is not None else "<anon>"
                self._record_function(node, file_id, rel, line, name, parent_label,
                                      in_test or self._is_test_path(rel) or name.startswith("test_"),
                                      attr_names, is_rust=True, exported=False)
                return
            if t == "struct_item":
                self._record_named(node, "struct", "type_identifier", file_id, rel, line,
                                   parent_label)
                return
            if t == "enum_item":
                self._record_named(node, "enum", "type_identifier", file_id, rel, line,
                                   parent_label)
                return
            if t == "trait_item":
                tname = self._type_name(node)
                self._record_named(node, "trait", "type_identifier", file_id, rel, line,
                                   parent_label)
                body = next((c for c in node.children if c.type == "declaration_list"), None)
                if body is not None:
                    self._handle_items(body.children, file_id, rel, data,
                                        f"trait {tname}", in_test or cfg_test, True)
                return
            if t == "impl_item":
                tname = self._type_name(node)
                body = next((c for c in node.children if c.type == "declaration_list"), None)
                if body is not None:
                    self._handle_items(body.children, file_id, rel, data,
                                        f"impl {tname}", in_test, True)
                return
            if t == "mod_item":
                mname = self._ident_name(node)
                qual = f"{parent_label}.{mname}" if parent_label else mname
                self.add_entity("module", mname, rel, line, qual=qual, parent=parent_label)
                body = next((c for c in node.children if c.type == "declaration_list"), None)
                if body is not None:
                    self._handle_items(body.children, file_id, rel, data, qual,
                                        in_test or cfg_test or mname == "tests", True)
                return
            if t == "use_declaration":
                target = self._use_target(node)
                self.add_edge(file_id, target, "use")
                return
            return

        # TypeScript
        if t == "function_declaration":
            name = self._ident_name(node)
            self._record_function(node, file_id, rel, line, name, parent_label,
                                  in_test or name.startswith("test_"), [],
                                  is_rust=False, exported=exported)
            return
        if t == "method_definition":
            name = self._ident_name(node)
            self._record_function(node, file_id, rel, line, name, parent_label,
                                  in_test or name.startswith("test_"), [],
                                  is_rust=False, exported=exported)
            return
        if t == "class_declaration":
            cname = self._ident_name(node)
            qual = f"{parent_label}.{cname}" if parent_label else cname
            self.add_entity("class", cname, rel, line, qual=qual, parent=parent_label,
                            is_pub=exported)
            body = next((c for c in node.children if c.type == "class_body"), None)
            if body is not None:
                self._handle_items(body.children, file_id, rel, data, f"class {cname}",
                                   in_test, False)
            return
        if t == "interface_declaration":
            iname = self._ident_name(node)
            qual = f"{parent_label}.{iname}" if parent_label else iname
            self.add_entity("interface", iname, rel, line, qual=qual, parent=parent_label,
                            is_pub=exported)
            return
        if t == "import_statement":
            target = self._import_target(node)
            self.add_edge(file_id, target, "import")
            return

    @staticmethod
    def _ident_name(node):
        for c in node.children:
            if c.type in ("identifier", "property_identifier", "type_identifier"):
                return node_text(c)
        return "<anon>"

    @staticmethod
    def _type_name(node):
        for c in node.children:
            if c.type == "type_identifier":
                return node_text(c)
        return "<anon>"

    def _record_named(self, node, kind, name_type, file_id, rel, line, parent_label):
        name = "<anon>"
        for c in node.children:
            if c.type == name_type:
                name = node_text(c)
                break
        qual = f"{parent_label}.{name}" if parent_label else name
        self.add_entity(kind, name, rel, line, qual=qual, parent=parent_label)

    @staticmethod
    def _use_target(node):
        text = node_text(node).strip()
        if text.startswith("use"):
            text = text[3:].strip()
        if text.endswith(";"):
            text = text[:-1].strip()
        return re.sub(r"\s+", " ", text)

    @staticmethod
    def _import_target(node):
        for c in node.children:
            if c.type == "string":
                return node_text(c).strip().strip("\"'")
        return node_text(node)

    # -- function bodies ------------------------------------------------
    def _record_function(self, node, file_id, rel, line, name, parent_label, is_test,
                         attr_names, is_rust, exported):
        qual = f"{parent_label}.{name}" if parent_label else name
        if is_rust:
            block = next((c for c in node.children if c.type == "block"), None)
            is_pub = any(c.type == "visibility_modifier" for c in node.children)
        else:
            block = next((c for c in node.children if c.type == "statement_block"), None)
            is_pub = exported
        calls = []
        single = False
        if block is not None:
            calls = self._collect_calls(block, is_rust)
            stmts = stmt_children(block)
            call_nodes = [n for n in self._iter_named(block)
                          if n.type == "call_expression"]
            single = len(stmts) == 1 and len(call_nodes) == 1
        line_count = node_line_count(node)
        fid = self.add_entity(
            "function", name, rel, line, qual=qual, parent=parent_label,
            line_count=line_count, is_test=is_test, is_pub=is_pub,
            calls=calls, body_is_single_call=single, attrs=attr_names,
        )
        for callee in calls:
            self.add_edge(fid, callee, "call")

    def _collect_calls(self, block, is_rust):
        seen = []
        for n in self._iter_named(block):
            if n.type != "call_expression":
                continue
            callee = self._callee_name(n, is_rust)
            if callee and callee not in seen:
                seen.append(callee)
        return seen

    @staticmethod
    def _iter_named(node):
        stack = [node]
        while stack:
            cur = stack.pop()
            yield cur
            stack.extend(reversed(cur.children))

    def _callee_name(self, call_node, is_rust):
        fn = call_node.child_by_field_name("function")
        if fn is None:
            named = [c for c in call_node.children if c.is_named]
            fn = named[0] if named else None
        if fn is None:
            return ""
        if fn.type in ("identifier", "property_identifier"):
            return node_text(fn)
        # scoped_identifier, field_expression, member_expression: take last segment
        last = ""
        for c in fn.children:
            if c.is_named and c.type not in ("crate", "self", "super"):
                last = node_text(c)
        return last

    # -- call resolution -------------------------------------------------
    def resolve_calls(self):
        for edge in self.edges:
            if edge["kind"] != "call":
                continue
            matches = self._name_index.get(edge["to"], [])
            edge["resolved_to"] = matches[0] if len(matches) == 1 else None


def git_rev(root):
    try:
        out = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True, timeout=10
        )
        rev = out.stdout.strip()
        return rev if rev else "unknown"
    except Exception:
        return "unknown"


def main(argv=None):
    script_dir = Path(__file__).resolve().parent
    default_root = script_dir.parent.parent  # devops/scan-graph -> repo root
    ap = argparse.ArgumentParser(description="Index repo code into a JSON graph.")
    ap.add_argument("repo_root", nargs="?", default=str(default_root))
    ap.add_argument("--out", default=None)
    args = ap.parse_args(argv)

    root = Path(args.repo_root).resolve()
    out = Path(args.out) if args.out else root / "devops" / "scan-graph" / "output" / "graph.json"
    out.parent.mkdir(parents=True, exist_ok=True)

    builder = GraphBuilder()
    builder.index_repo(root)
    builder.resolve_calls()

    graph = {
        "meta": {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "repo_rev": git_rev(root),
            "files_parsed": builder.files_parsed,
            "parse_errors": builder.parse_errors,
        },
        "entities": builder.entities,
        "edges": builder.edges,
    }
    out.write_text(json.dumps(graph, indent=2), encoding="utf-8")
    print(f"wrote {out}: {len(builder.entities)} entities, "
          f"{len(builder.edges)} edges, {builder.files_parsed} files parsed, "
          f"{builder.parse_errors} parse errors")
    return 0


if __name__ == "__main__":
    sys.exit(main())
