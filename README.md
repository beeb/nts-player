# NTS Web Radio Player

This is a standalone Wi-Fi connected web-radio player based on ESP-32. It was designed for the
[NTS.live](https://www.nts.live/) radio which has 18 live stations (2 main channels + 16 infinite mixtapes).

It's built around the ESP32-S3 and a PCM5122 DAC, with firmware written in Rust and a 3D-printed enclosure. The design
is made to be very simple to interact with: a single encoder with a large knob to select the station and change the
volume.

A ring of LEDs show which station is selected and the current volume level. The click action of the encoder switches
between the modes and allows to put the device to sleep.

Note: this device is not affiliated with NTS or Atonemo which have created a standalone player for NTS. Support your
favorite radio by [becoming a supporter](https://www.nts.live/supporters) or
[buying from the shop](https://shop.nts.live/).

## Features

- Streams MP3 web radio over HTTPS (e.g. NTS: 256 kbps, 44.1 kHz stereo)
- 18 stations on one detented encoder, one click per station
- 18-LED ring around the knob shows the current station (and volume in volume mode)
- Push the knob to toggle station / volume mode, long-press to sleep, press to wake
- Line out at up to 2.1 Vrms on 3.5 mm and RCA, wired in parallel
  - This is meant as line out, not as headphones output, it expects a load of 1 kOhm or more.
- Low standby current: LED ring and DAC are power-switched in deep sleep
- Powered over USB-C (5 V)

## Repository Layout

```text
firmware/   Rust firmware (ESP-IDF, std)
kicak/      KiCad schematic and PCB for the carrier board
enclosure/  3d-printable enclosure parts
docs/       Misc. information
```

## Pin Map

| Function                                              | GPIO      |
| ----------------------------------------------------- | --------- |
| DAC XSMT (soft mute, 10k pull-down)                   | 4         |
| I2S BCK / DIN / LRCK                                  | 5 / 6 / 7 |
| I2C SDA / SCL (4.7k pull-ups to always-on 3V3)        | 8 / 9     |
| Encoder A / B                                         | 10 / 11   |
| Encoder push (wake pin, 10k pull-up to always-on 3V3) | 12        |
| LED ring power enable (10k pull-down)                 | 13        |
| DAC LDO enable (10k pull-down)                        | 14        |
| LED data (via level shifter)                          | 21        |

Avoided: strapping pins 0, 3, 45, 46; GPIO35–37 (octal PSRAM); GPIO19/20 (USB); GPIO38 (onboard RGB LED); GPIO43/44
(UART0).

## Building and Flashing

TODO

## License

- Hardware (`kicad/`) and enclosure (`enclosure/`): [CC BY-NC-SA 4.0](LICENSE-CC-BY-NC-SA)
- Firmware (`firmware/`): dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option
