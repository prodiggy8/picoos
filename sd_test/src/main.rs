// ============================================================================
// EMBEDDED SYSTEM CONFIGURATION
// ============================================================================
// Tell Rust we're not using the standard library (no OS, no heap by default)
#![no_std]
// We're not using the standard main function - embedded entry point instead
#![no_main]

// ---- Boot2 for RP2040 (Stage 2 Bootloader) ----
// Embassy handles the boot2 automatically through the "boot2-w25q080" feature
// No need to manually define BOOT2_FIRMWARE when using Embassy

// ---- Logging and Panic Handling ----
use defmt::*;           // Efficient logging for embedded systems (smaller than println!)
use defmt_rtt as _;     // Real-Time Transfer: sends logs over debug probe
use panic_probe as _;   // What to do when program panics (crashes)

// ---- Embassy Framework ----
use embassy_executor::Spawner;
use embassy_rp::{
    gpio::{Level, Output},
    spi::{Spi, Config as SpiConfig, Phase, Polarity},
};
use embassy_time::{Timer, Duration};

// ---- embedded-hal Traits for Hardware Independence ----
use embedded_hal::digital::OutputPin;  // GPIO output operations (set high/low)
use embedded_hal::spi::SpiBus;  // SPI communication and clock mode

// ---- heapless for no-std collections ----
use heapless;  // Vec and String alternatives for embedded systems

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
    async fn read_fat_entry<SPI, CS>(
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
        sd_read_block(spi, cs, fat_sector, &mut buf, high_capacity).await?;

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
    async fn write_fat_entry<SPI, CS>(
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
        sd_read_block(spi, cs, fat_sector, &mut buf, high_capacity).await?;

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
            sd_write_block(spi, cs, sector, &buf, high_capacity).await?;
        }

        Ok(())
    }

    /// Find a free cluster by scanning the FAT for a zero entry.
    /// This is like malloc() for the filesystem - finds free space to allocate.
    async fn find_free_cluster<SPI, CS>(
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
            
            let entry = self.read_fat_entry(spi, cs, cluster, high_capacity).await?;
            if entry == 0 {  // 0 means free cluster
                return Ok(cluster);
            }
        }

        // Wrap around and search from beginning if we didn't find one
        for cluster in 2..start_hint {
            let entry = self.read_fat_entry(spi, cs, cluster, high_capacity).await?;
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
async fn spi_txrx<SPI>(spi: &mut SPI, byte: u8) -> u8
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
async fn sd_end_cmd<SPI, CS>(spi: &mut SPI, cs: &mut CS)
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    let _ = spi_txrx(spi, 0xFF).await;  // Send one dummy byte for timing
    let _ = cs.set_high();              // Deselect card (CS = high)
    let _ = spi_txrx(spi, 0xFF).await;  // Send another byte (gives card time to finish)
}

/// Send a command to the SD card and wait for response.
/// SD commands are 6 bytes: [cmd | arg3 | arg2 | arg1 | arg0 | crc]
/// Returns the R1 response byte (status byte from card).
async fn sd_send_cmd<SPI, CS>(
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
    let _ = cs.set_low();                          // Select card (CS = low)
    let _ = spi_txrx(spi, 0xFF).await;             // Dummy byte for timing

    // Send 6-byte command packet
    let _ = spi_txrx(spi, 0x40 | cmd).await;        // Command byte (always starts with 01xxxxxx)
    let _ = spi_txrx(spi, (arg >> 24) as u8).await; // Argument byte 3 (MSB)
    let _ = spi_txrx(spi, (arg >> 16) as u8).await; // Argument byte 2
    let _ = spi_txrx(spi, (arg >> 8) as u8).await;  // Argument byte 1
    let _ = spi_txrx(spi, arg as u8).await;         // Argument byte 0 (LSB)
    let _ = spi_txrx(spi, crc).await;               // CRC byte

    // Wait for response (card sends 0xFF while processing, then response byte)
    // Response byte has MSB=0 (distinguishes from 0xFF idle bytes)
    for _ in 0..255 {
        let resp = spi_txrx(spi, 0xFF).await;
        if resp & 0x80 == 0 {  // MSB clear = valid response
            return Ok(resp);
        }
    }

    sd_end_cmd(spi, cs).await;
    Err("CMD timeout")  // Card didn't respond in time
}

