//! STUN-less UDP hole punching via deterministic port prediction.
//!
//! Offline mesh nodes cannot reach public STUN servers. Symmetric NATs are
//! handled with a port-prediction sweep; easier NAT classes punch directly.
//! All timing / "socket" effects are injected via traits so unit tests remain
//! deterministic.

use serde::{Deserialize, Serialize};

/// Observed / inferred NAT behaviour for a local interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NatType {
    /// Endpoint-independent mapping & filtering.
    FullCone,
    /// Endpoint-independent mapping, address-dependent filtering.
    Restricted,
    /// Address-and-port-dependent filtering.
    PortRestricted,
    /// Address-and-port-dependent mapping (hardest; needs prediction).
    Symmetric,
    /// Not yet classified.
    Unknown,
}

impl NatType {
    pub fn requires_port_prediction(self) -> bool {
        matches!(self, NatType::Symmetric | NatType::Unknown)
    }
}

/// Hole-punch session state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PunchState {
    Idle,
    /// Gathering local candidates / inferring NAT class.
    Probing,
    /// Generating predicted remote ports (symmetric NAT).
    Predicting,
    /// Sending coordinated UDP probes to peer candidates.
    Punching,
    /// Bidirectional probes succeeded.
    Connected,
    /// Exhausted attempts or timed out.
    Failed,
}

/// A concrete UDP candidate (local or predicted remote).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketAddrLite {
    pub ip: String,
    pub port: u16,
}

impl SocketAddrLite {
    pub fn new(ip: impl Into<String>, port: u16) -> Self {
        Self {
            ip: ip.into(),
            port,
        }
    }
}

/// Configuration for a punch session.
#[derive(Debug, Clone)]
pub struct PunchConfig {
    pub max_attempts: u16,
    pub probe_interval_ms: u64,
    pub timeout_ms: u64,
    /// Ports to sweep outward for symmetric NAT prediction.
    pub prediction_spread: u16,
}

impl Default for PunchConfig {
    fn default() -> Self {
        Self {
            max_attempts: 50,
            probe_interval_ms: 20,
            timeout_ms: 5_000,
            prediction_spread: 50,
        }
    }
}

/// Injectable clock for deterministic tests.
pub trait Clock {
    fn now_ms(&self) -> u64;
}

/// Wall-ish monotonic counter (process-local).
#[derive(Debug, Default)]
pub struct StepClock {
    pub ms: u64,
}

impl StepClock {
    pub fn advance(&mut self, dt: u64) {
        self.ms = self.ms.saturating_add(dt);
    }
}

impl Clock for StepClock {
    fn now_ms(&self) -> u64 {
        self.ms
    }
}

/// NAT heuristic: given a sequence of observed external ports for successive
/// local binds, classify mapping behaviour.
pub trait NatHeuristic {
    fn classify(&self, samples: &[(u16, u16)]) -> NatType;
}

/// Default heuristic:
/// - constant external port across local ports → FullCone-ish
/// - external port == local port → FullCone
/// - external increments lock-step with local → Symmetric
/// - otherwise Restricted (conservative middle ground)
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultNatHeuristic;

impl NatHeuristic for DefaultNatHeuristic {
    fn classify(&self, samples: &[(u16, u16)]) -> NatType {
        if samples.len() < 2 {
            return NatType::Unknown;
        }
        let all_equal_ext = samples.windows(2).all(|w| w[0].1 == w[1].1);
        if all_equal_ext {
            return NatType::FullCone;
        }
        let identity = samples.iter().all(|(loc, ext)| loc == ext);
        if identity {
            return NatType::FullCone;
        }
        // Symmetric: external delta matches local delta (sequential allocator).
        let sequential = samples.windows(2).all(|w| {
            let d_loc = w[1].0.wrapping_sub(w[0].0);
            let d_ext = w[1].1.wrapping_sub(w[0].1);
            d_loc == d_ext && d_loc > 0
        });
        if sequential {
            return NatType::Symmetric;
        }
        NatType::Restricted
    }
}

/// Mockable classifier used in tests.
#[derive(Debug, Clone, Copy)]
pub struct FixedNatHeuristic(pub NatType);

