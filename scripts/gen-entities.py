#!/usr/bin/env python3
"""Regenerates src/io/markdown/entities.rs from the WHATWG entity list.

Usage: curl -fsSL https://html.spec.whatwg.org/entities.json | scripts/gen-entities.py > src/io/markdown/entities.rs

The table is packed: all names in one string and all replacements in
another, each with a u16 offset array, so an entry costs four bytes of
index instead of two fat pointers.
"""
import json
import sys


def rust_str(text):
    out = ""
    for ch in text:
        code = ord(ch)
        if ch == '"':
            out += '\\"'
        elif ch == "\\":
            out += "\\\\"
        elif 0x20 <= code < 0x7F:
            out += ch
        else:
            out += "\\u{%x}" % code
    return '"' + out + '"'


entities = json.load(sys.stdin)
rows = []
for name, value in entities.items():
    if not name.endswith(";"):
        continue
    replacement = "".join(chr(c) for c in value["codepoints"])
    rows.append((name[1:-1], replacement))
rows.sort(key=lambda row: row[0].encode())

names = "".join(row[0] for row in rows)
replacements = "".join(row[1] for row in rows)
name_offsets = [0]
replacement_offsets = [0]
for name, replacement in rows:
    name_offsets.append(name_offsets[-1] + len(name.encode()))
    replacement_offsets.append(replacement_offsets[-1] + len(replacement.encode()))
assert name_offsets[-1] < 65536 and replacement_offsets[-1] < 65536

print("//! HTML named character references, generated from the WHATWG entity list by")
print("//! `scripts/gen-entities.py`. Sorted by name for binary search. Do not edit.")
print()
print("pub const COUNT: usize = %d;" % len(rows))
print()
print("/// Every entity name, concatenated, in sorted order.")
print("pub static NAMES: &str = %s;" % rust_str(names))
print()
print("/// Byte offset of each name in `NAMES`, plus one past the end.")
print("pub static NAME_OFFSETS: [u16; COUNT + 1] = [")
for i in range(0, len(name_offsets), 12):
    print("    " + ", ".join(str(x) for x in name_offsets[i:i + 12]) + ",")
print("];")
print()
print("/// Every replacement, concatenated, in the same order.")
print("pub static REPLACEMENTS: &str = %s;" % rust_str(replacements))
print()
print("/// Byte offset of each replacement in `REPLACEMENTS`, plus one past the end.")
print("pub static REPLACEMENT_OFFSETS: [u16; COUNT + 1] = [")
for i in range(0, len(replacement_offsets), 12):
    print("    " + ", ".join(str(x) for x in replacement_offsets[i:i + 12]) + ",")
print("];")
print()
print("pub fn name(index: usize) -> &'static str {")
print("    let start = usize::from(NAME_OFFSETS[index]);")
print("    let end = usize::from(NAME_OFFSETS[index + 1]);")
print("    &NAMES[start..end]")
print("}")
print()
print("pub fn replacement(index: usize) -> &'static str {")
print("    let start = usize::from(REPLACEMENT_OFFSETS[index]);")
print("    let end = usize::from(REPLACEMENT_OFFSETS[index + 1]);")
print("    &REPLACEMENTS[start..end]")
print("}")
print()
print("/// Binary search by name.")
print("pub fn lookup(query: &str) -> Option<&'static str> {")
print("    let mut low = 0usize;")
print("    let mut high = COUNT;")
print("    while low < high {")
print("        let middle = low + (high - low) / 2;")
print("        match name(middle).cmp(query) {")
print("            std::cmp::Ordering::Less => low = middle + 1,")
print("            std::cmp::Ordering::Greater => high = middle,")
print("            std::cmp::Ordering::Equal => return Some(replacement(middle)),")
print("        }")
print("    }")
print("    None")
print("}")