/// Initialize the SD card: reset it, check version, and activate.
/// Returns: Ok(true) for high-capacity (SDHC/SDXC), Ok(false) for standard capacity.
async fn sd_init<SPI, CS>(spi: &mut SPI, cs: &mut CS) -> Result<bool, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Step 1: Send ≥80 clock pulses with CS high to let card power up
    info!("SD init: send ≥80 clocks with CS high");
    let _ = cs.set_high();
    for _ in 0..20 {  // 20 bytes × 8 bits = 160 clock pulses
        let _ = spi_txrx(spi, 0xFF).await;
    }

    Timer::after_millis(10).await;  // Wait for card to stabilize

    // Step 2: CMD0 - Reset card to idle state
    info!("SD init: CMD0");
    let mut r1 = 0xFF;
    for attempt in 0..10 {
        r1 = sd_send_cmd(spi, cs, 0, 0, 0x95).await?;  // CRC=0x95 is required for CMD0
        sd_end_cmd(spi, cs).await;
        info!("  CMD0 attempt {=u8}: r1 = {=u8:#04x}", attempt, r1);
        if r1 == 0x01 {  // 0x01 = "in idle state" (correct response)
            break;
        }
        // Add dummy bytes and delay between attempts
        for _ in 0..100 {
            let _ = spi_txrx(spi, 0xFF).await;
        }
        Timer::after_millis(100).await;
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
    let r1 = sd_send_cmd(spi, cs, 8, 0x0000_01AA, 0x87).await?;  // 0x1AA = test pattern, CRC=0x87
    let v2;  // Will be true for v2.0+ cards (SDHC/SDXC), false for v1.x

    if r1 == 0x01 {
        // Card supports CMD8 - it's a v2.0+ card
        let mut r7 = [0u8; 4];  // CMD8 returns R7 response (4 additional bytes)
        for b in r7.iter_mut() {
            *b = spi_txrx(spi, 0xFF).await;
        }
        sd_end_cmd(spi, cs).await;

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
        sd_end_cmd(spi, cs).await;
        v2 = false;
    } else {
        sd_end_cmd(spi, cs).await;
        return Err("CMD8 unexpected R1");
    }

    // Step 4: ACMD41 - Activate card and wait for initialization to complete
    // ACMD41 = CMD55 followed by CMD41 (all ACMDs require CMD55 first)
    info!("SD init: ACMD41 loop");
    for _ in 0..1000 {
        // Send CMD55 (next command is an application-specific command)
        let r1 = sd_send_cmd(spi, cs, 55, 0, 0x01).await?;
        sd_end_cmd(spi, cs).await;
        if r1 > 0x01 {
            return Err("CMD55 failed");
        }

        // Send CMD41 with HCS bit (tells card we support high capacity)
        let arg = if v2 { 1u32 << 30 } else { 0 };  // Bit 30 = HCS (Host Capacity Support)
        let r1 = sd_send_cmd(spi, cs, 41, arg, 0x01).await?;
        sd_end_cmd(spi, cs).await;

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
        let r1 = sd_send_cmd(spi, cs, 58, 0, 0x01).await?;
        if r1 != 0x00 {
            sd_end_cmd(spi, cs).await;
            return Err("CMD58 failed");
        }

        let mut ocr = [0u8; 4];  // Read 32-bit OCR register
        for b in ocr.iter_mut() {
            *b = spi_txrx(spi, 0xFF).await;
        }
        sd_end_cmd(spi, cs).await;

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
async fn sd_read_block<SPI, CS>(
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
    let r1 = sd_send_cmd(spi, cs, 17, addr, 0x01).await?;
    if r1 != 0x00 {
        sd_end_cmd(spi, cs).await;
        return Err("CMD17 bad R1");
    }

    // Wait for data token (0xFE) - card sends this when data is ready
    for _ in 0..10_000 {
        let token = spi_txrx(spi, 0xFF).await;
        if token == 0xFE {  // 0xFE = start of data block
            // Read 512 bytes of data
            for i in 0..512 {
                buf[i] = spi_txrx(spi, 0xFF).await;
            }
            // Read 2-byte CRC (we ignore it in SPI mode, but must read it)
            let _ = spi_txrx(spi, 0xFF).await;
            let _ = spi_txrx(spi, 0xFF).await;

            sd_end_cmd(spi, cs).await;
            return Ok(());
        }
    }

    sd_end_cmd(spi, cs).await;
    Err("data token timeout")
}

/// Write a 512-byte block to the SD card.
/// This is the fundamental write operation - all file writes use this.
async fn sd_write_block<SPI, CS>(
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
    let r1 = sd_send_cmd(spi, cs, 24, addr, 0x01).await?;
    if r1 != 0x00 {
        sd_end_cmd(spi, cs).await;
        return Err("CMD24 bad R1");
    }

    // Send start token (0xFE) - tells card "data is coming now"
    let _ = spi_txrx(spi, 0xFE).await;

    // Send 512 bytes of data
    for i in 0..512 {
        let _ = spi_txrx(spi, buf[i]).await;
    }

    // Send dummy CRC (2 bytes) - not checked in SPI mode, but required by protocol
    let _ = spi_txrx(spi, 0xFF).await;
    let _ = spi_txrx(spi, 0xFF).await;

    // Wait for data response token from card
    let response = spi_txrx(spi, 0xFF).await;
    // Bits 0-4 of response: xxx0101 = data accepted
    if (response & 0x1F) != 0x05 {
        sd_end_cmd(spi, cs).await;
        return Err("Write data rejected");
    }

    // Wait for card to finish writing (card holds DO low while busy)
    for _ in 0..100_000 {
        let status = spi_txrx(spi, 0xFF).await;
        if status == 0xFF {  // 0xFF = card is no longer busy
            sd_end_cmd(spi, cs).await;
            return Ok(());
        }
    }

    sd_end_cmd(spi, cs).await;
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
async fn fat32_write_file<SPI, CS>(
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
    fat32_write_file_in_dir(spi, cs, fat_info, fat_info.root_dir_cluster, filename, data, high_capacity).await
}

/// Find a file in a directory by name and return its directory entry.
/// This searches only the first sector of the directory (simple implementation).
/// A full implementation would scan all sectors/clusters of the directory.
async fn fat32_find_file<SPI, CS>(
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
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity).await?;
    
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
async fn fat32_create_directory<SPI, CS>(
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
    let dir_cluster = fat_info.find_free_cluster(spi, cs, 2, high_capacity).await?;
    info!("  Allocated cluster {=u32} for directory", dir_cluster);
    
    // Mark cluster as end-of-chain in FAT
    fat_info.write_fat_entry(spi, cs, dir_cluster, 0x0FFF_FFFF, high_capacity).await?;
    
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
    sd_write_block(spi, cs, dir_lba, &dir_buf, high_capacity).await?;
    
    // Add directory entry to parent directory
    let mut dir_entry = DirEntry::new(dirname, 0x10)?;  // 0x10 = directory attribute
    dir_entry.start_cluster = dir_cluster;
    dir_entry.size = 0;  // Directories have 0 size in FAT32
    
    fat32_add_dir_entry(spi, cs, fat_info, parent_cluster, &dir_entry, high_capacity).await?;
    
    info!("Directory created successfully");
    Ok(dir_cluster)
}

/// Add a directory entry to any directory (not just root)
async fn fat32_add_dir_entry<SPI, CS>(
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
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity).await?;
    
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
            sd_write_block(spi, cs, dir_lba, &buf, high_capacity).await?;
            info!("Directory entry added at offset {=usize}", offset);
            Ok(())
        }
        None => Err("No free directory entries")
    }
}

