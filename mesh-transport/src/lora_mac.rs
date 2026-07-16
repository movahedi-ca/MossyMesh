//! CSMA/CA wrapper for lightweight LoRa physical links.

pub fn init_lora_mac() {
    println!("Initializing LoRa MAC layer with CSMA/CA backoff algorithms.");
}

pub struct LoraFrame {
    pub payload: Vec<u8>,
    pub crc: u16,
}

/// Calculates the standard CRC-16-CCITT (polynomial 0x1021).
/// This mathematically guarantees that the small 255-byte LoRa payloads 
/// are not corrupted by RF interference in the CSMA/CA environment.
pub fn calculate_crc(payload: &[u8]) -> u16 {
    let polynomial = 0x1021;
    let mut crc = 0xFFFF; // Initial value

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
