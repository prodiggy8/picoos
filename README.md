##### PicoOS

A bare-metal, very simple operating system for the RP2040/RPI Pico-W written in Rust. Built on [Embassy](https://embassy.dev/).

**Features**

- shell with Unix-like commands
- SD card support with a FAT32-like file system
- text display via DVI, based on [DusterTheFirst's implementation](https://github.com/DusterTheFirst/pico-dvi-rs) 
- PS2 keyboard input via PIO
- run programs with syscalls (print, read)
- load and execute ELF binaries
- CYW43 wireless support
- TCP/IP stack via embassy-net
- debug with websockets

##### Prerequisites

- [Rust](https://rustup.rs/) with `thumbv6m-none-eabi` target
- [probe-rs](https://probe.rs/) for flashing (or [elf2uf2-rs](https://crates.io/crates/elf2uf2-rs) for USB boot mode)
- See PCB above for details on peripherals

##### Build & Run

```bash
cargo run
```