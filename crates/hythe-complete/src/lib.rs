//! Fast, offline code completion.
//!
//! No language server and no I/O: [`complete`] looks at the text around the
//! cursor and returns ranked suggestions from four sources:
//!
//! * **keywords, standard-library names and snippets** tuned per language
//!   (Rust, Python, JavaScript, TypeScript, Go, C, C++, Java),
//! * **members** that commonly follow `.` or `::`,
//! * **symbols found in the open document**, classified as functions, types,
//!   constants and variables, nearest to the cursor first,
//! * **symbols from other open documents**.
//!
//! Anything else falls back to words from the open files.

mod lang;
mod lang_extra;
mod rank;
mod scan;

use std::collections::{HashMap, HashSet};

pub use lang::Language;

/// What a suggestion is, used for ordering and for a label in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Function,
    Type,
    Variable,
    Constant,
    Module,
    Keyword,
    Snippet,
    /// Standard-library name.
    Builtin,
    /// Method or property that follows a `.` or `::`.
    Member,
}

impl Kind {
    /// One-letter tag for compact lists.
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Function => "ƒ",
            Kind::Type => "T",
            Kind::Variable => "v",
            Kind::Constant => "c",
            Kind::Module => "m",
            Kind::Keyword => "k",
            Kind::Snippet => "⌘",
            Kind::Builtin => "b",
            Kind::Member => "·",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Shown in the list and matched against what was typed.
    pub label: String,
    /// What gets inserted. May contain `$0` (final cursor) and `${name}`
    /// placeholders; use [`Item::expand`].
    pub insert: String,
    pub kind: Kind,
    /// Short description, e.g. `macro` or `Iterator`.
    pub detail: String,
}

/// An item's text ready to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    /// Character range to leave selected afterwards: the first `${placeholder}`
    /// if there is one (so typing replaces it), else the `$0` position as an
    /// empty range, else the end of the text.
    pub select: (usize, usize),
}

impl Item {
    /// Resolve placeholders. Every newline in the snippet is followed by `indent`
    /// so multi-line snippets line up with the line they're inserted on.
    pub fn expand(&self, indent: &str) -> Expanded {
        let mut text = String::new();
        let mut chars = 0usize;
        let mut first_placeholder: Option<(usize, usize)> = None;
        let mut cursor0: Option<usize> = None;

        let mut it = self.insert.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '$' if it.peek() == Some(&'0') => {
                    it.next();
                    cursor0.get_or_insert(chars);
                }
                '$' if it.peek() == Some(&'{') => {
                    it.next();
                    let mut name = String::new();
                    for n in it.by_ref() {
                        if n == '}' {
                            break;
                        }
                        name.push(n);
                    }
                    let len = name.chars().count();
                    first_placeholder.get_or_insert((chars, chars + len));
                    text.push_str(&name);
                    chars += len;
                }
                '\n' => {
                    text.push('\n');
                    text.push_str(indent);
                    chars += 1 + indent.chars().count();
                }
                _ => {
                    text.push(c);
                    chars += 1;
                }
            }
        }
        let select = first_placeholder.or(cursor0.map(|p| (p, p))).unwrap_or((chars, chars));
        Expanded { text, select }
    }
}

pub struct Request<'a> {
    pub language: Language,
    /// Full text of the document being edited.
    pub text: &'a str,
    /// Cursor position as a byte offset into `text`.
    pub cursor: usize,
    /// The user asked for completions (e.g. Ctrl+Space), so offer them even
    /// with nothing typed yet.
    pub explicit: bool,
    /// Text of other open documents, for cross-file symbols.
    pub others: &'a [&'a str],
}

#[derive(Debug, Clone, Default)]
pub struct Completions {
    /// Byte offset where the typed prefix begins; accepting an item replaces
    /// `start..cursor`.
    pub start: usize,
    pub items: Vec<Item>,
}

const MAX_ITEMS: usize = 40;
/// Documents larger than this are scanned only around the cursor.
const WINDOW: usize = 1 << 20;

struct Agg {
    kind: Kind,
    certain: bool,
    count: u32,
    members: u32,
    /// Bytes from the nearest occurrence to the cursor.
    dist: usize,
    other: bool,
}

