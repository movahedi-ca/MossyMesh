//! Fuzz the YATA/RGA + LWW-map CRDT merge for convergence (issue #36).
//!
//! Applies a random script of edits to two replicas, integrates the
//! resulting op logs in opposite orders, and asserts both replicas converge
//! to the same text and map state. This exercises deletes arriving before
//! their target inserts, concurrent interleavings, and LWW map races.

#![no_main]

use arbitrary::Arbitrary;
use consensus::crdt::{CrdtOp, Doc};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug, Clone, Copy)]
enum Action {
    /// Insert a byte as a char at a pseudo-random visible position.
    Insert(u8),
    /// Delete the first visible char (no-op when empty).
    Delete,
    /// LWW map set on one of 8 keys.
    MapSet(u8, u8),
    /// LWW map delete on one of 8 keys.
    MapDelete(u8),
}

fn apply(doc: &mut Doc, log: &mut Vec<CrdtOp>, action: Action) {
    match action {
        Action::Insert(byte) => {
            let len = doc.text_len();
            let idx = if len == 0 { 0 } else { (byte as usize) % (len + 1) };
            log.push(doc.insert_char(idx, byte as char));
        }
        Action::Delete => {
            if let Some(op) = doc.delete_char(0) {
                log.push(op);
            }
        }
        Action::MapSet(k, v) => {
            log.push(doc.map_set(format!("k{}", k % 8), vec![v]));
        }
        Action::MapDelete(k) => {
            log.push(doc.map_delete(format!("k{}", k % 8)));
        }
    }
}

fuzz_target!(|actions: Vec<Action>| {
    // Cap the script length: enough ops to interleave interestingly.
    let actions: Vec<Action> = actions.into_iter().take(256).collect();

    let mut a = Doc::new(1);
    let mut b = Doc::new(2);
    let mut ops_a = Vec::new();
    let mut ops_b = Vec::new();
    for action in &actions {
        apply(&mut a, &mut ops_a, *action);
        apply(&mut b, &mut ops_b, *action);
    }

    // Same op multiset, opposite integration orders. Replica `b` integrates
    // in reverse, so deletes land before their target inserts.
    for op in ops_a.iter().chain(ops_b.iter()) {
        a.integrate(op.clone());
    }
    for op in ops_b.iter().rev().chain(ops_a.iter().rev()) {
        b.integrate(op.clone());
    }

    assert_eq!(a.text(), b.text(), "replicas diverged on text");
    let mut keys_a: Vec<_> = a.map_keys().cloned().collect();
    let mut keys_b: Vec<_> = b.map_keys().cloned().collect();
    keys_a.sort();
    keys_b.sort();
    assert_eq!(keys_a, keys_b, "replicas diverged on map keys");
    for k in &keys_a {
        assert_eq!(
            a.map_get(k),
            b.map_get(k),
            "replicas diverged on map value"
        );
    }
});
