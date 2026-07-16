//! Node-level thermal tracking to deprioritize CPUs exceeding 75°C.
//!
//! Intelligent VRF Assignment includes Thermal-Aware routing: nodes above the
//! thermal ceiling are deprioritized (or excluded) from heavy compute jobs.

/// CPU temperature ceiling in Celsius — above this, node is deprioritized.
pub const MAX_TEMP_CELSIUS: f32 = 75.0;

/// Soft-warning band start (°C). Between this and MAX, weight begins to decay.
pub const THERMAL_SOFT_CELSIUS: f32 = 65.0;

/// Routing / scheduling priority derived from measured CPU temperature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalPriority {
    /// Cool enough for full scheduling weight.
    Full,
    /// Elevated temperature — reduced weight but still eligible.
    Reduced,
    /// Over `MAX_TEMP_CELSIUS` — deprioritized (excluded from heavy jobs).
    Deprioritized,
}

/// Snapshot of a node's thermal state for the scheduler.
#[derive(Debug, Clone, PartialEq)]
pub struct ThermalState {
    pub peer_id: String,
    pub temp_celsius: f32,
    pub priority: ThermalPriority,
}

impl ThermalState {
    pub fn new(peer_id: impl Into<String>, temp_celsius: f32) -> Self {
        let priority = classify_temperature(temp_celsius);
        Self {
            peer_id: peer_id.into(),
            temp_celsius,
            priority,
        }
    }

    pub fn is_deprioritized(&self) -> bool {
        self.priority == ThermalPriority::Deprioritized
    }
}

/// Classify CPU temperature into a scheduling priority band.
pub fn classify_temperature(temp_celsius: f32) -> ThermalPriority {
    if temp_celsius > MAX_TEMP_CELSIUS {
        ThermalPriority::Deprioritized
    } else if temp_celsius >= THERMAL_SOFT_CELSIUS {
        ThermalPriority::Reduced
    } else {
        ThermalPriority::Full
    }
}

/// Returns true when the node must be deprioritized (> 75°C).
pub fn should_deprioritize(temp_celsius: f32) -> bool {
    temp_celsius > MAX_TEMP_CELSIUS
}

/// Integer scheduling weight in `[0, 1000]` from temperature.
/// - Full: 1000
/// - Reduced: linear decay from 1000 @ 65°C toward 100 @ 75°C
/// - Deprioritized: 0
pub fn thermal_schedule_weight(temp_celsius: f32) -> u32 {
    if temp_celsius > MAX_TEMP_CELSIUS {
        return 0;
    }
    if temp_celsius < THERMAL_SOFT_CELSIUS {
        return 1000;
    }
    let span = MAX_TEMP_CELSIUS - THERMAL_SOFT_CELSIUS; // 10
    let t = (temp_celsius - THERMAL_SOFT_CELSIUS).clamp(0.0, span);
    let weight = 1000.0 - (t / span) * 900.0;
    weight.round() as u32
}

/// Filter a candidate peer list, dropping nodes that exceed the thermal ceiling.
pub fn filter_cool_peers(states: &[ThermalState]) -> Vec<&ThermalState> {
    states
        .iter()
        .filter(|s| !should_deprioritize(s.temp_celsius))
        .collect()
}

/// Sort peers for thermal-aware scheduling: highest weight (coolest) first.
pub fn sort_by_thermal_weight(states: &mut [ThermalState]) {
    states.sort_by(|a, b| {
        thermal_schedule_weight(b.temp_celsius)
            .cmp(&thermal_schedule_weight(a.temp_celsius))
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });
}

pub fn init_thermal_aware() {
    println!("Initializing Thermal-Aware routing to protect edge node CPUs.");
    let hot = ThermalState::new("hot-node", 82.0);
    let cool = ThermalState::new("cool-node", 48.0);
    println!(
        "Thermal demo: hot={:?} weight={} | cool={:?} weight={}",
        hot.priority,
        thermal_schedule_weight(hot.temp_celsius),
        cool.priority,
        thermal_schedule_weight(cool.temp_celsius)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deprioritize_over_75c() {
        assert!(!should_deprioritize(75.0));
        assert!(should_deprioritize(75.1));
        assert!(should_deprioritize(90.0));
        assert!(!should_deprioritize(74.9));
        assert_eq!(
            classify_temperature(80.0),
            ThermalPriority::Deprioritized
        );
    }

    #[test]
    fn test_thermal_filter_excludes_hot_nodes() {
        let states = vec![
            ThermalState::new("a", 40.0),
            ThermalState::new("b", 76.0),
            ThermalState::new("c", 70.0),
            ThermalState::new("d", 100.0),
        ];
        let cool = filter_cool_peers(&states);
        let ids: Vec<&str> = cool.iter().map(|s| s.peer_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]);
        assert!(cool.iter().all(|s| !s.is_deprioritized()));
    }

    #[test]
    fn test_thermal_weights() {
        assert_eq!(thermal_schedule_weight(50.0), 1000);
        assert_eq!(thermal_schedule_weight(65.0), 1000);
        assert_eq!(thermal_schedule_weight(75.0), 100);
        assert_eq!(thermal_schedule_weight(76.0), 0);
        let mid = thermal_schedule_weight(70.0);
        assert!(mid > 100 && mid < 1000);
    }

    #[test]
    fn test_sort_prefers_cooler_nodes() {
        let mut states = vec![
            ThermalState::new("hot", 74.0),
            ThermalState::new("cool", 40.0),
            ThermalState::new("warm", 68.0),
        ];
        sort_by_thermal_weight(&mut states);
        assert_eq!(states[0].peer_id, "cool");
        assert_eq!(states[2].peer_id, "hot");
    }

    #[test]
    fn test_soft_band_classification() {
        assert_eq!(classify_temperature(64.9), ThermalPriority::Full);
        assert_eq!(classify_temperature(65.0), ThermalPriority::Reduced);
        assert_eq!(classify_temperature(75.0), ThermalPriority::Reduced);
    }
}
