use alloc::format;
use embedded_hal::digital::v2::ToggleableOutputPin;
use rp_pico::hal;
use hal::gpio::{FunctionSioOutput, Pin, PinId, PullDown};
use hal::uart::{UartDevice, ValidUartPinout};
use defmt::info;
use rp_pico::pac::{self, interrupt};
use cortex_m::peripheral::NVIC;

use crate::{
    dvi::VERTICAL_REPEAT,
    render::{end_display_list, rgb, start_display_list, BW_PALETTE, FONT_HEIGHT},
};

use heapless::HistoryBuf;
use heapless::String;

static mut UART_BUF: [u8; 1024] = [0; 1024];
static mut HEAD: usize = 0;
static mut TAIL: usize = 0;

#[interrupt]
fn UART1_IRQ() {
    let uart = unsafe { &*pac::UART1::ptr() };
    while uart.uartfr.read().rxfe().bit_is_clear() {
        let data = uart.uartdr.read().data().bits();
        unsafe {
            let next_head = (HEAD + 1) % 1024;
            if next_head != TAIL {
                UART_BUF[HEAD] = data;
                HEAD = next_head;
            }
        }
    }
}

struct Counter<P: PinId> {
    led_pin: Pin<P, FunctionSioOutput, PullDown>,
    count: u32,
}

impl<P: PinId> Counter<P> {
    fn count(&mut self) {
        if self.count % 15 == 0 {
            self.led_pin.toggle().unwrap();
        }
        self.count = self.count.wrapping_add(1);
    }
}

fn hello_pico<P: PinId>(counter: &Counter<P>, h: &HistoryBuf<String<213>, 31>, s: &String<213>) {
    let height = 480 / VERTICAL_REPEAT as u32;
    let (mut rb, mut sb) = start_display_list();

    let mut i = 1;

    for line in h.oldest_ordered() {
        i += 1;

        if i > (height / FONT_HEIGHT) {
            i -= 1;
            info!("FULL!");
            break;
        }

        rb.begin_stripe(FONT_HEIGHT);
        let char_text = format!("{}", line);
        let width = rb.text(&char_text);
        let width = width + width % 2;
        rb.end_stripe();

        sb.begin_stripe(FONT_HEIGHT);
        if width > 0 {
            let scan_width = if width > 640 { 640 } else { width };
            sb.pal_1bpp(scan_width, &BW_PALETTE);
            if width < 640 {
                sb.solid(640 - width, rgb(0, 0, 0));
            }
        } else {
            sb.solid(640, rgb(0, 0, 0));
        }
        sb.end_stripe();
    }
    rb.begin_stripe(FONT_HEIGHT);
    let char_text = format!("{}", s);
    let width = rb.text(&char_text);
    let width = width + width % 2;
    rb.end_stripe();

    sb.begin_stripe(FONT_HEIGHT);
    if width > 0 {
        let scan_width = if width > 640 { 640 } else { width };
        sb.pal_1bpp(scan_width, &BW_PALETTE);
        if width < 640 {
            sb.solid(640 - width, rgb(0, 0, 0));
        }
    } else {
        sb.solid(640, rgb(0, 0, 0));
    }
    sb.end_stripe();

    // 32 MAX LINES

    let remaining = height - i * FONT_HEIGHT;
    rb.begin_stripe(remaining);
    rb.end_stripe();
    sb.begin_stripe(remaining);
    sb.solid(640, rgb(0, 0, 0));
    sb.end_stripe();

    end_display_list(rb, sb);
}

pub fn demo<L: PinId, D: UartDevice, P: ValidUartPinout<D>>(
    led_pin: Pin<L, FunctionSioOutput, PullDown>,
    uart: &mut hal::uart::UartPeripheral<hal::uart::Enabled, D, P>
) -> !
{
    let mut counter = Counter { led_pin, count: 0 };
    let mut buffer = [0u8; 1];
    
    let mut curr_line: String<213> = String::new();
    let mut history = HistoryBuf::<String<213>, 31>::new();

    loop {
        counter.count();
        match uart.read_raw(&mut buffer) {
            Ok(bytes_read) => {
                if bytes_read > 0 {
                    let c = buffer[0] as char;
                    if c == '\r' {
                        curr_line.clear();
                    } else if c == '\x08' {
                        curr_line.pop();
                    } else if c == '\n' {
                        history.write(curr_line.clone());
                        curr_line.clear();
                    } else if c == '\x7f' {
                        history = HistoryBuf::<String<213>, 31>::new();
                        curr_line.clear();
                    } else if c >= ' ' && c <= '~' {
                        curr_line.push(c);
                    }
                }
            }
            Err(_) => { }
        }
        hello_pico(&counter, &history, &curr_line);
        //info!("{}", curr_line);
    }
}

