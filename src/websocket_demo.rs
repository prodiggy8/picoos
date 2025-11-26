//! PS/2 Keyboard over WiFi WebSocket
//! Reads PS/2 keyboard input and sends it via WebSocket to a server

#![no_std]
#![no_main]
#![allow(async_fn_in_trait)]

use core::convert::Infallible;
use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config, Ipv4Address, Stack, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::{Duration, Timer};
use embedded_hal::digital::v2::InputPin;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use embedded_io_async::Write;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

const WIFI_NETWORK: &str = "ES-LAB"; // change to your network SSID
const WIFI_PASSWORD: &str = "ESl@b@123#@!"; // change to your network password

const SERVER_IP: Ipv4Address = Ipv4Address::new(172, 20, 34, 242);
const SERVER_PORT: u16 = 8080;

// ----------------------------------------------------------
// Key / event types
// ----------------------------------------------------------
#[derive(Clone, Copy)]
enum Key {
    Char(char),
    Enter,
    Backspace,
    RightShift,
    Numpad(u8),   // 0–9
    NumpadPlus,
}

#[derive(Clone, Copy)]
enum KeyEvent {
    Make(Key),   // key pressed
    Break(Key),  // key released
}

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // firmware
    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    // configuring Pico Wi-Fi module
    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    spawner.spawn(cyw43_task(runner)).unwrap();

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Performance)
        .await;

    let config = Config::dhcpv4(Default::default());

    // Generate random seed
    let mut rng = RoscRng;
    let seed = rng.next_u64();

    // Init network stack
    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );

    spawner.spawn(net_task(runner)).unwrap();

    // Connect to the WIFI network
    let mut attempts = 0u8;
    loop {
        match control
            .join(WIFI_NETWORK, JoinOptions::new(WIFI_PASSWORD.as_bytes()))
            .await
        {
            Ok(_) => break,
            Err(err) => {
                attempts = attempts.saturating_add(1);
                warn!(
                    "join failed ({}), attempt {}. Retrying…",
                    err.status, attempts
                );
                Timer::after(Duration::from_millis(500)).await;
            }
        }
    }

    info!("Waiting for link...");
    stack.wait_link_up().await;

    info!("Waiting for DHCP...");
    stack.wait_config_up().await;

    info!("Stack is up!");

    // PS/2 keyboard pins (GP0 = DATA, GP1 = CLOCK)
    let ps2_data = Input::new(p.PIN_0, Pull::Up);
    let ps2_clk = Input::new(p.PIN_1, Pull::Up);

    info!("PS/2 keyboard: waiting for scan codes...");
    
    let mut break_pending = false;
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_keep_alive(Some(Duration::from_millis(1000)));

    // Connect to server and perform WebSocket handshake
    loop {
        info!("Connecting to server...");
        if let Err(e) = socket.connect((SERVER_IP, SERVER_PORT)).await {
            warn!("Connection failed: {:?}", e);
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        info!("Connected via TCP");

        // Send WebSocket handshake
        let handshake = "GET /shell HTTP/1.1\r\n\
                        Host: pico-client\r\n\
                        Upgrade: websocket\r\n\
                        Connection: Upgrade\r\n\
                        Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                        Sec-WebSocket-Version: 13\r\n\
                        \r\n";
        
        if let Err(e) = socket.write_all(handshake.as_bytes()).await {
            warn!("Write error: {:?}", e);
            continue;
        }

        // Read handshake response
        let mut buf = [0u8; 512];
        match socket.read(&mut buf).await {
            Ok(n) => {
                let response = core::str::from_utf8(&buf[..n]).unwrap_or("");
                if !response.contains("101 Switching Protocols") {
                    warn!("Server did not upgrade connection");
                    continue;
                }
                info!("WebSocket upgrade successful");
                break; // Successfully connected
            }
            Err(e) => {
                warn!("Error reading handshake: {:?}", e);
                continue;
            }
        }
    }

    // Main loop: read PS/2 and send via WebSocket
    loop {
        let byte = read_ps2_byte_blocking(&ps2_clk, &ps2_data);
        info!("Scan code: {=u8:02x}", byte);

        if let Some(ev) = handle_scancode_stream(byte, &mut break_pending) {
            // Only send characters on MAKE (key press), not BREAK (release)
            if let KeyEvent::Make(key) = ev {
                let char_to_send = match key {
                    Key::Char(c) => {
                        info!("Key pressed: '{}'", c);
                        Some(c)
                    }
                    Key::Enter => {
                        info!("Key pressed: <Enter>");
                        Some('\n')
                    }
                    Key::Backspace => {
                        info!("Key pressed: <Backspace>");
                        Some('\x08')
                    }
                    _ => None
                };
                
                if let Some(c) = char_to_send {
                    if let Err(e) = send_masked_char(&mut socket, c, &mut rng).await {
                        warn!("Failed to send character: {:?}, reconnecting...", e);
                        // Connection lost, reconnect
                        loop {
                            Timer::after(Duration::from_secs(5)).await;
                            if socket.connect((SERVER_IP, SERVER_PORT)).await.is_ok() {
                                let handshake = "GET /shell HTTP/1.1\r\n\
                                                Host: pico-client\r\n\
                                                Upgrade: websocket\r\n\
                                                Connection: Upgrade\r\n\
                                                Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                                                Sec-WebSocket-Version: 13\r\n\
                                                \r\n";
                                if socket.write_all(handshake.as_bytes()).await.is_ok() {
                                    let mut buf = [0u8; 512];
                                    if let Ok(n) = socket.read(&mut buf).await {
                                        let response = core::str::from_utf8(&buf[..n]).unwrap_or("");
                                        if response.contains("101 Switching Protocols") {
                                            info!("Reconnected!");
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------
// WebSocket frame helper
// ----------------------------------------------------------
async fn send_masked_char(socket: &mut TcpSocket<'_>, c: char, rng: &mut RoscRng) -> Result<(), embassy_net::tcp::Error> {
    let mut frame = [0u8; 8];
    
    // FIN bit (bit 7) + Opcode Text (0x1) = 0x81
    frame[0] = 0x81; 
    
    // Byte 1: Mask bit (0x80) + Payload Len (1) = 0x81
    frame[1] = 0x81;

    // Bytes 2-5: Masking Key (Random)
    let mut mask_key = [0u8; 4];
    rng.fill_bytes(&mut mask_key);
    frame[2] = mask_key[0];
    frame[3] = mask_key[1];
    frame[4] = mask_key[2];
    frame[5] = mask_key[3];

    // data (masked)
    let data_byte = c as u8;
    frame[6] = data_byte ^ mask_key[0];

    socket.write_all(&frame[0..7]).await?;
    Ok(())
}

// ----------------------------------------------------------
// PS/2 protocol helpers (blocking version for tight timing)
// ----------------------------------------------------------

/// Wait for a falling edge on the clock line (high -> low) - blocking version
fn wait_falling_edge_blocking<P: InputPin<Error = Infallible>>(clk: &P) {
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

/// Blocking read of one PS/2 byte (scan code) - tight timing for reliability
fn read_ps2_byte_blocking<C, D>(clk: &C, data: &D) -> u8
where
    C: InputPin<Error = Infallible>,
    D: InputPin<Error = Infallible>,
{
    // 1. Wait for a real start bit: DATA = 0 on falling edge
    loop {
        wait_falling_edge_blocking(clk);
        if data.is_low().unwrap() {
            break;
        }
    }

    // 2. Read 8 data bits, LSB first
    let mut code: u8 = 0;
    for i in 0..8 {
        wait_falling_edge_blocking(clk);
        if data.is_high().unwrap() {
            code |= 1 << i;
        }
    }

    // 3. Parity bit
    wait_falling_edge_blocking(clk);
    let parity_bit_high = data.is_high().unwrap();

    // 4. Stop bit (should be 1)
    wait_falling_edge_blocking(clk);
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

        // Enter, Backspace & shift
        0x5A => Some(Key::Enter),
        0x66 => Some(Key::Backspace),
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