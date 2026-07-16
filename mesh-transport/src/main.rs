mod network;
mod simulation;

use anyhow::Result;
use libp2p::{
    core::upgrade,
    identity,
    kad::{store::MemoryStore, Kademlia, KademliaConfig},
    noise,
    swarm::{SwarmBuilder, SwarmEvent},
    tcp, yamux, PeerId, Transport,
};
use log::{error, info};
use std::time::Duration;
use tokio::time::sleep;

use network::{MeshBehaviour, OutEvent};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    info!("Starting MossyMesh Transport Daemon (Phase 1)...");

    // 1. Generate identity
    let id_keys = identity::Keypair::generate_ed25519();
    let peer_id = PeerId::from(id_keys.public());
    info!("Local Peer ID: {}", peer_id);

    // 2. Setup transport
    let tcp_transport = tcp::tokio::Transport::default();
    let transport = tcp_transport
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::Config::new(&id_keys).expect("Signing libp2p-noise static keypair"))
        .multiplex(yamux::Config::default())
        .boxed();

    // 3. Setup Kademlia DHT
    let store = MemoryStore::new(peer_id);
    let mut kad_config = KademliaConfig::default();
    kad_config.set_query_timeout(Duration::from_secs(5 * 60));
    let kademlia = Kademlia::with_config(peer_id, store, kad_config);

    // 4. Create Swarm
    let behaviour = MeshBehaviour { kademlia };
    let mut swarm = SwarmBuilder::with_tokio_executor(transport, behaviour, peer_id).build();

    // Listen on all interfaces
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Spawn a simulation loop to demonstrate Phase 1 requirement
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(10)).await;
            simulation::simulate_lora_transmission(
                b"smartphone test packet",
                "simulated_destination",
            ).await;
        }
    });

    // Event Loop
    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("Listening on {:?}", address);
            }
            SwarmEvent::Behaviour(OutEvent::Kademlia(e)) => {
                info!("Kademlia event: {:?}", e);
            }
            _ => {}
        }
    }
}
