# YAML

YAML 1.2 with the core schema, read into the value hub
(`src/io/yaml/reader.rs`) and written from it (`src/io/yaml/writer.rs`),
since 0.12.0. Like TOML it shares the hub with JSON, so `yaml -> json`
and `json -> yaml` are one reader and one writer each, and YAML reaches
TOML and CSV through JSON. This is the living map of what the reader and
writer handle, tied to the tests that prove it.

## Status

- `yaml -> json`: shipped, conditional (anchors expanded, tags outside
  the core schema dropped and reported, keys become strings, infinities
  and NaN become strings, a multi-document stream becomes an array).
- `json -> yaml`: shipped, lossless (JSON is a subset of YAML).

Oracles (`tests/yaml_json.rs`, fixtures in `tests/fixtures/yaml`): every
valid fixture reads to the JSON beside it byte for byte; every file under
`invalid/` is refused as malformed; the JSON-first fixtures write the
YAML beside them; every JSON fixture in the repository (YAML, TOML, and
JSON-first) survives JSON to YAML to JSON as the same tree with no loss
reported.

## What the reader handles

| YAML | Value |
|---|---|
| block mappings, `key: value`, keys plain, quoted, or aliased; `? ` explicit keys with block or flow nodes | tables in document order |
| block sequences, `- item`, compact mappings and sequences on the dash line, a sequence at its key's own indentation | arrays |
| flow collections `[a, b]` and `{a: b, c}` over lines, trailing commas, empty values, single-pair mappings inside sequences | arrays, tables |
| plain scalars over several lines (folded), `a:b`, `a#b`, `-x`, `?x`, URLs | core schema: null, booleans, integers (`0o`, `0x`), floats (`.inf`, `.nan`), else strings |
| double-quoted scalars with every escape (`\0 \a \b \t \n \v \f \r \e \" \/ \\ \N \_ \L \P \xXX \uXXXX \UXXXXXXXX`), line folding, escaped line breaks | strings |
| single-quoted scalars, `''`, line folding | strings |
| literal `\|` and folded `>` block scalars with chomping (`-`, `+`) and indentation indicators, more-indented lines, leading and trailing empty lines | strings |
| anchors `&a` and aliases `*a` on any node | the anchored value, copied |
| merge keys `<<: *a` and `<<: [*a, *b]` in block and flow mappings | the mapping's own keys win; merged keys take the merge's position |
| tags `!!str`, `!!int`, `!!float`, `!!bool`, `!!null` on scalars; `!!map`, `!!seq` on collections | the forced type (an unparsable forced scalar is an error) |
| other tags (`!custom`, `!!binary`, `!<verbatim>`) | dropped, the value kept, reported once per tag |
| `%YAML` and `%TAG` directives, `---` and `...` markers, several documents, comments, CRLF, a leading BOM | skipped; documents become one value or an array |

Refused, with the line and column: tabs as indentation, a duplicate key,
a line indented under no node, a mapping key inside a plain scalar (`a:
b` followed by an indented `c: d`), `a: b: c`, a sequence entry where a
mapping key was expected, an unknown alias, a bad escape, an unclosed
quote or flow collection, a directive without a `---` after it, and a
scalar that a `!!int`, `!!float`, or `!!bool` tag cannot parse.

## What the writer does

Block style throughout. Mappings as `key: value`, nested collections on
the following lines indented two more, sequences as `- item` with a
nested mapping compact on the dash line (`- a: 1`), empty collections as
`{}` and `[]`. Keys and strings are plain when the core schema would read
them back as the same string and double-quoted otherwise: anything that
resolves to null, a boolean, or a number, the YAML 1.1 words `yes`, `no`,
`on`, `off`, digits with underscores, leading or trailing whitespace, a
leading indicator, `: `, ` #`, a trailing `:`, tabs and control
characters, and the empty string. Strings holding newlines are written as
literal blocks (`|`, `|-`, `|+` by their trailing newlines) unless they
start with whitespace or hold other control characters. Floats always
carry a fraction or an exponent; `.inf`, `-.inf`, and `.nan` are written
as such.

## Known deviations

- Flow collections as keys (`[a, b]: v`) are refused in block mappings;
  as explicit keys (`? [a, b]`) they are written as their JSON text.
- Plain scalar continuation lines may begin with `- `, as the
  specification allows (`a: 1` followed by an indented `- b` reads as
  the string `1 - b`).
- Anchors are copied, not shared: a document that aliases a large node
  many times costs a copy per alias.
- Only the core schema resolves; `2001-12-14` and `1_000` are strings,
  `010` is the integer 10.

## Performance

`DOCS/benchmarks/yaml-json.md`. The reader cannot stream (aliases and
merge keys need the anchored subtrees in hand), but the tree is an arena
at about three bytes per input byte and aliases copy nodes only; the
input text held beside it is what a sliding window would remove.
