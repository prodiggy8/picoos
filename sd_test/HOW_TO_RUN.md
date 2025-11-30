# How to Run the FAT32 Filesystem on Raspberry Pi Pico

## Prerequisites

### Hardware Setup
1. **Raspberry Pi Pico** (RP2040)
2. **SD Card** formatted as FAT32 (8GB or smaller recommended)
3. **SD Card Module** or SD card breakout board
4. **Wiring** (according to your pin configuration):

```
SD Card Module → Raspberry Pi Pico
--------------------------------
VCC  → 3.3V (Pin 36)
GND  → GND  (Pin 38)
SCK  → GPIO 18 (Pin 24)
MOSI → GPIO 19 (Pin 25)
MISO → GPIO 16 (Pin 21)
CS   → GPIO 17 (Pin 22)
```

### Software Prerequisites
- Rust toolchain installed
- `cargo` and `rustup`
- ARM Cortex-M0+ target installed
- `probe-rs` or `elf2uf2-rs` for flashing

## Step 1: Install Rust Tools (if not already done)

```bash
# Install Rust (if needed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add ARM Cortex-M0+ target for RP2040
rustup target add thumbv6m-none-eabi

# Install flashing tools (choose one)
# Option A: probe-rs (for debug probe)
cargo install probe-rs --features cli

# Option B: elf2uf2-rs (for USB boot mode)
cargo install elf2uf2-rs --locked
```

## Step 2: Prepare the SD Card

1. **Format SD Card as FAT32**:
   - On macOS: Use Disk Utility, format as "MS-DOS (FAT)"
   - On Linux: Use `mkfs.vfat -F 32 /dev/sdX1`
   - On Windows: Right-click drive → Format → FAT32

2. **Optional: Create a test file**:
   - Create a file named `RUST.TXT` in the root directory
   - Add some text content to test reading
   - Safely eject the SD card

3. **Insert SD card into your SD card module**

## Step 3: Build the Project

Navigate to the sd_test directory and build:

```bash
cd /Users/limsangyoon/Desktop/CMU/15-348/sd_test

# Build in release mode (optimized)
cargo build --release

# Or build in debug mode (larger binary, easier debugging)
cargo build
```

## Step 4: Flash to Pico

### Option A: Using USB Boot Mode (Easiest)

1. **Enter BOOTSEL mode**:
   - Disconnect Pico from USB
   - Hold the BOOTSEL button on the Pico
   - Connect USB cable while holding BOOTSEL
   - Release button - Pico appears as USB drive

2. **Flash the firmware**:
```bash
# This will build and flash in one command
cargo run --release

# Or manually:
cargo build --release
elf2uf2-rs target/thumbv6m-none-eabi/release/sd_test
# Then copy the generated .uf2 file to the Pico drive
```

### Option B: Using Debug Probe (probe-rs)

If you have a debug probe (like another Pico running picoprobe):

```bash
# Flash and run
cargo run --release

# Or just flash without attaching
probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/sd_test
```

## Step 5: View Debug Output

To see the `defmt` log output, you need a debug probe connection:

```bash
# Using probe-rs to view logs
probe-rs run --chip RP2040 target/thumbv6m-none-eabi/release/sd_test
```

You should see output like:
```
sd_test: boot
Initializing SD card...
SD init: send ≥80 clocks with CS high
SD init: CMD0
  CMD0 attempt 0: r1 = 0x01
SD init: CMD8
  CMD8 R7: 0 0 1 170
SD init: ACMD41 loop
  ACMD41: card ready
SD init: CMD58
  OCR: 192 255 128 0
SD init complete! high_capacity = true
Reading boot sector (LBA 0)...
Boot sector read successfully!
=== FAT32 Filesystem Info ===
  Bytes per sector:     512
  Sectors per cluster:  8
  Reserved sectors:     32
  Number of FATs:       2
  FAT size (sectors):   1234
  Root dir cluster:     2
  Total sectors:        15759360
  FAT start LBA:        32
  Data start LBA:       2500
=============================
Reading root directory (cluster 2)...
Listing directory entries:
...
=== Testing File Write ===
Writing file: HELLO.TXT
  File size: 112 bytes, clusters needed: 1
  First cluster: 3
  Wrote 112 bytes across 1 clusters
Directory entry added at offset 32
File written successfully!
Successfully wrote HELLO.TXT!
Verifying file was written...
Root directory after write:
  File: HELLO   TXT, Size: 112, Cluster: 3
All tests complete. Entering idle loop.
```

## Step 6: Verify the File on PC

1. **Safely remove SD card from Pico** (power off first!)
2. **Insert SD card into your computer**
3. **Check root directory** - you should see `HELLO.TXT`
4. **Open the file** - it should contain:
   ```
   Hello from Pico OS!
   This file was written by the embedded filesystem.
   Rust is awesome for embedded systems!
   ```

## Troubleshooting

### "No SD card detected"
- Check wiring connections
- Verify SD card is properly inserted in module
- Try a different SD card
- Check that SD card module has power (3.3V)
- Verify CS pin is connected and configured correctly

### "Invalid boot signature"
- SD card might not be formatted as FAT32
- Try reformatting the card
- Make sure you're using FAT32, not exFAT or NTFS

### Build errors
```bash
# Clean and rebuild
cargo clean
cargo build --release

# Update dependencies
cargo update
```

### No debug output visible
- You need a debug probe for `defmt` output
- Alternative: Use `rtt-target` for RTT output with probe-rs
- Or: Modify code to use UART output instead

### "Write data rejected" or write errors
- SD card might be write-protected (check physical switch)
- Card might be corrupted - try reformatting
- Increase SPI speed after init might help reliability
- Some cheap SD cards have poor write support

## Performance Tips

### Increase SPI Speed After Init

The code initializes at 400kHz for compatibility. After successful init, you can speed it up:

```rust
// After sd_init succeeds, before reading/writing files:
drop(spi); // Drop the old SPI instance
let spi = hal::spi::Spi::<_, _, _, 8>::new(pac.SPI0, (mosi, miso, sck)).init(
    &mut pac.RESETS,
    clocks.peripheral_clock.freq(),
    10.MHz(),  // Much faster!
    MODE_0,
);
```

## Next Steps

Once this works, you can:
1. Add more files to test multi-file operations
2. Implement file deletion
3. Add subdirectory support
4. Create a shell interface for file operations
5. Integrate with your Pico OS project

## Quick Reference Commands

```bash
# Build
cargo build --release

# Flash via USB
cargo run --release

# Clean build
cargo clean && cargo build --release

# Check code without building
cargo check

# Format code
cargo fmt

# Run clippy for lints
cargo clippy
```

## Common Issues and Solutions

| Issue | Solution |
|-------|----------|
| Pico not detected in BOOTSEL mode | Try different USB cable, check USB port |
| Build fails with "target not found" | Run `rustup target add thumbv6m-none-eabi` |
| Out of memory errors | Use `--release` flag for smaller binary |
| SD card not responding | Check voltage (must be 3.3V not 5V!) |
| Corrupted filesystem | Reformat SD card, test with small file first |

Happy hacking! 🚀
