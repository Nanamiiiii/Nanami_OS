#![no_std]
#![no_main]
#![feature(abi_efiapi)]

use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::proto::media::file::{File, RegularFile, Directory, FileMode, FileAttribute, FileType, FileInfo};
use uefi::data_types::Align;
use core::panic::PanicInfo;
use core::fmt::Write;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn efi_main(_handle: Handle, system_table: SystemTable<Boot>) -> Status {
    let std_out = system_table.stdout();
    writeln!(std_out, "Hello, World!").unwrap();

    let bs = system_table.boot_services();
    // Getting memory map
    let memorymap_buffer: &mut [u8] = &mut [0; 4096 * 4];
    let (_memory_map_key, memory_descriptor_itr) = bs.memory_map(memorymap_buffer).unwrap_success();

    // Opening root directory
    let loaded_image = bs.handle_protocol::<LoadedImage>(_handle).unwrap_success().get();
    let device;
    let mut root_dir: Directory;
    unsafe{ device = (*loaded_image).device(); }
    let file_system = bs.handle_protocol::<SimpleFileSystem>(device).unwrap_success().get();
    unsafe{ root_dir = (*file_system).open_volume().unwrap_success(); }

    {
        // to use writeln! macro to RegularFile
        struct RegularFileWriter(RegularFile);
        impl core::fmt::Write for RegularFileWriter {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                self.0
                    .write(s.as_bytes())
                    .map_err(|_| core::fmt::Error)?
                    .unwrap();
                Ok(())
            }
        }

        let memorymap_file_handle = root_dir.open(
            "\\memmap", 
            FileMode::CreateReadWrite, 
            FileAttribute::empty()
        ).unwrap_success();

        let mut memorymap_file = match memorymap_file_handle.into_type().unwrap_success() {
            FileType::Regular(file) => RegularFileWriter(file),
            _ => panic!("Cannot open file!"),
        };
    
        writeln!(memorymap_file, "Index, Type, Type(name), PhysicalStart, NumberOfPages, Attribute").unwrap();

        // Writing memory descriptor
        for (i,mem_desc) in memory_descriptor_itr.enumerate() {
            writeln!(
                memorymap_file, 
                "{}, {:x}, {:?}, {:08x}, {}, {:x}", 
                i, mem_desc.ty.0, mem_desc.ty, mem_desc.phys_start, mem_desc.page_count, mem_desc.att
            ).unwrap();
        }
    }

    
    // Load Kernel
    let kernel_file_handle = root_dir.open(
            "\\kernel.elf",
            FileMode::Read,
            FileAttribute::empty()
        ).unwrap_success(); 
    
    let mut kernel_file = match kernel_file_handle.into_type().unwrap_success() {
        FileType::Regular(file) => file,
        _ => panic!("Connot read kernel!"),
    };

    /* TO GET SIZE OF 'FileInfo'
    let info_size = kernel_file.get_info::<FileInfo>(&mut []).expect_error("");
    writeln!(std_out, "FileInfo size = {:?}", info_size).unwrap();
    */

    const FILEINFO_SIZE: usize = 102; 
    let fileinfo_buf: &mut [u8] = &mut [0u8; FILEINFO_SIZE];

    assert!((fileinfo_buf.as_ptr() as usize) % FileInfo::alignment() == 0);

    let kernel_info: &FileInfo = kernel_file
        .get_info::<FileInfo>(&mut fileinfo_buf[..])
        .unwrap_success();
    let _kernel_file_size = kernel_info.file_size() as usize;
    
    loop {}
    
}
