#![allow(clippy::needless_return)]

mod board;
mod display;
mod sht31;
mod st7789;
mod mhz19b;
mod touch;

use crate::board::Board;
use crate::display::{co2_card_rect, render_ui_mock1};
use crate::st7789::{LCD_H, LCD_W};
use crate::touch::read_touch;

use anyhow::Result;
use embedded_graphics::geometry::Point;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use esp_idf_svc::log::{set_target_level, EspLogger};
use log::{error, LevelFilter};
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

    let Board {
        mut lcd,
        mut i2c,
        mut mhz19b,
        sht31,
    } = Board::init()?;
    let env_interval = Duration::from_millis(2000);
    let mut last_env_read = Instant::now() - env_interval;
    let mhz_interval = Duration::from_millis(5000);
    let mut last_mhz_read = Instant::now() - mhz_interval;

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
    const DISPLAY_OFF_TIMEOUT: Duration = Duration::from_secs(5); // timeout aftter which displays starts reducing brightness
    const DISPLAY_OFF_DURATION: Duration = Duration::from_secs(2); // duration for which display reduces brightness
    const DEFAULT_BRIGHTNESS: u8 = 10;
    const SLEEP_INTERFVAL: Duration = Duration::from_millis(200);
    lcd.set_brightness(100)?;
    let mut last_touch = Instant::now();
    let dimming_step: u8 = 1; // brightness change step during dimming/brightening
    let mut dimming_in_progress = false;
    let mut dimmed_brightness : u8 = DEFAULT_BRIGHTNESS; // current dimmed brightness level (if display dimming in progress)


    loop {

        if dimming_in_progress && dimmed_brightness > 0 {
            dimmed_brightness -= dimming_step;
            lcd.set_brightness(dimmed_brightness)?;
        }

        if last_env_read.elapsed() >= env_interval {
            match sht31.read(&mut i2c) {
                Ok(reading) => {
                    temperature_c = reading.temperature_c;
                    humidity_pct = reading.humidity_pct.clamp(0.0, 100.0).round() as u8;
                }
                Err(err) => {
                    error!("SHT31 read error: {:?}", err);
                }
            }
            last_env_read = Instant::now();
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
                // cancel display dimming/brightening if touch detected
                if dimming_in_progress {
                    log::info!("Touch detected - restoring brightness to {}%", DEFAULT_BRIGHTNESS);
                    dimming_in_progress = false;
                    lcd.set_brightness(DEFAULT_BRIGHTNESS)?;
                    dimmed_brightness = DEFAULT_BRIGHTNESS;
                }
                last_touch = Instant::now();

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

        if last_touch.elapsed() >= DISPLAY_OFF_TIMEOUT && !dimming_in_progress {
            dimming_in_progress = true;
        }

        if dimmed_brightness != 0 {
            let zero_mode = zero_feedback_until.is_some();
            render_ui_mock1(&mut frame, temperature_c, humidity_pct, co2, zero_mode)?;
            lcd.flush_full(&frame)?;
        }

        thread::sleep(SLEEP_INTERFVAL);
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
