use libp2p::{
    kad::{self, store::MemoryStore, KademliaEvent},
    swarm::NetworkBehaviour,
};

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "OutEvent")]
pub struct MeshBehaviour {
    pub kademlia: kad::Kademlia<MemoryStore>,
}

#[derive(Debug)]
pub enum OutEvent {
    Kademlia(KademliaEvent),
}

impl From<KademliaEvent> for OutEvent {
    fn from(event: KademliaEvent) -> Self {
        OutEvent::Kademlia(event)
    }
}
