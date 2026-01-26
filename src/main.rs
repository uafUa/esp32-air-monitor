#![allow(clippy::needless_return)]

mod board;
mod battery;
mod display;
mod ota;
mod sht31;
mod st7789;
mod mhz19b;
mod mqtt;
mod touch;
mod wifi;

use crate::board::Board;
use crate::display::{co2_card_rect, render_ui_mock1};
use crate::mqtt::{Command as MqttCommand, Telemetry as MqttTelemetry};
use crate::ota::{check_and_update, mark_app_valid, OTA_CHECK_INTERVAL};
use crate::st7789::{LCD_H, LCD_W};
use crate::touch::{read_touch, touch_take_pending};

use anyhow::Result;
use embedded_graphics::geometry::Point;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use esp_idf_svc::log::{set_target_level, EspLogger};
use esp_idf_svc::sys::esp_restart;
use log::{error, info, warn, LevelFilter};
use std::thread;
use std::time::{Duration, Instant};

use esp_idf_sys as sys;

fn main() -> Result<()> {
    sys::link_patches();
    EspLogger::initialize_default();
    // Note: UART0 TX/RX are used for MH-Z19B on this board; logging over UART0
    // will share the line with the sensor. Disable logs if that causes issues.
    let log_level = if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    log::set_max_level(log_level);
    let global_level = if cfg!(debug_assertions) {
        LevelFilter::Info
    } else {
        LevelFilter::Info
    };
    if let Err(err) = set_target_level("*", global_level) {
        warn!("Failed to set log level: {:?}", err);
    }
    if let Err(err) = set_target_level("c6_demo", LevelFilter::Debug) {
        warn!("Failed to set log level for c6_demo: {:?}", err);
    }
    if let Err(err) = set_target_level("c6_demo::mhz19b", LevelFilter::Debug) {
        warn!("Failed to set log level for c6_demo::mhz19b: {:?}", err);
    }
    if let Err(err) = set_target_level("adc_hal", LevelFilter::Off) {
        warn!("Failed to set log level for adc_hal: {:?}", err);
    }

    if let Err(err) = run() {
        error!("Fatal error, exiting main loop: {:?}", err);
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
    Ok(())
}

fn run() -> Result<()> {
    log::info!("App start");
    let Board {
        mut lcd,
        mut i2c,
        mut mhz19b,
        mut battery,
        sht31,
        mut wifi,
    } = Board::init()?;
    if let Err(err) = mark_app_valid() {
        warn!("OTA mark-running-valid failed: {:?}", err);
    }
    let mut mqtt = match wifi.as_mut() {
        Some(wifi) => match mqtt::init_mqtt(wifi) {
            Ok(client) => Some(client),
            Err(err) => {
                warn!("MQTT init failed: {:?}", err);
                None
            }
        },
        None => None,
    };
    let env_interval = Duration::from_millis(2000);
    let mut last_env_read = Instant::now() - env_interval;
    let mhz_interval = Duration::from_millis(5000);
    let mut last_mhz_read = Instant::now() - mhz_interval;
    const MHZ_ERR_REINIT_THRESHOLD: u8 = 3;
    let mut mhz_error_count: u8 = 0;
    let battery_interval = Duration::from_millis(10000);
    let mut last_battery_read = Instant::now() - battery_interval;
    let mut last_ota_check = Instant::now() - OTA_CHECK_INTERVAL;

    // ---- Framebuffer ----
    let mut frame: Vec<Rgb565> = vec![Rgb565::BLACK; LCD_W * LCD_H];

    // Live sensor readings.
    let mut temperature_c: Option<f32> = None;
    let mut humidity_pct: Option<u8> = None;
    let mut co2_value: Option<u16> = None;
    let mut co2_error = false;
    let mut battery_v: Option<f32> = None;

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
    const MQTT_PUBLISH_INTERVAL: Duration = Duration::from_secs(10);
    lcd.set_brightness(DEFAULT_BRIGHTNESS)?;
    let mut last_touch = Instant::now();
    let dimming_steps =
        (DISPLAY_OFF_DURATION.as_millis() / SLEEP_INTERFVAL.as_millis()).max(1) as u32;
    let dimming_step =
        ((DEFAULT_BRIGHTNESS as u32 + dimming_steps - 1) / dimming_steps) as u8;
    let mut dimming_in_progress = false;
    let mut dimmed_brightness: u8 = DEFAULT_BRIGHTNESS;
    let mut render_needed = true;
    let mut last_temp_display: Option<i32> = None;
    let mut last_humidity_display: Option<u8> = None;
    let mut last_co2_display: Option<u16> = None;
    let mut last_co2_error = false;
    let mut last_zero_mode = false;
    let mut last_battery_display: Option<i32> = None;
    let mut touch_active = false;
    let mut last_mqtt_publish = Instant::now();
    loop {
        if let Some(mqtt) = mqtt.as_mut() {
            while let Some(cmd) = mqtt.try_recv_command() {
                match cmd {
                    MqttCommand::ZeroCalibrate => {
                        if let Err(err) = mhz19b.calibrate_zero() {
                            error!("MQTT zero calibration failed: {:?}", err);
                        } else {
                            info!("MQTT zero calibration triggered");
                            zero_feedback_until = Some(Instant::now() + zero_feedback_duration);
                            render_needed = true;
                        }
                    }
                    MqttCommand::SetAbc(enabled) => {
                        if let Err(err) = mhz19b.set_abc(enabled) {
                            error!("MQTT set ABC failed: {:?}", err);
                        } else {
                            info!("MQTT set ABC: {}", enabled);
                        }
                    }
                    MqttCommand::SetBrightness(percent) => {
                        if let Err(err) = lcd.set_brightness(percent) {
                            error!("MQTT set brightness failed: {:?}", err);
                        } else {
                            info!("MQTT brightness set to {}%", percent);
                        }
                    }
                    MqttCommand::Reboot => unsafe {
                        info!("MQTT reboot requested");
                        esp_restart();
                    },
                }
            }
        }

        if dimming_in_progress && dimmed_brightness > 0 {
            dimmed_brightness = dimmed_brightness.saturating_sub(dimming_step);
            lcd.set_brightness(dimmed_brightness)?;
        }

        if last_env_read.elapsed() >= env_interval {
            match sht31.read(&mut i2c) {
                Ok(reading) => {
                    let new_temp = reading.temperature_c;
                    let new_humidity = reading.humidity_pct.clamp(0.0, 100.0).round() as u8;
                    let new_temp_display = (new_temp * 10.0).round() as i32;
                    if Some(new_temp_display) != last_temp_display
                        || Some(new_humidity) != last_humidity_display
                    {
                        render_needed = true;
                        last_temp_display = Some(new_temp_display);
                        last_humidity_display = Some(new_humidity);
                    }
                    temperature_c = Some(new_temp);
                    humidity_pct = Some(new_humidity);
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
                    if last_co2_display != Some(ppm) || last_co2_error {
                        render_needed = true;
                        last_co2_display = Some(ppm);
                        last_co2_error = false;
                    }
                    co2_value = Some(ppm);
                    co2_error = false;
                    mhz_error_count = 0;
                }
                Err(err) => {
                    error!("MH-Z19B read error: {:?}", err);
                    if !last_co2_error || last_co2_display.is_some() {
                        render_needed = true;
                        last_co2_display = None;
                        last_co2_error = true;
                    }
                    co2_value = None;
                    co2_error = true;
                    mhz_error_count = mhz_error_count.saturating_add(1);
                    if mhz_error_count >= MHZ_ERR_REINIT_THRESHOLD {
                        error!(
                            "MH-Z19B consecutive errors reached {}, reinitializing UART",
                            MHZ_ERR_REINIT_THRESHOLD
                        );
                        if let Err(err) = mhz19b.reinit_uart() {
                            error!("MH-Z19B UART reinit failed: {:?}", err);
                        }
                        mhz_error_count = 0;
                    }
                }
            }
            last_mhz_read = Instant::now();
        }

        if last_ota_check.elapsed() >= OTA_CHECK_INTERVAL {
            if let Some(wifi) = wifi.as_mut() {
                if let Err(err) = check_and_update(wifi) {
                    error!("OTA check failed: {:?}", err);
                }
            }
            last_ota_check = Instant::now();
        }

        if last_battery_read.elapsed() >= battery_interval {
            match battery.read_voltage() {
                Ok(voltage) => {
                    let display_cv = (voltage * 100.0).round() as i32;
                    if last_battery_display != Some(display_cv) {
                        render_needed = true;
                        last_battery_display = Some(display_cv);
                    }
                    battery_v = Some(voltage);
                }
                Err(err) => error!("Battery read error: {:?}", err),
            }
            last_battery_read = Instant::now();
        }

        if last_mqtt_publish.elapsed() >= MQTT_PUBLISH_INTERVAL {
            if let Some(mqtt) = mqtt.as_mut() {
                let telemetry = MqttTelemetry {
                    co2_ppm: co2_value,
                    temp_c: temperature_c,
                    humidity_pct,
                    battery_v,
                };
                if let Err(err) = mqtt.publish_status(&telemetry) {
                    warn!("MQTT publish failed: {:?}", err);
                }
            }
            last_mqtt_publish = Instant::now();
        }

        let irq_pending = touch_take_pending();
        let should_read_touch = irq_pending || touch_active;
        let touch_in_co2 = if should_read_touch {
            match read_touch(&mut i2c) {
                Ok(Some((x, y))) => {
                    touch_active = true;
                    // cancel display dimming/brightening if touch detected
                    if dimming_in_progress {
                        log::info!(
                            "Touch detected - restoring brightness to {}%",
                            DEFAULT_BRIGHTNESS
                        );
                        dimming_in_progress = false;
                        if dimmed_brightness == 0 {
                            render_needed = true;
                        }
                        lcd.set_brightness(DEFAULT_BRIGHTNESS)?;
                        dimmed_brightness = DEFAULT_BRIGHTNESS;
                    }
                    last_touch = Instant::now();

                    let pt = touch_to_view(x, y);
                    co2_rect.contains(pt)
                }
                Ok(None) => {
                    touch_active = false;
                    false
                }
                Err(_) => {
                    touch_active = false;
                    false
                }
            }
        } else {
            false
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

        let zero_mode = zero_feedback_until.is_some();
        if zero_mode != last_zero_mode {
            render_needed = true;
            last_zero_mode = zero_mode;
        }

        if dimmed_brightness != 0 && render_needed {
            render_ui_mock1(
                &mut frame,
                temperature_c,
                humidity_pct,
                co2_value,
                co2_error,
                zero_mode,
                battery_v,
            )?;
            lcd.flush_full(&frame)?;
        render_needed = false;
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
