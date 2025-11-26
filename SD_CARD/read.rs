#![no_std]
#![no_main]

// Board support
use rp_pico as bsp;
use bsp::entry;

// Boot2 for RP2040 (same style as other bins)
#[link_section = ".boot2"]
#[no_mangle]
#[used]
pub static BOOT2_FIRMWARE: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

// Logging + panic
use defmt::*;
use defmt_rtt as _;
use panic_probe as _;

// HAL imports
use bsp::hal::{
    clocks::{init_clocks_and_plls, Clock},
    fugit::HertzU32,
    gpio::FunctionSpi,
    pac,
    sio::Sio,
    spi::Spi,
    watchdog::Watchdog,
};

// embedded-hal 1.0 traits
use embedded_hal::digital::OutputPin;
use embedded_hal::spi::{SpiBus, MODE_0};

// --------------------------------------------------------
// SPI helper: send one byte, read one byte
// --------------------------------------------------------
fn spi_txrx<SPI>(spi: &mut SPI, byte: u8) -> u8
where
    SPI: SpiBus<u8>,
{
    let mut buf = [byte];
    if spi.transfer_in_place(&mut buf).is_ok() {
        buf[0]
    } else {
        0xFF
    }
}

// --------------------------------------------------------
// End an SD command: give extra clocks and release CS
// --------------------------------------------------------
fn sd_end_cmd<SPI, CS>(spi: &mut SPI, cs: &mut CS)
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    let _ = spi_txrx(spi, 0xFF); // extra clock
    let _ = cs.set_high();
    let _ = spi_txrx(spi, 0xFF); // another dummy
}

// --------------------------------------------------------
// Send an SD command over SPI.
//
// Leaves CS LOW on success; caller must call sd_end_cmd().
// On timeout, this function releases CS and returns Err.
// --------------------------------------------------------
fn sd_send_cmd<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    cmd: u8,
    arg: u32,
    crc: u8,
) -> Result<u8, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Select card
    let _ = cs.set_low();

    // One dummy before command
    let _ = spi_txrx(spi, 0xFF);

    // Command packet: [0x40|cmd][arg(4)][crc]
    let _ = spi_txrx(spi, 0x40 | cmd);
    let _ = spi_txrx(spi, (arg >> 24) as u8);
    let _ = spi_txrx(spi, (arg >> 16) as u8);
    let _ = spi_txrx(spi, (arg >> 8) as u8);
    let _ = spi_txrx(spi, arg as u8);
    let _ = spi_txrx(spi, crc);

    // Wait for R1 response (MSB must become 0)
    for _ in 0..255 {
        let resp = spi_txrx(spi, 0xFF);
        if resp & 0x80 == 0 {
            return Ok(resp);
        }
    }

    // Timeout: release the card and error out
    sd_end_cmd(spi, cs);
    Err("CMD timeout")
}

// --------------------------------------------------------
// Initialize SD card in SPI mode.
// Returns Ok(is_high_capacity).
// --------------------------------------------------------
fn sd_init<SPI, CS>(spi: &mut SPI, cs: &mut CS) -> Result<bool, &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    info!("SD init: send ≥80 clocks with CS high");
    let _ = cs.set_high();
    for _ in 0..20 {
        let _ = spi_txrx(spi, 0xFF);
    }

    // small delay
    cortex_m::asm::delay(10_000);

    // CMD0: GO_IDLE_STATE
    info!("SD init: CMD0");
    let mut r1 = 0xFF;
    for attempt in 0..10 {
        r1 = sd_send_cmd(spi, cs, 0, 0, 0x95)?;
        sd_end_cmd(spi, cs);
        info!("  CMD0 attempt {=u8}: r1 = {=u8:#04x}", attempt, r1);
        if r1 == 0x01 {
            break;
        }
        for _ in 0..10 {
            let _ = spi_txrx(spi, 0xFF);
        }
    }

    if r1 != 0x01 {
        error!("CMD0 final r1 = {=u8:#04x}, expected 0x01", r1);
        return Err("CMD0 did not enter IDLE (r1 != 0x01)");
    }

    // CMD8: SEND_IF_COND
    info!("SD init: CMD8");
    let r1 = sd_send_cmd(spi, cs, 8, 0x0000_01AA, 0x87)?;
    let mut v2 = false;

    if r1 == 0x01 {
        let mut r7 = [0u8; 4];
        for b in r7.iter_mut() {
            *b = spi_txrx(spi, 0xFF);
        }
        sd_end_cmd(spi, cs);

        info!("  CMD8 R7: {=u8} {=u8} {=u8} {=u8}", r7[0], r7[1], r7[2], r7[3]);
        if r7[2] == 0x01 && r7[3] == 0xAA {
            v2 = true;
        } else {
            return Err("CMD8 bad echo pattern");
        }
    } else if (r1 & 0x04) != 0 {
        info!("CMD8 illegal -> old card (v1.x/MMC)");
        sd_end_cmd(spi, cs);
        v2 = false;
    } else {
        sd_end_cmd(spi, cs);
        return Err("CMD8 unexpected R1");
    }

    // ACMD41 loop: send CMD55 then ACMD41 until card leaves idle
    info!("SD init: ACMD41 loop");
    let mut high_capacity = false;

    for _ in 0..1000 {
        // CMD55
        let r1 = sd_send_cmd(spi, cs, 55, 0, 0x01)?;
        sd_end_cmd(spi, cs);
        if r1 > 0x01 {
            return Err("CMD55 failed");
        }

        // ACMD41
        let arg = if v2 { 1u32 << 30 } else { 0 };
        let r1 = sd_send_cmd(spi, cs, 41, arg, 0x01)?;
        sd_end_cmd(spi, cs);

        if r1 == 0x00 {
            break;
        }

        let _ = spi_txrx(spi, 0xFF);
    }

    // CMD58: read OCR
    if v2 {
        info!("SD init: CMD58");
        let r1 = sd_send_cmd(spi, cs, 58, 0, 0x01)?;
        if r1 != 0x00 {
            sd_end_cmd(spi, cs);
            return Err("CMD58 failed");
        }

        let mut ocr = [0u8; 4];
        for b in ocr.iter_mut() {
            *b = spi_txrx(spi, 0xFF);
        }
        sd_end_cmd(spi, cs);

        info!("  OCR: {=u8} {=u8} {=u8} {=u8}", ocr[0], ocr[1], ocr[2], ocr[3]);
        high_capacity = (ocr[0] & 0x40) != 0;
    } else {
        high_capacity = false;
    }

    Ok(high_capacity)
}

