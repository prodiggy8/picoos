#!/bin/bash
# Build and flash script for SD card filesystem project

set -e  # Exit on error

echo "🦀 Building SD Card Filesystem for Raspberry Pi Pico..."
echo ""

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Build the project
echo -e "${BLUE}📦 Building project...${NC}"
cargo build --release

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ Build successful!${NC}"
    echo ""
    
    # Check binary size
    SIZE=$(ls -lh target/thumbv6m-none-eabi/release/sd_test | awk '{print $5}')
    echo -e "${BLUE}📊 Binary size: ${SIZE}${NC}"
    echo ""
    
    # Instructions for flashing
    echo -e "${YELLOW}📝 To flash to Pico:${NC}"
    echo ""
    echo "Option 1: USB Boot Mode (BOOTSEL)"
    echo "  1. Hold BOOTSEL button on Pico"
    echo "  2. Connect USB cable"
    echo "  3. Release BOOTSEL"
    echo "  4. Run: cargo run --release"
    echo ""
    echo "Option 2: With debug probe"
    echo "  Run: probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/sd_test"
    echo ""
    
    # Check if elf2uf2-rs is installed
    if command -v elf2uf2-rs &> /dev/null; then
        echo -e "${GREEN}✅ elf2uf2-rs is installed${NC}"
        echo "   You can use 'cargo run --release' to flash via USB"
    else
        echo -e "${YELLOW}⚠️  elf2uf2-rs not found${NC}"
        echo "   Install with: cargo install elf2uf2-rs --locked"
    fi
    echo ""
    
    # Check for probe-rs
    if command -v probe-rs &> /dev/null; then
        echo -e "${GREEN}✅ probe-rs is installed${NC}"
        echo "   You can use debug probe for flashing and logging"
    else
        echo -e "${YELLOW}⚠️  probe-rs not found${NC}"
        echo "   Install with: cargo install probe-rs --features cli"
    fi
    
else
    echo -e "${YELLOW}❌ Build failed!${NC}"
    exit 1
fi

echo ""
echo -e "${GREEN}🎉 Ready to flash!${NC}"
