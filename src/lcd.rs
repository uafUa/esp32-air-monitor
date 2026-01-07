use anyhow::{bail, Result};
use embedded_graphics::pixelcolor::{IntoStorage, Rgb565};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{CornerRadii, PrimitiveStyleBuilder, Rectangle, RoundedRectangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use embedded_graphics_framebuf::backends::FrameBufferBackend;
use esp_idf_hal::gpio::{AnyIOPin, PinDriver};
use esp_idf_hal::ledc::{self, LedcDriver, LedcTimerDriver};
use esp_idf_hal::spi::{SpiDeviceDriver, SpiDriver};
use std::thread;
use std::time::Duration;
use u8g2_fonts::{fonts, U8g2TextStyle};

pub const LCD_CLK_GPIO: i32 = 1;
pub const LCD_MOSI_GPIO: i32 = 2;
pub const LCD_CS_GPIO: i32 = 14;
pub const LCD_DC_GPIO: i32 = 15;
pub const LCD_RST_GPIO: i32 = 22;
pub const LCD_BL_GPIO: i32 = 23;

// Panel resolution (physical pixels).
pub const LCD_W: usize = 172;
pub const LCD_H: usize = 320;
// Landscape view resolution (after hardware rotation).
pub const LCD_VIEW_W: usize = LCD_H;
pub const LCD_VIEW_H: usize = LCD_W;
// Controller RAM offsets (panel-specific; align drawing with visible glass).
pub const LCD_X_GAP: u16 = 0;
pub const LCD_Y_GAP: u16 = 34;

struct LinearRgb565Slice<'a> {
    data: &'a mut [Rgb565],
}

impl<'a> LinearRgb565Slice<'a> {
    fn new(data: &'a mut [Rgb565]) -> Self {
        Self { data }
    }
}

