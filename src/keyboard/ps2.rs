use embassy_rp::pio::StateMachine;
use embassy_rp::peripherals::PIO0;
use embassy_sync::channel::Sender;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use heapless::String;

#[derive(Debug, PartialEq)]
enum PS2State {
    Idle,
    Break,         // Got 0xF0, next is break code
    Extended,      // Got 0xE0, next is extended code
    ExtendedBreak, // Got 0xE0 then 0xF0, next is extended break code
}

// Converts PS2 scan code to ASCII letter/digit/symbol (if applicable)
fn scancode_to_char(scancode: u8, shift: bool) -> Option<char> {
    match scancode {
        0x1C => Some(if shift { 'A' } else { 'a' }),
        0x32 => Some(if shift { 'B' } else { 'b' }),
        0x21 => Some(if shift { 'C' } else { 'c' }),
        0x23 => Some(if shift { 'D' } else { 'd' }),
        0x24 => Some(if shift { 'E' } else { 'e' }),
        0x2B => Some(if shift { 'F' } else { 'f' }),
        0x34 => Some(if shift { 'G' } else { 'g' }),
        0x33 => Some(if shift { 'H' } else { 'h' }),
        0x43 => Some(if shift { 'I' } else { 'i' }),
        0x3B => Some(if shift { 'J' } else { 'j' }),
        0x42 => Some(if shift { 'K' } else { 'k' }),
        0x4B => Some(if shift { 'L' } else { 'l' }),
        0x3A => Some(if shift { 'M' } else { 'm' }),
        0x31 => Some(if shift { 'N' } else { 'n' }),
        0x44 => Some(if shift { 'O' } else { 'o' }),
        0x4D => Some(if shift { 'P' } else { 'p' }),
        0x15 => Some(if shift { 'Q' } else { 'q' }),
        0x2D => Some(if shift { 'R' } else { 'r' }),
        0x1B => Some(if shift { 'S' } else { 's' }),
        0x2C => Some(if shift { 'T' } else { 't' }),
        0x3C => Some(if shift { 'U' } else { 'u' }),
        0x2A => Some(if shift { 'V' } else { 'v' }),
        0x1D => Some(if shift { 'W' } else { 'w' }),
        0x22 => Some(if shift { 'X' } else { 'x' }),
        0x35 => Some(if shift { 'Y' } else { 'y' }),
        0x1A => Some(if shift { 'Z' } else { 'z' }),
        0x16 => Some(if shift { '!' } else { '1' }),
        0x1E => Some(if shift { '@' } else { '2' }),
        0x26 => Some(if shift { '#' } else { '3' }),
        0x25 => Some(if shift { '$' } else { '4' }),
        0x2E => Some(if shift { '%' } else { '5' }),
        0x36 => Some(if shift { '^' } else { '6' }),
        0x3D => Some(if shift { '&' } else { '7' }),
        0x3E => Some(if shift { '*' } else { '8' }),
        0x46 => Some(if shift { '(' } else { '9' }),
        0x45 => Some(if shift { ')' } else { '0' }),
        0x29 => Some(' '),  // Space
        0x49 => Some(if shift { '>' } else { '.' }),  // Period/dot
        0x4A => Some(if shift { '?' } else { '/' }),  // Slash
        _ => None,
    }
}

/* Data = high  Clock = high    Idle state
 * Data = high  Clock = low     Communication Inhibited
 * Data = low   Clock = high    Host Request-to-Send
 */
#[embassy_executor::task]
pub async fn input_reader(
    mut sm: StateMachine<'static, PIO0, 1>,
    sender: Sender<'static, ThreadModeRawMutex, char, 32>
) {
    let mut state = PS2State::Idle;
    let mut ctrl_pressed = false;
    let mut shift_pressed = false;
    let mut current_line = String::<256>::new();

    loop {
        let raw_frame = sm.rx().wait_pull().await;
        let scancode = raw_frame >> 21;
        
        let start  = (scancode >> 0x0) & 0x01;
        let code   = ((scancode >> 0x1) & 0xFF) as u8;
        let parity = (scancode >> 0x9) & 0x01;
        let stop   = (scancode >> 0xA) & 0x01;
        
        // Stop signal is always high
        if !(start == 0) {
            // Bad start bit

        } else if !(stop == 1) {
            // Bad stop bit

        } else {
            let parity_chk = ((code.count_ones() + parity) & 1) == 1;

            if !parity_chk {
                // Parity error

            } else {
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
                            }
                            0x12 | 0x59 => {
                                shift_pressed = true;
                            }
                            0x66 => {
                                current_line.pop();
                                sender.try_send('\x08').ok();
                            }
                            0x5A => {
                                current_line.clear();
                                sender.try_send('\n').ok();
                            }
                            _ => {
                                if let Some(ch) = scancode_to_char(code, shift_pressed) {
                                    if ctrl_pressed {
                                        // Ctrl+key combinations
                                        match ch {
                                            'c' => {
                                                sender.try_send('\x03').ok();
                                            }
                                            'z' => {
                                                sender.try_send('\x1A').ok();
                                            }
                                            _ => {}
                                        }
                                    } else {
                                        let _ = current_line.push(ch);
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
                        } else if code == 0x12 || code == 0x59 {
                            shift_pressed = false;
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
                                state = PS2State::Idle;
                            }
                            0x74 => {
                                state = PS2State::Idle;
                            }
                            0x6C => {
                                state = PS2State::Idle;
                            }
                            0x69 => {
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
    }
}