pub fn complete(req: &Request) -> Completions {
    let text = req.text;
    let mut cursor = req.cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let lang = req.language;
    let d = lang.data();

    // The identifier being typed.
    let mut start = cursor;
    for (i, c) in text[..cursor].char_indices().rev() {
        if scan::is_ident_char(c, d.dollar_idents, lang.dash_idents()) {
            start = i;
        } else {
            break;
        }
    }
    let prefix = &text[start..cursor];
    let none = Completions { start, items: Vec::new() };
    if prefix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return none;
    }

    // After `.`, `::` or (C/C++) `->` only members make sense.
    let before = &text[..start];
    let member_ctx = before.ends_with('.')
        || (d.scope_operator && before.ends_with("::"))
        || (d.preprocessor && before.ends_with("->"));
    if prefix.is_empty() && !req.explicit && !member_ctx {
        return none;
    }

    let line_start = text[..cursor].rfind('\n').map_or(0, |i| i + 1);
    if in_string_or_comment(&text[line_start..cursor], lang) {
        return none;
    }

    // Look at the whole document, or a window around the cursor for huge ones.
    let (view, offset) = window(text, cursor);
    let cursor_in_view = cursor - offset;

    let mut words: HashMap<&str, Agg> = HashMap::new();
    scan::scan(view, lang, |w| {
        // Not the word being typed.
        if w.start <= cursor_in_view && cursor_in_view <= w.end {
            return;
        }
        if w.text.chars().count() < 2 || d.keywords.contains(&w.text) {
            return;
        }
        let dist = w.start.abs_diff(cursor_in_view);
        let a = words.entry(w.text).or_insert(Agg {
            kind: w.kind,
            certain: w.certain,
            count: 0,
            members: 0,
            dist,
            other: false,
        });
        a.count += 1;
        a.members += w.member as u32;
        a.dist = a.dist.min(dist);
        if w.certain && !a.certain {
            a.kind = w.kind;
            a.certain = true;
        }
    });
    for other in req.others {
        scan::scan(other, lang, |w| {
            if w.text.chars().count() < 2 || d.keywords.contains(&w.text) {
                return;
            }
            let a = words.entry(w.text).or_insert(Agg {
                kind: w.kind,
                certain: w.certain,
                count: 0,
                members: 0,
                dist: usize::MAX,
                other: true,
            });
            if a.other {
                a.count += 1;
                a.members += w.member as u32;
                if w.certain && !a.certain {
                    a.kind = w.kind;
                    a.certain = true;
                }
            }
        });
    }

    // (item, rank group, proximity, frequency)
    let mut cands: Vec<(Item, u32, usize, u32)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |item: Item, group: u32, dist: usize, count: u32, cands: &mut Vec<_>| {
        if seen.insert(item.label.clone()) {
            cands.push((item, group, dist, count));
        }
    };
    let entry = |e: &lang::Entry, kind: Kind| Item {
        label: e.0.to_string(),
        insert: if e.1.is_empty() { e.0.to_string() } else { e.1.to_string() },
        kind,
        detail: e.2.to_string(),
    };

    if member_ctx {
        // `std::` and `console.` offer the library names written under that
        // qualifier, minus the qualifier itself.
        let sep = if before.ends_with("::") { "::" } else if before.ends_with("->") { "->" } else { "." };
        let qualifier = qualifier_before(&before[..before.len() - sep.len()]);
        if !qualifier.is_empty() && sep != "->" {
            let qualified = format!("{qualifier}{sep}");
            for e in d.builtins {
                if let Some(rest) = e.0.strip_prefix(&qualified) {
                    if rest.is_empty() {
                        continue;
                    }
                    let insert = if e.1.is_empty() { rest } else { e.1.strip_prefix(&qualified).unwrap_or(e.1) };
                    let item = Item {
                        label: rest.to_string(),
                        insert: insert.to_string(),
                        kind: Kind::Builtin,
                        detail: e.2.to_string(),
                    };
                    push(item, 1, 0, 0, &mut cands);
                }
            }
        }
        for e in d.members {
            push(entry(e, Kind::Member), 1, 0, 0, &mut cands);
        }
        for (name, a) in &words {
            if a.members > 0 {
                let item = Item {
                    label: name.to_string(),
                    insert: name.to_string(),
                    kind: if a.kind == Kind::Variable { Kind::Member } else { a.kind },
                    detail: String::new(),
                };
                push(item, if a.other { 4 } else { 0 }, a.dist, a.count, &mut cands);
            }
        }
    } else {
        for s in d.snippets {
            let item = Item {
                label: s.0.to_string(),
                insert: s.2.to_string(),
                kind: Kind::Snippet,
                detail: s.1.to_string(),
            };
            push(item, 1, 0, 0, &mut cands);
        }
        for e in d.builtins {
            push(entry(e, Kind::Builtin), 2, 0, 0, &mut cands);
        }
        for kw in d.keywords {
            let item = Item {
                label: kw.to_string(),
                insert: kw.to_string(),
                kind: Kind::Keyword,
                detail: String::new(),
            };
            push(item, 3, 0, 0, &mut cands);
        }
        for (name, a) in &words {
            let item = Item {
                label: name.to_string(),
                insert: name.to_string(),
                kind: a.kind,
                detail: String::new(),
            };
            let group = match (a.other, a.certain) {
                (false, true) => 0,
                (false, false) => 2,
                (true, _) => 4,
            };
            push(item, group, a.dist, a.count, &mut cands);
        }
    }

    // Score against what was typed, matching qualified names (`std::string`,
    // `console.log`) on their last segment too.
    let mut scored: Vec<(u32, u32, usize, u32, usize, Item)> = cands
        .into_iter()
        .filter_map(|(item, group, dist, count)| {
            let full = rank::score(prefix, &item.label);
            let tail = item
                .label
                .rsplit(['.', ':'])
                .next()
                .filter(|t| *t != item.label)
                .and_then(|t| rank::score(prefix, t).map(|s| s + 50));
            let m = match (full, tail) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => a.or(b)?,
            };
            Some((m, group, dist / 64, u32::MAX - count, item.label.len(), item))
        })
        .collect();
    scored.sort_by(|a, b| (a.0, a.1, a.2, a.3, a.4, &a.5.label).cmp(&(b.0, b.1, b.2, b.3, b.4, &b.5.label)));
    scored.truncate(MAX_ITEMS);

    Completions { start, items: scored.into_iter().map(|s| s.5).collect() }
}

