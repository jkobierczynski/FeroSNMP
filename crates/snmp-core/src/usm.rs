//! The User-based Security Model of SNMPv3 (RFC 3414): user settings, key
//! derivation, authentication (HMAC with MD5, SHA-1 and SHA-2, RFC 3414 and
//! RFC 7860) and privacy (DES-CBC, RFC 3414; AES-CFB, RFC 3826).
//!
//! All crypto is pure Rust, from the RustCrypto crates.
//!
//! # AES-192 and AES-256 keys
//!
//! RFC 3826 only defines AES-128. For AES-192 and AES-256 the localized key
//! from MD5 (16 bytes) or SHA-1 (20 bytes) is too short, and two drafts
//! extend it in different ways, so both sides must use the same one:
//!
//! - [`PrivProtocol::Aes192`] and [`PrivProtocol::Aes256`] follow
//!   draft-blumenthal-aes-usm-04: append the hash of the key. net-snmp calls
//!   these `AES-192` and `AES-256`; most agents use this one.
//! - [`PrivProtocol::Aes192Cisco`] and [`PrivProtocol::Aes256Cisco`] follow
//!   draft-reeder-snmpv3-usm-3desede: append a key localized from the key
//!   itself as if it were a password. Cisco IOS uses this; net-snmp calls
//!   these `AES-192C` and `AES-256C`.
//!
//! With SHA-256 and longer hashes the key is long enough and the two give
//! the same result.

use std::fmt;
use std::str::FromStr;

use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use hmac::{Hmac, Mac};
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};

/// How much of a message is protected (RFC 3411, `SnmpSecurityLevel`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecurityLevel {
    /// No authentication, no privacy: only the user name.
    #[default]
    NoAuthNoPriv,
    /// Messages are authenticated but not encrypted.
    AuthNoPriv,
    /// Messages are authenticated and encrypted.
    AuthPriv,
}

impl SecurityLevel {
    /// Every level, from least to most protected.
    pub const ALL: [SecurityLevel; 3] = [
        SecurityLevel::NoAuthNoPriv,
        SecurityLevel::AuthNoPriv,
        SecurityLevel::AuthPriv,
    ];

    /// The name used by RFC 3411 and net-snmp, such as `authPriv`.
    pub fn name(self) -> &'static str {
        match self {
            SecurityLevel::NoAuthNoPriv => "noAuthNoPriv",
            SecurityLevel::AuthNoPriv => "authNoPriv",
            SecurityLevel::AuthPriv => "authPriv",
        }
    }

    /// Whether messages are authenticated.
    pub fn has_auth(self) -> bool {
        self != SecurityLevel::NoAuthNoPriv
    }

    /// Whether messages are encrypted.
    pub fn has_priv(self) -> bool {
        self == SecurityLevel::AuthPriv
    }
}

impl fmt::Display for SecurityLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for SecurityLevel {
    type Err = String;

    /// `noAuthNoPriv`, `authNoPriv` or `authPriv`, ignoring case.
    fn from_str(text: &str) -> Result<Self, String> {
        SecurityLevel::ALL
            .into_iter()
            .find(|level| level.name().eq_ignore_ascii_case(text.trim()))
            .ok_or_else(|| {
                format!("unknown security level {text:?}; use noAuthNoPriv, authNoPriv or authPriv")
            })
    }
}

/// Authentication protocol: HMAC with this hash.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AuthProtocol {
    /// HMAC-MD5-96 (RFC 3414).
    Md5,
    /// HMAC-SHA-96, with SHA-1 (RFC 3414).
    #[default]
    Sha1,
    /// HMAC-128-SHA-224 (RFC 7860).
    Sha224,
    /// HMAC-192-SHA-256 (RFC 7860).
    Sha256,
    /// HMAC-256-SHA-384 (RFC 7860).
    Sha384,
    /// HMAC-384-SHA-512 (RFC 7860).
    Sha512,
}

impl AuthProtocol {
    /// Every protocol, weakest first.
    pub const ALL: [AuthProtocol; 6] = [
        AuthProtocol::Md5,
        AuthProtocol::Sha1,
        AuthProtocol::Sha224,
        AuthProtocol::Sha256,
        AuthProtocol::Sha384,
        AuthProtocol::Sha512,
    ];

