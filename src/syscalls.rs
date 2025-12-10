#![no_std]

use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;

// Global channel for userland output
pub static DISPLAY_CHANNEL: Channel<ThreadModeRawMutex, char, 256> = Channel::new();

// Global channel for userland input
pub static INPUT_CHANNEL: Channel<ThreadModeRawMutex, char, 32> = Channel::new();

#[repr(C)]
pub struct SyscallTable {
    pub print: extern "C" fn(*const u8, usize),
    pub read: extern "C" fn() -> u32, // Returns char cast to u32, or 0 if empty
}

// --- System Call ---
// This function is passed to userland programs to allow them to print.
// In the future, we should pass a syscall table that programs can use!
pub extern "C" fn sys_print(ptr: *const u8, len: usize) {
    let slice = unsafe { core::slice::from_raw_parts(ptr, len) };
    if let Ok(s) = core::str::from_utf8(slice) {
        let sender = DISPLAY_CHANNEL.sender();
        for c in s.chars() {
            let _ = sender.try_send(c);
        }
    }
}

pub extern "C" fn sys_read() -> u32 {
    if let Ok(c) = INPUT_CHANNEL.try_receive() {
        c as u32
    } else {
        0
    }
}
