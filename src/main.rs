#![no_std]
#![no_main]

// Keyboard imports
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Pull, Level, Output};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// SD
use embassy_rp::spi::{Spi, Config as SpiConfig, Phase, Polarity};

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
use heapless;

// Trait for TcpSocket::write_all
use embedded_io_async::Write;
use embedded_hal::digital::OutputPin;
use embedded_hal::spi::SpiBus;
use core::fmt::Write as FMTWrite;

mod keyboard;
mod dvi;
mod syscalls;
mod userland;
mod sd;

use keyboard::ps2::input_reader;
use dvi::dvi::Dvi;
use userland::{dummy_program, keyboard_program, counter_program, user_task_runner};
use sd::sd::{sd_init, sd_read_block, Fat32Info, fat32_list_directory, fat32_find_file, fat32_write_file_at_path, fat32_find_directory_by_path, fat32_create_directory, fat32_delete_file, fat32_delete_directory, fat32_read_file_complete, DirEntry};

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
static OUTPUT_BUFFER: StaticCell<String<1024>> = StaticCell::new();
static SHELL_BUFFER: StaticCell<String<128>> = StaticCell::new();
static SD_BUFFER: StaticCell<[u8; 512]> = StaticCell::new();

// Shell state to track current directory
struct ShellState {
    current_path: String<128>,
    current_cluster: u32,
}

impl ShellState {
    fn new(root_cluster: u32) -> Self {
        Self {
            current_path: String::new(),
            current_cluster: root_cluster,
        }
    }
}

// Pending shell commands that need async execution
// all FS ops are part of the shell (kernel land)
enum PendingCommand {
    None,
    Ls,
    Pwd,
    Cd(String<64>),
    Touch(String<64>),
    Cat(String<64>),
    Mkdir(String<64>),
    Rm(String<64>),
    Rmdir(String<64>),
}

