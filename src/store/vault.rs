//! The history's encryption key, and encryption of its files.
//!
//! A random master key encrypts everything. It is stored wrapped by a key that
//! lives inside the TPM, and that result is sealed to the Windows account, so
//! reading it takes both this PC's chip and this user's sign-in. A PC without a
//! TPM relies on the account alone.

use std::{
    fmt,
    fs::{self, File},
    io::{ErrorKind, Write as _},
    path::Path,
};

use anyhow::{Context as _, Result};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead as _, KeyInit as _, Payload},
};

use crate::platform::protect::{self, TpmKey};

/// The name of mocli's key in the TPM.
pub const TPM_KEY: &str = "mocli history key";
const KEY_FILE: &str = "history.key";
const KEY_MAGIC: &[u8] = b"mocli-key-1";
const FILE_MAGIC: &[u8] = b"MCL1";
const NONCE_LENGTH: usize = 24;

/// What keeps the master key secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protector {
    /// The TPM chip and the Windows account.
    Tpm,
    /// The Windows account only.
    Account,
}

impl Protector {
    fn tag(self) -> u8 {
        match self {
            Self::Tpm => 1,
            Self::Account => 2,
        }
    }

    fn seal(self, secret: &[u8], tpm_key: &str) -> Result<Vec<u8>> {
        match self {
            Self::Tpm => {
                let wrapped = TpmKey::open_or_create(tpm_key)?.wrap(secret)?;
                protect::seal_for_user(&wrapped)
            }
            Self::Account => protect::seal_for_user(secret),
        }
    }

    fn unseal(self, sealed: &[u8], tpm_key: &str) -> Result<Vec<u8>> {
        let data = protect::unseal_for_user(sealed)?;
        match self {
            Self::Tpm => TpmKey::open_or_create(tpm_key)?.unwrap(&data),
            Self::Account => Ok(data),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MasterKey([u8; 32]);

impl fmt::Debug for MasterKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MasterKey(..)")
    }
}

impl MasterKey {
    pub fn random() -> Result<Self> {
        let mut key = [0; 32];
        getrandom::fill(&mut key).context("no random numbers")?;
        Ok(Self(key))
    }

    fn derive(&self, purpose: &str) -> [u8; 32] {
        blake3::derive_key(purpose, &self.0)
    }

    /// The database key, written the way SQLCipher takes a raw key.
    pub fn database(&self) -> String {
        let key = blake3::Hash::from_bytes(self.derive("mocli 2026 history database"));
        format!("x'{}'", key.to_hex())
    }

    /// Encrypts the files kept beside the database.
    pub fn files(&self) -> Cipher {
        Cipher {
            aead: XChaCha20Poly1305::new(&self.derive("mocli 2026 history files").into()),
            names: self.derive("mocli 2026 history file names"),
        }
    }
}

pub struct Cipher {
    aead: XChaCha20Poly1305,
    names: [u8; 32],
}

impl Cipher {
    /// A file name for `content` that does not reveal what it is made from.
    pub fn name(&self, content: &str) -> String {
        blake3::keyed_hash(&self.names, content.as_bytes())
            .to_hex()
            .to_string()
    }

    /// Encrypts the content of the file called `name`.
    pub fn seal(&self, name: &str, content: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0; NONCE_LENGTH];
        getrandom::fill(&mut nonce).context("no random numbers")?;
        let payload = Payload {
            msg: content,
            // A file moved to another name no longer opens.
            aad: name.as_bytes(),
        };
        let sealed = self
            .aead
            .encrypt(&XNonce::from(nonce), payload)
            .ok()
            .context("could not encrypt")?;
        Ok([FILE_MAGIC, &nonce, &sealed].concat())
    }

    pub fn open(&self, name: &str, data: &[u8]) -> Result<Vec<u8>> {
        let (nonce, sealed) = data
            .strip_prefix(FILE_MAGIC)
            .and_then(|rest| rest.split_first_chunk::<NONCE_LENGTH>())
            .context("not an encrypted file")?;
        let payload = Payload {
            msg: sealed,
            aad: name.as_bytes(),
        };
        self.aead
            .decrypt(&XNonce::from(*nonce), payload)
            .ok()
            .context("the file is damaged or belongs to another key")
    }
}

