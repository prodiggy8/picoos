use heapless::String;
use crate::syscalls::sys_print;

#[no_mangle]
pub extern "C" fn dummy_program(
    print_fn: extern "C" fn(*const u8, usize),
    args_ptr: *const u8,
    args_len: usize
) {
    // Echo program: prints its arguments
    if args_len > 0 {
        print_fn(args_ptr, args_len);
    }
    
    let newline = "\n";
    print_fn(newline.as_ptr(), newline.len());
}

#[embassy_executor::task(pool_size = 2)]
pub async fn user_task_runner(addr: usize, args: String<64>) {
    // Signature: fn(syscall_print, args_ptr, args_len)
    let program: extern "C" fn(
        extern "C" fn(*const u8, usize),
        *const u8,
        usize
    ) = unsafe { core::mem::transmute(addr) };
    
    program(sys_print, args.as_ptr(), args.len());
}
