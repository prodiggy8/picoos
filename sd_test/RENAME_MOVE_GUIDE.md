# 📝 File Rename & Move Guide

## 🎯 Overview

Your FAT32 filesystem now supports **renaming** and **moving** files! This guide shows you how to use these powerful features.

---

## 🔄 File Renaming

### Basic Renaming

Rename a file in the same directory:

```rust
fat32_rename_file_at_path(
    &mut spi,
    &mut cs,
    &fat_info,
    "/README.TXT",      // Old path
    "WELCOME.TXT",      // New name (8.3 format)
    high_capacity
)?;
```

**Result:** `/README.TXT` → `/WELCOME.TXT`

### Renaming in Subdirectories

```rust
fat32_rename_file_at_path(
    &mut spi,
    &mut cs,
    &fat_info,
    "/DOCS/GUIDE.TXT",   // Old path
    "MANUAL.TXT",        // New name
    high_capacity
)?;
```

**Result:** `/DOCS/GUIDE.TXT` → `/DOCS/MANUAL.TXT`

---

## 🚚 File Moving

### Move to Different Directory

Move a file from one directory to another:

```rust
fat32_move_file(
    &mut spi,
    &mut cs,
    &fat_info,
    "/README.TXT",       // Source path
    "/MUSIC/INFO.TXT",   // Destination path
    high_capacity
)?;
```

**Result:** 
- ❌ Deleted: `/README.TXT`
- ✅ Created: `/MUSIC/INFO.TXT`

### Move AND Rename

You can move and rename in one operation:

```rust
fat32_move_file(
    &mut spi,
    &mut cs,
    &fat_info,
    "/DOCS/REPORT.TXT",  // Source
    "/MUSIC/NOTES.TXT",  // Destination (different name!)
    high_capacity
)?;
```

**Result:**
- ❌ Deleted: `/DOCS/REPORT.TXT`
- ✅ Created: `/MUSIC/NOTES.TXT`

---

## 📋 Key Functions

### 1. `fat32_rename_file_at_path`

Rename a file in its current directory.

**Parameters:**
- `old_path` - Current file path (e.g., `/README.TXT`)
- `new_name` - New filename in 8.3 format (e.g., `WELCOME.TXT`)

**What it does:**
- ✅ Updates the directory entry with new name
- ✅ Preserves file content, size, and cluster chain
- ❌ Fails if new name already exists

### 2. `fat32_move_file`

Move a file to a different directory (optionally renaming it).

**Parameters:**
- `src_path` - Source file path (e.g., `/README.TXT`)
- `dest_path` - Destination path with new name (e.g., `/MUSIC/INFO.TXT`)

**What it does:**
- ✅ Creates new directory entry in destination
- ✅ Removes old directory entry from source
- ✅ Preserves file data (no copying needed!)
- ❌ Fails if destination file already exists

---

## ⚠️ Important Notes

### 8.3 Filename Format

Both operations require **8.3 format** filenames:
- **Maximum 8 characters** for filename
- **Maximum 3 characters** for extension
- **Uppercase automatically**

✅ Valid: `README.TXT`, `MANUAL.DOC`, `DATA.BIN`  
❌ Invalid: `VERYLONGFILENAME.TXT`, `FILE.HTML`

### Error Handling

Common errors:
- `"File with new name already exists"` - Destination name is taken
- `"Source file not found"` - Old file doesn't exist
- `"Destination file already exists"` - Can't overwrite
- `"Filename too long"` - Exceeds 8.3 format limits

### What Gets Preserved?

When renaming or moving:
- ✅ **File content** (no data is copied or changed)
- ✅ **File size**
- ✅ **Cluster chain** (same physical location on SD card)
- ✅ **File attributes** (archive, directory, etc.)

### Performance

Both operations are **very fast** because:
- No file data is copied
- Only directory entries are updated
- Cluster chain remains unchanged

---

## 💡 Usage Examples

### Example 1: Organize Your Files

```rust
// Move all text files to DOCS directory
fat32_move_file(&mut spi, &mut cs, &fat_info, 
    "/FILE1.TXT", "/DOCS/FILE1.TXT", high_capacity)?;
fat32_move_file(&mut spi, &mut cs, &fat_info, 
    "/FILE2.TXT", "/DOCS/FILE2.TXT", high_capacity)?;
```

### Example 2: Rename with Versioning

```rust
// Rename old version
fat32_rename_file_at_path(&mut spi, &mut cs, &fat_info,
    "/CONFIG.TXT", "CONFIG.BAK", high_capacity)?;

// Write new version
fat32_write_file_at_path(&mut spi, &mut cs, &fat_info,
    "/CONFIG.TXT", new_config_data, high_capacity)?;
```

### Example 3: Archive Old Data

```rust
// Create archive directory
fat32_create_directory(&mut spi, &mut cs, &fat_info,
    fat_info.root_dir_cluster, "ARCHIVE", high_capacity)?;

// Move old files to archive
fat32_move_file(&mut spi, &mut cs, &fat_info,
    "/OLD.DAT", "/ARCHIVE/OLD.DAT", high_capacity)?;
```

---

## 🧪 Test Results

Running the built-in tests will:

1. **TEST 7: File Renaming**
   - Rename `/README.TXT` → `/WELCOME.TXT`
   - Rename `/DOCS/GUIDE.TXT` → `/DOCS/MANUAL.TXT`
   - Verify old names are gone, new names exist

2. **TEST 8: File Moving**
   - Move `/WELCOME.TXT` → `/MUSIC/INFO.TXT`
   - Move `/DOCS/REPORT.TXT` → `/MUSIC/NOTES.TXT`
   - Verify files removed from source, exist in destination

### Expected SD Card Contents

After running all tests:

```
/
├── DOCS/
│   ├── MANUAL.TXT     (renamed from GUIDE.TXT)
│   └── REPORTS/       (empty subdirectory)
└── MUSIC/
    ├── INFO.TXT       (moved from /WELCOME.TXT)
    └── NOTES.TXT      (moved from /DOCS/REPORT.TXT)
```

---

## 🚀 Next Steps

Now that you have rename and move, you could add:

1. **Copy files** (duplicate file data to new location)
2. **Batch operations** (rename/move multiple files)
3. **Undo/redo** (keep track of file operations)
4. **Directory moving** (move entire directories)
5. **File attributes** (mark files as read-only, hidden, etc.)

---

## 📚 Related Documentation

- **QUICK_ADD_FILES.md** - How to create files
- **FILE_DELETION_GUIDE.md** - How to delete files/directories
- **VERIFICATION_GUIDE.md** - How to verify filesystem operations

Happy file management! 🎉
