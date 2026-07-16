from crewai import Task

def create_tasks(agents):
    # Phase 1 Tasks
    transport_task = Task(
        description='Develop the Rust code to translate a smartphone test packet to a LoRa transmission payload based on reticulum-rs logic.',
        expected_output='Rust source code implementing the physical layer transport.',
        agent=agents['lora_engineer']
    )

    dht_task = Task(
        description='Configure libp2p Kademlia DHT for offline pathfinding.',
        expected_output='A working rust module `network.rs` with Kademlia configuration.',
        agent=agents['dht_specialist']
    )

    portal_task = Task(
        description='Implement a modern offline-first React PWA and Nginx configuration for the Captive Portal.',
        expected_output='A built React frontend and dockerized Nginx setup.',
        agent=agents['portal_developer']
    )

    # We can define more tasks for Phase 2-5 here, but to start the autonomous process, 
    # the Lead Architect will review the Phase 1 implementation.
    integration_task = Task(
        description='Review the Phase 1 Transport Layer (LoRa + DHT + Captive Portal) and ensure it meets the MossyMesh Master Blueprint SLAs.',
        expected_output='A verified report confirming Phase 1 integration and determinism.',
        agent=agents['lead_architect']
    )

    return [transport_task, dht_task, portal_task, integration_task]
