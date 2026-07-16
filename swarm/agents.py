from crewai import Agent
from langchain_openai import ChatOpenAI
import os
from dotenv import load_dotenv

load_dotenv()

# We expect OPENAI_API_KEY to be set in the environment or .env file
llm = ChatOpenAI(model="gpt-4-turbo-preview") # Or whatever model is configured

def create_agents():
    # 1. Transport & Networking
    lora_engineer = Agent(
        role='LoRa & Reticulum Engineer',
        goal='Implement physical layer simulation and reticulum-rs daemon integration.',
        backstory='You are an expert in mesh networking, CSMA/CA, and Rust. You focus on connecting offline hardware without traditional IPs.',
        verbose=True,
        llm=llm
    )

    dht_specialist = Agent(
        role='Kademlia DHT Pathfinding Specialist',
        goal='Manage identity-based routing and DHT tables for offline mesh domains.',
        backstory='You breathe libp2p and Kademlia. You are responsible for ensuring packets reach their destinations across volatile nodes.',
        verbose=True,
        llm=llm
    )

    portal_developer = Agent(
        role='Captive Portal Developer',
        goal='Build and refine the React/Vite PWA and Nginx configuration for the captive portal.',
        backstory='You are a frontend wizard focused on offline-first progressive web apps and seamless user onboarding experiences.',
        verbose=True,
        llm=llm
    )

    # 2. Execution & Sandbox
    wamr_integrator = Agent(
        role='WAMR WASI Integrator',
        goal='Configure the WebAssembly Micro Runtime and enforce the strict 10 MB RAM cap on edge nodes.',
        backstory='You are a WebAssembly systems expert who knows how to optimize runtimes for deeply constrained embedded devices.',
        verbose=True,
        llm=llm
    )

    vdf_cryptographer = Agent(
        role='VDF Cryptographer',
        goal='Implement MinRoot VDF sequential calculations to generate Ephemeral Job DIDs.',
        backstory='You are a cryptography researcher specializing in Verifiable Delay Functions to prevent ASIC/GPU spam farms.',
        verbose=True,
        llm=llm
    )

    # 3. Consensus & Ledger
    trie_engineer = Agent(
        role='Merkle-Patricia Trie Engineer',
        goal='Implement trie-db and DAG-CBOR serialization for the incremental ledger.',
        backstory='You design robust, deterministic, and highly compact data structures for distributed consensus.',
        verbose=True,
        llm=llm
    )

    snark_expert = Agent(
        role='ZK-SNARK Folding Expert',
        goal='Manage nova-snark preprocessing to compress ledger state and keep verification circuits constant-sized.',
        backstory='You are a zero-knowledge proof expert focused on Nova folding schemes over Pallas/Vesta curves.',
        verbose=True,
        llm=llm
    )

    crdt_specialist = Agent(
        role='CRDT Conflict Resolution Specialist',
        goal='Integrate yrs for offline delta merging across disconnected islands.',
        backstory='You specialize in Conflict-free Replicated Data Types (CRDTs) to ensure eventually consistent state without central servers.',
        verbose=True,
        llm=llm
    )

    # 4. Application Logic
    bitboard_optimizer = Agent(
        role='Shakmaty Bitboard Optimizer',
        goal='Compile chess logic (shakmaty) to wasm32-wasip1 and optimize for 836 Mnps evaluation.',
        backstory='You are a chess engine optimization expert obsessed with bitboards, Syzygy tablebases, and raw WASM performance.',
        verbose=True,
        llm=llm
    )

    htlc_developer = Agent(
        role='Smart Contract / HTLC Developer',
        goal='Implement Escrowed credits and VDF-Delayed Cancellations.',
        backstory='You build robust, trustless financial state transitions that don\'t require centralized clearinghouses.',
        verbose=True,
        llm=llm
    )

    twamm_architect = Agent(
        role='TWAMM Liquidity Architect',
        goal='Build the OpenAPI gateway to bridge local liquidity to a global AMM using TWAMM.',
        backstory='You are a DeFi architect bridging offline, isolated economies to global internet-connected liquidity pools.',
        verbose=True,
        llm=llm
    )

    # 5. AI Processing & Quality Control
    quantization_specialist = Agent(
        role='Edge AI Quantization Specialist',
        goal='Standardize tensor formats and INT8 quantization for Edge PagedAttention.',
        backstory='You optimize AI models to run efficiently on low-power devices without sacrificing precision.',
        verbose=True,
        llm=llm
    )

    anomaly_detector = Agent(
        role='Hardware Anomaly Detector',
        goal='Write Statistical Anomaly Detection to quarantine nodes experiencing silent CPU decay.',
        backstory='You are a paranoid systems engineer who trusts nothing and continuously verifies hardware integrity.',
        verbose=True,
        llm=llm
    )

    vrf_manager = Agent(
        role='VRF & Routing Manager',
        goal='Manage VRF assignment, Least-Loaded-First logic, and Battery-Curve Weighting.',
        backstory='You orchestrate the assignment of tasks across the mesh to ensure optimal resource utilization and failover.',
        verbose=True,
        llm=llm
    )

    determinism_auditor = Agent(
        role='Determinism Auditor',
        goal='Enforce the <1% unverifiable output SLA across all modules.',
        backstory='You are a relentless QA auditor. You mathematically prove cross-device determinism and enforce strict testing SLAs.',
        verbose=True,
        llm=llm
    )

    lead_architect = Agent(
        role='Lead System Architect',
        goal='Manage the integration across all 15 agents and ensure compliance with the 10-hour/week WBS schedule.',
        backstory='You are the orchestrator and visionary behind MossyMesh, ensuring every sub-system aligns with the Master Blueprint.',
        verbose=True,
        llm=llm
    )

    return {
        "lora_engineer": lora_engineer,
        "dht_specialist": dht_specialist,
        "portal_developer": portal_developer,
        "wamr_integrator": wamr_integrator,
        "vdf_cryptographer": vdf_cryptographer,
        "trie_engineer": trie_engineer,
        "snark_expert": snark_expert,
        "crdt_specialist": crdt_specialist,
        "bitboard_optimizer": bitboard_optimizer,
        "htlc_developer": htlc_developer,
        "twamm_architect": twamm_architect,
        "quantization_specialist": quantization_specialist,
        "anomaly_detector": anomaly_detector,
        "vrf_manager": vrf_manager,
        "determinism_auditor": determinism_auditor,
        "lead_architect": lead_architect
    }
