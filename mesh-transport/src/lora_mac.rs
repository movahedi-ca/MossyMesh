//! CSMA/CA MAC for lightweight LoRa physical links (simulation-capable).
//!
//! Provides deterministic airtime estimation (integer microseconds),
//! EU-style duty-cycle accounting, and a CSMA/CA contention model suitable
//! for offline mesh simulation without RF hardware.

/// Maximum application payload per LoRa frame (SX126x / Semtech common limit).
pub const LORA_MAX_PAYLOAD: usize = 255;

/// Default CSMA/CA slot time in microseconds (channel-time quantum).
pub const DEFAULT_SLOT_US: u32 = 1000;

/// Default contention window (slots) for random backoff.
pub const DEFAULT_CW_SLOTS: u32 = 16;

/// Default CCA (clear-channel assessment) listen duration in microseconds.
pub const DEFAULT_CCA_US: u32 = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadingFactor {
    Sf7 = 7,
    Sf8 = 8,
    Sf9 = 9,
    Sf10 = 10,
    Sf11 = 11,
    Sf12 = 12,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bandwidth {
    Bw125 = 125_000,
    Bw250 = 250_000,
    Bw500 = 500_000,
}

/// Coding rate CR = 4/(4+n); stored as the additive n (1..=4 → 4/5..4/8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodingRate(pub u8);

impl CodingRate {
    pub const CR_4_5: CodingRate = CodingRate(1);
    pub const CR_4_6: CodingRate = CodingRate(2);
    pub const CR_4_7: CodingRate = CodingRate(3);
    pub const CR_4_8: CodingRate = CodingRate(4);
}

/// Radio parameters used for airtime and MAC decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoraRadioConfig {
    pub sf: SpreadingFactor,
    pub bw: Bandwidth,
    pub cr: CodingRate,
    pub preamble_symbols: u32,
    pub explicit_header: bool,
    pub crc_enabled: bool,
    pub low_data_rate_optimize: bool,
}

impl Default for LoraRadioConfig {
    fn default() -> Self {
        // Conservative long-range defaults suitable for sparse mesh islands.
        Self {
            sf: SpreadingFactor::Sf9,
            bw: Bandwidth::Bw125,
            cr: CodingRate::CR_4_5,
            preamble_symbols: 8,
            explicit_header: true,
            crc_enabled: true,
            low_data_rate_optimize: false,
        }
    }
}

/// Single LoRa MAC frame with CRC-16 over payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoraFrame {
    pub payload: Vec<u8>,
    pub crc: u16,
}

impl LoraFrame {
    pub fn new(payload: Vec<u8>) -> Result<Self, LoraMacError> {
        if payload.len() > LORA_MAX_PAYLOAD {
            return Err(LoraMacError::PayloadTooLarge {
                len: payload.len(),
                max: LORA_MAX_PAYLOAD,
            });
        }
        let crc = calculate_crc(&payload);
        Ok(Self { payload, crc })
    }

