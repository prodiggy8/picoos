#![no_std]
#![no_main]

// Keyboard imports
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Input, Pull, Level, Output};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};
use heapless::String;

// Screen imports
use core::str::from_utf8;
use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use embassy_net::tcp::{TcpSocket, State};
use embassy_net::{Config, Ipv4Address, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_sync::channel::{Channel, Sender};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// Trait for TcpSocket::write_all
use embedded_io_async::Write;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

const WIFI_NETWORK: &str = "ES-LAB"; // change to your network SSID
const WIFI_PASSWORD: &str = "ESl@b@123#@!"; // change to your network password

const SERVER_IP: Ipv4Address = Ipv4Address::new(172, 20, 34, 242);
const SERVER_PORT: u16 = 8080;

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

static CHAR_CHANNEL: StaticCell<Channel<ThreadModeRawMutex, char, 32>> = StaticCell::new();

async fn send_masked_char(socket: &mut TcpSocket<'_>, c: char, rng: &mut RoscRng) {
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
    frame[6] = data_byte ^ mask_key[0]; // Simple XOR with first byte of mask since index is 0

    if let Err(e) = socket.write_all(&frame[0..7]).await {
        warn!("Failed to send char: {:?}", e);
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let char_channel = CHAR_CHANNEL.init(Channel::new());
    let mut rng = RoscRng;

    // ===== Network handling =====

    // Firmware
    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    // Configuring Wi-Fi module
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

    println!("[Network] Waiting for link...");
    stack.wait_link_up().await;

    println!("[Network] Waiting for DHCP...");
    stack.wait_config_up().await;

    println!("[Network] Stack is up!");

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_keep_alive(Some(Duration::from_millis(2500)));

    // ===== Network handling =====

    // Configure inputs
    let data: Input<'static> = Input::new(p.PIN_0, Pull::Up);
    let clk: Input<'static> = Input::new(p.PIN_1, Pull::Up);

    // Spawn keyboard task
    spawner.spawn(input_reader(clk, data, char_channel.sender())).unwrap();

    loop {
        // ===== Network handling =====
        if socket.state() == State::Closed || socket.state() == State::Closing {

            if let Err(e) = socket.connect((SERVER_IP, SERVER_PORT)).await {
                info!("Connection failed: {:?}", e);
                Timer::after(Duration::from_secs(5)).await;
                continue;
            }

            info!("Connected via TCP");

            let handshake = "GET /shell HTTP/1.1\r\n\
                            Host: pico-client\r\n\
                            Upgrade: websocket\r\n\
                            Connection: Upgrade\r\n\
                            Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                            Sec-WebSocket-Version: 13\r\n\
                            \r\n";
            
            if let Err(e) = socket.write_all(handshake.as_bytes()).await {
                info!("Write error: {:?}", e);
                continue;
            }

            let mut buf = [0u8; 512];

            match socket.read(&mut buf).await {
                Ok(n) => {
                    let response = core::str::from_utf8(&buf[..n]).unwrap_or("");
                    if !response.contains("101 Switching Protocols") {
                        info!("Server did not upgrade connection: {}", response);
                        continue;
                    }
                    info!("Upgrade successful");
                }
                Err(e) => {
                    info!("Error reading handshake: {:?}", e);
                    continue;
                }
            }
        }
        // ===== Network handling =====
        
        if let Ok(ch) = char_channel.try_receive() {
            send_masked_char(&mut socket, ch, &mut rng).await;
        } else {
            Timer::after_micros(10).await;
        }
    }
}

/* Embassy has a p.wait_for_falling_edge, however, it returns a future
 * We can't have awaits in this portion, it is too slow to handle PS2
 */
fn wait_for_falling_edge_stable(clk: &Input<'static>) {
    while clk.is_low() {}
    while clk.is_high() {}
}

// PS2 State Machine States
#[derive(Debug, PartialEq)]
enum PS2State {
    Idle,
    Break,         // Got 0xF0, next is break code
    Extended,      // Got 0xE0, next is extended code
    ExtendedBreak, // Got 0xE0 then 0xF0, next is extended break code
}

// Converts PS2 scan code to ASCII letter/digit/symbol (if applicable)
fn scancode_to_char(scancode: u8) -> Option<char> {
    match scancode {
        0x1C => Some('a'), 0x32 => Some('b'), 0x21 => Some('c'), 0x23 => Some('d'),
        0x24 => Some('e'), 0x2B => Some('f'), 0x34 => Some('g'), 0x33 => Some('h'),
        0x43 => Some('i'), 0x3B => Some('j'), 0x42 => Some('k'), 0x4B => Some('l'),
        0x3A => Some('m'), 0x31 => Some('n'), 0x44 => Some('o'), 0x4D => Some('p'),
        0x15 => Some('q'), 0x2D => Some('r'), 0x1B => Some('s'), 0x2C => Some('t'),
        0x3C => Some('u'), 0x2A => Some('v'), 0x1D => Some('w'), 0x22 => Some('x'),
        0x35 => Some('y'), 0x1A => Some('z'),
        0x16 => Some('1'), 0x1E => Some('2'), 0x26 => Some('3'), 0x25 => Some('4'),
        0x2E => Some('5'), 0x36 => Some('6'), 0x3D => Some('7'), 0x3E => Some('8'),
        0x46 => Some('9'), 0x45 => Some('0'),
        0x29 => Some(' '),  // Space
        0x49 => Some('.'),  // Period/dot
        0x4A => Some('/'),  // Slash
        _ => None,
    }
}

/* Data = high  Clock = high    Idle state
 * Data = high  Clock = low     Communication Inhibited
 * Data = low   Clock = high    Host Request-to-Send
 */
#[embassy_executor::task]
async fn input_reader(
    clk: Input<'static>,
    data: Input<'static>,
    sender: Sender<'static, ThreadModeRawMutex, char, 32>
) {
    let mut state = PS2State::Idle;
    let mut ctrl_pressed = false;
    let mut current_line = String::<256>::new();

    loop {
        // Burst polling!
        let mut caught_edge = false;
        
        // Sample appropriately
        for _ in 0..2000 {
            if clk.is_low() {
                caught_edge = true;
                break;
            }
        }

        // If no character in this period, just yield to let network work!
        if !caught_edge {
            Timer::after_micros(10).await; 
            continue;
        }
        
        // Brute-force disable embassy's asynchronicity, otherwise too slow
        let (code, parity, stop) = cortex_m::interrupt::free(|_| {
            let mut code: u8 = 0;
            for i in 0..8 {
                wait_for_falling_edge_stable(&clk);

                if data.is_high() {
                    code |= 1 << i;
                }
            }

            // Parity check
            wait_for_falling_edge_stable(&clk);
            let parity: bool = data.is_high();
            
            // Stop signal is always high
            wait_for_falling_edge_stable(&clk);
            let stop: bool = data.is_high();

            (code, parity, stop)
        });
        
        // Stop signal is always high
        if !stop {
            warn!("PS2 bad stop bit, got byte {=u8:02x}", code);
        } else {
            let parity_bit: u32 = if parity { 1 } else { 0 };
            let parity_chk: bool = ((code.count_ones() + parity_bit) & 1) == 1;

            if !parity_chk {
                warn!("PS2 parity error, got byte {=u8:02x}", code);
            } else {
                // State machine for PS2 protocol
                match state {
                    PS2State::Idle => {
                        match code {
                            0xF0 => {
                                // Break code prefix
                                state = PS2State::Break;
                            }
                            0xE0 => {
                                // Extended code prefix
                                state = PS2State::Extended;
                            }
                            0x14 => {
                                // Left or Right Ctrl pressed
                                ctrl_pressed = true;
                                println!("CTRL pressed");
                            }
                            0x66 => {
                                current_line.pop();
                                // println!("{}", current_line.as_str());
                                sender.try_send('\x08').ok();
                            }
                            0x5A => {
                                // println!("{}", current_line.as_str());
                                current_line.clear();
                                sender.try_send('\n').ok();
                            }
                            _ => {
                                // Check if it's a letter or digit
                                if let Some(ch) = scancode_to_char(code) {
                                    if ctrl_pressed {
                                        // Handle Ctrl+key combinations
                                        match ch {
                                            'c' => println!("CTRL-C"),
                                            'z' => println!("CTRL-Z"),
                                            _ => println!("CTRL+{}", ch),
                                        }
                                    } else {
                                        let _ = current_line.push(ch);
                                        // println!("{}", current_line.as_str());
                                        sender.try_send(ch).ok();
                                    }
                                }
                            }
                        }
                    }
                    PS2State::Break => {
                        // Key released
                        if code == 0x14 {
                            // Ctrl released
                            ctrl_pressed = false;
                            println!("CTRL released");
                        }
                        state = PS2State::Idle;
                    }
                    PS2State::Extended => {
                        // Extended key codes (arrows, home, end, etc.)
                        match code {
                            0xF0 => {
                                // Extended break code coming next
                                state = PS2State::ExtendedBreak;
                            }
                            0x6B => {
                                println!("Key event: ARROW LEFT");
                                state = PS2State::Idle;
                            }
                            0x74 => {
                                println!("Key event: ARROW RIGHT");
                                state = PS2State::Idle;
                            }
                            0x6C => {
                                println!("Key event: HOME");
                                state = PS2State::Idle;
                            }
                            0x69 => {
                                println!("Key event: END");
                                state = PS2State::Idle;
                            }
                            _ => {
                                state = PS2State::Idle;
                            }
                        }
                    }
                    PS2State::ExtendedBreak => {
                        // Extended key released - ignore for now
                        state = PS2State::Idle;
                    }
                }
            }
        }
        
        Timer::after_micros(10).await;
    }
}