# 🎯 How to Run Your SD Card Filesystem

## The Simplest Way (Recommended)

### Step 1: Build
```bash
cd /Users/limsangyoon/Desktop/CMU/15-348/sd_test
./build.sh
```

### Step 2: Flash to Pico
```bash
./flash.sh
```
This script will guide you through:
1. Putting Pico in BOOTSEL mode
2. Flashing the firmware
3. Running the SD card test

## Alternative: Manual Commands

### Option A: USB Flashing (No Debug Probe Needed)

**First time setup:**
```bash
# Install the USB flasher tool
cargo install elf2uf2-rs --locked
```

**Edit `.cargo/config.toml` to use USB mode:**
```toml
# Comment out probe-rs line:
#runner = "probe-rs run --chip RP2040 --protocol swd"

# Uncomment elf2uf2-rs line:
runner = "elf2uf2-rs -d"
```

**Then flash:**
1. Hold BOOTSEL button on Pico
2. Connect USB cable
3. Release BOOTSEL
4. Run: `cargo run --release`

### Option B: Debug Probe (See Live Logs)

**Setup:**
```bash
# Install probe-rs
cargo install probe-rs --features cli
```

**Your current config is set for this! Just run:**
```bash
cargo run --release
```

This will:
- Flash the firmware
- Attach to the Pico
- Show live debug output (all the `info!()` messages)

## What Each Method Does

| Method | Flashing | Debug Logs | Hardware Needed |
|--------|----------|------------|-----------------|
| **USB (elf2uf2-rs)** | ✅ Yes | ❌ No | Just USB cable |
| **Debug Probe (probe-rs)** | ✅ Yes | ✅ Yes | Debug probe or second Pico |

## Quick Command Reference

```bash
# Using the helper scripts (easiest!)
./build.sh              # Build the project
./flash.sh              # Flash via USB (guides you)

# Manual build
cargo build --release   # Build optimized binary
cargo build             # Build debug binary (larger)

# Manual flash
cargo run --release     # Flash using configured runner

# Clean and rebuild
cargo clean
cargo build --release

# Just compile-check (fast, no binary)
cargo check
```

## Expected Output

### During Flash (USB mode)
```
🔨 Building and flashing...
   Compiling sd_test v0.1.0
    Finished release [optimized + debuginfo] target(s)
     Running elf2uf2-rs -d target/thumbv6m-none-eabi/release/sd_test
✅ Flash complete!
```

### During Flash (Debug probe mode)
```
   Compiling sd_test v0.1.0
    Finished release [optimized + debuginfo] target(s)
     Running probe-rs run --chip RP2040 --protocol swd
INFO  sd_test: boot
INFO  Initializing SD card...
INFO  SD init: send ≥80 clocks with CS high
INFO  SD init: CMD0
INFO  SD init complete! high_capacity = true
...
INFO  Successfully wrote HELLO.TXT!
```

## Verify Success

After flashing:

1. **LED Behavior**: Pico should be running (built-in LED might blink depending on code)

2. **Check SD Card**:
   - Power off Pico
   - Remove SD card
   - Insert into PC
   - You should see `HELLO.TXT` in root directory!

3. **Read the File**:
   ```
   Hello from Pico OS!
   This file was written by the embedded filesystem.
   Rust is awesome for embedded systems!
   ```

## Troubleshooting

### "error: no probe was found"
- You're using probe-rs runner but no debug probe is connected
- Solution: Switch to USB mode (edit `.cargo/config.toml`)

### "error: elf2uf2-rs not found"
```bash
cargo install elf2uf2-rs --locked
```

### "No device found in BOOTSEL mode"
- Pico not in BOOTSEL mode properly
- Try again: unplug, hold BOOTSEL, plug in, release

### Build fails
```bash
# Make sure ARM target is installed
rustup target add thumbv6m-none-eabi

# Clean and rebuild
cargo clean
cargo build --release
```

## Current Configuration

Your `.cargo/config.toml` is currently set to use:
- **probe-rs** (debug probe method)

To switch to USB mode:
1. Edit `.cargo/config.toml`
2. Comment line 5: `#runner = "probe-rs run --chip RP2040 --protocol swd"`
3. Uncomment line 6: `runner = "elf2uf2-rs -d"`

## Recommended Workflow

**For Development (with debug probe):**
```bash
cargo run --release    # Flash and see live logs
```

**For Quick Testing (USB only):**
```bash
./flash.sh             # Guided USB flashing
```

**For Building Only:**
```bash
./build.sh             # Build and show instructions
```

---

## 🚀 Ready to Start?

**Absolute fastest path:**
1. Connect SD card to Pico (check wiring!)
2. Run: `./flash.sh`
3. Follow the prompts
4. Check SD card for HELLO.TXT

That's it! 🎉
