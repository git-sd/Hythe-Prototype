use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Capture names we recognise; the index into this list is the class id
/// stored per byte and mapped to a colour by the renderer.
pub const NAMES: &[&str] = &[
    "attribute",
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
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "string",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

pub const NONE: u8 = u8::MAX;

pub struct RustHighlighter {
    highlighter: Highlighter,
    config: HighlightConfiguration,
}

impl RustHighlighter {
    pub fn new() -> Self {
        let mut config = HighlightConfiguration::new(
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        )
        .expect("rust highlight query");
        config.configure(NAMES);
        Self {
            highlighter: Highlighter::new(),
            config,
        }
    }

    /// One class id per byte of `source` (`NONE` where unhighlighted).
    pub fn highlight(&mut self, source: &str) -> Vec<u8> {
        let mut classes = vec![NONE; source.len()];
        let Ok(events) = self
            .highlighter
            .highlight(&self.config, source.as_bytes(), None, None, |_| None)
        else {
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
}
