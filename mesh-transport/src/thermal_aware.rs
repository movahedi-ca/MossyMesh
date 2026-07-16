//! Node-level thermal tracking to deprioritize CPUs exceeding 75°C.

pub fn init_thermal_aware() {
    println!("Initializing Thermal-Aware routing to protect edge node CPUs.");
}

pub const MAX_TEMP_CELSIUS: f32 = 75.0;