/// The dotted/scoped path that ends `text`: `a.b` in `x = a.b`, `std` in `use std`.
fn qualifier_before(text: &str) -> &str {
    let mut start = text.len();
    for (i, c) in text.char_indices().rev() {
        if c.is_alphanumeric() || c == '_' || c == '.' || c == ':' {
            start = i;
        } else {
            break;
        }
    }
    &text[start..]
}

/// `text` limited to `WINDOW` bytes either side of `cursor` (aligned to
/// character and line boundaries), and the offset of that slice in `text`.
fn window(text: &str, cursor: usize) -> (&str, usize) {
    if text.len() <= 2 * WINDOW {
        return (text, 0);
    }
    let mut a = cursor.saturating_sub(WINDOW);
    let mut b = (cursor + WINDOW).min(text.len());
    while !text.is_char_boundary(a) {
        a += 1;
    }
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    // Start on a line boundary so a cut can't land inside a string or comment.
    if let Some(nl) = text[a..cursor].find('\n') {
        a += nl + 1;
    }
    (&text[a..b], a)
}

/// Is the end of `line` (the current line up to the cursor) inside a string
/// or a comment? Only looks at one line, which is what matters in practice.
fn in_string_or_comment(line: &str, lang: Language) -> bool {
    let d = lang.data();
    let lc = d.line_comment;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut open: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = open {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == q {
                open = None;
            }
        } else {
            if !lc.is_empty() && line_starts_with(&chars[i..], lc) {
                return true;
            }
            if d.block_comments && line_starts_with(&chars[i..], "/*") {
                // Inside the block unless it closes before the cursor.
                let rest: String = chars[i + 2..].iter().collect();
                match rest.find("*/") {
                    Some(end) => {
                        i += 2 + rest[..end].chars().count() + 2;
                        continue;
                    }
                    None => return true,
                }
            }
            if c == '"' || (c == '`' && d.backtick_strings) {
                open = Some(c);
            } else if c == '\'' {
                let is_char = !d.lifetimes
                    || chars.get(i + 1) == Some(&'\\')
                    || chars.get(i + 2) == Some(&'\'');
                if is_char {
                    open = Some(c);
                }
            }
        }
        i += 1;
    }
    open.is_some()
}

