//! STUN-less hole punching via deterministic port prediction for heavy lines.

pub fn init_stun_hole_punch() {
    println!("Initializing STUN-less deterministic port prediction hole punching.");
}

/// Deterministic Symmetric NAT Port Prediction Algorithm.
/// Since offline mesh nodes lack STUN servers, we predict the external port 
/// by sequentially scanning a calculated probabilistic spread.
pub fn predict_nat_port(internal_port: u16, attempt: u16) -> u16 {
    // A standard symmetric NAT increments ports sequentially.
    // We scan a spread of up to 50 ports outward from the base prediction.
    let base_prediction = internal_port.wrapping_add(2); // Typical offset
    
    // Spread calculation: alternate up and down (e.g. +1, -1, +2, -2)
    let spread = if attempt % 2 == 0 {
        attempt / 2
    } else {
        !((attempt / 2)) + 1 // Two's complement negative
    };
    
    base_prediction.wrapping_add(spread)
}