fn invoke_shell(
    ch: char,
    shell_buffer: &mut String<128>,
    output_buffer: &mut String<1024>,
    spawner: Spawner,
    foreground: &mut bool,
    pending_cmd: &mut PendingCommand,
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
        } else if cmd == "ls" {
            *pending_cmd = PendingCommand::Ls;
        } else if cmd == "pwd" {
            *pending_cmd = PendingCommand::Pwd;
        } else if let Some(rest) = cmd.strip_prefix("cd ") {
            let target = rest.trim();
            let mut target_str = String::<64>::new();
            core::write!(target_str, "{}", target).ok();
            *pending_cmd = PendingCommand::Cd(target_str);
        } else if let Some(rest) = cmd.strip_prefix("touch ") {
            let filename = rest.trim();
            if !filename.is_empty() {
                let mut filename_str = String::<64>::new();
                core::write!(filename_str, "{}", filename).ok();
                *pending_cmd = PendingCommand::Touch(filename_str);
            } else {
                defmt::info!("Usage: touch <filename>");
                core::write!(output_buffer, "$ ").ok();
            }
        } else if let Some(rest) = cmd.strip_prefix("cat ") {
            let filename = rest.trim();
            if !filename.is_empty() {
                let mut filename_str = String::<64>::new();
                core::write!(filename_str, "{}", filename).ok();
                *pending_cmd = PendingCommand::Cat(filename_str);
            } else {
                defmt::info!("Usage: cat <filename>");
                core::write!(output_buffer, "$ ").ok();
            }
        } else if let Some(rest) = cmd.strip_prefix("mkdir ") {
            let dirname = rest.trim();
            if !dirname.is_empty() {
                let mut dirname_str = String::<64>::new();
                core::write!(dirname_str, "{}", dirname).ok();
                *pending_cmd = PendingCommand::Mkdir(dirname_str);
            } else {
                defmt::info!("Usage: mkdir <directory>");
                core::write!(output_buffer, "$ ").ok();
            }
        } else if let Some(rest) = cmd.strip_prefix("rm ") {
            let filename = rest.trim();
            if !filename.is_empty() {
                let mut filename_str = String::<64>::new();
                core::write!(filename_str, "{}", filename).ok();
                *pending_cmd = PendingCommand::Rm(filename_str);
            } else {
                defmt::info!("Usage: rm <filename>");
                core::write!(output_buffer, "$ ").ok();
            }
        } else if let Some(rest) = cmd.strip_prefix("rmdir ") {
            let dirname = rest.trim();
            if !dirname.is_empty() {
                let mut dirname_str = String::<64>::new();
                core::write!(dirname_str, "{}", dirname).ok();
                *pending_cmd = PendingCommand::Rmdir(dirname_str);
            } else {
                defmt::info!("Usage: rmdir <directory>");
                core::write!(output_buffer, "$ ").ok();
            }
        } else if cmd == "counter" {
            // Run counter program
            let mut args = String::<64>::new();
            if spawner.spawn(user_task_runner(counter_program as usize, args)).is_ok() {
                *foreground = true;
            } else {
                core::write!(output_buffer, "Failed to spawn counter\n$ ").ok();
            }
        } else if cmd == "clear" {
            output_buffer.clear();
            shell_buffer.clear();
            core::write!(output_buffer, "\x7f").ok();
            core::write!(output_buffer, "$ ").ok();
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
        // Do nothing

    // Handle every other character
    } else {
        shell_buffer.push(ch).ok();
        output_buffer.push(ch).ok();
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut p = embassy_rp::init(Default::default());

    // ===== INIT SD CARD
    let mut cs = Output::new(p.PIN_17, Level::High);
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = 400_000;  // 400 kHz for SD card initialization
    spi_config.phase = Phase::CaptureOnFirstTransition;
    spi_config.polarity = Polarity::IdleLow;

    let mut spi = Spi::new(
        p.SPI0,
        p.PIN_18,  // CLK
        p.PIN_19,  // MOSI
        p.PIN_16,  // MISO
        p.DMA_CH4,
        p.DMA_CH5,
        spi_config,
    );

    info!("Initializing SD card...\n");
    let high_capacity = match sd_init(&mut spi, &mut cs).await {
        Ok(hc) => hc,
        Err(e) => {
            error!("SD init failed: {}", e);
            loop { 
                Timer::after(Duration::from_millis(1000)).await;
            }  // Embassy async wait instead of blocking
        }
    };

    let buf = SD_BUFFER.init([0u8; 512]);
    sd_read_block(&mut spi, &mut cs, 0, buf, high_capacity).await.ok();
    
    // Parse FAT32 information from boot sector
    let fat_info = match Fat32Info::parse(buf) {
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

    // ===== INIT SYSCALL UTILS
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
    
    let output_buffer = OUTPUT_BUFFER.init(String::new()); // OS Output Buffer
    let shell_buffer = SHELL_BUFFER.init(String::new());
    let mut shell_state = ShellState::new(fat_info.root_dir_cluster);
    let mut pending_cmd = PendingCommand::None;
    
    output_buffer.clear();
    core::write!(output_buffer, "File System Initialized!\n");
    
    // Print dummy program addresses for testing
    core::write!(output_buffer, "Loaded p1 to 0x{:x}\n", dummy_program as usize).ok();
    Timer::after_millis(5).await;
    
    core::write!(output_buffer, "Loaded p2 to 0x{:x}\n", keyboard_program as usize).ok();
    Timer::after_millis(5).await;

    // emptying
    for c in output_buffer.chars() {
        dvi.write(&[c as u8]).await.ok();
        Timer::after_millis(10).await;
    }
    output_buffer.clear();

    // emptying
    for c in output_buffer.chars() {
        dvi.write(&[c as u8]).await.ok();
        Timer::after_millis(10).await;
    }
    output_buffer.clear();

    let mut led  = Output::new(p.PIN_15, Level::Low);

    // Spawn tasks
    spawner.spawn(blinky(led)).unwrap();
    spawner.spawn(input_reader(pio.sm1, char_channel.sender())).unwrap();
    
    let mut args = String::<64>::new();
    // core::write!(args, "Hello World").ok();
    // spawner.spawn(user_task_runner(dummy_program as usize, args)).unwrap();
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
                invoke_shell(ch, shell_buffer, output_buffer, spawner, &mut foreground, &mut pending_cmd);
            }
        }

        // Process pending async commands
        match pending_cmd {
            PendingCommand::Pwd => {
                if shell_state.current_path.is_empty() {
                    defmt::info!("Current directory: /");
                } else {
                    defmt::info!("Current directory: {}", shell_state.current_path.as_str());
                }
                core::write!(output_buffer, "$ ").ok();
                pending_cmd = PendingCommand::None;
            }
            PendingCommand::Ls => {
                defmt::info!("Contents of {}:", if shell_state.current_path.is_empty() { "/" } else { shell_state.current_path.as_str() });
                if let Err(e) = fat32_list_directory(&mut spi, &mut cs, &fat_info, shell_state.current_cluster, high_capacity).await {
                    defmt::info!("Error listing directory: {}", e);
                }
                core::write!(output_buffer, "$ ").ok();
                pending_cmd = PendingCommand::None;
            }
            PendingCommand::Cd(ref target) => {
                let target_str = target.as_str();
                if target_str == ".." {
                    // Go to parent directory
                    if !shell_state.current_path.is_empty() {
                        if let Some(last_slash) = shell_state.current_path.rfind('/') {
                            shell_state.current_path.truncate(last_slash);
                            if shell_state.current_path.is_empty() {
                                shell_state.current_cluster = fat_info.root_dir_cluster;
                            } else {
                                // Find the cluster for the new path
                                match fat32_find_directory_by_path(&mut spi, &mut cs, &fat_info, shell_state.current_path.as_str(), high_capacity).await {
                                    Ok(cluster) => shell_state.current_cluster = cluster,
                                    Err(_) => {
                                        shell_state.current_cluster = fat_info.root_dir_cluster;
                                        shell_state.current_path.clear();
                                    }
                                }
                            }
                        }
                    }
                    defmt::info!("Changed to parent directory");
                } else if target_str == "/" {
                    // Go to root
                    shell_state.current_cluster = fat_info.root_dir_cluster;
                    shell_state.current_path.clear();
                    defmt::info!("Changed to root directory");
                } else {
                    // Try to find the directory
                    match fat32_find_file(&mut spi, &mut cs, &fat_info, shell_state.current_cluster, target_str, high_capacity).await {
                        Ok(Some(entry)) => {
                            if entry.attr & 0x10 != 0 {  // It's a directory
                                shell_state.current_cluster = entry.start_cluster;
                                if !shell_state.current_path.is_empty() {
                                    shell_state.current_path.push('/').ok();
                                }
                                shell_state.current_path.push_str(target_str).ok();
                                defmt::info!("Changed to directory '{}'", target_str);
                            } else {
                                defmt::info!("'{}' is not a directory", target_str);
                            }
                        }
                        Ok(None) => defmt::info!("Directory '{}' not found", target_str),
                        Err(e) => defmt::info!("Error: {}", e),
                    }
                }
                core::write!(output_buffer, "$ ").ok();
                pending_cmd = PendingCommand::None;
            }
            PendingCommand::Touch(ref filename) => {
                let filename_str = filename.as_str();
                // Build full path
                let mut full_path = String::<256>::new();
                if !shell_state.current_path.is_empty() {
                    core::write!(full_path, "{}/{}", shell_state.current_path.as_str(), filename_str).ok();
                } else {
                    core::write!(full_path, "/{}", filename_str).ok();
                }
                
                let empty_data = &[];
                match fat32_write_file_at_path(&mut spi, &mut cs, &fat_info, full_path.as_str(), empty_data, high_capacity).await {
                    Ok(()) => defmt::info!("Created empty file '{}'", filename_str),
                    Err(e) => defmt::info!("Failed to create file: {}", e),
                }
                core::write!(output_buffer, "$ ").ok();
                pending_cmd = PendingCommand::None;
            }
            PendingCommand::Cat(ref filename) => {
                let filename_str = filename.as_str();
                // Find the file
                match fat32_find_file(&mut spi, &mut cs, &fat_info, shell_state.current_cluster, filename_str, high_capacity).await {
                    Ok(Some(entry)) => {
                        if entry.attr & 0x10 == 0 {  // It's a file
                            let mut read_buf = [0u8; 512];
                            match fat32_read_file_complete(&mut spi, &mut cs, &fat_info, entry.start_cluster, entry.size, &mut read_buf, high_capacity).await {
                                Ok(bytes_read) => {
                                    defmt::info!("=== {} ({} bytes) ===", filename_str, bytes_read);
                                    // Print file content in chunks
                                    for chunk in read_buf[..bytes_read].chunks(64) {
                                        defmt::info!("{=[u8]:a}", chunk);
                                    }
                                    defmt::info!("=== End of {} ===", filename_str);
                                }
                                Err(e) => defmt::info!("Error reading file: {}", e),
                            }
                        } else {
                            defmt::info!("'{}' is a directory", filename_str);
                        }
                    }
                    Ok(None) => defmt::info!("File '{}' not found", filename_str),
                    Err(e) => defmt::info!("Error: {}", e),
                }
                core::write!(output_buffer, "$ ").ok();
                pending_cmd = PendingCommand::None;
            }
            PendingCommand::Mkdir(ref dirname) => {
                let dirname_str = dirname.as_str();
                match fat32_create_directory(&mut spi, &mut cs, &fat_info, shell_state.current_cluster, dirname_str, high_capacity).await {
                    Ok(_) => defmt::info!("Created directory '{}'", dirname_str),
                    Err(e) => defmt::info!("Failed to create directory: {}", e),
                }
                core::write!(output_buffer, "$ ").ok();
                pending_cmd = PendingCommand::None;
            }
            PendingCommand::Rm(ref filename) => {
                let filename_str = filename.as_str();
                match fat32_delete_file(&mut spi, &mut cs, &fat_info, shell_state.current_cluster, filename_str, high_capacity).await {
                    Ok(()) => defmt::info!("Deleted file '{}'", filename_str),
                    Err(e) => defmt::info!("Failed to delete file: {}", e),
                }
                core::write!(output_buffer, "$ ").ok();
                pending_cmd = PendingCommand::None;
            }
            PendingCommand::Rmdir(ref dirname) => {
                let dirname_str = dirname.as_str();
                match fat32_delete_directory(&mut spi, &mut cs, &fat_info, shell_state.current_cluster, dirname_str, high_capacity).await {
                    Ok(()) => defmt::info!("Deleted directory '{}'", dirname_str),
                    Err(e) => defmt::info!("Failed to delete directory: {}", e),
                }
                core::write!(output_buffer, "$ ").ok();
                pending_cmd = PendingCommand::None;
            }
            PendingCommand::None => {}
        }

        // Ownership of the DVI
        if foreground {
            if let Ok(c) = display_channel.try_receive() {
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
