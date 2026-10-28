//! The text of a MIB file, split into classified tokens for display.
//!
//! The tokens come from mib-rs's lossless syntax tree, which keeps every
//! byte of the file (comments, whitespace and text it could not parse), so
//! a viewer can colour exactly what is in the file and point at the place a
//! [`Diagnostic`](crate::Diagnostic) refers to.

use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use mib_rs::{SyntaxCategory, SyntaxKind};

/// What a token is, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenClass {
    /// A `-- comment`.
    Comment,
    /// A keyword: `OBJECT-TYPE`, `SYNTAX`, `INTEGER`, `read-only`, ...
    Keyword,
    /// A quoted string, such as a DESCRIPTION.
    String,
    /// A number, or a hex or binary string.
    Number,
    /// A name starting with an uppercase letter: a type or module.
    TypeName,
    /// A name starting with a lowercase letter: an object or other value.
    ValueName,
    /// Text the parser could not make sense of.
    Error,
}

/// One classified token: a byte range in [`SourceView::text`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Where the token is in the text, in bytes.
    pub range: Range<usize>,
    /// What it is.
    pub class: TokenClass,
}

/// A MIB file's text and its tokens. Whitespace and punctuation are not
/// listed; everything else is.
#[derive(Debug, Clone)]
pub struct SourceView {
    /// The file.
    pub file: PathBuf,
    /// The text. Files that are not UTF-8 are read as Latin-1, as many
    /// vendor MIBs are, so each byte is one character and the columns of
    /// diagnostics still match.
    pub text: String,
    /// The tokens, in order.
    pub tokens: Vec<Token>,
}

impl SourceView {
    /// Read a file from disk and classify its tokens.
    pub fn read(file: &Path) -> io::Result<SourceView> {
        Ok(SourceView::from_bytes(file, fs::read(file)?))
    }

    /// Classify the tokens of `bytes`, the content of `file`.
    pub fn from_bytes(file: &Path, bytes: Vec<u8>) -> SourceView {
        let (text, offsets) = match String::from_utf8(bytes) {
            Ok(text) => (text, None),
            Err(error) => {
                let bytes = error.into_bytes();
                let text: String = bytes.iter().map(|&b| char::from(b)).collect();
                // Where each byte of the file starts in `text`.
                let mut offsets = Vec::with_capacity(bytes.len() + 1);
                let mut at = 0;
                for &b in &bytes {
                    offsets.push(at);
                    at += char::from(b).len_utf8();
                }
                offsets.push(at);
                (text, Some(offsets))
            }
        };
        let to_text = |offset: usize| match &offsets {
            Some(offsets) => offsets[offset.min(offsets.len() - 1)],
            None => offset,
        };

        // The tree is built from the original bytes, so offsets are mapped
        // into `text` for files that are not UTF-8.
        let original: Vec<u8> = match &offsets {
            None => text.as_bytes().to_vec(),
            Some(_) => text.chars().map(|c| c as u32 as u8).collect(),
        };
        let candidate = mib_rs::SourceCandidate::new(
            mib_rs::CandidateId::new(file.display().to_string()),
            mib_rs::SourceOrigin::File {
                path: file.to_path_buf(),
            },
            file.display().to_string(),
            original,
        );
        let tokens = match mib_rs::cst::parse(candidate) {
            Ok((tree, _)) => tree
                .tokens()
                .filter_map(|token| {
                    let class = classify(token.kind())?;
                    let range = token.range();
                    Some(Token {
                        range: to_text(range.start().as_usize())..to_text(range.end().as_usize()),
                        class,
                    })
                })
                .collect(),
            // Only a file over 4 GB cannot be parsed; show it uncoloured.
            Err(_) => Vec::new(),
        };
        SourceView {
            file: file.to_path_buf(),
            text,
            tokens,
        }
    }

