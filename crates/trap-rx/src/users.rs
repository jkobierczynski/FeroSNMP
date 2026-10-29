//! The SNMPv3 trap users, read from `createUser` lines as in net-snmp's
//! `snmptrapd.conf`.

use std::fmt;
use std::path::Path;

use snmp_core::{AuthProtocol, PrivProtocol, TrapUser, Usm, parse_hex};

/// A problem in a users file, with its line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsersError {
    /// Line number, from 1; 0 when the file could not be read.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for UsersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(&self.message)
        } else {
            write!(f, "line {}: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for UsersError {}

/// Read the trap users from a file; see [`parse_users`].
pub fn read_users(path: &Path) -> Result<Vec<TrapUser>, UsersError> {
    let text = std::fs::read_to_string(path).map_err(|e| UsersError {
        line: 0,
        message: format!("cannot read {}: {e}", path.display()),
    })?;
    parse_users(&text).map_err(|e| UsersError {
        message: format!("{}: {}", path.display(), e.message),
        ..e
    })
}

/// Read trap users from `createUser` lines, as net-snmp writes them:
///
/// ```text
/// createUser [-e ENGINEID] NAME [AUTH-PROTOCOL AUTH-PASSWORD [PRIV-PROTOCOL [PRIV-PASSWORD]]]
/// ```
///
/// such as `createUser -e 0x8000000001020304 admin SHA-256 "my auth pass"
/// AES "my priv pass"`. Without a privacy password, the authentication
/// password is used for privacy too, as net-snmp does. Without `-e` the user
/// is accepted from any sender (net-snmp's snmptrapd requires `-e` for
/// traps). Passwords may be quoted with `"`.
///
/// Blank lines, `#` comments and every other line (the rest of a
/// `snmptrapd.conf`, such as `authUser`) are skipped, so a snmptrapd.conf
/// can be used as it is.
pub fn parse_users(text: &str) -> Result<Vec<TrapUser>, UsersError> {
    let mut users = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let error = |message: String| UsersError {
            line: number + 1,
            message,
        };
        let words = split_words(line).map_err(error)?;
        let Some(first) = words.first() else {
            continue;
        };
        if first.starts_with('#') || !first.eq_ignore_ascii_case("createUser") {
            continue;
        }
        users.push(parse_create_user(&words[1..]).map_err(error)?);
    }
    Ok(users)
}

fn parse_create_user(words: &[String]) -> Result<TrapUser, String> {
    let mut words = words.iter().map(String::as_str);
    let mut name = words.next();
    let mut engine_id = None;
    if name == Some("-e") {
        let id = words
            .next()
            .ok_or("-e needs an engine ID in hex, such as 0x8000000001020304")?;
        let digits = id.trim_start_matches("0x").trim_start_matches("0X");
        engine_id = Some(parse_hex(digits).map_err(|e| format!("engine ID {id:?}: {e}"))?);
        name = words.next();
    } else if let Some(option) = name.filter(|n| n.starts_with('-')) {
        return Err(format!(
            "createUser option {option} is not supported; only -e ENGINEID is (keys must be \
             given as passwords)"
        ));
    }
    let name = name.ok_or("createUser needs a user name")?;
    let mut usm = Usm::new(name);
    if let Some(protocol) = words.next() {
        let protocol: AuthProtocol = protocol.parse()?;
        let password = words
            .next()
            .ok_or_else(|| format!("{protocol} needs an authentication password"))?;
        usm = usm.with_auth(protocol, password);
        if let Some(protocol) = words.next() {
            let protocol: PrivProtocol = protocol.parse()?;
            let password = words.next().unwrap_or_default();
            usm = usm.with_privacy(protocol, password);
        }
    }
    if let Some(extra) = words.next() {
        return Err(format!("unexpected {extra:?} at the end of the line"));
    }
    usm.check()?;
    let user = TrapUser::new(usm);
    Ok(match engine_id {
        Some(id) => user.with_engine_id(id),
        None => user,
    })
}

/// Split a line into words; `"..."` keeps spaces, `\"` and `\\` escape.
fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else {
            return Ok(words);
        };
        let mut word = String::new();
        if first == '"' {
            chars.next();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(c) => word.push(c),
                        None => return Err("unfinished \\ at the end of the line".to_string()),
                    },
                    Some(c) => word.push(c),
                    None => return Err("a quote \" is not closed".to_string()),
                }
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                word.push(c);
            }
        }
        words.push(word);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snmp_core::{SecurityLevel, hex};

    #[test]
    fn reads_create_user_lines() {
        let text = r#"
# snmptrapd.conf
createUser -e 0x80001F88040102030405 admin SHA-256 "auth pass 1" AES-256 "priv pass 2"
createUser plain
createUser md5only MD5 ferosnmp-auth
createUser same SHA ferosnmp-auth AES
authUser log,execute admin
disableAuthorization yes
"#;
        let users = parse_users(text).unwrap();
        assert_eq!(users.len(), 4);
        let admin = &users[0];
        assert_eq!(
            hex(admin.engine_id.as_ref().unwrap()),
            "80001f88040102030405"
        );
        assert_eq!(admin.usm.user, "admin");
        assert_eq!(admin.usm.level, SecurityLevel::AuthPriv);
        assert_eq!(admin.usm.auth_protocol, AuthProtocol::Sha256);
        assert_eq!(admin.usm.auth_password, "auth pass 1");
        assert_eq!(admin.usm.priv_protocol, PrivProtocol::Aes256);
        assert_eq!(admin.usm.priv_password, "priv pass 2");
        assert_eq!(users[1].usm.level, SecurityLevel::NoAuthNoPriv);
        assert_eq!(users[1].engine_id, None);
        assert_eq!(users[2].usm.level, SecurityLevel::AuthNoPriv);
        // No privacy password: the authentication password is used.
        assert_eq!(users[3].usm.level, SecurityLevel::AuthPriv);
        assert_eq!(users[3].usm.priv_password, "");
    }

    #[test]
    fn explains_mistakes() {
        for (line, expected) in [
            ("createUser", "needs a user name"),
            ("createUser -e", "-e needs an engine ID"),
            ("createUser -e 0xZZ x", "engine ID"),
            ("createUser -l x", "not supported"),
            ("createUser x SHA", "needs an authentication password"),
            ("createUser x SHA short", "at least 8 characters"),
            (
                "createUser x SHA1 ferosnmp-auth BLOWFISH",
                "unknown privacy protocol",
            ),
            (
                "createUser x WHIRLPOOL ferosnmp-auth",
                "unknown authentication protocol",
            ),
            (
                "createUser x SHA ferosnmp-auth AES ferosnmp-priv extra",
                "unexpected",
            ),
            ("createUser \"x", "not closed"),
        ] {
            let error = parse_users(&format!("\n{line}\n")).unwrap_err();
            assert_eq!(error.line, 2, "{line}");
            assert!(
                error.message.contains(expected),
                "{line}: {}",
                error.message
            );
        }
    }
}