// --------------------------------------------------------
// Read a single 512-byte block (LBA) into `buf`
// --------------------------------------------------------
fn sd_read_block<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    lba: u32,
    buf: &mut [u8; 512],
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // Address: byte address for SDSC; block address for SDHC/SDXC
    let addr = if high_capacity { lba } else { lba * 512 };

    info!("Read block: lba={=u32}, addr={=u32}", lba, addr);

    let r1 = sd_send_cmd(spi, cs, 17, addr, 0x01)?;
    if r1 != 0x00 {
        sd_end_cmd(spi, cs);
        return Err("CMD17 bad R1");
    }

    // Wait for data token (0xFE)
    for _ in 0..10_000 {
        let token = spi_txrx(spi, 0xFF);
        if token == 0xFE {
            // Read 512 data bytes
            for i in 0..512 {
                buf[i] = spi_txrx(spi, 0xFF);
            }
            // Discard CRC
            let _ = spi_txrx(spi, 0xFF);
            let _ = spi_txrx(spi, 0xFF);

            sd_end_cmd(spi, cs);
            return Ok(());
        }
    }

    sd_end_cmd(spi, cs);
    Err("data token timeout")
}

// --------------------------------------------------------
// main: read block 100 and verify pattern from sdcard_write
// --------------------------------------------------------
#[entry]
fn main() -> ! {
    info!("sdcard_read: start");

    // Peripherals / clocks
    let mut pac = pac::Peripherals::take().unwrap();
    let mut watchdog = Watchdog::new(pac.WATCHDOG);
    let sio = Sio::new(pac.SIO);

    let clocks = init_clocks_and_plls(
        12_000_000,          // 12 MHz crystal
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let pins = bsp::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    // SPI0 pins for SD card
    //
    // SD SCK  -> GPIO18
    // SD MOSI -> GPIO19
    // SD MISO -> GPIO16
    // SD CS   -> GPIO17
    let spi_sck = pins.gpio18.into_function::<FunctionSpi>();
    let spi_mosi = pins.gpio19.into_function::<FunctionSpi>();
    let spi_miso = pins.gpio16.into_function::<FunctionSpi>();
    let spi_pins = (spi_mosi, spi_miso, spi_sck);

    let mut spi = Spi::<_, _, _, 8>::new(pac.SPI0, spi_pins).init(
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
        HertzU32::from_raw(400_000), // 400 kHz
        MODE_0,
    );

    let mut sd_cs = pins.gpio17.into_push_pull_output();
    let _ = sd_cs.set_high(); // deselect

    info!("Init SD...");
    let high_capacity = match sd_init(&mut spi, &mut sd_cs) {
        Ok(hc) => {
            info!("SD init OK; high_capacity = {=bool}", hc);
            hc
        }
        Err(e) => {
            error!("SD init failed: {}", e);
            loop {
                cortex_m::asm::bkpt();
            }
        }
    };

    // Read the same LBA that sdcard_write.rs used (100)
    let lba: u32 = 100;
    let mut buf = [0u8; 512];

    info!("Reading block {=u32}...", lba);
    match sd_read_block(&mut spi, &mut sd_cs, lba, &mut buf, high_capacity) {
        Ok(()) => {
            info!(
                "First 4 bytes: {=u8:#04x} {=u8:#04x} {=u8:#04x} {=u8:#04x}",
                buf[0], buf[1], buf[2], buf[3]
            );

            // Check pattern: 0xDE 0xAD 0xBE 0xEF + (i as u8)
            let mut ok = true;

            if buf[0] != 0xDE || buf[1] != 0xAD || buf[2] != 0xBE || buf[3] != 0xEF {
                error!("Signature mismatch in first 4 bytes");
                ok = false;
            }

            for i in 4..512 {
                let expected = (i & 0xFF) as u8;
                if buf[i] != expected {
                    error!(
                        "Mismatch at index {=u32}: got {=u8:#04x}, expected {=u8:#04x}",
                        i as u32,
                        buf[i],
                        expected
                    );
                    ok = false;
                    break;
                }
            }

            if ok {
                info!("Block {=u32} matches expected pattern! :)", lba);
            } else {
                error!("Block {=u32} does NOT match pattern :(", lba);
            }
        }
        Err(e) => {
            error!("Read failed: {}", e);
        }
    }

    info!("sdcard_read: done, looping");
    loop {
        cortex_m::asm::wfi();
    }
}
