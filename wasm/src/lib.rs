//! Sublime in the browser. This crate is the WebAssembly face of the
//! library: a handful of exported functions that move bytes across the
//! module's linear memory and run the same planner and converters the
//! command line runs. Nothing here touches a file system, a thread, or a
//! clock, and nothing leaves the visitor's machine.
//!
//! The contract, in the host's terms (`sublime.js` implements it):
//!
//! - `alloc(len)` returns a place to write `len` bytes; `dealloc(ptr, len)`
//!   gives it back.
//! - `convert(from, from_len, to, to_len, input, input_len)` converts and
//!   returns a status: 0 converted losslessly, 2 converted with loss (the
//!   message says what), 1 a conversion error, 3 no path between the
//!   formats, 4 an unknown format id. The result is read through
//!   `output_ptr()` and `output_len()`, the message through `message_ptr()`
//!   and `message_len()`.
//! - `formats()` and `paths()` write JSON into the message buffer and
//!   return its length.
//!
//! This is the one place in Sublime with `unsafe`: the lines that turn the
//! host's pointers and lengths into slices, and the one that reclaims a
//! buffer the host was given. Everything past them is safe library code.

use std::cell::RefCell;

use sublime::converter::ConvertOptions;
use sublime::event::{Context, NullSink};
use sublime::format::find_by_id;
use sublime::planner::{self, PlanError, PlanOptions};
use sublime::registry;

thread_local! {
    static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static MESSAGE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Statuses `convert` returns.
const CONVERTED: u32 = 0;
const FAILED: u32 = 1;
const CONVERTED_WITH_LOSS: u32 = 2;
const NO_PATH: u32 = 3;
const UNKNOWN_FORMAT: u32 = 4;

/// Reserves `len` bytes for the host to fill.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len.max(1));
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// Releases a buffer `alloc` returned, of the same `len`.
///
/// # Safety
///
/// `ptr` must have come from `alloc(len)` and must not be released twice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: `alloc` forgot a vector of exactly this capacity; rebuilding
    // it here frees it once.
    let buffer = unsafe { Vec::from_raw_parts(ptr, 0, len.max(1)) };
    drop(buffer);
}

/// Converts `input` from one format id to another. See the module notes
/// for the statuses and where the result and message are read.
///
/// # Safety
///
/// The three pointer and length pairs must describe regions the host
/// wrote into memory from `alloc` and keeps alive for the whole call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert(
    from_ptr: *const u8,
    from_len: usize,
    to_ptr: *const u8,
    to_len: usize,
    input_ptr: *const u8,
    input_len: usize,
) -> u32 {
    // SAFETY: the caller upholds the contract above.
    let (from, to, input) = unsafe {
        (
            std::slice::from_raw_parts(from_ptr, from_len),
            std::slice::from_raw_parts(to_ptr, to_len),
            std::slice::from_raw_parts(input_ptr, input_len),
        )
    };
    run(from, to, input)
}

fn run(from: &[u8], to: &[u8], input: &[u8]) -> u32 {
    set_message("");
    OUTPUT.with(|output| output.borrow_mut().clear());
    let known = registry::all_formats();
    let (Ok(from), Ok(to)) = (std::str::from_utf8(from), std::str::from_utf8(to)) else {
        set_message("format ids must be UTF-8");
        return UNKNOWN_FORMAT;
    };
    let from = match find_by_id(from, &known) {
        Ok(format) => format,
        Err(error) => {
            set_message(&error.to_string());
            return UNKNOWN_FORMAT;
        }
    };
    let to = match find_by_id(to, &known) {
        Ok(format) => format,
        Err(error) => {
            set_message(&error.to_string());
            return UNKNOWN_FORMAT;
        }
    };
    let plan_options = PlanOptions {
        strict: false,
        via: None,
    };
    let plan = match planner::plan(registry::all_converters(), from, to, &plan_options) {
        Ok(plan) => plan,
        Err(error @ PlanError::NoPath { .. }) => {
            set_message(&error.to_string());
            return NO_PATH;
        }
        Err(error) => {
            set_message(&error.to_string());
            return FAILED;
        }
    };
    let options = ConvertOptions::default();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut source: &[u8] = input;
    let mut result = Vec::new();
    let outcome = planner::execute(
        &plan,
        sublime::converter::Input::Stream(&mut source),
        &mut result,
        &mut context,
    );
    if let Err(error) = outcome {
        set_message(&error.to_string());
        return FAILED;
    }
    OUTPUT.with(|output| *output.borrow_mut() = result);
    let notes = plan.loss_descriptions();
    if notes.is_empty() {
        CONVERTED
    } else {
        set_message(&notes.join("; "));
        CONVERTED_WITH_LOSS
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn output_ptr() -> *const u8 {
    OUTPUT.with(|output| output.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn output_len() -> usize {
    OUTPUT.with(|output| output.borrow().len())
}

#[unsafe(no_mangle)]
pub extern "C" fn message_ptr() -> *const u8 {
    MESSAGE.with(|message| message.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn message_len() -> usize {
    MESSAGE.with(|message| message.borrow().len())
}

/// Every format as JSON: `[{"id","name","extensions":[...],"category"}]`,
/// written into the message buffer.
#[unsafe(no_mangle)]
pub extern "C" fn formats() -> usize {
    let mut json = String::from("[");
    for (index, format) in registry::all_formats().iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str("{\"id\":");
        json_string(&mut json, format.id);
        json.push_str(",\"name\":");
        json_string(&mut json, format.display_name);
        json.push_str(",\"extensions\":[");
        for (position, extension) in format.extensions.iter().enumerate() {
            if position > 0 {
                json.push(',');
            }
            json_string(&mut json, extension);
        }
        json.push_str("],\"category\":");
        json_string(&mut json, &format!("{:?}", format.category).to_ascii_lowercase());
        json.push('}');
    }
    json.push(']');
    set_message(&json);
    json.len()
}

/// Every conversion path as JSON: `[{"from","to","fidelity"}]`.
#[unsafe(no_mangle)]
pub extern "C" fn paths() -> usize {
    let formats = registry::all_formats();
    let converters = registry::all_converters();
    let options = PlanOptions {
        strict: false,
        via: None,
    };
    let mut json = String::from("[");
    let mut first = true;
    for from in &formats {
        for to in &formats {
            if from == to {
                continue;
            }
            let Ok(plan) = planner::plan(converters, from, to, &options) else {
                continue;
            };
            if !first {
                json.push(',');
            }
            first = false;
            json.push_str("{\"from\":");
            json_string(&mut json, from.id);
            json.push_str(",\"to\":");
            json_string(&mut json, to.id);
            json.push_str(",\"fidelity\":");
            json_string(&mut json, plan.worst_fidelity().label());
            json.push('}');
        }
    }
    json.push(']');
    set_message(&json);
    json.len()
}

fn set_message(text: &str) {
    MESSAGE.with(|message| {
        let mut message = message.borrow_mut();
        message.clear();
        message.extend_from_slice(text.as_bytes());
    });
}

fn json_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(out, format_args!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}
