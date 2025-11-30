// ============================================================================
// EMBEDDED SYSTEM CONFIGURATION
// ============================================================================
// Tell Rust we're not using the standard library (no OS, no heap by default)
#![no_std]
// We're not using the standard main function - embedded entry point instead
#![no_main]

// ---- Boot2 for RP2040 (Stage 2 Bootloader) ----
// This is the second-stage bootloader that runs when the RP2040 boots up.
// It configures external flash memory before our main program runs.
#[link_section = ".boot2"]  // Place this in the .boot2 section of binary
#[no_mangle]                 // Don't change the symbol name during compilation
#[used]                      // Force the compiler to include this even if it seems unused
pub static BOOT2_FIRMWARE: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

// ---- Logging and Panic Handling ----
use defmt::*;           // Efficient logging for embedded systems (smaller than println!)
use defmt_rtt as _;     // Real-Time Transfer: sends logs over debug probe
use panic_probe as _;   // What to do when program panics (crashes)

// ---- Board Hardware Abstraction Layer (HAL) ----
use rp_pico::entry;     // The #[entry] macro - marks our main() function
use rp_pico::hal::{self as hal, pac, watchdog::Watchdog};
use hal::sio::Sio;      // Single-cycle I/O for GPIO
use hal::fugit::RateExtU32;  // Allows writing "400.kHz()" instead of raw numbers
use hal::clocks::Clock; // Clock management

// ---- embedded-hal Traits for Hardware Independence ----
// These traits let us write code that works with any SPI/GPIO implementation
use embedded_hal::digital::OutputPin;  // GPIO output operations (set high/low)
use embedded_hal::spi::{SpiBus, MODE_0};  // SPI communication and clock mode

// ============================================================================
// FAT32 FILESYSTEM DATA STRUCTURES
// ============================================================================
// FAT32 is the File Allocation Table filesystem used on SD cards.
// It organizes data into:
// - Boot Sector: Contains filesystem metadata
// - FAT (File Allocation Table): Maps which clusters belong to which files
// - Data Region: Actual file contents stored in "clusters"

/// Represents a single directory entry (file or folder) in FAT32 format.
/// Each entry is exactly 32 bytes and contains metadata about a file/directory.
#[derive(Debug, Clone, Copy)]
struct DirEntry {
    name: [u8; 11],        // 8.3 format: "HELLO   TXT" for "HELLO.TXT" (no dot stored)
    attr: u8,              // File attributes: 0x10=directory, 0x20=archive (regular file)
    size: u32,             // File size in bytes (0 for directories)
    start_cluster: u32,    // First cluster number where file data begins (clusters are like blocks)
}

impl DirEntry {
    /// Parse a 32-byte directory entry from raw bytes read from SD card.
    /// Returns None if the entry is invalid, deleted, or special (long filename).
    fn parse(buf: &[u8]) -> Option<Self> {
        // Safety check: need at least 32 bytes for a directory entry
        if buf.len() < 32 {
            return None;
        }

        let first_byte = buf[0];
        
        // Skip empty/deleted entries
        // 0x00 = end of directory (no more entries after this)
        // 0xE5 = deleted file (slot can be reused)
        if first_byte == 0x00 || first_byte == 0xE5 {
            return None;
        }

        let attr = buf[11];  // Attribute byte at offset 11
        
        // Skip long filename entries and volume labels
        // 0x0F = long filename entry (used for names >8.3 format)
        // 0x08 bit = volume label (the disk name, not a file)
        if attr == 0x0F || (attr & 0x08) != 0 {
            return None;
        }

        // Copy the 8.3 filename (11 bytes starting at offset 0)
        let mut name = [0u8; 11];
        name.copy_from_slice(&buf[0..11]);  // copy_from_slice is from Rust's core library

        // Parse file size (4 bytes, little-endian, at offset 28-31)
        let size = u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]);
        
        // Parse starting cluster number (split into high and low words)
        // FAT32 uses 32-bit cluster numbers, stored in two 16-bit fields
        let cluster_hi = u16::from_le_bytes([buf[20], buf[21]]) as u32;  // High 16 bits at offset 20
        let cluster_lo = u16::from_le_bytes([buf[26], buf[27]]) as u32;  // Low 16 bits at offset 26
        let start_cluster = (cluster_hi << 16) | cluster_lo;  // Combine into 32-bit number

        Some(DirEntry {
            name,
            attr,
            size,
            start_cluster,
        })
    }

    /// Encode this directory entry into a 32-byte buffer for writing to SD card.
    /// This is the reverse of parse() - converts our struct to raw bytes.
    fn encode(&self, buf: &mut [u8]) {
        // Safety check: need exactly 32 bytes
        if buf.len() < 32 {
            return;
        }

        // Clear the buffer to zeros first (important for unused fields)
        buf[0..32].fill(0);

        // Write name in 8.3 format (11 bytes at offset 0)
        buf[0..11].copy_from_slice(&self.name);

        // Write attributes at offset 11
        buf[11] = self.attr;

        // Offsets 12-19: Reserved for timestamps (creation time, modified time, etc.)
        // We skip these for simplicity, but a full implementation would set them

        // Write first cluster high word (16 bits at offset 20-21)
        let cluster_hi = ((self.start_cluster >> 16) & 0xFFFF) as u16;
        buf[20..22].copy_from_slice(&cluster_hi.to_le_bytes());

        // Offsets 22-25: Write time and date (skipped for simplicity)
        
        // Write first cluster low word (16 bits at offset 26-27)
        let cluster_lo = (self.start_cluster & 0xFFFF) as u16;
        buf[26..28].copy_from_slice(&cluster_lo.to_le_bytes());

        // Write file size (4 bytes at offset 28-31)
        buf[28..32].copy_from_slice(&self.size.to_le_bytes());
    }

    /// Create a new directory entry from a filename string.
    /// Converts modern filenames like "hello.txt" into FAT32's 8.3 format: "HELLO   TXT"
    fn new(name: &str, attr: u8) -> Result<Self, &'static str> {
        let mut name_bytes = [b' '; 11];  // Start with 11 spaces (padding)
        
        // Parse filename.ext into 8.3 format (without heap allocations - embedded requirement!)
        if let Some(dot_pos) = name.find('.') {
            // Has extension (e.g., "hello.txt")
            let basename = &name[..dot_pos];      // "hello"
            let ext = &name[dot_pos + 1..];       // "txt"
            
            // Validate lengths: max 8 chars for name, 3 for extension
            if basename.is_empty() || basename.len() > 8 {
                return Err("Filename too long (max 8 chars)");
            }
            if ext.len() > 3 {
                return Err("Extension too long (max 3 chars)");
            }
            
            // Copy basename and convert to uppercase (FAT32 is case-insensitive)
            // First 8 bytes are filename, padded with spaces
            for (i, c) in basename.bytes().enumerate() {
                name_bytes[i] = c.to_ascii_uppercase();
            }
            
            // Copy extension (bytes 8-10)
            for (i, c) in ext.bytes().enumerate() {
                name_bytes[8 + i] = c.to_ascii_uppercase();
            }
        } else {
            // No extension (e.g., "README")
            if name.is_empty() || name.len() > 8 {
                return Err("Filename too long (max 8 chars)");
            }
            
            // Copy name and convert to uppercase
            for (i, c) in name.bytes().enumerate() {
                name_bytes[i] = c.to_ascii_uppercase();
            }
            // Extension part stays as spaces (bytes 8-10)
        }

        Ok(DirEntry {
            name: name_bytes,
            attr,
            size: 0,              // New files start at 0 bytes
            start_cluster: 0,     // Will be set when we allocate clusters
        })
    }
}

/// Filesystem metadata parsed from the boot sector of an SD card.
/// This tells us how the FAT32 filesystem is organized on the card.
// #[derive(Debug, Clone, Copy)]  // Commented out to save space
struct Fat32Info {
    bytes_per_sector: u16,      // Usually 512 bytes (one SD card block)
    sectors_per_cluster: u8,    // How many sectors form one cluster (cluster = allocation unit)
    reserved_sectors: u16,      // Sectors before first FAT (usually contains boot sector)
    num_fats: u8,               // Number of FAT copies (usually 2 for redundancy)
    fat_size_sectors: u32,      // Size of one FAT in sectors
    root_dir_cluster: u32,      // Starting cluster of root directory (/)
    total_sectors: u32,         // Total sectors on the volume
    // Calculated fields (derived from above):
    fat_start_lba: u32,         // LBA (Logical Block Address) where first FAT starts
    data_start_lba: u32,        // LBA where actual file data starts
}

