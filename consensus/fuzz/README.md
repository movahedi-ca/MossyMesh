# consensus fuzzing (cargo-fuzz)

Fuzz targets for the consensus CRDT (YATA/RGA sequence + LWW map).

## Prerequisites

- Rust nightly (cargo-fuzz needs `-Zsanitizer`):
  `rustup toolchain install nightly`
- cargo-fuzz: `cargo install cargo-fuzz`

## Run

```sh
cd consensus/fuzz
cargo +nightly fuzz run crdt_merge
```

## Targets

- `crdt_merge`: applies a random script of inserts, deletes, and map ops to
  two replicas, integrates the resulting op logs in opposite orders, and
  asserts both replicas converge (same text, same map keys and values).
  Catches ordering bugs like delete-before-insert divergence (issue #31).

## Corpus and artifacts

Interesting inputs are saved under `corpus/crdt_merge/` (gitignored).
Crashes go to `artifacts/crdt_merge/` (gitignored); minimize a crasher with
`cargo +nightly fuzz cmin crdt_merge` before filing it as an issue.