impl<'a> FrameBufferBackend for LinearRgb565Slice<'a> {
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

pub struct Jd9853<'a, T>
where
    T: ledc::LedcTimer,
{
    spi_dev: SpiDeviceDriver<'a, SpiDriver<'a>>,
    dc: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
    rst: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
    bl_pwm: LedcDriver<'a>,
    bl_timer: LedcTimerDriver<'a, T>,
    x_gap: u16,
    y_gap: u16,
    w: u16,
    h: u16,
    txbuf: Vec<u8>,
}

impl<'a, T> Jd9853<'a, T>
where
    T: ledc::LedcTimer,
{
    pub fn new(
        spi_dev: SpiDeviceDriver<'a, SpiDriver<'a>>,
        dc: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
        rst: PinDriver<'a, AnyIOPin, esp_idf_hal::gpio::Output>,
        bl_pwm: LedcDriver<'a>,
        bl_timer: LedcTimerDriver<'a, T>,
    ) -> Result<Self> {
        let mut lcd = Self {
            spi_dev,
            dc,
            rst,
            bl_pwm,
            bl_timer,
            x_gap: LCD_X_GAP,
            y_gap: LCD_Y_GAP,
            w: LCD_VIEW_W as u16,
            h: LCD_VIEW_H as u16,
            txbuf: vec![0u8; LCD_W * LCD_H * 2],
        };

        // Hardware reset and panel init sequence specific to JD9853.
        lcd.reset()?;
        lcd.init_sequence()?;
        Ok(lcd)
    }

    fn reset(&mut self) -> Result<()> {
        // Reset pulse + backlight enable for this panel wiring.
        self.set_backlight_pwm(0)?;
        self.rst.set_low()?;
        thread::sleep(Duration::from_millis(10));
        self.rst.set_high()?;
        thread::sleep(Duration::from_millis(120));
        self.set_backlight_pwm(100)?;
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

        // Rotate to landscape using MV+MX and enable BGR color order.
        self.cmd(0x36, &[0x68])?; // MADCTL
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
        let y0 = y0 + self.y_gap;
        let y1 = y1 + self.y_gap;

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

        // Convert RGB565 to big-endian byte stream (panel expects BE).
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

    pub fn set_brightness(&mut self, percent: u8) -> Result<()> {
        self.set_backlight_pwm(percent)?;
        // self.set_display_brightness(percent)?;
        Ok(())
    }

    fn set_backlight_pwm(&mut self, percent: u8) -> Result<()> {
        let pct = percent.min(100) as u32;
        let max = self.bl_pwm.get_max_duty();
        let duty = max * pct / 100;
        self.bl_pwm.set_duty(duty)?;
        Ok(())
    }

    fn set_display_brightness(&mut self, percent: u8) -> Result<()> {
        // Enable brightness control (BCTRL) and backlight (BL) in WRCTRLD.
        self.cmd(0x53, &[0x24])?;
        let value = ((percent.min(100) as u16) * 255 / 100) as u8;
        log::info!("Setting display brightness to {}% {}", percent, value);
        self.cmd(0x51, &[value])?;
        Ok(())
    }
}

fn ui_cards() -> (Rectangle, Rectangle, Rectangle) {
    let view_w = LCD_VIEW_W;
    let view_h = LCD_VIEW_H;
    let pad = 14i32;
    let gap = 10i32;
    let content_w = view_w as i32 - 2 * pad;
    let content_h = view_h as i32 - 2 * pad;
    let left_w = (content_w * 58) / 100;
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
    let view_w = LCD_VIEW_W;
    let view_h = LCD_VIEW_H;
    let backend = LinearRgb565Slice::new(frame);
    let mut fb = embedded_graphics_framebuf::FrameBuf::<Rgb565, _>::new(backend, view_w, view_h);

    let bg = Rgb565::new(0, 0, 0);
    let frame_color = Rgb565::new(16, 32, 16);
    let card_stroke = Rgb565::new(3, 8, 5);

    let label_color = Rgb565::new(31, 63, 33);
    let co2_color = Rgb565::new(0, 63, 31);
    let temp_color = Rgb565::new(31, 32, 0);
    let hum_color = Rgb565::new(0, 32, 31);
    let ok_color = Rgb565::new(0, 63, 0);

    fb.clear(bg)?;

    let frame_style = PrimitiveStyleBuilder::new()
        .stroke_color(frame_color)
        .stroke_width(3)
        .build();
    let card_style = PrimitiveStyleBuilder::new()
        .stroke_color(card_stroke)
        .stroke_width(2)
        .fill_color(card_stroke)
        .build();

    let frame_rect = Rectangle::new(
        Point::new(4, 4),
        Size::new((view_w - 8) as u32, (view_h - 8) as u32),
    );
    let frame_round = RoundedRectangle::with_equal_corners(frame_rect, Size::new(12, 12));
    frame_round.into_styled(frame_style).draw(&mut fb)?;

    let (panel_co, panel_temp, panel_hum) = ui_cards();

    let card_radii = CornerRadii::new(Size::new(10, 10));
    RoundedRectangle::new(panel_co, card_radii).into_styled(card_style).draw(&mut fb)?;
    RoundedRectangle::new(panel_temp, card_radii).into_styled(card_style).draw(&mut fb)?;
    RoundedRectangle::new(panel_hum, card_radii).into_styled(card_style).draw(&mut fb)?;

    let style_label = U8g2TextStyle::new(fonts::u8g2_font_helvR10_tf, label_color);
    let style_co2_value = U8g2TextStyle::new(fonts::u8g2_font_fub35_tf, co2_color);
    let style_temp_value = U8g2TextStyle::new(fonts::u8g2_font_helvB24_tf, temp_color);
    let style_hum_value = U8g2TextStyle::new(fonts::u8g2_font_helvB24_tf, hum_color);
    let style_status = U8g2TextStyle::new(fonts::u8g2_font_helvB12_tf, ok_color);
    let center_text = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    let left_center_x = panel_co.center().x;
    let left_top = panel_co.top_left;
    let left_h = panel_co.size.height as i32;
    let co2_val_y = left_top.y + (left_h * 40) / 100;
    let ppm_y = left_top.y + (left_h * 68) / 100;
    let status_y = left_top.y + (left_h * 82) / 100;

    if zero_mode {
        Text::with_text_style("ZERO", Point::new(left_center_x, co2_val_y), style_co2_value, center_text)
            .draw(&mut fb)?;
    } else {
        Text::with_text_style(
            &format!("{}", co2_ppm),
            Point::new(left_center_x, co2_val_y),
            style_co2_value,
            center_text,
        )
        .draw(&mut fb)?;
        Text::with_text_style("ppm", Point::new(left_center_x, ppm_y), style_label, center_text).draw(&mut fb)?;
        Text::with_text_style("Good", Point::new(left_center_x, status_y), style_status, center_text)
            .draw(&mut fb)?;
    }

    let rt_center_x = panel_temp.center().x;
    let rt_center_y = panel_temp.center().y;
    Text::with_text_style(
        &format!("{:.1}°C", temperature_c),
        Point::new(rt_center_x, rt_center_y),
        style_temp_value,
        center_text,
    )
    .draw(&mut fb)?;

    let rb_center_x = panel_hum.center().x;
    let rb_center_y = panel_hum.center().y;
    Text::with_text_style(
        &format!("{}%", humidity_pct),
        Point::new(rb_center_x, rb_center_y),
        style_hum_value,
        center_text,
    )
    .draw(&mut fb)?;

    Ok(())
}
