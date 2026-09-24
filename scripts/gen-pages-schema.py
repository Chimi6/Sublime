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
    return files


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

    types_source = Path("src/io/pages/types.rs").read_text()
    names = re.findall(r'\(\d+, "([^"]+)"\)', types_source)
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

    lines = [
        "//! Message schemas of Apple Pages documents, generated by",
        "//! `scripts/gen-pages-schema.py` from the community's reverse-engineered",
        "//! iWork protobuf definitions. Field numbers and names are facts about the",
        "//! format; only messages reachable from the Pages type registry are kept.",
        "//! Do not edit by hand.",
        "",
        "use crate::io::protobuf::schema::{Field, Kind, Message};",
        "",
        "/// Every message, sorted by name.",
        "pub static MESSAGES: &[Message] = &[",
    ]
    field_count = 0
    for name in order:
        fields = sorted(parser.messages[name], key=lambda field: field[0])
        entries = []
        seen = set()
        for number, field_name, type_name, label, packed, package, scope in fields:
            if number in seen:
                continue
            seen.add(number)
            kind, target = parser.resolve(type_name, package, scope)
            if kind == "message":
                kind_text = "Kind::Reference" if target == "TSP.Reference" else f"Kind::Message({index[target]})"
            else:
                kind_text = f"Kind::{target}"
            repeated = "true" if label == "repeated" else "false"
            entries.append(f'        Field {{ number: {number}, name: "{field_name}", kind: {kind_text}, repeated: {repeated}, packed: {"true" if packed else "false"} }},')
            field_count += 1
        lines.append(f'    Message {{ name: "{name}", fields: &[')
        lines.extend(entries)
        lines.append("    ] },")
    lines.append("];")
    lines.append("")
    lines.append("/// Messages named in the type registry but absent from the schema sources.")
    lines.append(f"pub const UNKNOWN_TYPES: &[&str] = &[{', '.join(f'\"{name}\"' for name in missing)}];")
    lines.append("")
    Path("src/io/pages/schema.rs").write_text("\n".join(lines) + "\n")
    print(f"{len(order)} messages, {field_count} fields, {len(missing)} registry names without a schema")


if __name__ == "__main__":
    main()
