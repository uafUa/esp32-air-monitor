# Copilot Instructions (c6-demo)

## Overview
This project targets an ESP32‑C6 Touch LCD board (1.47" ST7789 + touch). It displays CO2, temperature, and humidity using an on-device UI and reads sensors via UART/I2C.

## Hardware / Pin Mapping
- LCD (SPI2): SCLK GPIO1, MOSI GPIO2, CS GPIO14, DC GPIO15, RST GPIO22, BL GPIO23
- Touch (I2C): SDA GPIO18, SCL GPIO19, RST GPIO20, INT GPIO21
- MH‑Z19B (UART0): TX GPIO16, RX GPIO17, 9600 baud
- SHT31 (I2C): shared bus GPIO18/19, default address 0x44

## Module Layout
- `src/board.rs`: one entry point to init peripherals. `Board::init()` returns lcd/i2c/mhz19b/sht31.
- `src/st7789.rs`: ST7789 LCD driver (SPI), init, brightness control.
- `src/display.rs`: UI layout & drawing with embedded‑graphics + u8g2 fonts.
- `src/touch.rs`: touch controller I2C init, scan, read.
- `src/mhz19b.rs`: MH‑Z19B UART protocol (read, zero calibration, ABC on/off).
- `src/sht31.rs`: SHT31 I2C read (single‑shot high repeatability + CRC).

## Display Details
- Panel size: 172x320 (LCD_W/LCD_H).
- UI is landscape: LCD_VIEW_W=320, LCD_VIEW_H=172.
- MADCTL=0x68 (MV+MX+BGR). Offsets: LCD_X_GAP=0, LCD_Y_GAP=34.
- Framebuffer is full panel size; render in landscape view.

## Runtime Logic
- SHT31 read every ~2s; values shown in UI.
- MH‑Z19B read every ~5s; CO2 shown in UI.
- Touch in CO2 card for ~2s triggers zero calibration; “ZERO” is displayed briefly.
- ABC is disabled at boot via `mhz19b.set_abc(false)`.

## Notes
- UART0 is used for MH‑Z19B, so serial logs may interfere.
- Brightness uses PWM + WRCTRLD/WRDISBV.
- See `wiring.md` for wiring; see `docs/CONTEXT.md` for a concise project summary.
