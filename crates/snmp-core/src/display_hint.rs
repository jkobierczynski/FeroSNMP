//! DISPLAY-HINT formatting (RFC 2579, section 3.1).

use std::fmt::Write;

/// Format an integer with a hint such as `d-2` (two decimals), `x`, `o` or
/// `b`. Returns `None` when the hint is not a valid integer hint.
pub fn format_integer(hint: &str, value: i64) -> Option<String> {
    let mut chars = hint.trim().chars();
    let kind = chars.next()?;
    let rest = chars.as_str();
    match (kind, rest) {
        ('d', "") => Some(value.to_string()),
        ('d', decimals) => {
            let decimals: usize = decimals.strip_prefix('-')?.parse().ok()?;
            if decimals == 0 {
                return Some(value.to_string());
            }
            let digits = value.unsigned_abs().to_string();
            let digits = format!("{digits:0>width$}", width = decimals + 1);
            let (whole, fraction) = digits.split_at(digits.len() - decimals);
            let sign = if value < 0 { "-" } else { "" };
            Some(format!("{sign}{whole}.{fraction}"))
        }
        ('x', "") => Some(signed_radix(value, |n| format!("{n:x}"))),
        ('o', "") => Some(signed_radix(value, |n| format!("{n:o}"))),
        ('b', "") => Some(signed_radix(value, |n| format!("{n:b}"))),
        _ => None,
    }
}

fn signed_radix(value: i64, digits: impl Fn(u64) -> String) -> String {
    let sign = if value < 0 { "-" } else { "" };
    format!("{sign}{}", digits(value.unsigned_abs()))
}

/// One part of an octet-string hint, such as `1x:` or `*1d.`.
#[derive(Debug, PartialEq, Eq)]
struct Spec {
    /// The first byte of the data says how often this part repeats.
    repeat: bool,
    /// Bytes taken per application.
    length: usize,
    /// `d`, `x`, `o`, `a` or `t`.
    format: char,
    /// Written between applications.
    separator: Option<char>,
    /// Written after the repeated group.
    terminator: Option<char>,
}

fn parse_octet_hint(hint: &str) -> Option<Vec<Spec>> {
    let chars: Vec<char> = hint.trim().chars().collect();
    let mut specs = Vec::new();
    let mut i = 0;
    let delimiter = |c: Option<&char>| c.copied().filter(|c| !c.is_ascii_digit() && *c != '*');
    while i < chars.len() {
        let repeat = chars[i] == '*';
        if repeat {
            i += 1;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let length: usize = chars[start..i].iter().collect::<String>().parse().ok()?;
        let format = *chars.get(i)?;
        if !matches!(format, 'd' | 'x' | 'o' | 'a' | 't') || length == 0 {
            return None;
        }
        i += 1;
        let separator = delimiter(chars.get(i));
        if separator.is_some() {
            i += 1;
        }
        let terminator = if repeat {
            let terminator = delimiter(chars.get(i));
            if terminator.is_some() {
                i += 1;
            }
            terminator
        } else {
            None
        };
        specs.push(Spec {
            repeat,
            length,
            format,
            separator,
            terminator,
        });
    }
    (!specs.is_empty()).then_some(specs)
}

/// Format octets with a hint such as `255a` (text), `1x:` (MAC address) or
/// `2d-1d-1d,1d:1d:1d.1d,1a1d:1d` (DateAndTime). The last part of the hint
/// repeats until the data runs out. Returns `None` when the hint is not a
/// valid octet-string hint.
pub fn format_octets(hint: &str, data: &[u8]) -> Option<String> {
    let specs = parse_octet_hint(hint)?;
    let mut out = String::new();
    let mut pos = 0;
    let mut part = 0;
    while pos < data.len() {
        let spec = &specs[part.min(specs.len() - 1)];
        part += 1;
        let repeat = if spec.repeat {
            let count = usize::from(data[pos]);
            pos += 1;
            count
        } else {
            1
        };
        for i in 0..repeat {
            if pos >= data.len() {
                break;
            }
            let end = (pos + spec.length).min(data.len());
            write_chunk(&mut out, spec, &data[pos..end]);
            pos = end;
            let last_of_group = i + 1 == repeat && spec.terminator.is_some();
            if let Some(separator) = spec.separator
                && pos < data.len()
                && !last_of_group
            {
                out.push(separator);
            }
        }
        if let Some(terminator) = spec.terminator
            && pos < data.len()
        {
            out.push(terminator);
        }
    }
    Some(out)
}

fn write_chunk(out: &mut String, spec: &Spec, chunk: &[u8]) {
    let number = || chunk.iter().fold(0u128, |n, &b| (n << 8) | u128::from(b));
    match spec.format {
        'a' | 't' => out.push_str(&String::from_utf8_lossy(chunk)),
        'd' => {
            let _ = write!(out, "{}", number());
        }
        // Two digits per byte, so 00:1b:21 stays readable.
        'x' => {
            let _ = write!(out, "{:0width$x}", number(), width = chunk.len() * 2);
        }
        'o' => {
            let _ = write!(out, "{:o}", number());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_hints() {
        assert_eq!(format_integer("d", 42).unwrap(), "42");
        assert_eq!(format_integer("d-1", -125).unwrap(), "-12.5");
        assert_eq!(format_integer("d-2", 5).unwrap(), "0.05");
        assert_eq!(format_integer("d-2", -5).unwrap(), "-0.05");
        assert_eq!(format_integer("d-0", 7).unwrap(), "7");
        assert_eq!(format_integer("x", 255).unwrap(), "ff");
        assert_eq!(format_integer("o", 8).unwrap(), "10");
        assert_eq!(format_integer("b", 5).unwrap(), "101");
        assert_eq!(format_integer("q", 5), None);
        assert_eq!(format_integer("d-x", 5), None);
    }

    #[test]
    fn text_and_mac_addresses() {
        assert_eq!(format_octets("255a", b"Linux").unwrap(), "Linux");
        assert_eq!(
            format_octets("1x:", &[0x00, 0x1b, 0x21, 0x0a, 0xff, 0x09]).unwrap(),
            "00:1b:21:0a:ff:09"
        );
    }

    #[test]
    fn date_and_time() {
        let hint = "2d-1d-1d,1d:1d:1d.1d,1a1d:1d";
        let local = [0x07, 0xea, 0x0a, 0x1d, 0x0d, 0x2d, 0x1e, 0x05];
        assert_eq!(
            format_octets(hint, &local).unwrap(),
            "2026-10-29,13:45:30.5"
        );
        let zoned = [
            0x07, 0xea, 0x0a, 0x1d, 0x0d, 0x2d, 0x1e, 0x05, b'+', 0x01, 0x00,
        ];
        assert_eq!(
            format_octets(hint, &zoned).unwrap(),
            "2026-10-29,13:45:30.5,+1:0"
        );
    }

    #[test]
    fn ip_addresses_and_repeats() {
        assert_eq!(
            format_octets("1d.1d.1d.1d", &[192, 0, 2, 1]).unwrap(),
            "192.0.2.1"
        );
        // A count byte, then that many 1-byte numbers separated by '.',
        // and ';' after the group.
        assert_eq!(
            format_octets("*1d.;1a", &[3, 1, 2, 3, b'x', b'y']).unwrap(),
            "1.2.3;xy"
        );
    }

    #[test]
    fn invalid_hints() {
        assert_eq!(format_octets("", b"x"), None);
        assert_eq!(format_octets("x", b"x"), None);
        assert_eq!(format_octets("1q", b"x"), None);
        assert_eq!(format_octets("0a", b"x"), None);
    }
}
