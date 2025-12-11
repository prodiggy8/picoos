#![allow(dead_code)]

use core::mem::size_of;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf32_Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u32,
    pub e_phoff: u32,
    pub e_shoff: u32,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf32_Phdr {
    pub p_type: u32,
    pub p_offset: u32,
    pub p_vaddr: u32,
    pub p_paddr: u32,
    pub p_filesz: u32,
    pub p_memsz: u32,
    pub p_flags: u32,
    pub p_align: u32,
}

pub const PT_LOAD: u32 = 1;


pub unsafe fn load_elf(elf_data: &[u8]) -> Option<extern "C" fn()> {
    if elf_data.len() < size_of::<Elf32_Ehdr>() {
        return None;
    }

    let header_ptr = elf_data.as_ptr() as *const Elf32_Ehdr;
    let header = &*header_ptr;

    // Verify magic: 0x7F 'E' 'L' 'F'
    if header.e_ident[0] != 0x7f || header.e_ident[1] != b'E' || header.e_ident[2] != b'L' || header.e_ident[3] != b'F' {
        return None;
    }

    if header.e_ident[4] != 1 {
        return None;
    }

    let ph_offset = header.e_phoff as usize;
    let ph_count = header.e_phnum as usize;
    let ph_size = header.e_phentsize as usize;

    for i in 0..ph_count {
        // pointer to the current Program Header
        let ph_ptr = elf_data.as_ptr().add(ph_offset + i * ph_size) as *const Elf32_Phdr;
        let ph = &*ph_ptr;

        if ph.p_type == PT_LOAD {
            let dest = ph.p_vaddr as *mut u8;
            let src = elf_data.as_ptr().add(ph.p_offset as usize);
            let filesz = ph.p_filesz as usize;
            let memsz = ph.p_memsz as usize;

            // Copy segment data from ELF to memory
            if filesz > 0 {
                core::ptr::copy_nonoverlapping(src, dest, filesz);
            }

            if memsz > filesz {
                let bss_dest = dest.add(filesz);
                let bss_len = memsz - filesz;
                core::ptr::write_bytes(bss_dest, 0, bss_len);
            }
        }
    }

    let entry = header.e_entry;
    Some(core::mem::transmute(entry))
}
