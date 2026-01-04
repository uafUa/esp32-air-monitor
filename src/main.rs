#![allow(clippy::needless_return)]

mod lcd;
mod sensors;
mod touch;

use crate::lcd::{render_ui_mock1, Jd9853, LCD_H, LCD_W};
use crate::sensors::dht::{Dht22Sensor, DHT_GPIO};
use crate::touch::{gpio_setup_touch_lines, i2c_scan, probe_touch, read_touch, touch_reset_pulse};

use anyhow::Result;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use esp_idf_hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::prelude::*;
use esp_idf_hal::spi::config::{Config as SpiDeviceConfig, DriverConfig as SpiDriverConfig};
use esp_idf_hal::spi::{SpiDeviceDriver, SpiDriver};
use esp_idf_svc::log::EspLogger;
use log::{error, info};
use std::thread;
use std::time::{Duration, Instant};

use esp_idf_sys as sys;

fn main() -> Result<()> {
    sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    // ---- Touch/I2C bring-up ----
    gpio_setup_touch_lines();
    touch_reset_pulse();

    let i2c_cfg = I2cConfig::new().baudrate(100.kHz().into());
    let mut i2c = I2cDriver::new(peripherals.i2c0, pins.gpio18, pins.gpio19, &i2c_cfg)?;

    i2c_scan(&mut i2c);
    if let Err(e) = probe_touch(&mut i2c) {
        error!("Touch probe failed: {:?}", e);
    }

    info!("DHT22 sensor on GPIO{}", DHT_GPIO);
    let dht_pin = PinDriver::input_output_od(AnyIOPin::from(pins.gpio4))?;
    let mut dht22 = Dht22Sensor::new(dht_pin)?;
    let dht_interval = Duration::from_millis(2000);
    let mut last_dht_read = Instant::now() - dht_interval;

    // ---- LCD bring-up (SPI) ----
    let spi_driver_cfg = SpiDriverConfig::new();
    let spi_dev_cfg = SpiDeviceConfig::new().baudrate(40.MHz().into());

    let spi_driver = SpiDriver::new(
        peripherals.spi2,
        pins.gpio1, // SCLK
        pins.gpio2, // MOSI
        Option::<esp_idf_hal::gpio::Gpio0>::None, // MISO not used
        &spi_driver_cfg,
    )?;

    let spi_dev = SpiDeviceDriver::new(
        spi_driver,
        Some(pins.gpio14),
        &spi_dev_cfg,
    )?;

    let dc = PinDriver::output(AnyIOPin::from(pins.gpio15))?;
    let rst = PinDriver::output(AnyIOPin::from(pins.gpio22))?;
    let bl = PinDriver::output(AnyIOPin::from(pins.gpio23))?;

    let mut lcd = Jd9853::new(spi_dev, dc, rst, bl)?;

    // ---- Framebuffer ----
    let mut frame: Vec<Rgb565> = vec![Rgb565::BLACK; LCD_W * LCD_H];

    // Live sensor readings + fallback animated CO2 value
    let mut temperature_c: f32 = 21.6;
    let mut humidity_pct: u8 = 45;
    let mut co2: u16 = 840;
    let mut co2_phase = 0.0f32;

    loop {
        if last_dht_read.elapsed() >= dht_interval {
            match dht22.read() {
                Ok(reading) => {
                    temperature_c = reading.temperature_c;
                    humidity_pct = reading.humidity_pct.clamp(0.0, 100.0).round() as u8;
                }
                Err(err) => {
                    error!("DHT22 read error: {:?}", err);
                }
            }
            last_dht_read = Instant::now();
        }

        match read_touch(&mut i2c) {
            Ok(Some((x, y))) => info!("Touch: x={} y={}", x, y),
            Ok(None) => {}
            Err(e) => error!("Touch read error: {:?}", e),
        }

        render_ui_mock1(&mut frame, temperature_c, humidity_pct, co2)?;
        lcd.flush_full(&frame)?;

        // Fake CO2 animation until that sensor is wired in
        co2_phase += 0.02;
        if co2_phase > 20.0 {
            co2_phase = 0.0;
        }
        co2 = 700 + (((co2_phase * 50.0) as u16) % 600);

        thread::sleep(Duration::from_millis(200));
    }
}