/// List all entries in a directory
async fn fat32_list_directory<SPI, CS>(
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
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity).await?;
    
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
async fn fat32_navigate_path<'a, SPI, CS>(
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
        if let Some(entry) = fat32_find_file(spi, cs, fat_info, current_cluster, component, high_capacity).await? {
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
async fn fat32_write_file_at_path<SPI, CS>(
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
    let (parent_cluster, filename) = fat32_navigate_path(spi, cs, fat_info, path, high_capacity).await?;
    
    let filename = filename.ok_or("Path is a directory, not a file")?;
    
    // Write the file in the parent directory
    fat32_write_file_in_dir(spi, cs, fat_info, parent_cluster, filename, data, high_capacity).await?;
    
    Ok(())
}

/// Write a file to a specific directory cluster.
/// This handles:
/// - Allocating clusters for the file data
/// - Writing data to those clusters
/// - Creating a directory entry
/// - Updating the FAT to link clusters together
async fn fat32_write_file_in_dir<SPI, CS>(
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
        let first_cluster = fat_info.find_free_cluster(spi, cs, 2, high_capacity).await?;
        dir_entry.start_cluster = first_cluster;
        info!("  First cluster: {=u32}", first_cluster);

        let mut current_cluster = first_cluster;
        let mut bytes_written = 0;

        // Write data cluster by cluster
        for cluster_idx in 0..clusters_needed {
            // Allocate next cluster if needed (or mark end-of-chain for last cluster)
            let next_cluster = if cluster_idx + 1 < clusters_needed {
                fat_info.find_free_cluster(spi, cs, current_cluster + 1, high_capacity).await?
            } else {
                0x0FFF_FFFF // End of chain marker
            };

            // Update FAT to link current cluster to next (or mark end-of-chain)
            fat_info.write_fat_entry(spi, cs, current_cluster, next_cluster, high_capacity).await?;

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
                sd_write_block(spi, cs, cluster_lba + sector_idx as u32, &sector_buf, high_capacity).await?;
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
    fat32_add_dir_entry(spi, cs, fat_info, dir_cluster, &dir_entry, high_capacity).await?;

    info!("File written successfully!");
    Ok(())
}

/// Read a file from a specific path
async fn fat32_read_file_at_path<SPI, CS>(
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
    let (parent_cluster, filename) = fat32_navigate_path(spi, cs, fat_info, path, high_capacity).await?;
    
    let filename = filename.ok_or("Path is a directory, not a file")?;
    
    // Find the file
    let entry = fat32_find_file(spi, cs, fat_info, parent_cluster, filename, high_capacity).await?
        .ok_or("File not found")?;
    
    // Make sure it's a file, not a directory
    if (entry.attr & 0x10) != 0 {
        return Err("Path is a directory, not a file");
    }
    
    // Read the file
    fat32_read_file_complete(spi, cs, fat_info, entry.start_cluster, entry.size, buffer, high_capacity).await
}

/// Read a complete file by following the FAT chain
async fn fat32_read_file_complete<SPI, CS>(
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
            sd_read_block(spi, cs, cluster_lba + sector_idx as u32, &mut sector_buf, high_capacity).await?;
            
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
        let next_cluster = fat_info.read_fat_entry(spi, cs, current_cluster, high_capacity).await?;
        
        // Check for end of chain
        if next_cluster >= 0x0FFF_FFF8 {
            return Ok(bytes_read);
        }
        
        current_cluster = next_cluster;
    }
}

/// Delete a file by freeing its clusters and marking directory entry as deleted
async fn fat32_delete_file<SPI, CS>(
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
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity).await?;
    
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
            let next_cluster = fat_info.read_fat_entry(spi, cs, current_cluster, high_capacity).await?;
            
            // Mark cluster as free (0x00000000)
            fat_info.write_fat_entry(spi, cs, current_cluster, 0x0000_0000, high_capacity).await?;
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
    sd_write_block(spi, cs, dir_lba, &buf, high_capacity).await?;
    
    info!("File deleted successfully");
    Ok(())
}

