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

This is a `std` (ESP-IDF based) Rust project. The ESP-IDF C SDK is downloaded and built automatically by the
`esp-idf-sys` build script. Run all commands from the repository root.

### Prerequisites with Nix

The [flake](./flake.nix) provides a dev shell (x86_64 Linux only) with the Xtensa Rust toolchain from esp-rs (pinned in
[`nix/esp-rust.nix`](./nix/esp-rust.nix)), `ldproxy`, `espflash` and the tools needed by the ESP-IDF build:

```sh
nix develop
```

### Prerequisites without Nix

Espressif's [Rust on ESP book](https://docs.espressif.com/projects/rust/book/getting-started/index.html) is the
reference for the toolchain setup. TL;DR:

1. Install the
   [ESP-IDF host dependencies](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/get-started/linux-macos-setup.html#step-1-install-prerequisites)

2. Install [Rust with `rustup`](https://rustup.rs/).

3. Install [`espup`](https://github.com/esp-rs/espup) and the Xtensa toolchain:

   ```sh
   cargo install espup --locked
   espup install
   ```

4. Activate the environment from `espup`. **This has to be done in every new shell**:

   ```sh
   . $HOME/export-esp.sh
   ```

5. Install the linker wrapper and the flashing tool:

   ```sh
   cargo install ldproxy --locked
   cargo install espflash --locked
   ```

   `ldproxy` is required by [`.cargo/config.toml`](./.cargo/config.toml) to use `espflash` as the cargo `runner`.

### ESP-IDF

The version is pinned in [`.cargo/config.toml`](./.cargo/config.toml):

```toml
[env]
ESP_IDF_VERSION = "v5.5.5"
```

On the first `cargo build`, the `esp-idf-sys` build script clones that tag into `.embuild/` and downloads the matching
ESP-IDF toolchains. This will take a few GBs of space and take a while the first time.

### Build

```sh
cargo build --release
```

### Flash and Monitor

Connect the board over USB, then:

```sh
cargo run --release
```

The `runner` configured in [`.cargo/config.toml`](./.cargo/config.toml) invokes `espflash flash --monitor`, which
flashes the firmware and then opens the serial monitor. To flash without the monitor, or from a script, give the port
explicitly:

```sh
espflash flash --port /dev/ttyACM0 target/xtensa-esp32s3-espidf/release/nts-player
```

### WSL

Pass the board through to WSL with [usbipd-win](https://github.com/dorssel/usbipd-win). In PowerShell:

```powershell
usbipd list                                         # find the board's bus ID
usbipd bind --busid <BUSID>                         # once, as administrator
usbipd attach --wsl --busid <BUSID> --auto-attach   # each session
```

Your WSL user must be in the `dialout` group to open `/dev/ttyACM0`.

## License

- Hardware (`kicad/`) and enclosure (`enclosure/`): [CC BY-NC-SA 4.0](LICENSE-CC-BY-NC-SA)
- Firmware (`firmware/`): dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option
