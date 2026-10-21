//! Passwords the user chose to save, in Windows Credential Manager (generic
//! credentials, encrypted for the Windows user). Visible and removable under
//! Control Panel > Credential Manager > Windows Credentials.

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

use crate::askpass::Store;

/// Stored secrets are UTF-16, as `cmdkey` and most Windows tools store them.
pub struct CredentialManager;

fn is_not_found(e: &windows::core::Error) -> bool {
    e.code() == ERROR_NOT_FOUND.to_hresult()
}

impl Store for CredentialManager {
    fn get(&self, target: &str) -> Result<Option<String>, String> {
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: on success CredReadW allocates `credential`, freed below.
        match unsafe {
            CredReadW(
                &HSTRING::from(target),
                CRED_TYPE_GENERIC,
                None,
                &mut credential,
            )
        } {
            Err(e) if is_not_found(&e) => return Ok(None),
            Err(e) => return Err(format!("cannot read the saved password: {e}")),
            Ok(()) => {}
        }
        // SAFETY: the blob is CredentialBlobSize bytes owned by `credential`.
        let units: Vec<u16> = unsafe {
            let c = &*credential;
            let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
            let units = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            CredFree(credential as *const _);
            units
        };
        String::from_utf16(&units)
            .map(Some)
            .map_err(|_| "the saved password is not valid text".to_string())
    }

    fn set(&self, target: &str, secret: &str) -> Result<(), String> {
        let mut target: Vec<u16> = target.encode_utf16().chain([0]).collect();
        let mut blob: Vec<u8> = secret.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            // This Windows user on this computer; not roamed to other PCs.
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // SAFETY: the pointers in `credential` outlive the call.
        let result = unsafe { CredWriteW(&credential, 0) };
        blob.fill(0);
        result.map_err(|e| format!("cannot save the password in Windows Credential Manager: {e}"))
    }

    fn delete(&self, target: &str) -> Result<(), String> {
        // SAFETY: plain call with an owned, NUL-terminated name.
        match unsafe { CredDeleteW(&HSTRING::from(target), CRED_TYPE_GENERIC, None) } {
            Err(e) if !is_not_found(&e) => Err(format!("cannot remove the saved password: {e}")),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_reads_and_deletes_a_generic_credential() {
        let target = format!("simter-test:{}", std::process::id());
        let store = CredentialManager;
        assert_eq!(store.get(&target).unwrap(), None);
        store.set(&target, "máy chủ pw 🔑").unwrap();
        let read = store.get(&target);
        store.delete(&target).unwrap();
        assert_eq!(read.unwrap().as_deref(), Some("máy chủ pw 🔑"));
        assert_eq!(store.get(&target).unwrap(), None);
        // Deleting what is not there is not an error.
        store.delete(&target).unwrap();
    }
}
