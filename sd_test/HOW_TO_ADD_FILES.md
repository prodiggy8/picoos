# 📁 How to Add Files and Directories to Your Filesystem

## 🎯 Three Ways to Add Files and Directories

---

## **Method 1: Edit the Code (Recommended for Custom Content)** ⭐

### Step 1: Open `src/main.rs`

Find the section after TEST 2 (around line 1200) and add your own files:

```rust
// ========================================================================
// YOUR CUSTOM FILES - Add your own files and directories here!
// ========================================================================
info!("\n=== Creating Custom Files ===");

// Example 1: Create a simple text file in root
let my_data = b"Hello from my custom file!";
match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", my_data, high_capacity) {
    Ok(()) => info!("✓ Created /MYFILE.TXT"),
    Err(e) => error!("✗ Failed: {}", e),
}

// Example 2: Create a directory
match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "PHOTOS", high_capacity) {
    Ok(_) => info!("✓ Created /PHOTOS directory"),
    Err(e) => error!("✗ Failed: {}", e),
}

// Example 3: Create a file in a subdirectory
let photo_info = b"Photo taken on 2025-11-29\nCamera: Pico OS\nResolution: 640x480";
match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/PHOTOS/INFO.TXT", photo_info, high_capacity) {
    Ok(()) => info!("✓ Created /PHOTOS/INFO.TXT"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

### Step 2: Build and Flash

```bash
cargo build --release
./flash.sh
```

### Step 3: Check Your SD Card

Remove the SD card and check on your computer - you'll see your new files!

---

## **Method 2: Use the Provided Functions** 🛠️

You can use these functions anywhere in your code:

### **Create a Directory**
```rust
// Create directory in root
fat32_create_directory(
    &mut spi, 
    &mut cs, 
    &fat_info, 
    fat_info.root_dir_cluster,  // Parent cluster (root)
    "MYDIR",                      // Directory name (8.3 format)
    high_capacity
)?;

// Create subdirectory
let parent_cluster = 3; // Get this from finding the parent directory
fat32_create_directory(
    &mut spi, 
    &mut cs, 
    &fat_info, 
    parent_cluster,     // Parent directory cluster
    "SUBDIR",           // Subdirectory name
    high_capacity
)?;
```

### **Write a File Using Path**
```rust
let data = b"Your file content here!";

// Write to root
fat32_write_file_at_path(
    &mut spi, 
    &mut cs, 
    &fat_info, 
    "/MYFILE.TXT",    // Full path
    data,             // File content
    high_capacity
)?;

// Write to subdirectory
fat32_write_file_at_path(
    &mut spi, 
    &mut cs, 
    &fat_info, 
    "/DOCS/NOTES.TXT",  // Path with directory
    data, 
    high_capacity
)?;
```

### **Write a File to a Specific Directory (if you know the cluster)**
```rust
let data = b"File content";
let dir_cluster = 5;  // Cluster number of the directory

fat32_write_file_in_dir(
    &mut spi, 
    &mut cs, 
    &fat_info, 
    dir_cluster,      // Directory cluster
    "FILE.TXT",       // Filename (8.3 format)
    data, 
    high_capacity
)?;
```

---

## **Method 3: Create Files from Your Computer** 💻

### After Running the Code Once:

1. **Run your program first** to create the initial filesystem structure
2. **Remove the SD card** from the Pico
3. **Insert it into your computer**
4. **Add files normally** using your file explorer:
   - Drag and drop files
   - Create new folders
   - Edit text files
5. **Safely eject** the SD card
6. **Put it back in the Pico**
7. **Run the program again** - it will read your files!

**Important:** 
- Files must use **8.3 filename format** (e.g., `FILENAME.TXT`, not `my-long-filename.txt`)
- Or the filesystem may not recognize them properly

---

## 📝 **Filename Rules (8.3 Format)**

Your filesystem uses the classic **8.3 format**:
- **Max 8 characters** for the filename
- **Max 3 characters** for the extension
- **No spaces** (use uppercase)
- **All uppercase** is automatic

### ✅ Valid Filenames:
```
README.TXT
PHOTO001.JPG
DATA.CSV
MYFILE.DAT
CONFIG.INI
LOG_2024.TXT
```

### ❌ Invalid Filenames:
```
my-long-filename.txt  ❌ Too long
file name.txt         ❌ Spaces not allowed
document.docx         ❌ Extension too long
verylongname.txt      ❌ Filename too long
```

---

## 🔥 **Quick Start Template**

Add this to your `main()` function after the filesystem initialization:

```rust
// ========================================================================
// CUSTOM FILE CREATION - Edit this section!
// ========================================================================

info!("\n=== Creating Your Custom Files ===");

// 1. Create your directory structure
let custom_dirs = ["PROJECTS", "NOTES", "LOGS"];
for dir_name in &custom_dirs {
    match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, dir_name, high_capacity) {
        Ok(_) => info!("✓ Created /{}", dir_name),
        Err(e) => error!("✗ Failed to create {}: {}", dir_name, e),
    }
}

// 2. Create your files
let files = [
    ("/PROJECTS/TODO.TXT", b"Project TODO list:\n- Feature 1\n- Feature 2\n"),
    ("/NOTES/IDEAS.TXT", b"Ideas:\n- Build a sensor system\n- Add WiFi support\n"),
    ("/LOGS/BOOT.LOG", b"System booted successfully\nTimestamp: 2025-11-29\n"),
];

for (path, content) in &files {
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, path, content, high_capacity) {
        Ok(()) => info!("✓ Created {} ({} bytes)", path, content.len()),
        Err(e) => error!("✗ Failed to create {}: {}", path, e),
    }
}

info!("✅ Custom files created!");
```

---

## 📊 **Example: Complete Custom Setup**

Here's a complete example you can copy-paste into your `main()`:

```rust
// After TEST 2, add this section:

// ========================================================================
// CUSTOM CONTENT - Your personal files!
// ========================================================================
info!("\n=== Creating Personal Workspace ===");

// Step 1: Create directory structure
match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "PROJECTS", high_capacity) {
    Ok(proj_cluster) => {
        info!("✓ Created /PROJECTS");
        
        // Create subdirectories
        fat32_create_directory(&mut spi, &mut cs, &fat_info, proj_cluster, "ACTIVE", high_capacity).ok();
        fat32_create_directory(&mut spi, &mut cs, &fat_info, proj_cluster, "ARCHIVE", high_capacity).ok();
        info!("✓ Created project subdirectories");
    }
    Err(e) => error!("✗ Failed: {}", e),
}

// Step 2: Create configuration file
let config = b"# Configuration File\n\
system_name=PicoOS\n\
version=1.0\n\
debug=true\n";

fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/CONFIG.INI", config, high_capacity).ok();

// Step 3: Create a log file
let log = b"[2025-11-29 12:00:00] System started\n\
[2025-11-29 12:00:01] SD card initialized\n\
[2025-11-29 12:00:02] FAT32 filesystem mounted\n";

fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/SYSTEM.LOG", log, high_capacity).ok();

// Step 4: Create a data file in subdirectory
let data = b"Sensor Data Log\n\
Timestamp,Temperature,Humidity\n\
12:00:00,23.5,45.2\n\
12:01:00,23.6,45.1\n";

fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/PROJECTS/DATA.CSV", data, high_capacity).ok();

info!("✅ Personal workspace created!");
```

---

## 🎨 **Interactive Example Code**

I'll add a custom section to your main.rs where you can easily add files:

