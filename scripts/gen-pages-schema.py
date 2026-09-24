#!/usr/bin/env python3
"""Generates src/io/pages/schema.rs: the message schemas Pages documents use.

The schemas are the community's reverse-engineered protobuf definitions
for iWork (field numbers and names are facts about the format; only the
subset Pages documents contain is kept). Sources, fetched on first run
into a cache directory:

  https://github.com/masaccio/numbers-parser  src/protos/*.proto   (shared TS* packages)
  https://github.com/orcastor/iwork-converter proto/TPArchives.proto (Pages TP package)

Roots are every message named in src/io/pages/types.rs except undo command
archives, which never appear in saved documents; every message reachable
from a root through message-typed fields is included.

Usage: scripts/gen-pages-schema.py [cache-dir]
"""

import re
import subprocess
import sys
from pathlib import Path

NUMBERS_PARSER = "https://raw.githubusercontent.com/masaccio/numbers-parser/main/src/protos/"
SHARED = [
    "TSPMessages.proto", "TSPArchiveMessages.proto", "TSPDatabaseMessages.proto", "TSKArchives.proto",
    "TSSArchives.proto", "TSDArchives.proto", "TSWPArchives.proto", "TSTArchives.proto",
    "TSTStylePropertyArchiving.proto", "TSCEArchives.proto", "TSCHArchives.proto",
    "TSCHArchives_Common.proto", "TSCHArchives_GEN.proto", "TSCH3DArchives.proto",
    "TSCHPreUFFArchives.proto", "TSAArchives.proto", "TSCKArchives.proto",
]
PAGES = "https://raw.githubusercontent.com/orcastor/iwork-converter/master/proto/TPArchives.proto"
# Type registries: message type id -> message name, as the apps register them.
REGISTRIES = {
    "registry-pages.json": "https://raw.githubusercontent.com/dunhamsteve/iwork/master/codegen/Pages.json",
    "registry-common.json": "https://raw.githubusercontent.com/dunhamsteve/iwork/master/codegen/Common.json",
    "registry-numbers.py": "https://raw.githubusercontent.com/masaccio/numbers-parser/main/src/numbers_parser/generated/mapping.py",
}
# Ids the registries name wrongly or not at all, settled against the fixtures.
REGISTRY_OVERRIDES = {10143: "TP.SectionTemplateArchive", 10016: "TP.UserDefinedGuideMapArchive"}

SCALARS = {
    "int32": "Int", "int64": "Int", "sint32": "Sint", "sint64": "Sint",
    "uint32": "Uint", "uint64": "Uint", "bool": "Bool",
    "fixed32": "Fixed32", "sfixed32": "Fixed32", "fixed64": "Fixed64", "sfixed64": "Fixed64",
    "float": "Float", "double": "Double", "string": "String", "bytes": "Bytes",
}


def fetch(cache):
    cache.mkdir(parents=True, exist_ok=True)
    files = []
    for name in SHARED:
        path = cache / name
        if not path.exists():
            subprocess.run(["curl", "-sSf", "-L", NUMBERS_PARSER + name, "-o", str(path)], check=True)
        files.append(path)
    path = cache / "TPArchives.proto"
    if not path.exists():
        subprocess.run(["curl", "-sSf", "-L", PAGES, "-o", str(path)], check=True)
    files.append(path)
    for name, url in REGISTRIES.items():
        path = cache / name
        if not path.exists():
            subprocess.run(["curl", "-sSf", "-L", url, "-o", str(path)], check=True)
    return files


def registry(cache):
    """Pages type ids to message names: the Pages registry wins, then the
    shared one, then Numbers' for shared packages it alone lists."""
    import json
    pages = json.loads((cache / "registry-pages.json").read_text())
    common = json.loads((cache / "registry-common.json").read_text())
    numbers = dict(re.findall(r'"(\d+)":\s*"([^"]+)"', (cache / "registry-numbers.py").read_text()))
    table = {}
    for key, value in numbers.items():
        if not (10000 <= int(key) < 11000) and not value.startswith(("TN.", "KN.")):
            table[int(key)] = value
    for key, value in common.items():
        table[int(key)] = value
    for key, value in pages.items():
        table[int(key)] = value
    for key, value in numbers.items():
        if 10000 <= int(key) < 11000 and int(key) not in table and not value.startswith("TN."):
            table[int(key)] = value
    table.update(REGISTRY_OVERRIDES)
    return table


def tokenize(text):
    text = re.sub(r"//[^\n]*", "", text)
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.findall(r'"[^"]*"|[A-Za-z_.][A-Za-z0-9_.]*|-?\d+|[{}=;\[\],]', text)


class Parser:
    def __init__(self):
        self.messages = {}   # full name -> list of (number, name, type, label, packed)
        self.enums = set()
        self.extends = []    # (target type, scope package, fields)

    def parse(self, text):
        tokens = tokenize(text)
        self.tokens = tokens
        self.pos = 0
        package = ""
        scope = []
        while self.pos < len(tokens):
            token = tokens[self.pos]
            if token == "package":
                package = tokens[self.pos + 1]
                self.pos += 3
            elif token in ("import", "syntax", "option"):
                while tokens[self.pos] != ";":
                    self.pos += 1
                self.pos += 1
            elif token == "message":
                self.message(package, scope)
            elif token == "enum":
                self.enum(package, scope)
            elif token == "extend":
                self.extend(package, scope)
            else:
                self.pos += 1

    def qualified(self, package, scope, name):
        return ".".join([package] + scope + [name])

    def message(self, package, scope):
        name = self.tokens[self.pos + 1]
        full = self.qualified(package, scope, name)
        self.pos += 3  # message Name {
        fields = self.messages.setdefault(full, [])
        scope = scope + [name]
        while self.tokens[self.pos] != "}":
            token = self.tokens[self.pos]
            if token == "message":
                self.message(package, scope)
            elif token == "enum":
                self.enum(package, scope)
            elif token == "extend":
                self.extend(package, scope)
            elif token in ("optional", "required", "repeated"):
                fields.append(self.field(package, scope))
            elif token in ("reserved", "option", "extensions"):
                while self.tokens[self.pos] != ";":
                    self.pos += 1
                self.pos += 1
            else:
                self.pos += 1
        self.pos += 1

    def enum(self, package, scope):
        name = self.tokens[self.pos + 1]
        self.enums.add(self.qualified(package, scope, name))
        self.pos += 3
        depth = 1
        while depth > 0:
            token = self.tokens[self.pos]
            if token == "{":
                depth += 1
            elif token == "}":
                depth -= 1
            self.pos += 1

    def extend(self, package, scope):
        target = self.tokens[self.pos + 1]
        self.pos += 3
        fields = []
        while self.tokens[self.pos] != "}":
            if self.tokens[self.pos] in ("optional", "required", "repeated"):
                fields.append(self.field(package, scope))
            else:
                self.pos += 1
        self.pos += 1
        self.extends.append((target, package, scope, fields))

    def field(self, package, scope):
        label = self.tokens[self.pos]
        type_name = self.tokens[self.pos + 1]
        name = self.tokens[self.pos + 2]
        number = int(self.tokens[self.pos + 4])
        self.pos += 5
        packed = False
        if self.tokens[self.pos] == "[":
            while self.tokens[self.pos] != "]":
                if self.tokens[self.pos] == "packed" and self.tokens[self.pos + 2] == "true":
                    packed = True
                self.pos += 1
            self.pos += 1
        self.pos += 1  # ;
        return (number, name, type_name, label, packed, package, scope)

    def resolve(self, type_name, package, scope):
        if type_name in SCALARS:
            return ("scalar", SCALARS[type_name])
        if type_name.startswith("."):
            full = type_name[1:]
            candidates = [full]
        else:
            candidates = []
            for depth in range(len(scope), -1, -1):
                candidates.append(".".join([package] + scope[:depth] + [type_name]))
            candidates.append(type_name)
        for candidate in candidates:
            if candidate in self.messages:
                return ("message", candidate)
            if candidate in self.enums:
                return ("scalar", "Enum")
        # Enums declared in files outside the set decode as varints anyway.
        print(f"warning: {type_name} unresolved in {package}.{'.'.join(scope)}; treated as an enum", file=sys.stderr)
        return ("scalar", "Enum")


def main():
    cache = Path(sys.argv[1] if len(sys.argv) > 1 else "temp/iwork-protos")
    files = fetch(cache)
    parser = Parser()
    for path in files:
        parser.parse(path.read_text())
    for target, package, scope, fields in parser.extends:
        if target.startswith(".google."):
            continue  # custom option declarations, not document data
        kind, full = parser.resolve(target, package, scope)
        parser.messages[full].extend(fields)

    types = registry(cache)
    names = list(types.values())
    roots = [name for name in names if name in parser.messages and "Command" not in name]
    # The object stream's own headers, decoded with the same machinery.
    roots += ["TSP.ArchiveInfo", "TSP.MessageInfo"]
    missing = sorted(set(name for name in names if name not in parser.messages and "Command" not in name))

    included = {}
    order = []
    stack = list(roots)
    while stack:
        name = stack.pop()
        if name in included:
            continue
        included[name] = True
        order.append(name)
        for number, field_name, type_name, label, packed, package, scope in parser.messages[name]:
            kind, target = parser.resolve(type_name, package, scope)
            if kind == "message" and target != "TSP.Reference" and target not in included:
                stack.append(target)
    order.sort()
    index = {name: position for position, name in enumerate(order)}

    # Pack: one deduplicated name blob, fixed-size records.
    kind_codes = {"Int": 0, "Uint": 1, "Sint": 2, "Bool": 3, "Enum": 4, "Fixed32": 5, "Fixed64": 6,
                  "Float": 7, "Double": 8, "String": 9, "Bytes": 10, "Reference": 11}
    blob = ""
    offsets = {}

    def intern(name):
        nonlocal blob
        if name not in offsets:
            if len(name) > 255:
                raise ValueError(f"name too long: {name}")
            offsets[name] = len(blob)
            blob += name
        return offsets[name], len(name)

    message_records = []
    field_records = []
    field_count = 0
    for name in order:
        fields = sorted(parser.messages[name], key=lambda field: field[0])
        seen = set()
        name_counts = {}
        for number, field_name, *_ in fields:
            name_counts[field_name] = name_counts.get(field_name, 0) + 1
        start = len(field_records)
        for number, field_name, type_name, label, packed, package, scope in fields:
            if number in seen:
                continue
            seen.add(number)
            if name_counts[field_name] > 1:
                field_name = f"{field_name}_{number}"
            kind, target = parser.resolve(type_name, package, scope)
            if kind == "message" and target != "TSP.Reference":
                code, message_index = 12, index[target]
            elif kind == "message":
                code, message_index = kind_codes["Reference"], 0
            else:
                code, message_index = kind_codes[target], 0
            flags = (1 if label == "repeated" else 0) | (2 if packed else 0)
            offset, length = intern(field_name)
            if number > 0xFFFF:
                raise ValueError(f"field number {number} does not fit in 16 bits")
            field_records.append((number, offset, length, code, flags, message_index))
            field_count += 1
        offset, length = intern(name)
        message_records.append((offset, length, start, len(field_records) - start))

    lines = [
        "//! Message schemas of Apple Pages documents, generated by",
        "//! `scripts/gen-pages-schema.py` from the community's reverse-engineered",
        "//! iWork protobuf definitions. Field numbers and names are facts about the",
        "//! format; only messages reachable from the Pages type registry are kept.",
        "//! Do not edit by hand.",
        "",
        "use crate::io::protobuf::schema::{FieldRecord, MessageRecord, Schema};",
        "",
        "pub static SCHEMA: Schema = Schema {",
        "    names: NAMES,",
        "    messages: MESSAGES,",
        "    fields: FIELDS,",
        "};",
        "",
        "/// Message names in table order, for reference: " + ", ".join(order[:8]) + ", ...",
        f"const NAMES: &str = {blob!r};".replace("'", '"') if '"' not in blob else None,
        "",
        "/// Sorted by name: (name offset, name length, first field, field count).",
        "const MESSAGES: &[MessageRecord] = &[",
    ]
    if lines[-4] is None:
        raise ValueError("a name contains a double quote")
    for offset, length, start, count in message_records:
        lines.append(f"    MessageRecord {{ name_offset: {offset}, name_length: {length}, fields_start: {start}, fields_length: {count} }},")
    lines.append("];")
    lines.append("")
    lines.append("/// Per message, sorted by number: (number, name offset, name length, kind, flags, nested message).")
    lines.append("const FIELDS: &[FieldRecord] = &[")
    for number, offset, length, code, flags, message_index in field_records:
        lines.append(f"    FieldRecord {{ number: {number}, name_offset: {offset}, name_length: {length}, kind: {code}, flags: {flags}, message_index: {message_index} }},")
    lines.append("];")
    lines.append("")
    lines.append("/// Messages named in the type registry but absent from the schema sources.")
    lines.append(f"pub const UNKNOWN_TYPES: &[&str] = &[{', '.join(f'\"{name}\"' for name in missing)}];")
    lines.append("")
    Path("src/io/pages/schema.rs").write_text("\n".join(lines) + "\n")

    type_lines = [
        "//! Message type ids as Pages registers them, mapped to the schema's",
        "//! messages. Generated by `scripts/gen-pages-schema.py` from the published",
        "//! registries; ids whose message has no schema keep their name here.",
        "//! Do not edit by hand.",
        "",
        "use super::schema::SCHEMA;",
        "",
        "/// Name of a message type, if known.",
        "pub fn type_name(message_type: u32) -> Option<&'static str> {",
        "    if let Some(index) = schema_index(message_type) {",
        "        return SCHEMA.message_at(index).map(|message| message.name());",
        "    }",
        "    let index = NAMED_ONLY.binary_search_by_key(&message_type, |entry| entry.0).ok()?;",
        "    Some(NAMED_ONLY[index].1)",
        "}",
        "",
        "/// Index of the type's message in the schema, if it has one.",
        "pub fn schema_index(message_type: u32) -> Option<u16> {",
        "    let index = TYPES.binary_search_by_key(&message_type, |entry| entry.0).ok()?;",
        "    Some(TYPES[index].1)",
        "}",
        "",
        "/// (type id, schema message index), sorted by id.",
        "const TYPES: &[(u32, u16)] = &[",
    ]
    named_only = []
    for type_id in sorted(types):
        name = types[type_id]
        if name in index:
            type_lines.append(f"    ({type_id}, {index[name]}),")
        else:
            named_only.append((type_id, name))
    type_lines.append("];")
    type_lines.append("")
    type_lines.append("/// Types without a schema, by name.")
    type_lines.append("const NAMED_ONLY: &[(u32, &str)] = &[")
    for type_id, name in named_only:
        type_lines.append(f'    ({type_id}, "{name}"),')
    type_lines.append("];")
    type_lines.append("")
    type_lines.append("#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn names_resolve_through_the_schema() {\n        assert!(TYPES.windows(2).all(|pair| pair[0].0 < pair[1].0));\n        assert_eq!(type_name(2001), Some(\"TSWP.StorageArchive\"));\n        assert_eq!(type_name(10000), Some(\"TP.DocumentArchive\"));\n        assert_eq!(type_name(999_999), None);\n    }\n}")
    Path("src/io/pages/types.rs").write_text("\n".join(type_lines) + "\n")
    print(f"{len(order)} messages, {field_count} fields, {len(blob)} name bytes, {len(missing)} registry names without a schema")


if __name__ == "__main__":
    main()