impl NatHeuristic for FixedNatHeuristic {
    fn classify(&self, _samples: &[(u16, u16)]) -> NatType {
        self.0
    }
}

/// Channel that records punch probes (no real sockets).
pub trait PunchTransport {
    /// Send a hole-punch probe toward `remote`. Returns true if a peer response
    /// is observed (simulated success).
    fn send_probe(&mut self, local: &SocketAddrLite, remote: &SocketAddrLite) -> bool;
}

/// Transport that succeeds when remote port is in an allow-list.
#[derive(Debug, Default)]
pub struct MockPunchTransport {
    pub allow_ports: Vec<u16>,
    pub probes: Vec<(u16, u16)>, // (local_port, remote_port)
}

impl PunchTransport for MockPunchTransport {
    fn send_probe(&mut self, local: &SocketAddrLite, remote: &SocketAddrLite) -> bool {
        self.probes.push((local.port, remote.port));
        self.allow_ports.contains(&remote.port)
    }
}

/// Deterministic Symmetric NAT Port Prediction Algorithm.
/// Since offline mesh nodes lack STUN servers, we predict the external port
/// by sequentially scanning a calculated probabilistic spread.
pub fn predict_nat_port(internal_port: u16, attempt: u16) -> u16 {
    // A standard symmetric NAT increments ports sequentially.
    // We scan a spread of up to 50 ports outward from the base prediction.
    let base_prediction = internal_port.wrapping_add(2); // Typical offset

    // Spread: alternate up and down (+0, +1, -1, +2, -2, …)
    let spread: i32 = if attempt == 0 {
        0
    } else if attempt % 2 == 1 {
        ((attempt + 1) / 2) as i32
    } else {
        -((attempt / 2) as i32)
    };

    (base_prediction as i32).wrapping_add(spread) as u16
}

/// Generate `n` predicted remote ports starting from an observed base.
pub fn predicted_ports(base_internal: u16, n: u16) -> Vec<u16> {
    (0..n).map(|i| predict_nat_port(base_internal, i)).collect()
}

/// Active hole-punch session.
#[derive(Debug)]
pub struct HolePunchSession {
    pub state: PunchState,
    pub local_nat: NatType,
    pub remote_nat: NatType,
    pub local: SocketAddrLite,
    pub remote_base: SocketAddrLite,
    pub config: PunchConfig,
    pub attempt: u16,
    pub started_ms: u64,
    pub connected_remote: Option<SocketAddrLite>,
    port_samples: Vec<(u16, u16)>,
    candidates: Vec<SocketAddrLite>,
}

impl HolePunchSession {
    pub fn new(
        local: SocketAddrLite,
        remote_base: SocketAddrLite,
        config: PunchConfig,
        now_ms: u64,
    ) -> Self {
        Self {
            state: PunchState::Idle,
            local_nat: NatType::Unknown,
            remote_nat: NatType::Unknown,
            local,
            remote_base,
            config,
            attempt: 0,
            started_ms: now_ms,
            connected_remote: None,
            port_samples: Vec::new(),
            candidates: Vec::new(),
        }
    }

    pub fn start(&mut self) {
        self.state = PunchState::Probing;
        self.attempt = 0;
        self.connected_remote = None;
        self.candidates.clear();
    }

    /// Record a (local_port, observed_external_port) sample during probing.
    pub fn push_port_sample(&mut self, local_port: u16, external_port: u16) {
        self.port_samples.push((local_port, external_port));
    }

