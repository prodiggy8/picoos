# FAT32 Filesystem Write Support

## Overview
Your Pico OS now has complete FAT32 write support! You can create and write files to an SD card formatted with FAT32.

## What Was Added

### 1. **SD Card Write Function** (`sd_write_block`)
Low-level function to write 512-byte sectors to the SD card using CMD24.

### 2. **FAT Table Operations**
- `read_fat_entry()` - Read a FAT entry to follow cluster chains
- `write_fat_entry()` - Update FAT to link clusters or mark end-of-chain
- `find_free_cluster()` - Scan FAT to find available clusters

### 3. **Directory Entry Structure** (`DirEntry`)
- Parse existing directory entries from raw bytes
- Create new directory entries with 8.3 filenames
- Encode entries back to 32-byte format for writing

### 4. **High-Level File Writing** (`fat32_write_file`)
Complete file writing that:
- Creates a directory entry
- Allocates clusters as needed
- Updates the FAT table
- Writes file data across multiple clusters if necessary
- Adds the entry to the root directory

## Usage Example

```rust
// Assuming you have initialized the SD card and parsed FAT32 info
let data = b"Hello from Pico OS!";

match fat32_write_file(&mut spi, &mut cs, &fat_info, "TEST.TXT", data, high_capacity) {
    Ok(()) => info!("File written successfully!"),
    Err(e) => error!("Write failed: {}", e),
}
```

## Filename Format
Files must use 8.3 format (max 8 chars for name, 3 for extension):
-  "HELLO.TXT"
-  "DATA.BIN"
-  "README" (no extension)
-  "VERYLONGNAME.TXT" (name too long)
-  "FILE.JSON" (extension too long)

## Current Limitations

1. **Root Directory Only**: Currently writes to root directory only. No subdirectory support yet.

2. **First Sector Only**: Only scans first sector of root directory for free entries (16 slots). Full implementation would scan multiple sectors.

3. **No Timestamps**: Directory entries don't include creation/modification timestamps yet.

4. **No File Updates**: Can only create new files. Updating existing files not implemented.

5. **No File Deletion**: Can't delete files yet (would need to mark entry as 0xE5 and free clusters).

## Next Steps for Full Filesystem

### Recommended additions:

1. **Update Existing Files**
   - Find existing entry by name
   - Truncate or extend cluster chain
   - Update file size in directory entry

2. **File Deletion**
   ```rust
   fn fat32_delete_file(filename: &str) -> Result<(), Error>
   ```

3. **Subdirectory Support**
   - Navigate directory tree
   - Create new directories
   - Write to files in subdirectories

4. **Better Directory Scanning**
   - Handle multi-sector/cluster directories
   - Support long filenames (LFN)

5. **File Abstraction Layer**
   ```rust
   struct File {
       entry: DirEntry,
       position: u32,
       dirty: bool,
   }
   
   impl File {
       fn open(path: &str) -> Result<Self, Error>;
       fn create(path: &str) -> Result<Self, Error>;
       fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error>;
       fn write(&mut self, buf: &[u8]) -> Result<usize, Error>;
       fn seek(&mut self, pos: SeekFrom) -> Result<u32, Error>;
       fn flush(&mut self) -> Result<(), Error>;
   }
   ```

6. **Caching Layer**
   - Cache FAT sectors
   - Cache directory sectors
   - Batch writes for better performance

7. **Error Handling**
   Replace `&'static str` with proper error types:
   ```rust
   enum FsError {
       IoError,
       NotFound,
       NoSpace,
       InvalidFilename,
       AlreadyExists,
       DirectoryFull,
   }
   ```

8. **Wear Leveling Considerations**
   - Track frequently written sectors
   - Implement wear-aware allocation

## Testing Your Write Implementation

1. **Format SD Card**: Make sure SD card is FAT32 formatted
2. **Flash the Code**: Upload to your Pico
3. **Check the Output**: Look for "Successfully wrote HELLO.TXT!" in your logs
4. **Verify on PC**: 
   - Remove SD card from Pico
   - Insert into PC
   - You should see HELLO.TXT in the root directory!

## Safety Notes

**IMPORTANT**: 
- Always properly unmount/eject SD card before removing from Pico
- Writing to FAT can corrupt the filesystem if interrupted
- Test on a backup SD card first
- The code doesn't implement fsinfo sector updates (shows wrong free space on PC)

## Performance Tips

- Writing is slow at 400kHz. After init, you can increase SPI speed:
  ```rust
  // After sd_init succeeds
  let mut spi = spi.reconfigure(|config| {
      config.set_baudrate(10.MHz())
  });
  ```

- Batch writes when possible to reduce FAT table updates
- Align file sizes to cluster boundaries when possible

## Integration with Pico OS

For a complete OS, you might want to:
1. Move filesystem code to a separate module/crate
2. Create a VFS (Virtual File System) layer
3. Add file descriptors and process-level file tables
4. Implement buffered I/O
5. Add synchronization for multi-process access

Happy coding! 🦀
