use anyhow::Result;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{CornerRadii, PrimitiveStyleBuilder, Rectangle, RoundedRectangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use embedded_graphics_framebuf::backends::FrameBufferBackend;
use u8g2_fonts::{fonts, U8g2TextStyle};

use crate::st7789::{LCD_VIEW_H, LCD_VIEW_W};

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
