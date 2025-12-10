use heapless::String;
use crate::syscalls::{sys_print, sys_read, SyscallTable};
use embassy_time::Timer;

static mut KEYBOARD_BUFFER: String<64> = String::new();

#[inline(never)]
#[link_section = ".data.ramfunc"]
#[no_mangle]
pub extern "C" fn dummy_program(
    table: &SyscallTable,
    args_ptr: *const u8,
    args_len: usize
) -> u32 {
    // Echo its arguments
    (table.print)(args_ptr, args_len);
    
    let newline = "\n";
    (table.print)(newline.as_ptr(), newline.len());
    
    0 // Exit
}

#[inline(never)]
#[link_section = ".data.ramfunc"]
#[no_mangle]
pub extern "C" fn keyboard_program(
    table: &SyscallTable,
    _args_ptr: *const u8,
    _args_len: usize
) -> u32 {
    let buffer = unsafe { &mut KEYBOARD_BUFFER };
    
    let c_u32 = (table.read)();
    if c_u32 != 0 {
        if let Some(c) = char::from_u32(c_u32) {
            // Echo character
            let mut temp = String::<4>::new();
            if temp.push(c).is_ok() {
                (table.print)(temp.as_ptr(), temp.len());
            }

            if c == '\n' || c == '\r' {
                // Print buffer
                let prefix = "Buffer: ";
                (table.print)(prefix.as_ptr(), prefix.len());
                (table.print)(buffer.as_ptr(), buffer.len());
                let newline = "\n";
                (table.print)(newline.as_ptr(), newline.len());
                buffer.clear();
            } else {
                let _ = buffer.push(c);
            }
        }
    }
    
    1 // Continue
}

#[embassy_executor::task(pool_size = 2)]
pub async fn user_task_runner(addr: usize, args: String<64>) {
    // Signature: fn(&SyscallTable, args_ptr, args_len) -> u32
    let program: extern "C" fn(
        &SyscallTable,
        *const u8,
        usize
    ) -> u32 = unsafe { core::mem::transmute(addr) };
    
    let table = SyscallTable {
        print: sys_print,
        read: sys_read,
    };
    
    loop {
        let res = program(&table, args.as_ptr(), args.len());
        if res == 0 {
            break;
        }
        Timer::after_millis(10).await;
    }
}
