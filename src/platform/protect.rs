//! Protecting a secret with the TPM chip and with the user's Windows account.

use std::{ptr, slice};

use anyhow::{Context as _, Result};
use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree, NTE_BAD_KEYSET},
        Security::Cryptography::{
            BCRYPT_OAEP_PADDING_INFO, BCRYPT_SHA256_ALGORITHM, CERT_KEY_SPEC, CRYPT_INTEGER_BLOB,
            CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
            MS_PLATFORM_CRYPTO_PROVIDER, NCRYPT_ALLOW_DECRYPT_FLAG, NCRYPT_FLAGS, NCRYPT_HANDLE,
            NCRYPT_KEY_HANDLE, NCRYPT_KEY_USAGE_PROPERTY, NCRYPT_LENGTH_PROPERTY,
            NCRYPT_PAD_OAEP_FLAG, NCRYPT_PROV_HANDLE, NCRYPT_RSA_ALGORITHM, NCRYPT_SILENT_FLAG,
            NCryptCreatePersistedKey, NCryptDecrypt, NCryptEncrypt, NCryptFinalizeKey,
            NCryptFreeObject, NCryptOpenKey, NCryptOpenStorageProvider, NCryptSetProperty,
        },
    },
    core::{HSTRING, PCWSTR},
};

/// A CNG handle, released when dropped.
struct Handle(NCRYPT_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is owned and used nowhere after this.
        let _ = unsafe { NCryptFreeObject(self.0) };
    }
}

/// An RSA key that was generated inside the TPM and cannot leave it.
pub struct TpmKey(Handle);

impl TpmKey {
    /// Opens the user's key of this name, creating it if there is none. Fails
    /// on a PC without a usable TPM.
    pub fn open_or_create(name: &str) -> Result<Self> {
        let mut provider = NCRYPT_PROV_HANDLE::default();
        // The "platform" provider keeps its keys in the TPM and nowhere else.
        // SAFETY: valid out pointer and a static name.
        unsafe { NCryptOpenStorageProvider(&mut provider, MS_PLATFORM_CRYPTO_PROVIDER, 0) }
            .context("the TPM is not available")?;
        let _provider = Handle(NCRYPT_HANDLE(provider.0));

        let name = HSTRING::from(name);
        let mut key = NCRYPT_KEY_HANDLE::default();
        // SAFETY: valid handles and pointers; `name` outlives the call.
        let opened = unsafe {
            NCryptOpenKey(
                provider,
                &mut key,
                PCWSTR(name.as_ptr()),
                CERT_KEY_SPEC(0),
                NCRYPT_SILENT_FLAG,
            )
        };
        match opened {
            Ok(()) => return Ok(Self(Handle(NCRYPT_HANDLE(key.0)))),
            Err(error) if error.code() == NTE_BAD_KEYSET => {}
            Err(error) => return Err(error).context("could not open the TPM key"),
        }

        // SAFETY: as above.
        unsafe {
            NCryptCreatePersistedKey(
                provider,
                &mut key,
                NCRYPT_RSA_ALGORITHM,
                PCWSTR(name.as_ptr()),
                CERT_KEY_SPEC(0),
                NCRYPT_FLAGS(0),
            )
        }
        .context("the TPM could not create a key")?;
        let key = Self(Handle(NCRYPT_HANDLE(key.0)));
        // No export policy is set: TPM keys cannot be exported by default.
        for (property, value) in [
            (NCRYPT_LENGTH_PROPERTY, 2048u32),
            (NCRYPT_KEY_USAGE_PROPERTY, NCRYPT_ALLOW_DECRYPT_FLAG),
        ] {
            // SAFETY: a live key handle and a four-byte value.
            unsafe { NCryptSetProperty(key.0.0, property, &value.to_le_bytes(), NCRYPT_FLAGS(0)) }
                .context("could not configure the TPM key")?;
        }
        // SAFETY: a live key handle.
        unsafe { NCryptFinalizeKey(key.handle(), NCRYPT_SILENT_FLAG) }
            .context("the TPM could not create a key")?;
        Ok(key)
    }

