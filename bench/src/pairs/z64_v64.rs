//! N64 ROM byte orders (z64, v64, n64): ours against `n64romconvert` 1.1
//! (the Rust converter), with ucon64 and `dd conv=swab` run by
//! `bench/pairs/z64-v64.sh`. `gen` writes a cartridge-sized big-endian ROM
//! of pseudo-random words.

use std::fs;

use sublime::converters::n64::{N64_TO_Z64, V64_TO_N64, V64_TO_Z64, Z64_TO_N64, Z64_TO_V64};

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen", [bytes, output]) => generate(bytes, output),
        ("ours", [from_to, input, output]) => {
            let converter: &dyn sublime::converter::Converter = match from_to.as_str() {
                "z64-v64" => &Z64_TO_V64,
                "v64-z64" => &V64_TO_Z64,
                "z64-n64" => &Z64_TO_N64,
                "n64-z64" => &N64_TO_Z64,
                "v64-n64" => &V64_TO_N64,
                other => return Err(format!("unknown direction {other}")),
            };
            run_ours(converter, input, output)
        }
        ("crate", [from_to, input, output]) => {
            match from_to.as_str() {
                "z64-v64" | "v64-z64" => n64romconvert::byte_swap(input, output),
                "z64-n64" | "n64-z64" => n64romconvert::endian_swap(input, output),
                "v64-n64" => n64romconvert::byte_endian_swap(input, output),
                other => return Err(format!("unknown direction {other}")),
            }
            Ok(())
        }
        _ => Err("z64-v64 modes: gen <bytes> <out> | ours|crate <from-to> <in> <out>".to_string()),
    }
}

/// A big-endian ROM: Paper Mario's first header word and pseudo-random
/// words to `bytes`.
fn generate(bytes: &str, output: &str) -> Result<(), String> {
    let length: usize = bytes.parse().map_err(|_| "bytes is a number")?;
    let mut rom = Vec::with_capacity(length);
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    while rom.len() < length {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        rom.extend_from_slice(&state.to_le_bytes());
    }
    rom.truncate(length);
    rom[..4].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
    fs::write(output, rom).map_err(|error| error.to_string())
}