impl Fat32Info {
    /// Parse FAT32 filesystem information from the boot sector (sector 0).
    /// The boot sector contains the BIOS Parameter Block (BPB) with all metadata.
    fn parse(boot_sector: &[u8; 512]) -> Result<Self, &'static str> {
        // Check boot signature (bytes 510-511 must be 0x55 0xAA)
        // This is a magic number that identifies a valid boot sector
        if boot_sector[510] != 0x55 || boot_sector[511] != 0xAA {
            return Err("Invalid boot signature");
        }

        // Parse BIOS Parameter Block (BPB) - standard offsets defined by FAT32 spec
        let bytes_per_sector = u16::from_le_bytes([boot_sector[11], boot_sector[12]]);
        let sectors_per_cluster = boot_sector[13];
        let reserved_sectors = u16::from_le_bytes([boot_sector[14], boot_sector[15]]);
        let num_fats = boot_sector[16];  // Usually 2 (primary and backup)
        
        // For FAT32, total sectors is at offset 32 (4 bytes, little-endian)
        // FAT12/FAT16 use a different offset - this is FAT32 specific
        let total_sectors = u32::from_le_bytes([
            boot_sector[32],
            boot_sector[33],
            boot_sector[34],
            boot_sector[35],
        ]);

        // FAT32-specific: FAT size at offset 36 (4 bytes)
        // This is how many sectors each FAT occupies
        let fat_size_sectors = u32::from_le_bytes([
            boot_sector[36],
            boot_sector[37],
            boot_sector[38],
            boot_sector[39],
        ]);

        // Root directory cluster at offset 44 (4 bytes)
        // In FAT32, root directory is a regular cluster chain (unlike FAT16)
        let root_dir_cluster = u32::from_le_bytes([
            boot_sector[44],
            boot_sector[45],
            boot_sector[46],
            boot_sector[47],
        ]);

        // Calculate partition layout:
        // [Reserved Sectors | FAT #1 | FAT #2 | ... | Data Region]
        let fat_start_lba = reserved_sectors as u32;
        let data_start_lba = fat_start_lba + (num_fats as u32 * fat_size_sectors);

        Ok(Fat32Info {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            fat_size_sectors,
            root_dir_cluster,
            total_sectors,
            fat_start_lba,
            data_start_lba,
        })
    }

    /// Convert a cluster number to its corresponding LBA (sector address).
    /// Clusters are the filesystem's view, LBA is the disk's view.
    fn cluster_to_lba(&self, cluster: u32) -> u32 {
        // Clusters start at 2 (0 and 1 are reserved in FAT spec)
        // So cluster 2 is the first actual data cluster
        self.data_start_lba + ((cluster - 2) * self.sectors_per_cluster as u32)
    }

    /// Read a FAT entry to get the next cluster in the chain.
    /// Files can span multiple clusters; FAT is a linked list telling us which cluster comes next.
    /// Returns: next cluster number, or >= 0x0FFFFFF8 for end-of-chain, or 0 for free cluster
    fn read_fat_entry<SPI, CS>(
        &self,
        spi: &mut SPI,
        cs: &mut CS,
        cluster: u32,
        high_capacity: bool,
    ) -> Result<u32, &'static str>
    where
        SPI: SpiBus<u8>,
        CS: OutputPin,
    {
        // Each FAT entry is 4 bytes in FAT32 (unlike FAT16 which uses 2 bytes)
        let fat_offset = cluster * 4;  // Byte offset within FAT
        
        // Calculate which sector of the FAT contains this entry
        let fat_sector = self.fat_start_lba + (fat_offset / self.bytes_per_sector as u32);
        
        // Offset within that sector
        let entry_offset = (fat_offset % self.bytes_per_sector as u32) as usize;

        // Read the sector containing the FAT entry
        let mut buf = [0u8; 512];
        sd_read_block(spi, cs, fat_sector, &mut buf, high_capacity)?;

        // Extract the 4-byte FAT entry and mask off top 4 bits (reserved, not used)
        let entry = u32::from_le_bytes([
            buf[entry_offset],
            buf[entry_offset + 1],
            buf[entry_offset + 2],
            buf[entry_offset + 3],
        ]) & 0x0FFF_FFFF;  // Only lower 28 bits are used in FAT32

        Ok(entry)
    }

    /// Write a FAT entry to link clusters or mark end-of-chain.
    /// Updates ALL FAT copies (usually 2) to keep them synchronized.
    fn write_fat_entry<SPI, CS>(
        &self,
        spi: &mut SPI,
        cs: &mut CS,
        cluster: u32,
        value: u32,  // Next cluster number, 0 for free, 0x0FFFFFFF for end-of-chain
        high_capacity: bool,
    ) -> Result<(), &'static str>
    where
        SPI: SpiBus<u8>,
        CS: OutputPin,
    {
        // Each FAT entry is 4 bytes in FAT32
        let fat_offset = cluster * 4;
        let fat_sector = self.fat_start_lba + (fat_offset / self.bytes_per_sector as u32);
        let entry_offset = (fat_offset % self.bytes_per_sector as u32) as usize;

        // Read the sector first (we need to preserve other FAT entries in same sector)
        let mut buf = [0u8; 512];
        sd_read_block(spi, cs, fat_sector, &mut buf, high_capacity)?;

        // Preserve top 4 bits (reserved), write new value in lower 28 bits
        let old_value = u32::from_le_bytes([
            buf[entry_offset],
            buf[entry_offset + 1],
            buf[entry_offset + 2],
            buf[entry_offset + 3],
        ]);
        let new_value = (old_value & 0xF000_0000) | (value & 0x0FFF_FFFF);
        let bytes = new_value.to_le_bytes();
        
        // Update the buffer with new FAT entry
        buf[entry_offset] = bytes[0];
        buf[entry_offset + 1] = bytes[1];
        buf[entry_offset + 2] = bytes[2];
        buf[entry_offset + 3] = bytes[3];

        // Write to ALL FAT copies (usually 2) for redundancy
        for fat_num in 0..self.num_fats {
            let sector = self.fat_start_lba + (fat_num as u32 * self.fat_size_sectors) 
                + (fat_offset / self.bytes_per_sector as u32);
            sd_write_block(spi, cs, sector, &buf, high_capacity)?;
        }

        Ok(())
    }

    /// Find a free cluster by scanning the FAT for a zero entry.
    /// This is like malloc() for the filesystem - finds free space to allocate.
    fn find_free_cluster<SPI, CS>(
        &self,
        spi: &mut SPI,
        cs: &mut CS,
        start_hint: u32,  // Where to start searching (optimization: search near last allocation)
        high_capacity: bool,
    ) -> Result<u32, &'static str>
    where
        SPI: SpiBus<u8>,
        CS: OutputPin,
    {
        // Calculate maximum valid cluster number
        let max_cluster = self.total_sectors / self.sectors_per_cluster as u32;
        
        // Start searching from hint (typically last allocated cluster)
        for cluster in start_hint..max_cluster {
            if cluster < 2 {
                continue; // Clusters 0 and 1 are reserved in FAT specification
            }
            
            let entry = self.read_fat_entry(spi, cs, cluster, high_capacity)?;
            if entry == 0 {  // 0 means free cluster
                return Ok(cluster);
            }
        }

        // Wrap around and search from beginning if we didn't find one
        for cluster in 2..start_hint {
            let entry = self.read_fat_entry(spi, cs, cluster, high_capacity)?;
            if entry == 0 {
                return Ok(cluster);
            }
        }

        Err("No free clusters")  // Disk is full!
    }
}

// ============================================================================
// SD CARD LOW-LEVEL COMMUNICATION FUNCTIONS
// ============================================================================
// These functions implement the SD card protocol over SPI.
// SD cards use a command-response protocol with specific timing requirements.

/// Send and receive one byte over SPI (full-duplex communication).
/// SPI is synchronous: we send a byte and simultaneously receive a byte.
fn spi_txrx<SPI>(spi: &mut SPI, byte: u8) -> u8
where
    SPI: SpiBus<u8>,
{
    let mut buf = [byte];
    // transfer_in_place sends buf[0] and replaces it with received byte
    if spi.transfer_in_place(&mut buf).is_ok() {
        buf[0]  // Return received byte
    } else {
        0xFF    // Return 0xFF on error (idle state for SD cards)
    }
}

