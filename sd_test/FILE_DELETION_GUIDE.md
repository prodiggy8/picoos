# 🗑️ File Deletion Guide

Your FAT32 filesystem now supports **file and directory deletion**! This guide shows you how to use the deletion features.

---

## 🎯 **Available Deletion Functions**

### **1. Delete a File (Path-Based)** ⭐ **EASIEST**

```rust
fat32_delete_file_at_path(
    &mut spi,
    &mut cs,
    &fat_info,
    "/path/to/file.txt",  // Full path to file
    high_capacity
)?;
```

**Example:**
```rust
// Delete a file in root
match fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, "/OLDFILE.TXT", high_capacity) {
    Ok(()) => info!("✓ File deleted"),
    Err(e) => error!("✗ Failed: {}", e),
}

// Delete a file in subdirectory
match fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, "/DOCS/OLD.TXT", high_capacity) {
    Ok(()) => info!("✓ File deleted"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

### **2. Delete a File (Directory-Based)**

```rust
fat32_delete_file(
    &mut spi,
    &mut cs,
    &fat_info,
    dir_cluster,     // Directory cluster containing the file
    "FILENAME.TXT",  // Filename to delete
    high_capacity
)?;
```

**Example:**
```rust
// Delete from root directory
let root_cluster = fat_info.root_dir_cluster;
match fat32_delete_file(&mut spi, &mut cs, &fat_info, root_cluster, "TEMP.TXT", high_capacity) {
    Ok(()) => info!("✓ Deleted TEMP.TXT from root"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

### **3. Delete an Empty Directory (Path-Based)** ⭐ **RECOMMENDED**

```rust
fat32_delete_directory_at_path(
    &mut spi,
    &mut cs,
    &fat_info,
    "/path/to/directory",
    high_capacity
)?;
```

**Important:** Directory must be empty (no files or subdirectories)!

**Example:**
```rust
// Delete an empty directory
match fat32_delete_directory_at_path(&mut spi, &mut cs, &fat_info, "/TEMP", high_capacity) {
    Ok(()) => info!("✓ Directory deleted"),
    Err(e) => error!("✗ Failed: {}", e),  // Will fail if not empty
}
```

---

### **4. Delete a Directory (Cluster-Based)**

```rust
fat32_delete_directory(
    &mut spi,
    &mut cs,
    &fat_info,
    parent_cluster,  // Parent directory cluster
    "DIRNAME",       // Directory name to delete
    high_capacity
)?;
```

---

## 📋 **What Happens When You Delete?**

### **File Deletion Process:**

1. **Find the file** in the directory
2. **Free all clusters** in the FAT chain (mark as 0x00000000)
3. **Mark directory entry** as deleted (first byte = 0xE5)
4. **Space is now available** for new files

### **Directory Deletion Process:**

1. **Verify directory is empty** (only . and .. entries)
2. **Free the directory cluster**
3. **Mark directory entry as deleted**

---

## ✅ **Complete Examples**

### **Example 1: Delete Temporary Files**

```rust
info!("Cleaning up temporary files...");

let temp_files = [
    "/TEMP.TXT",
    "/CACHE.DAT",
    "/LOGS/OLD.LOG",
];

for path in &temp_files {
    match fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, path, high_capacity) {
        Ok(()) => info!("✓ Deleted {}", path),
        Err(e) => error!("✗ Failed to delete {}: {}", path, e),
    }
}
```

---

### **Example 2: Delete and Recreate a File**

```rust
// Delete old version
fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, "/DATA.TXT", high_capacity).ok();

// Write new version
let new_data = b"Updated data here!";
match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/DATA.TXT", new_data, high_capacity) {
    Ok(()) => info!("✓ File updated"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

### **Example 3: Clean Up Empty Directories**

```rust
// First, delete all files in the directory
fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, "/TEMP/FILE1.TXT", high_capacity).ok();
fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, "/TEMP/FILE2.TXT", high_capacity).ok();

// Now delete the empty directory
match fat32_delete_directory_at_path(&mut spi, &mut cs, &fat_info, "/TEMP", high_capacity) {
    Ok(()) => info!("✓ Cleaned up /TEMP directory"),
    Err(e) => error!("✗ Failed: {}", e),
}
```

---

### **Example 4: Safe Deletion with Verification**

```rust
let file_to_delete = "/OLDDATA.TXT";

