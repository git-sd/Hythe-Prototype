use std::collections::HashMap;
use std::path::Path;

use hythe_complete::Language;
use tree_sitter::Language as TsLanguage;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter};

/// Capture names we recognise; the index into this list is the class id
/// stored per byte and mapped to a colour by the renderer. Grammars name
/// captures more finely (`keyword.control`, `function.method`); the highlighter
/// matches those to the longest name here.
pub const NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "escape",
    "function",
    "function.builtin",
    "function.macro",
    "keyword",
    "label",
    "module",
    "namespace",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.escape",
    "string.regexp",
    "string.special",
    "tag",
    "tag.delimiter",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

pub const NONE: u8 = u8::MAX;

/// Files larger than this are shown without colours so typing stays instant.
pub const MAX_HIGHLIGHT_BYTES: usize = 2 * 1024 * 1024;

/// A grammar we can highlight. Like [`Language`] but splits out the variants
/// that need a different grammar (JSX, TSX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Syntax {
    Rust,
    Python,
    JavaScript,
    Jsx,
    TypeScript,
    Tsx,
    Go,
    C,
    Cpp,
    Java,
    CSharp,
    Ruby,
    Php,
    Shell,
    Html,
    Css,
    Json,
    Toml,
}

impl Syntax {
    /// Grammar for a file, from its language and extension. `None` for plain text.
    pub fn for_file(lang: Language, path: Option<&Path>) -> Option<Syntax> {
        let ext = path
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        Some(match lang {
            Language::Rust => Syntax::Rust,
            Language::Python => Syntax::Python,
            Language::JavaScript if ext == "jsx" => Syntax::Jsx,
            Language::JavaScript => Syntax::JavaScript,
            Language::TypeScript if ext == "tsx" => Syntax::Tsx,
            Language::TypeScript => Syntax::TypeScript,
            Language::Go => Syntax::Go,
            Language::C => Syntax::C,
            Language::Cpp => Syntax::Cpp,
            Language::Java => Syntax::Java,
            Language::CSharp => Syntax::CSharp,
            Language::Ruby => Syntax::Ruby,
            Language::Php => Syntax::Php,
            Language::Shell => Syntax::Shell,
            Language::Html => Syntax::Html,
            Language::Css => Syntax::Css,
            Language::Json => Syntax::Json,
            Language::Toml => Syntax::Toml,
            Language::Plain => return None,
        })
    }

    #[allow(dead_code)]
    pub const ALL: [Syntax; 18] = [
        Syntax::Rust,
        Syntax::Python,
        Syntax::JavaScript,
        Syntax::Jsx,
        Syntax::TypeScript,
        Syntax::Tsx,
        Syntax::Go,
        Syntax::C,
        Syntax::Cpp,
        Syntax::Java,
        Syntax::CSharp,
        Syntax::Ruby,
        Syntax::Php,
        Syntax::Shell,
        Syntax::Html,
        Syntax::Css,
        Syntax::Json,
        Syntax::Toml,
    ];

