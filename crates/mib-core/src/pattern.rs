//! Simple wildcard patterns for searching names: `*` matches any text (also
//! none) and `?` matches exactly one character. Case is ignored.

/// A compiled search pattern, as used by [`MibSet::search`](crate::MibSet::search).
///
/// ```
/// use mib_core::Pattern;
///
/// assert!(Pattern::new("if*Octets").matches("ifInOctets"));
/// assert!(Pattern::new("descr").matches("sysDescr"));
/// ```
#[derive(Debug, Clone)]
pub struct Pattern {
    /// The pattern in lower case, as characters.
    chars: Vec<char>,
}

impl Pattern {
    /// Compile a pattern. Without `*` or `?`, the pattern matches every text
    /// that contains it, as if it were written `*pattern*`.
    pub fn new(pattern: &str) -> Self {
        let mut text = pattern.to_lowercase();
        if !text.contains(['*', '?']) {
            text = format!("*{text}*");
        }
        Pattern {
            chars: text.chars().collect(),
        }
    }

    /// True if the pattern was written as `MODULE::name` and should be
    /// matched against qualified names.
    pub fn is_qualified(&self) -> bool {
        // `::` cannot appear in a plain name, so its presence is enough.
        self.chars.windows(2).any(|pair| pair == [':', ':'])
    }

    /// True if the whole of `text` matches the pattern.
    pub fn matches(&self, text: &str) -> bool {
        let text: Vec<char> = text.to_lowercase().chars().collect();
        let pattern = &self.chars;

        // Walk through text and pattern together. When a `*` is seen, remember
        // where it was; if a later character does not match, go back and let
        // that `*` swallow one more character of the text. This needs no
        // recursion and is linear for the usual patterns.
        let (mut p, mut t) = (0, 0);
        let mut last_star: Option<(usize, usize)> = None;
        while t < text.len() {
            if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
                p += 1;
                t += 1;
            } else if p < pattern.len() && pattern[p] == '*' {
                last_star = Some((p, t));
                p += 1;
            } else if let Some((star_p, star_t)) = last_star {
                p = star_p + 1;
                t = star_t + 1;
                last_star = Some((star_p, star_t + 1));
            } else {
                return false;
            }
        }
        // Only `*`s may be left in the pattern.
        pattern[p..].iter().all(|&c| c == '*')
    }
}

#[cfg(test)]
mod tests {
    use super::Pattern;

    fn matches(pattern: &str, text: &str) -> bool {
        Pattern::new(pattern).matches(text)
    }

    #[test]
    fn plain_text_matches_anywhere() {
        assert!(matches("descr", "sysDescr"));
        assert!(matches("DESCR", "sysDescr"));
        assert!(matches("sys", "sysDescr"));
        assert!(!matches("descr", "sysName"));
    }

    #[test]
    fn star_matches_any_text() {
        assert!(matches("if*Octets", "ifInOctets"));
        assert!(matches("if*Octets", "ifHCOutOctets"));
        assert!(matches("if*octets", "ifOctets"));
        assert!(!matches("if*Octets", "ifInOctetsTotal"));
        assert!(matches("*", ""));
        assert!(matches("a**b", "ab"));
        assert!(matches("*a*b*", "xxaxxbxx"));
    }

    #[test]
    fn question_mark_matches_one_character() {
        assert!(matches("ifIn?rrors", "ifInErrors"));
        assert!(!matches("ifIn?rrors", "ifInrrors"));
        assert!(!matches("sys?", "sys"));
    }

    #[test]
    fn non_ascii_text_is_compared_by_character() {
        assert!(matches("t?st", "tést"));
    }
}
