//! N64 ROMs between z64, v64, and n64 through the binary. The ROM is made
//! here (a header and three megabytes of pseudo-random words, more than
//! the converter's buffer), and each order is written by index, apart
//! from the converter's own code.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sublime"))
}

fn run(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .env_remove("SUBLIME_LOG")
        .env("NO_COLOR", "1")
        .output()
        .expect("binary runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("utf8 stderr")
}

fn temp_dir(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("sublime-n64-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("temp dir");
    path
}

/// A big-endian ROM: Paper Mario's first header words, a title, and
/// pseudo-random words to `length`.
fn rom(length: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(&[0x80, 0x37, 0x12, 0x40, 0x00, 0x00, 0x00, 0x0f]);
    let mut state: u32 = 0x1234_5678;
    while bytes.len() < length {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        bytes.push((state >> 24) as u8);
    }
    bytes[0x20..0x2b].copy_from_slice(b"SYNTHETIC  ");
    bytes
}

/// `rom` in an order: the source byte for each place in a word.
fn ordered(rom: &[u8], order: &str) -> Vec<u8> {
    let places: [usize; 4] = match order {
        "z64" => [0, 1, 2, 3],
        "v64" => [1, 0, 3, 2],
        "n64" => [3, 2, 1, 0],
        _ => unreachable!(),
    };
    (0..rom.len())
        .map(|index| rom[index - index % 4 + places[index % 4]])
        .collect()
}

const ORDERS: [&str; 3] = ["z64", "v64", "n64"];

#[test]
fn every_order_reaches_every_other() {
    let dir = temp_dir("pairs");
    let rom = rom(3 << 20);
    for from in ORDERS {
        let input = dir.join(format!("in.{from}"));
        fs::write(&input, ordered(&rom, from)).unwrap();
        for to in ORDERS.iter().filter(|to| **to != from) {
            let output = dir.join(format!("{from}-out.{to}"));
            let result = run(&["convert", input.to_str().unwrap(), output.to_str().unwrap()]);
            assert_eq!(
                result.status.code(),
                Some(0),
                "{from} -> {to}: {}",
                stderr(&result)
            );
            assert!(stderr(&result).is_empty(), "{}", stderr(&result));
            assert!(
                fs::read(&output).unwrap() == ordered(&rom, to),
                "{from} -> {to}"
            );
        }
    }
}

#[test]
fn a_rom_is_read_in_the_order_its_header_shows() {
    let dir = temp_dir("named");
    let rom = rom(1 << 16);
    // A byte-swapped ROM saved as .z64, made big-endian: the same name
    // both ways, which only the header can tell apart.
    let input = dir.join("game.z64");
    fs::write(&input, ordered(&rom, "v64")).unwrap();
    let output = dir.join("fixed.z64");
    let result = run(&["convert", input.to_str().unwrap(), output.to_str().unwrap()]);
    assert_eq!(result.status.code(), Some(0), "{}", stderr(&result));
    assert!(stderr(&result).contains("holds v64"), "{}", stderr(&result));
    assert!(fs::read(&output).unwrap() == rom);

    // Named by --from, the header still decides.
    let input = dir.join("game.rom");
    fs::write(&input, ordered(&rom, "n64")).unwrap();
    let output = dir.join("out.v64");
    let result = run(&[
        "convert",
        "--from",
        "z64",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(0), "{}", stderr(&result));
    assert!(stderr(&result).contains("little-endian (n64), not big-endian (z64)"));
    assert!(fs::read(&output).unwrap() == ordered(&rom, "v64"));

    // Batch mode reads each file by its header too.
    let out_dir = dir.join("batch");
    let result = run(&[
        "convert",
        dir.join("game.z64").to_str().unwrap(),
        "--to",
        "n64",
        "--out-dir",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(0), "{}", stderr(&result));
    assert!(fs::read(out_dir.join("game.n64")).unwrap() == ordered(&rom, "n64"));
}

#[test]
fn reads_from_standard_input() {
    let rom = rom(1 << 12);
    let mut child = Command::new(binary())
        .args(["convert", "--from", "v64", "--to", "z64", "-"])
        .env_remove("SUBLIME_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&ordered(&rom, "v64"))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(output.stdout == rom);
}

#[test]
fn a_rom_of_part_words_is_refused_and_nothing_written() {
    let dir = temp_dir("partial");
    let mut rom = rom(1 << 12);
    rom.truncate(rom.len() - 2);
    let input = dir.join("cut.z64");
    fs::write(&input, &rom).unwrap();
    let output = dir.join("cut.v64");
    let result = run(&["convert", input.to_str().unwrap(), output.to_str().unwrap()]);
    assert_eq!(result.status.code(), Some(1));
    assert!(stderr(&result).contains("whole 32-bit words, and this one is 4094 bytes"));
    assert!(!output.exists());
}
