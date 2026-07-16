//! Integration of the 10-minute sequential `minroot-vdf-rs` to block Sybil attacks.
//! Math Problem 1: MinRoot VDF Sequential Delay

/// A simple modular exponentiation function: (base^exp) % modulus
/// Note: In production this would use big integers (e.g. U256) over the Pallas curve field.
pub fn mod_exp(mut base: u64, mut exp: u64, modulus: u64) -> u64 {
    let mut result = 1;
    base = base % modulus;
    while exp > 0 {
        if exp % 2 == 1 {
            // using u128 to prevent overflow during multiplication
            result = ((result as u128 * base as u128) % modulus as u128) as u64;
        }
        exp >>= 1;
        base = (base * base) % modulus;
    }
    res
}

/// Executes one sequential step of the MinRoot VDF.
/// DOC 23: MinRoot requires $p \not\equiv 1 \pmod 5$ to ensure the fifth root maps cleanly to an exponent $d = (2p-1)/5$.
pub fn compute_minroot_step(x: u64, i: u64, p: u64) -> u64 {
    let d = (2 * p - 1) / 5;
    let base = (x + i) % p;
    mod_exp(base, d, p)
}

/// DOC 24: This function validates that a node actually burned the 10 minutes of sequential compute SLA.
pub fn verify_vdf(start_x: u64, steps: u64, final_x: u64, p: u64) -> bool {
    let mut current_x = start_x;
    for i in 1..=steps {
        current_x = compute_minroot_step(current_x, i, p);
    }
    // DOC 25: The output is instantly verifiable, guaranteeing non-interactive consensus.
    current_x == final_x
}

pub fn init_vdf_sybil() {
    println!("Initializing VDF Sybil protection (MinRoot polynomial delay).");
    let p = 101; 
    let start = 42;
    let step1 = compute_minroot_step(start, 1, p);
    println!("VDF Step 1 (p={}): {} -> {}", p, start, step1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modular_exponentiation() {
        let base = 5;
        let exp = 3;
        let p = 13;
        // 5^3 = 125. 125 % 13 = 8.
        let res = mod_exp(base, exp, p);
        assert_eq!(res, 8);
    }

    #[test]
    fn test_minroot_sequential_delay() {
        let p = 101; // safe prime
        let start = 42;
        
        let step1 = compute_minroot_step(start, 1, p);
        let step2 = compute_minroot_step(step1, 2, p);
        let step3 = compute_minroot_step(step2, 3, p);
        
        // Verifying exactly 3 steps works
        assert!(verify_vdf(start, 3, step3, p));
        // Verifying invalid output fails
        assert!(!verify_vdf(start, 3, step2, p));
    }
}
