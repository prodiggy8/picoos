const DUMMY_ELF: &[u8] = &[
    0x7f, 0x45, 0x4c, 0x46, 0x01, 0x01, 0x01, 0x00, // e_ident
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x02, 0x00,                                     // e_type (ET_EXEC)
    0x28, 0x00,                                     // e_machine (EM_ARM)
    0x01, 0x00, 0x00, 0x00,                         // e_version
    0x01, 0x00, 0x03, 0x20,                         // e_entry (0x20030001) - Thumb bit set
    0x34, 0x00, 0x00, 0x00,                         // e_phoff (52)
    0x00, 0x00, 0x00, 0x00,                         // e_shoff (0)
    0x00, 0x00, 0x00, 0x00,                         // e_flags
    0x34, 0x00,                                     // e_ehsize (52)
    0x20, 0x00,                                     // e_phentsize (32)
    0x01, 0x00,                                     // e_phnum (1)
    0x28, 0x00,                                     // e_shentsize (40)
    0x00, 0x00,                                     // e_shnum (0)
    0x00, 0x00,                                     // e_shstrndx (0)

    0x01, 0x00, 0x00, 0x00,                         // p_type (PT_LOAD)
    0x54, 0x00, 0x00, 0x00,                         // p_offset (84)
    0x00, 0x00, 0x03, 0x20,                         // p_vaddr (0x20030000)
    0x00, 0x00, 0x03, 0x20,                         // p_paddr (0x20030000)
    0x10, 0x00, 0x00, 0x00,                         // p_filesz (16 bytes)
    0x10, 0x00, 0x00, 0x00,                         // p_memsz (16 bytes)
    0x05, 0x00, 0x00, 0x00,                         // p_flags (RX)
    0x04, 0x00, 0x00, 0x00,                         // p_align

    // Code (8 bytes)
    0x09, 0x68,                                     // ldr r1, [r0]
    0x01, 0x31,                                     // adds r1, #1
    0x09, 0x60,                                     // str r1, [r0]
    0x70, 0x47,                                     // bx lr
    0x00, 0x00, 0x00, 0x00,                         // Padding
    0x00, 0x00, 0x00, 0x00                          // Padding
];

pub const ECHO_ELF: &[u8] = &[
    0x7f, 0x45, 0x4c, 0x46, 0x01, 0x01, 0x01, 0x00, // e_ident
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x02, 0x00,                                     // e_type (ET_EXEC)
    0x28, 0x00,                                     // e_machine (EM_ARM)
    0x01, 0x00, 0x00, 0x00,                         // e_version
    0x01, 0x00, 0x03, 0x20,                         // e_entry (0x20030001) - Thumb bit set
    0x34, 0x00, 0x00, 0x00,                         // e_phoff (52)
    0x00, 0x00, 0x00, 0x00,                         // e_shoff (0)
    0x00, 0x00, 0x00, 0x00,                         // e_flags
    0x34, 0x00,                                     // e_ehsize (52)
    0x20, 0x00,                                     // e_phentsize (32)
    0x01, 0x00,                                     // e_phnum (1)
    0x28, 0x00,                                     // e_shentsize (40)
    0x00, 0x00,                                     // e_shnum (0)
    0x00, 0x00,                                     // e_shstrndx (0)

    0x01, 0x00, 0x00, 0x00,                         // p_type (PT_LOAD)
    0x54, 0x00, 0x00, 0x00,                         // p_offset (84)
    0x00, 0x00, 0x03, 0x20,                         // p_vaddr (0x20030000)
    0x00, 0x00, 0x03, 0x20,                         // p_paddr (0x20030000)
    0x10, 0x00, 0x00, 0x00,                         // p_filesz (16 bytes)
    0x10, 0x00, 0x00, 0x00,                         // p_memsz (16 bytes)
    0x05, 0x00, 0x00, 0x00,                         // p_flags (RX)
    0x04, 0x00, 0x00, 0x00,                         // p_align

    // Code (10 bytes)
    0x09, 0x68,                                     // ldr r1, [r0]
    0x01, 0x31,                                     // adds r1, #1
    0x09, 0x60,                                     // str r1, [r0]
    0x01, 0x20,                                     // movs r0, #1
    0x70, 0x47,                                     // bx lr
    0x00, 0x00                                      // Padding
];
