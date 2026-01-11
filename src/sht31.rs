use std::thread;
use std::time::Duration;

use esp_idf_hal::i2c::I2cDriver;

#[derive(Debug, Clone, Copy)]
pub struct ShtReading {
    pub temperature_c: f32,
    pub humidity_pct: f32,
}

#[derive(Debug)]
pub enum ShtError {
    I2c(esp_idf_hal::sys::EspError),
    Crc,
}

pub struct Sht31 {
    addr: u8,
}

impl Sht31 {
    pub const DEFAULT_ADDR: u8 = 0x44;

    pub fn new(addr: u8) -> Self {
        Self { addr }
    }

    pub fn new_default() -> Self {
        Self::new(Self::DEFAULT_ADDR)
    }

    pub fn read(&self, i2c: &mut I2cDriver<'_>) -> Result<ShtReading, ShtError> {
        // Single-shot, high repeatability, no clock stretching.
        // Command: 0x24 0x00 (datasheet).
        let cmd = [0x24, 0x00];
        i2c.write(self.addr, &cmd, esp_idf_hal::delay::BLOCK)
            .map_err(ShtError::I2c)?;

        // Measurement time up to ~15ms for high repeatability.
        thread::sleep(Duration::from_millis(15));

        let mut data = [0u8; 6];
        i2c.read(self.addr, &mut data, esp_idf_hal::delay::BLOCK)
            .map_err(ShtError::I2c)?;

        if crc8(&data[0..2]) != data[2] || crc8(&data[3..5]) != data[5] {
            return Err(ShtError::Crc);
        }

        let raw_temp = u16::from_be_bytes([data[0], data[1]]);
        let raw_rh = u16::from_be_bytes([data[3], data[4]]);

        let temperature = -45.0 + 175.0 * (raw_temp as f32) / 65535.0;
        let humidity = 100.0 * (raw_rh as f32) / 65535.0;

        Ok(ShtReading {
            temperature_c: temperature,
            humidity_pct: humidity,
        })
    }
}

fn crc8(bytes: &[u8]) -> u8 {
    // CRC-8 with polynomial 0x31, init 0xFF (Sensirion standard).
    let mut crc = 0xFFu8;
    for byte in bytes {
        crc ^= *byte;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ 0x31;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}
