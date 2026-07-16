//! Battery-curve weighting to route heavy traffic through AC-powered nodes.
//! Math Problem 3: Deterministic Sigmoid Penalty Curve
//! DOC 11: This file solves the Non-Deterministic Float Problem by strictly avoiding `f32`/`f64`.

/// Computes a deterministic integer approximation of a sigmoid weighting function.
/// Formula: W(b) = 1000 / (1 + e^(-k * (b - threshold)))
/// For cross-device consensus determinism, we avoid floats and use a Taylor series
/// or fixed lookup approach. Here we implement an integer rational approximation.
/// Returns a weight from 0 to 1000.
/// DOC 12: The rational approximation $S(x) \approx 1/2 + x / (2(1 + |x|))$ is scaled by 1000 to remain in `u32` integer space.
pub fn calculate_battery_weight(battery_level: u8) -> u32 {
    // DOC 13: 20% acts as a cliff; nodes below this are effectively dead for routing purposes.
    let threshold: i32 = 20; // 20% battery is the critical drop-off
    let b = battery_level as i32;
    
    let diff = b - threshold;
    
    // Extreme cases to avoid overflow/underflow in approximation
    if diff <= -10 {
        return 0; // effectively dead for heavy routing
    }
    if diff >= 10 {
        // DOC 14: If battery is >= 30%, it is treated equivalently to 100% (AC power) for routing.
        return 1000; // full capacity routing
    }
    
    // Rational approximation of sigmoid scaled by 1000
    // S(x) ~= 1/2 + x / (2 * (1 + |x|))
    // We scale diff by a factor (e.g., k=1)
    let k_x = diff; 
    
    // DOC 15: We perform algebraic multiplication first to prevent precision loss during the integer division.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_battery_threshold_death() {
        // Battery is heavily penalized below 20%. Specifically 10% is fully dead (0 weight).
        let weight = calculate_battery_weight(9);
        assert_eq!(weight, 0);
    }

    #[test]
    fn test_battery_ac_power() {
        // Battery >= 30% acts as AC power (max weight 1000).
        let weight = calculate_battery_weight(100);
        assert_eq!(weight, 1000);
        let weight_30 = calculate_battery_weight(30);
        assert_eq!(weight_30, 1000);
    }

    #[test]
    fn test_battery_curve_determinism() {
        // Curve scaling verification
        let w_15 = calculate_battery_weight(15);
        let w_20 = calculate_battery_weight(20); // The threshold
        let w_25 = calculate_battery_weight(25);
        
        assert!(w_15 < w_20);
        assert!(w_20 < w_25);
        
        // Exact integer mapping for the 20% cliff
        assert_eq!(w_20, 500); // exactly at 20% diff = 0, weight = 500
    }
}