    fn handle(&self) -> NCRYPT_KEY_HANDLE {
        NCRYPT_KEY_HANDLE(self.0.0.0)
    }

    /// Encrypts a short secret so that only this TPM key can read it.
    pub fn wrap(&self, secret: &[u8]) -> Result<Vec<u8>> {
        self.transform(secret, true)
            .context("the TPM key could not encrypt")
    }

    /// Reverses `wrap`. The decryption happens inside the TPM.
    pub fn unwrap(&self, wrapped: &[u8]) -> Result<Vec<u8>> {
        self.transform(wrapped, false)
            .context("the TPM could not decrypt")
    }

    fn transform(&self, input: &[u8], encrypt: bool) -> windows::core::Result<Vec<u8>> {
        let padding = BCRYPT_OAEP_PADDING_INFO {
            pszAlgId: BCRYPT_SHA256_ALGORITHM,
            pbLabel: ptr::null_mut(),
            cbLabel: 0,
        };
        let padding = Some((&raw const padding).cast());
        let flags = NCRYPT_PAD_OAEP_FLAG | NCRYPT_SILENT_FLAG;
        // Larger than any output of a 2048-bit key.
        let mut output = vec![0; 512];
        let mut length = 0;
        // SAFETY: the buffers and the padding description outlive the call.
        unsafe {
            if encrypt {
                NCryptEncrypt(
                    self.handle(),
                    Some(input),
                    padding,
                    Some(&mut output),
                    &mut length,
                    flags,
                )?;
            } else {
                NCryptDecrypt(
                    self.handle(),
                    Some(input),
                    padding,
                    Some(&mut output),
                    &mut length,
                    flags,
                )?;
            }
        }
        output.truncate(length as usize);
        Ok(output)
    }

    /// Removes the key from the TPM.
    #[cfg(test)]
    pub fn delete(self) -> Result<()> {
        use windows::Win32::Security::Cryptography::NCryptDeleteKey;

        let handle = self.handle();
        // Deleting also frees the handle.
        std::mem::forget(self);
        // SAFETY: a live key handle, not used again.
        unsafe { NCryptDeleteKey(handle, 0) }?;
        Ok(())
    }
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr().cast_mut(),
    }
}

/// Copies a result of the data protection API and frees it.
fn take(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    // SAFETY: on success the API returns `cbData` bytes that the caller frees.
    unsafe {
        let data = slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(Some(HLOCAL(output.pbData.cast())));
        data
    }
}

/// Encrypts data so that only this Windows user on this PC can read it.
pub fn seal_for_user(data: &[u8]) -> Result<Vec<u8>> {
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: the API only reads the input blob.
    unsafe {
        CryptProtectData(
            &blob(data),
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .context("Windows could not protect the key")?;
    Ok(take(output))
}

/// Reverses `seal_for_user`.
pub fn unseal_for_user(data: &[u8]) -> Result<Vec<u8>> {
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as above.
    unsafe {
        CryptUnprotectData(
            &blob(data),
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .context("Windows could not unprotect the key")?;
    Ok(take(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_account_seals_and_unseals() {
        let sealed = seal_for_user(b"secret").unwrap();
        assert_ne!(sealed, b"secret");
        assert_eq!(unseal_for_user(&sealed).unwrap(), b"secret");
        assert!(unseal_for_user(b"not sealed").is_err());
    }

    #[test]
    fn a_tpm_key_wraps_and_is_found_again() {
        let name = format!("mocli test key {}", std::process::id());
        let key = match TpmKey::open_or_create(&name) {
            Ok(key) => key,
            // Build machines have no TPM.
            Err(error) => return println!("skipped: {error:#}"),
        };
        let wrapped = key.wrap(&[7; 32]).unwrap();
        drop(key);

        let again = TpmKey::open_or_create(&name).unwrap();
        let unwrapped = again.unwrap(&wrapped);
        again.delete().unwrap();
        assert_eq!(unwrapped.unwrap(), [7; 32]);
    }
}