    /// The byte offset in [`text`](Self::text) of a line and column, both
    /// starting at 1, as in a [`Location`](crate::Location). Columns count
    /// characters. A column past the end of the line gives the end of the
    /// line; a line past the end of the text gives `None`.
    pub fn offset(&self, line: u32, column: u32) -> Option<usize> {
        let mut start = 0;
        for _ in 1..line.max(1) {
            start += self.text[start..].find('\n')? + 1;
        }
        let line_text = &self.text[start..];
        let line_text = &line_text[..line_text.find('\n').unwrap_or(line_text.len())];
        let line_text = line_text.strip_suffix('\r').unwrap_or(line_text);
        let column = column.max(1) as usize - 1;
        let within = line_text
            .char_indices()
            .nth(column)
            .map_or(line_text.len(), |(at, _)| at);
        Some(start + within)
    }
}

fn classify(kind: SyntaxKind) -> Option<TokenClass> {
    Some(match kind {
        SyntaxKind::Comment => TokenClass::Comment,
        SyntaxKind::QuotedString => TokenClass::String,
        SyntaxKind::Number
        | SyntaxKind::NegativeNumber
        | SyntaxKind::HexString
        | SyntaxKind::BinString => TokenClass::Number,
        SyntaxKind::UppercaseIdent => TokenClass::TypeName,
        SyntaxKind::LowercaseIdent => TokenClass::ValueName,
        SyntaxKind::ErrorToken | SyntaxKind::ForbiddenKeyword => TokenClass::Error,
        kind if kind.category() == SyntaxCategory::Keyword => TokenClass::Keyword,
        _ => return None,
    })
}

/// The line and column (from 1) of a byte offset, counting columns in
/// characters when the text is UTF-8 and in bytes otherwise, the same way
/// diagnostics from mib-rs do.
pub(crate) fn line_column(bytes: &[u8], offset: usize) -> (u32, u32) {
    let offset = offset.min(bytes.len());
    let before = &bytes[..offset];
    let line_start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |at| at + 1);
    let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
    let column = match std::str::from_utf8(&bytes[line_start..offset]) {
        Ok(text) if std::str::from_utf8(bytes).is_ok() => text.chars().count(),
        _ => offset - line_start,
    };
    (line as u32, column as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str =
        "-- comment\nX-MIB DEFINITIONS ::= BEGIN\nx OBJECT IDENTIFIER ::= { iso 3 }\nEND\n";

    #[test]
    fn tokens_are_classified() {
        let view = SourceView::from_bytes(Path::new("X-MIB"), TEXT.as_bytes().to_vec());
        let classes: Vec<(&str, TokenClass)> = view
            .tokens
            .iter()
            .map(|token| (&view.text[token.range.clone()], token.class))
            .collect();
        assert_eq!(classes[0], ("-- comment", TokenClass::Comment));
        assert_eq!(classes[1], ("X-MIB", TokenClass::TypeName));
        assert_eq!(classes[2], ("DEFINITIONS", TokenClass::Keyword));
        assert!(classes.contains(&("x", TokenClass::ValueName)));
        assert!(classes.contains(&("3", TokenClass::Number)));
    }

    #[test]
    fn latin1_files_keep_their_columns() {
        let mut bytes = b"-- caf".to_vec();
        bytes.push(0xE9); // é in Latin-1
        bytes.extend_from_slice(b"\nX-MIB DEFINITIONS ::= BEGIN END\n");
        let view = SourceView::from_bytes(Path::new("X-MIB"), bytes);
        assert!(view.text.starts_with("-- café\n"));
        let comment = &view.tokens[0];
        assert_eq!(&view.text[comment.range.clone()], "-- café");
        assert_eq!(view.offset(2, 1), Some("-- café\n".len()));
    }

    #[test]
    fn offsets_count_characters() {
        let view = SourceView::from_bytes(Path::new("t"), "ab\r\nçd\n".as_bytes().to_vec());
        assert_eq!(view.offset(1, 2), Some(1));
        assert_eq!(view.offset(1, 9), Some(2)); // end of line, before \r\n
        assert_eq!(view.offset(2, 2), Some(4 + 'ç'.len_utf8()));
        assert_eq!(view.offset(5, 1), None);
    }

    #[test]
    fn line_columns_match_mib_rs() {
        assert_eq!(line_column(b"ab\ncd", 4), (2, 2));
        assert_eq!(line_column("é\nxé y".as_bytes(), 7), (2, 4));
        assert_eq!(line_column(&[b'a', 0xE9, b'b'], 2), (1, 3));
    }
}
