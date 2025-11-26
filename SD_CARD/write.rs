#![no_std]
#![no_main]

// Board support
use rp_pico as bsp;
use bsp::entry;

// Boot2 for RP2040
#[link_section = ".boot2"]
#[no_mangle]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_GENERIC_03H;

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
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    info!("SD init: send 80+ clocks with CS high");
    cs.set_high().ok();
    for _ in 0..20 {
        let _ = spi_txrx(spi, 0xFF);
    }

    cortex_m::asm::delay(10000);

    // CMD0: GO_IDLE_STATE
    info!("SD init: CMD0");
    let mut r1 = 0xFF;
    for attempt in 0..10 {
        r1 = sd_send_cmd(spi, cs, 0, 0, 0x95)?;
        sd_end_cmd(spi, cs);
        info!("CMD0 attempt {=u32}: r1 = {=u8:#04x}", attempt, r1);
        if r1 == 0x01 {
            break;
        }
        for _ in 0..10 {
            let _ = spi_txrx(spi, 0xFF);
        }
    }
    if r1 != 0x01 {
        return Err("CMD0 did not enter IDLE");
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
        info!("CMD8 R7: {=u8} {=u8} {=u8} {=u8}", r7[0], r7[1], r7[2], r7[3]);
        if r7[2] == 0x01 && r7[3] == 0xAA {
            v2 = true;
        } else {
            return Err("CMD8 bad echo");
        }
    } else if (r1 & 0x04) != 0 {
        info!("CMD8 illegal -> v1.x/MMC");
        sd_end_cmd(spi, cs);
        v2 = false;
    } else {
        sd_end_cmd(spi, cs);
        return Err("CMD8 unexpected R1");
    }

    // ACMD41 loop
    info!("SD init: ACMD41 loop");
    for _ in 0..1000 {
        let r1 = sd_send_cmd(spi, cs, 55, 0, 0x01)?;
        sd_end_cmd(spi, cs);
        if r1 > 0x01 {
            return Err("CMD55 failed");
        }

        let arg = if v2 { 1u32 << 30 } else { 0 };
        let r1 = sd_send_cmd(spi, cs, 41, arg, 0x01)?;
        sd_end_cmd(spi, cs);
        if r1 == 0x00 {
            break;
        }
        let _ = spi_txrx(spi, 0xFF);
    }

    // CMD58: read OCR
    let mut high_capacity = false;
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
        high_capacity = (ocr[0] & 0x40) != 0;
    }

    Ok(high_capacity)
}

// --------------------------------------------------------
// Write a single 512-byte block (LBA) from `buf`
// --------------------------------------------------------
fn sd_write_block<SPI, CS>(
    spi: &mut SPI,
    cs: &mut CS,
    lba: u32,
    buf: &[u8; 512],
    high_capacity: bool,
) -> Result<(), &'static str>
where
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    let addr = if high_capacity { lba } else { lba * 512 };
    info!("Write block: lba={=u32}, addr={=u32}", lba, addr);

    // CMD24: WRITE_SINGLE_BLOCK
    let r1 = sd_send_cmd(spi, cs, 24, addr, 0x01)?;
    if r1 != 0x00 {
        sd_end_cmd(spi, cs);
        return Err("CMD24 bad R1");
    }

    // Start token
    spi_txrx(spi, 0xFE);

    // Data
    for i in 0..512 {
        spi_txrx(spi, buf[i]);
    }

    // Dummy CRC
    spi_txrx(spi, 0xFF);
    spi_txrx(spi, 0xFF);

    // Data response
    let resp = spi_txrx(spi, 0xFF);
    if (resp & 0x1F) != 0x05 {
        sd_end_cmd(spi, cs);
        return Err("Write rejected");
    }

    // Wait not busy
    for _ in 0..100000 {
        let busy = spi_txrx(spi, 0xFF);
        if busy == 0xFF {
            sd_end_cmd(spi, cs);
            return Ok(());
        }
    }

    sd_end_cmd(spi, cs);
    Err("Write timeout")
}

// --------------------------------------------------------
// main: write test pattern to block 100
// --------------------------------------------------------
#[entry]
fn main() -> ! {
    info!("sd_write: start");

    let mut pac = pac::Peripherals::take().unwrap();
    let mut watchdog = Watchdog::new(pac.WATCHDOG);
    let sio = Sio::new(pac.SIO);

    let clocks = init_clocks_and_plls(
        12_000_000,
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

    // SPI0: SCK=GP18, MOSI=GP19, MISO=GP16
    let spi_sck = pins.gpio18.into_function::<FunctionSpi>();
    let spi_mosi = pins.gpio19.into_function::<FunctionSpi>();
    let spi_miso = pins.gpio16.into_function::<FunctionSpi>();
    let spi_pins = (spi_mosi, spi_miso, spi_sck);

    let mut spi = Spi::<_, _, _, 8>::new(pac.SPI0, spi_pins).init(
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
        HertzU32::from_raw(400_000),
        MODE_0,
    );

    let mut sd_cs = pins.gpio17.into_push_pull_output();
    sd_cs.set_high().ok();

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

    // Prepare test pattern
    let mut buf = [0u8; 512];
    for i in 0..512 {
        buf[i] = i as u8;
    }
    buf[0] = 0xDE;
    buf[1] = 0xAD;
    buf[2] = 0xBE;
    buf[3] = 0xEF;

    let lba = 100; // test block

    info!("Writing pattern to block {=u32}...", lba);
    match sd_write_block(&mut spi, &mut sd_cs, lba, &buf, high_capacity) {
        Ok(()) => info!("Write OK."),
        Err(e) => error!("Write failed: {}", e),
    }

    info!("sd_write: done, looping");
    loop {
        cortex_m::asm::wfi();
    }
}