    /// The name net-snmp uses, such as `SHA-256`. SHA-1 is `SHA`.
    pub fn name(self) -> &'static str {
        match self {
            AuthProtocol::Md5 => "MD5",
            AuthProtocol::Sha1 => "SHA",
            AuthProtocol::Sha224 => "SHA-224",
            AuthProtocol::Sha256 => "SHA-256",
            AuthProtocol::Sha384 => "SHA-384",
            AuthProtocol::Sha512 => "SHA-512",
        }
    }

    /// Length of the authentication code in a message
    /// (`msgAuthenticationParameters`).
    pub fn mac_len(self) -> usize {
        match self {
            AuthProtocol::Md5 | AuthProtocol::Sha1 => 12,
            AuthProtocol::Sha224 => 16,
            AuthProtocol::Sha256 => 24,
            AuthProtocol::Sha384 => 32,
            AuthProtocol::Sha512 => 48,
        }
    }

    fn hash(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut hasher = D::new();
            for part in parts {
                hasher.update(part);
            }
            hasher.finalize().to_vec()
        }
        match self {
            AuthProtocol::Md5 => run::<Md5>(parts),
            AuthProtocol::Sha1 => run::<Sha1>(parts),
            AuthProtocol::Sha224 => run::<Sha224>(parts),
            AuthProtocol::Sha256 => run::<Sha256>(parts),
            AuthProtocol::Sha384 => run::<Sha384>(parts),
            AuthProtocol::Sha512 => run::<Sha512>(parts),
        }
    }

    /// Turn a password into a key (RFC 3414, A.2): hash one megabyte of
    /// the password repeated.
    pub(crate) fn password_to_key(self, password: &[u8]) -> Vec<u8> {
        fn run<D: Digest>(password: &[u8]) -> Vec<u8> {
            const TOTAL: usize = 1_048_576;
            let mut hasher = D::new();
            let mut block = [0u8; 64];
            let mut index = 0;
            for _ in 0..TOTAL / block.len() {
                for byte in &mut block {
                    *byte = password[index % password.len()];
                    index += 1;
                }
                hasher.update(block);
            }
            hasher.finalize().to_vec()
        }
        assert!(!password.is_empty(), "checked by Usm::check");
        match self {
            AuthProtocol::Md5 => run::<Md5>(password),
            AuthProtocol::Sha1 => run::<Sha1>(password),
            AuthProtocol::Sha224 => run::<Sha224>(password),
            AuthProtocol::Sha256 => run::<Sha256>(password),
            AuthProtocol::Sha384 => run::<Sha384>(password),
            AuthProtocol::Sha512 => run::<Sha512>(password),
        }
    }

    /// Bind a key to one agent (RFC 3414, A.2.2): hash(key, engine ID, key).
    pub(crate) fn localize(self, key: &[u8], engine_id: &[u8]) -> Vec<u8> {
        self.hash(&[key, engine_id, key])
    }

    /// The authentication code for a whole message, truncated to
    /// [`mac_len`](Self::mac_len).
    pub(crate) fn mac(self, key: &[u8], message: &[u8]) -> Vec<u8> {
        macro_rules! run {
            ($hash:ty) => {{
                let mut mac = <Hmac<$hash> as KeyInit>::new_from_slice(key)
                    .expect("HMAC takes keys of any length");
                mac.update(message);
                mac.finalize().into_bytes().to_vec()
            }};
        }
        let mut code = match self {
            AuthProtocol::Md5 => run!(Md5),
            AuthProtocol::Sha1 => run!(Sha1),
            AuthProtocol::Sha224 => run!(Sha224),
            AuthProtocol::Sha256 => run!(Sha256),
            AuthProtocol::Sha384 => run!(Sha384),
            AuthProtocol::Sha512 => run!(Sha512),
        };
        code.truncate(self.mac_len());
        code
    }
}

impl fmt::Display for AuthProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for AuthProtocol {
    type Err = String;