    /// Build the grammar and its highlight query. Some grammars ship queries
    /// that only extend a parent's (C++ on C, TypeScript on JavaScript), so
    /// those are layered, parent first.
    fn configuration(self) -> Result<HighlightConfiguration, String> {
        let join = |parts: &[&str]| parts.join("\n");
        let (language, name, query, injections): (TsLanguage, &str, String, &str) = match self {
            Syntax::Rust => (
                tree_sitter_rust::LANGUAGE.into(),
                "rust",
                tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
                tree_sitter_rust::INJECTIONS_QUERY,
            ),
            Syntax::Python => (
                tree_sitter_python::LANGUAGE.into(),
                "python",
                tree_sitter_python::HIGHLIGHTS_QUERY.into(),
                "",
            ),
            Syntax::JavaScript => (
                tree_sitter_javascript::LANGUAGE.into(),
                "javascript",
                tree_sitter_javascript::HIGHLIGHT_QUERY.into(),
                "",
            ),
            Syntax::Jsx => (
                tree_sitter_javascript::LANGUAGE.into(),
                "jsx",
                join(&[
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                ]),
                "",
            ),
            Syntax::TypeScript => (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                join(&[
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                ]),
                "",
            ),
            Syntax::Tsx => (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                join(&[
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                ]),
                "",
            ),
            Syntax::Go => (
                tree_sitter_go::LANGUAGE.into(),
                "go",
                tree_sitter_go::HIGHLIGHTS_QUERY.into(),
                "",
            ),
            Syntax::C => (
                tree_sitter_c::LANGUAGE.into(),
                "c",
                tree_sitter_c::HIGHLIGHT_QUERY.into(),
                "",
            ),
            Syntax::Cpp => (
                tree_sitter_cpp::LANGUAGE.into(),
                "cpp",
                join(&[tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY]),
                "",
            ),
            Syntax::Java => (
                tree_sitter_java::LANGUAGE.into(),
                "java",
                tree_sitter_java::HIGHLIGHTS_QUERY.into(),
                "",
            ),
            Syntax::CSharp => (
                tree_sitter_c_sharp::LANGUAGE.into(),
                "c-sharp",
                tree_sitter_c_sharp::HIGHLIGHTS_QUERY.into(),
                "",
            ),
            Syntax::Ruby => (
                tree_sitter_ruby::LANGUAGE.into(),
                "ruby",
                tree_sitter_ruby::HIGHLIGHTS_QUERY.into(),
                "",
            ),
            Syntax::Php => (
                tree_sitter_php::LANGUAGE_PHP.into(),
                "php",
                tree_sitter_php::HIGHLIGHTS_QUERY.into(),
                "",
            ),
            Syntax::Shell => (
                tree_sitter_bash::LANGUAGE.into(),
                "bash",
                tree_sitter_bash::HIGHLIGHT_QUERY.into(),
                "",
            ),
            Syntax::Html => (
                tree_sitter_html::LANGUAGE.into(),
                "html",
                tree_sitter_html::HIGHLIGHTS_QUERY.into(),
                "",
            ),
            Syntax::Css => (
                tree_sitter_css::LANGUAGE.into(),
                "css",
                tree_sitter_css::HIGHLIGHTS_QUERY.into(),
                "",
            ),
            Syntax::Json => (
                tree_sitter_json::LANGUAGE.into(),
                "json",
                tree_sitter_json::HIGHLIGHTS_QUERY.into(),
                "",
            ),
            Syntax::Toml => (
                tree_sitter_toml_ng::LANGUAGE.into(),
                "toml",
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(),
                "",
            ),
        };
        let mut config = HighlightConfiguration::new(language, name, &query, injections, "")
            .map_err(|e| format!("{self:?}: {e}"))?;
        config.configure(NAMES);
        Ok(config)
    }
}

/// Syntax highlighter for every supported language. Grammars are loaded the
/// first time a file of that language is highlighted.
pub struct Highlighter {
    inner: TsHighlighter,
    configs: HashMap<Syntax, Option<HighlightConfiguration>>,
}

impl Highlighter {
    pub fn new() -> Self {
        Self { inner: TsHighlighter::new(), configs: HashMap::new() }
    }

    /// One class id per byte of `source` (`NONE` where unhighlighted).
    pub fn highlight(&mut self, syntax: Syntax, source: &str) -> Vec<u8> {
        let mut classes = vec![NONE; source.len()];
        let config = self.configs.entry(syntax).or_insert_with(|| match syntax.configuration() {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("highlighting disabled: {e}");
                None
            }
        });
        let Some(config) = config.as_ref() else {
            return classes;
        };
        let Ok(events) = self.inner.highlight(config, source.as_bytes(), None, None, |_| None) else {
            return classes;
        };

