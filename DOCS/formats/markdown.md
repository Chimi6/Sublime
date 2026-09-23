# Markdown

What we know about parsing Markdown, and the choices made in `src/io/markdown/`.

## Scope

CommonMark 0.31.2 in full, plus the GitHub Flavored Markdown extensions
(tables, strikethrough, task list items, autolink literals, the raw HTML tag
filter) and footnotes in the form cmark-gfm renders them. All 652 CommonMark
specification examples, all 24 GFM extension examples, and cmark-gfm's own
extension examples are CI tests (`tests/markdown_spec.rs`); any regression
fails the build.

## Design

The parser produces a stream of `Event`s (start and end tags, text, code,
breaks, raw HTML, footnote references, task markers). Renderers consume the
stream and never see Markdown syntax. `io::html` is the first renderer;
Markdown to plain text or JSON is another renderer over the same events, and
a PDF renderer would build a tree from them when it needs layout.

Parsing has two phases, as the specification's appendix describes:

1. **Block structure** (`block.rs`). One pass over the lines maintains the
   stack of open containers, matches continuation markers, tries new block
   starts, and adds the line's text to the tip. Tabs expand to four-column
   stops and a partially consumed tab contributes the remaining spaces.
   Link reference definitions are collected when a paragraph closes, which
   is why the whole document is read before any output.
2. **Inline content** (`inline.rs`). Each block's text becomes a small tree:
   a delimiter stack resolves emphasis, strong, and strikethrough by the
   specification's algorithm, a bracket stack resolves links, images, and
   footnote references, and a post-pass splits text at autolink literals.

Inline parsing happens lazily, one top-level block at a time, so memory is
proportional to the input rather than to the event stream. There are two
ways to consume the events. `Parser` is an iterator, for renderers that
want to pull. `parse_into` pushes every event into an `EventSink` as it is
made, with nothing buffered in between; the HTML writer is such a sink,
which is how the converter runs. A sink receives text that borrows the
source document wherever possible (block lines are ranges into it, and
inline nodes are ranges into the block's text) and is told when an event
borrows temporary storage instead, so a renderer never copies text it can
write straight out. The HTML writer streams its output in 256 KiB chunks.

Block nodes and lines live in flat arenas with `u32` links, and the inline
parser reuses its node, delimiter, and bracket arenas from block to block.

The entity table (2,125 named references) is generated from the WHATWG list
by `scripts/gen-entities.py` into two packed strings with 16-bit offset
arrays, and searched by binary search.

## Corpus policy

- The CommonMark corpus runs with the tag filter and autolink literals
  disabled. Both are GFM extensions that change CommonMark's expected output
  for a handful of examples (raw `<script>` blocks, bare URLs). This matches
  how cmark-gfm tests the base specification.
- The GFM corpus runs with every extension enabled.
- cmark-gfm's `extensions.txt` examples run with every extension enabled,
  except its task list examples: cmark-gfm renders
  `<input type="checkbox" disabled="" />` while the GFM specification renders
  `<input disabled="" type="checkbox">`. We follow the specification.
- An expected output of `<IGNORE>` means the input must not crash the
  parser; the output is not compared.

## Known deviations

- **Unicode punctuation.** The specification classifies delimiter-run
  neighbors using Unicode general categories P and S. The standard library
  has no category lookup and we carry no tables, so a non-ASCII character
  counts as punctuation when it is not alphanumeric, not whitespace, and not
  a control character. That also sweeps in combining marks and format
  characters. No specification example depends on the difference.
- **Case folding of labels.** Link labels are matched after lowercasing with
  the three full-folding cases the specification exercises (`ß` to `ss`,
  long s, final sigma). Other multi-character foldings are not applied.
- **NUL bytes** are replaced with U+FFFD before parsing, per the
  specification's security note.

## Renderers

Four renderers consume the event stream today; each is an `EventSink`
and streams its output.

- **HTML** (`io::html`), the reference renderer. Its output is compared
  byte for byte against the specification corpora.
- **Markdown** (`io::markdown::writer`). Writes events back as Markdown in
  one canonical form: ATX headings (setext only when a level-one or
  level-two heading spans lines), `-` bullets alternating with `*` so
  adjacent lists stay separate, `*` emphasis (with `_` for a run opened
  directly inside another, so `*_a_*` does not become `**a**`), backtick
  fences longer than any run inside, pipe tables with a delimiter row,
  `\` hard breaks, `***` rules, and `[^label]` footnotes. Text is escaped
  wherever it could be read as markup, including at line starts, before a
  link, and around autolink literals. The result parses to the same events:
  every CommonMark, GFM, and cmark-gfm example round-trips in CI (indented
  code directly after a list is written fenced, which is the one place the
  form changes). This writer is what makes Markdown a middle node: any
  format that reaches the events reaches Markdown.
- **Plain text** (`io::text`). Readable text with structure kept: list
  markers and indentation, block quotes indented by two spaces, tables as
  columns padded to equal width with a dashed line under the header, links
  as `text (url)` when the text is not the URL itself, images as their alt
  text, footnotes numbered in order of first reference and listed at the
  end as `[n] body`. Emphasis markers, raw HTML, and image sources are
  dropped, which is why the converter is declared lossy.
- **Events as JSON** (`io::markdown::events_json`), format id
  `markdown-json`. One object per event in one array; the first key names
  the event and the other keys are its attributes. It is also a reader, so
  Markdown -> JSON -> Markdown is a lossless round trip at the event level.

  ```text
  {"start":"paragraph"}                      {"end":"paragraph"}
  {"start":"heading","level":2}              {"end":"heading","level":2}
  {"start":"list","first":null,"tight":true} {"end":"list","ordered":false}
  {"start":"code_block","fenced":true,"info":"rust"}
  {"start":"footnote_definition","label":"1"}
  {"start":"table","alignments":["none","left","center","right"]}
  {"start":"link","destination":"/x","title":""}   (also image)
  {"text":"hello"}   (also code, html, inline_html)
  {"break":"soft"}   {"break":"hard"}   {"rule":true}
  {"footnote_reference":"1"}   {"task_list_marker":true}
  ```

  Tags: paragraph, heading, block_quote, code_block, html_block, list,
  item, footnote_definition, table, table_head, table_row, table_cell,
  emphasis, strong, strikethrough, link, image. The format has no file
  extension of its own; select it with `--to markdown-json` or `--from
  markdown-json`.

### HTML output conventions

The HTML writer reproduces cmark's layout byte for byte: block-level tags are
preceded by a newline if the output does not already end with one, `<li>` is
followed by content directly, tight list items drop their paragraph tags, and
an empty code block renders as `<pre><code></code></pre>`. Footnotes render
as cmark-gfm does: a `<section class="footnotes">` at the end, numbered by
order of first reference, with one back-reference per use.

## Extensions not included

Math, wiki links, description lists, front matter, heading attributes,
superscript and subscript, emoji shortcodes. Each is a vendor extension with
no shared specification; add one when a path needs it, with its own corpus.