    /// `MD5`, `SHA` (or `SHA1`, `SHA-1`), `SHA-224`, `SHA-256`, `SHA-384`,
    /// `SHA-512`, ignoring case; the dash is optional.
    fn from_str(text: &str) -> Result<Self, String> {
        let key = text.trim().to_ascii_uppercase().replace(['-', '_'], "");
        Ok(match key.as_str() {
            "MD5" => AuthProtocol::Md5,
            "SHA" | "SHA1" => AuthProtocol::Sha1,
            "SHA224" => AuthProtocol::Sha224,
            "SHA256" => AuthProtocol::Sha256,
            "SHA384" => AuthProtocol::Sha384,
            "SHA512" => AuthProtocol::Sha512,
            _ => {
                return Err(format!(
                    "unknown authentication protocol {text:?}; use MD5, SHA, SHA-224, \
                     SHA-256, SHA-384 or SHA-512"
                ));
            }
        })
    }
}

/// Privacy (encryption) protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PrivProtocol {
    /// DES-CBC (RFC 3414). Weak; only for old devices.
    Des,
    /// AES-128-CFB (RFC 3826).
    #[default]
    Aes128,
    /// AES-192-CFB with the key extension most agents use (see the
    /// module documentation).
    Aes192,
    /// AES-256-CFB with the key extension most agents use.
    Aes256,
    /// AES-192-CFB with Cisco's key extension.
    Aes192Cisco,
    /// AES-256-CFB with Cisco's key extension.
    Aes256Cisco,
}

impl PrivProtocol {
    /// Every protocol.
    pub const ALL: [PrivProtocol; 6] = [
        PrivProtocol::Des,
        PrivProtocol::Aes128,
        PrivProtocol::Aes192,
        PrivProtocol::Aes256,
        PrivProtocol::Aes192Cisco,
        PrivProtocol::Aes256Cisco,
    ];

