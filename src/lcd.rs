use anyhow::{bail, Result};
use embedded_graphics::mono_font::{ascii::FONT_6X10, iso_8859_1::FONT_10X20, MonoTextStyle};
use embedded_graphics::pixelcolor::{IntoStorage, Rgb565};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyleBuilder, Rectangle};
use embedded_graphics::text::{Alignment, Text};
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

// Panel resolution (physical pixels).
pub const LCD_W: usize = 172;
pub const LCD_H: usize = 320;
// Controller RAM X-offset (panel-specific; aligns drawing with visible glass).
pub const LCD_X_GAP: u16 = 34;

struct RotatedRgb565Slice<'a> {
    data: &'a mut [Rgb565],
    logical_w: usize,
    physical_w: usize,
}

impl<'a> RotatedRgb565Slice<'a> {
    fn new(data: &'a mut [Rgb565], logical_w: usize, physical_w: usize) -> Self {
        Self {
            data,
            logical_w,
            physical_w,
        }
    }
}

impl<'a> FrameBufferBackend for RotatedRgb565Slice<'a> {
    type Color = Rgb565;

    fn set(&mut self, index: usize, color: Rgb565) {
        let x = index % self.logical_w;
        let y = index / self.logical_w;
        let x_p = self.physical_w - 1 - y;
        let y_p = x;
        let mapped = y_p * self.physical_w + x_p;
        self.data[mapped] = color;
    }

    fn get(&self, index: usize) -> Rgb565 {
        let x = index % self.logical_w;
        let y = index / self.logical_w;
        let x_p = self.physical_w - 1 - y;
        let y_p = x;
        let mapped = y_p * self.physical_w + x_p;
        self.data[mapped]
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

        // Hardware reset and panel init sequence specific to JD9853.
        lcd.reset()?;
        lcd.init_sequence()?;
        Ok(lcd)
    }

    fn reset(&mut self) -> Result<()> {
        // Reset pulse + backlight enable for this panel wiring.
        self.bl.set_low()?;
        self.rst.set_low()?;
        thread::sleep(Duration::from_millis(10));
        self.rst.set_high()?;
        thread::sleep(Duration::from_millis(120));
        self.bl.set_high()?;
        Ok(())
    }

    fn write_cmd(&mut self, cmd: u8) -> Result<()> {
        // D/C low selects command phase.
        self.dc.set_low()?;
        self.spi_dev.write(&[cmd])?;
        Ok(())
    }

    fn write_data(&mut self, data: &[u8]) -> Result<()> {
        // D/C high selects data phase; chunk to limit SPI transaction size.
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
        // Vendor-provided init sequence tuned for this JD9853 module.
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
        // Apply panel X offset before setting address window.
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

        // Convert RGB565 to big-endian byte stream (most LCD controllers expect BE).
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

struct ScaledDrawTarget<'a, DT> {
    target: &'a mut DT,
    scale: i32,
}

impl<'a, DT> DrawTarget for ScaledDrawTarget<'a, DT>
where
    DT: DrawTarget<Color = Rgb565> + OriginDimensions,
{
    type Color = Rgb565;
    type Error = DT::Error;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let scale = self.scale;
        for Pixel(point, color) in pixels.into_iter() {
            let x0 = point.x * scale;
            let y0 = point.y * scale;
            for dy in 0..scale {
                for dx in 0..scale {
                    self.target.draw_iter(core::iter::once(Pixel(
                        Point::new(x0 + dx, y0 + dy),
                        color,
                    )))?;
                }
            }
        }
        Ok(())
    }
}

impl<DT> OriginDimensions for ScaledDrawTarget<'_, DT>
where
    DT: OriginDimensions,
{
    fn size(&self) -> Size {
        let size = self.target.size();
        Size::new(
            size.width / self.scale as u32,
            size.height / self.scale as u32,
        )
    }
}

fn draw_text_scaled<DT>(
    target: &mut DT,
    text: &str,
    pos: Point,
    style: MonoTextStyle<Rgb565>,
    alignment: Alignment,
    scale: i32,
) -> Result<(), DT::Error>
where
    DT: DrawTarget<Color = Rgb565> + OriginDimensions,
{
    let mut scaled = ScaledDrawTarget { target, scale };
    let pos = Point::new(pos.x / scale, pos.y / scale);
    Text::with_alignment(text, pos, style, alignment)
        .draw(&mut scaled)
        .map(|_| ())
}

