# TOML

TOML 1.0, read into the value hub (`src/io/toml/reader.rs`) and written
from it (`src/io/toml/writer.rs`), since 0.11.0. The value hub
(`src/value`) is the tree the tree-shaped data formats share: JSON reads
into it through a push sink, so `toml -> json` and `json -> toml` are
one reader and one writer each. This is the living map of what the
reader and writer handle, tied to the tests that prove it.

## Status

- `toml -> json`: shipped, conditional (dates, times, infinities, and
  NaN become strings, reported once per key path).
- `json -> toml`: shipped, conditional (the root must be an object,
  nulls are dropped and reported, integers beyond 64 bits become floats).
- `toml -> yaml`, `toml -> xml`, and back: shipped directly (0.14.0), conditional; `toml -> csv` and `csv -> toml` reach through JSON.

Oracles (`tests/toml_json.rs`, fixtures in `tests/fixtures/toml`): every
valid fixture reads to the JSON beside it byte for byte; every fixture
survives TOML to JSON to TOML to JSON with the same tree (members
sorted, since TOML lays plain keys before sub-tables); every file under
`invalid/` is refused as malformed; the JSON-first fixture writes the
TOML beside it and reports its dropped nulls.

## What the reader handles

| TOML | Value |
|---|---|
| bare, basic-quoted, literal-quoted, and dotted keys, whitespace around dots | table members in document order |
| `[a.b]` headers, implicit parents, a parent defined after its child | tables |
| `[[a]]` arrays of tables, nested `[a.b]` and `[[a.b]]` under the last element | arrays of tables |
| inline tables `{ k = v }`, nested, with dotted keys inside | tables (closed after the brace) |
| arrays over lines, trailing comma, comments inside, mixed types | arrays |
| basic strings with every escape (`\b \t \n \f \r \" \\ \uXXXX \UXXXXXXXX`) | strings |
| multi-line basic strings: first newline trimmed, line-ending backslash, up to two quotes before the closing delimiter | strings |
| literal and multi-line literal strings | strings, verbatim |
| integers: sign, `_` separators, `0x`, `0o`, `0b`, the full 64-bit range | integers |
| floats: fraction, exponent, `_` separators, `inf`, `-inf`, `nan` | floats |
| offset datetimes (`T`, `t`, or a space; `Z` or `+HH:MM`), local datetimes, local dates, local times, fractional seconds | datetimes, kept as written |
| `true`, `false` | booleans |
| comments, CRLF line endings, a leading BOM | skipped |

Refused, with the line and column: a key or table defined twice, a
header on a table that dotted keys defined, dotted keys into a table a
header defined, an inline table or static array extended later, a value
on the same line as another, leading zeros, a bare fraction (`.5`),
control characters in strings or comments, bad escapes, integers past
64 bits, dates and times out of range, and unterminated strings and
arrays.

## What the writer does

A table's plain members first (`key = value`), then each sub-table under
a `[a.b]` header and each array of tables under `[[a.b]]` headers, all in
document order, a blank line before every header. Arrays that are not
entirely tables are written inline (`[1, "two", 2.5]`), and tables inside
them as inline tables (`{ a = 1 }`). Keys are bare when they can be and
basic-quoted otherwise. Strings are basic-quoted with the escapes above;
control characters become `\uXXXX`. Floats always carry a fraction or an
exponent (`1.0`, `1e21`) so they read back as floats. Nulls are dropped
and reported by path (`list[1]`, `sub.gone`).

## Known deviations

- Member order is kept by the reader and by JSON; the TOML writer puts
  plain members before sub-tables because the format requires it.
- Datetimes are validated for shape and range (month, day, hour, minute,
  second, offset) but not for the calendar (February 30 reads).
- TOML 1.1 additions (`\e`, `\xHH`, newlines in inline tables) are not
  accepted.

## Performance

`DOCS/benchmarks/toml-json.md`. The value tree is an arena (32-byte
nodes, strings as spans into one text buffer), about three bytes per
input byte on the dense shape; the reader holds the input text beside
it, which a sliding window like XML's would remove.