    /// The name net-snmp uses, such as `AES-256` or `AES-256C`. AES-128 is
    /// `AES`.
    pub fn name(self) -> &'static str {
        match self {
            PrivProtocol::Des => "DES",
            PrivProtocol::Aes128 => "AES",
            PrivProtocol::Aes192 => "AES-192",
            PrivProtocol::Aes256 => "AES-256",
            PrivProtocol::Aes192Cisco => "AES-192C",
            PrivProtocol::Aes256Cisco => "AES-256C",
        }
    }

    /// Bytes of localized key this protocol uses. For DES, the second half
    /// is the pre-IV.
    fn key_len(self) -> usize {
        match self {
            PrivProtocol::Des | PrivProtocol::Aes128 => 16,
            PrivProtocol::Aes192 | PrivProtocol::Aes192Cisco => 24,
            PrivProtocol::Aes256 | PrivProtocol::Aes256Cisco => 32,
        }
    }

    /// The localized privacy key for a password, extended when the hash is
    /// shorter than the cipher needs.
    pub(crate) fn localized_key(
        self,
        auth: AuthProtocol,
        password: &[u8],
        engine_id: &[u8],
    ) -> Vec<u8> {
        let mut key = auth.localize(&auth.password_to_key(password), engine_id);
        let needed = self.key_len();
        while key.len() < needed {
            let more = match self {
                PrivProtocol::Aes192Cisco | PrivProtocol::Aes256Cisco => {
                    auth.localize(&auth.password_to_key(&key), engine_id)
                }
                _ => auth.hash(&[&key]),
            };
            key.extend(more);
        }
        key.truncate(needed);
        key
    }

    /// Encrypt a scoped PDU. Returns the ciphertext and the
    /// `msgPrivacyParameters` (the salt). `salt` must differ for every
    /// message sent with the same key.
    pub(crate) fn encrypt(
        self,
        key: &[u8],
        boots: u32,
        time: u32,
        salt: u64,
        plaintext: &[u8],
    ) -> (Vec<u8>, Vec<u8>) {
        match self {
            PrivProtocol::Des => {
                // RFC 3414, 8.1.1.1: salt = engine boots and a local counter.
                let mut salt_bytes = boots.to_be_bytes().to_vec();
                salt_bytes.extend((salt as u32).to_be_bytes());
                let cipher = des::Des::new_from_slice(&key[..8]).expect("8-byte key");
                let mut iv = [0u8; 8];
                for (i, byte) in iv.iter_mut().enumerate() {
                    *byte = key[8 + i] ^ salt_bytes[i];
                }
                let mut data = plaintext.to_vec();
                // Pad to whole blocks; the padding is ignored when decoding.
                data.resize(plaintext.len().div_ceil(8) * 8, 0);
                let mut previous = iv;
                for chunk in data.chunks_mut(8) {
                    let mut block = des::cipher::Block::<des::Des>::default();
                    for i in 0..8 {
                        block[i] = chunk[i] ^ previous[i];
                    }
                    cipher.encrypt_block(&mut block);
                    chunk.copy_from_slice(&block);
                    previous.copy_from_slice(&block);
                }
                (data, salt_bytes)
            }
            _ => {
                let salt_bytes = salt.to_be_bytes().to_vec();
                let iv = aes_iv(boots, time, &salt_bytes);
                let mut data = plaintext.to_vec();
                aes_cfb(&key[..self.key_len()], iv, &mut data, true);
                (data, salt_bytes)
            }
        }
    }

    /// Decrypt a scoped PDU, with `boots` and `time` from the same message.
    pub(crate) fn decrypt(
        self,
        key: &[u8],
        boots: u32,
        time: u32,
        salt: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, String> {
        if salt.len() != 8 {
            return Err(format!(
                "the privacy parameters are {} bytes; {} needs 8",
                salt.len(),
                self.name()
            ));
        }
        match self {
            PrivProtocol::Des => {
                if !ciphertext.len().is_multiple_of(8) {
                    return Err("the DES-encrypted data is not a whole number of blocks".into());
                }
                let cipher = des::Des::new_from_slice(&key[..8]).expect("8-byte key");
                let mut previous = [0u8; 8];
                for (i, byte) in previous.iter_mut().enumerate() {
                    *byte = key[8 + i] ^ salt[i];
                }
                let mut data = ciphertext.to_vec();
                for chunk in data.chunks_mut(8) {
                    let mut block = des::cipher::Block::<des::Des>::default();
                    block.copy_from_slice(chunk);
                    cipher.decrypt_block(&mut block);
                    let mut next = [0u8; 8];
                    next.copy_from_slice(chunk);
                    for i in 0..8 {
                        chunk[i] = block[i] ^ previous[i];
                    }
                    previous = next;
                }
                Ok(data)
            }
            _ => {
                let iv = aes_iv(boots, time, salt);
                let mut data = ciphertext.to_vec();
                aes_cfb(&key[..self.key_len()], iv, &mut data, false);
                Ok(data)
            }
        }
    }
}

impl fmt::Display for PrivProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for PrivProtocol {
    type Err = String;

    /// `DES`, `AES` (or `AES-128`), `AES-192`, `AES-256`, `AES-192C`,
    /// `AES-256C`, ignoring case; the dash is optional.
    fn from_str(text: &str) -> Result<Self, String> {
        let key = text.trim().to_ascii_uppercase().replace(['-', '_'], "");
        Ok(match key.as_str() {
            "DES" => PrivProtocol::Des,
            "AES" | "AES128" => PrivProtocol::Aes128,
            "AES192" => PrivProtocol::Aes192,
            "AES256" => PrivProtocol::Aes256,
            "AES192C" => PrivProtocol::Aes192Cisco,
            "AES256C" => PrivProtocol::Aes256Cisco,
            _ => {
                return Err(format!(
                    "unknown privacy protocol {text:?}; use DES, AES, AES-192, AES-256, \
                     AES-192C or AES-256C (C: Cisco's key extension)"
                ));
            }
        })
    }
}

/// RFC 3826, 3.1.2.1: the IV is boots, time and the salt.
fn aes_iv(boots: u32, time: u32, salt: &[u8]) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[..4].copy_from_slice(&boots.to_be_bytes());
    iv[4..8].copy_from_slice(&time.to_be_bytes());
    iv[8..].copy_from_slice(salt);
    iv
}

