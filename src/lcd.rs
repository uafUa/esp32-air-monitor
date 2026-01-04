use anyhow::{bail, Result};
use embedded_graphics::mono_font::{ascii::FONT_10X20, ascii::FONT_6X10, MonoTextStyle};
use embedded_graphics::pixelcolor::{IntoStorage, Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyleBuilder, Rectangle};
use embedded_graphics::text::Text;
use embedded_graphics_framebuf::backends::FrameBufferBackend;
use esp_idf_hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_hal::spi::{SpiDeviceDriver, SpiDriver};
use std::thread;
use std::time::Duration;

pub const LCD_CLK_GPIO: i32 = 1;
pub const LCD_MOSI_GPIO: i32 = 2;
pub const LCD_CS_GPIO: i32 = 14;
pub const LCD_DC_GPIO: i32 = 15;
pub const LCD_RST_GPIO: i32 = 22;
pub const LCD_BL_GPIO: i32 = 23;

pub const LCD_W: usize = 172;
pub const LCD_H: usize = 320;
pub const LCD_X_GAP: u16 = 34;

pub struct Rgb565Slice<'a> {
    data: &'a mut [Rgb565],
}

impl<'a> Rgb565Slice<'a> {
    pub fn new(data: &'a mut [Rgb565]) -> Self {
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

pub struct Jd9853<'a> {
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
    pub fn new(
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
        thread::sleep(Duration::from_millis(10));
        self.rst.set_high()?;
        thread::sleep(Duration::from_millis(120));
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
        thread::sleep(Duration::from_millis(120));

        self.cmd(0x29, &[])?; // display on
        thread::sleep(Duration::from_millis(20));

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

    pub fn flush_full(&mut self, frame: &[Rgb565]) -> Result<()> {
        if frame.len() != LCD_W * LCD_H {
            bail!("frame size mismatch: {}", frame.len());
        }

        self.set_window(0, 0, self.w - 1, self.h - 1)?;

        let need = frame.len() * 2;
        if self.txbuf.len() != need {
            self.txbuf.resize(need, 0);
        }

        for (i, px) in frame.iter().copied().enumerate() {
            let raw: u16 = px.into_storage();
            self.txbuf[2 * i] = (raw >> 8) as u8;
            self.txbuf[2 * i + 1] = (raw & 0xFF) as u8;
        }

        self.write_cmd(0x2C)?;

        self.dc.set_high()?;
        const CHUNK: usize = 4096;
        for chunk in self.txbuf.chunks(CHUNK) {
            self.spi_dev.write(chunk)?;
        }

        Ok(())
    }
}

pub fn render_ui_mock1(frame: &mut [Rgb565], temperature_c: f32, humidity_pct: u8, co2_ppm: u16) -> Result<()> {
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
    Text::new(&format!("{:.1}°C", temperature_c), left.top_left + Point::new(10, 54), value)
        .draw(&mut fb)?;

    Text::new("HUM", right.top_left + Point::new(10, 16), label).draw(&mut fb)?;
    Text::new(&format!("{}%", humidity_pct), right.top_left + Point::new(10, 54), value).draw(&mut fb)?;

    Text::new("CO2", bottom.top_left + Point::new(10, 18), label).draw(&mut fb)?;
    Text::new(&format!("{} ppm", co2_ppm), bottom.top_left + Point::new(10, 70), value).draw(&mut fb)?;

    Ok(())
}
