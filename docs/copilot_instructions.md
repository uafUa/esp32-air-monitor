# Copilot Instructions (c6-demo)

## Overview
This project targets an ESP32‑C6 Touch LCD board (1.47" ST7789 + touch). It displays CO2, temperature, humidity, and battery voltage using an on-device UI and reads sensors via UART/I2C. OTA updates are pulled over HTTP.

## Hardware / Pin Mapping
- LCD (SPI2): SCLK GPIO1, MOSI GPIO2, CS GPIO14, DC GPIO15, RST GPIO22, BL GPIO23
- Touch (I2C): SDA GPIO18, SCL GPIO19, RST GPIO20, INT GPIO21
- MH‑Z19B (UART0): TX GPIO16, RX GPIO17, 9600 baud
- SHT31 (I2C): shared bus GPIO18/19, default address 0x44

## Module Layout
- `src/board.rs`: one entry point to init peripherals. `Board::init()` returns lcd/i2c/mhz19b/sht31/wifi/battery.
- `src/st7789.rs`: ST7789 LCD driver (SPI), init, brightness control.
- `src/display.rs`: UI layout & drawing with embedded‑graphics + u8g2 fonts.
- `src/touch.rs`: touch controller I2C init, scan, read.
- `src/mhz19b.rs`: MH‑Z19B UART protocol (read, zero calibration, ABC on/off).
- `src/sht31.rs`: SHT31 I2C read (single‑shot high repeatability + CRC).
- `src/battery.rs`: ADC battery voltage reader.
- `src/wifi.rs`: Wi‑Fi init and reconnect helpers.
- `src/ota.rs`: OTA check/download/apply logic (HTTP + ESP‑IDF OTA).

## Display Details
- Panel size: 172x320 (LCD_W/LCD_H).
- UI is landscape: LCD_VIEW_W=320, LCD_VIEW_H=172.
- MADCTL=0x68 (MV+MX+BGR). Offsets: LCD_X_GAP=0, LCD_Y_GAP=34.
- Framebuffer is full panel size; render in landscape view.

## Runtime Logic
- SHT31 read every ~2s; values shown in UI (or "n/a" if missing).
- MH‑Z19B read every ~5s; CO2 shown in UI (or error state if missing).
- Touch in CO2 card for ~2s triggers zero calibration; “ZERO” is displayed briefly.
- ABC is disabled at boot in `Board::init()` via `mhz19b.set_abc(false)`.
- OTA periodically checks `OTA_BASE_URL` + `latest.txt` and flashes if a higher filename version is found.

## Build + OTA Artifacts
- `scripts/build-export.sh` increments `scripts/build-number.txt`, builds, then exports OTA.
- OTA build number comes from `OTA_BUILD` or `scripts/build-number.txt` via `build.rs`.
- `scripts/export-ota.sh` uses `espflash save-image` on the ELF (`target/.../c6-demo`) and writes `c6-co####.bin` + `latest.txt`.

## Notes
- UART0 is used for MH‑Z19B, so serial logs may interfere.
- Brightness uses PWM + WRCTRLD/WRDISBV.
- See `wiring.md` for wiring; see `docs/CONTEXT.md` for a concise project summary.

## MQTT
- Defaults: `MQTT_HOST=homeassistant.local`, `MQTT_PORT=1883`, `MQTT_PREFIX=c6-demo`.
- Topics:
  - Status: `<prefix>/status` (JSON telemetry).
  - Commands: `<prefix>/cmd` (`zero_calibrate`, `abc:on|off`, `brightness:NN`, `reboot`).
  - Availability: `<prefix>/availability` (`online`/`offline`, retained + LWT).
- HomeAssistant discovery is published at boot to `homeassistant/sensor/.../config`.