/// End an SD command by deselecting the card (CS high) with proper timing.
/// SD cards need clock cycles with CS high between commands.
fn sd_end_cmd<SPI, CS>(spi: &mut SPI, cs: &mut CS)
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    let _ = spi_txrx(spi, 0xFF);  // Send one dummy byte for timing
    let _ = cs.set_high();         // Deselect card (CS = high)
    let _ = spi_txrx(spi, 0xFF);  // Send another byte (gives card time to finish)
}

/// Send a command to the SD card and wait for response.
/// SD commands are 6 bytes: [cmd | arg3 | arg2 | arg1 | arg0 | crc]
/// Returns the R1 response byte (status byte from card).
fn sd_send_cmd<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    cmd: u8,      // Command number (0-63)
    arg: u32,     // 32-bit argument (meaning depends on command)
    crc: u8,      // CRC-7 checksum (only needed for CMD0 and CMD8)
) -> Result<u8, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    let _ = cs.set_low();           // Select card (CS = low)
    let _ = spi_txrx(spi, 0xFF);    // Dummy byte for timing

    // Send 6-byte command packet
    let _ = spi_txrx(spi, 0x40 | cmd);        // Command byte (always starts with 01xxxxxx)
    let _ = spi_txrx(spi, (arg >> 24) as u8); // Argument byte 3 (MSB)
    let _ = spi_txrx(spi, (arg >> 16) as u8); // Argument byte 2
    let _ = spi_txrx(spi, (arg >> 8) as u8);  // Argument byte 1
    let _ = spi_txrx(spi, arg as u8);         // Argument byte 0 (LSB)
    let _ = spi_txrx(spi, crc);               // CRC byte

    // Wait for response (card sends 0xFF while processing, then response byte)
    // Response byte has MSB=0 (distinguishes from 0xFF idle bytes)
    for _ in 0..255 {
        let resp = spi_txrx(spi, 0xFF);
        if resp & 0x80 == 0 {  // MSB clear = valid response
            return Ok(resp);
        }
    }

    sd_end_cmd(spi, cs);
    Err("CMD timeout")  // Card didn't respond in time
}

/// Initialize the SD card: reset it, check version, and activate.
/// Returns: Ok(true) for high-capacity (SDHC/SDXC), Ok(false) for standard capacity.
fn sd_init<SPI, CS>(spi: &mut SPI, cs: &mut CS) -> Result<bool, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Step 1: Send ≥80 clock pulses with CS high to let card power up
    info!("SD init: send ≥80 clocks with CS high");
    let _ = cs.set_high();
    for _ in 0..20 {  // 20 bytes × 8 bits = 160 clock pulses
        let _ = spi_txrx(spi, 0xFF);
    }

    cortex_m::asm::delay(10_000);  // Wait for card to stabilize

    // Step 2: CMD0 - Reset card to idle state
    info!("SD init: CMD0");
    let mut r1 = 0xFF;
    for attempt in 0..10 {
        r1 = sd_send_cmd(spi, cs, 0, 0, 0x95)?;  // CRC=0x95 is required for CMD0
        sd_end_cmd(spi, cs);
        info!("  CMD0 attempt {=u8}: r1 = {=u8:#04x}", attempt, r1);
        if r1 == 0x01 {  // 0x01 = "in idle state" (correct response)
            break;
        }
        // Add dummy bytes and delay between attempts
        for _ in 0..100 {
            let _ = spi_txrx(spi, 0xFF);
        }
        cortex_m::asm::delay(100_000);
    }

    // Check if card responded
    if r1 == 0xFF {
        error!("CMD0 no response - check SD card connection!");
        return Err("No SD card detected");
    } else if r1 != 0x01 {
        error!("CMD0 unexpected r1 = {=u8:#04x}, expected 0x01", r1);
        return Err("CMD0 did not enter IDLE");
    }

    // Step 3: CMD8 - Check card version and voltage range
    info!("SD init: CMD8");
    let r1 = sd_send_cmd(spi, cs, 8, 0x0000_01AA, 0x87)?;  // 0x1AA = test pattern, CRC=0x87
    let v2;  // Will be true for v2.0+ cards (SDHC/SDXC), false for v1.x

    if r1 == 0x01 {
        // Card supports CMD8 - it's a v2.0+ card
        let mut r7 = [0u8; 4];  // CMD8 returns R7 response (4 additional bytes)
        for b in r7.iter_mut() {
            *b = spi_txrx(spi, 0xFF);
        }
        sd_end_cmd(spi, cs);

        info!("  CMD8 R7: {=u8} {=u8} {=u8} {=u8}", r7[0], r7[1], r7[2], r7[3]);
        // Check if card echoed back our test pattern (0x01AA)
        if r7[2] == 0x01 && r7[3] == 0xAA {
            v2 = true;  // Valid v2.0+ card
        } else {
            return Err("CMD8 bad echo pattern");
        }
    } else if (r1 & 0x04) != 0 {
        // Card doesn't support CMD8 (illegal command bit set) - it's a v1.x card
        info!("CMD8 illegal -> old card (v1.x/MMC)");
        sd_end_cmd(spi, cs);
        v2 = false;
    } else {
        sd_end_cmd(spi, cs);
        return Err("CMD8 unexpected R1");
    }

    // Step 4: ACMD41 - Activate card and wait for initialization to complete
    // ACMD41 = CMD55 followed by CMD41 (all ACMDs require CMD55 first)
    info!("SD init: ACMD41 loop");
    for _ in 0..1000 {
        // Send CMD55 (next command is an application-specific command)
        let r1 = sd_send_cmd(spi, cs, 55, 0, 0x01)?;
        sd_end_cmd(spi, cs);
        if r1 > 0x01 {
            return Err("CMD55 failed");
        }

        // Send CMD41 with HCS bit (tells card we support high capacity)
        let arg = if v2 { 1u32 << 30 } else { 0 };  // Bit 30 = HCS (Host Capacity Support)
        let r1 = sd_send_cmd(spi, cs, 41, arg, 0x01)?;
        sd_end_cmd(spi, cs);

        if r1 == 0x00 {  // 0x00 = card is ready (initialization complete)
            info!("  ACMD41: card ready");
            break;
        }

        let _ = spi_txrx(spi, 0xFF);  // Timing byte
    }

    // Step 5: CMD58 - Read OCR (Operating Conditions Register) to check card type
    let mut high_capacity = false;
    if v2 {
        info!("SD init: CMD58");
        let r1 = sd_send_cmd(spi, cs, 58, 0, 0x01)?;
        if r1 != 0x00 {
            sd_end_cmd(spi, cs);
            return Err("CMD58 failed");
        }

        let mut ocr = [0u8; 4];  // Read 32-bit OCR register
        for b in ocr.iter_mut() {
            *b = spi_txrx(spi, 0xFF);
        }
        sd_end_cmd(spi, cs);

        info!("  OCR: {=u8} {=u8} {=u8} {=u8}", ocr[0], ocr[1], ocr[2], ocr[3]);
        // Bit 30 of OCR = CCS (Card Capacity Status)
        // CCS=1 means SDHC/SDXC (uses block addressing)
        // CCS=0 means SDSC (uses byte addressing)
        high_capacity = (ocr[0] & 0x40) != 0;
    }

    info!("SD init complete! high_capacity = {=bool}", high_capacity);
    Ok(high_capacity)
}

/// Read a 512-byte block from the SD card.
/// This is the fundamental read operation - all file reads use this.
fn sd_read_block<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    lba: u32,              // Logical Block Address (sector number)
    buf: &mut [u8; 512],   // Buffer to store the 512 bytes read
    high_capacity: bool,   // true for SDHC/SDXC, false for SDSC
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Calculate address: SDHC uses block addressing, SDSC uses byte addressing
    let addr = if high_capacity { lba } else { lba * 512 };
    info!("Read block: lba={=u32}, addr={=u32}", lba, addr);

    // CMD17: READ_SINGLE_BLOCK
    let r1 = sd_send_cmd(spi, cs, 17, addr, 0x01)?;
    if r1 != 0x00 {
        sd_end_cmd(spi, cs);
        return Err("CMD17 bad R1");
    }

    // Wait for data token (0xFE) - card sends this when data is ready
    for _ in 0..10_000 {
        let token = spi_txrx(spi, 0xFF);
        if token == 0xFE {  // 0xFE = start of data block
            // Read 512 bytes of data
            for i in 0..512 {
                buf[i] = spi_txrx(spi, 0xFF);
            }
            // Read 2-byte CRC (we ignore it in SPI mode, but must read it)
            let _ = spi_txrx(spi, 0xFF);
            let _ = spi_txrx(spi, 0xFF);

            sd_end_cmd(spi, cs);
            return Ok(());
        }
    }

    sd_end_cmd(spi, cs);
    Err("data token timeout")
}

