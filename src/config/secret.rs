use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use keyring::Entry;

const SERVICE: &str = "dterminal";
const KEY_USER: &str = "secret-key";
const NONCE_LEN: usize = 12;

/// Reads the app's symmetric key from the OS keyring (libsecret/gnome-keyring
/// on Linux — the same layer Electron's `safeStorage` uses), generating and
/// storing one on first use. Returns `None` if no keyring backend is
/// available, in which case callers fall back to a tagged plaintext store —
/// the same fallback `sessionStore.js`'s `encryptSecret`/`decryptSecret` used.
fn load_or_create_key() -> Option<[u8; 32]> {
    let entry = Entry::new(SERVICE, KEY_USER).ok()?;

    if let Ok(existing) = entry.get_password() {
        if let Ok(bytes) = STANDARD.decode(existing) {
            if bytes.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                return Some(key);
            }
        }
    }

    let mut key = [0u8; 32];
    getrandom::fill(&mut key).ok()?;
    entry.set_password(&STANDARD.encode(key)).ok()?;
    Some(key)
}

pub fn encrypt_secret(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    if let Some(key_bytes) = load_or_create_key() {
        if let Ok(cipher) = Aes256Gcm::new_from_slice(&key_bytes) {
            let mut nonce_bytes = [0u8; NONCE_LEN];
            if getrandom::fill(&mut nonce_bytes).is_ok() {
                let nonce = Nonce::try_from(nonce_bytes.as_slice()).expect("nonce is 12 bytes");
                if let Ok(ciphertext) = cipher.encrypt(&nonce, plain.as_bytes()) {
                    let mut combined = nonce_bytes.to_vec();
                    combined.extend_from_slice(&ciphertext);
                    return format!("enc:{}", STANDARD.encode(combined));
                }
            }
        }
    }
    format!("plain:{plain}")
}

pub fn decrypt_secret(stored: &str) -> String {
    if stored.is_empty() {
        return String::new();
    }
    if let Some(rest) = stored.strip_prefix("enc:") {
        let decrypted = load_or_create_key().and_then(|key_bytes| {
            let combined = STANDARD.decode(rest).ok()?;
            if combined.len() <= NONCE_LEN {
                return None;
            }
            let cipher = Aes256Gcm::new_from_slice(&key_bytes).ok()?;
            let (nonce_bytes, ciphertext) = combined.split_at(NONCE_LEN);
            let nonce = Nonce::try_from(nonce_bytes).ok()?;
            let plain = cipher.decrypt(&nonce, ciphertext).ok()?;
            String::from_utf8(plain).ok()
        });
        return decrypted.unwrap_or_default();
    }
    if let Some(rest) = stored.strip_prefix("plain:") {
        return rest.to_string();
    }
    String::new()
}
