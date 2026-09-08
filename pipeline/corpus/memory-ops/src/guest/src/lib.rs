#![cfg_attr(feature = "guest", no_std)]
#![allow(unused_assignments, asm_sub_register)]

extern crate alloc;

use alloc::vec::Vec;

/// FIXED, crate-generic harness signature. Do not change it: the driver feeds
/// input FILES and parses one output line, so the shape must stay stable across
/// every issue and crate under test. Output is variable-length on purpose — a
/// fixed-size return could truncate a large result and hide a late divergence.
#[jolt::provable(heap_size = 32768, max_trace_length = 65536)]
fn harness(input: Vec<u8>) -> Vec<u8> {
    // (1) DECODE ---------------------------------------------------------------
    // The lifted code under test (`memory_ops`) takes NO arguments — it exercises
    // RISC-V memory instructions via inline asm on a local buffer. So `input` is
    // unused. Byte layout: no fields consumed (input ignored); DECODE is total.
    let _ = &input;

    // (2) CALL -----------------------------------------------------------------
    // Lifted verbatim from memory-ops `memory_ops()`. Byte-for-byte identical
    // computation: SB/LB/LBU + SH/LH/LHU over an 8-byte stack buffer.
    let (val_lb, val_lbu, val_lh, val_lhu): (i32, u32, i32, u32) = {
        use core::arch::asm;

        let mut data: [u8; 8] = [0; 8];
        unsafe {
            let ptr = data.as_mut_ptr();

            // Store Byte (SB instruction)
            asm!(
                "sb {value}, 0({ptr})",
                ptr = in(reg) ptr,
                value = in(reg) 0x12,
            );

            // Load Byte Signed (LB instruction)
            let mut val_lb: i32 = 0;
            asm!(
                "lb {val}, 0({ptr})",
                ptr = in(reg) ptr,
                val = out(reg) val_lb,
            );

            // Load Byte Unsigned (LBU instruction)
            let mut val_lbu: u32 = 0;
            asm!(
                "lbu {val}, 1({ptr})",
                ptr = in(reg) ptr,
                val = out(reg) val_lbu,
            );

            // Store Halfword (SH instruction)
            asm!(
                "sh {value}, 2({ptr})",
                ptr = in(reg) ptr,
                value = in(reg) 0x3456,
            );

            // Load Halfword Signed (LH instruction)
            let mut val_lh: i32 = 0;
            asm!(
                "lh {val}, 2({ptr})",
                ptr = in(reg) ptr,
                val = out(reg) val_lh,
            );

            // Load Halfword Unsigned (LHU instruction)
            let mut val_lhu: u32 = 0;
            asm!(
                "lhu {val}, 4({ptr})",
                ptr = in(reg) ptr,
                val = out(reg) val_lhu,
            );

            // Return these values so that the load instructions
            // don't get optimized away
            (val_lb, val_lbu, val_lh, val_lhu)
        }
    };

    // (3) ENCODE ---------------------------------------------------------------
    // Output layout: 4 x 4-byte LE = 16 bytes: val_lb(i32), val_lbu(u32),
    // val_lh(i32), val_lhu(u32).
    let mut out = Vec::with_capacity(16);
    out.extend_from_slice(&val_lb.to_le_bytes());
    out.extend_from_slice(&val_lbu.to_le_bytes());
    out.extend_from_slice(&val_lh.to_le_bytes());
    out.extend_from_slice(&val_lhu.to_le_bytes());
    out
}