/// Delete a file at a specific path
async fn fat32_delete_file_at_path<SPI, CS>(
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
    let (parent_cluster, filename) = fat32_navigate_path(spi, cs, fat_info, path, high_capacity).await?;
    let filename = filename.ok_or("Path is root directory, not a file")?;
    
    // Delete the file
    fat32_delete_file(spi, cs, fat_info, parent_cluster, filename, high_capacity).await
}

/// Delete an empty directory
async fn fat32_delete_directory<SPI, CS>(
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
    let dir_entry = fat32_find_file(spi, cs, fat_info, parent_cluster, dirname, high_capacity).await?
        .ok_or("Directory not found")?;
    
    // Verify it's a directory
    if (dir_entry.attr & 0x10) == 0 {
        return Err("Not a directory");
    }
    
    // Check if directory is empty (only . and .. entries)
    let dir_lba = fat_info.cluster_to_lba(dir_entry.start_cluster);
    let mut buf = [0u8; 512];
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity).await?;
    
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
    fat_info.write_fat_entry(spi, cs, dir_entry.start_cluster, 0x0000_0000, high_capacity).await?;
    
    // Remove directory entry from parent
    let parent_lba = fat_info.cluster_to_lba(parent_cluster);
    sd_read_block(spi, cs, parent_lba, &mut buf, high_capacity).await?;
    
    // Find and mark the directory entry as deleted
    let search_entry = DirEntry::new(dirname, 0x10)?;
    for entry_idx in 0..16 {
        let offset = entry_idx * 32;
        if let Some(entry) = DirEntry::parse(&buf[offset..offset + 32]) {
            if entry.name == search_entry.name {
                buf[offset] = 0xE5; // Mark as deleted
                sd_write_block(spi, cs, parent_lba, &buf, high_capacity).await?;
                info!("Directory deleted successfully");
                return Ok(());
            }
        }
    }
    
    Err("Directory entry not found in parent")
}

/// Delete a directory at a specific path (must be empty)
async fn fat32_delete_directory_at_path<SPI, CS>(
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
    let (parent_cluster, dirname) = fat32_navigate_path(spi, cs, fat_info, path, high_capacity).await?;
    let dirname = dirname.ok_or("Cannot delete root directory")?;
    
    // Delete the directory
    fat32_delete_directory(spi, cs, fat_info, parent_cluster, dirname, high_capacity).await
}

