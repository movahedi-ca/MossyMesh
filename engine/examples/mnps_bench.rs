//! Print measured Mnps for the fixed engine workload (native host).
//!
//! ```text
//! cargo run -p engine --example mnps_bench --release
//! ```
//!
//! ~836 Mnps is aspirational only — this program always prints measured values.
//! See devops/engine-wasm.md for wasm32-wasip1 and bench notes.

use engine::{benchmark_mnps_detailed, init_engine};

fn main() {
    init_engine();
    let r = benchmark_mnps_detailed();
    println!("workload: {}", r.workload);
    println!("nodes:    {}", r.nodes);
    println!("seconds:  {:.6}", r.seconds);
    println!("mnps:     {:.4}", r.mnps);
    println!("(measured — do not hard-code aspirational ~836)");
}
