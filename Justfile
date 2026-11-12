target := "target/xtensa-esp32s3-espidf"

# list the recipes
default:
    @just --list

# build the release firmware
build:
    cargo build --release

# build, flash over USB and open the serial monitor
flash:
    cargo run --release

# open the serial monitor
monitor:
    espflash monitor

# save the release firmware as nts-player-<version>.bin, to send for updates over Wi-Fi
export: build
    espflash save-image --chip esp32s3 {{target}}/release/nts-player \
        nts-player-$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "nts-player") | .version').bin

# check formatting and lints
check:
    cargo fmt --check
    cargo clippy --workspace --all-targets --all-features

# command used by the cargo runner
[private]
flash-elf elf:
    espflash flash --baud 921600 --monitor \
        --bootloader "$(dirname '{{elf}}')/bootloader.bin" \
        --partition-table firmware/partitions.csv \
        --target-app-partition ota_0 \
        --erase-parts otadata \
        '{{elf}}'
