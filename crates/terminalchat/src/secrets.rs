//! Where the F-List password is kept between starts (`password_storage` in config.json):
//!
//! - `flat`: in config.json as plain text (as before, permissions 600).
//! - `onetime`: encrypted in the RAM folder of the session (`$XDG_RUNTIME_DIR`); the key depends on
//!   the boot, so it is gone after a reboot. On systems without a boot id (Windows) it lives only
//!   while the client runs.
//! - `masterpass`: encrypted in config.json with a key from a master password (Argon2id), which is
//!   asked at every start.
//! - `systemstore`: in the password manager of the desktop (Secret Service such as KWallet or
//!   GNOME Keyring, Windows Credential Manager, macOS Keychain).

use std::fs;
use std::path::PathBuf;

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};

pub const FLAT: &str = "flat";
pub const ONETIME: &str = "onetime";
pub const MASTERPASS: &str = "masterpass";
pub const SYSTEMSTORE: &str = "systemstore";
pub const METHODS: [&str; 4] = [FLAT, ONETIME, MASTERPASS, SYSTEMSTORE];

const SERVICE: &str = "terminalchat";
const MASTER_PREFIX: &str = "v1:";

pub fn describe(method: &str) -> &'static str {
    match method {
        FLAT => "plain text in config.json",
        ONETIME if onetime_persists() => "encrypted in memory until reboot",
        ONETIME => "only while the client runs",
        MASTERPASS => "encrypted with a master password",
        SYSTEMSTORE => "system password manager",
        _ => "",
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes).expect("no random numbers from the system");
    bytes
}

fn seal(key: &[u8; 32], plain: &str) -> Vec<u8> {
    let nonce = random::<12>();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(Nonce::from_slice(&nonce), plain.as_bytes()).expect("encryption cannot fail"));
    out
}

fn open(key: &[u8; 32], data: &[u8]) -> Option<String> {
    if data.len() < 12 {
        return None;
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let plain = cipher.decrypt(Nonce::from_slice(&data[..12]), &data[12..]).ok()?;
    String::from_utf8(plain).ok()
}

// ------------------------------------------------------------------ masterpass

fn master_key(master: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    argon2::Argon2::default().hash_password_into(master.as_bytes(), salt, &mut key).expect("valid argon2 parameters");
    key
}

/// The password encrypted with the master password, as text for config.json.
pub fn lock_with_master(password: &str, master: &str) -> String {
    let salt = random::<16>();
    let mut data = salt.to_vec();
    data.extend(seal(&master_key(master, &salt), password));
    format!("{MASTER_PREFIX}{}", base64::engine::general_purpose::STANDARD.encode(data))
}

/// None for a wrong master password or a damaged entry.
pub fn unlock_with_master(secret: &str, master: &str) -> Option<String> {
    let data = base64::engine::general_purpose::STANDARD.decode(secret.strip_prefix(MASTER_PREFIX)?).ok()?;
    if data.len() < 16 {
        return None;
    }
    open(&master_key(master, &data[..16]), &data[16..])
}

// ------------------------------------------------------------------ onetime

fn boot_id() -> Option<String> {
    let id = fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_string())
}

fn user_name() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default()
}

/// Key for the onetime file: changes with every boot, differs per machine, user and account.
fn onetime_key(account: &str) -> Option<[u8; 32]> {
    let boot = boot_id()?;
    let machine = fs::read_to_string("/etc/machine-id").unwrap_or_default();
    let mut hash = Sha256::new();
    for part in ["terminalchat-onetime", &boot, machine.trim(), &user_name(), &account.to_lowercase()] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    Some(hash.finalize().into())
}

fn onetime_file(account: &str) -> PathBuf {
    let base = match std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir).join(SERVICE),
        None => std::env::temp_dir().join(format!("{SERVICE}-{}", user_name())),
    };
    let name: String = Sha256::digest(account.to_lowercase().as_bytes()).iter().take(8).map(|byte| format!("{byte:02x}")).collect();
    base.join(format!("{name}.pw"))
}

/// Whether onetime survives a restart of the client here (it needs a boot id).
pub fn onetime_persists() -> bool {
    boot_id().is_some()
}

pub fn onetime_save(account: &str, password: &str) -> Result<(), String> {
    let Some(key) = onetime_key(account) else { return Ok(()) };
    let path = onetime_file(account);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        }
    }
    fs::write(&path, seal(&key, password)).map_err(|error| format!("{}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn onetime_load(account: &str) -> Option<String> {
    let key = onetime_key(account)?;
    open(&key, &fs::read(onetime_file(account)).ok()?)
}

pub fn onetime_forget(account: &str) {
    let _ = fs::remove_file(onetime_file(account));
}

// ------------------------------------------------------------------ systemstore

/// Whether a system password manager can be reached. Blocks briefly (D-Bus); call it off the UI.
pub fn system_available() -> bool {
    if cfg!(all(unix, not(target_os = "macos"))) && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none_or(|value| value.is_empty()) {
        return false;
    }
    keyring::Entry::store_status().is_ok()
}

fn entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &account.to_lowercase()).map_err(|error| format!("password manager: {error}"))
}

pub fn system_save(account: &str, password: &str) -> Result<(), String> {
    entry(account)?.set_password(password).map_err(|error| format!("password manager: {error}"))
}

pub fn system_load(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("password manager: {error}")),
    }
}

pub fn system_forget(account: &str) {
    if system_available() {
        if let Ok(entry) = entry(account) {
            let _ = entry.delete_credential();
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn the_master_password_opens_only_its_own_lock() {
        let secret = lock_with_master("hunter2 ♥", "my master");
        assert!(secret.starts_with(MASTER_PREFIX));
        assert_eq!(unlock_with_master(&secret, "my master").as_deref(), Some("hunter2 ♥"));
        assert_eq!(unlock_with_master(&secret, "wrong"), None);
        assert_eq!(unlock_with_master("v1:broken", "my master"), None);
        assert_ne!(lock_with_master("hunter2 ♥", "my master"), secret, "every lock uses a new salt and nonce");
    }


    #[test]
    fn onetime_keeps_the_password_until_it_is_forgotten() {
        if !onetime_persists() {
            return;
        }
        let account = format!("test-account-{}", std::process::id());
        onetime_save(&account, "secret").unwrap();
        assert_eq!(onetime_load(&account).as_deref(), Some("secret"));
        let stored = fs::read(onetime_file(&account)).unwrap();
        assert!(!String::from_utf8_lossy(&stored).contains("secret"));
        onetime_forget(&account);
        assert_eq!(onetime_load(&account), None);
    }
}
