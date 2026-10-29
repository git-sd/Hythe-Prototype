# hythe-complete

Fast, offline code completion for editors. No language server, no I/O: give
it the text and the cursor, get back ranked suggestions.

```rust
use hythe_complete::{complete, Language, Request};

let text = "fn compute_total() {}\nfn main() { comp }";
let result = complete(&Request {
    language: Language::Rust,
    text,
    cursor: text.len() - 2, // byte offset, just after `comp`
    explicit: false,        // true for Ctrl+Space
    others: &[],            // text of other open files
});
assert_eq!(result.items[0].label, "compute_total");

// Accepting an item replaces `result.start..cursor` with its expansion.
let e = result.items[0].expand("    "); // pass the line's indentation
```

## Where suggestions come from

- **Per-language tables** for Rust, Python, JavaScript, TypeScript, Go, C,
  C++ and Java: keywords, standard-library names, and snippets (`fn`, `for`,
  `iferr`, `sout`, `ifmain`, …).
- **Members** after `.`, `::` or (C/C++) `->`: `x.unw` offers `unwrap`,
  `console.` offers `log`, `std::` offers `string`, `vector`, …
- **Symbols in the open document**, classified as function / type / constant /
  variable from definition keywords, calls and naming, nearest to the cursor first.
- **Symbols from other open documents**, ranked below local ones.
- Everything else falls back to words in the open files.

Matching is case-insensitive prefix first, then word-boundary abbreviations
(`gun` → `getUserName`, `hw` → `hello_world`), then looser subsequences.
Nothing is offered inside strings or comments.

## Snippets

An item's `insert` may contain `$0` (final cursor) and `${name}` placeholders.
`Item::expand` resolves them, indents continuation lines, and returns the
character range to leave selected: the first placeholder, so typing replaces
it, otherwise the `$0` position. Tab-stop chains between placeholders are not
implemented.

## Performance

The document is scanned once per request. A 3.5 MB file with 60,000 functions
completes in well under half a second in the test suite; documents over 2 MB
are scanned only around the cursor.

## Not included

Type-aware completion (which methods does *this* value have?) needs a language
server. The `Request` shape leaves room to merge LSP results in later.