/// Rename a file in the same directory
async fn fat32_rename_file<SPI, CS>(
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
    if let Some(_) = fat32_find_file(spi, cs, fat_info, dir_cluster, new_name, high_capacity).await? {
        return Err("File with new name already exists");
    }
    
    // Find the file with old name
    let search_entry = DirEntry::new(old_name, 0)?;
    let dir_lba = fat_info.cluster_to_lba(dir_cluster);
    let mut buf = [0u8; 512];
    sd_read_block(spi, cs, dir_lba, &mut buf, high_capacity).await?;
    
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
                sd_write_block(spi, cs, dir_lba, &buf, high_capacity).await?;
                
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
async fn fat32_rename_file_at_path<SPI, CS>(
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
    let (parent_cluster, old_filename) = fat32_navigate_path(spi, cs, fat_info, old_path, high_capacity).await?;
    let old_filename = old_filename.ok_or("Path is root directory, not a file")?;
    
    // Rename the file in its current directory
    fat32_rename_file(spi, cs, fat_info, parent_cluster, old_filename, new_name, high_capacity).await
}

/// Move a file from one directory to another
async fn fat32_move_file<SPI, CS>(
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
    let (src_parent_cluster, src_filename) = fat32_navigate_path(spi, cs, fat_info, src_path, high_capacity).await?;
    let src_filename = src_filename.ok_or("Source path is a directory, not a file")?;
    
    // Navigate to destination parent and get new filename
    let (dest_parent_cluster, dest_filename) = fat32_navigate_path(spi, cs, fat_info, dest_path, high_capacity).await?;
    let dest_filename = dest_filename.ok_or("Destination path is a directory, not a file")?;
    
    // Find the source file entry
    let src_entry = fat32_find_file(spi, cs, fat_info, src_parent_cluster, src_filename, high_capacity).await?
        .ok_or("Source file not found")?;
    
    // Check if destination file already exists
    if let Some(_) = fat32_find_file(spi, cs, fat_info, dest_parent_cluster, dest_filename, high_capacity).await? {
        return Err("Destination file already exists");
    }
    
    // Create new directory entry in destination with new name
    let mut new_entry = DirEntry::new(dest_filename, src_entry.attr)?;
    new_entry.start_cluster = src_entry.start_cluster;
    new_entry.size = src_entry.size;
    
    // Add entry to destination directory
    fat32_add_dir_entry(spi, cs, fat_info, dest_parent_cluster, &new_entry, high_capacity).await?;
    
    // Remove entry from source directory (mark as deleted)
    let src_lba = fat_info.cluster_to_lba(src_parent_cluster);
    let mut buf = [0u8; 512];
    sd_read_block(spi, cs, src_lba, &mut buf, high_capacity).await?;
    
    let search_entry = DirEntry::new(src_filename, 0)?;
    for entry_idx in 0..16 {
        let offset = entry_idx * 32;
        if let Some(entry) = DirEntry::parse(&buf[offset..offset + 32]) {
            if entry.name == search_entry.name {
                buf[offset] = 0xE5; // Mark as deleted
                sd_write_block(spi, cs, src_lba, &buf, high_capacity).await?;
                info!("File moved successfully");
                return Ok(());
            }
        }
    }
    
    Err("Failed to remove source entry")
}

/// Verify that a file or directory exists at the given path
async fn fat32_verify_exists<SPI, CS>(
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
    match fat32_navigate_path(spi, cs, fat_info, path, high_capacity).await {
        Ok((parent_cluster, Some(filename))) => {
            // It's a file or directory - check if it exists
            match fat32_find_file(spi, cs, fat_info, parent_cluster, filename, high_capacity).await? {
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

/// Main entry point for Embassy executor
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("sd_test: Advanced Filesystem Test with Embassy");

    // ========================================================================
    // EMBASSY HARDWARE INITIALIZATION
    // ========================================================================
    
    // Initialize Embassy peripherals (this replaces HAL initialization)
    let p = embassy_rp::init(Default::default());

    // ========================================================================
    // GPIO AND SPI SETUP FOR SD CARD (Embassy version)
    // ========================================================================
    
    // Configure CS pin as output (manual control)
    let mut cs = Output::new(p.PIN_17, Level::High);  // Start with CS high (deselected)

    // Configure SPI with Embassy
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = 400_000;  // 400 kHz for SD card initialization
    spi_config.phase = Phase::CaptureOnFirstTransition;
    spi_config.polarity = Polarity::IdleLow;

    // Initialize SPI0 with Embassy
    let mut spi = Spi::new(
        p.SPI0,
        p.PIN_18,  // CLK
        p.PIN_19,  // MOSI
        p.PIN_16,  // MISO
        p.DMA_CH0,
        p.DMA_CH1,
        spi_config,
    );

    // ========================================================================
    // SD CARD INITIALIZATION
    // ========================================================================
    
    info!("Initializing SD card...");
    let high_capacity = match sd_init(&mut spi, &mut cs).await {
        Ok(hc) => hc,
        Err(e) => {
            error!("SD init failed: {}", e);
            loop { 
                Timer::after(Duration::from_millis(1000)).await;
            }  // Embassy async wait instead of blocking
        }
    };

    // ========================================================================
    // FILESYSTEM INITIALIZATION
    // ========================================================================
    
    // Read boot sector (sector 0) to get filesystem metadata
    let mut buf = [0u8; 512];
    sd_read_block(&mut spi, &mut cs, 0, &mut buf, high_capacity).await.ok();
    
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
    // TEST 1: Create a directory structure (minimal for shell demo)
    // ========================================================================
    info!("\n=== TEST 1: Creating Basic Directory Structure ===");
    
    match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "DOCS", high_capacity).await {
        Ok(_) => info!("✓ Created /DOCS directory"),
        Err(e) => error!("✗ Failed to create directory: {}", e),
    }

    match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "MUSIC", high_capacity).await {
        Ok(_) => info!("✓ Created /MUSIC directory"),
        Err(e) => error!("✗ Failed: {}", e),
    }

    // ========================================================================
    // TEST 2: Write minimal files for shell demo
    // ========================================================================
    info!("\n=== TEST 2: Writing Basic Files ===");
    
    let readme_data = b"Pico OS Shell Demo!\nUse 'help' for commands.";
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/README.TXT", readme_data, high_capacity).await {
        Ok(()) => info!("✓ Wrote /README.TXT ({} bytes)", readme_data.len()),
        Err(e) => error!("✗ Failed: {}", e),
    }

    let doc_data = b"Simple test file in DOCS.";
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/DOCS/HELLO.TXT", doc_data, high_capacity).await {
        Ok(()) => info!("✓ Wrote /DOCS/HELLO.TXT ({} bytes)", doc_data.len()),
        Err(e) => error!("✗ Failed: {}", e),
    }

    // ========================================================================
    // YOUR CUSTOM FILES - Add your own files and directories here!
    // ========================================================================
    // Uncomment and edit the examples below to create your own files:
    
    
    info!("\n=== Creating Custom Files ===");
    
    // Example 1: Create a simple text file
    let my_data = b"Hello! This is my custom file.";
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/MYFILE.TXT", my_data, high_capacity).await {
        Ok(()) => info!("✓ Created /MYFILE.TXT"),
        Err(e) => error!("✗ Failed: {}", e),
    }
    
    // Example 2: Create a new directory
    match fat32_create_directory(&mut spi, &mut cs, &fat_info, fat_info.root_dir_cluster, "PHOTOS", high_capacity).await {
        Ok(_) => info!("✓ Created /PHOTOS directory"),
        Err(e) => error!("✗ Failed: {}", e),
    }
    
    // Example 3: Create a file in a subdirectory
    let photo_info = b"Photo Information\nDate: 2025-11-29\nCamera: Pico";
    match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, "/PHOTOS/INFO.TXT", photo_info, high_capacity).await {
        Ok(()) => info!("✓ Created /PHOTOS/INFO.TXT"),
        Err(e) => error!("✗ Failed: {}", e),
    }
    

    // Skip heavy tests to save memory for shell demo
    info!("\n=== Basic filesystem ready ===");

    // Skip all heavy tests to save memory for shell demo
    
    // ========================================================================
    // INTERACTIVE SHELL DEMO
    // ========================================================================
    info!("\n=== Starting Interactive Shell Demo ===");
    
    // Run the shell demo
    if let Err(e) = run_shell_demo(&mut spi, &mut cs, &fat_info, high_capacity).await {
        if e != "exit" {
            error!("Shell error: {}", e);
        }
    }

    // After shell demo, keep the device alive
    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