    /// Run one state-machine step. Returns the new state.
    pub fn tick<C: Clock, H: NatHeuristic, T: PunchTransport>(
        &mut self,
        clock: &C,
        heuristic: &H,
        transport: &mut T,
    ) -> PunchState {
        if self.state == PunchState::Connected || self.state == PunchState::Failed {
            return self.state;
        }

        if clock.now_ms().saturating_sub(self.started_ms) > self.config.timeout_ms {
            self.state = PunchState::Failed;
            return self.state;
        }

        match self.state {
            PunchState::Idle => {
                self.start();
            }
            PunchState::Probing => {
                self.local_nat = heuristic.classify(&self.port_samples);
                // Remote NAT may be supplied out-of-band; default Unknown.
                self.state = PunchState::Predicting;
            }
            PunchState::Predicting => {
                self.candidates = self.build_candidates();
                self.state = PunchState::Punching;
                self.attempt = 0;
            }
            PunchState::Punching => {
                if self.attempt as usize >= self.candidates.len()
                    || self.attempt >= self.config.max_attempts
                {
                    self.state = PunchState::Failed;
                    return self.state;
                }
                let remote = self.candidates[self.attempt as usize].clone();
                let ok = transport.send_probe(&self.local, &remote);
                self.attempt = self.attempt.saturating_add(1);
                if ok {
                    self.connected_remote = Some(remote);
                    self.state = PunchState::Connected;
                }
            }
            PunchState::Connected | PunchState::Failed => {}
        }
        self.state
    }

    /// Drive the machine until terminal state or `max_ticks`.
    pub fn run_until_done<H: NatHeuristic, T: PunchTransport>(
        &mut self,
        clock: &mut StepClock,
        heuristic: &H,
        transport: &mut T,
        max_ticks: usize,
    ) -> PunchState {
        if self.state == PunchState::Idle {
            self.started_ms = clock.now_ms();
            self.start();
        }
        for _ in 0..max_ticks {
            let s = self.tick(clock, heuristic, transport);
            if matches!(s, PunchState::Connected | PunchState::Failed) {
                return s;
            }
            clock.advance(self.config.probe_interval_ms);
        }
        if !matches!(self.state, PunchState::Connected | PunchState::Failed) {
            self.state = PunchState::Failed;
        }
        self.state
    }

    fn build_candidates(&self) -> Vec<SocketAddrLite> {
        let ip = self.remote_base.ip.clone();
        if self.local_nat.requires_port_prediction()
            || self.remote_nat.requires_port_prediction()
        {
            predicted_ports(self.remote_base.port, self.config.prediction_spread)
                .into_iter()
                .map(|p| SocketAddrLite::new(ip.clone(), p))
                .collect()
        } else {
            // Easy NAT: punch the advertised port first, then tiny neighborhood.
            let mut ports = vec![self.remote_base.port];
            for i in 1..=4u16 {
                ports.push(self.remote_base.port.wrapping_add(i));
                ports.push(self.remote_base.port.wrapping_sub(i));
            }
            ports
                .into_iter()
                .map(|p| SocketAddrLite::new(ip.clone(), p))
                .collect()
        }
    }
}

/// High-level helper: punch with default heuristic and provided transport.
pub fn punch_hole<T: PunchTransport>(
    local: SocketAddrLite,
    remote: SocketAddrLite,
    local_samples: &[(u16, u16)],
    remote_nat: NatType,
    transport: &mut T,
) -> Result<SocketAddrLite, PunchState> {
    let mut clock = StepClock { ms: 0 };
    let mut session = HolePunchSession::new(local, remote, PunchConfig::default(), 0);
    for &(l, e) in local_samples {
        session.push_port_sample(l, e);
    }
    session.remote_nat = remote_nat;
    let state = session.run_until_done(&mut clock, &DefaultNatHeuristic, transport, 128);
    match state {
        PunchState::Connected => Ok(session.connected_remote.expect("connected")),
        other => Err(other),
    }
}

