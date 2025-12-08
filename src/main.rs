#![no_std]
#![no_main]

// Keyboard imports
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Pull, Level, Output};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// Screen imports
use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config, Ipv4Address, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::pio;
use embassy_rp::pio::program::pio_file;
use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use static_cell::StaticCell;
use heapless::String;

// Trait for TcpSocket::write_all
use embedded_io_async::Write;
use core::fmt::Write as FMTWrite;

mod keyboard;
mod dvi;

use keyboard::ps2::input_reader;
use dvi::dvi::Dvi;

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

async fn send_masked_slice(socket: &mut TcpSocket<'_>, payload: &[u8], rng: &mut RoscRng) {
    let len = payload.len();
    if len == 0 { return; }
    
    // Header (2 bytes) + Mask Key (4 bytes)
    let mut header = [0u8; 6]; 

    // FIN + Text Frame (0x81)
    header[0] = 0x81;
    // Mask bit (0x80) + Length (assuming < 126 for this demo)
    header[1] = 0x80 | (len as u8);

    // Generate random mask
    let mut mask_key = [0u8; 4];
    rng.fill_bytes(&mut mask_key);
    header[2] = mask_key[0];
    header[3] = mask_key[1];
    header[4] = mask_key[2];
    header[5] = mask_key[3];

    // Send Header
    if socket.write_all(&header).await.is_err() { return; }

    // Mask Payload (in a temp buffer to avoid allocating too much)
    let mut masked_buf = [0u8; 256];
    // Copy and mask at the same time
    for (i, byte) in payload.iter().enumerate() {
        if i >= masked_buf.len() { break; }
        masked_buf[i] = byte ^ mask_key[i % 4];
    }

    // Send Payload
    let _ = socket.write_all(&masked_buf[..len]).await;
}

struct CounterProcess {
    running: bool,
    count: u32,
    limit: u32,
    next_update: u64,
}

struct GameProcess {
    running: bool,
    target: u32,
    input_buffer: String<8>,
}

#[derive(PartialEq)]
enum ForegroundTask {
    Shell,
    Counter,
    Game,
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let char_channel = CHAR_CHANNEL.init(Channel::new());
    let mut rng = RoscRng;
    
    let mut pio = Pio::new(p.PIO0, Irqs);

    let mut data = pio.common.make_pio_pin(p.PIN_0);
    let mut clk = pio.common.make_pio_pin(p.PIN_1);
    data.set_pull(Pull::Up);
    clk.set_pull(Pull::Up);

    let mut cfg = pio::Config::default();
    
    let mut dvi = Dvi::new(p.UART1, p.PIN_8, p.PIN_9, p.DMA_CH2, p.DMA_CH3);

    let prg = pio_file!(
        "src/keyboard/ps2.pio", 
        select_program("ps2")
    );

    cfg.use_program(&pio.common.load_program(&prg.program), &[]);
    cfg.set_in_pins(&[&data, &clk]);
    cfg.shift_in = embassy_rp::pio::ShiftConfig {
        auto_fill: false,
        direction: embassy_rp::pio::ShiftDirection::Right, 
        threshold: 32,
    };
    pio.sm1.set_config(&cfg);
    pio.sm1.set_enable(true);
    
    // ===== Network handling =====

    // // Firmware
    // let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    // let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };
    
    // // Configuring Wi-Fi module
    // let pwr = Output::new(p.PIN_23, Level::Low);
    // let cs = Output::new(p.PIN_25, Level::High);
    // let spi = PioSpi::new(
    //     &mut pio.common,
    //     pio.sm0,
    //     DEFAULT_CLOCK_DIVIDER,
    //     pio.irq0,
    //     cs,
    //     p.PIN_24,
    //     p.PIN_29,
    //     p.DMA_CH0,
    // );

    // static STATE: StaticCell<cyw43::State> = StaticCell::new();
    // let state = STATE.init(cyw43::State::new());
    // let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    // spawner.spawn(cyw43_task(runner)).unwrap();

    // control.init(clm).await;
    // control
    //     .set_power_management(cyw43::PowerManagementMode::Performance)
    //     .await;

