//! A single fast pass over a document that finds identifiers, guesses what
//! each one is, and skips comments and strings.

use crate::lang::{Language, LangData};
use crate::Kind;

/// One identifier occurrence.
pub(crate) struct Word<'a> {
    pub text: &'a str,
    pub start: usize,
    pub end: usize,
    /// Best guess at what the identifier names.
    pub kind: Kind,
    /// `true` if the kind came from a definition keyword (`fn`, `class`, …)
    /// or a call, which outranks a mere guess from the spelling.
    pub certain: bool,
    /// Written right after `.`, `::` or `->`.
    pub member: bool,
}

pub(crate) fn is_ident_start(c: char, dollar: bool) -> bool {
    c == '_' || c.is_alphabetic() || (dollar && c == '$')
}

pub(crate) fn is_ident_char(c: char, dollar: bool) -> bool {
    c == '_' || c.is_alphanumeric() || (dollar && c == '$')
}

/// Kind guessed from spelling alone: `MAX_LEN` is a constant, `Widget` a type.
fn guess_kind(word: &str) -> Kind {
    let letters = || word.chars().filter(|c| c.is_alphabetic());
    let has_lower = letters().any(|c| c.is_lowercase());
    let first_upper = word.chars().next().is_some_and(|c| c.is_uppercase());
    if word.len() > 1 && first_upper && !has_lower {
        Kind::Constant
    } else if first_upper {
        Kind::Type
    } else {
        Kind::Variable
    }
}

