//! This example uses the RP Pico W board Wifi chip (cyw43).
//! Connects to Wifi network and makes a web request to get the current time.

#![no_std]
#![no_main]
#![allow(async_fn_in_trait)]

use core::str::from_utf8;

use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_net::{Config, Ipv4Address, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::{Duration, Timer};
use reqwless::client::{HttpClient, TlsConfig, TlsVerify};
use reqwless::request::Method;
use serde::Deserialize;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _, serde_json_core};

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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Hello World!");

    let p = embassy_rp::init(Default::default());
    let mut rng = RoscRng;

    // let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    // let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download 43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download 43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

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
    // Use static IP configuration instead of DHCP
    //let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
    //    address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 69, 2), 24),
    //    dns_servers: Vec::new(),
    //    gateway: Some(Ipv4Address::new(192, 168, 69, 1)),
    //});

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

    info!("waiting for link...");
    stack.wait_link_up().await;

    info!("waiting for DHCP...");
    stack.wait_config_up().await;

    // And now we can use it!
    info!("Stack is up!");
    
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        info!("Connecting to WebSocket Server at {}:{}", SERVER_IP, SERVER_PORT);

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if let Err(e) = socket.connect((SERVER_IP, SERVER_PORT)).await {
            info!("Connection failed: {:?}", e);
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        info!("Connected via TCP. Sending WebSocket Handshake...");

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

        let message = "Hello, Wardf";

        for c in message.chars() {
            send_masked_char(&mut socket, c, &mut rng).await;
            Timer::after(Duration::from_millis(200)).await;
        }
        
        // Send Backspaces to fix "Wrold" -> "World"
        for _ in 0..5 {
            send_masked_char(&mut socket, '\x08', &mut rng).await; // \x08 is Backspace
            Timer::after(Duration::from_millis(300)).await;
        }

        let correction = "World\n";
        for char in correction.chars() {
            send_masked_char(&mut socket, char, &mut rng).await;
            Timer::after(Duration::from_millis(200)).await;
        }
        
        info!("done");
        Timer::after(Duration::from_secs(15)).await;
        
        let message2 = "! Hello!";

        for c in message2.chars() {
            send_masked_char(&mut socket, c, &mut rng).await;
            Timer::after(Duration::from_millis(200)).await;
        }

        /*
        //let client_state = TcpClientState::<1, 1024, 1024>::new();
        //let tcp_client = TcpClient::new(stack, &client_state);
        //let dns_client = DnsSocket::new(stack);

        //let mut http_client = HttpClient::new(&tcp_client, &dns_client);
        //let url = "http://meowfacts.herokuapp.com/";

        //info!("connecting to {}", &url);

        //let mut request = match http_client.request(Method::GET, &url).await {
        //    Ok(req) => req,
        //    Err(e) => {
        //        error!("Failed to make HTTP request: {:?}", e);
        //        return; // handle the error
        //    }
        //};

        //info!("Request done");

        //let response = match request.send(&mut rx_buffer).await {
        //    Ok(resp) => resp,
        //    Err(_e) => {
        //        error!("Failed to send HTTP request");
        //        return; // handle the error;
        //    }
        //};

        //info!("Response done");

        //let body = match from_utf8(response.body().read_to_end().await.unwrap()) {
        //    Ok(b) => b,
        //    Err(_e) => {
        //        error!("Failed to read response body");
        //        return; // handle the error
        //    }
        //};
        //info!("Response body: {:?}", &body);

        // Parse the JSON response
        // Visit https://meowfacts.herokuapp.com/ to see the raw json
        //#[derive(Debug, Deserialize)]
        //struct ApiResponse<'a> {
        // Tell the serde json parser that the data from this array will borrow from the input
        //    #[serde(borrow)]
        // There is a single field, data, that is an array containing a single string
        //    data: [&'a str; 1],
        // other fields as needed
        //}

        //let bytes = body.as_bytes();
        //match serde_json_core::de::from_slice::<ApiResponse>(bytes) {
        //    Ok((output, _used)) => {
        //        info!("Cat fact: {:?}", output.data[0]);
        //    }
        //    Err(_e) => {
        //        error!("Failed to parse response body");
        //        return; // handle the error
        //    }
        //}

        //Timer::after(Duration::from_secs(30)).await;*/
    }
}

async fn send_masked_char(socket: &mut TcpSocket<'_>, c: char, rng: &mut RoscRng) {
    let mut frame = [0u8; 8]; // 7 bytes payload
    
    // FIN (bit 0; message ended) + Opcode Text (bits 4-7; 0x1 for text)
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

    // Byte 6: The data (masked)
    let data_byte = c as u8;
    frame[6] = data_byte ^ mask_key[0]; // Simple XOR with first byte of mask since index is 0

    if let Err(e) = socket.write_all(&frame[0..7]).await {
        warn!("Failed to send char: {:?}", e);
    }
}