    // let config = Config::dhcpv4(Default::default());

    // // Generate random seed
    // let seed = rng.next_u64();

    // // Init network stack
    // static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    // let (stack, runner) = embassy_net::new(
    //     net_device,
    //     config,
    //     RESOURCES.init(StackResources::new()),
    //     seed,
    // );

    // spawner.spawn(net_task(runner)).unwrap();

    // // Connect to the WIFI network
    // let mut attempts = 0u8;
    // loop {
    //     match control
    //         .join(WIFI_NETWORK, JoinOptions::new(WIFI_PASSWORD.as_bytes()))
    //         .await
    //     {
    //         Ok(_) => break,
    //         Err(err) => {
    //             attempts = attempts.saturating_add(1);
    //             warn!(
    //                 "join failed ({}), attempt {}. Retrying…",
    //                 err.status, attempts
    //             );
    //             Timer::after(Duration::from_millis(500)).await;
    //         }
    //     }
    // }

    // println!("[Network] Waiting for link...");
    // stack.wait_link_up().await;

    // println!("[Network] Waiting for DHCP...");
    // stack.wait_config_up().await;

    // println!("[Network] Stack is up!");

    let mut counter_proc = CounterProcess { running: false, count: 0, limit: 250, next_update: 0 };
    let mut game_proc = GameProcess { running: false, target: 0, input_buffer: String::new() };
    let mut current_focus = ForegroundTask::Shell;
    
    let mut shell_buffer = String::<64>::new();
    let mut output_buffer = String::<256>::new(); // OS Output Buffer


    let mut led  = Output::new(p.PIN_15, Level::Low);

    //let mut rx_buffer = [0; 4096];
    //let mut tx_buffer = [0; 4096];

    //let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    //socket.set_keep_alive(Some(Duration::from_millis(2500)));
    
    // Spawn tasks
    spawner.spawn(blinky(led)).unwrap();
    spawner.spawn(input_reader(pio.sm1, char_channel.sender())).unwrap();
    
