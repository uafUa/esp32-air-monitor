# Project Context (c6-demo)

## Hardware
- Board: ESP32-C6 Touch LCD 1.47" (ST7789 controller + touch).
- LCD (SPI2): SCLK GPIO1, MOSI GPIO2, CS GPIO14, DC GPIO15, RST GPIO22, BL GPIO23.
- Touch (I2C): SDA GPIO18, SCL GPIO19, RST GPIO20, INT GPIO21.
- MH-Z19B (UART0): TX GPIO16, RX GPIO17, 9600 baud.
- SHT31 (I2C): same bus as touch (GPIO18/19), addr 0x44 by default.

## Code Layout
- `src/board.rs`: single entry point to init all peripherals and return a `Board`.
- `src/st7789.rs`: LCD driver + init + brightness control.
- `src/display.rs`: UI rendering with embedded-graphics + u8g2 fonts.
- `src/touch.rs`: I2C init, scan, touch read.
- `src/mhz19b.rs`: MH-Z19B UART driver.
- `src/sht31.rs`: SHT31 I2C driver (single-shot, CRC).
- `src/wifi.rs`: Wi-Fi init + connect helpers.
- `src/ota.rs`: OTA check/download/apply logic (HTTP + ESP-IDF OTA).
- `src/main.rs`: uses `Board::init()`; reads SHT31 for temp/humidity; reads MH-Z19B for CO2; renders UI; touch hold in CO2 area triggers zero calibration; periodic OTA checks.

## Display Notes
- LCD is driven in landscape using MADCTL (0x36) = 0x68 (MV+MX+BGR).
- Panel offsets: LCD_X_GAP=0, LCD_Y_GAP=34.
- Framebuffer size: `LCD_W * LCD_H` (172x320), but UI uses `LCD_VIEW_W/LCD_VIEW_H` (320x172).
- Brightness: PWM via LEDC + WRCTRLD/WRDISBV commands.

## Wiring Docs
- See `wiring.md` for current sensor wiring.

## Build/Flash
- Typical: `cargo build` / `cargo run` with ESP-IDF toolchain.
- Optional scripts: `scripts/build.sh` and `scripts/flash.sh` (if you keep them).

## Toolchain
- `rust-toolchain.toml` pins the Rust toolchain used for ESP builds.
