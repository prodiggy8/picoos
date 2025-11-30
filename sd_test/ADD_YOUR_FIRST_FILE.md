# ⚡ Super Simple: Add Your First File

## 🎯 3-Step Guide

### **Step 1: Open `src/main.rs`**

Look for **line 1218** - you'll see this comment:
```rust
// 🎨 YOUR CUSTOM FILES - Add your own files and directories here!
```

### **Step 2: Uncomment the Example Code**

You'll see a block that starts with `/*` and ends with `*/`:

**BEFORE (commented out):**
```rust
/*
info!("\n=== Creating Custom Files ===");

// Example 1: Create a simple text file
let my_data = b"Hello! This is my custom file.";
match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", my_data, high_capacity) {
    Ok(()) => info!("✓ Created /MYFILE.TXT"),
    Err(e) => error!("✗ Failed: {}", e),
}
*/
```

**AFTER (uncommented):**
```rust
info!("\n=== Creating Custom Files ===");

// Example 1: Create a simple text file
let my_data = b"Hello! This is my custom file.";
match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", my_data, high_capacity) {
    Ok(()) => info!("✓ Created /MYFILE.TXT"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

Just **delete** the `/*` at the start and `*/` at the end!

### **Step 3: Build and Flash**

```bash
./flash.sh
```

That's it! Your file `MYFILE.TXT` will be on the SD card! 🎉

---

## 🔧 Add Your Own Custom File

Replace the example with your own content:

```rust
info!("\n=== Creating Custom Files ===");

// My custom file
let my_data = b"PUT YOUR TEXT HERE!";
match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/YOURNAME.TXT", my_data, high_capacity) {
    Ok(()) => info!("✓ Created /YOURNAME.TXT"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

## 📂 Add a Directory Too

```rust
info!("\n=== Creating Custom Files ===");

// Create a directory
match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "MYFOLDER", high_capacity) {
    Ok(_) => info!("✓ Created /MYFOLDER"),
    Err(e) => error!("✗ Failed: {}", e),
}

// Create a file inside it
let data = b"File in my folder!";
match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/MYFOLDER/FILE.TXT", data, high_capacity) {
    Ok(()) => info!("✓ Created /MYFOLDER/FILE.TXT"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

## ✅ Verify It Worked

After running `./flash.sh`, check the serial output for:
```
INFO  === Creating Custom Files ===
INFO  ✓ Created /MYFILE.TXT
```

Then remove the SD card and check it on your computer - you'll see the files! 📁

---

## 🎨 More Examples

**Create a log file:**
```rust
let log = b"[12:00:00] System started\n[12:00:01] Ready\n";
fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/SYSTEM.LOG", log, high_capacity).ok();
```

**Create a config file:**
```rust
let config = b"name=MyPico\nversion=1.0\ndebug=true\n";
fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/CONFIG.INI", config, high_capacity).ok();
```

**Create a CSV data file:**
```rust
let csv = b"Time,Value\n12:00,23.5\n12:01,23.6\n";
fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/DATA.CSV", csv, high_capacity).ok();
```

---

## 💡 Remember

- **Filenames**: Max 8 characters + 3 for extension (e.g., `FILENAME.TXT`)
- **Content**: Use `b"text"` for byte strings
- **Path**: Start with `/` for root directory
- **Subdirectories**: Create directory first, then files

That's all you need to know! Happy coding! 🚀
