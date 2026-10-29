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

/// Read an integer written as [`format_integer`] shows it: `-12.5` with
/// the hint `d-1` is -125. Returns `None` when the text does not fit the
/// hint, or the hint is not a valid integer hint.
pub fn parse_integer(hint: &str, text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mut chars = hint.trim().chars();
    let kind = chars.next()?;
    let magnitude = match (kind, chars.as_str()) {
        ('d', "") => digits.parse::<i64>().ok()?,
        ('d', decimals) => {
            let decimals: u32 = decimals.strip_prefix('-')?.parse().ok()?;
            let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
            if fraction.len() > decimals as usize
                || !(whole.chars().chain(fraction.chars())).all(|c| c.is_ascii_digit())
                || (whole.is_empty() && fraction.is_empty())
            {
                return None;
            }
            let scaled = format!("{whole}{fraction:0<width$}", width = decimals as usize);
            scaled.parse::<i64>().ok()?
        }
        ('x', "") => i64::from_str_radix(digits, 16).ok()?,
        ('o', "") => i64::from_str_radix(digits, 8).ok()?,
        ('b', "") => i64::from_str_radix(digits, 2).ok()?,
        _ => return None,
    };
    // from_str_radix and parse accept a sign of their own; one is enough.
    if digits.starts_with(['-', '+']) {
        return None;
    }
    Some(if negative { -magnitude } else { magnitude })
}

/// Read octets written as [`format_octets`] shows them: `00:1b:21:0a:ff:09`
/// with the hint `1x:` is six bytes, `2026-10-29,13:45:30.5` with the
/// DateAndTime hint is eight. Numbers may leave out leading zeros. Hints
/// with a repeat count (`*`) are not supported.
pub fn parse_octets(hint: &str, text: &str) -> Result<Vec<u8>, String> {
    let specs =
        parse_octet_hint(hint).ok_or_else(|| format!("{hint:?} is not a display hint for text"))?;
    if specs.iter().any(|spec| spec.repeat) {
        return Err(format!(
            "the display hint {hint:?} repeats parts, which cannot be typed in; give the bytes in hex"
        ));
    }
    let chars: Vec<char> = text.chars().collect();
    let mismatch = |at: usize| {
        let rest: String = chars[at..].iter().collect();
        format!("{text:?} does not fit the display hint {hint:?} at {rest:?}")
    };
    let mut out = Vec::new();
    let mut pos = 0;
    let mut part = 0;
    while pos < chars.len() {
        let spec = &specs[part.min(specs.len() - 1)];
        part += 1;
        let start = pos;
        match spec.format {
            'a' | 't' => {
                let mut bytes = 0;
                while pos < chars.len() && Some(chars[pos]) != spec.separator {
                    let len = chars[pos].len_utf8();
                    if bytes + len > spec.length {
                        break;
                    }
                    bytes += len;
                    pos += 1;
                }
                let chunk: String = chars[start..pos].iter().collect();
                out.extend_from_slice(chunk.as_bytes());
            }
            format => {
                let radix = match format {
                    'd' => 10,
                    'x' => 16,
                    _ => 8,
                };
                // Hex has a fixed number of digits per byte, so "001b21"
                // also reads with "1x:".
                let most = if radix == 16 {
                    spec.length * 2
                } else {
                    usize::MAX
                };
                while pos < chars.len() && pos - start < most && chars[pos].is_digit(radix) {
                    pos += 1;
                }
                let digits: String = chars[start..pos].iter().collect();
                let number = u128::from_str_radix(&digits, radix).map_err(|_| mismatch(start))?;
                let width = spec.length.min(16);
                if spec.length < 16 && number >> (8 * width) != 0 {
                    return Err(format!(
                        "{digits} does not fit in {} byte{} (display hint {hint:?})",
                        spec.length,
                        if spec.length == 1 { "" } else { "s" }
                    ));
                }
                out.extend_from_slice(&number.to_be_bytes()[16 - width..]);
            }
        }
        if pos == start {
            return Err(mismatch(start));
        }
        if pos < chars.len() && Some(chars[pos]) == spec.separator {
            pos += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_read_back_what_they_show() {
        assert_eq!(parse_integer("d-1", "-12.5"), Some(-125));
        assert_eq!(parse_integer("d-2", "3"), Some(300));
        assert_eq!(parse_integer("d-2", "0.05"), Some(5));
        assert_eq!(parse_integer("d-1", "1.25"), None);
        assert_eq!(parse_integer("x", "ff"), Some(255));
        assert_eq!(parse_integer("d", "--1"), None);

        let mac = [0x00, 0x1b, 0x21, 0x0a, 0xff, 0x09];
        assert_eq!(parse_octets("1x:", "00:1b:21:0a:ff:09").unwrap(), mac);
        assert_eq!(parse_octets("1x:", "001b210aff09").unwrap(), mac);
        assert_eq!(parse_octets("1x:", "0:1b:21:a:ff:9").unwrap(), mac);
        assert_eq!(parse_octets("255a", "Linux").unwrap(), b"Linux");
        assert_eq!(
            parse_octets("1d.1d.1d.1d", "192.0.2.1").unwrap(),
            [192, 0, 2, 1]
        );
        let hint = "2d-1d-1d,1d:1d:1d.1d,1a1d:1d";
        for bytes in [
            &[0x07, 0xea, 0x0a, 0x1d, 0x0d, 0x2d, 0x1e, 0x05][..],
            &[
                0x07, 0xea, 0x0a, 0x1d, 0x0d, 0x2d, 0x1e, 0x05, b'+', 0x01, 0x00,
            ],
        ] {
            let text = format_octets(hint, bytes).unwrap();
            assert_eq!(parse_octets(hint, &text).unwrap(), bytes, "{text}");
        }
        assert!(
            parse_octets("1x:", "00:zz")
                .unwrap_err()
                .contains("at \"zz\"")
        );
        assert!(
            parse_octets("1d.", "300")
                .unwrap_err()
                .contains("does not fit in 1 byte")
        );
    }

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
