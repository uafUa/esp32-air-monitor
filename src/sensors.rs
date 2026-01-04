pub mod dht {
    use anyhow::Result;
    use esp_idf_hal::delay::Ets;
    use esp_idf_hal::gpio::{AnyIOPin, InputOutput, PinDriver, Pull};
    use esp_idf_sys as sys;

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

    pub struct Dht22Sensor<'a> {
        pin: PinDriver<'a, AnyIOPin, InputOutput>,
    }

    impl<'a> Dht22Sensor<'a> {
        pub fn new(mut pin: PinDriver<'a, AnyIOPin, InputOutput>) -> Result<Self> {
            pin.set_pull(Pull::Up)?;
            pin.set_high()?;
            Ok(Self { pin })
        }

        pub fn read(&mut self) -> core::result::Result<DhtReading, DhtError> {
            self.pin.set_low().map_err(DhtError::Gpio)?;
            Ets::delay_ms(2);
            self.pin.set_high().map_err(DhtError::Gpio)?;
            Ets::delay_us(30);

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
                    value <<= 1;
                    if high_len > 50 {
                        value |= 1;
                    }
                }
                *byte = value;
            }

            let checksum = ((data[0] as u16 + data[1] as u16 + data[2] as u16 + data[3] as u16) & 0xFF) as u8;
            if checksum != data[4] {
                return Err(DhtError::Checksum);
            }

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

    fn now_us() -> i64 {
        unsafe { sys::esp_timer_get_time() }
    }
}