/// Write a 512-byte block to the SD card.
/// This is the fundamental write operation - all file writes use this.
fn sd_write_block<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    lba: u32,              // Logical Block Address (sector number)
    buf: &[u8; 512],       // Data to write (exactly 512 bytes)
    high_capacity: bool,   // true for SDHC/SDXC, false for SDSC
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Calculate address: SDHC uses block addressing, SDSC uses byte addressing
    let addr = if high_capacity { lba } else { lba * 512 };
    info!("Write block: lba={=u32}, addr={=u32}", lba, addr);

    // CMD24: WRITE_SINGLE_BLOCK
    let r1 = sd_send_cmd(spi, cs, 24, addr, 0x01)?;
    if r1 != 0x00 {
        sd_end_cmd(spi, cs);
        return Err("CMD24 bad R1");
    }

    // Send start token (0xFE) - tells card "data is coming now"
    let _ = spi_txrx(spi, 0xFE);

    // Send 512 bytes of data
    for i in 0..512 {
        let _ = spi_txrx(spi, buf[i]);
    }

    // Send dummy CRC (2 bytes) - not checked in SPI mode, but required by protocol
    let _ = spi_txrx(spi, 0xFF);
    let _ = spi_txrx(spi, 0xFF);

    // Wait for data response token from card
    let response = spi_txrx(spi, 0xFF);
    // Bits 0-4 of response: xxx0101 = data accepted
    if (response & 0x1F) != 0x05 {
        sd_end_cmd(spi, cs);
        return Err("Write data rejected");
    }

    // Wait for card to finish writing (card holds DO low while busy)
    for _ in 0..100_000 {
        let status = spi_txrx(spi, 0xFF);
        if status == 0xFF {  // 0xFF = card is no longer busy
            sd_end_cmd(spi, cs);
            return Ok(());
        }
    }

    sd_end_cmd(spi, cs);
    Err("Write busy timeout")
}

// ============================================================================
// HIGH-LEVEL FAT32 FILESYSTEM FUNCTIONS
// ============================================================================
// These functions provide a user-friendly interface to the FAT32 filesystem.
// They handle:
// - File creation, reading, writing, deletion
// - Directory creation, navigation, deletion
// - Path parsing (e.g., "/DOCS/REPORT.TXT")
// - File renaming and moving between directories

/// Write a file to the root directory (legacy function for compatibility).
/// Modern code should use fat32_write_file_at_path() which supports subdirectories.
fn fat32_write_file<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    filename: &str,
    data: &[u8],
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Delegate to the directory-based version, using root directory
    fat32_write_file_in_dir(spi, cs, fat_info, fat_info.root_dir_cluster, filename, data, high_capacity)
}

/// Find a file in a directory by name and return its directory entry.
/// This searches only the first sector of the directory (simple implementation).
/// A full implementation would scan all sectors/clusters of the directory.
fn fat32_find_file<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    dir_cluster: u32,   // Which directory to search in
    filename: &str,     // Filename to search for (e.g., "README.TXT")
    high_capacity: bool,
) -> Result<Option<DirEntry>, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Create search name in 8.3 format for comparison
    let search_entry = DirEntry::new(filename, 0)?;
    
    // Read first sector of directory
    let dir_lba = fat_info.cluster_to_lba(dir_cluster);
    let mut buf = [0u8; 512];
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity)?;
    
    // Scan directory entries (16 entries per sector: 512 bytes / 32 bytes per entry)
    for entry_idx in 0..16 {
        let offset = entry_idx * 32;
        if let Some(entry) = DirEntry::parse(&buf[offset..offset + 32]) {
            // Compare names (case-insensitive since FAT32 stores uppercase)
            if entry.name == search_entry.name {
                return Ok(Some(entry));
            }
        }
        
        // Check for end of directory (first byte = 0x00)
        if buf[offset] == 0x00 {
            break;
        }
    }
    
    Ok(None)  // File not found
}

/// Create a new directory
fn fat32_create_directory<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    parent_cluster: u32,
    dirname: &str,
    high_capacity: bool,
) -> Result<u32, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Creating directory: {}", dirname);
    
    // Allocate a cluster for the new directory
    let dir_cluster = fat_info.find_free_cluster(spi, cs, 2, high_capacity)?;
    info!("  Allocated cluster {=u32} for directory", dir_cluster);
    
    // Mark cluster as end-of-chain in FAT
    fat_info.write_fat_entry(spi, cs, dir_cluster, 0x0FFF_FFFF, high_capacity)?;
    
    // Initialize directory cluster with . and .. entries
    let mut dir_buf = [0u8; 512];
    
    // Create "." entry (points to self)
    let dot_entry = DirEntry {
        name: [b'.', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' '],
        attr: 0x10,  // Directory attribute
        size: 0,
        start_cluster: dir_cluster,
    };
    dot_entry.encode(&mut dir_buf[0..32]);
    
    // Create ".." entry (points to parent)
    let dotdot_entry = DirEntry {
        name: [b'.', b'.', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' '],
        attr: 0x10,  // Directory attribute
        size: 0,
        start_cluster: parent_cluster,
    };
    dotdot_entry.encode(&mut dir_buf[32..64]);
    
    // Write directory cluster
    let dir_lba = fat_info.cluster_to_lba(dir_cluster);
    sd_write_block(spi, cs, dir_lba, &dir_buf, high_capacity)?;
    
    // Add directory entry to parent directory
    let mut dir_entry = DirEntry::new(dirname, 0x10)?;  // 0x10 = directory attribute
    dir_entry.start_cluster = dir_cluster;
    dir_entry.size = 0;  // Directories have 0 size in FAT32
    
    fat32_add_dir_entry(spi, cs, fat_info, parent_cluster, &dir_entry, high_capacity)?;
    
    info!("Directory created successfully");
    Ok(dir_cluster)
}

/// Add a directory entry to any directory (not just root)
fn fat32_add_dir_entry<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    dir_cluster: u32,
    entry: &DirEntry,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    let dir_lba = fat_info.cluster_to_lba(dir_cluster);
    let mut buf = [0u8; 512];
    
    // Search for a free slot in the directory
    // For simplicity, we'll just scan the first sector
    // A full implementation would scan multiple sectors/clusters
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity)?;
    
    // Find first free entry (starts with 0x00 or 0xE5)
    let mut found_slot = None;
    for i in 0..16 {
        let offset = i * 32;
        let first_byte = buf[offset];
        
        if first_byte == 0x00 || first_byte == 0xE5 {
            found_slot = Some(offset);
            break;
        }
    }
    
    match found_slot {
        Some(offset) => {
            // Write the directory entry
            entry.encode(&mut buf[offset..offset + 32]);
            sd_write_block(spi, cs, dir_lba, &buf, high_capacity)?;
            info!("Directory entry added at offset {=usize}", offset);
            Ok(())
        }
        None => Err("No free directory entries")
    }
}

/// List all entries in a directory
fn fat32_list_directory<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    dir_cluster: u32,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Listing directory (cluster {=u32}):", dir_cluster);
    
    let dir_lba = fat_info.cluster_to_lba(dir_cluster);
    let mut buf = [0u8; 512];
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity)?;
    
    for entry_idx in 0..16 {
        let offset = entry_idx * 32;
        let first_byte = buf[offset];
        
        // End of directory
        if first_byte == 0x00 {
            break;
        }
        
        // Skip deleted entries
        if first_byte == 0xE5 {
            continue;
        }
        
        if let Some(entry) = DirEntry::parse(&buf[offset..offset + 32]) {
            let file_type = if (entry.attr & 0x10) != 0 { "DIR " } else { "FILE" };
            info!("  [{=str}] {=[u8]:a} - {=u32} bytes, cluster {=u32}", 
                file_type, &entry.name[..], entry.size, entry.start_cluster);
        }
    }
    
    Ok(())
}

