pub mod dht {
    use anyhow::Result;
    use esp_idf_hal::delay::Ets;
    use esp_idf_hal::gpio::{InputPin, InputOutput, OutputPin, PinDriver, Pull};
    use esp_idf_hal::peripheral::Peripheral;
    use esp_idf_sys as sys;

    // DHT22 data line GPIO. Must have a pull-up (internal or external ~4.7k).
    pub const DHT_GPIO: i32 = 4;

    #[derive(Debug, Clone, Copy)]
    pub struct DhtReading {
        pub temperature_c: f32,
        pub humidity_pct: f32,
    }

    #[derive(Debug)]
    pub enum DhtError {
        Timeout(&'static str),
        Checksum,
        Gpio(esp_idf_hal::sys::EspError),
    }

    pub struct Dht22Sensor<'a, P>
    where
        P: InputPin + OutputPin,
    {
        pin: PinDriver<'a, P, InputOutput>,
    }

    impl<'a, P> Dht22Sensor<'a, P>
    where
        P: InputPin + OutputPin,
    {
        // DHT22 requires open-drain I/O with pull-up; we drive low and release high.
        pub fn new(mut pin: PinDriver<'a, P, InputOutput>) -> Result<Self> {
            pin.set_pull(Pull::Up)?;
            pin.set_high()?;
            Ok(Self { pin })
        }

        // Bit-bang the DHT22 single-wire protocol using precise microsecond timing.
        pub fn read(&mut self) -> core::result::Result<DhtReading, DhtError> {
            self.pin.set_low().map_err(DhtError::Gpio)?;
            // Start signal: keep low for at least 1ms (2ms used here).
            Ets::delay_ms(2);
            self.pin.set_high().map_err(DhtError::Gpio)?;
            // Release line and wait ~20–40us before sensor response.
            Ets::delay_us(30);

            // Sensor response sequence: ~80us low, ~80us high, then data.
            self.wait_for_level(false, 200, "response low")?;
            self.wait_for_level(true, 200, "response high")?;
            self.wait_for_level(false, 200, "data preamble")?;

            let mut data = [0u8; 5];
            for byte in data.iter_mut() {
                let mut value = 0u8;
                for _ in 0..8 {
                    self.wait_for_level(true, 80, "bit high")?;
                    let start = now_us();
                    self.wait_for_level(false, 120, "bit low")?;
                    let high_len = now_us() - start;
                    // High pulse ~26–28us => 0, ~70us => 1.
                    value <<= 1;
                    if high_len > 50 {
                        value |= 1;
                    }
                }
                *byte = value;
            }

            // Checksum is the low byte of the sum of the first 4 data bytes.
            let checksum = ((data[0] as u16 + data[1] as u16 + data[2] as u16 + data[3] as u16) & 0xFF) as u8;
            if checksum != data[4] {
                return Err(DhtError::Checksum);
            }

            // DHT22 format: 16-bit humidity (0.1% RH), 16-bit temp (0.1C, sign bit).
            let raw_humidity = u16::from(data[0]) << 8 | u16::from(data[1]);
            let raw_temp = u16::from(data[2]) << 8 | u16::from(data[3]);

            let humidity = raw_humidity as f32 / 10.0;
            let mut temperature = (raw_temp & 0x7FFF) as f32 / 10.0;
            if raw_temp & 0x8000 != 0 {
                temperature = -temperature;
            }

            Ok(DhtReading {
                temperature_c: temperature,
                humidity_pct: humidity,
            })
        }

        fn wait_for_level(
            &mut self,
            high: bool,
            timeout_us: u32,
            stage: &'static str,
        ) -> core::result::Result<(), DhtError> {
            let deadline = now_us() + timeout_us as i64;
            while now_us() <= deadline {
                if self.pin.is_high() == high {
                    return Ok(());
                }
            }
            Err(DhtError::Timeout(stage))
        }
    }

    pub fn init_dht22<'d, P>(
        pin: impl Peripheral<P = P> + 'd,
    ) -> Result<Dht22Sensor<'d, P>>
    where
        P: InputPin + OutputPin,
    {
        let dht_pin = PinDriver::input_output_od(pin)?;
        Dht22Sensor::new(dht_pin)
    }

    // ESP timer in microseconds for tight pulse timing.
    fn now_us() -> i64 {
        unsafe { sys::esp_timer_get_time() }
    }
}

pub mod mhz19b {
    use core::fmt;
    use esp_idf_hal::delay::{TickType, BLOCK};
    use esp_idf_hal::uart::UartDriver;
    use esp_idf_hal::gpio::{InputPin, OutputPin};
    use esp_idf_hal::peripheral::Peripheral;
    use esp_idf_hal::uart::UartConfig;
    use esp_idf_hal::prelude::*;
    use std::time::{Duration, Instant};

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
                return Err(MhzError::Timeout);
            }
            if buf[0] != 0xFF || buf[1] != 0x86 {
                return Err(MhzError::Frame);
            }

            let expected = checksum(&buf[1..8]);
            if buf[8] != expected {
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
}
