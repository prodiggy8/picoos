#!/bin/bash
# Quick flash script for USB boot mode

set -e

echo " Quick Flash to Pico via USB"
echo ""
echo " Instructions:"
echo "  1. Hold BOOTSEL button on your Pico"
echo "  2. Connect USB cable (while holding button)"
echo "  3. Release BOOTSEL button"
echo "  4. Press Enter to continue..."
echo ""
read -p "Ready? Press Enter when Pico is in BOOTSEL mode..."

echo ""
echo "🔨 Building and flashing..."
cargo run --release

if [ $? -eq 0 ]; then
    echo ""
    echo " Flash complete!"
    echo ""
    echo "Next steps:"
    echo "  - Pico will reboot automatically"
    echo "  - SD card operations will start"
    echo "  - Check SD card on your PC to see HELLO.TXT"
    echo ""
    echo "To see debug logs:"
    echo "  - Use a debug probe with: probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/sd_test"
else
    echo ""
    echo " Flash failed!"
    echo ""
    echo "Common issues:"
    echo "  - Pico not in BOOTSEL mode (doesn't appear as USB drive)"
    echo "  - elf2uf2-rs not installed: cargo install elf2uf2-rs --locked"
    echo "  - Wrong runner in .cargo/config.toml"
fi
