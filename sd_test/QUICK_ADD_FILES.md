# 🚀 Quick Reference: Add Files & Directories

## Copy-Paste Code Snippets

---

## 📁 **Create a Directory**

```rust
// In root directory
match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "MYDIR", high_capacity) {
    Ok(cluster) => {
        info!("✓ Created /MYDIR (cluster {})", cluster);
    }
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

## 📄 **Create a File in Root**

```rust
let content = b"Your file content here!";
match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", content, high_capacity) {
    Ok(()) => info!("✓ Created /MYFILE.TXT"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

## 📂 **Create a File in Subdirectory**

```rust
// First create the directory
fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "DATA", high_capacity).ok();

// Then create file in it
let data = b"Sensor readings: 23.5C, 45% humidity";
match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/DATA/SENSOR.TXT", data, high_capacity) {
    Ok(()) => info!("✓ Created /DATA/SENSOR.TXT"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

## 🗂️ **Create Multiple Directories**

```rust
let directories = ["PROJECTS", "PHOTOS", "LOGS", "CONFIG"];
for dir_name in &directories {
    match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, dir_name, high_capacity) {
        Ok(_) => info!("✓ Created /{}", dir_name),
        Err(e) => error!("✗ Failed to create {}: {}", dir_name, e),
    }
}
```

---

## 📝 **Create Multiple Files**

```rust
let files = [
    ("/TODO.TXT", b"Task 1\nTask 2\nTask 3\n"),
    ("/NOTES.TXT", b"Important notes here"),
    ("/CONFIG.INI", b"setting1=value1\nsetting2=value2\n"),
];

for (path, content) in &files {
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, path, content, high_capacity) {
        Ok(()) => info!("✓ Created {} ({} bytes)", path, content.len()),
        Err(e) => error!("✗ Failed: {}", e),
    }
}
```

---

## 🌲 **Create Directory Tree**

```rust
// Create main directory
match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "PROJECT", high_capacity) {
    Ok(proj_cluster) => {
        info!("✓ Created /PROJECT");
        
        // Create subdirectories
        fat32_create_directory(&mut spi, &mut cs, &fat_info, proj_cluster, "SRC", high_capacity).ok();
        fat32_create_directory(&mut spi, &mut cs, &fat_info, proj_cluster, "DOCS", high_capacity).ok();
        fat32_create_directory(&mut spi, &mut cs, &fat_info, proj_cluster, "TESTS", high_capacity).ok();
        
        info!("✓ Created subdirectories");
    }
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

## 📊 **Real-World Example: Data Logging**

```rust
// Create logging system
info!("Setting up data logging system...");

// Create LOG directory
match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "LOGS", high_capacity) {
    Ok(_) => info!("✓ Created /LOGS"),
    Err(e) => error!("✗ Failed: {}", e),
}

// Create log file
let log_data = b"[2025-11-29 12:00:00] System boot\n\
[2025-11-29 12:00:01] SD card OK\n\
[2025-11-29 12:00:02] FAT32 mounted\n\
[2025-11-29 12:00:03] Ready\n";

match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/LOGS/BOOT.LOG", log_data, high_capacity) {
    Ok(()) => info!("✓ Created boot log"),
    Err(e) => error!("✗ Failed: {}", e),
}

// Create sensor data file
let sensor_data = b"Timestamp,Temp,Humidity\n\
12:00:00,23.5,45.2\n\
12:01:00,23.6,45.1\n\
12:02:00,23.4,45.3\n";

match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/LOGS/SENSOR.CSV", sensor_data, high_capacity) {
    Ok(()) => info!("✓ Created sensor data"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

## 🎮 **Where to Add Your Code**

### Option 1: In the Custom Section (Recommended)
Open `src/main.rs` and find this section (around line 1200):

```rust
// ========================================================================
// 🎨 YOUR CUSTOM FILES - Add your own files and directories here!
// ========================================================================
// Uncomment and edit the examples below:

/*
info!("\n=== Creating Custom Files ===");

// YOUR CODE HERE!

*/
```

**Uncomment the block** (remove `/*` and `*/`) and add your code inside!

### Option 2: Create a New Test Section
Add anywhere before the final summary:

```rust
// ========================================================================
// MY CUSTOM TEST
// ========================================================================
info!("\n=== My Custom Files ===");

// Your code here
```

---

## 🔨 **Full Working Example**

Here's a complete example you can paste into the custom section:

```rust
info!("\n=== Creating My Personal Files ===");

// 1. Create workspace structure
let dirs = ["WORK", "PERSONAL", "ARCHIVE"];
for dir in &dirs {
    fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, dir, high_capacity).ok();
}

// 2. Create work files
let work_files = [
    ("/WORK/TODO.TXT", b"1. Review code\n2. Write tests\n3. Deploy\n"),
    ("/WORK/NOTES.TXT", b"Meeting notes:\n- Feature X approved\n- Release on Friday\n"),
];

for (path, content) in &work_files {
    fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, path, content, high_capacity).ok();
}

// 3. Create personal files
let personal = b"Personal Notes\n\nIdeas for weekend project:\n- Add WiFi\n- Build sensor array\n";
fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/PERSONAL/IDEAS.TXT", personal, high_capacity).ok();

// 4. Create archive with timestamp
let archive = b"Archive created: 2025-11-29\nOld project files stored here.\n";
fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/ARCHIVE/README.TXT", archive, high_capacity).ok();

info!("✅ Personal workspace ready!");
```

---

## ✅ **Testing Your Files**

After adding files, verify them:

```rust
// List what you created
info!("\n=== Verifying My Files ===");

// Check root directory
fat32_list_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, high_capacity).ok();

// Read back a file to verify
let mut buf = [0u8; 256];
match fat32_read_file_at_path(&mut spi, &mut cs, &fat_info, "/WORK/TODO.TXT", &mut buf, high_capacity) {
    Ok(size) => {
        info!("✓ Read /WORK/TODO.TXT: {} bytes", size);
        info!("  Content: {=[u8]:a}", &buf[..size]);
    }
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

## 🎯 **Next Steps**

1. **Edit `src/main.rs`** - Find the "YOUR CUSTOM FILES" section
2. **Uncomment the examples** or add your own code
3. **Build**: `cargo build --release`
4. **Flash**: `./flash.sh`
5. **Check logs** for ✓ or ✗ indicators
6. **Remove SD card** and verify on your computer!

---

## 💡 **Pro Tips**

- **Keep filenames short**: Max 8 chars + 3 char extension
- **Use uppercase**: It's automatic but keeps things consistent
- **Test incrementally**: Add one file, test, then add more
- **Check the logs**: They show exactly what succeeded/failed
- **Verify on PC**: Always check the SD card on your computer to confirm

Happy coding! 🚀
