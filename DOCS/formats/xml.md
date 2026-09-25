# XML

XML as a data format: a strict reader that maps a document into the
value hub (`src/io/xml/tree.rs`) and a writer that maps it back
(`src/io/xml/writer.rs`), since 0.13.0. The small lenient pull reader the
Word reader uses (`src/io/xml/reader.rs`) stays as it is. This is the
living map of the mapping and of what the reader accepts, tied to the
tests that prove it.

## Status

- `xml -> json`: shipped, conditional (the mapping below; comments,
  processing instructions, the doctype, and the order of text against
  elements in mixed content are dropped and reported once each).
- `json -> xml`: shipped, conditional (the inverse mapping; a root
  without exactly one member is wrapped in `<root>` with a warning;
  numbers, booleans, and nulls become text).
- TOML, YAML, and CSV reach XML through JSON.

Oracles (`tests/xml_json.rs`, fixtures in `tests/fixtures/xml`): every
well-formed fixture reads to the JSON beside it byte for byte and
survives XML to JSON to XML to JSON as the same JSON; every file under
`invalid/` is refused as malformed; the JSON-first fixtures write the
XML beside them, or are refused where the mapping has no answer (a key
that is not an XML name, an attribute that is not a scalar).

## The mapping

The convention xmltodict and quick-xml's serde support share, so a JSON
document made here reads elsewhere.

| XML | JSON |
|---|---|
| the document | `{"root": node}` |
| an element with attributes or child elements | an object: attributes first as `"@name": "text"`, then children in document order |
| an element with only text | the text as a string |
| an element with nothing | `null` |
| text beside attributes or children | `"#text": "..."` after the children; runs of text between children are trimmed and joined with one space |
| child elements sharing a name | one array at the first one's position |
| CDATA, entity and character references | text, decoded |
| namespaces | names as written (`ns:tag`, `@xmlns:ns`) |
| every attribute and text value | a string, never a number or boolean |

Surrounding whitespace of text is trimmed; whitespace-only text between
elements is dropped; line endings become `\n`.

Writing runs the table backwards: the root object's single member is
the root element, `@` members attributes, `#text` the text (written
first, on its own line, when children follow), arrays repeated elements
(nested arrays flatten into more repeats), scalars text with `&`, `<`,
`>`, and `"` escaped. The output is pretty printed two spaces per level
with leaf elements on one line and an `<?xml ...?>` declaration.

## What the reader accepts and refuses

Accepts: the XML declaration, a doctype with an internal subset (skipped),
comments and processing instructions anywhere (skipped), CDATA, the five
predefined entities and decimal and hexadecimal character references,
single- and double-quoted attributes, self-closing tags, namespaced
names, CRLF, a leading BOM, any non-ASCII name character.

Refuses, with the line and column: a mismatched or missing end tag, more
than one root or content after it, no root, an undefined entity, an
unquoted attribute, an attribute given twice, `<` in an attribute value
or in text, `]]>` in text, `--` inside a comment, and an unterminated
comment, CDATA section, instruction, doctype, or tag. Names are checked
by byte class (ASCII letters, digits, `_`, `:`, `-`, `.`, and any
non-ASCII byte), not against the Unicode name tables.

## Known deviations

- Mixed content loses order: `<p>a <b>x</b> z</p>` reads as
  `{"b": "x", "#text": "a z"}`, and writes back with the text first.
- Attribute order is kept but attributes always come before children in
  the object, whatever the source order of text.
- No type inference: `<n>3</n>` is `"3"`. An opt-in inference pass is a
  roadmap item shared with CSV.
- Entities declared in the internal subset are not expanded; a reference
  to one is refused as undefined.

## Performance

`DOCS/benchmarks/xml-json.md`. The reader never holds the input: it
pulls the file through a 256 KiB window and scans text, names, and
whitespace a word at a time, so the value tree is the only cost per
byte, and the arena-backed tree in `STATE.md` Tech Debt is the lever
shared with TOML and YAML.