        let mut stack: Vec<u8> = Vec::new();
        for event in events {
            match event {
                Ok(HighlightEvent::HighlightStart(h)) => stack.push(h.0 as u8),
                Ok(HighlightEvent::HighlightEnd) => {
                    stack.pop();
                }
                Ok(HighlightEvent::Source { start, end }) => {
                    if let Some(&class) = stack.last() {
                        classes[start..end].fill(class);
                    }
                }
                Err(_) => break,
            }
        }
        classes
    }

    /// Whether the grammar for `syntax` loads (its queries compile).
    #[cfg(test)]
    pub fn available(&mut self, syntax: Syntax) -> Result<(), String> {
        syntax.configuration().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_at(classes: &[u8], src: &str, needle: &str) -> Option<&'static str> {
        let at = src.find(needle)?;
        let class = classes[at];
        (class != NONE).then(|| NAMES[class as usize])
    }

    fn highlight(syntax: Syntax, src: &str) -> Vec<u8> {
        Highlighter::new().highlight(syntax, src)
    }

    #[test]
    fn every_grammar_loads() {
        let mut h = Highlighter::new();
        for syntax in Syntax::ALL {
            if let Err(e) = h.available(syntax) {
                panic!("{e}");
            }
        }
    }

    /// Each language gets keywords, strings and comments coloured.
    #[test]
    fn colours_keywords_strings_and_comments() {
        let cases: &[(Syntax, &str, &str, &str, &str)] = &[
            // (syntax, source, a keyword, a string body, a comment marker)
            (Syntax::Rust, "fn main() { let s = \"hello\"; } // note", "fn", "hello", "note"),
            (Syntax::Python, "def f():\n    s = \"hello\"  # note\n    return s", "def", "hello", "note"),
            (Syntax::JavaScript, "function f() { const s = \"hello\"; } // note", "function", "hello", "note"),
            (Syntax::Jsx, "function f() { const s = \"hello\"; return <div/>; } // note", "function", "hello", "note"),
            (Syntax::TypeScript, "function f(): string { const s = \"hello\"; } // note", "function", "hello", "note"),
            (Syntax::Tsx, "function f(): string { const s = \"hello\"; return <div/>; } // note", "function", "hello", "note"),
            (Syntax::Go, "package main\nfunc f() { s := \"hello\" } // note", "func", "hello", "note"),
            (Syntax::C, "int main() { char *s = \"hello\"; return 0; } // note", "return", "hello", "note"),
            (Syntax::Cpp, "class A { public: int f() { auto s = \"hello\"; return 0; } }; // note", "class", "hello", "note"),
            (Syntax::Java, "class A { void f() { String s = \"hello\"; } } // note", "class", "hello", "note"),
            (Syntax::CSharp, "class A { void F() { var s = \"hello\"; } } // note", "class", "hello", "note"),
            (Syntax::Ruby, "def f\n  s = \"hello\" # note\nend", "def", "hello", "note"),
            (Syntax::Php, "<?php function f() { $s = \"hello\"; } // note", "function", "hello", "note"),
            (Syntax::Shell, "if true; then echo \"hello\"; fi # note", "if", "hello", "note"),
        ];
        for (syntax, src, keyword, string, comment) in cases {
            let c = highlight(*syntax, src);
            let kw = name_at(&c, src, keyword);
            assert!(
                kw.is_some_and(|n| n.starts_with("keyword") || n == "function" || n == "type" || n.starts_with("constant")),
                "{syntax:?}: `{keyword}` was {kw:?}"
            );
            let st = name_at(&c, src, string);
            assert!(st.is_some_and(|n| n.starts_with("string")), "{syntax:?}: string was {st:?}");
            let cm = name_at(&c, src, comment);
            assert_eq!(cm, Some("comment"), "{syntax:?}: comment was {cm:?}");
        }
    }

    #[test]
    fn data_formats_and_markup() {
        let json = "{\"key\": 12, \"ok\": true}";
        let c = highlight(Syntax::Json, json);
        assert!(name_at(&c, json, "key").is_some_and(|n| n == "property" || n.starts_with("string")));
        assert!(name_at(&c, json, "12").is_some_and(|n| n == "number" || n.starts_with("constant")));

        let toml = "[package]\nname = \"hythe\"\n# note\n";
        let c = highlight(Syntax::Toml, toml);
        assert!(name_at(&c, toml, "hythe").is_some_and(|n| n.starts_with("string")));
        assert_eq!(name_at(&c, toml, "note"), Some("comment"));

        let html = "<div class=\"box\">hi</div><!-- note -->";
        let c = highlight(Syntax::Html, html);
        assert!(name_at(&c, html, "div").is_some_and(|n| n.starts_with("tag")));
        assert!(name_at(&c, html, "box").is_some_and(|n| n.starts_with("string")));

        let css = "a { color: red; } /* note */";
        let c = highlight(Syntax::Css, css);
        assert!(name_at(&c, css, "color").is_some_and(|n| n == "property"));
        assert_eq!(name_at(&c, css, "note"), Some("comment"));
    }

    #[test]
    fn cpp_inherits_c_highlighting() {
        let src = "int main() { return 0; } // x\nclass A {};";
        let c = highlight(Syntax::Cpp, src);
        assert!(name_at(&c, src, "return").is_some_and(|n| n.starts_with("keyword")));
        assert!(name_at(&c, src, "class").is_some_and(|n| n.starts_with("keyword")));
    }

    #[test]
    fn syntax_is_chosen_from_language_and_extension() {
        let p = |s: &str| Some(std::path::PathBuf::from(s));
        assert_eq!(Syntax::for_file(Language::JavaScript, p("a.jsx").as_deref()), Some(Syntax::Jsx));
        assert_eq!(Syntax::for_file(Language::JavaScript, p("a.js").as_deref()), Some(Syntax::JavaScript));
        assert_eq!(Syntax::for_file(Language::TypeScript, p("a.tsx").as_deref()), Some(Syntax::Tsx));
        assert_eq!(Syntax::for_file(Language::Plain, None), None);
    }

    #[test]
    fn offsets_cover_multibyte_text() {
        let src = "let größe = \"naïve\"; // ünï";
        let c = highlight(Syntax::Rust, src);
        assert_eq!(c.len(), src.len());
        assert!(name_at(&c, src, "naïve").is_some_and(|n| n.starts_with("string")));
    }
}
