#![no_std]
#![no_main]
#![feature(abi_efiapi)]
#![feature(asm)]

use uefi::prelude::*;
// use uefi::proto::console::gop::FrameBuffer;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::proto::media::file::{File, RegularFile, Directory, FileMode, FileAttribute, FileType, FileInfo};
use uefi::table::boot::{AllocateType, MemoryType};
use uefi::data_types::Align;
use core::mem;
use core::slice::from_raw_parts_mut;
use core::panic::PanicInfo;
use elf_rs::*;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn efi_main(_handle: Handle, system_table: SystemTable<Boot>) -> Status {
    use core::fmt::Write;
  
    writeln!(system_table.stdout(), "Hello, World!").unwrap();

    let bs = system_table.boot_services();

    // Getting memory map
    let mut memorymap_buffer= [0; 4096 * 4];
    let _memory_map_size: usize = bs.memory_map_size();
    writeln!(system_table.stdout(), "memorymap buf: {:p}, memorymap size: {:08x}", memorymap_buffer.as_mut_ptr(), _memory_map_size).unwrap();
    let (_memory_map_key, memory_descriptor_itr) = bs.memory_map(&mut memorymap_buffer).unwrap_success();

    // Opening root directory
    let loaded_image = bs.handle_protocol::<LoadedImage>(_handle).unwrap_success().get();
    let device;
    let mut root_dir: Directory;
    unsafe { device = (*loaded_image).device(); }
    let file_system = bs.handle_protocol::<SimpleFileSystem>(device).unwrap_success().get();
    unsafe { root_dir = (*file_system).open_volume().unwrap_success(); }

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
        for (i, mem_desc) in memory_descriptor_itr.enumerate() {
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

    /* TO GET SIZE OF 'FileInfo' */
    let info_size = kernel_file.get_info::<FileInfo>(&mut []).expect_error("");
    writeln!(system_table.stdout(), "FileInfo size = {:?}", info_size).unwrap();

    const FILEINFO_SIZE: usize = 102; 
    let fileinfo_buf: &mut [u8] = &mut [0u8; FILEINFO_SIZE];

    assert!((fileinfo_buf.as_ptr() as usize) % FileInfo::alignment() == 0);

    let kernel_info: &FileInfo = kernel_file
        .get_info::<FileInfo>(&mut fileinfo_buf[..])
        .unwrap_success();
    let _kernel_file_size: u64 = kernel_info.file_size();
    const KERNEL_BASE_ADDR: usize = 0x100000;
    const PAGE_SIZE: usize = 0x1000;

    // Load kernel to memory pool
    let tmp_kernel_ptr = bs.allocate_pool(
            MemoryType::LOADER_DATA, 
            (_kernel_file_size as usize + 0xfff) / PAGE_SIZE
        ).unwrap_success();

    let tmp_kernel_buf: &mut [u8] = unsafe { from_raw_parts_mut(tmp_kernel_ptr as *mut u8, _kernel_file_size as usize) }; 
    let _kernel_read_size = kernel_file.read(tmp_kernel_buf).unwrap_success();
    assert_eq!(_kernel_read_size, _kernel_file_size as usize);
    kernel_file.close();

    // Reading ELF
    let elf = Elf::from_bytes(&tmp_kernel_buf).unwrap();
    let e = match elf {
        Elf::Elf64(tmp) => tmp,
        _ => panic!("An error occured while reading ELF."),
    };
    let mut kernel_st: u64 = core::u64::MAX;
    let mut kernel_ed: u64 = 0;
    // writeln!(system_table.stdout(), "{:?} {:?}", e, e.header()).unwrap();
    for _header_iter in e.program_header_iter() {
        // writeln!(system_table.stdout(), "{:x?}", _header_iter).unwrap();
        let header = _header_iter.ph;
        if header.ph_type() == ProgramType::LOAD {
            kernel_st = core::cmp::min(kernel_st, header.vaddr());
            kernel_ed = core::cmp::max(kernel_ed, header.vaddr() + header.memsz());
        }
    }#[allow(clippy::empty_loop)]

    writeln!(system_table.stdout(), "Kernel: 0x{:x} ({} bytes) {:p} - {:p}", KERNEL_BASE_ADDR, _kernel_file_size, kernel_st as *const u8, kernel_ed as *const u8).unwrap();

    let kernel_size = kernel_ed as usize - kernel_st as usize;
    let page_count = (kernel_size + PAGE_SIZE - 1) / PAGE_SIZE;
    let page_addr = bs.allocate_pages(
            AllocateType::Address(KERNEL_BASE_ADDR),
            MemoryType::LOADER_DATA,
            page_count
        ).unwrap_success();

    for _header_iter in e.program_header_iter() {
        let ph = _header_iter.ph;
        if ph.ph_type() != ProgramType::LOAD {
            continue;
        }
        let offset = ph.offset() as usize;
        let filesz = ph.filesz() as usize;
        let memsz = ph.memsz() as usize;
        let destination = unsafe {
            from_raw_parts_mut(ph.vaddr() as *mut u8, memsz)
        };
        destination[..filesz].copy_from_slice(&tmp_kernel_buf[offset..offset + filesz]);
        destination[filesz..].fill(0);
    }

    let kernel: &mut [u8] = unsafe { from_raw_parts_mut(page_addr as *mut u8, kernel_size) };

    type EntryPoint = extern "sysv64" fn() -> ();
    let entry = {
        const ENTRY_OFFSET: usize = 24;
        let entry_addr = kernel.as_ptr().wrapping_add(ENTRY_OFFSET) as *const u64;
        unsafe { mem::transmute::<u64, EntryPoint>(entry_addr.read()) }
    };

    // Exit boot service
    let (_systable_runtime, _descriptor_itr) = system_table.exit_boot_services(_handle, &mut memorymap_buffer).unwrap_success();
 
    entry();

    #[allow(clippy::empty_loop)]
    loop {}
    
}
