#![allow(clippy::needless_return)]

mod lcd;
mod sensors;
mod touch;

use crate::lcd::{co2_card_rect, render_ui_mock1, Jd9853, LCD_H, LCD_W};
use crate::sensors::dht::{Dht22Sensor, DHT_GPIO};
use crate::sensors::mhz19b::{Mhz19b, MHZ19B_BAUD};
use crate::touch::{gpio_setup_touch_lines, i2c_scan, probe_touch, read_touch, touch_reset_pulse};

use anyhow::Result;
use embedded_graphics::geometry::Point;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use esp_idf_hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::ledc::config::TimerConfig as LedcTimerConfig;
use esp_idf_hal::ledc::{LedcDriver, LedcTimerDriver};
use esp_idf_hal::prelude::*;
use esp_idf_hal::spi::config::{Config as SpiDeviceConfig, DriverConfig as SpiDriverConfig};
use esp_idf_hal::spi::{SpiDeviceDriver, SpiDriver};
use esp_idf_hal::uart::{UartConfig, UartDriver};
use esp_idf_svc::log::{set_target_level, EspLogger};
use log::{error, info, LevelFilter};
use std::thread;
use std::time::{Duration, Instant};

use esp_idf_sys as sys;

fn main() -> Result<()> {
    sys::link_patches();
    EspLogger::initialize_default();
    // Note: UART0 TX/RX are used for MH-Z19B on this board; logging over UART0
    // will share the line with the sensor. Disable logs if that causes issues.
    log::set_max_level(LevelFilter::Debug);
    set_target_level("c6_demo", LevelFilter::Debug)?;
    set_target_level("c6_demo::sensors", LevelFilter::Debug)?;

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    // ---- Touch/I2C bring-up (GPIO18/19 + reset/interrupt lines) ----
    gpio_setup_touch_lines();
    touch_reset_pulse();

    let i2c_cfg = I2cConfig::new().baudrate(100.kHz().into());
    let mut i2c = I2cDriver::new(peripherals.i2c0, pins.gpio18, pins.gpio19, &i2c_cfg)?;

    i2c_scan(&mut i2c);
    if let Err(e) = probe_touch(&mut i2c) {
        error!("Touch probe failed: {:?}", e);
    }

    // DHT22 on a single-wire GPIO with pull-up; read every ~2s.
    info!("DHT22 sensor on GPIO{}", DHT_GPIO);
    let dht_pin = PinDriver::input_output_od(AnyIOPin::from(pins.gpio4))?;
    let mut dht22 = Dht22Sensor::new(dht_pin)?;
    let dht_interval = Duration::from_millis(2000);
    let mut last_dht_read = Instant::now() - dht_interval;

    // MH-Z19B CO2 sensor on UART0 (TXD=GPIO16, RXD=GPIO17).
    let uart_cfg = UartConfig::new().baudrate(MHZ19B_BAUD.Hz());
    let uart = UartDriver::new(
        peripherals.uart0,
        pins.gpio16,
        pins.gpio17,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &uart_cfg,
    )?;
    let mut mhz19b = Mhz19b::new(uart);
    let mhz_interval = Duration::from_millis(5000);
    let mut last_mhz_read = Instant::now() - mhz_interval;

    // ---- LCD bring-up (SPI2: SCLK=GPIO1, MOSI=GPIO2, CS=GPIO14) ----
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

    // LCD control pins: D/C, RESET. Backlight uses PWM on GPIO23.
    let dc = PinDriver::output(AnyIOPin::from(pins.gpio15))?;
    let rst = PinDriver::output(AnyIOPin::from(pins.gpio22))?;
    let bl_timer = LedcTimerDriver::new(
        peripherals.ledc.timer0,
        &LedcTimerConfig::default().frequency(5.kHz().into()),
    )?;
    let bl_pwm = LedcDriver::new(peripherals.ledc.channel0, &bl_timer, pins.gpio23)?;

    let mut lcd = Jd9853::new(spi_dev, dc, rst, bl_pwm, bl_timer)?;
    lcd.set_brightness(10)?;

    // ---- Framebuffer ----
    let mut frame: Vec<Rgb565> = vec![Rgb565::BLACK; LCD_W * LCD_H];

    // Live sensor readings + fallback animated CO2 value
    let mut temperature_c: f32 = 21.6;
    let mut humidity_pct: u8 = 45;
    let mut co2: u16 = 840;

    let co2_rect = co2_card_rect();
    let hold_duration = Duration::from_secs(2);
    let zero_feedback_duration = Duration::from_secs(3);
    let mut co2_hold_start: Option<Instant> = None;
    let mut co2_hold_triggered = false;
    let mut zero_feedback_until: Option<Instant> = None;

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

        if last_mhz_read.elapsed() >= mhz_interval {
            match mhz19b.read_ppm_with_frame(2000) {
                Ok((ppm, _frame)) => {
                    co2 = ppm;
                }
                Err(err) => error!("MH-Z19B read error: {:?}", err),
            }
            last_mhz_read = Instant::now();
        }

        let touch_in_co2 = 
        match read_touch(&mut i2c) {
            Ok(Some((x, y))) => {
                let pt = touch_to_view(x, y);
                co2_rect.contains(pt)
            }
            Ok(None) => { false } 
            Err(_) => { false }
        };

        if let Some(until) = zero_feedback_until {
            if Instant::now() >= until {
                zero_feedback_until = None;
            }
        }

        if touch_in_co2 {
            if co2_hold_start.is_none() {
                co2_hold_start = Some(Instant::now());
            }
            if !co2_hold_triggered {
                if let Some(start) = co2_hold_start {
                    if start.elapsed() >= hold_duration {
                        co2_hold_triggered = true;
                        if let Err(err) = mhz19b.calibrate_zero() {
                            error!("MH-Z19B zero calibration failed: {:?}", err);
                        }
                        zero_feedback_until = Some(Instant::now() + zero_feedback_duration);
                    }
                }
            }
        } else {
            co2_hold_start = None;
            co2_hold_triggered = false;
        }

        let zero_mode = zero_feedback_until.is_some();
        render_ui_mock1(&mut frame, temperature_c, humidity_pct, co2, zero_mode)?;
        lcd.flush_full(&frame)?;

        thread::sleep(Duration::from_millis(200));
    }
}

fn touch_to_view(x: u16, y: u16) -> Point {
    // The UI is rendered in landscape (320x172) by rotating the framebuffer.
    // Touch controller reports the native panel coordinates (172x320).
    let x_p = x as i32;
    let y_p = y as i32;
    let view_x = y_p;
    let view_y = (LCD_W as i32 - 1) - x_p;
    Point::new(view_x, view_y)
}