// ============================================================================
// COMMAND LINE INTERFACE
// ============================================================================

/// Represents the current shell state
struct ShellState {
    current_path: heapless::String<128>,  // Current working directory (reduced size)
    current_cluster: u32,  // Current directory cluster
}

impl ShellState {
    fn new(root_cluster: u32) -> Self {
        Self {
            current_path: heapless::String::new(),
            current_cluster: root_cluster,
        }
    }

    fn set_path(&mut self, path: &str) -> Result<(), &'static str> {
        self.current_path.clear();
        self.current_path.push_str(path).map_err(|_| "Path too long")?;
        Ok(())
    }
}

/// Parse and execute a command
async fn execute_command<SPI, CS>(
    command: &str,
    shell_state: &mut ShellState,
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Split command into parts
    let parts: heapless::Vec<&str, 8> = command.trim().split_whitespace().collect();
    if parts.is_empty() {
        return Ok(());
    }

    let cmd = parts[0];
    match cmd {
        "ls" | "dir" => {
            info!("Contents of {}:", if shell_state.current_path.is_empty() { "/" } else { shell_state.current_path.as_str() });
            fat32_list_directory(spi, cs, fat_info, shell_state.current_cluster, high_capacity).await?;
        }
        
        "pwd" => {
            if shell_state.current_path.is_empty() {
                info!("Current directory: /");
            } else {
                info!("Current directory: {}", shell_state.current_path.as_str());
            }
        }
        
        "cd" => {
            if parts.len() < 2 {
                info!("Usage: cd <directory>");
                return Ok(());
            }
            
            let target = parts[1];
            if target == ".." {
                // Go to parent directory
                if !shell_state.current_path.is_empty() {
                    // Remove last directory from path
                    if let Some(last_slash) = shell_state.current_path.rfind('/') {
                        shell_state.current_path.truncate(last_slash);
                        if shell_state.current_path.is_empty() {
                            shell_state.current_cluster = fat_info.root_dir_cluster;
                        } else {
                            // Find the cluster for the new current directory
                            match fat32_find_directory_by_path(spi, cs, fat_info, shell_state.current_path.as_str(), high_capacity).await {
                                Ok(cluster) => shell_state.current_cluster = cluster,
                                Err(_) => {
                                    shell_state.current_cluster = fat_info.root_dir_cluster;
                                    shell_state.current_path.clear();
                                }
                            }
                        }
                    } else {
                        shell_state.current_cluster = fat_info.root_dir_cluster;
                        shell_state.current_path.clear();
                    }
                }
            } else if target == "/" {
                // Go to root
                shell_state.current_cluster = fat_info.root_dir_cluster;
                shell_state.current_path.clear();
            } else {
                // Find the directory in current location
                match fat32_find_file(spi, cs, fat_info, shell_state.current_cluster, target, high_capacity).await {
                    Ok(Some(entry)) => {
                        if entry.attr & 0x10 != 0 {  // It's a directory
                            shell_state.current_cluster = entry.start_cluster;
                            if !shell_state.current_path.is_empty() {
                                shell_state.current_path.push('/').map_err(|_| "Path too long")?;
                            }
                            shell_state.current_path.push_str(target).map_err(|_| "Path too long")?;
                        } else {
                            info!("'{}' is not a directory", target);
                        }
                    }
                    Ok(None) => info!("Directory '{}' not found", target),
                    Err(e) => info!("Error: {}", e),
                }
            }
        }
        
        "cat" | "type" => {
            if parts.len() < 2 {
                info!("Usage: cat <filename>");
                return Ok(());
            }
            
            let filename = parts[1];
            let mut read_buf = [0u8; 512];  // Reduced buffer size
            
            match fat32_find_file(spi, cs, fat_info, shell_state.current_cluster, filename, high_capacity).await {
                Ok(Some(entry)) => {
                    if entry.attr & 0x10 == 0 {  // It's a file
                        match fat32_read_file_content(spi, cs, fat_info, entry, &mut read_buf, high_capacity).await {
                            Ok(bytes_read) => {
                                info!("=== {} ({} bytes) ===", filename, bytes_read);
                                // Print file content (safely handle non-UTF8)
                                for chunk in read_buf[..bytes_read].chunks(64) {
                                    info!("{=[u8]:a}", chunk);
                                }
                                info!("=== End of {} ===", filename);
                            }
                            Err(e) => info!("Error reading file: {}", e),
                        }
                    } else {
                        info!("'{}' is a directory", filename);
                    }
                }
                Ok(None) => info!("File '{}' not found", filename),
                Err(e) => info!("Error: {}", e),
            }
        }
        
        "mkdir" => {
            if parts.len() < 2 {
                info!("Usage: mkdir <directory_name>");
                return Ok(());
            }
            
            let dirname = parts[1];
            match fat32_create_directory(spi, cs, fat_info, shell_state.current_cluster, dirname, high_capacity).await {
                Ok(_) => info!("Created directory '{}'", dirname),
                Err(e) => info!("Failed to create directory: {}", e),
            }
        }
        
        "rm" | "del" => {
            if parts.len() < 2 {
                info!("Usage: rm <filename>");
                return Ok(());
            }
            
            let filename = parts[1];
            // Build full path for deletion
            let mut full_path = heapless::String::<256>::new();
            if !shell_state.current_path.is_empty() {
                full_path.push_str(&shell_state.current_path).map_err(|_| "Path too long")?;
                full_path.push('/').map_err(|_| "Path too long")?;
            } else {
                full_path.push('/').map_err(|_| "Path too long")?;
            }
            full_path.push_str(filename).map_err(|_| "Path too long")?;
            
            match fat32_delete_file_at_path(spi, cs, fat_info, full_path.as_str(), high_capacity).await {
                Ok(()) => info!("Deleted '{}'", filename),
                Err(e) => info!("Failed to delete: {}", e),
            }
        }
        
        "rmdir" => {
            if parts.len() < 2 {
                info!("Usage: rmdir <directory_name>");
                return Ok(());
            }
            
            let dirname = parts[1];
            // Build full path for deletion
            let mut full_path = heapless::String::<256>::new();
            if !shell_state.current_path.is_empty() {
                full_path.push_str(&shell_state.current_path).map_err(|_| "Path too long")?;
                full_path.push('/').map_err(|_| "Path too long")?;
            } else {
                full_path.push('/').map_err(|_| "Path too long")?;
            }
            full_path.push_str(dirname).map_err(|_| "Path too long")?;
            
            match fat32_delete_directory_at_path(spi, cs, fat_info, full_path.as_str(), high_capacity).await {
                Ok(()) => info!("Deleted directory '{}'", dirname),
                Err(e) => info!("Failed to delete directory: {}", e),
            }
        }
        
        "touch" => {
            if parts.len() < 2 {
                info!("Usage: touch <filename>");
                return Ok(());
            }
            
            let filename = parts[1];
            let empty_data = b"";
            
            // Build full path for creation
            let mut full_path = heapless::String::<256>::new();
            if !shell_state.current_path.is_empty() {
                full_path.push_str(&shell_state.current_path).map_err(|_| "Path too long")?;
                full_path.push('/').map_err(|_| "Path too long")?;
            } else {
                full_path.push('/').map_err(|_| "Path too long")?;
            }
            full_path.push_str(filename).map_err(|_| "Path too long")?;
            
            match fat32_write_file_at_path(spi, cs, fat_info, full_path.as_str(), empty_data, high_capacity).await {
                Ok(()) => info!("Created empty file '{}'", filename),
                Err(e) => info!("Failed to create file: {}", e),
            }
        }
        
        "help" => {
            info!("Available commands:");
            info!("  ls, dir           - List directory contents");
            info!("  pwd              - Show current directory");
            info!("  cd <dir>         - Change directory");
            info!("  cat <file>       - Display file contents");
            info!("  mkdir <dir>      - Create directory");
            info!("  rm <file>        - Delete file");
            info!("  rmdir <dir>      - Delete directory");
            info!("  touch <file>     - Create empty file");
            info!("  help             - Show this help");
            info!("  exit             - Exit shell (run tests)");
        }
        
        "exit" => {
            info!("Exiting shell...");
            return Err("exit");  // Use error to signal exit
        }
        
        _ => {
            info!("Unknown command '{}'. Type 'help' for available commands.", cmd);
        }
    }
    
    Ok(())
}

