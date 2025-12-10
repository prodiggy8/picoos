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
use heapless::Vec;

// Trait for TcpSocket::write_all
use embedded_io_async::Write;
use core::fmt::Write as FMTWrite;

mod keyboard;
mod dvi;
mod syscalls;
mod userland;

use keyboard::ps2::input_reader;
use dvi::dvi::Dvi;
use userland::{dummy_program, user_task_runner};

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

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

fn invoke_shell(
    ch: char,
    shell_buffer: &mut String<128>,
    output_buffer: &mut String<256>,
    spawner: Spawner,
    foreground: &mut bool
) {
    if ch == '\n' || ch == '\r' {
        // Write to DVI & parse command
        core::write!(output_buffer, "\n").ok();
        let cmd = shell_buffer.as_str().trim();

        if let Some(rest) = cmd.strip_prefix("exec ") {
            let rest = rest.trim();
            let (addr_str, args_str) = match rest.find(char::is_whitespace) {
                Some(idx) => (&rest[..idx], &rest[idx..]),
                None => (rest, ""),
            };
            
            let clean_addr = addr_str.trim_start_matches("0x");
            if let Ok(addr) = usize::from_str_radix(clean_addr, 16) {
                    let mut args = String::<64>::new();
                    core::write!(args, "{}", args_str.trim()).ok();
                    if spawner.spawn(user_task_runner(addr, args)).is_ok() {
                        *foreground = true;
                    } else {
                        core::write!(output_buffer, "Failed to spawn\n$ ").ok();
                    }
            } else {
                    core::write!(output_buffer, "Invalid address\n$ ").ok();
            }
        } else if !cmd.is_empty() {
            core::write!(output_buffer, "Unknown command\n$ ").ok();
        } else {
            core::write!(output_buffer, "$ ").ok();
        }
        
        // Start from scratch again
        shell_buffer.clear();
    
    // Handle Backspace
    } else if ch == '\x08' || ch == '\x7F' {
        if shell_buffer.pop().is_some() {
            core::write!(output_buffer, "\x08 \x08").ok();
        }

    // Ignore CTRL-Z: shell never leaves the foreground except if substitute is invoked
    } else if ch == '\x1A' {
        continue;

    // Handle every other character
    } else {
        shell_buffer.push(ch).ok();
        output_buffer.push(ch).ok();
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut p = embassy_rp::init(Default::default());
    let char_channel = CHAR_CHANNEL.init(Channel::new());
    let display_channel = &syscalls::DISPLAY_CHANNEL;
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
    
    let mut output_buffer = String::<256>::new(); // OS Output Buffer
    let mut shell_buffer = String::<128>::new();

    // Print dummy program address for testing
    core::write!(output_buffer, "Dummy Program Address: 0x{:x}\n$ ", dummy_program as usize).ok();

    let mut led  = Output::new(p.PIN_15, Level::Low);

    // Spawn tasks
    spawner.spawn(blinky(led)).unwrap();
    spawner.spawn(input_reader(pio.sm1, char_channel.sender())).unwrap();
    
    let mut args = String::<64>::new();
    core::write!(args, "Hello World").ok();
    spawner.spawn(user_task_runner(dummy_program as usize, args)).unwrap();
    let mut foreground = false;

    loop {
        if let Ok(ch) = char_channel.try_receive() {
            // If process: send the input to such process
            if foreground {
                // If CTRL-C: set foreground to None
                if ch == '\x03' {
                    foreground = false;
                    core::write!(output_buffer, "^C\n$ ").ok();
                } else {
                    let _ = syscalls::INPUT_CHANNEL.try_send(ch);
                }
            } else {
                invoke_shell(ch, &mut shell_buffer, &mut output_buffer, spawner, &mut foreground);
            }
        }

        // Ownership of the DVI
        if foreground {
            while let Ok(c) = display_channel.try_receive() {
                output_buffer.push(c).ok();
            }
        }

        // Video Driver
        if !output_buffer.is_empty() {
            for c in output_buffer.chars() {
                dvi.write(&[c as u8]).await.ok();
                Timer::after_millis(10).await;
            }
            output_buffer.clear();
        }

        // Yield to Network/Keyboard
        Timer::after_millis(5).await;
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