fn ui_cards() -> (Rectangle, Rectangle, Rectangle) {
    let view_w = LCD_H;
    let view_h = LCD_W;
    let pad = 10i32;
    let gap = 6i32;
    let content_w = view_w as i32 - 2 * pad;
    let content_h = view_h as i32 - 2 * pad;
    let left_w = (content_w - gap) / 2;
    let right_w = content_w - left_w - gap;
    let right_h = (content_h - gap) / 2;

    let left = Rectangle::new(Point::new(pad, pad), Size::new(left_w as u32, content_h as u32));
    let right_top = Rectangle::new(
        Point::new(pad + left_w + gap, pad),
        Size::new(right_w as u32, right_h as u32),
    );
    let right_bottom = Rectangle::new(
        Point::new(pad + left_w + gap, pad + right_h + gap),
        Size::new(right_w as u32, right_h as u32),
    );

    (left, right_top, right_bottom)
}

pub fn co2_card_rect() -> Rectangle {
    let (left, _, _) = ui_cards();
    left
}

pub fn render_ui_mock1(
    frame: &mut [Rgb565],
    temperature_c: f32,
    humidity_pct: u8,
    co2_ppm: u16,
    zero_mode: bool,
) -> Result<()> {
    let view_w = LCD_H;
    let view_h = LCD_W;
    let backend = RotatedRgb565Slice::new(frame, view_w, LCD_W);
    let mut fb = embedded_graphics_framebuf::FrameBuf::<Rgb565, _>::new(backend, view_w, view_h);

    let bg = Rgb565::new(1, 2, 4);
    let frame_color = Rgb565::new(9, 11, 14);
    let card_fill = Rgb565::new(2, 3, 5);
    let card_stroke = Rgb565::new(5, 6, 8);

    let label_color = Rgb565::new(18, 20, 22);
    let co2_color = Rgb565::new(0, 47, 31);
    let temp_color = Rgb565::new(31, 19, 0);
    let hum_color = Rgb565::new(8, 24, 31);
    let ok_color = Rgb565::new(0, 45, 12);

    fb.clear(bg)?;

    let frame_style = PrimitiveStyleBuilder::new()
        .stroke_color(frame_color)
        .stroke_width(2)
        .build();
    let card_style = PrimitiveStyleBuilder::new()
        .stroke_color(card_stroke)
        .stroke_width(2)
        .fill_color(card_fill)
        .build();

    let frame_rect = Rectangle::new(
        Point::new(4, 4),
        Size::new((view_w - 8) as u32, (view_h - 8) as u32),
    );
    frame_rect.into_styled(frame_style).draw(&mut fb)?;

    let (left, right_top, right_bottom) = ui_cards();
    let right_h = right_top.size.height as i32;

    left.into_styled(card_style).draw(&mut fb)?;
    right_top.into_styled(card_style).draw(&mut fb)?;
    right_bottom.into_styled(card_style).draw(&mut fb)?;

    let label = MonoTextStyle::new(&FONT_6X10, label_color);
    let co2_value = MonoTextStyle::new(&FONT_10X20, co2_color);
    let temp_value = MonoTextStyle::new(&FONT_10X20, temp_color);
    let hum_value = MonoTextStyle::new(&FONT_10X20, hum_color);
    let status = MonoTextStyle::new(&FONT_6X10, ok_color);
    let value_scale = 2;
    let value_height = (FONT_10X20.character_size.height as i32) * value_scale;

    let left_center_x = left.center().x;
    let left_top = left.top_left;
    let left_h = left.size.height as i32;
    let co2_val_y = left_top.y + left_h / 2 - value_height / 2;
    let ppm_y = co2_val_y + value_height + 6;
    let status_y = ppm_y + 18;

    if zero_mode {
        draw_text_scaled(
            &mut fb,
            "ZERO",
            Point::new(left_center_x, co2_val_y),
            co2_value,
            Alignment::Center,
            value_scale,
        )?;
    } else {
        draw_text_scaled(
            &mut fb,
            &format!("{}", co2_ppm),
            Point::new(left_center_x, co2_val_y),
            co2_value,
            Alignment::Center,
            value_scale,
        )?;
        Text::with_alignment("ppm", Point::new(left_center_x, ppm_y), label, Alignment::Center).draw(&mut fb)?;
        Text::with_alignment("Good", Point::new(left_center_x, status_y), status, Alignment::Center).draw(&mut fb)?;
    }

    let rt_center_x = right_top.center().x;
    let rt_top = right_top.top_left;
    let temp_val_y = rt_top.y + right_h / 2 - value_height / 2;
    draw_text_scaled(
        &mut fb,
        &format!("{:.1}°C", temperature_c),
        Point::new(rt_center_x, temp_val_y),
        temp_value,
        Alignment::Center,
        value_scale,
    )?;

    let rb_center_x = right_bottom.center().x;
    let rb_top = right_bottom.top_left;
    let hum_val_y = rb_top.y + right_h / 2 - value_height / 2;
    draw_text_scaled(
        &mut fb,
        &format!("{}%", humidity_pct),
        Point::new(rb_center_x, hum_val_y),
        hum_value,
        Alignment::Center,
        value_scale,
    )?;

    Ok(())
}