/// AES in CFB mode with 128-bit feedback, in place.
fn aes_cfb(key: &[u8], iv: [u8; 16], data: &mut [u8], encrypt: bool) {
    fn run<C: BlockCipherEncrypt + KeyInit>(
        key: &[u8],
        iv: [u8; 16],
        data: &mut [u8],
        encrypt: bool,
    ) {
        let cipher = C::new_from_slice(key).expect("key length checked by key_len");
        let mut feedback = iv;
        for chunk in data.chunks_mut(16) {
            let mut stream = aes::Block::default();
            stream.copy_from_slice(&feedback);
            let stream = {
                let mut block = aes::cipher::Block::<C>::default();
                block.copy_from_slice(&stream);
                cipher.encrypt_block(&mut block);
                block
            };
            for (i, byte) in chunk.iter_mut().enumerate() {
                let input = *byte;
                *byte ^= stream[i];
                // The ciphertext feeds the next block.
                feedback[i] = if encrypt { *byte } else { input };
            }
        }
    }
    match key.len() {
        16 => run::<aes::Aes128>(key, iv, data, encrypt),
        24 => run::<aes::Aes192>(key, iv, data, encrypt),
        _ => run::<aes::Aes256>(key, iv, data, encrypt),
    }
}

/// The SNMPv3 user and the secrets for talking to an agent.
///
/// Passwords must be at least 8 characters (RFC 3414, 11.2), as net-snmp
/// also requires. [`Debug`] leaves them out.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Usm {
    /// The user name (`msgUserName`, the security name).
    pub user: String,
    /// How much protection to ask for.
    pub level: SecurityLevel,
    /// Authentication protocol, for `authNoPriv` and `authPriv`.
    pub auth_protocol: AuthProtocol,
    /// Authentication password.
    pub auth_password: String,
    /// Privacy protocol, for `authPriv`.
    pub priv_protocol: PrivProtocol,
    /// Privacy password. When empty, the authentication password is used,
    /// as net-snmp does.
    pub priv_password: String,
    /// The context name; usually empty.
    pub context_name: String,
    /// The context engine ID, when it is not the agent's own engine ID
    /// (for a proxy). `None` uses the engine ID the agent reports.
    pub context_engine_id: Option<Vec<u8>>,
}

impl Usm {
    /// A user at `noAuthNoPriv`.
    pub fn new(user: impl Into<String>) -> Usm {
        Usm {
            user: user.into(),
            ..Usm::default()
        }
    }

    /// Ask for authentication with this protocol and password.
    pub fn with_auth(mut self, protocol: AuthProtocol, password: impl Into<String>) -> Usm {
        self.auth_protocol = protocol;
        self.auth_password = password.into();
        self.level = self.level.max(SecurityLevel::AuthNoPriv);
        self
    }

    /// Ask for privacy with this protocol and password. Needs
    /// authentication as well.
    pub fn with_privacy(mut self, protocol: PrivProtocol, password: impl Into<String>) -> Usm {
        self.priv_protocol = protocol;
        self.priv_password = password.into();
        self.level = SecurityLevel::AuthPriv;
        self
    }

    /// Set the context name.
    pub fn with_context(mut self, name: impl Into<String>) -> Usm {
        self.context_name = name.into();
        self
    }

    /// The privacy password actually used.
    pub(crate) fn effective_priv_password(&self) -> &str {
        if self.priv_password.is_empty() {
            &self.auth_password
        } else {
            &self.priv_password
        }
    }

    /// What is wrong with these settings, if anything.
    pub fn check(&self) -> Result<(), String> {
        if self.user.is_empty() {
            return Err("SNMPv3 needs a user name".to_string());
        }
        if self.user.len() > 32 {
            return Err("the user name is longer than 32 characters (RFC 3414)".to_string());
        }
        if self.level.has_auth() && self.auth_password.chars().count() < 8 {
            return Err(format!(
                "the authentication password must be at least 8 characters for {} \
                 (RFC 3414; net-snmp refuses shorter ones too)",
                self.level
            ));
        }
        if self.level.has_priv() && self.effective_priv_password().chars().count() < 8 {
            return Err(
                "the privacy password must be at least 8 characters (RFC 3414)".to_string(),
            );
        }
        Ok(())
    }

