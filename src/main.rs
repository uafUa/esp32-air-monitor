// main.rs
// ESP32-C6 Touch LCD 1.47" (JD9853 LCD over SPI + AXS5106L touch over I2C)
//
// UI mockup #1:
//   TEMP        HUM
//   21.6°C      45%
//
//   CO2
//   840 ppm
//
// Notes:
// - Touch: esp-idf-hal I2C0 + AXS5106L (0x63), with NO repeated-start reads
// - LCD:   Pure esp-idf-hal SPI (no esp_lcd structs/bindgen unions)
// - UI:    embedded-graphics rendered into an RGB565 framebuffer (embedded-graphics-framebuf)
//
// Pin map (your board):
// LCD (SPI):  CLK=GPIO1, MOSI=GPIO2, CS=GPIO14, DC=GPIO15, RST=GPIO22, BL=GPIO23
// Touch(I2C): SDA=GPIO18, SCL=GPIO19, RST=GPIO20, INT=GPIO21

#![allow(clippy::needless_return)]

use anyhow::{bail, Result};
use esp_idf_hal::delay::Ets;
use esp_idf_hal::gpio::{AnyIOPin, InputOutput, PinDriver, Pull};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::prelude::*;
use esp_idf_hal::spi::{SpiDeviceDriver, SpiDriver};
use esp_idf_hal::spi::config::{Config as SpiDeviceConfig, DriverConfig as SpiDriverConfig};
use esp_idf_svc::log::EspLogger;
use log::{error, info};

use esp_idf_sys as sys;

use embedded_graphics::pixelcolor::{IntoStorage, Rgb565, RgbColor};
use embedded_graphics_framebuf::backends::FrameBufferBackend;

use std::time::{Duration, Instant};

type HalResult<T> = core::result::Result<T, esp_idf_hal::sys::EspError>;

struct Rgb565Slice<'a> {
    data: &'a mut [Rgb565],
}

impl<'a> Rgb565Slice<'a> {
    fn new(data: &'a mut [Rgb565]) -> Self {
        Self { data }
    }
}

impl<'a> FrameBufferBackend for Rgb565Slice<'a> {
    type Color = Rgb565;

    fn set(&mut self, index: usize, color: Rgb565) {
        self.data[index] = color;
    }

    fn get(&self, index: usize) -> Rgb565 {
        self.data[index]
    }

    fn nr_elements(&self) -> usize {
        self.data.len()
    }
}

// ----------------- Touch constants -----------------
const TP_ADDR: u8 = 0x63;

const TP_SDA_GPIO: i32 = 18;
const TP_SCL_GPIO: i32 = 19;
const TP_RST_GPIO: i32 = 20;
const TP_INT_GPIO: i32 = 21;

// ----------------- LCD constants -----------------
const LCD_CLK_GPIO: i32 = 1;
const LCD_MOSI_GPIO: i32 = 2;
const LCD_CS_GPIO: i32 = 14;
const LCD_DC_GPIO: i32 = 15;
const LCD_RST_GPIO: i32 = 22;
const LCD_BL_GPIO: i32 = 23;

// Panel resolution
const LCD_W: usize = 172;
const LCD_H: usize = 320;

// Panel internal X offset (from vendor driver)
const LCD_X_GAP: u16 = 34;

// Sensor pins
const DHT_GPIO: i32 = 4;

#[derive(Debug, Clone, Copy)]
struct DhtReading {
    temperature_c: f32,
    humidity_pct: f32,
}

#[derive(Debug)]
enum DhtError {
    Timeout(&'static str),
    Checksum,
    Gpio(esp_idf_hal::sys::EspError),
}

struct Dht22Sensor<'a> {
    pin: PinDriver<'a, AnyIOPin, InputOutput>,
}

impl<'a> Dht22Sensor<'a> {
    fn new(mut pin: PinDriver<'a, AnyIOPin, InputOutput>) -> Result<Self> {
        pin.set_pull(Pull::Up)?;
        pin.set_high()?;
        Ok(Self { pin })
    }

