# HTML

Sublime writes HTML from the Markdown event stream
(`src/io/html/writer.rs`, in the form the CommonMark and GFM
specifications expect) and reads HTML into the same stream
(`src/io/html/reader.rs`, since 0.10.0), so a page reaches Markdown,
text, Markdown JSON, and (through the events bridge) Word. This is the
living map of what the reader handles, tied to the tests that prove it.

## Status

- `markdown -> html`, `pages -> html`, `docx -> html`: shipped.
- `html -> markdown`, `text`, `markdown-json`, `docx`: shipped (0.10.0).

Oracles (`tests/html_document.rs`): the HTML our own writer produces for
the CommonMark and GFM corpora reads back to events that render to the
same HTML (580 CommonMark and every GFM example, raw-HTML examples
aside, compared with whitespace and paragraph-edge spaces normalized);
a page of tag soup (uppercase tags, unquoted attributes, unclosed `p`
and `li`, entities, script and style, a table without `thead`) reads to
the Markdown a person expects; footnotes and task lists in cmark-gfm's
markup read back.

## What the reader is

Not a browser's parser. A tokenizer for tags, attributes (quoted or
not), text, comments, doctype, and character references (the full HTML
entity table, numeric references), and an element stack that emits
events as tags go by:

| HTML | Events |
|---|---|
| `p`, text outside any paragraph | paragraph (an implicit one opens for stray text and closes at the next block) |
| `h1` to `h6` | heading |
| `blockquote` | block quote |
| `pre` (with `code class="language-x"`) | fenced code block with its language; text verbatim, one newline after `<pre>` dropped, `br` a newline |
| `ul`, `ol start` | list; loose when the first item holds a `p` of its own |
| `li` | item; a new item closes the previous |
| `hr` | rule |
| `table`, `thead`, `tr`, `th`, `td` | table; the first row is the head, alignment from `align` or `style="text-align"`; events are held until the head row tells the columns |
| `a href title` | link; `a` without `href` is transparent |
| `img src alt title` | image |
| `em`, `i`, `cite`, `var`, `dfn` | emphasis |
| `strong`, `b` | strong |
| `s`, `del`, `strike` | strikethrough |
| `code`, `kbd`, `samp`, `tt` | code span (inner tags ignored; a block tag ends it) |
| `br` | hard break |
| `sup class="footnote-ref"` with `a href="#fn-x"` | footnote reference |
| `section class="footnotes"` with `li id="fn-x"` | footnote definition; back-reference links dropped |
| `input type="checkbox"` | task list marker |
| `script`, `style`, `head`, `title`, `template`, `svg`, `math`, `iframe`, `noscript`, `select`, `textarea`, `canvas`, `video`, `audio`, `object` | skipped with their content |
| `div`, `section`, `article`, `main`, `header`, `footer`, `nav`, `aside`, `figure`, `details`, `dl`, `form`, `body`, `html` | block containers: their blocks flow through, closing one closes them |
| anything else | inline and transparent: text flows through, closing one closes nothing |

Whitespace collapses as in a browser: runs of whitespace are one space,
none at the start of a paragraph or before it ends, none after a hard
break or a task marker, and none between a table's cells. Text is
borrowed from the source in the longest runs whose whitespace is already
single spaces; only decoded references are copied.

Tag soup: an end tag with no open element is ignored; an end tag closes
everything above its element; a block start closes an open paragraph and
its inline formatting; an item with no list gets one; a cell with no row
gets one.

## Known deviations

- Nested tables lose the inner table (its text flows).
- `dl`, `dt`, `dd`, `figure`, `details` have no Markdown form and read as
  paragraphs.
- CSS is ignored: hidden elements are read, `text-align` is read only on
  table cells.
- Character references are decoded only with their `;`, as the writer
  produces them; legacy forms without it stay as written.
- Encodings other than UTF-8 are not sniffed; the input is read as text.
