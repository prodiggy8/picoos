# 🔍 Filesystem Verification Guide

## How to Check Your SD Card Contents

### Method 1: Check on Your Computer (Recommended) 💻

1. **Stop your Pico** - Unplug it from power/USB
2. **Remove the SD card** from the Pico
3. **Insert into your computer** using an SD card reader
4. **Open the SD card** in your file explorer

You should see this structure:
```
SD Card Root/
├── README.TXT           (159 bytes)
├── DOCS/                (directory)
│   ├── GUIDE.TXT        (~100 bytes)
│   ├── REPORT.TXT       (932 bytes - multi-cluster)
│   └── REPORTS/         (empty subdirectory)
└── MUSIC/               (empty directory)
```

5. **Open README.TXT** - Should contain:
```
Welcome to Pico OS Filesystem!

This filesystem supports:
- Multi-cluster files
- Subdirectories
- Path navigation

Built with Rust!
```

6. **Navigate to DOCS folder** - You should see `GUIDE.TXT`, `REPORT.TXT`, and `REPORTS/` subdirectory

---

### Method 2: Check Serial Monitor Output 📟

When you run the program with `./flash.sh`, the serial output will show:

```
INFO  === TEST 4: Listing Directories ===
INFO  Root directory:
INFO    [DIR ] DOCS       - 0 bytes, cluster 3
INFO    [DIR ] MUSIC      - 0 bytes, cluster 5
INFO    [FILE] README  TXT - 159 bytes, cluster 2

INFO  /DOCS directory:
INFO    [FILE] GUIDE   TXT - 101 bytes, cluster 4
INFO    [FILE] REPORT  TXT - 932 bytes, cluster 6
INFO    [DIR ] REPORTS     - 0 bytes, cluster 7
```

**NEW in this version:** Test 5 will verify each file/directory exists:

```
INFO  === TEST 5: Verifying Filesystem Structure ===
INFO    ✓ FILE exists: /README.TXT
INFO    ✓ DIR exists: /DOCS
INFO    ✓ FILE exists: /DOCS/GUIDE.TXT
INFO    ✓ FILE exists: /DOCS/REPORT.TXT
INFO    ✓ DIR exists: /DOCS/REPORTS
INFO    ✓ DIR exists: /MUSIC

INFO  ✅ All files and directories verified successfully!
```

---

### Method 3: Use Command Line Tools (macOS/Linux) 🖥️

If your SD card is mounted at `/Volumes/SDCARD` (or similar):

```bash
# List root directory
ls -lh /Volumes/SDCARD/

# Should show:
# drwxr-xr-x  DOCS/
# drwxr-xr-x  MUSIC/
# -rw-r--r--  README.TXT

# List DOCS directory
ls -lh /Volumes/SDCARD/DOCS/

# Should show:
# drwxr-xr-x  REPORTS/
# -rw-r--r--  GUIDE.TXT
# -rw-r--r--  REPORT.TXT

# Display file contents
cat /Volumes/SDCARD/README.TXT
cat /Volumes/SDCARD/DOCS/GUIDE.TXT

# Check file sizes
ls -lh /Volumes/SDCARD/DOCS/REPORT.TXT
# Should be around 932 bytes
```

---

### Method 4: Use Windows Command Prompt 🪟

If your SD card is drive `E:`:

```cmd
REM List root directory
dir E:\

REM Should show:
REM <DIR>  DOCS
REM <DIR>  MUSIC
REM       README.TXT

REM List DOCS directory
dir E:\DOCS\

REM Display file contents
type E:\README.TXT
type E:\DOCS\GUIDE.TXT

REM Check specific file
dir E:\DOCS\REPORT.TXT
```

---

## 🐛 Troubleshooting

### "I don't see the files!"

**Possible causes:**

1. **SD card wasn't properly formatted as FAT32**
   - Solution: Format SD card as FAT32 on your computer first

2. **Program crashed before completing writes**
   - Check serial output for errors
   - Look for `ERROR` messages

3. **SD card connection issues**
   - Verify wiring: MISO=GP16, CS=GP17, SCK=GP18, MOSI=GP19
   - Check for loose connections

4. **Wrong SD card slot/reader**
   - Make sure you're checking the correct drive/volume

### "Files are corrupted or wrong size"

1. **Multi-cluster issues** - Check if `REPORT.TXT` is the full 932 bytes
2. **FAT table corruption** - Try reformatting and running again
3. **Buffer overflows** - Check serial logs for any errors

### "Directories are empty"

- Empty directories (like `MUSIC/` and `REPORTS/`) are expected!
- They should still appear in the file listing, just with no files inside

---

## ✅ Expected Results

After running the program successfully, you should have:

| Path | Type | Size (bytes) | Notes |
|------|------|--------------|-------|
| `/README.TXT` | File | 159 | Single cluster |
| `/DOCS/` | Directory | 0 | Contains 3 items |
| `/DOCS/GUIDE.TXT` | File | ~101 | Single cluster |
| `/DOCS/REPORT.TXT` | File | 932 | **Multi-cluster** (tests cluster chaining) |
| `/DOCS/REPORTS/` | Directory | 0 | Empty subdirectory |
| `/MUSIC/` | Directory | 0 | Empty directory |

---

## 🚀 Quick Test

Run the updated program:

```bash
./flash.sh
```

Look for this line in the output:
```
✅ All files and directories verified successfully!
```

If you see that, everything is working! 🎉

---

## 📸 What Success Looks Like

**Serial Monitor Output:**
```
INFO  🎉 All filesystem tests complete!
INFO  Features demonstrated:
INFO    ✓ Multi-cluster file reading/writing
INFO    ✓ Directory creation
INFO    ✓ Subdirectory navigation
INFO    ✓ Path-based file operations
INFO    ✓ Directory listing
INFO    ✓ File/directory verification
```

**Windows Explorer / macOS Finder:**
- You can double-click into DOCS folder
- You can open README.TXT and read the welcome message
- REPORT.TXT should be readable and contain Lorem Ipsum text
- REPORTS and MUSIC folders should be empty but visible

---

## 💡 Tips

- **Always safely eject** the SD card from your Pico before removing it
- **Format as FAT32** - not exFAT or NTFS
- **Use a quality SD card** - cheap cards can have reliability issues
- **Check serial logs first** - errors will appear there before filesystem corruption
- **Keep backups** - test on a dedicated SD card, not one with important data

---

Need help? Check the serial output for specific error messages and verify your SD card is properly formatted as FAT32!