/// Parse a path and navigate to the target directory
fn fat32_navigate_path<'a, SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    path: &'a str,
    high_capacity: bool,
) -> Result<(u32, Option<&'a str>), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Start from root directory
    let mut current_cluster = fat_info.root_dir_cluster;
    
    // Skip leading slash
    let path = if path.starts_with('/') {
        &path[1..]
    } else {
        path
    };
    
    // Empty path means root
    if path.is_empty() {
        return Ok((current_cluster, None));
    }
    
    // Split path into components
    let mut remaining = path;
    let mut last_component = None;
    
    while !remaining.is_empty() {
        // Find next slash
        let (component, rest) = if let Some(pos) = remaining.find('/') {
            (&remaining[..pos], &remaining[pos + 1..])
        } else {
            // Last component - could be file or directory
            last_component = Some(remaining);
            break;
        };
        
        // Navigate to this directory
        if let Some(entry) = fat32_find_file(spi, cs, fat_info, current_cluster, component, high_capacity)? {
            if (entry.attr & 0x10) == 0 {
                return Err("Path component is not a directory");
            }
            current_cluster = entry.start_cluster;
        } else {
            return Err("Directory not found");
        }
        
        remaining = rest;
    }
    
    Ok((current_cluster, last_component))
}

/// Write a file at a specific path (creates parent directories if needed)
fn fat32_write_file_at_path<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    path: &str,
    data: &[u8],
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Writing file at path: {}", path);
    
    // Navigate to parent directory and get filename
    let (parent_cluster, filename) = fat32_navigate_path(spi, cs, fat_info, path, high_capacity)?;
    
    let filename = filename.ok_or("Path is a directory, not a file")?;
    
    // Write the file in the parent directory
    fat32_write_file_in_dir(spi, cs, fat_info, parent_cluster, filename, data, high_capacity)?;
    
    Ok(())
}

/// Write a file to a specific directory cluster.
/// This handles:
/// - Allocating clusters for the file data
/// - Writing data to those clusters
/// - Creating a directory entry
/// - Updating the FAT to link clusters together
fn fat32_write_file_in_dir<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    dir_cluster: u32,    // Which directory to create file in
    filename: &str,      // Name of file (8.3 format)
    data: &[u8],         // File contents
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Writing file: {} in directory cluster {=u32}", filename, dir_cluster);

    // Create directory entry with file metadata
    let mut dir_entry = DirEntry::new(filename, 0x20)?; // 0x20 = archive attribute (regular file)
    dir_entry.size = data.len() as u32;

    // Calculate how many clusters we need to store this file
    let bytes_per_cluster = fat_info.sectors_per_cluster as u32 * fat_info.bytes_per_sector as u32;
    let clusters_needed = if data.is_empty() {
        0  // Empty file needs no clusters
    } else {
        // Round up: (size + cluster_size - 1) / cluster_size
        (data.len() as u32 + bytes_per_cluster - 1) / bytes_per_cluster
    };
    
    info!("  File size: {=u32} bytes, clusters needed: {=u32}", data.len() as u32, clusters_needed);

    if clusters_needed == 0 {
        // Empty file - no clusters needed
        dir_entry.start_cluster = 0;
    } else {
        // Allocate first cluster
        let first_cluster = fat_info.find_free_cluster(spi, cs, 2, high_capacity)?;
        dir_entry.start_cluster = first_cluster;
        info!("  First cluster: {=u32}", first_cluster);

        let mut current_cluster = first_cluster;
        let mut bytes_written = 0;

        // Write data cluster by cluster
        for cluster_idx in 0..clusters_needed {
            // Allocate next cluster if needed (or mark end-of-chain for last cluster)
            let next_cluster = if cluster_idx + 1 < clusters_needed {
                fat_info.find_free_cluster(spi, cs, current_cluster + 1, high_capacity)?
            } else {
                0x0FFF_FFFF // End of chain marker
            };

            // Update FAT to link current cluster to next (or mark end-of-chain)
            fat_info.write_fat_entry(spi, cs, current_cluster, next_cluster, high_capacity)?;

            // Write data to all sectors in this cluster
            let cluster_lba = fat_info.cluster_to_lba(current_cluster);
            
            for sector_idx in 0..fat_info.sectors_per_cluster {
                let mut sector_buf = [0u8; 512];
                let sector_offset = bytes_written;
                let bytes_to_copy = (data.len() - sector_offset).min(512);  // Don't overflow
                
                if bytes_to_copy > 0 {
                    // Copy data into sector buffer (rest stays zero-filled)
                    sector_buf[..bytes_to_copy].copy_from_slice(&data[sector_offset..sector_offset + bytes_to_copy]);
                }
                
                // Write sector to SD card
                sd_write_block(spi, cs, cluster_lba + sector_idx as u32, &sector_buf, high_capacity)?;
                bytes_written += bytes_to_copy;
                
                if bytes_written >= data.len() {
                    break;  // All data written
                }
            }

            if next_cluster == 0x0FFF_FFFF {
                break;  // Last cluster
            }
            current_cluster = next_cluster;
        }

        info!("  Wrote {=u32} bytes across {=u32} clusters", bytes_written as u32, clusters_needed);
    }

    // Add directory entry to the parent directory
    fat32_add_dir_entry(spi, cs, fat_info, dir_cluster, &dir_entry, high_capacity)?;

    info!("File written successfully!");
    Ok(())
}

/// Read a file from a specific path
fn fat32_read_file_at_path<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    path: &str,
    buffer: &mut [u8],
    high_capacity: bool,
) -> Result<usize, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Reading file at path: {}", path);
    
    // Navigate to parent directory and get filename
    let (parent_cluster, filename) = fat32_navigate_path(spi, cs, fat_info, path, high_capacity)?;
    
    let filename = filename.ok_or("Path is a directory, not a file")?;
    
    // Find the file
    let entry = fat32_find_file(spi, cs, fat_info, parent_cluster, filename, high_capacity)?
        .ok_or("File not found")?;
    
    // Make sure it's a file, not a directory
    if (entry.attr & 0x10) != 0 {
        return Err("Path is a directory, not a file");
    }
    
    // Read the file
    fat32_read_file_complete(spi, cs, fat_info, entry.start_cluster, entry.size, buffer, high_capacity)
}

/// Read a complete file by following the FAT chain
fn fat32_read_file_complete<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    start_cluster: u32,
    file_size: u32,
    buffer: &mut [u8],
    high_capacity: bool,
) -> Result<usize, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    if file_size == 0 {
        return Ok(0);
    }
    
    if buffer.len() < file_size as usize {
        return Err("Buffer too small");
    }
    
    let mut bytes_read = 0;
    let mut current_cluster = start_cluster;
    
    loop {
        // Read all sectors in this cluster
        let cluster_lba = fat_info.cluster_to_lba(current_cluster);
        
        for sector_idx in 0..fat_info.sectors_per_cluster {
            let mut sector_buf = [0u8; 512];
            sd_read_block(spi, cs, cluster_lba + sector_idx as u32, &mut sector_buf, high_capacity)?;
            
            // Copy data to output buffer
            let bytes_to_copy = ((file_size as usize - bytes_read).min(512)).min(buffer.len() - bytes_read);
            if bytes_to_copy > 0 {
                buffer[bytes_read..bytes_read + bytes_to_copy].copy_from_slice(&sector_buf[..bytes_to_copy]);
                bytes_read += bytes_to_copy;
            }
            
            if bytes_read >= file_size as usize {
                return Ok(bytes_read);
            }
        }
        
        // Get next cluster from FAT
        let next_cluster = fat_info.read_fat_entry(spi, cs, current_cluster, high_capacity)?;
        
        // Check for end of chain
        if next_cluster >= 0x0FFF_FFF8 {
            return Ok(bytes_read);
        }
        
        current_cluster = next_cluster;
    }
}