// Check if file exists first
match fat32_verify_exists(&mut spi, &mut cs, &fat_info, file_to_delete, high_capacity) {
    Ok(true) => {
        // File exists, delete it
        match fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, file_to_delete, high_capacity) {
            Ok(()) => {
                info!("✓ Deleted {}", file_to_delete);
                
                // Verify deletion
                match fat32_verify_exists(&mut spi, &mut cs, &fat_info, file_to_delete, high_capacity) {
                    Ok(false) => info!("✓ Deletion verified"),
                    Ok(true) => error!("✗ File still exists!"),
                    Err(e) => error!("✗ Verification failed: {}", e),
                }
            }
            Err(e) => error!("✗ Failed to delete: {}", e),
        }
    }
    Ok(false) => info!("File doesn't exist, nothing to delete"),
    Err(e) => error!("✗ Error checking file: {}", e),
}
```

---

## 🚨 **Error Handling**

### **Common Errors:**

| Error | Cause | Solution |
|-------|-------|----------|
| `"File not found"` | File doesn't exist | Check path and filename |
| `"Directory not empty"` | Trying to delete non-empty dir | Delete all files first |
| `"Cannot delete directory with delete_file"` | Used wrong function | Use `delete_directory` instead |
| `"Not a directory"` | Trying to delete file as directory | Use `delete_file` instead |

---

## 💡 **Best Practices**

### ✅ **DO:**
- Check if file exists before deleting
- Verify deletion was successful
- Delete all files before deleting a directory
- Handle errors gracefully

### ❌ **DON'T:**
- Delete files you need!
- Try to delete non-empty directories
- Delete root directory
- Delete while file is being read

---

## 🎨 **Add to Your Custom Section**

Add this to the "YOUR CUSTOM FILES" section in `main.rs`:

```rust
// Delete example files
info!("\n=== Deleting Files ===");

// Delete a temporary file
match fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", high_capacity) {
    Ok(()) => info!("✓ Deleted /MYFILE.TXT"),
    Err(e) => error!("✗ Failed: {}", e),
}

// Verify it's gone
match fat32_verify_exists(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", high_capacity) {
    Ok(false) => info!("✓ File successfully deleted"),
    Ok(true) => error!("✗ File still exists!"),
    Err(e) => error!("✗ Error: {}", e),
}
```

---

## 🔧 **Advanced: Batch Deletion**

```rust
// Delete multiple files with pattern matching
let files_to_delete = [
    "/TEMP1.TXT",
    "/TEMP2.TXT",
    "/CACHE.DAT",
    "/LOGS/OLD.LOG",
];

let mut deleted_count = 0;
for file_path in &files_to_delete {
    if fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, file_path, high_capacity).is_ok() {
        deleted_count += 1;
    }
}

info!("Deleted {}/{} files", deleted_count, files_to_delete.len());
```

---

## 🧪 **Testing Your Deletions**

### **Method 1: Check Serial Output**

Look for these log messages:
```
INFO  Deleting file at path: /MYFILE.TXT
INFO    Freed 1 clusters
INFO  File deleted successfully
INFO  ✓ Verified /MYFILE.TXT is deleted
```

### **Method 2: Check on Computer**

1. Flash your code
2. Remove SD card
3. Check in File Explorer/Finder
4. Deleted files should be gone!

### **Method 3: Use Verification**

```rust
// Before deletion
fat32_verify_exists(...) // Returns Ok(true)

// Delete
fat32_delete_file_at_path(...)

// After deletion
fat32_verify_exists(...) // Returns Ok(false)
```

---

## 📊 **How It Works Internally**

### **FAT Chain Deletion:**

When you delete a file with 3 clusters:

**Before Deletion:**
```
Cluster 5 → Cluster 8 → Cluster 12 → EOC
FAT[5] = 8
FAT[8] = 12
FAT[12] = 0xFFFFFFFF
```

**After Deletion:**
```
FAT[5] = 0x00000000  (free)
FAT[8] = 0x00000000  (free)
FAT[12] = 0x00000000 (free)
```

### **Directory Entry:**

**Before:** `MYFILE  TXT ...data...`
**After:** `�YFILE  TXT ...data...` (first byte = 0xE5)

---

## 🎉 **What's New**

Your filesystem now has:
- ✅ File deletion with cluster freeing
- ✅ Directory deletion (empty only)
- ✅ Path-based deletion
- ✅ Proper error handling
- ✅ FAT chain cleanup

---

## 🚀 **Next Steps**

Want to add more features?
- **Recursive directory deletion** (delete non-empty directories)
- **File renaming**
- **File moving between directories**
- **Trash/recycle bin** (mark as deleted but don't free clusters)

Check `NEXT_STEPS_ROADMAP.md` for more ideas!

---

Happy deleting! 🗑️✨
