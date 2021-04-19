#![no_std]
#![no_main]
#![feature(asm)]

use core::panic::PanicInfo;
#[panic_handler]
fn panic (_info: &PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn kernel_main() {
    loop {
        unsafe { asm!("hlt") }
    }
}
