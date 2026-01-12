use anyhow::Result;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{CornerRadii, PrimitiveStyleBuilder, Rectangle, RoundedRectangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use embedded_graphics_framebuf::backends::FrameBufferBackend;
use u8g2_fonts::{fonts, U8g2TextStyle};

use crate::st7789::{LCD_VIEW_H, LCD_VIEW_W};

const COLOR_BG: Rgb565 = Rgb565::new(0, 0, 0);
const COLOR_FRAME: Rgb565 = Rgb565::new(16, 32, 16);
const COLOR_CARD: Rgb565 = Rgb565::new(3, 8, 5);
const COLOR_LABEL: Rgb565 = Rgb565::new(31, 63, 33);
const COLOR_CO2_ZERO: Rgb565 = Rgb565::new(0, 63, 31);
const COLOR_TEMP: Rgb565 = Rgb565::new(31, 32, 0);
const COLOR_HUM: Rgb565 = Rgb565::new(0, 32, 31);
const COLOR_GOOD: Rgb565 = Rgb565::new(0, 63, 0);
const COLOR_FAIR: Rgb565 = Rgb565::new(31, 63, 0);
const COLOR_POOR: Rgb565 = Rgb565::new(31, 24, 0);
const COLOR_BAD: Rgb565 = Rgb565::new(31, 0, 0);

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
    co2_ppm: Option<u16>,
    co2_error: bool,
    zero_mode: bool,
    battery_v: Option<f32>,
) -> Result<()> {
    let view_w = LCD_VIEW_W;
    let view_h = LCD_VIEW_H;
    let backend = LinearRgb565Slice::new(frame);
    let mut fb = embedded_graphics_framebuf::FrameBuf::<Rgb565, _>::new(backend, view_w, view_h);

    fb.clear(COLOR_BG)?;

    let frame_style = PrimitiveStyleBuilder::new()
        .stroke_color(COLOR_FRAME)
        .stroke_width(3)
        .build();
    let card_style = PrimitiveStyleBuilder::new()
        .stroke_color(COLOR_CARD)
        .stroke_width(2)
        .fill_color(COLOR_CARD)
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

    let (co2_value_text, co2_value_color, status_text_opt, status_color) = if zero_mode {
        ("ZERO".to_string(), COLOR_CO2_ZERO, None, COLOR_CO2_ZERO)
    } else if co2_error {
        ("ERR".to_string(), COLOR_BAD, None, COLOR_BAD)
    } else if let Some(ppm) = co2_ppm {
        let (status_text, status_color) = co2_status(ppm);
        (format!("{}", ppm), status_color, Some(status_text), status_color)
    } else {
        ("...".to_string(), COLOR_LABEL, None, COLOR_LABEL)
    };

    let style_label = U8g2TextStyle::new(fonts::u8g2_font_helvR10_tf, COLOR_LABEL);
    let style_co2_value = U8g2TextStyle::new(fonts::u8g2_font_fub35_tf, co2_value_color);
    let style_temp_value = U8g2TextStyle::new(fonts::u8g2_font_helvB24_tf, COLOR_TEMP);
    let style_hum_value = U8g2TextStyle::new(fonts::u8g2_font_helvB24_tf, COLOR_HUM);
    let style_status = U8g2TextStyle::new(fonts::u8g2_font_helvB12_tf, status_color);
    let center_text = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();
    let right_top_text = TextStyleBuilder::new()
        .alignment(Alignment::Right)
        .baseline(Baseline::Top)
        .build();

    let style_label_battery = U8g2TextStyle::new(fonts::u8g2_font_helvR10_tf, COLOR_LABEL);
    let battery_text = match battery_v {
        Some(voltage) => format!("BAT {:.2}V", voltage),
        None => "BAT --.-V".to_string(),
    };
    let battery_pos = Point::new(
        frame_rect.top_left.x + frame_rect.size.width as i32 - 6,
        frame_rect.top_left.y + 6,
    );
    Text::with_text_style(&battery_text, battery_pos, style_label_battery, right_top_text)
        .draw(&mut fb)?;

    let left_center_x = panel_co.center().x;
    let left_top = panel_co.top_left;
    let left_h = panel_co.size.height as i32;
    let co2_val_y = left_top.y + (left_h * 40) / 100;
    let ppm_y = left_top.y + (left_h * 68) / 100;
    let status_y = left_top.y + (left_h * 82) / 100;

    Text::with_text_style(
        &co2_value_text,
        Point::new(left_center_x, co2_val_y),
        style_co2_value,
        center_text,
    )
    .draw(&mut fb)?;

    if let Some(status_text) = status_text_opt {
        Text::with_text_style("ppm", Point::new(left_center_x, ppm_y), style_label, center_text)
            .draw(&mut fb)?;
        Text::with_text_style(status_text, Point::new(left_center_x, status_y), style_status, center_text)
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

fn co2_status(co2_ppm: u16) -> (&'static str, Rgb565) {
    if co2_ppm < 600 {
        ("Good", COLOR_GOOD)
    } else if co2_ppm <= 1000 {
        ("Fair", COLOR_FAIR)
    } else if co2_ppm <= 1500 {
        ("Poor", COLOR_POOR)
    } else {
        ("Bad", COLOR_BAD)
    }
}
