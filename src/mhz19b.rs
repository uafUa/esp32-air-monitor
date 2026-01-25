use core::fmt;
use std::time::{Duration, Instant};

use esp_idf_hal::delay::{TickType, BLOCK};
use esp_idf_hal::gpio::{InputPin, OutputPin};
use esp_idf_hal::peripheral::Peripheral;
use esp_idf_hal::prelude::*;
use esp_idf_hal::uart::{UartConfig, UartDriver};
use log::{debug, error};

pub const MHZ19B_BAUD: u32 = 9_600;

#[derive(Debug)]
pub enum MhzError {
    Timeout,
    Frame,
    Checksum,
    Uart(esp_idf_hal::sys::EspError),
}

impl fmt::Display for MhzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "timeout waiting for MH-Z19B frame"),
            Self::Frame => write!(f, "invalid MH-Z19B frame header"),
            Self::Checksum => write!(f, "MH-Z19B checksum mismatch"),
            Self::Uart(err) => write!(f, "UART error: {err}"),
        }
    }
}

impl std::error::Error for MhzError {}

pub struct Mhz19b<'a> {
    uart: UartDriver<'a>,
}

impl<'a> Mhz19b<'a> {
    pub fn new(uart: UartDriver<'a>) -> Self {
        Self { uart }
    }

    pub fn read_ppm(&mut self, timeout_ms: u64) -> Result<u16, MhzError> {
        let (ppm, _frame) = self.read_ppm_with_frame(timeout_ms)?;
        Ok(ppm)
    }

    pub fn read_ppm_with_frame(&mut self, timeout_ms: u64) -> Result<(u16, [u8; 9]), MhzError> {
        let frame = self.read_frame(timeout_ms)?;
        let ppm = (u16::from(frame[2]) << 8) | u16::from(frame[3]);
        Ok((ppm, frame))
    }

    pub fn calibrate_zero(&mut self) -> Result<(), MhzError> {
        let mut cmd = [0xFFu8, 0x01, 0x87, 0, 0, 0, 0, 0, 0];
        cmd[8] = checksum(&cmd[1..8]);
        self.uart.write(&cmd).map_err(MhzError::Uart)?;
        self.uart.wait_tx_done(BLOCK).map_err(MhzError::Uart)?;
        Ok(())
    }

    pub fn set_abc(&mut self, enabled: bool) -> Result<(), MhzError> {
        // ABC (automatic baseline correction) enable/disable command.
        let abc = if enabled { 0xA0 } else { 0x00 };
        let mut cmd = [0xFFu8, 0x01, 0x79, abc, 0, 0, 0, 0, 0];
        cmd[8] = checksum(&cmd[1..8]);
        self.uart.write(&cmd).map_err(MhzError::Uart)?;
        self.uart.wait_tx_done(BLOCK).map_err(MhzError::Uart)?;
        Ok(())
    }

    pub fn reinit_uart(&mut self) -> Result<(), MhzError> {
        self.uart.clear_rx().map_err(MhzError::Uart)?;
        self.uart
            .change_baudrate(MHZ19B_BAUD.Hz())
            .map_err(MhzError::Uart)?;
        Ok(())
    }

    pub fn uart_mut(&mut self) -> &mut UartDriver<'a> {
        &mut self.uart
    }

    fn read_frame(&mut self, timeout_ms: u64) -> Result<[u8; 9], MhzError> {
        let mut cmd = [0xFFu8, 0x01, 0x86, 0, 0, 0, 0, 0, 0];
        cmd[8] = checksum(&cmd[1..8]);

        self.uart.write(&cmd).map_err(MhzError::Uart)?;
        self.uart.wait_tx_done(BLOCK).map_err(MhzError::Uart)?;

        let mut buf = [0u8; 9];
        let mut received = 0usize;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(1));

        while received < buf.len() && Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = TickType::new_millis(remaining.as_millis() as u64).ticks();
            let n = self
                .uart
                .read(&mut buf[received..], timeout)
                .map_err(MhzError::Uart)?;
            if n == 0 {
                continue;
            }
            received += n;
        }

        if received < buf.len() {
            error!(
                "MH-Z19B timeout: got {}/9 bytes: {:02X?}",
                received,
                &buf[..received]
            );
            return Err(MhzError::Timeout);
        }

        debug!("MH-Z19B frame: {:02X?}", buf);
        if buf[0] != 0xFF || buf[1] != 0x86 {
            error!("MH-Z19B frame header mismatch: {:02X?}", buf);
            return Err(MhzError::Frame);
        }

        let expected = checksum(&buf[1..8]);
        if buf[8] != expected {
            error!(
                "MH-Z19B checksum mismatch: expected {:02X}, got {:02X}, frame {:02X?}",
                expected, buf[8], buf
            );
            return Err(MhzError::Checksum);
        }

        Ok(buf)
    }
}

pub fn init_mhz19b<'d>(
    uart: impl Peripheral<P = esp_idf_hal::uart::UART0> + 'd,
    tx: impl Peripheral<P = impl OutputPin> + 'd,
    rx: impl Peripheral<P = impl InputPin> + 'd,
) -> Result<Mhz19b<'d>, MhzError> {
    let uart_cfg = UartConfig::new().baudrate(MHZ19B_BAUD.Hz());
    let uart = UartDriver::new(
        uart,
        tx,
        rx,
        None::<esp_idf_hal::gpio::AnyIOPin>,
        None::<esp_idf_hal::gpio::AnyIOPin>,
        &uart_cfg,
    )
    .map_err(MhzError::Uart)?;

    Ok(Mhz19b::new(uart))
}

fn checksum(bytes: &[u8]) -> u8 {
    let sum: u16 = bytes.iter().map(|b| *b as u16).sum();
    (0xFFu16 - (sum & 0xFF) + 1) as u8
}