    loop {
        // ===== Network handling =====
        // if socket.state() == State::Closed || socket.state() == State::Closing {

        //     if let Err(e) = socket.connect((SERVER_IP, SERVER_PORT)).await {
        //         info!("Connection failed: {:?}", e);
        //         Timer::after(Duration::from_secs(5)).await;
        //         continue;
        //     }

        //     info!("Connected via TCP");

        //     let handshake = "GET /shell HTTP/1.1\r\n\
        //                     Host: pico-client\r\n\
        //                     Upgrade: websocket\r\n\
        //                     Connection: Upgrade\r\n\
        //                     Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
        //                     Sec-WebSocket-Version: 13\r\n\
        //                     \r\n";
            
        //     if let Err(e) = socket.write_all(handshake.as_bytes()).await {
        //         info!("Write error: {:?}", e);
        //         continue;
        //     }

        //     let mut buf = [0u8; 512];

        //     match socket.read(&mut buf).await {
        //         Ok(n) => {
        //             let response = core::str::from_utf8(&buf[..n]).unwrap_or("");
        //             if !response.contains("101 Switching Protocols") {
        //                 info!("Server did not upgrade connection: {}", response);
        //                 continue;
        //             }
        //             info!("Upgrade successful");
        //         }
        //         Err(e) => {
        //             info!("Error reading handshake: {:?}", e);
        //             continue;
        //         }
        //     }
        // }
        // ===== Network handling =====
        while let Ok(ch) = char_channel.try_receive() {

            // Handle Ctrl-Z
            if ch == '\x1A' {
                match current_focus {
                    ForegroundTask::Counter => {
                        current_focus = ForegroundTask::Shell;
                        core::write!(output_buffer, "\n[1]+ Stopped counter (Backgrounded)\n$ ").ok();
                    },
                    ForegroundTask::Game => {
                        core::write!(output_buffer, "Cannot background Game!\nGuess: ").ok();
                    },
                    ForegroundTask::Shell => { core::write!(output_buffer, "^Z\n$ ").ok(); }
                }
                continue;
            }

            // Route Key to Focused Task
            match current_focus {
                ForegroundTask::Shell => {
                    output_buffer.push(ch).ok(); // Echo
                    if ch == '\n' {
                        let cmd = shell_buffer.as_str().trim();
                        if cmd == "counter" {
                            counter_proc.running = true;
                            counter_proc.count = 0;
                            counter_proc.next_update = embassy_time::Instant::now().as_millis();
                            current_focus = ForegroundTask::Counter;
                            core::write!(output_buffer, "Starting counter...\n").ok();
                        } else if cmd == "game" {
                            game_proc.running = true;
                            game_proc.target = (rng.next_u32() % 100) + 1;
                            game_proc.input_buffer.clear();
                            current_focus = ForegroundTask::Game;
                            core::write!(output_buffer, "Guess (1-100): ").ok();
                        } else if !shell_buffer.is_empty() {
                            core::write!(output_buffer, "Unknown: {}\n$ ", cmd).ok();
                        } else {
                            core::write!(output_buffer, "$ ").ok();
                        }
                        shell_buffer.clear();
                    } else if ch == '\x08' {
                        if !shell_buffer.is_empty() { shell_buffer.pop(); }
                    } else {
                        shell_buffer.push(ch).ok();
                    }
                },
                ForegroundTask::Game => {
                    if ch == '\n' {
                        output_buffer.push_str("\n").ok();
                        if let Ok(guess) = game_proc.input_buffer.as_str().parse::<u32>() {
                            if guess < game_proc.target {
                                core::write!(output_buffer, "Bigger!\nGuess: ").ok();
                            } else if guess > game_proc.target {
                                core::write!(output_buffer, "Smaller!\nGuess: ").ok();
                            } else {
                                core::write!(output_buffer, "CORRECT! Win.\n$ ").ok();
                                game_proc.running = false;
                                current_focus = ForegroundTask::Shell;
                            }
                        }
                        game_proc.input_buffer.clear();
                    } else if ch.is_digit(10) {
                        output_buffer.push(ch).ok();
                        game_proc.input_buffer.push(ch).ok();
                    }
                },
                ForegroundTask::Counter => {} // Counter takes no input
            }
        }

        // B. Background Task Management
        if counter_proc.running {
            let now = embassy_time::Instant::now().as_millis();
            if now > counter_proc.next_update + 200 { // 200ms speed
                counter_proc.count += 1;
                counter_proc.next_update = now;

                // Only print if focused
                if current_focus == ForegroundTask::Counter {
                    core::write!(output_buffer, "{}\n", counter_proc.count).ok();
                }

                if counter_proc.count >= counter_proc.limit {
                    counter_proc.running = false;
                    core::write!(output_buffer, "\n[Process 'counter' finished!]helloworld1helloworld2helloworld3helloworld4helloworld5helloworld6helloworld7helloworld8helloworld9helloworld10\n").ok();
                    if current_focus == ForegroundTask::Counter {
                        output_buffer.push_str("$ ").ok();
                        current_focus = ForegroundTask::Shell;
                    }
                }
            }
        }

        // C. Video Driver (Batch Send)
        if !output_buffer.is_empty() {
            //dvi.write(output_buffer.as_bytes()).await.ok();
            for c in output_buffer.chars() {
                dvi.write(&[c as u8]).await.ok();
                Timer::after_millis(10).await;
            }
            output_buffer.clear();
        }

        // Yield to Network/Keyboard
        Timer::after_millis(5).await;

        // if let Ok(ch) = char_channel.try_receive() {
        //     //send_masked_char(&mut socket, ch, &mut rng).await;
        //     let parsed = ch as u8; // THIS ASSUMES ASCII!!!!
        //     match dvi.write(&[parsed]).await {
        //         Ok(_) => info!("sending {}", ch),
        //         Err(_) => error!("Error sending uart"),
        //     }

        // } else {
        //     Timer::after_micros(10).await;
        // }
    }
}

#[embassy_executor::task]
async fn blinky(mut led: Output<'static>) {
    loop {
        led.set_high();
        Timer::after_millis(100).await;
        led.set_low();
        Timer::after_millis(900).await;    }
}