/// Reads the key kept in `directory`, if there is one.
pub fn load(directory: &Path, tpm_key: &str) -> Result<Option<(MasterKey, Protector)>> {
    let data = match fs::read(directory.join(KEY_FILE)) {
        Ok(data) => data,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("could not read the key file"),
    };
    let (protector, sealed) = data
        .strip_prefix(KEY_MAGIC)
        .and_then(|rest| rest.split_first())
        .and_then(|(&tag, sealed)| {
            let protector = [Protector::Tpm, Protector::Account]
                .into_iter()
                .find(|protector| protector.tag() == tag)?;
            Some((protector, sealed))
        })
        .context("the key file is damaged")?;
    let key = protector.unseal(sealed, tpm_key)?;
    let key = key.try_into().ok().context("the key file is damaged")?;
    Ok(Some((MasterKey(key), protector)))
}

/// Makes a new key and stores it in `directory`, in the TPM if there is one.
pub fn create(directory: &Path, tpm_key: &str) -> Result<(MasterKey, Protector)> {
    let key = MasterKey::random()?;
    // Nothing is encrypted with a key until reading it back has worked.
    let sealed = |protector: Protector| -> Result<Vec<u8>> {
        let sealed = protector.seal(&key.0, tpm_key)?;
        anyhow::ensure!(
            protector.unseal(&sealed, tpm_key)? == key.0,
            "the key did not survive a round trip"
        );
        Ok(sealed)
    };
    let (protector, sealed) = match sealed(Protector::Tpm) {
        Ok(data) => (Protector::Tpm, data),
        Err(error) => {
            tracing::warn!(
                error = format!("{error:#}"),
                "no TPM; the key is protected by the Windows account only"
            );
            (Protector::Account, sealed(Protector::Account)?)
        }
    };

    let path = directory.join(KEY_FILE);
    let temporary = path.with_extension("key.tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(&[KEY_MAGIC, &[protector.tag()], &sealed].concat())?;
    // The history becomes unreadable if this is lost.
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, &path).context("could not save the key")?;
    Ok((key, protector))
}

pub fn remove(directory: &Path) -> Result<()> {
    match fs::remove_file(directory.join(KEY_FILE)) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(error).context("could not delete the key file")
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::Scratch;

    #[test]
    fn files_open_only_with_their_key_and_name() {
        let cipher = MasterKey::random().unwrap().files();
        let sealed = cipher.seal("a.png", b"picture").unwrap();
        assert!(!sealed.windows(7).any(|part| part == b"picture"));
        assert_eq!(cipher.open("a.png", &sealed).unwrap(), b"picture");
        assert!(cipher.open("b.png", &sealed).is_err());
        assert!(cipher.open("a.png", b"picture").is_err());
        let other = MasterKey::random().unwrap().files();
        assert!(other.open("a.png", &sealed).is_err());
        assert_ne!(cipher.name("content"), other.name("content"));
    }

    #[test]
    fn a_created_key_loads_again() {
        let scratch = Scratch::new("vault");
        fs::create_dir_all(&scratch.0).unwrap();
        let name = format!("mocli test vault key {}", std::process::id());
        assert!(load(&scratch.0, &name).unwrap().is_none());

        let created = create(&scratch.0, &name).unwrap();
        let loaded = load(&scratch.0, &name).unwrap().unwrap();
        let stored = fs::read(scratch.0.join(KEY_FILE)).unwrap();
        if created.1 == Protector::Tpm {
            TpmKey::open_or_create(&name).unwrap().delete().unwrap();
            // Without the chip's key the file is useless.
            assert!(load(&scratch.0, &name).is_err());
            TpmKey::open_or_create(&name).unwrap().delete().unwrap();
        }

        assert_eq!(loaded, created);
        assert!(!stored.windows(32).any(|part| part == created.0.0));
        remove(&scratch.0).unwrap();
        assert!(load(&scratch.0, &name).unwrap().is_none());
    }
}