/// Simple command input simulation (since we don't have keyboard input)
/// This demonstrates how the shell would work with user input
async fn run_shell_demo<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    let mut shell_state = ShellState::new(fat_info.root_dir_cluster);
    
    info!("\n========================================");
    info!("Welcome to Pico OS File System Shell!");
    info!("Type 'help' for available commands.");
    info!("========================================\n");
    
    // Simulate some user commands - shorter demo to reduce memory usage
    let demo_commands = [
        "help",
        "pwd",
        "ls", 
        "cd DOCS",
        "pwd",
        "ls",
        "cd ..",
        "pwd",
        "cd MUSIC", 
        "ls",
        "cd /",
        "ls",
    ];
    
    for command in demo_commands.iter() {
        info!("\n$ {}", command);
        Timer::after(Duration::from_millis(300)).await;  // Shorter pause
        
        match execute_command(command, &mut shell_state, spi, cs, fat_info, high_capacity).await {
            Ok(()) => {},
            Err("exit") => break,
            Err(e) => info!("Error: {}", e),
        }
    }
    
    info!("\nShell demo completed!");
    Ok(())
}

/// Helper function to find a directory by full path
async fn fat32_find_directory_by_path<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    path: &str,
    high_capacity: bool,
) -> Result<u32, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    if path.is_empty() || path == "/" {
        return Ok(fat_info.root_dir_cluster);
    }
    
    let mut current_cluster = fat_info.root_dir_cluster;
    
    // Split path and navigate through each directory
    for dir_name in path.split('/').filter(|s| !s.is_empty()) {
        match fat32_find_file(spi, cs, fat_info, current_cluster, dir_name, high_capacity).await {
            Ok(Some(entry)) => {
                if entry.attr & 0x10 != 0 {  // Is directory
                    current_cluster = entry.start_cluster;
                } else {
                    return Err("Not a directory");
                }
            }
            Ok(None) => return Err("Directory not found"),
            Err(e) => return Err(e),
        }
    }
    
    Ok(current_cluster)
}

