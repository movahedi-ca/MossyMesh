//! Battery-curve weighting to route heavy traffic through AC-powered nodes.
//! Math Problem 3: Deterministic Sigmoid Penalty Curve

/// Computes a deterministic integer approximation of a sigmoid weighting function.
/// Formula: W(b) = 1000 / (1 + e^(-k * (b - threshold)))
/// For cross-device consensus determinism, we avoid floats and use a Taylor series
/// or fixed lookup approach. Here we implement an integer rational approximation.
/// Returns a weight from 0 to 1000.
pub fn calculate_battery_weight(battery_level: u8) -> u32 {
    let threshold: i32 = 20; // 20% battery is the critical drop-off
    let b = battery_level as i32;
    
    let diff = b - threshold;
    
    // Extreme cases to avoid overflow/underflow in approximation
    if diff <= -10 {
        return 0; // effectively dead for heavy routing
    }
    if diff >= 10 {
        return 1000; // full capacity routing
    }
    
    // Rational approximation of sigmoid scaled by 1000
    // S(x) ~= 1/2 + x / (2 * (1 + |x|))
    // We scale diff by a factor (e.g., k=1)
    let k_x = diff; 
    
    let numerator = k_x * 1000;
    let denominator = 2 * (1 + k_x.abs());
    
    let mut weight = 500 + (numerator / denominator);
    
    // clamp between 0 and 1000
    if weight < 0 {
        weight = 0;
    } else if weight > 1000 {
        weight = 1000;
    }
    
    weight as u32
}

pub fn init_battery_tracker() {
    println!("Initializing Battery-Curve weighting (prefer AC-powered nodes).");
    let w_low = calculate_battery_weight(15);
    let w_mid = calculate_battery_weight(20);
    let w_high = calculate_battery_weight(50);
    println!("Battery weights -> 15%: {}, 20%: {}, 50%: {}", w_low, w_mid, w_high);
}
