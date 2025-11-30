# 🚀 Quick Start Guide - SD Card Filesystem

## TL;DR - Fast Track

```bash
# 1. Navigate to project
cd /Users/limsangyoon/Desktop/CMU/15-348/sd_test

# 2. Build
./build.sh
# OR
cargo build --release

# 3. Flash to Pico (choose one method below)
```

## Flashing Methods

### Method 1: USB Boot Mode (Recommended for beginners)

1. **Prepare Pico:**
   - Unplug Pico from USB
   - Hold the white BOOTSEL button on the Pico
   - Plug in USB cable while holding button
   - Release button - Pico appears as USB drive "RPI-RP2"

2. **Flash:**
   ```bash
   # If you have elf2uf2-rs installed
   cargo run --release
   
   # OR manually copy the UF2 file
   # The .uf2 file will be in target/thumbv6m-none-eabi/release/
   ```

### Method 2: Debug Probe (For development with logging)

```bash
# This will flash AND show live debug logs
probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/sd_test
```

## Installation (First Time Only)

```bash
# Add ARM target for Pico
rustup target add thumbv6m-none-eabi

# Install USB flasher (easier, no debug probe needed)
cargo install elf2uf2-rs --locked

# OR install debug probe tool (requires hardware debug probe)
cargo install probe-rs --features cli
```

## Hardware Setup Checklist

- [ ] SD card formatted as FAT32
- [ ] SD card module connected to Pico:
  - VCC  → Pin 36 (3.3V) ⚠️ NOT 5V!
  - GND  → Pin 38 (GND)
  - SCK  → Pin 24 (GPIO 18)
  - MOSI → Pin 25 (GPIO 19)
  - MISO → Pin 21 (GPIO 16)
  - CS   → Pin 22 (GPIO 17)
- [ ] SD card inserted in module
- [ ] Pico connected to computer via USB

## What to Expect

After flashing, the Pico will:
1. ✅ Initialize SD card
2. ✅ Read and parse FAT32 boot sector
3. ✅ List files in root directory
4. ✅ Read any RUST.TXT file if present
5. ✅ **Write a new file called HELLO.TXT**
6. ✅ Verify the write by reading directory again

## Verify It Worked

1. **Remove SD card** from Pico (power off first!)
2. **Insert into computer**
3. **Open root directory** - you should see:
   ```
   HELLO.TXT  (112 bytes)
   ```
4. **Open HELLO.TXT** - should contain:
   ```
   Hello from Pico OS!
   This file was written by the embedded filesystem.
   Rust is awesome for embedded systems!
   ```

## Debug Output (with probe-rs or probe-run)

You'll see logs like:
```
INFO  sd_test: boot
INFO  Initializing SD card...
INFO  SD init complete! high_capacity = true
INFO  === FAT32 Filesystem Info ===
INFO    Bytes per sector: 512
INFO    Sectors per cluster: 8
...
INFO  === Testing File Write ===
INFO  Writing file: HELLO.TXT
INFO  Successfully wrote HELLO.TXT!
```

## Common Commands

```bash
# Build only (no flash)
cargo build --release

# Build and flash via USB
cargo run --release

# Build and flash via debug probe
probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/sd_test

# Clean build artifacts
cargo clean

# Check code without building
cargo check

# Run the build script
./build.sh
```

## Troubleshooting Quick Fixes

| Problem | Quick Fix |
|---------|-----------|
| "No SD card detected" | Check wiring, especially CS pin |
| "Invalid boot signature" | Reformat SD as FAT32 |
| Can't flash | Hold BOOTSEL, reconnect USB |
| No debug output | Install probe-rs or use USB method |
| Build error | `cargo clean && cargo build --release` |

## Next: Modify the Code

After verifying it works, try:
- Change the message in HELLO.TXT
- Write multiple files
- Change the filename
- Increase file size

The write test is at the bottom of `main()` in `src/main.rs`:
```rust
let test_data = b"Your custom message here!";
match fat32_write_file(&mut spi, &mut cs, &info, "MYFILE.TXT", test_data, high_capacity) {
    // ...
}
```

---

**Ready to go?** Run `./build.sh` and follow the prompts! 🎉
