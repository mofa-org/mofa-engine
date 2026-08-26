//! OS-keychain secret storage (PLAT-04: 密钥仅存 OS 钥匙串).
//!
//! Provider credentials can be referenced from config as
//! `api_key = "keychain:ACCOUNT"` — the file then holds only the account
//! name, and the secret itself lives in the operating system's credential
//! store (macOS Keychain, Windows Credential Manager, Linux Secret Service).
//! The [`crate::config`] loader resolves the reference at parse time, so the
//! rest of the engine only ever sees the real key in memory.
//!
//! When no OS credential store is available (headless Linux without a
//! secret service, minimal container images) every operation returns an
//! honest error and config loading fails loudly for keychain-referenced
//! keys — we never silently fall back to plaintext.

/// The service name all MoFA entries live under in the OS credential store.
const SERVICE: &str = "mofa-engine";

// Tests need a PERSISTENT in-memory stand-in: keyring's own mock keeps the
// password inside each Entry (EntryOnly persistence), so a
// store→new-Entry→load round trip always reads back empty. This map mimics
// a real keychain instead: process-wide, keyed by account.
#[cfg(test)]
static MOCK_INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
static MOCK_MAP: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> =
    std::sync::Mutex::new(None);

/// Store `secret` under `account`, replacing any previous value.
pub fn store(account: &str, secret: &str) -> Result<(), String> {
    validate_account(account)?;
    #[cfg(test)]
    if mock_active() {
        mock_map()
            .get_or_insert_with(Default::default)
            .insert(account.to_string(), secret.to_string());
        return Ok(());
    }
    entry(account)?
        .set_password(secret)
        .map_err(|e| format!("keychain store '{account}' failed: {e}"))
}

/// Load the secret stored under `account`; `None` when no entry exists.
pub fn load(account: &str) -> Result<Option<String>, String> {
    validate_account(account)?;
    #[cfg(test)]
    if mock_active() {
        return Ok(mock_map().as_ref().and_then(|m| m.get(account).cloned()));
    }
    match entry(account)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keychain load '{account}' failed: {e}")),
    }
}

/// Delete the entry (idempotent: a missing entry is success).
pub fn delete(account: &str) -> Result<(), String> {
    validate_account(account)?;
    #[cfg(test)]
    if mock_active() {
        if let Some(map) = mock_map().as_mut() {
            map.remove(account);
        }
        return Ok(());
    }
    match entry(account)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keychain delete '{account}' failed: {e}")),
    }
}

/// Whether an OS credential store is reachable at all (probe write+delete).
/// Always true under the test stand-in.
pub fn available() -> bool {
    #[cfg(test)]
    if mock_active() {
        return true;
    }
    let probe = format!("mofa-probe-{}", uuid::Uuid::new_v4());
    match entry(&probe) {
        Ok(e) => e
            .set_password("probe")
            .and_then(|_| e.delete_credential())
            .is_ok(),
        Err(_) => false,
    }
}

fn validate_account(account: &str) -> Result<(), String> {
    if account.trim().is_empty() {
        return Err("keychain account must not be empty".into());
    }
    Ok(())
}

fn entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, account).map_err(|e| format!("OS keychain unavailable: {e}"))
}

// ==================== test stand-in ====================

/// Install the persistent in-memory stand-in for the OS keychain,
/// process-wide. Tests run in parallel and share one map, mirroring a real
/// keychain's cross-entry persistence.
#[cfg(test)]
#[doc(hidden)]
pub fn install_mock_store_once() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        MOCK_INSTALLED.store(true, std::sync::atomic::Ordering::SeqCst);
    });
}

#[cfg(test)]
fn mock_active() -> bool {
    MOCK_INSTALLED.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
fn mock_map() -> std::sync::MutexGuard<'static, Option<std::collections::HashMap<String, String>>> {
    MOCK_MAP.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_store() {
        install_mock_store_once();
    }

    #[test]
    fn store_load_delete_round_trip() {
        mock_store();
        store("test/openai", "sk-123").unwrap();
        assert_eq!(load("test/openai").unwrap().as_deref(), Some("sk-123"));

        // Overwrite replaces.
        store("test/openai", "sk-456").unwrap();
        assert_eq!(load("test/openai").unwrap().as_deref(), Some("sk-456"));

        delete("test/openai").unwrap();
        assert_eq!(load("test/openai").unwrap(), None);
    }

    #[test]
    fn delete_is_idempotent_and_missing_loads_none() {
        mock_store();
        assert_eq!(load("never/stored").unwrap(), None);
        delete("never/stored").unwrap();
    }

    #[test]
    fn empty_accounts_are_rejected() {
        mock_store();
        assert!(store("  ", "x").is_err());
        assert!(load("").is_err());
    }

    #[test]
    fn probe_reports_available_under_the_stand_in() {
        mock_store();
        assert!(available());
    }
}