    /// One line such as `user admin, authPriv, SHA-256/AES`.
    pub fn summary(&self) -> String {
        match self.level {
            SecurityLevel::NoAuthNoPriv => format!("user {}, noAuthNoPriv", self.user),
            SecurityLevel::AuthNoPriv => {
                format!("user {}, authNoPriv, {}", self.user, self.auth_protocol)
            }
            SecurityLevel::AuthPriv => format!(
                "user {}, authPriv, {}/{}",
                self.user, self.auth_protocol, self.priv_protocol
            ),
        }
    }
}

impl fmt::Debug for Usm {
    // The passwords are left out.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Usm")
            .field("user", &self.user)
            .field("level", &self.level)
            .field("auth_protocol", &self.auth_protocol)
            .field("priv_protocol", &self.priv_protocol)
            .field("context_name", &self.context_name)
            .finish_non_exhaustive()
    }
}

/// Localized keys for one user at one agent.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Keys {
    pub auth: Vec<u8>,
    pub privacy: Vec<u8>,
}

impl Keys {
    pub fn derive(usm: &Usm, engine_id: &[u8]) -> Keys {
        let auth = if usm.level.has_auth() {
            usm.auth_protocol.localize(
                &usm.auth_protocol
                    .password_to_key(usm.auth_password.as_bytes()),
                engine_id,
            )
        } else {
            Vec::new()
        };
        let privacy = if usm.level.has_priv() {
            usm.priv_protocol.localized_key(
                usm.auth_protocol,
                usm.effective_priv_password().as_bytes(),
                engine_id,
            )
        } else {
            Vec::new()
        };
        Keys { auth, privacy }
    }
}

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Keys { .. }")
    }
}

