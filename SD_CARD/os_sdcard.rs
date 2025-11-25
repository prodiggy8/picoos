#![no_std]
#![no_main]

// Board support
use rp_pico as bsp;
use bsp::entry;

// Logging + panic
use defmt::*;
use defmt_rtt as _;
use panic_probe as _;

// HAL imports
use bsp::hal::{
    clocks::{init_clocks_and_plls, Clock},
    pac,
    sio::Sio,
    spi::Spi,
    watchdog::Watchdog,
    gpio::FunctionSpi,
};

// Traits
use embedded_hal::blocking::spi::Transfer;
use embedded_hal::digital::v2::OutputPin;
use embedded_hal::spi::MODE_0;

// --------------------------------------------------------
// Simple SPI helper: send one byte, return what we read
// --------------------------------------------------------
fn spi_txrx<SPI>(spi: &mut SPI, byte: u8) -> u8
where
    SPI: Transfer<u8>,
{
    let mut buf = [byte];
    if spi.transfer(&mut buf).is_ok() {
        buf[0]
    } else {
        0xFF
    }
}

// --------------------------------------------------------
// SD helpers: end a command (release CS properly)
// --------------------------------------------------------
fn sd_end_cmd<SPI, CS>(spi: &mut SPI, cs: &mut CS)
where
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    let _ = spi_txrx(spi, 0xFF); // one extra clock
    cs.set_high().ok();
    let _ = spi_txrx(spi, 0xFF); // another dummy
}

// --------------------------------------------------------
// Send an SD command over SPI.
// Leaves CS *LOW* on success; caller must call sd_end_cmd().
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
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    // Select card
    cs.set_low().ok();

    // One dummy before command
    let _ = spi_txrx(spi, 0xFF);

    // Command packet: [0x40|cmd][arg(4)][crc]
    spi_txrx(spi, 0x40 | cmd);
    spi_txrx(spi, (arg >> 24) as u8);
    spi_txrx(spi, (arg >> 16) as u8);
    spi_txrx(spi, (arg >> 8) as u8);
    spi_txrx(spi, arg as u8);
    spi_txrx(spi, crc);

    // Wait for R1 response (MSB must become 0)
    for _ in 0..=8 {
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
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    info!("SD init: send 80 clocks");
    // Make sure CS is high, send >=74 clocks with MOSI high
    cs.set_high().ok();
    for _ in 0..10 {
        let _ = spi_txrx(spi, 0xFF);
    }

    // CMD0: GO_IDLE_STATE
    info!("SD init: CMD0");
    let r1 = sd_send_cmd(spi, cs, 0, 0, 0x95)?;
    sd_end_cmd(spi, cs);
    if r1 != 0x01 {
        return Err("CMD0 did not enter IDLE (r1 != 0x01)");
    }

    // CMD8: SEND_IF_COND, check SD v2 and voltage range
    info!("SD init: CMD8");
    let r1 = sd_send_cmd(spi, cs, 8, 0x0000_01AA, 0x87)?;
    let mut v2 = false;

    if r1 == 0x01 {
        // R7 response: 4 more bytes (we only check last two: 0x01, 0xAA)
        let mut r7 = [0u8; 4];
        for b in r7.iter_mut() {
            *b = spi_txrx(spi, 0xFF);
        }
        sd_end_cmd(spi, cs);

        info!("CMD8 R7: {=u8} {=u8} {=u8} {=u8}", r7[0], r7[1], r7[2], r7[3]);

        if r7[2] == 0x01 && r7[3] == 0xAA {
            v2 = true;
        } else {
            return Err("CMD8 bad echo pattern");
        }
    } else if (r1 & 0x04) != 0 {
        // Illegal command => probably SD v1.x or MMC
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

        // ACMD41 (CMD41 with HCS bit if v2)
        let arg = if v2 { 1u32 << 30 } else { 0 };
        let r1 = sd_send_cmd(spi, cs, 41, arg, 0x01)?;
        sd_end_cmd(spi, cs);

        if r1 == 0x00 {
            // Card is ready
            break;
        }

        // Tiny delay via dummy clocks
        let _ = spi_txrx(spi, 0xFF);
    }

    // CMD58: read OCR and check CCS (only meaningful for SD v2)
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

        info!("OCR: {=u8} {=u8} {=u8} {=u8}", ocr[0], ocr[1], ocr[2], ocr[3]);

        // CCS (bit 30 of OCR) -> high capacity
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
    SPI: Transfer<u8>,
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
    for _ in 0..10000 {
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
// Main
// --------------------------------------------------------
#[entry]
fn main() -> ! {
    info!("os_sdcard: start");

    // Grab peripherals
    let mut pac = pac::Peripherals::take().unwrap();
    let mut watchdog = Watchdog::new(pac.WATCHDOG);
    let sio = Sio::new(pac.SIO);

    // 12 MHz crystal on Pico
    let external_xtal_freq_hz = 12_000_000u32;

    let clocks = init_clocks_and_plls(
        external_xtal_freq_hz,
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

    // -----------------------------
    // SPI0 pins for SD card
    // SCK  -> GPIO18
    // MOSI -> GPIO19
    // MISO -> GPIO16
    // CS   -> GPIO17
    // -----------------------------
    let spi_sck = pins.gpio18.into_function::<FunctionSpi>();
    let spi_mosi = pins.gpio19.into_function::<FunctionSpi>();
    let spi_miso = pins.gpio16.into_function::<FunctionSpi>();

    // Valid pin layout: (Tx/MOSI, Rx/MISO, SCK)
    let spi_pins = (spi_mosi, spi_miso, spi_sck);

    // *** IMPORTANT FIX: explicitly specify DS = 8 ***
    let mut spi = Spi::<_, _, _, 8>::new(pac.SPI0, spi_pins).init(
        &mut pac.RESETS,
        // peripheral clock frequency
        clocks.peripheral_clock.freq(),
        // SPI baudrate: 400 kHz for SD init
        bsp::hal::fugit::HertzU32::from_raw(400_000),
        MODE_0,
    );

    // Chip-select pin for SD card
    let mut sd_cs = pins.gpio17.into_push_pull_output();
    sd_cs.set_high().ok(); // deselect

    info!("Init SD card over SPI...");
    let high_capacity = match sd_init(&mut spi, &mut sd_cs) {
        Ok(hc) => {
            info!("SD init OK. High-capacity: {=bool}", hc);
            hc
        }
        Err(e) => {
            error!("SD init FAILED: {}", e);
            loop {
                cortex_m::asm::bkpt();
            }
        }
    };

    // Try reading block 0
    let mut block0 = [0u8; 512];
    match sd_read_block(&mut spi, &mut sd_cs, 0, &mut block0, high_capacity) {
        Ok(()) => {
            info!(
                "Block0 first bytes: {=u8} {=u8} {=u8} {=u8}",
                block0[0],
                block0[1],
                block0[2],
                block0[3]
            );
        }
        Err(e) => {
            error!("Read block0 failed: {}", e);
        }
    }

    info!("Done. Looping forever.");
    loop {
        cortex_m::asm::wfi();
    }
}
