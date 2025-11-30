##  You're Ready to Flash!

### Hardware Setup Checklist:
- [ ] Debug probe connected to Pico (SWD + Ground)
- [ ] SD card module wired to Pico:
  - VCC  → 3.3V (Pin 36)
  - GND  → GND (Pin 38)
  - SCK  → GPIO 18 (Pin 24)
  - MOSI → GPIO 19 (Pin 25)
  - MISO → GPIO 16 (Pin 21)
  - CS   → GPIO 17 (Pin 22)
- [ ] SD card inserted and formatted as FAT32
- [ ] Pico powered (via USB or debug probe)

### Flash Command:
```bash
cargo run --release
```

### What You'll See:

```
    Finished `release` profile [optimized + debuginfo] target(s)
     Running `probe-rs run --chip RP2040 --protocol swd target/thumbv6m-none-eabi/release/sd_test`
      Erasing ✔ [00:00:00] [####################] 32.00 KiB/32.00 KiB @ 45.36 KiB/s
  Programming ✔ [00:00:03] [####################] 32.00 KiB/32.00 KiB @ 10.12 KiB/s
INFO  sd_test: boot
INFO  Initializing SD card...
INFO  SD init: send ≥80 clocks with CS high
INFO  SD init: CMD0
INFO    CMD0 attempt 0: r1 = 0x01
INFO  SD init: CMD8
INFO    CMD8 R7: 0 0 1 170
INFO  SD init: ACMD41 loop
INFO    ACMD41: card ready
INFO  SD init: CMD58
INFO    OCR: 192 255 128 0
INFO  SD init complete! high_capacity = true
INFO  Reading boot sector (LBA 0)...
INFO  Read block: lba=0, addr=0
INFO  Boot sector read successfully!
INFO  === FAT32 Filesystem Info ===
INFO    Bytes per sector:     512
INFO    Sectors per cluster:  8
INFO    Reserved sectors:     32
INFO    Number of FATs:       2
INFO    FAT size (sectors):   1234
INFO    Root dir cluster:     2
INFO    Total sectors:        15759360
INFO    FAT start LBA:        32
INFO    Data start LBA:       2500
INFO  =============================
INFO  Reading root directory (cluster 2)...
INFO    Root dir starts at LBA 2500
INFO  Read block: lba=2500, addr=2500
INFO  Root directory read successfully!
INFO  Listing directory entries:
INFO  Filesystem exploration complete.
INFO  === Testing File Write ===
INFO  Writing file: HELLO.TXT
INFO    File size: 112 bytes, clusters needed: 1
INFO    First cluster: 3
INFO  Write block: lba=2508, addr=2508
INFO    Wrote 112 bytes across 1 clusters
INFO  Read block: lba=2500, addr=2500
INFO  Write block: lba=2500, addr=2500
INFO  Directory entry added at offset 32
INFO  File written successfully!
INFO  Successfully wrote HELLO.TXT!
INFO  Verifying file was written...
INFO  Read block: lba=2500, addr=2500
INFO  Root directory after write:
INFO    File: HELLO   TXT, Size: 112, Cluster: 3
INFO  All tests complete. Entering idle loop.
```

### After Flashing:

1. **The program will run automatically** - you'll see all the logs above
2. **Wait for "All tests complete"** message
3. **Press Ctrl+C** to stop probe-rs (Pico keeps running)
4. **Power off Pico**
5. **Remove SD card**
6. **Insert SD card into your computer**
7. **Check for HELLO.TXT** in the root directory!

### Troubleshooting:

**"Error: No probe was found"**
- Check debug probe connection
- Make sure debug probe is powered
- Try unplugging and replugging the debug probe

**"Error: The firmware could not be flashed"**
- Pico might not be in the right state
- Try resetting the Pico
- Check SWD connections (SWDIO, SWCLK, GND)

**SD card errors in logs**
- Check wiring carefully
- Make sure SD card is FAT32 formatted
- Try a different SD card
- Verify 3.3V power to SD module (NOT 5V!)

**Ready? Run:** `cargo run --release`