pub fn init_stun_hole_punch() {
    println!("Initializing STUN-less deterministic port prediction hole punching.");
    let mut transport = MockPunchTransport {
        allow_ports: vec![predict_nat_port(40000, 3)],
        probes: Vec::new(),
    };
    let local = SocketAddrLite::new("10.0.0.2", 40000);
    let remote = SocketAddrLite::new("10.0.0.3", 40000);
    // Symmetric-looking samples: (local, external) advance together.
    let samples = [(40000, 50000), (40001, 50001), (40002, 50002)];
    match punch_hole(local, remote, &samples, NatType::Symmetric, &mut transport) {
        Ok(addr) => println!(
            "Hole punch demo connected via {}:{} ({} probes)",
            addr.ip,
            addr.port,
            transport.probes.len()
        ),
        Err(st) => println!(
            "Hole punch demo ended in {:?} after {} probes",
            st,
            transport.probes.len()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predict_port_spread_is_deterministic() {
        let ports: Vec<u16> = (0..7).map(|i| predict_nat_port(1000, i)).collect();
        // base = 1002; attempts: 0→+0, 1→+1, 2→-1, 3→+2, 4→-2, …
        assert_eq!(ports[0], 1002);
        assert_eq!(ports[1], 1003);
        assert_eq!(ports[2], 1001);
        assert_eq!(ports[3], 1004);
        assert_eq!(ports[4], 1000);
        assert_eq!(predicted_ports(1000, 7), ports);
    }

    #[test]
    fn heuristic_detects_symmetric() {
        let h = DefaultNatHeuristic;
        let samples = [(1000, 2000), (1001, 2001), (1002, 2002)];
        assert_eq!(h.classify(&samples), NatType::Symmetric);
    }

    #[test]
    fn heuristic_detects_full_cone() {
        let h = DefaultNatHeuristic;
        let samples = [(1000, 5555), (1001, 5555), (1002, 5555)];
        assert_eq!(h.classify(&samples), NatType::FullCone);
    }

    #[test]
    fn state_machine_connects_on_allow_list() {
        let mut clock = StepClock { ms: 0 };
        let mut session = HolePunchSession::new(
            SocketAddrLite::new("10.0.0.1", 4000),
            SocketAddrLite::new("10.0.0.2", 5000),
            PunchConfig {
                max_attempts: 20,
                probe_interval_ms: 10,
                timeout_ms: 10_000,
                prediction_spread: 20,
            },
            0,
        );
        // Force symmetric path.
        session.push_port_sample(4000, 6000);
        session.push_port_sample(4001, 6001);
        session.remote_nat = NatType::Symmetric;

        // Allow the port that attempt index 5 predicts from base 5000.
        let winner = predict_nat_port(5000, 5);
        let mut transport = MockPunchTransport {
            allow_ports: vec![winner],
            probes: Vec::new(),
        };

        let state =
            session.run_until_done(&mut clock, &DefaultNatHeuristic, &mut transport, 64);
        assert_eq!(state, PunchState::Connected);
        assert_eq!(session.connected_remote.unwrap().port, winner);
        assert!(!transport.probes.is_empty());
    }

    #[test]
    fn state_machine_fails_when_no_port_matches() {
        let mut clock = StepClock { ms: 0 };
        let mut session = HolePunchSession::new(
            SocketAddrLite::new("10.0.0.1", 4000),
            SocketAddrLite::new("10.0.0.2", 5000),
            PunchConfig {
                max_attempts: 5,
                probe_interval_ms: 10,
                timeout_ms: 10_000,
                prediction_spread: 5,
            },
            0,
        );
        session.remote_nat = NatType::FullCone;
        let mut transport = MockPunchTransport {
            allow_ports: vec![], // never succeeds
            probes: Vec::new(),
        };
        let state =
            session.run_until_done(&mut clock, &FixedNatHeuristic(NatType::FullCone), &mut transport, 32);
        assert_eq!(state, PunchState::Failed);
    }

    #[test]
    fn timeout_fails_session() {
        let clock = StepClock { ms: 0 };
        let mut session = HolePunchSession::new(
            SocketAddrLite::new("10.0.0.1", 1),
            SocketAddrLite::new("10.0.0.2", 2),
            PunchConfig {
                max_attempts: 100,
                probe_interval_ms: 10,
                timeout_ms: 50,
                prediction_spread: 50,
            },
            0,
        );
        session.start();
        // Jump past timeout.
        let late = StepClock { ms: 100 };
        let mut transport = MockPunchTransport::default();
        let state = session.tick(&late, &FixedNatHeuristic(NatType::FullCone), &mut transport);
        assert_eq!(state, PunchState::Failed);
        let _ = clock;
    }

    #[test]
    fn fixed_heuristic_is_mockable() {
        let h = FixedNatHeuristic(NatType::PortRestricted);
        assert_eq!(h.classify(&[]), NatType::PortRestricted);
        assert!(NatType::Symmetric.requires_port_prediction());
        assert!(!NatType::FullCone.requires_port_prediction());
    }
}
