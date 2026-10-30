//! The Windows Credential Manager, for SNMPv3 passwords (see `secrets.rs`).

use windows::Win32::Security::Credentials::{
    CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
    CredReadW, CredWriteW,
};
use windows::core::{HSTRING, PWSTR};

use crate::secrets::{self, Change, WithSecrets};

/// The secret stored under `key`, or `None` when there is none (or it
/// cannot be read).
pub fn read(key: &str) -> Option<String> {
    let name = HSTRING::from(key);
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: CredReadW writes a pointer to a buffer it allocates, which is
    // read and then freed with CredFree.
    unsafe {
        CredReadW(&name, CRED_TYPE_GENERIC, None, &mut credential).ok()?;
        let blob = {
            let c = &*credential;
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
        };
        CredFree(credential as *const _);
        String::from_utf8(blob).ok()
    }
}

/// Apply the changes `secrets::changes` decided on. Returns the first
/// error, after trying every change.
pub fn apply(changes: &[Change]) -> Result<(), String> {
    let mut first_error = None;
    for change in changes {
        let result = match change {
            Change::Write {
                key,
                kind,
                user,
                secret,
            } => write(key, kind.comment(), user, secret),
            Change::Delete { key } => {
                // SAFETY: deletes a credential by name; a missing one is an
                // error we do not care about.
                let _ =
                    unsafe { CredDeleteW(&HSTRING::from(key.as_str()), CRED_TYPE_GENERIC, None) };
                Ok(())
            }
        };
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn write(key: &str, comment: &str, user: &str, secret: &str) -> Result<(), String> {
    let mut name: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let mut comment: Vec<u16> = comment.encode_utf16().chain([0]).collect();
    let mut user: Vec<u16> = user.encode_utf16().chain([0]).collect();
    let mut blob = secret.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_mut_ptr()),
        Comment: PWSTR(comment.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user.as_mut_ptr()),
        ..CREDENTIALW::default()
    };
    // SAFETY: every pointer in `credential` points into a buffer that lives
    // until the call returns; CredWriteW copies them.
    let result = unsafe { CredWriteW(&credential, 0) };
    blob.fill(0);
    result.map_err(|error| format!("cannot store {key} in the Credential Manager: {error}"))
}

/// Fill in the SNMPv3 passwords of saved agents or trap users from the
/// Credential Manager.
pub fn load_passwords<T: WithSecrets>(items: &mut [T]) {
    secrets::fill(items, read);
}

/// Store the SNMPv3 passwords for the agents or trap users going from `old`
/// to `new`.
pub fn save_passwords<T: WithSecrets>(old: &[T], new: &[T]) -> Result<(), String> {
    apply(&secrets::changes(old, new))
}