fn line_starts_with(chars: &[char], pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, p)| chars.get(k) == Some(&p))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Complete at the `|` in `src` (which is removed).
    fn at(lang: Language, src: &str) -> Completions {
        at_with(lang, src, &[], false)
    }

    fn at_with(lang: Language, src: &str, others: &[&str], explicit: bool) -> Completions {
        let cursor = src.find('|').expect("marker");
        let text = src.replacen('|', "", 1);
        complete(&Request { language: lang, text: &text, cursor, explicit, others })
    }

    fn labels(c: &Completions) -> Vec<&str> {
        c.items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn language_from_extension() {
        assert_eq!(Language::from_extension("rs"), Language::Rust);
        assert_eq!(Language::from_extension("TSX"), Language::TypeScript);
        assert_eq!(Language::from_extension("hpp"), Language::Cpp);
        assert_eq!(Language::from_extension("md"), Language::Plain);
    }

    #[test]
    fn rust_macros_keywords_and_snippets() {
        let c = at(Language::Rust, "fn main() {\n    prin|\n}");
        assert_eq!(c.items[0].label, "print!");
        assert!(labels(&c).contains(&"println!"));
        let c = at(Language::Rust, "mat|");
        assert_eq!(c.items[0].label, "match");
        assert_eq!(c.items[0].kind, Kind::Snippet);
        // The keyword `match` is replaced by the richer snippet, not listed twice.
        assert_eq!(labels(&c).iter().filter(|l| **l == "match").count(), 1);
    }

    #[test]
    fn local_symbols_come_first_with_the_right_kind() {
        let src = "fn compute_total(x: i32) -> i32 { x }\nstruct Counter;\nfn main() { comp| }";
        let c = at(Language::Rust, src);
        assert_eq!(c.items[0].label, "compute_total");
        assert_eq!(c.items[0].kind, Kind::Function);
        let c = at(Language::Rust, "struct Counter;\nfn main() { let c: Coun| }");
        assert_eq!(c.items[0].label, "Counter");
        assert_eq!(c.items[0].kind, Kind::Type);
    }

    #[test]
    fn the_word_being_typed_is_not_suggested() {
        let c = at(Language::Rust, "fn main() { zebra_stripes| }");
        assert!(!labels(&c).contains(&"zebra_stripes"));
    }

    #[test]
    fn member_context_offers_members_not_keywords() {
        let c = at(Language::Rust, "let v = x.unw|");
        assert_eq!(c.items[0].label, "unwrap");
        assert!(c.items.iter().all(|i| i.kind != Kind::Keyword && i.kind != Kind::Snippet));
        // Names used after a dot elsewhere in the file are offered too.
        let c = at(Language::Rust, "a.frobnicate(); b.frob|");
        assert_eq!(c.items[0].label, "frobnicate");
    }

    #[test]
    fn auto_triggers_on_the_dot_but_not_on_blank_space() {
        let c = at(Language::Rust, "x.|");
        assert!(!c.items.is_empty());
        let c = at(Language::Rust, "let a = |");
        assert!(c.items.is_empty());
        let c = at_with(Language::Rust, "let a = |", &[], true);
        assert!(!c.items.is_empty());
    }

    #[test]
    fn nothing_inside_strings_or_comments() {
        assert!(at(Language::Rust, "let s = \"prin|\";").items.is_empty());
        assert!(at(Language::Rust, "// prin|").items.is_empty());
        assert!(at(Language::Python, "x = 1  # pri|").items.is_empty());
        assert!(at(Language::C, "/* pri| */").items.is_empty());
        // A closed string earlier on the line doesn't block completion.
        assert!(!at(Language::Rust, "let s = \"a\"; prin|").items.is_empty());
        // Lifetimes are not strings.
        assert!(!at(Language::Rust, "fn f<'a>(x: &'a str) { prin| }").items.is_empty());
    }

    #[test]
    fn exact_match_ranks_ahead_of_longer_ones() {
        let c = at(Language::Python, "for|");
        assert_eq!(c.items[0].label, "for");
        assert!(labels(&c).contains(&"forr"));
        let c = at(Language::Rust, "for|");
        assert_eq!(c.items[0].label, "for");
    }

    #[test]
    fn python() {
        let c = at(Language::Python, "def helper_fn(): pass\nhelp|");
        assert_eq!(c.items[0].label, "helper_fn");
        let c = at(Language::Python, "ifma|");
        assert_eq!(c.items[0].label, "ifmain");
        let c = at(Language::Python, "items = []\nitems.app|");
        assert_eq!(c.items[0].label, "append");
    }

    #[test]
    fn javascript_and_typescript() {
        let c = at(Language::JavaScript, "con|");
        assert!(labels(&c).contains(&"const"));
        assert!(labels(&c).contains(&"console.log"));
        let c = at(Language::JavaScript, "arr.for|");
        assert_eq!(c.items[0].label, "forEach");
        let c = at(Language::TypeScript, "interf|");
        assert_eq!(c.items[0].label, "interface");
        assert_eq!(c.items[0].kind, Kind::Snippet);
        // `interface` is not a JavaScript keyword.
        assert!(labels(&at(Language::JavaScript, "interf|")).iter().all(|l| *l != "interface"));
    }

    #[test]
    fn go() {
        let c = at(Language::Go, "iferr|");
        assert_eq!(c.items[0].label, "iferr");
        let c = at(Language::Go, "fmt.Pri|");
        assert!(labels(&c).starts_with(&["Printf", "Println"]) || labels(&c).starts_with(&["Println", "Printf"]));
        let c = at(Language::Go, "func Serve() {}\nfunc main() { Ser| }");
        assert_eq!(c.items[0].label, "Serve");
    }

    #[test]
    fn c_cpp_and_java() {
        let c = at(Language::C, "#define BUFFER_SIZE 64\nchar b[BUFF|];");
        assert_eq!(c.items[0].label, "BUFFER_SIZE");
        assert_eq!(c.items[0].kind, Kind::Constant);
        let c = at(Language::Cpp, "std::vector<int> v;\nv.push|");
        assert_eq!(c.items[0].label, "push_back");
        // After `std::` the qualifier is already typed, so only the rest is offered.
        let c = at(Language::Cpp, "std::str|");
        assert_eq!(c.items[0].label, "string");
        assert!(labels(&c).contains(&"string_view"));
        let c = at(Language::JavaScript, "console.|");
        assert!(labels(&c).contains(&"log"));
        assert_eq!(c.items.iter().find(|i| i.label == "log").unwrap().insert, "log($0)");
        // Without a qualifier the full qualified name is offered.
        let c = at(Language::Cpp, "std::vector<int> v; std|");
        assert!(labels(&c).contains(&"std::vector") || labels(&c).contains(&"std"));
        let c = at(Language::Java, "sou|");
        assert_eq!(c.items[0].label, "sout");
        let c = at(Language::Java, "list.isEm|");
        assert_eq!(c.items[0].label, "isEmpty");
    }

    #[test]
    fn camel_case_and_snake_case_abbreviations() {
        let c = at(Language::JavaScript, "function getUserName() {}\ngun|");
        assert_eq!(c.items[0].label, "getUserName");
        let c = at(Language::Python, "def hello_world(): pass\nhw|");
        assert_eq!(c.items[0].label, "hello_world");
    }

    #[test]
    fn other_documents_supply_symbols_with_lower_priority() {
        let others = ["pub fn shared_helper() {}\nfn zeta_local_name() {}"];
        let c = at_with(Language::Rust, "fn main() { shar| }", &others, false);
        assert_eq!(c.items[0].label, "shared_helper");
        // A local symbol beats an equally good one from another file.
        let others = ["fn parse_thing() {}"];
        let c = at_with(Language::Rust, "fn parse_this() {}\nfn main() { pars| }", &others, false);
        assert_eq!(c.items[0].label, "parse_this");
    }

    #[test]
    fn plain_text_completes_from_words() {
        let c = at(Language::Plain, "elephant\nelep|");
        assert_eq!(labels(&c), vec!["elephant"]);
    }

    #[test]
    fn start_is_the_prefix_offset() {
        let src = "let x = prin";
        let c = complete(&Request { language: Language::Rust, text: src, cursor: src.len(), explicit: false, others: &[] });
        assert_eq!(c.start, "let x = ".len());
    }

    #[test]
    fn expand_places_cursor_and_indents() {
        let item = Item {
            label: "fn".into(),
            insert: "fn ${name}($0) {\n    \n}".into(),
            kind: Kind::Snippet,
            detail: String::new(),
        };
        let e = item.expand("  ");
        assert_eq!(e.text, "fn name() {\n      \n  }");
        // First placeholder (`name`) is selected.
        assert_eq!(&e.text[e.select.0..e.select.1], "name");

        let item = Item { label: "p".into(), insert: "println!(\"$0\")".into(), kind: Kind::Builtin, detail: String::new() };
        let e = item.expand("");
        assert_eq!(e.text, "println!(\"\")");
        assert_eq!(e.select, (10, 10));

        let plain = Item { label: "x".into(), insert: "abc".into(), kind: Kind::Keyword, detail: String::new() };
        assert_eq!(plain.expand("").select, (3, 3));
    }

    #[test]
    fn handles_unicode_and_cursor_inside_multibyte_text() {
        let src = "let größe = 1;\ngrö|";
        let c = at(Language::Rust, src);
        assert_eq!(c.items[0].label, "größe");
    }

    const ALL: [Language; 17] = [
        Language::Rust,
        Language::Python,
        Language::JavaScript,
        Language::TypeScript,
        Language::Go,
        Language::C,
        Language::Cpp,
        Language::Java,
        Language::CSharp,
        Language::Ruby,
        Language::Php,
        Language::Shell,
        Language::Html,
        Language::Css,
        Language::Json,
        Language::Toml,
        Language::Plain,
    ];

    #[test]
    fn every_language_has_well_formed_data() {
        for lang in ALL {
            let d = lang.data();
            let mut triggers = HashSet::new();
            for (trigger, detail, body) in d.snippets {
                assert!(triggers.insert(*trigger), "{lang:?}: duplicate snippet `{trigger}`");
                assert!(!detail.is_empty() && !body.is_empty(), "{lang:?}: empty snippet `{trigger}`");
                // Placeholders must be closed and the expansion must not be empty.
                assert_eq!(body.matches("${").count(), body.matches('}').count().min(body.matches("${").count()),
                    "{lang:?}: unbalanced placeholder in `{trigger}`");
                let item = Item { label: trigger.to_string(), insert: body.to_string(), kind: Kind::Snippet, detail: String::new() };
                let e = item.expand("  ");
                assert!(!e.text.is_empty(), "{lang:?}: `{trigger}` expands to nothing");
                assert!(e.select.0 <= e.select.1 && e.select.1 <= e.text.chars().count(), "{lang:?}: bad selection for `{trigger}`");
                assert!(!e.text.contains("${") && !e.text.contains("$0"), "{lang:?}: unresolved marker in `{trigger}`");
            }
            for (label, insert, _) in d.builtins.iter().chain(d.members) {
                assert!(!label.is_empty(), "{lang:?}: empty label");
                let _ = insert;
            }
            for kw in d.keywords {
                assert!(!kw.is_empty() && !kw.contains(' '), "{lang:?}: odd keyword `{kw}`");
            }
        }
    }

    #[test]
    fn every_programming_language_has_snippets_and_names() {
        for lang in ALL {
            if lang == Language::Plain {
                continue;
            }
            assert!(lang.data().snippets.len() >= 4, "{lang:?} has too few snippets");
            assert!(!lang.name().is_empty());
        }
        // Names that show in the status bar.
        assert_eq!(Language::CSharp.name(), "C#");
        assert_eq!(Language::Php.name(), "PHP");
    }

    #[test]
    fn extensions_map_to_the_new_languages() {
        for (ext, lang) in [
            ("cs", Language::CSharp),
            ("rb", Language::Ruby),
            ("php", Language::Php),
            ("sh", Language::Shell),
            ("bash", Language::Shell),
            ("html", Language::Html),
            ("css", Language::Css),
            ("json", Language::Json),
            ("toml", Language::Toml),
        ] {
            assert_eq!(Language::from_extension(ext), lang, ".{ext}");
        }
    }

    #[test]
    fn new_languages_complete() {
        let c = at(Language::CSharp, "Console.Wri|");
        assert!(labels(&c).contains(&"WriteLine"), "{:?}", labels(&c));
        let c = at(Language::CSharp, "cw|");
        assert_eq!(c.items[0].label, "cw");
        let c = at(Language::Ruby, "list.each_w|");
        assert_eq!(c.items[0].label, "each_with_index");
        let c = at(Language::Ruby, "attr_acc|");
        assert_eq!(c.items[0].label, "attr_accessor");
        let c = at(Language::Php, "<?php\n$user = 1;\n$us|");
        assert_eq!(c.items[0].label, "$user");
        let c = at(Language::Php, "array_ma|");
        assert_eq!(c.items[0].label, "array_map");
        let c = at(Language::Shell, "shebang|");
        assert_eq!(c.items[0].label, "shebang");
        // Variables are written with their `$`, so that is the word.
        let c = at(Language::Shell, "echo $FILE_COUNT\necho $FILE_C|");
        assert_eq!(c.items[0].label, "$FILE_COUNT");
    }

    #[test]
    fn html_css_json_toml() {
        let c = at(Language::Html, "html5|");
        assert_eq!(c.items[0].label, "html5");
        let c = at(Language::Html, "<div clas|");
        assert_eq!(c.items[0].label, "class");
        // Hyphenated names are one word in CSS.
        let c = at(Language::Css, "a { background-c| }");
        assert_eq!(c.items[0].label, "background-color");
        assert_eq!(c.start, "a { ".len());
        let c = at(Language::Css, "flexc|");
        assert_eq!(c.items[0].label, "flexc");
        let c = at(Language::Json, "tru|");
        assert_eq!(c.items[0].label, "true");
        let c = at(Language::Toml, "[dep|");
        assert_eq!(c.items[0].label, "dep");
        assert!(labels(&c).contains(&"deps"));
    }

    #[test]
    fn new_snippets_for_the_original_languages() {
        for (lang, trigger) in [
            (Language::Rust, "impltrait"),
            (Language::Rust, "display"),
            (Language::Python, "listc"),
            (Language::Python, "argparse"),
            (Language::JavaScript, "usestate"),
            (Language::TypeScript, "props"),
            (Language::Go, "tbl"),
            (Language::Go, "http"),
            (Language::C, "malloc"),
            (Language::Cpp, "uptr"),
            (Language::Java, "stream"),
        ] {
            let src = format!("{trigger}|");
            let c = at(lang, &src);
            assert_eq!(c.items[0].label, trigger, "{lang:?}");
            assert_eq!(c.items[0].kind, Kind::Snippet, "{lang:?} {trigger}");
        }
    }

    #[test]
    fn large_documents_stay_fast() {
        let mut text = String::new();
        for i in 0..60_000 {
            text.push_str(&format!("fn function_number_{i}(arg: usize) -> usize {{ arg + {i} }}\n"));
        }
        text.push_str("fn main() { function_number_59 }");
        let cursor = text.len() - 2;
        let start = std::time::Instant::now();
        let c = complete(&Request { language: Language::Rust, text: &text, cursor, explicit: false, others: &[] });
        let elapsed = start.elapsed();
        assert!(!c.items.is_empty());
        assert!(c.items.iter().any(|i| i.label.starts_with("function_number_59")));
        assert!(elapsed.as_millis() < 500, "took {elapsed:?} on {} bytes", text.len());
    }
}
