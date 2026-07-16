use log::info;
use tokio::time::{sleep, Duration};

pub async fn simulate_lora_transmission(data: &[u8], target: &str) {
    info!("Transmitting {} bytes via simulated LoRa to node: {}", data.len(), target);
    
    // Simulate CSMA/CA backoff delay
    sleep(Duration::from_millis(150)).await;
    
    info!("LoRa transmission successful!");
}