/// Delete a file by freeing its clusters and marking directory entry as deleted
fn fat32_delete_file<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    dir_cluster: u32,
    filename: &str,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Deleting file: {} from directory cluster {=u32}", filename, dir_cluster);
    
    // Find the file in the directory
    let search_entry = DirEntry::new(filename, 0)?;
    let dir_lba = fat_info.cluster_to_lba(dir_cluster);
    let mut buf = [0u8; 512];
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity)?;
    
    let mut entry_offset = None;
    let mut file_entry = None;
    
    // Scan directory entries to find the file
    for entry_idx in 0..16 {
        let offset = entry_idx * 32;
        if let Some(entry) = DirEntry::parse(&buf[offset..offset + 32]) {
            if entry.name == search_entry.name {
                entry_offset = Some(offset);
                file_entry = Some(entry);
                break;
            }
        }
    }
    
    let entry_offset = entry_offset.ok_or("File not found")?;
    let file_entry = file_entry.unwrap();
    
    // Make sure it's a file, not a directory
    if (file_entry.attr & 0x10) != 0 {
        return Err("Cannot delete directory with delete_file (use delete_directory)");
    }
    
    // Free all clusters in the FAT chain
    if file_entry.start_cluster != 0 {
        let mut current_cluster = file_entry.start_cluster;
        let mut clusters_freed = 0;
        
        loop {
            // Read next cluster before we free current one
            let next_cluster = fat_info.read_fat_entry(spi, cs, current_cluster, high_capacity)?;
            
            // Mark cluster as free (0x00000000)
            fat_info.write_fat_entry(spi, cs, current_cluster, 0x0000_0000, high_capacity)?;
            clusters_freed += 1;
            
            // Check if we've reached end of chain
            if next_cluster >= 0x0FFF_FFF8 || next_cluster == 0 {
                break;
            }
            
            current_cluster = next_cluster;
        }
        
        info!("  Freed {=u32} clusters", clusters_freed);
    }
    
    // Mark directory entry as deleted (first byte = 0xE5)
    buf[entry_offset] = 0xE5;
    sd_write_block(spi, cs, dir_lba, &buf, high_capacity)?;
    
    info!("File deleted successfully");
    Ok(())
}

/// Delete a file at a specific path
fn fat32_delete_file_at_path<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    path: &str,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Deleting file at path: {}", path);
    
    // Navigate to parent directory and get filename
    let (parent_cluster, filename) = fat32_navigate_path(spi, cs, fat_info, path, high_capacity)?;
    let filename = filename.ok_or("Path is root directory, not a file")?;
    
    // Delete the file
    fat32_delete_file(spi, cs, fat_info, parent_cluster, filename, high_capacity)
}

/// Delete an empty directory
fn fat32_delete_directory<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    parent_cluster: u32,
    dirname: &str,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Deleting directory: {}", dirname);
    
    // Find the directory entry
    let dir_entry = fat32_find_file(spi, cs, fat_info, parent_cluster, dirname, high_capacity)?
        .ok_or("Directory not found")?;
    
    // Verify it's a directory
    if (dir_entry.attr & 0x10) == 0 {
        return Err("Not a directory");
    }
    
    // Check if directory is empty (only . and .. entries)
    let dir_lba = fat_info.cluster_to_lba(dir_entry.start_cluster);
    let mut buf = [0u8; 512];
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity)?;
    
    let mut entry_count = 0;
    for entry_idx in 0..16 {
        let offset = entry_idx * 32;
        let first_byte = buf[offset];
        
        if first_byte == 0x00 {
            break; // End of directory
        }
        
        if first_byte != 0xE5 {
            entry_count += 1;
        }
    }
    
    // Directory should only have . and .. entries (2 entries)
    if entry_count > 2 {
        return Err("Directory not empty");
    }
    
    // Free the directory's cluster
    fat_info.write_fat_entry(spi, cs, dir_entry.start_cluster, 0x0000_0000, high_capacity)?;
    
    // Remove directory entry from parent
    let parent_lba = fat_info.cluster_to_lba(parent_cluster);
    sd_read_block(spi, cs, parent_lba, &mut buf, high_capacity)?;
    
    // Find and mark the directory entry as deleted
    let search_entry = DirEntry::new(dirname, 0x10)?;
    for entry_idx in 0..16 {
        let offset = entry_idx * 32;
        if let Some(entry) = DirEntry::parse(&buf[offset..offset + 32]) {
            if entry.name == search_entry.name {
                buf[offset] = 0xE5; // Mark as deleted
                sd_write_block(spi, cs, parent_lba, &buf, high_capacity)?;
                info!("Directory deleted successfully");
                return Ok(());
            }
        }
    }
    
    Err("Directory entry not found in parent")
}

/// Delete a directory at a specific path (must be empty)
fn fat32_delete_directory_at_path<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    path: &str,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Deleting directory at path: {}", path);
    
    // Navigate to parent directory and get directory name
    let (parent_cluster, dirname) = fat32_navigate_path(spi, cs, fat_info, path, high_capacity)?;
    let dirname = dirname.ok_or("Cannot delete root directory")?;
    
    // Delete the directory
    fat32_delete_directory(spi, cs, fat_info, parent_cluster, dirname, high_capacity)
}

/// Rename a file in the same directory
fn fat32_rename_file<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    dir_cluster: u32,
    old_name: &str,
    new_name: &str,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Renaming file: {} -> {} in directory cluster {=u32}", old_name, new_name, dir_cluster);
    
    // Check if new name already exists
    if let Some(_) = fat32_find_file(spi, cs, fat_info, dir_cluster, new_name, high_capacity)? {
        return Err("File with new name already exists");
    }
    
    // Find the file with old name
    let search_entry = DirEntry::new(old_name, 0)?;
    let dir_lba = fat_info.cluster_to_lba(dir_cluster);
    let mut buf = [0u8; 512];
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity)?;
    
    // Find the entry to rename
    for entry_idx in 0..16 {
        let offset = entry_idx * 32;
        if let Some(mut entry) = DirEntry::parse(&buf[offset..offset + 32]) {
            if entry.name == search_entry.name {
                // Create new name in 8.3 format
                let new_entry = DirEntry::new(new_name, entry.attr)?;
                
                // Update the name while preserving other fields
                entry.name = new_entry.name;
                
                // Write back the modified entry
                entry.encode(&mut buf[offset..offset + 32]);
                sd_write_block(spi, cs, dir_lba, &buf, high_capacity)?;
                
                info!("File renamed successfully");
                return Ok(());
            }
        }
        
        // Check for end of directory
        if buf[offset] == 0x00 {
            break;
        }
    }
    
    Err("File not found")
}

/// Rename a file at a specific path
fn fat32_rename_file_at_path<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    old_path: &str,
    new_name: &str,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Renaming file at path: {} -> {}", old_path, new_name);
    
    // Navigate to parent directory and get old filename
    let (parent_cluster, old_filename) = fat32_navigate_path(spi, cs, fat_info, old_path, high_capacity)?;
    let old_filename = old_filename.ok_or("Path is root directory, not a file")?;
    
    // Rename the file in its current directory
    fat32_rename_file(spi, cs, fat_info, parent_cluster, old_filename, new_name, high_capacity)
}

/// Move a file from one directory to another
fn fat32_move_file<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    src_path: &str,
    dest_path: &str,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("Moving file: {} -> {}", src_path, dest_path);
    
    // Navigate to source parent and get filename
    let (src_parent_cluster, src_filename) = fat32_navigate_path(spi, cs, fat_info, src_path, high_capacity)?;
    let src_filename = src_filename.ok_or("Source path is a directory, not a file")?;
    
    // Navigate to destination parent and get new filename
    let (dest_parent_cluster, dest_filename) = fat32_navigate_path(spi, cs, fat_info, dest_path, high_capacity)?;
    let dest_filename = dest_filename.ok_or("Destination path is a directory, not a file")?;
    
    // Find the source file entry
    let src_entry = fat32_find_file(spi, cs, fat_info, src_parent_cluster, src_filename, high_capacity)?
        .ok_or("Source file not found")?;
    
    // Check if destination file already exists
    if let Some(_) = fat32_find_file(spi, cs, fat_info, dest_parent_cluster, dest_filename, high_capacity)? {
        return Err("Destination file already exists");
    }
    
    // Create new directory entry in destination with new name
    let mut new_entry = DirEntry::new(dest_filename, src_entry.attr)?;
    new_entry.start_cluster = src_entry.start_cluster;
    new_entry.size = src_entry.size;
    
    // Add entry to destination directory
    fat32_add_dir_entry(spi, cs, fat_info, dest_parent_cluster, &new_entry, high_capacity)?;
    
    // Remove entry from source directory (mark as deleted)
    let src_lba = fat_info.cluster_to_lba(src_parent_cluster);
    let mut buf = [0u8; 512];
    sd_read_block(spi, cs, src_lba, &mut buf, high_capacity)?;
    
    let search_entry = DirEntry::new(src_filename, 0)?;
    for entry_idx in 0..16 {
        let offset = entry_idx * 32;
        if let Some(entry) = DirEntry::parse(&buf[offset..offset + 32]) {
            if entry.name == search_entry.name {
                buf[offset] = 0xE5; // Mark as deleted
                sd_write_block(spi, cs, src_lba, &buf, high_capacity)?;
                info!("File moved successfully");
                return Ok(());
            }
        }
    }
    
    Err("Failed to remove source entry")
}

