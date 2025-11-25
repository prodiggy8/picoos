#![no_std]
#![no_main]

// ----------------------------------------------------------
// Boot2 for RP2040 (needed so the Pico actually boots)
// ----------------------------------------------------------
#[link_section = ".boot2"]
#[no_mangle]
#[used]
pub static BOOT2_FIRMWARE: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

// ----------------------------------------------------------
// Crates / imports
// ----------------------------------------------------------
use core::convert::Infallible;

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;

use embedded_hal::digital::v2::InputPin;

use rp_pico::entry;
use rp_pico::hal::{self as hal, pac, watchdog::Watchdog};

// ----------------------------------------------------------
// Simple key / event types
// ----------------------------------------------------------
#[derive(Clone, Copy)]
enum Key {
    Char(char),
    Enter,
    RightShift,
    Numpad(u8),   // 0–9
    NumpadPlus,
}

#[derive(Clone, Copy)]
enum KeyEvent {
    Make(Key),   // key pressed
    Break(Key),  // key released
}

// ----------------------------------------------------------
// Entry point
// ----------------------------------------------------------
#[entry]
fn main() -> ! {
    // --- Standard rp-pico init ---
    let mut pac = pac::Peripherals::take().unwrap();

    let mut watchdog = Watchdog::new(pac.WATCHDOG);

    // 12 MHz external crystal on the Pico board
    let _clocks = hal::clocks::init_clocks_and_plls(
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

    let sio = hal::Sio::new(pac.SIO);

    let pins = rp_pico::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    // ------------------------------------------------------
    // PS/2 wiring:
    //   CLOCK -> GP2
    //   DATA  -> GP3
    //
    // (Change these if you wired different pins.)
    // ------------------------------------------------------
    let clk = pins.gpio1.into_floating_input();
    let data = pins.gpio0.into_floating_input();

    info!("PS/2 keyboard: waiting for scan codes...");

    // State for handling F0 (break) prefix
    let mut break_pending = false;

    loop {
        let byte = read_ps2_byte(&clk, &data);

        // Always log raw scan code
        info!("Scan code: {=u8:02x}", byte);

        if let Some(ev) = handle_scancode_stream(byte, &mut break_pending) {
            log_key_event(ev);
        }
    }
}

// ----------------------------------------------------------
// PS/2 protocol helpers
// ----------------------------------------------------------

/// Wait for a falling edge on the clock line (high -> low).
fn wait_falling_edge<P: InputPin<Error = Infallible>>(clk: &P) {
    // Ensure we start from high
    while clk.is_low().unwrap() {}
    // Then wait until it goes low
    while clk.is_high().unwrap() {}
}

/// Check PS/2 odd parity (data + parity bit must have an odd number of 1s).
fn odd_parity_ok(byte: u8, parity_high: bool) -> bool {
    let ones = byte.count_ones();
    let p = if parity_high { 1 } else { 0 };
    ((ones + p) & 1) == 1
}

/// Blocking read of one PS/2 byte (scan code).
///
/// Frame format (device -> host):
///   start (0)
///   8 data bits (LSB first)
///   parity (odd)
///   stop (1)
fn read_ps2_byte<C, D>(clk: &C, data: &D) -> u8
where
    C: InputPin<Error = Infallible>,
    D: InputPin<Error = Infallible>,
{
    // 1. Wait for a real start bit: DATA = 0 on falling edge
    loop {
        wait_falling_edge(clk);
        if data.is_low().unwrap() {
            break;
        }
    }

    // 2. Read 8 data bits, LSB first
    let mut code: u8 = 0;
    for i in 0..8 {
        wait_falling_edge(clk);
        if data.is_high().unwrap() {
            code |= 1 << i;
        }
    }

    // 3. Parity bit (we just check it)
    wait_falling_edge(clk);
    let parity_bit_high = data.is_high().unwrap();

    // 4. Stop bit (should be 1)
    wait_falling_edge(clk);
    let stop_high = data.is_high().unwrap();

    if !stop_high {
        warn!("PS/2: bad stop bit, got byte {=u8:02x}", code);
    } else if !odd_parity_ok(code, parity_bit_high) {
        warn!("PS/2: parity error for byte {=u8:02x}", code);
    }

    code
}

// ----------------------------------------------------------
// Scan code set 2 decoding
// ----------------------------------------------------------

/// Map a *make* scan code (set 2) to a Key.
fn decode_make_set2(sc: u8) -> Option<Key> {
    match sc {
        // Letters
        0x1C => Some(Key::Char('a')),
        0x32 => Some(Key::Char('b')),
        0x21 => Some(Key::Char('c')),
        0x23 => Some(Key::Char('d')),
        0x24 => Some(Key::Char('e')),
        0x2B => Some(Key::Char('f')),
        0x34 => Some(Key::Char('g')),
        0x33 => Some(Key::Char('h')),
        0x43 => Some(Key::Char('i')),
        0x3B => Some(Key::Char('j')),
        0x42 => Some(Key::Char('k')),
        0x4B => Some(Key::Char('l')),
        0x3A => Some(Key::Char('m')),
        0x31 => Some(Key::Char('n')),
        0x44 => Some(Key::Char('o')),
        0x4D => Some(Key::Char('p')),
        0x15 => Some(Key::Char('q')),
        0x2D => Some(Key::Char('r')),
        0x1B => Some(Key::Char('s')),
        0x2C => Some(Key::Char('t')),
        0x3C => Some(Key::Char('u')),
        0x2A => Some(Key::Char('v')),
        0x1D => Some(Key::Char('w')),
        0x22 => Some(Key::Char('x')),
        0x35 => Some(Key::Char('y')),
        0x1A => Some(Key::Char('z')),

        // Top row digits
        0x45 => Some(Key::Char('0')),
        0x16 => Some(Key::Char('1')),
        0x1E => Some(Key::Char('2')),
        0x26 => Some(Key::Char('3')),
        0x25 => Some(Key::Char('4')),
        0x2E => Some(Key::Char('5')),
        0x36 => Some(Key::Char('6')),
        0x3D => Some(Key::Char('7')),
        0x3E => Some(Key::Char('8')),
        0x46 => Some(Key::Char('9')),
        0x49 => Some(Key::Char('.')),

        // Space + some punctuation
        0x29 => Some(Key::Char(' ')),   // Space
        0x4C => Some(Key::Char(';')),
        0x54 => Some(Key::Char('[')),

        // Enter & shift
        0x5A => Some(Key::Enter),
        0x59 => Some(Key::RightShift),

        // Keypad digits
        0x70 => Some(Key::Numpad(0)),
        0x69 => Some(Key::Numpad(1)),
        0x72 => Some(Key::Numpad(2)),
        0x7A => Some(Key::Numpad(3)),
        0x6B => Some(Key::Numpad(4)),
        0x73 => Some(Key::Numpad(5)),
        0x74 => Some(Key::Numpad(6)),
        0x6C => Some(Key::Numpad(7)),
        0x75 => Some(Key::Numpad(8)),
        0x7D => Some(Key::Numpad(9)),
        0x79 => Some(Key::NumpadPlus),

        _ => None,
    }
}

/// Feed bytes from the PS/2 reader into this, and get key events.
fn handle_scancode_stream(byte: u8, break_pending: &mut bool) -> Option<KeyEvent> {
    if *break_pending {
        *break_pending = false;
        if let Some(key) = decode_make_set2(byte) {
            return Some(KeyEvent::Break(key));
        }
        return None;
    }

    if byte == 0xF0 {
        // Next code will be a BREAK
        *break_pending = true;
        None
    } else {
        decode_make_set2(byte).map(KeyEvent::Make)
    }
}

// ----------------------------------------------------------
// Logging helper
// ----------------------------------------------------------

fn log_key_event(ev: KeyEvent) {
    match ev {
        KeyEvent::Make(Key::Char(c)) => {
            info!("MAKE  '{}'", c);
        }
        KeyEvent::Break(Key::Char(c)) => {
            info!("BREAK '{}'", c);
        }
        KeyEvent::Make(Key::Enter) => {
            info!("MAKE  <Enter>");
        }
        KeyEvent::Break(Key::Enter) => {
            info!("BREAK <Enter>");
        }
        KeyEvent::Make(Key::RightShift) => {
            info!("MAKE  <RightShift>");
        }
        KeyEvent::Break(Key::RightShift) => {
            info!("BREAK <RightShift>");
        }
        KeyEvent::Make(Key::Numpad(n)) => {
            info!("MAKE  <Num{}>", n);
        }
        KeyEvent::Break(Key::Numpad(n)) => {
            info!("BREAK <Num{}>", n);
        }
        KeyEvent::Make(Key::NumpadPlus) => {
            info!("MAKE  <Num+>");
        }
        KeyEvent::Break(Key::NumpadPlus) => {
            info!("BREAK <Num+>");
        }
    }
}