/// Read file content directly from a directory entry
async fn fat32_read_file_content<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    fat_info: &Fat32Info,
    entry: DirEntry,
    buf: &mut [u8],
    high_capacity: bool,
) -> Result<usize, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    let mut current_cluster = entry.start_cluster;
    let mut bytes_read = 0;
    let mut buf_offset = 0;
    
    // Read cluster chain
    loop {
        if current_cluster < 2 || current_cluster >= 0x0FFF_FFF8 {
            break;  // Invalid or end-of-chain
        }
        
        // Calculate cluster size in bytes
        let cluster_size = fat_info.sectors_per_cluster as usize * fat_info.bytes_per_sector as usize;
        let remaining_buf = buf.len() - buf_offset;
        let remaining_file = entry.size as usize - bytes_read;
        let to_read = cluster_size.min(remaining_buf).min(remaining_file);
        
        if to_read == 0 {
            break;
        }
        
        // Read the cluster
        let cluster_lba = fat_info.cluster_to_lba(current_cluster);
        let mut cluster_buf = [0u8; 512];
        
        for sector_offset in 0..fat_info.sectors_per_cluster {
            let sector_lba = cluster_lba + sector_offset as u32;
            sd_read_block(spi, cs, sector_lba, &mut cluster_buf, high_capacity).await?;
            
            let sector_to_read = (512).min(to_read - (sector_offset as usize * 512)).min(remaining_file - bytes_read);
            if sector_to_read > 0 {
                buf[buf_offset..buf_offset + sector_to_read].copy_from_slice(&cluster_buf[..sector_to_read]);
                buf_offset += sector_to_read;
                bytes_read += sector_to_read;
            }
        }
        
        if bytes_read >= entry.size as usize {
            break;
        }
        
        // Get next cluster from FAT
        current_cluster = fat_info.read_fat_entry(spi, cs, current_cluster, high_capacity).await?;
    }
    
    Ok(bytes_read)
}
