//! MIB source as RTF for the rich edit control in the source viewer.
//!
//! Colouring a rich edit control token by token takes one message per token,
//! which is slow for vendor MIBs with tens of thousands of tokens. Instead the
//! whole file is written as one RTF document, with a colour per token class
//! and a highlight on the place the diagnostic points at, and handed to the
//! control in one go.
//!
//! The control counts positions in UTF-16 units, with each line break as one
//! position; [`position`] converts byte offsets in the text to those.

use std::ops::Range;

use mib_core::{SourceView, TokenClass};

/// Colours in the RTF colour table, numbered from 1 (0 is "automatic").
const COLORS: [(u8, u8, u8); 8] = [
    (0, 0, 192),     // 1 keyword
    (0, 128, 0),     // 2 comment
    (163, 21, 21),   // 3 string
    (9, 120, 90),    // 4 number
    (38, 110, 140),  // 5 type name
    (205, 0, 0),     // 6 error
    (255, 225, 120), // 7 highlight background
    (90, 90, 90),    // 8 plain text in the "why" report
];

const HIGHLIGHT: usize = 7;

fn color(class: TokenClass) -> Option<usize> {
    match class {
        TokenClass::Keyword => Some(1),
        TokenClass::Comment => Some(2),
        TokenClass::String => Some(3),
        TokenClass::Number => Some(4),
        TokenClass::TypeName => Some(5),
        TokenClass::Error => Some(6),
        TokenClass::ValueName => None,
    }
}

/// The source as RTF, with `highlight` (a byte range in the text) marked.
pub fn source_rtf(view: &SourceView, highlight: Option<Range<usize>>) -> String {
    let text = &view.text;
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for token in &view.tokens {
        cuts.push(token.range.start);
        cuts.push(token.range.end);
    }
    if let Some(range) = &highlight {
        cuts.push(range.start.min(text.len()));
        cuts.push(range.end.min(text.len()));
    }
    cuts.retain(|&cut| text.is_char_boundary(cut));
    cuts.sort_unstable();
    cuts.dedup();

    let mut out = header(text.len());
    let mut token = 0;
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        while token < view.tokens.len() && view.tokens[token].range.end <= start {
            token += 1;
        }
        let class = view
            .tokens
            .get(token)
            .filter(|t| t.range.start <= start)
            .and_then(|t| color(t.class));
        let marked = highlight
            .as_ref()
            .is_some_and(|range| range.start <= start && start < range.end);
        let piece = &text[start..end];
        if class.is_none() && !marked {
            escape(piece, &mut out);
            continue;
        }
        out.push('{');
        if let Some(color) = class {
            out.push_str(&format!("\\cf{color}"));
        }
        if marked {
            out.push_str(&format!("\\highlight{HIGHLIGHT}"));
        }
        out.push(' ');
        escape(piece, &mut out);
        out.push('}');
    }
    out.push('}');
    out
}

/// Plain text (the "Why this symbol?" report) as RTF in the same font.
pub fn plain_rtf(text: &str) -> String {
    let mut out = header(text.len());
    escape(text, &mut out);
    out.push('}');
    out
}

fn header(length: usize) -> String {
    let mut out = String::with_capacity(length * 2 + 512);
    out.push_str("{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0\\fmodern Consolas;}}{\\colortbl ;");
    for (r, g, b) in COLORS {
        out.push_str(&format!("\\red{r}\\green{g}\\blue{b};"));
    }
    out.push_str("}\\f0\\fs19 ");
    out
}

/// Append `text` to RTF, escaping what RTF gives a meaning to.
fn escape(text: &str, out: &mut String) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\par\n");
            }
            '\n' => out.push_str("\\par\n"),
            ' '..='~' => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    // RTF writes Unicode as a signed 16-bit number, followed
                    // by a fallback character for old readers.
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
}

/// The rich edit position of a byte offset in `text`: UTF-16 units, with
/// each line break (`\r\n`, `\n` or `\r`) counted as one.
pub fn position(text: &str, offset: usize) -> u32 {
    let mut position = 0u32;
    let mut chars = text[..offset.min(text.len())].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' && chars.peek() == Some(&'\n') {
            chars.next();
        }
        position += c.len_utf16() as u32;
    }
    position
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn positions_count_line_breaks_once() {
        let text = "ab\r\ncd\ne\u{1F600}f";
        assert_eq!(position(text, 2), 2);
        assert_eq!(position(text, 4), 3); // after \r\n
        assert_eq!(position(text, 7), 6); // after \n
        assert_eq!(position(text, 8), 7);
        assert_eq!(position(text, 12), 9); // the emoji is two UTF-16 units
        assert_eq!(position(text, 999), 10);
    }

    #[test]
    fn rtf_escapes_and_colours() {
        let text = "-- {x}\\ é\r\nX-MIB DEFINITIONS ::= BEGIN END\n";
        let view = SourceView::from_bytes(Path::new("X-MIB"), text.as_bytes().to_vec());
        let start = text.find("DEFINITIONS").unwrap();
        let rtf = source_rtf(&view, Some(start..start + "DEFINITIONS".len()));
        assert!(rtf.starts_with("{\\rtf1"));
        assert!(rtf.ends_with('}'));
        assert!(rtf.contains("{\\cf2 -- \\{x\\}\\\\ \\u233?}"), "{rtf}");
        assert!(rtf.contains("\\par\n"));
        assert!(rtf.contains("{\\cf1\\highlight7 DEFINITIONS}"), "{rtf}");
        assert!(rtf.contains("{\\cf5 X-MIB}"), "{rtf}");
        // Exactly one line break per source line.
        assert_eq!(rtf.matches("\\par").count(), 2);
    }

    #[test]
    fn highlight_inside_a_token_splits_it() {
        let text = "-- comment\n";
        let view = SourceView::from_bytes(Path::new("t"), text.as_bytes().to_vec());
        let rtf = source_rtf(&view, Some(3..5));
        assert!(
            rtf.contains("{\\cf2 -- }{\\cf2\\highlight7 co}{\\cf2 mment}"),
            "{rtf}"
        );
    }

    #[test]
    fn plain_text_has_no_colours() {
        let rtf = plain_rtf("a\nb");
        assert!(rtf.ends_with(" a\\par\nb}"), "{rtf}");
    }
}
