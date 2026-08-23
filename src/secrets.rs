//! Shared local-secret encryption used by any module that needs to keep a
//! third-party credential on disk (the AI assistant's provider API key,
//! the ntfy auth token...). Extracted from what used to be private to
//! `ai.rs` so it isn't duplicated per feature.
//!
//! Security note (same one surfaced to the user in the UI, repeated here
//! for anyone reading the code): AES-256-GCM at rest with the decryption
//! key kept in its own file (`secret_key.bin`, `chmod 600` on Unix) is real
//! protection against a stray copy of just the config file (backup, bug
//! report, `.json` synced somewhere) - but not equivalent to an OS
//! keychain/HSM, since the key lives on the same disk as the ciphertext.
//! That's an inherent limitation of a local app with no external secret
//! store, not something worth quietly overclaiming.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedBlob {
    pub nonce: String,      // base64, 12 bytes
    pub ciphertext: String, // base64
}

fn key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("secret_key.bin")
}

pub async fn restrict_to_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = tokio::fs::metadata(path).await {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = tokio::fs::set_permissions(path, perms).await;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path; // no portable equivalent; NTFS ACLs already default to owner-only for user profile dirs
    }
}

/// Loads (generating on first use) the single local AES-256 key used to
/// encrypt every secret this app keeps at rest, shared across features so
/// there's one key file to protect and back up (or lose - see `decrypt`'s
/// error message), not one per feature.
///
/// Migration note: earlier versions kept a separate `ai_key.bin` used only
/// for the AI assistant's key. If `secret_key.bin` doesn't exist yet but
/// `ai_key.bin` does, it's renamed into place rather than generating a
/// fresh key - otherwise every existing user's already-configured AI
/// provider key would silently stop decrypting (it'd just read back as
/// empty, not error) the moment they upgraded.
pub async fn load_or_create_key(data_dir: &Path) -> Result<[u8; 32]> {
    let path = key_path(data_dir);
    if !path.exists() {
        let legacy_path = data_dir.join("ai_key.bin");
        if legacy_path.exists() {
            let _ = tokio::fs::rename(&legacy_path, &path).await;
        }
    }
    if let Ok(bytes) = tokio::fs::read(&path).await {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
    }
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    tokio::fs::write(&path, key).await.context("impossible d'ecrire la cle de chiffrement locale")?;
    restrict_to_owner(&path).await;
    Ok(key)
}

pub fn encrypt(key: &[u8; 32], plaintext: &str) -> Result<EncryptedBlob> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| anyhow::anyhow!("echec du chiffrement"))?;
    Ok(EncryptedBlob { nonce: B64.encode(nonce_bytes), ciphertext: B64.encode(ciphertext) })
}

pub fn decrypt(key: &[u8; 32], blob: &EncryptedBlob) -> Result<String> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce_bytes = B64.decode(&blob.nonce).context("nonce invalide")?;
    let ciphertext = B64.decode(&blob.ciphertext).context("donnees chiffrees invalides")?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext.as_slice())
        .map_err(|_| anyhow::anyhow!("echec du dechiffrement (cle locale changee ou fichier corrompu ?)"))?;
    String::from_utf8(plaintext).context("secret dechiffre invalide (UTF-8)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut key = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut key);
        let blob = encrypt(&key, "super-secret-value").unwrap();
        assert_ne!(blob.ciphertext, "super-secret-value");
        assert_eq!(decrypt(&key, &blob).unwrap(), "super-secret-value");
    }
}