/// Verify that a file or directory exists at the given path
fn fat32_verify_exists<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    path: &str,
    high_capacity: bool,
) -> Result<bool, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    match fat32_navigate_path(spi, cs, fat_info, path, high_capacity) {
        Ok((parent_cluster, Some(filename))) => {
            // It's a file or directory - check if it exists
            match fat32_find_file(spi, cs, fat_info, parent_cluster, filename, high_capacity)? {
                Some(_) => Ok(true),
                None => Ok(false),
            }
        }
        Ok((_, None)) => {
            // It's the root directory or a valid directory path
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

/// Main entry point for the embedded program.
/// The #[entry] attribute marks this as the entry point (replaces standard main()).
/// The ! return type means this function never returns (embedded systems run forever).
#[entry]
fn main() -> ! {
    info!("sd_test: Advanced Filesystem Test");

    // ========================================================================
    // HARDWARE INITIALIZATION
    // ========================================================================
    
    // Take ownership of peripheral singletons (can only be done once)
    let mut pac = pac::Peripherals::take().unwrap();
    let _core = pac::CorePeripherals::take().unwrap();
    let mut watchdog = Watchdog::new(pac.WATCHDOG);

    // Initialize clocks and PLLs (Phase-Locked Loops) to run at proper speed
    // XOSC = external oscillator crystal (12 MHz on Pico)
    let clocks = hal::clocks::init_clocks_and_plls(
        rp_pico::XOSC_CRYSTAL_FREQ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    // ========================================================================
    // GPIO AND SPI SETUP FOR SD CARD
    // ========================================================================
    
    // Setup GPIO pins through Single-cycle I/O block
    let sio = Sio::new(pac.SIO);
    let pins = rp_pico::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    // Configure SPI pins (using SPI0 peripheral on Pico)
    let sck = pins.gpio18.into_function::<hal::gpio::FunctionSpi>();   // Clock
    let mosi = pins.gpio19.into_function::<hal::gpio::FunctionSpi>();  // Master Out Slave In
    let miso = pins.gpio16.into_function::<hal::gpio::FunctionSpi>();  // Master In Slave Out
    let mut cs = pins.gpio17.into_push_pull_output();                  // Chip Select (manual control)
    cs.set_high().ok();  // Start with CS high (deselected)

    // Initialize SPI peripheral at 400 kHz (slow for initialization)
    // MODE_0: CPOL=0, CPHA=0 (clock idles low, data sampled on rising edge)
    let mut spi = hal::spi::Spi::<_, _, _, 8>::new(pac.SPI0, (mosi, miso, sck)).init(
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
        400.kHz(),  // 400 kHz is safe for SD card initialization
        MODE_0,
    );

    // ========================================================================
    // SD CARD INITIALIZATION
    // ========================================================================
    
    info!("Initializing SD card...");
    let high_capacity = match sd_init(&mut spi, &mut cs) {
        Ok(hc) => hc,
        Err(e) => {
            error!("SD init failed: {}", e);
            loop { cortex_m::asm::bkpt(); }  // Breakpoint for debugging
        }
    };

    // ========================================================================
    // FILESYSTEM INITIALIZATION
    // ========================================================================
    
    // Read boot sector (sector 0) to get filesystem metadata
    let mut buf = [0u8; 512];
    sd_read_block(&mut spi, &mut cs, 0, &mut buf, high_capacity).ok();
    
    // Parse FAT32 information from boot sector
    let fat_info = match Fat32Info::parse(&buf) {
        Ok(info) => {
            info!("=== FAT32 Filesystem Ready ===");
            info!("  Root cluster: {=u32}", info.root_dir_cluster);
            info!("  Sectors/cluster: {=u8}", info.sectors_per_cluster);
            info!("==============================");
            info
        }
        Err(e) => {
            error!("Failed to parse FAT32: {}", e);
            loop { cortex_m::asm::bkpt(); }
        }
    };

    // ========================================================================
    // TEST 1: Create a directory structure
    // ========================================================================
    info!("\n=== TEST 1: Creating Directory Structure ===");
    
    match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "DOCS", high_capacity) {
        Ok(docs_cluster) => {
            info!("✓ Created /DOCS directory");
            
            // Create a subdirectory
            match fat32_create_directory(&mut spi, &mut cs, &fat_info, docs_cluster, "REPORTS", high_capacity) {
                Ok(_) => info!("✓ Created /DOCS/REPORTS subdirectory"),
                Err(e) => error!("✗ Failed to create subdirectory: {}", e),
            }
        }
        Err(e) => error!("✗ Failed to create directory: {}", e),
    }

    match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "MUSIC", high_capacity) {
        Ok(_) => info!("✓ Created /MUSIC directory"),
        Err(e) => error!("✗ Failed: {}", e),
    }

    // ========================================================================
    // TEST 2: Write files to different directories
    // ========================================================================
    info!("\n=== TEST 2: Writing Files to Directories ===");
    
    let readme_data = b"Welcome to Pico OS Filesystem!\n\nThis filesystem supports:\n- Multi-cluster files\n- Subdirectories\n- Path navigation\n\nBuilt with Rust!";
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/README.TXT", readme_data, high_capacity) {
        Ok(()) => info!("✓ Wrote /README.TXT ({} bytes)", readme_data.len()),
        Err(e) => error!("✗ Failed: {}", e),
    }

    let doc_data = b"Project Documentation\n\nVersion 1.0\nDate: 2025-11-29\n\nThis is a test document in the DOCS folder.";
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/DOCS/GUIDE.TXT", doc_data, high_capacity) {
        Ok(()) => info!("✓ Wrote /DOCS/GUIDE.TXT ({} bytes)", doc_data.len()),
        Err(e) => error!("✗ Failed: {}", e),
    }

    // Write a larger multi-cluster file
    let large_data = b"This is a larger file to test multi-cluster support!\n\
        Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor \
        incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud \
        exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure \
        dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. \
        Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit \
        anim id est laborum. Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium \
        doloremque laudantium, totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi \
        architecto beatae vitae dicta sunt explicabo. Nemo enim ipsam voluptatem quia voluptas sit aspernatur \
        aut odit aut fugit, sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt.";
    
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/DOCS/REPORT.TXT", large_data, high_capacity) {
        Ok(()) => info!("✓ Wrote /DOCS/REPORT.TXT ({} bytes, multi-cluster)", large_data.len()),
        Err(e) => error!("✗ Failed: {}", e),
    }

    // ========================================================================
    // 🎨 YOUR CUSTOM FILES - Add your own files and directories here!
    // ========================================================================
    // Uncomment and edit the examples below to create your own files:
    
    
    info!("\n=== Creating Custom Files ===");
    
    // Example 1: Create a simple text file
    let my_data = b"Hello! This is my custom file.";
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", my_data, high_capacity) {
        Ok(()) => info!("✓ Created /MYFILE.TXT"),
        Err(e) => error!("✗ Failed: {}", e),
    }
    
    // Example 2: Create a new directory
    match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "PHOTOS", high_capacity) {
        Ok(_) => info!("✓ Created /PHOTOS directory"),
        Err(e) => error!("✗ Failed: {}", e),
    }
    
    // Example 3: Create a file in a subdirectory
    let photo_info = b"Photo Information\nDate: 2025-11-29\nCamera: Pico";
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/PHOTOS/INFO.TXT", photo_info, high_capacity) {
        Ok(()) => info!("✓ Created /PHOTOS/INFO.TXT"),
        Err(e) => error!("✗ Failed: {}", e),
    }
    

    // ========================================================================
    // TEST 3: Read files back and verify
    // ========================================================================
    info!("\n=== TEST 3: Reading Files Back ===");
    
    let mut read_buf = [0u8; 1024];
    match fat32_read_file_at_path(&mut spi, &mut cs, &fat_info, "/README.TXT", &mut read_buf, high_capacity) {
        Ok(bytes_read) => {
            info!("✓ Read /README.TXT: {} bytes", bytes_read);
            info!("  First 64 bytes: {=[u8]:a}", &read_buf[..64.min(bytes_read)]);
        }
        Err(e) => error!("✗ Failed to read: {}", e),
    }

    match fat32_read_file_at_path(&mut spi, &mut cs, &fat_info, "/DOCS/REPORT.TXT", &mut read_buf, high_capacity) {
        Ok(bytes_read) => {
            info!("✓ Read /DOCS/REPORT.TXT: {} bytes (multi-cluster)", bytes_read);
            if bytes_read == large_data.len() {
                info!("  ✓ Size matches!");
            } else {
                error!("  ✗ Size mismatch! Expected {} got {}", large_data.len(), bytes_read);
            }
        }
        Err(e) => error!("✗ Failed to read: {}", e),
    }

    // ========================================================================
    // TEST 4: List directory contents
    // ========================================================================
    info!("\n=== TEST 4: Listing Directories ===");
    
    info!("Root directory:");
    fat32_list_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, high_capacity).ok();
    
    // List DOCS directory
    if let Ok(Some(docs_entry)) = fat32_find_file(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "DOCS", high_capacity) {
        info!("\n/DOCS directory:");
        fat32_list_directory(&mut spi, &mut cs, &fat_info, docs_entry.start_cluster, high_capacity).ok();
    }

    // ========================================================================
    // TEST 5: Verify all created files and directories exist
    // ========================================================================
    info!("\n=== TEST 5: Verifying Filesystem Structure ===");
    
    let paths_to_verify = [
        ("/README.TXT", "FILE"),
        ("/DOCS", "DIR"),
        ("/DOCS/GUIDE.TXT", "FILE"),
        ("/DOCS/REPORT.TXT", "FILE"),
        ("/DOCS/REPORTS", "DIR"),
        ("/MUSIC", "DIR"),
    ];
    
    let mut all_verified = true;
    for (path, expected_type) in &paths_to_verify {
        match fat32_verify_exists(&mut spi, &mut cs, &fat_info, path, high_capacity) {
            Ok(true) => info!("  ✓ {} exists: {}", expected_type, path),
            Ok(false) => {
                error!("  ✗ {} NOT FOUND: {}", expected_type, path);
                all_verified = false;
            }
            Err(e) => {
                error!("  ✗ Error checking {}: {}", path, e);
                all_verified = false;
            }
        }
    }
    
    if all_verified {
        info!("\n✅ All files and directories verified successfully!");
    } else {
        error!("\n❌ Some files or directories are missing!");
    }

    // ========================================================================
    // TEST 6: File and Directory Deletion
    // ========================================================================
    info!("\n=== TEST 6: Testing File Deletion ===");
    
    // Delete a file
    match fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", high_capacity) {
        Ok(()) => info!("✓ Deleted /MYFILE.TXT"),
        Err(e) => error!("✗ Failed to delete file: {}", e),
    }
    
    // Verify file is gone
    match fat32_verify_exists(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", high_capacity) {
        Ok(false) => info!("✓ Verified /MYFILE.TXT is deleted"),
        Ok(true) => error!("✗ File still exists!"),
        Err(e) => error!("✗ Error verifying: {}", e),
    }
    
    // Delete a file from subdirectory
    match fat32_delete_file_at_path(&mut spi, &mut cs, &fat_info, "/PHOTOS/INFO.TXT", high_capacity) {
        Ok(()) => info!("✓ Deleted /PHOTOS/INFO.TXT"),
        Err(e) => error!("✗ Failed: {}", e),
    }
    
    // Delete an empty directory (PHOTOS should be empty now)
    match fat32_delete_directory_at_path(&mut spi, &mut cs, &fat_info, "/PHOTOS", high_capacity) {
        Ok(()) => info!("✓ Deleted /PHOTOS directory"),
        Err(e) => error!("✗ Failed to delete directory: {}", e),
    }
    
    // Try to delete a non-empty directory (should fail)
    info!("Testing deletion of non-empty directory (should fail):");
    match fat32_delete_directory_at_path(&mut spi, &mut cs, &fat_info, "/DOCS", high_capacity) {
        Ok(()) => error!("✗ Should not have deleted non-empty directory!"),
        Err(e) => info!("✓ Correctly rejected: {}", e),
    }
    
    // List root directory after deletions
    info!("\nRoot directory after deletions:");
    fat32_list_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, high_capacity).ok();

    // ========================================================================
    // TEST 7: File Renaming
    // ========================================================================
    info!("\n=== TEST 7: Testing File Renaming ===");
    
    // Rename a file in the root directory
    match fat32_rename_file_at_path(&mut spi, &mut cs, &fat_info, "/README.TXT", "WELCOME.TXT", high_capacity) {
        Ok(()) => info!("✓ Renamed /README.TXT -> /WELCOME.TXT"),
        Err(e) => error!("✗ Failed to rename: {}", e),
    }
    
    // Verify old name is gone and new name exists
    match fat32_verify_exists(&mut spi, &mut cs, &fat_info, "/README.TXT", high_capacity) {
        Ok(false) => info!("✓ Verified old name /README.TXT is gone"),
        Ok(true) => error!("✗ Old name still exists!"),
        Err(e) => error!("✗ Error: {}", e),
    }
    
    match fat32_verify_exists(&mut spi, &mut cs, &fat_info, "/WELCOME.TXT", high_capacity) {
        Ok(true) => info!("✓ Verified new name /WELCOME.TXT exists"),
        Ok(false) => error!("✗ New name not found!"),
        Err(e) => error!("✗ Error: {}", e),
    }
    
    // Rename a file in a subdirectory
    match fat32_rename_file_at_path(&mut spi, &mut cs, &fat_info, "/DOCS/GUIDE.TXT", "MANUAL.TXT", high_capacity) {
        Ok(()) => info!("✓ Renamed /DOCS/GUIDE.TXT -> /DOCS/MANUAL.TXT"),
        Err(e) => error!("✗ Failed: {}", e),
    }

    // ========================================================================
    // TEST 8: File Moving
    // ========================================================================
    info!("\n=== TEST 8: Testing File Moving ===");
    
    // Move a file from root to MUSIC directory
    match fat32_move_file(&mut spi, &mut cs, &fat_info, "/WELCOME.TXT", "/MUSIC/INFO.TXT", high_capacity) {
        Ok(()) => info!("✓ Moved /WELCOME.TXT -> /MUSIC/INFO.TXT"),
        Err(e) => error!("✗ Failed to move: {}", e),
    }
    
    // Verify file is gone from source and exists in destination
    match fat32_verify_exists(&mut spi, &mut cs, &fat_info, "/WELCOME.TXT", high_capacity) {
        Ok(false) => info!("✓ Verified file removed from source"),
        Ok(true) => error!("✗ File still exists in source!"),
        Err(e) => error!("✗ Error: {}", e),
    }
    
    match fat32_verify_exists(&mut spi, &mut cs, &fat_info, "/MUSIC/INFO.TXT", high_capacity) {
        Ok(true) => info!("✓ Verified file exists in destination"),
        Ok(false) => error!("✗ File not found in destination!"),
        Err(e) => error!("✗ Error: {}", e),
    }
    
    // Move and rename a file at the same time
    match fat32_move_file(&mut spi, &mut cs, &fat_info, "/DOCS/REPORT.TXT", "/MUSIC/NOTES.TXT", high_capacity) {
        Ok(()) => info!("✓ Moved /DOCS/REPORT.TXT -> /MUSIC/NOTES.TXT"),
        Err(e) => error!("✗ Failed: {}", e),
    }
    
    // List directories to show the changes
    info!("\nRoot directory after rename/move:");
    fat32_list_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, high_capacity).ok();
    
    if let Ok(Some(docs_entry)) = fat32_find_file(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "DOCS", high_capacity) {
        info!("\n/DOCS directory after rename/move:");
        fat32_list_directory(&mut spi, &mut cs, &fat_info, docs_entry.start_cluster, high_capacity).ok();
    }
    
    if let Ok(Some(music_entry)) = fat32_find_file(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "MUSIC", high_capacity) {
        info!("\n/MUSIC directory after move:");
        fat32_list_directory(&mut spi, &mut cs, &fat_info, music_entry.start_cluster, high_capacity).ok();
    }

    loop {
        cortex_m::asm::wfi();
    }
}