/// Scan `text`, calling `on_word` for each identifier outside comments and strings.
pub(crate) fn scan<'a>(text: &'a str, lang: Language, mut on_word: impl FnMut(Word<'a>)) {
    let d: &LangData = lang.data();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let dollar = d.dollar_idents;
    let line_comment = d.line_comment.as_bytes();

    let mut i = 0;
    // The identifier before this one, kept across whitespace only.
    let mut prev: Option<&'a str> = None;
    // The last non-space byte(s) before the current identifier, for member detection.
    let mut last_punct: [u8; 2] = [0, 0];

    while i < n {
        let b = bytes[i];

        // Line comment.
        if !line_comment.is_empty() && bytes[i..].starts_with(line_comment) {
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment.
        if d.block_comments && bytes[i..].starts_with(b"/*") {
            i += 2;
            while i < n && !bytes[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        // Strings.
        if b == b'"' || b == b'\'' || (b == b'`' && d.backtick_strings) {
            if b == b'\'' && d.lifetimes {
                // `'a'` and `'\n'` are char literals; `'a` is a lifetime.
                let rest = &bytes[i + 1..];
                let is_char = rest.first() == Some(&b'\\')
                    || rest.get(1) == Some(&b'\'')
                    || (rest.first().is_some_and(|c| !c.is_ascii()) && rest.iter().take(5).any(|c| *c == b'\''));
                if !is_char {
                    i += 1;
                    continue;
                }
            }
            let triple = d.triple_quotes && bytes[i..].starts_with(&[b, b, b]);
            let multiline = triple || b == b'`';
            i += if triple { 3 } else { 1 };
            while i < n {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if triple {
                    if bytes[i..].starts_with(&[b, b, b]) {
                        i += 3;
                        break;
                    }
                } else if bytes[i] == b {
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' && !multiline {
                    break;
                }
                i += 1;
            }
            i = i.min(n);
            prev = None;
            last_punct = [0, 0];
            continue;
        }
        // Preprocessor directive: only `#define NAME` introduces a name.
        if d.preprocessor && b == b'#' {
            let mut j = i + 1;
            while j < n && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            let s = j;
            while j < n && bytes[j].is_ascii_alphabetic() {
                j += 1;
            }
            prev = (&text[s..j] == "define").then_some("#define");
            i = j;
            continue;
        }

        // Identifier.
        let c = text[i..].chars().next().unwrap();
        if is_ident_start(c, dollar) {
            let start = i;
            let mut end = i;
            for ch in text[i..].chars() {
                if is_ident_char(ch, dollar) {
                    end += ch.len_utf8();
                } else {
                    break;
                }
            }
            let word = &text[start..end];

            let member = last_punct[1] == b'.'
                || (last_punct == [b':', b':'])
                || (last_punct == [b'-', b'>']);

            // Kind from the preceding definition keyword, a call, or the spelling.
            let mut kind = guess_kind(word);
            let mut certain = false;
            if let Some(p) = prev {
                if let Some((_, k)) = d.defs.iter().find(|(kw, _)| *kw == p) {
                    kind = *k;
                    certain = true;
                }
            }
            if !certain {
                let mut j = end;
                while j < n && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                // `name(` is a call or declaration; `name!(` a Rust macro call.
                if j < n && (bytes[j] == b'(' || (bytes[j] == b'!' && lang == Language::Rust && bytes.get(j + 1) == Some(&b'('))) {
                    if !d.keywords.contains(&word) {
                        kind = Kind::Function;
                        certain = true;
                    }
                }
            }

            on_word(Word { text: word, start, end, kind, certain, member });

            // `mut`, `ref` and `pub`-like qualifiers don't break `let mut x`.
            if !matches!(word, "mut" | "ref" | "async" | "unsafe" | "pub" | "static" | "final") || prev.is_none() {
                prev = Some(word);
            }
            last_punct = [0, 0];
            i = end;
            continue;
        }

        // Anything else.
        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' {
            // Whitespace keeps `prev`, but a newline ends the statement.
            if b == b'\n' {
                prev = None;
            }
        } else if b == b'&' || b == b'*' {
            // `&mut x`, `*const T`: transparent.
        } else {
            prev = None;
            last_punct = [last_punct[1], b];
        }
        i += text[i..].chars().next().map_or(1, |c| c.len_utf8());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(text: &str, lang: Language) -> Vec<(String, Kind, bool)> {
        let mut out = Vec::new();
        scan(text, lang, |w| out.push((w.text.to_string(), w.kind, w.member)));
        out
    }

    fn kind_of(words: &[(String, Kind, bool)], name: &str) -> Kind {
        words.iter().find(|w| w.0 == name).unwrap_or_else(|| panic!("{name} not found")).1
    }

    #[test]
    fn rust_definitions_and_calls() {
        let w = collect("pub fn build(x: i32) -> Widget { let mut count = 0; MAX_LEN; helper(1) }", Language::Rust);
        assert_eq!(kind_of(&w, "build"), Kind::Function);
        assert_eq!(kind_of(&w, "Widget"), Kind::Type);
        assert_eq!(kind_of(&w, "count"), Kind::Variable);
        assert_eq!(kind_of(&w, "MAX_LEN"), Kind::Constant);
        assert_eq!(kind_of(&w, "helper"), Kind::Function);
    }

    #[test]
    fn comments_and_strings_are_skipped() {
        let w = collect("let a = \"in_string\"; // in_comment\n/* in_block */ real", Language::Rust);
        let names: Vec<&str> = w.iter().map(|w| w.0.as_str()).collect();
        assert!(names.contains(&"real"));
        assert!(!names.contains(&"in_string"));
        assert!(!names.contains(&"in_comment"));
        assert!(!names.contains(&"in_block"));
    }

    #[test]
    fn rust_lifetimes_are_not_strings() {
        let w = collect("fn f<'a>(x: &'a str) -> &'a str { visible }", Language::Rust);
        assert!(w.iter().any(|w| w.0 == "visible"));
    }

    #[test]
    fn python_defs_and_triple_quotes() {
        let w = collect("class Foo:\n    \"\"\"doc_text\"\"\"\n    def run(self): pass\n# note_here\n", Language::Python);
        assert_eq!(kind_of(&w, "Foo"), Kind::Type);
        assert_eq!(kind_of(&w, "run"), Kind::Function);
        assert!(!w.iter().any(|w| w.0 == "doc_text" || w.0 == "note_here"));
    }

    #[test]
    fn member_access_is_detected() {
        let w = collect("value.unwrap_or(1); Foo::new(); ptr->next;", Language::Cpp);
        for name in ["unwrap_or", "new", "next"] {
            assert!(w.iter().find(|w| w.0 == name).unwrap().2, "{name} should be a member");
        }
        assert!(!w.iter().find(|w| w.0 == "value").unwrap().2);
    }

    #[test]
    fn js_and_go_declarations() {
        let js = collect("const total = 1; function addUp() {} class Cart {}", Language::JavaScript);
        assert_eq!(kind_of(&js, "total"), Kind::Variable);
        assert_eq!(kind_of(&js, "addUp"), Kind::Function);
        assert_eq!(kind_of(&js, "Cart"), Kind::Type);
        let go = collect("func Serve() {}\ntype Server struct{}\nconst Port = 80", Language::Go);
        assert_eq!(kind_of(&go, "Serve"), Kind::Function);
        assert_eq!(kind_of(&go, "Server"), Kind::Type);
        assert_eq!(kind_of(&go, "Port"), Kind::Constant);
    }

    #[test]
    fn c_defines_and_structs() {
        let w = collect("#define LIMIT 10\nstruct node { int v; };\n", Language::C);
        assert_eq!(kind_of(&w, "LIMIT"), Kind::Constant);
        assert_eq!(kind_of(&w, "node"), Kind::Type);
    }
}
