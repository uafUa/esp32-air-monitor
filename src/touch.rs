use anyhow::Result;
use esp_idf_hal::i2c::I2cDriver;
use esp_idf_sys as sys;
use log::{error, info};
use std::thread;
use std::time::Duration;

type HalResult<T> = core::result::Result<T, esp_idf_hal::sys::EspError>;

pub const TP_ADDR: u8 = 0x63;
pub const TP_SDA_GPIO: i32 = 18;
pub const TP_SCL_GPIO: i32 = 19;
pub const TP_RST_GPIO: i32 = 20;
pub const TP_INT_GPIO: i32 = 21;

// Touch controller uses open-drain I2C + external/internal pull-ups.
pub fn gpio_setup_touch_lines() {
    unsafe {
        sys::gpio_reset_pin(TP_SDA_GPIO);
        sys::gpio_reset_pin(TP_SCL_GPIO);
        sys::gpio_reset_pin(TP_RST_GPIO);
        sys::gpio_reset_pin(TP_INT_GPIO);

        sys::gpio_set_direction(TP_SDA_GPIO, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD);
        sys::gpio_set_direction(TP_SCL_GPIO, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD);
        sys::gpio_pullup_en(TP_SDA_GPIO);
        sys::gpio_pullup_en(TP_SCL_GPIO);
        sys::gpio_pulldown_dis(TP_SDA_GPIO);
        sys::gpio_pulldown_dis(TP_SCL_GPIO);

        // Touch reset line is a push-pull output.
        sys::gpio_set_direction(TP_RST_GPIO, sys::gpio_mode_t_GPIO_MODE_OUTPUT);

        // Touch interrupt is an input with pull-up.
        sys::gpio_set_direction(TP_INT_GPIO, sys::gpio_mode_t_GPIO_MODE_INPUT);
        sys::gpio_pullup_en(TP_INT_GPIO);
        sys::gpio_pulldown_dis(TP_INT_GPIO);
    }
}

// Datasheet-friendly reset pulse for the touch controller.
pub fn touch_reset_pulse() {
    unsafe {
        sys::gpio_set_level(TP_RST_GPIO, 0);
        thread::sleep(Duration::from_millis(5));
        sys::gpio_set_level(TP_RST_GPIO, 1);
        thread::sleep(Duration::from_millis(150));
    }
}

pub fn i2c_scan(i2c: &mut I2cDriver<'_>) {
    info!("Scanning I2C...");
    let mut found = 0;
    for addr in 0x08u8..0x78u8 {
        if i2c.write(addr, &[], esp_idf_hal::delay::BLOCK).is_ok() {
            info!("I2C device at 0x{:02X}", addr);
            found += 1;
        }
    }
    if found == 0 {
        error!("No I2C devices found (wrong pins / no pullups / power gating)");
    }
}

fn read_reg_no_restart(i2c: &mut I2cDriver<'_>, reg: u8, out: &mut [u8]) -> HalResult<()> {
    // Controller dislikes repeated-start, so we do write+stop then read.
    const RETRIES: usize = 3;
    for _ in 0..RETRIES {
        if i2c.write(TP_ADDR, &[reg], esp_idf_hal::delay::BLOCK).is_ok() {
            if i2c.read(TP_ADDR, out, esp_idf_hal::delay::BLOCK).is_ok() {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(2));
    }

    Err(esp_idf_hal::sys::EspError::from(sys::ESP_FAIL as i32).unwrap())
}

pub fn probe_touch(i2c: &mut I2cDriver<'_>) -> Result<()> {
    let mut buf = [0u8; 8];
    read_reg_no_restart(i2c, 0x02, &mut buf)?;
    info!("Touch probe OK, first bytes @0x02: {:02X?}", buf);
    Ok(())
}

pub fn read_touch(i2c: &mut I2cDriver<'_>) -> Result<Option<(u16, u16)>> {
    let mut d = [0u8; 8];
    read_reg_no_restart(i2c, 0x02, &mut d)?;

    // First byte low nibble = number of touch points.
    let points = d[0] & 0x0F;
    if points == 0 {
        return Ok(None);
    }

    let x = (((d[1] as u16) & 0x0F) << 8) | d[2] as u16;
    let y = (((d[3] as u16) & 0x0F) << 8) | d[4] as u16;

    Ok(Some((x, y)))
}