    pub fn verify_crc(&self) -> bool {
        calculate_crc(&self.payload) == self.crc
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoraMacError {
    PayloadTooLarge { len: usize, max: usize },
    DutyCycleExceeded { needed_us: u64, remaining_us: u64 },
    ChannelBusy,
    MaxRetriesExceeded,
}

/// Calculates the standard CRC-16-CCITT (polynomial 0x1021).
/// Guarantees small LoRa payloads are not accepted when corrupted by RF noise.
pub fn calculate_crc(payload: &[u8]) -> u16 {
    let polynomial = 0x1021u16;
    let mut crc = 0xFFFFu16;

    for &byte in payload {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ polynomial;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Symbol duration in microseconds: T_sym = 2^SF / BW * 1e6.
pub fn symbol_duration_us(cfg: &LoraRadioConfig) -> u64 {
    let sf = cfg.sf as u32;
    let bw = cfg.bw as u32 as u64;
    // (2^SF * 1_000_000) / BW
    ((1u64 << sf) * 1_000_000) / bw
}

/// Integer-ceil division.
fn div_ceil(n: i64, d: i64) -> i64 {
    if d <= 0 {
        return 0;
    }
    if n <= 0 {
        return 0;
    }
    (n + d - 1) / d
}

/// Semtech SX127x/SX126x airtime estimate in microseconds (integer, deterministic).
///
/// ```text
/// T_preamble = (N_preamble + 4.25) * T_sym
/// payload_symb = 8 + max(ceil((8PL - 4SF + 28 + 16CRC - 20H) / (4(SF-2DE))) * (CR+4), 0)
/// T_packet   = T_preamble + payload_symb * T_sym
/// ```
pub fn estimate_airtime_us(payload_len: usize, cfg: &LoraRadioConfig) -> u64 {
    let sf = cfg.sf as i64;
    let t_sym = symbol_duration_us(cfg);

    // Preamble: (N + 4.25) * T_sym = N*T_sym + (17/4)*T_sym
    let t_preamble = (cfg.preamble_symbols as u64) * t_sym + (17 * t_sym) / 4;

    let pl = payload_len as i64;
    let crc = if cfg.crc_enabled { 1i64 } else { 0 };
    let h = if cfg.explicit_header { 0i64 } else { 1 };
    let de = if cfg.low_data_rate_optimize { 1i64 } else { 0 };
    let cr_n = cfg.cr.0.max(1).min(4) as i64;

    let numerator = 8 * pl - 4 * sf + 28 + 16 * crc - 20 * h;
    let denom = 4 * (sf - 2 * de);
    let ceil_term = div_ceil(numerator, denom);
    let payload_symb = 8 + std::cmp::max(ceil_term * (cr_n + 4), 0);

    t_preamble + (payload_symb as u64) * t_sym
}

/// Rolling duty-cycle tracker for regulatory limits (e.g. EU 868 MHz 1%).
#[derive(Debug, Clone)]
pub struct DutyCycleTracker {
    /// Maximum on-air fraction in basis points (100 = 1.00%).
    pub limit_bp: u32,
    /// Observation window in microseconds (default 1 hour).
    pub window_us: u64,
    /// Cumulative on-air time inside the current window.
    on_air_us: u64,
    /// Simulated clock (microseconds since epoch/start).
    now_us: u64,
    /// Start of the current duty-cycle window.
    window_start_us: u64,
}

impl DutyCycleTracker {
    /// EU 868.0–868.6: 1% duty cycle over a 1-hour window.
    pub fn eu868_1pct() -> Self {
        Self {
            limit_bp: 100, // 1.00%
            window_us: 3_600_000_000, // 1 hour
            on_air_us: 0,
            now_us: 0,
            window_start_us: 0,
        }
    }

    pub fn with_limit(limit_bp: u32, window_us: u64) -> Self {
        Self {
            limit_bp,
            window_us,
            on_air_us: 0,
            now_us: 0,
            window_start_us: 0,
        }
    }

    pub fn advance_time(&mut self, delta_us: u64) {
        self.now_us = self.now_us.saturating_add(delta_us);
        self.maybe_roll_window();
    }

    pub fn set_time(&mut self, now_us: u64) {
        self.now_us = now_us;
        self.maybe_roll_window();
    }

    fn maybe_roll_window(&mut self) {
        if self.now_us.saturating_sub(self.window_start_us) >= self.window_us {
            // Coarse reset: full windows elapsed drop prior on-air credit.
            let elapsed = self.now_us.saturating_sub(self.window_start_us);
            let windows = elapsed / self.window_us;
            self.window_start_us = self
                .window_start_us
                .saturating_add(windows * self.window_us);
            self.on_air_us = 0;
        }
    }

    /// Maximum on-air budget for the window in microseconds.
    pub fn budget_us(&self) -> u64 {
        (self.window_us.saturating_mul(self.limit_bp as u64)) / 10_000
    }

    pub fn remaining_us(&self) -> u64 {
        self.budget_us().saturating_sub(self.on_air_us)
    }

    pub fn can_transmit(&self, airtime_us: u64) -> bool {
        airtime_us <= self.remaining_us()
    }

    /// Record a successful transmission's on-air time.
    pub fn record_tx(&mut self, airtime_us: u64) -> Result<(), LoraMacError> {
        self.maybe_roll_window();
        if !self.can_transmit(airtime_us) {
            return Err(LoraMacError::DutyCycleExceeded {
                needed_us: airtime_us,
                remaining_us: self.remaining_us(),
            });
        }
        self.on_air_us = self.on_air_us.saturating_add(airtime_us);
        self.now_us = self.now_us.saturating_add(airtime_us);
        Ok(())
    }

    pub fn on_air_us(&self) -> u64 {
        self.on_air_us
    }
}

/// CSMA/CA state machine for simulation.
#[derive(Debug, Clone)]
pub struct CsmaCaController {
    pub slot_us: u32,
    pub cw_slots: u32,
    pub cca_us: u32,
    pub max_retries: u32,
    /// Deterministic PRNG state (LCG) for backoff without OS entropy.
    rng_state: u32,
    /// Simulated channel occupancy: true = busy at next CCA.
    pub channel_busy: bool,
}

impl Default for CsmaCaController {
    fn default() -> Self {
        Self::new(0xC0FFEE)
    }
}

impl CsmaCaController {
    pub fn new(seed: u32) -> Self {
        Self {
            slot_us: DEFAULT_SLOT_US,
            cw_slots: DEFAULT_CW_SLOTS,
            cca_us: DEFAULT_CCA_US,
            max_retries: 5,
            rng_state: seed.max(1),
            channel_busy: false,
        }
    }

    /// Deterministic LCG: X_{n+1} = (aX + c) mod 2^32.
    fn next_u32(&mut self) -> u32 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
        self.rng_state
    }

    /// Random backoff in \[0, cw_slots) slots, returned as microseconds.
    pub fn random_backoff_us(&mut self) -> u64 {
        let slots = self.next_u32() % self.cw_slots.max(1);
        (slots as u64) * (self.slot_us as u64)
    }

    /// Run CSMA/CA before transmitting.
    ///
    /// Returns total contention delay (CCA + backoffs) on success.
    /// Does **not** include airtime itself.
    pub fn contend_for_channel(&mut self) -> Result<u64, LoraMacError> {
        let mut total_delay = 0u64;
        let mut attempt = 0u32;

        loop {
            // CCA listen period
            total_delay = total_delay.saturating_add(self.cca_us as u64);

            if !self.channel_busy {
                return Ok(total_delay);
            }

            attempt += 1;
            if attempt > self.max_retries {
                return Err(LoraMacError::MaxRetriesExceeded);
            }

            // Exponential-ish growth of CW with attempt, capped.
            let expand = self.cw_slots.saturating_mul(1 << (attempt.min(4) - 1));
            let old_cw = self.cw_slots;
            self.cw_slots = expand.min(64);
            let backoff = self.random_backoff_us();
            self.cw_slots = old_cw;
            total_delay = total_delay.saturating_add(backoff);

            // After backoff, channel is assumed free for simulation unless
            // the caller re-asserts `channel_busy`.
            self.channel_busy = false;
        }
    }
}

/// High-level LoRa MAC facade used by the mesh stack and simulators.
#[derive(Debug, Clone)]
pub struct LoraMac {
    pub config: LoraRadioConfig,
    pub duty: DutyCycleTracker,
    pub csma: CsmaCaController,
}

impl LoraMac {
    pub fn new(config: LoraRadioConfig, seed: u32) -> Self {
        Self {
            config,
            duty: DutyCycleTracker::eu868_1pct(),
            csma: CsmaCaController::new(seed),
        }
    }

    pub fn default_sim() -> Self {
        Self::new(LoraRadioConfig::default(), 0xA5A5_5A5A)
    }

    /// Build a frame and estimate airtime without transmitting.
    pub fn prepare_frame(&self, payload: Vec<u8>) -> Result<(LoraFrame, u64), LoraMacError> {
        let frame = LoraFrame::new(payload)?;
        let airtime = estimate_airtime_us(frame.payload.len(), &self.config);
        Ok((frame, airtime))
    }

    /// Full TX path: duty-cycle check → CSMA/CA → record airtime.
    /// Returns (frame, airtime_us, contention_delay_us).
    pub fn transmit(&mut self, payload: Vec<u8>) -> Result<(LoraFrame, u64, u64), LoraMacError> {
        let (frame, airtime) = self.prepare_frame(payload)?;
        if !self.duty.can_transmit(airtime) {
            return Err(LoraMacError::DutyCycleExceeded {
                needed_us: airtime,
                remaining_us: self.duty.remaining_us(),
            });
        }
        let delay = self.csma.contend_for_channel()?;
        self.duty.advance_time(delay);
        self.duty.record_tx(airtime)?;
        Ok((frame, airtime, delay))
    }
}

pub fn init_lora_mac() {
    println!("Initializing LoRa MAC layer with CSMA/CA backoff algorithms.");
    let mut mac = LoraMac::default_sim();
    match mac.transmit(b"mossy-hello".to_vec()) {
        Ok((frame, airtime, delay)) => {
            println!(
                "LoRa sim TX ok: {} bytes, airtime={} us, csma_delay={} us, crc=0x{:04X}",
                frame.payload.len(),
                airtime,
                delay,
                frame.crc
            );
        }
        Err(e) => println!("LoRa sim TX failed: {:?}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_roundtrip_and_detect_corruption() {
        let payload = b"mesh-payload-42".to_vec();
        let frame = LoraFrame::new(payload.clone()).unwrap();
        assert!(frame.verify_crc());
        let mut bad = frame.clone();
        bad.payload[0] ^= 0xFF;
        assert!(!bad.verify_crc());
    }

    #[test]
    fn payload_size_limit() {
        let ok = LoraFrame::new(vec![0u8; LORA_MAX_PAYLOAD]);
        assert!(ok.is_ok());
        let err = LoraFrame::new(vec![0u8; LORA_MAX_PAYLOAD + 1]);
        assert!(matches!(err, Err(LoraMacError::PayloadTooLarge { .. })));
    }

    #[test]
    fn airtime_increases_with_payload_and_sf() {
        let mut cfg = LoraRadioConfig::default();
        cfg.sf = SpreadingFactor::Sf7;
        let t_small = estimate_airtime_us(10, &cfg);
        let t_large = estimate_airtime_us(100, &cfg);
        assert!(t_large > t_small);

        cfg.sf = SpreadingFactor::Sf12;
        let t_sf12 = estimate_airtime_us(10, &cfg);
        cfg.sf = SpreadingFactor::Sf7;
        let t_sf7 = estimate_airtime_us(10, &cfg);
        assert!(t_sf12 > t_sf7);
    }

    #[test]
    fn airtime_is_deterministic() {
        let cfg = LoraRadioConfig::default();
        let a = estimate_airtime_us(42, &cfg);
        let b = estimate_airtime_us(42, &cfg);
        assert_eq!(a, b);
        // Sanity: non-zero and under a minute for SF9/125k/42B
        assert!(a > 10_000);
        assert!(a < 60_000_000);
    }

    #[test]
    fn duty_cycle_blocks_excess_tx() {
        // Tiny window, 10% duty → 10_000 us budget in 100_000 us window
        let mut duty = DutyCycleTracker::with_limit(1000, 100_000);
        duty.record_tx(9_000).unwrap();
        assert!(duty.can_transmit(1_000));
        assert!(!duty.can_transmit(1_001));
        let err = duty.record_tx(2_000);
        assert!(matches!(err, Err(LoraMacError::DutyCycleExceeded { .. })));
    }

    #[test]
    fn duty_cycle_window_resets() {
        let mut duty = DutyCycleTracker::with_limit(1000, 100_000);
        duty.record_tx(10_000).unwrap();
        assert_eq!(duty.remaining_us(), 0);
        duty.advance_time(100_000);
        assert_eq!(duty.remaining_us(), 10_000);
    }

    #[test]
    fn csma_backoff_deterministic_for_seed() {
        let mut a = CsmaCaController::new(12345);
        let mut b = CsmaCaController::new(12345);
        let seq_a: Vec<u64> = (0..8).map(|_| a.random_backoff_us()).collect();
        let seq_b: Vec<u64> = (0..8).map(|_| b.random_backoff_us()).collect();
        assert_eq!(seq_a, seq_b);

        let mut c = CsmaCaController::new(99999);
        let seq_c: Vec<u64> = (0..8).map(|_| c.random_backoff_us()).collect();
        assert_ne!(seq_a, seq_c);
    }

    #[test]
    fn csma_retries_when_busy_then_succeeds() {
        let mut csma = CsmaCaController::new(7);
        csma.channel_busy = true;
        csma.max_retries = 3;
        let delay = csma.contend_for_channel().unwrap();
        assert!(delay >= csma.cca_us as u64);
    }

    #[test]
    fn transmit_path_records_duty_cycle() {
        let mut mac = LoraMac::default_sim();
        let before = mac.duty.on_air_us();
        let (_frame, airtime, _delay) = mac.transmit(b"ping".to_vec()).unwrap();
        assert_eq!(mac.duty.on_air_us(), before + airtime);
    }

    #[test]
    fn symbol_duration_sf7_bw125() {
        let mut cfg = LoraRadioConfig::default();
        cfg.sf = SpreadingFactor::Sf7;
        cfg.bw = Bandwidth::Bw125;
        // 2^7 / 125000 * 1e6 = 1024 us
        assert_eq!(symbol_duration_us(&cfg), 1024);
    }
}
