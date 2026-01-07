# Wiring

This project uses a DHT22 (AM2302) and an MH-Z19B CO2 sensor. The tables below
show the wiring used by the current code and a recommended UART mapping.

## DHT22 (single-wire, GPIO4)

ESP32-C6 uses an open-drain GPIO with a pull-up for the DHT22 data line.

```
ESP32-C6         DHT22
3V3   ---------> VCC (pin 1)
GPIO4 ---------> DATA (pin 2)
GND   ---------> GND (pin 4)
                 NC (pin 3) no connect
```

Notes:
- Add a 4.7k-10k pull-up from DATA to 3V3 if the sensor module does not include one.
- Keep the DATA line short and avoid running next to noisy power lines.

## MH-Z19B (UART1 example)

The MH-Z19B runs on 5V power and uses UART at 9600 baud.

```
ESP32-C6                         MH-Z19B
5V (USB/VBUS)  --------------->  VCC
GND            --------------->  GND
TXD (GPIO16)   --------------->  RXD
RXD (GPIO17)   <---------------  TXD
```

Notes:
- TXD/RXD on this board are GPIO16/GPIO17 (default UART). If you use them for the
  sensor, the USB serial console may be busy or unavailable.
- ESP32 UART is 3.3V logic. MH-Z19B TX is usually 3.3V and can connect directly
  to ESP32 RXD.
- If your MH-Z19B RX truly requires 5V logic, use a proper level shifter to
  translate 3.3V -> 5V (a divider would lower the level too much).
- If you use different GPIOs for UART, update the UART pin mapping in code.