/// Hex text of bytes, such as an engine ID: `80001f8803...`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes from hex text, with an optional `0x` and `:` or space separators.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    let digits: String = text
        .chars()
        .filter(|c| !matches!(c, ':' | ' ' | '-'))
        .collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return Err(format!("{text:?} is not an even number of hex digits"));
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| format!("{text:?} is not hex"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENGINE: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];

    #[test]
    fn rfc3414_md5_key_localization() {
        // RFC 3414, A.3.1: password "maplesyrup", engine 000000000000000000000002.
        let ku = AuthProtocol::Md5.password_to_key(b"maplesyrup");
        assert_eq!(hex(&ku), "9faf3283884e92834ebc9847d8edd963");
        let kul = AuthProtocol::Md5.localize(&ku, &ENGINE);
        assert_eq!(hex(&kul), "526f5eed9fcce26f8964c2930787d82b");
    }

    #[test]
    fn rfc3414_sha_key_localization() {
        // RFC 3414, A.3.2.
        let ku = AuthProtocol::Sha1.password_to_key(b"maplesyrup");
        assert_eq!(hex(&ku), "9fb5cc0381497b3793528939ff788d5d79145211");
        let kul = AuthProtocol::Sha1.localize(&ku, &ENGINE);
        assert_eq!(hex(&kul), "6695febc9288e36282235fc7151f128497b38f3f");
    }

    #[test]
    fn rfc7860_key_lengths_and_macs() {
        for protocol in AuthProtocol::ALL {
            let kul = protocol.localize(&protocol.password_to_key(b"maplesyrup"), &ENGINE);
            assert_eq!(protocol.mac(&kul, b"message").len(), protocol.mac_len());
        }
        // RFC 4231, test case 1, truncated to 24 bytes as RFC 7860 does.
        let key = [0x0bu8; 20];
        assert_eq!(
            hex(&AuthProtocol::Sha256.mac(&key, b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da7"
        );
    }

    #[test]
    fn priv_keys_are_extended_when_needed() {
        let blumenthal =
            PrivProtocol::Aes256.localized_key(AuthProtocol::Md5, b"maplesyrup", &ENGINE);
        let reeder =
            PrivProtocol::Aes256Cisco.localized_key(AuthProtocol::Md5, b"maplesyrup", &ENGINE);
        assert_eq!(blumenthal.len(), 32);
        assert_eq!(reeder.len(), 32);
        // Both start with the plain localized key and differ after it.
        assert_eq!(hex(&blumenthal[..16]), "526f5eed9fcce26f8964c2930787d82b");
        assert_eq!(blumenthal[..16], reeder[..16]);
        assert_ne!(blumenthal[16..], reeder[16..]);
        // Blumenthal: the second half is the hash of the first.
        assert_eq!(
            blumenthal[16..],
            AuthProtocol::Md5.hash(&[&blumenthal[..16]])[..]
        );
        // With SHA-512 the key is long enough and both agree.
        assert_eq!(
            PrivProtocol::Aes256.localized_key(AuthProtocol::Sha512, b"maplesyrup", &ENGINE),
            PrivProtocol::Aes256Cisco.localized_key(AuthProtocol::Sha512, b"maplesyrup", &ENGINE)
        );
    }

    #[test]
    fn aes_cfb_matches_nist() {
        // NIST SP 800-38A, F.3.13 (CFB128-AES128.Encrypt), first two blocks.
        let key = parse_hex("2b7e151628aed2a6abf7158809cf4f3c").unwrap();
        let iv: [u8; 16] = parse_hex("000102030405060708090a0b0c0d0e0f")
            .unwrap()
            .try_into()
            .unwrap();
        let mut data =
            parse_hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51").unwrap();
        aes_cfb(&key, iv, &mut data, true);
        assert_eq!(
            hex(&data),
            "3b3fd92eb72dad20333449f8e83cfb4ac8a64537a0b3a93fcde3cdad9f1ce58b"
        );
        aes_cfb(&key, iv, &mut data, false);
        assert_eq!(
            hex(&data),
            "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51"
        );
    }

    #[test]
    fn encryption_round_trips() {
        let plaintext = b"a scoped PDU of an odd length".to_vec();
        for protocol in PrivProtocol::ALL {
            let key = protocol.localized_key(AuthProtocol::Sha1, b"secret123", &ENGINE);
            let (ciphertext, salt) = protocol.encrypt(&key, 7, 1234, 99, &plaintext);
            assert_ne!(ciphertext[..plaintext.len()], plaintext[..], "{protocol}");
            let back = protocol.decrypt(&key, 7, 1234, &salt, &ciphertext).unwrap();
            assert_eq!(back[..plaintext.len()], plaintext[..], "{protocol}");
        }
    }

    #[test]
    fn names_parse() {
        for protocol in AuthProtocol::ALL {
            assert_eq!(protocol.name().parse::<AuthProtocol>().unwrap(), protocol);
        }
        for protocol in PrivProtocol::ALL {
            assert_eq!(protocol.name().parse::<PrivProtocol>().unwrap(), protocol);
        }
        for level in SecurityLevel::ALL {
            assert_eq!(level.name().parse::<SecurityLevel>().unwrap(), level);
        }
        assert_eq!("sha1".parse::<AuthProtocol>().unwrap(), AuthProtocol::Sha1);
        assert_eq!(
            "aes128".parse::<PrivProtocol>().unwrap(),
            PrivProtocol::Aes128
        );
        assert_eq!(
            "aes256c".parse::<PrivProtocol>().unwrap(),
            PrivProtocol::Aes256Cisco
        );
        assert!("rot13".parse::<PrivProtocol>().is_err());
    }

    #[test]
    fn settings_are_checked() {
        assert!(Usm::new("").check().is_err());
        assert!(Usm::new("bob").check().is_ok());
        assert!(
            Usm::new("bob")
                .with_auth(AuthProtocol::Sha256, "short")
                .check()
                .is_err()
        );
        let usm = Usm::new("bob")
            .with_auth(AuthProtocol::Sha256, "long enough")
            .with_privacy(PrivProtocol::Aes128, "");
        assert_eq!(usm.level, SecurityLevel::AuthPriv);
        assert!(usm.check().is_ok(), "the auth password is used for privacy");
        assert!(!format!("{usm:?}").contains("long enough"));
        assert_eq!(usm.summary(), "user bob, authPriv, SHA-256/AES");
    }

    #[test]
    fn hex_text() {
        assert_eq!(parse_hex("0x80:00:1f").unwrap(), [0x80, 0x00, 0x1f]);
        assert_eq!(hex(&[0x80, 0x00, 0x1f]), "80001f");
        assert!(parse_hex("abc").is_err());
        assert!(parse_hex("zz").is_err());
    }
}