    fn read(&mut self) -> core::result::Result<DhtReading, DhtError> {
        self.pin.set_low().map_err(DhtError::Gpio)?;
        Ets::delay_ms(2);
        self.pin.set_high().map_err(DhtError::Gpio)?;
        Ets::delay_us(30);

        // Sensor response: low -> high -> low before data bits.
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

// ----------------- Low-level GPIO helpers (touch lines) -----------------
fn gpio_setup_touch_lines() {
    unsafe {
        sys::gpio_reset_pin(TP_SDA_GPIO);
        sys::gpio_reset_pin(TP_SCL_GPIO);
        sys::gpio_reset_pin(TP_RST_GPIO);
        sys::gpio_reset_pin(TP_INT_GPIO);

        // I2C open-drain + pullups
        sys::gpio_set_direction(TP_SDA_GPIO, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD);
        sys::gpio_set_direction(TP_SCL_GPIO, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD);
        sys::gpio_pullup_en(TP_SDA_GPIO);
        sys::gpio_pullup_en(TP_SCL_GPIO);
        sys::gpio_pulldown_dis(TP_SDA_GPIO);
        sys::gpio_pulldown_dis(TP_SCL_GPIO);

        // Touch reset output
        sys::gpio_set_direction(TP_RST_GPIO, sys::gpio_mode_t_GPIO_MODE_OUTPUT);

        // Touch interrupt input (pullup on)
        sys::gpio_set_direction(TP_INT_GPIO, sys::gpio_mode_t_GPIO_MODE_INPUT);
        sys::gpio_pullup_en(TP_INT_GPIO);
        sys::gpio_pulldown_dis(TP_INT_GPIO);
    }
}

fn touch_reset_pulse() {
    unsafe {
        sys::gpio_set_level(TP_RST_GPIO, 0);
        std::thread::sleep(std::time::Duration::from_millis(5));
        sys::gpio_set_level(TP_RST_GPIO, 1);
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

fn i2c_scan(i2c: &mut I2cDriver<'_>) {
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

// ----------------- Touch (AXS5106L) -----------------
fn read_reg_no_restart(i2c: &mut I2cDriver<'_>, reg: u8, out: &mut [u8]) -> HalResult<()> {
    // AXS5106L dislikes repeated-start. Do WRITE(reg)+STOP, then READ.
    const RETRIES: usize = 3;
    for _ in 0..RETRIES {
        if i2c.write(TP_ADDR, &[reg], esp_idf_hal::delay::BLOCK).is_ok() {
            if i2c.read(TP_ADDR, out, esp_idf_hal::delay::BLOCK).is_ok() {
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    // `EspError::from` returns Option<EspError>
    Err(esp_idf_hal::sys::EspError::from(sys::ESP_FAIL as i32).unwrap())
}

fn probe_touch(i2c: &mut I2cDriver<'_>) -> Result<()> {
    let mut buf = [0u8; 8];
    read_reg_no_restart(i2c, 0x02, &mut buf)?;
    info!("Touch probe OK, first bytes @0x02: {:02X?}", buf);
    Ok(())
}

fn read_touch(i2c: &mut I2cDriver<'_>) -> Result<Option<(u16, u16)>> {
    let mut d = [0u8; 8];
    read_reg_no_restart(i2c, 0x02, &mut d)?;

    let points = d[0] & 0x0F;
    if points == 0 {
        return Ok(None);
    }

    let x = (((d[1] as u16) & 0x0F) << 8) | d[2] as u16;
    let y = (((d[3] as u16) & 0x0F) << 8) | d[4] as u16;

    Ok(Some((x, y)))
}

// ----------------- LCD (JD9853) over SPI (esp-idf-hal only) -----------------
// We avoid esp_lcd bindgen structs because they changed shape across IDF versions.

struct Jd9853<'a> {
    spi_dev: SpiDeviceDriver<'a, SpiDriver<'a>>,
    dc: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
    rst: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
    bl: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
    x_gap: u16,
    w: u16,
    h: u16,
    txbuf: Vec<u8>,
}

impl<'a> Jd9853<'a> {
    fn new(
        spi_dev: SpiDeviceDriver<'a, SpiDriver<'a>>,
        dc: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
        rst: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
        bl: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
    ) -> Result<Self> {
        let mut lcd = Self {
            spi_dev,
            dc,
            rst,
            bl,
            x_gap: LCD_X_GAP,
            w: LCD_W as u16,
            h: LCD_H as u16,
            txbuf: vec![0u8; LCD_W * LCD_H * 2],
        };

        lcd.reset()?;
        lcd.init_sequence()?;
        Ok(lcd)
    }

    fn reset(&mut self) -> Result<()> {
        self.bl.set_low()?;
        self.rst.set_low()?;
        std::thread::sleep(std::time::Duration::from_millis(10));
        self.rst.set_high()?;
        std::thread::sleep(std::time::Duration::from_millis(120));
        self.bl.set_high()?;
        Ok(())
    }

    fn write_cmd(&mut self, cmd: u8) -> Result<()> {
        self.dc.set_low()?;
        self.spi_dev.write(&[cmd])?;
        Ok(())
    }

    fn write_data(&mut self, data: &[u8]) -> Result<()> {
        self.dc.set_high()?;
        const CHUNK: usize = 4096;
        for chunk in data.chunks(CHUNK) {
            self.spi_dev.write(chunk)?;
        }
        Ok(())
    }

    fn cmd(&mut self, cmd: u8, data: &[u8]) -> Result<()> {
        self.write_cmd(cmd)?;
        if !data.is_empty() {
            self.write_data(data)?;
        }
        Ok(())
    }

    fn init_sequence(&mut self) -> Result<()> {
        self.cmd(0xDF, &[0x98, 0x53, 0x81])?;
        self.cmd(0xDE, &[0x00])?;

        self.cmd(0xB2, &[0x23])?;
        self.cmd(
            0xB7,
            &[0x00, 0x1A, 0x1B, 0x21, 0x22, 0x21, 0x22, 0x1B, 0x1A, 0x00],
        )?;
        self.cmd(0xBB, &[0x0B])?;
        self.cmd(0xC0, &[0x12])?;
        self.cmd(0xC1, &[0x10])?;
        self.cmd(0xC3, &[0x0E])?;
        self.cmd(0xC4, &[0x07])?;
        self.cmd(0xC5, &[0x27])?;
        self.cmd(0xC6, &[0x1F])?;
        self.cmd(0xC7, &[0x1F])?;

        self.cmd(0xD0, &[0xA4, 0xA1])?;
        self.cmd(0xD2, &[0x2C, 0x2C])?;
        self.cmd(0xD3, &[0x4A, 0x4A])?;
        self.cmd(0xD4, &[0x0A, 0x00, 0x00, 0x00])?;
        self.cmd(0xD6, &[0xD5])?;

        self.cmd(
            0xE0,
            &[0x00, 0x04, 0x0E, 0x08, 0x17, 0x0A, 0x40, 0x79, 0x4D, 0x07, 0x0E, 0x0A, 0x1A, 0x1D, 0x0F],
        )?;
        self.cmd(
            0xE1,
            &[0x00, 0x1D, 0x20, 0x02, 0x0E, 0x05, 0x2E, 0x25, 0x47, 0x04, 0x0C, 0x0B, 0x1D, 0x23, 0x0F],
        )?;

        self.cmd(0x36, &[0x00])?; // MADCTL
        self.cmd(0x3A, &[0x55])?; // RGB565

        self.cmd(0x11, &[])?; // sleep out
        std::thread::sleep(std::time::Duration::from_millis(120));

        self.cmd(0x29, &[])?; // display on
        std::thread::sleep(std::time::Duration::from_millis(20));

        Ok(())
    }

    fn set_window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) -> Result<()> {
        let x0 = x0 + self.x_gap;
        let x1 = x1 + self.x_gap;

        let caset = [(x0 >> 8) as u8, (x0 & 0xFF) as u8, (x1 >> 8) as u8, (x1 & 0xFF) as u8];
        let raset = [(y0 >> 8) as u8, (y0 & 0xFF) as u8, (y1 >> 8) as u8, (y1 & 0xFF) as u8];

        self.cmd(0x2A, &caset)?;
        self.cmd(0x2B, &raset)?;
        Ok(())
    }

    fn flush_full(&mut self, frame: &[Rgb565]) -> Result<()> {
        if frame.len() != LCD_W * LCD_H {
            bail!("frame size mismatch: {}", frame.len());
        }

        self.set_window(0, 0, self.w - 1, self.h - 1)?;

        let need = frame.len() * 2;
        if self.txbuf.len() != need {
            self.txbuf.resize(need, 0);
        }

        // Convert Rgb565 pixels to big-endian bytes.
        for (i, px) in frame.iter().copied().enumerate() {
            let raw: u16 = px.into_storage();
            self.txbuf[2 * i] = (raw >> 8) as u8;
            self.txbuf[2 * i + 1] = (raw & 0xFF) as u8;
        }

        self.write_cmd(0x2C)?;

        // Stream directly from txbuf (avoids borrow issues).
        self.dc.set_high()?;
        const CHUNK: usize = 4096;
        for chunk in self.txbuf.chunks(CHUNK) {
            self.spi_dev.write(chunk)?;
        }

        Ok(())
    }
}

// ----------------- UI renderer (mockup #1) -----------------
fn render_ui_mock1(frame: &mut [Rgb565], temperature_c: f32, humidity_pct: u8, co2_ppm: u16) -> Result<()> {
    use embedded_graphics::mono_font::{ascii::FONT_10X20, ascii::FONT_6X10, MonoTextStyle};
    use embedded_graphics::pixelcolor::Rgb565;
    use embedded_graphics::prelude::*;
    use embedded_graphics::primitives::{PrimitiveStyleBuilder, Rectangle};
    use embedded_graphics::text::Text;

    let backend = Rgb565Slice::new(frame);
    let mut fb = embedded_graphics_framebuf::FrameBuf::<Rgb565, _>::new(backend, LCD_W, LCD_H);

    fb.clear(Rgb565::BLACK)?;

    let card_style = PrimitiveStyleBuilder::new()
        .stroke_color(Rgb565::new(8, 8, 8))
        .stroke_width(2)
        .fill_color(Rgb565::BLACK)
        .build();

    let pad = 10i32;
    let gap = 8i32;
    let top_h = 110i32;
    let w = LCD_W as i32;

    let left = Rectangle::new(
        Point::new(pad, pad),
        Size::new(((w - 2 * pad - gap) / 2) as u32, top_h as u32),
    );
    let right = Rectangle::new(
        Point::new(pad + ((w - 2 * pad - gap) / 2) + gap, pad),
        Size::new(((w - 2 * pad - gap) / 2) as u32, top_h as u32),
    );
    let bottom = Rectangle::new(
        Point::new(pad, pad + top_h + gap),
        Size::new((w - 2 * pad) as u32, (LCD_H as i32 - (pad + top_h + gap) - pad) as u32),
    );

    left.into_styled(card_style).draw(&mut fb)?;
    right.into_styled(card_style).draw(&mut fb)?;
    bottom.into_styled(card_style).draw(&mut fb)?;

    let label = MonoTextStyle::new(&FONT_6X10, Rgb565::new(20, 20, 20));
    let value = MonoTextStyle::new(&FONT_10X20, Rgb565::new(31, 31, 31));

    Text::new("TEMP", left.top_left + Point::new(10, 16), label).draw(&mut fb)?;
    Text::new(&format!("{:.1}°C", temperature_c), left.top_left + Point::new(10, 54), value).draw(&mut fb)?;

    Text::new("HUM", right.top_left + Point::new(10, 16), label).draw(&mut fb)?;
    Text::new(&format!("{}%", humidity_pct), right.top_left + Point::new(10, 54), value).draw(&mut fb)?;

    Text::new("CO2", bottom.top_left + Point::new(10, 18), label).draw(&mut fb)?;
    Text::new(&format!("{} ppm", co2_ppm), bottom.top_left + Point::new(10, 70), value).draw(&mut fb)?;

    Ok(())
}

// ----------------- Main -----------------
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

        std::thread::sleep(Duration::from_millis(200));
    }
}
