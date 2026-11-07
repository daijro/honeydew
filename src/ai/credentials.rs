//! Provider/endpoint-scoped native secrets. Never falls back to a file or a mock store.
use super::Profile;
use sha2::{Digest, Sha256};

const SERVICE: &str = "Honeydew.AI.APIKey.v1";

pub fn supported() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd"
    ))
}

pub fn store_name() -> &'static str {
    if cfg!(any(target_os = "macos", target_os = "ios")) {
        "Apple Keychain"
    } else if cfg!(target_os = "windows") {
        "Windows Credential Manager"
    } else if cfg!(any(target_os = "linux", target_os = "freebsd")) {
        "the desktop Secret Service keyring"
    } else {
        "secure storage (unavailable on this platform)"
    }
}

/// Uses the same URL validation/canonicalization as the HTTP client. Changing only model does not change keys.
pub fn endpoint(profile: &Profile) -> Result<String, String> {
    Ok(super::client::base(profile)?.to_string())
}

pub fn fingerprint(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}

pub fn account(profile: &Profile) -> Result<String, String> {
    let provider =
        serde_json::to_string(&profile.provider).map_err(|_| "Invalid provider.".to_owned())?;
    let identity = format!("{provider}\n{}", endpoint(profile)?);
    Ok(format!("provider-endpoint-{}", fingerprint(&identity)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Load,
    Save,
    Delete,
}

#[derive(Debug)]
pub enum StoreError {
    Missing,
    Unavailable,
}

/// Injectable for lifecycle tests; production always selects the platform's real secure store.
pub trait Store {
    fn get(&self, account: &str) -> Result<String, StoreError>;
    fn set(&self, account: &str, key: &str) -> Result<(), StoreError>;
    fn delete(&self, account: &str) -> Result<(), StoreError>;
}

pub struct NativeStore;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd"
))]
impl NativeStore {
    fn entry(account: &str) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(SERVICE, account).map_err(|_| StoreError::Unavailable)
    }
    fn error(error: keyring::Error) -> StoreError {
        // Error variants can contain raw credential bytes. Never format/log them or return them to the UI.
        match error {
            keyring::Error::NoEntry => StoreError::Missing,
            _ => StoreError::Unavailable,
        }
    }
}
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd"
))]
impl Store for NativeStore {
    fn get(&self, account: &str) -> Result<String, StoreError> {
        Self::entry(account)?.get_password().map_err(Self::error)
    }
    fn set(&self, account: &str, key: &str) -> Result<(), StoreError> {
        Self::entry(account)?.set_password(key).map_err(Self::error)
    }
    fn delete(&self, account: &str) -> Result<(), StoreError> {
        Self::entry(account)?
            .delete_credential()
            .map_err(Self::error)
    }
}
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd"
)))]
impl Store for NativeStore {
    fn get(&self, _: &str) -> Result<String, StoreError> {
        Err(StoreError::Unavailable)
    }
    fn set(&self, _: &str, _: &str) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
    fn delete(&self, _: &str) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
}

pub fn perform(
    store: &impl Store,
    operation: Operation,
    profile: &Profile,
) -> Result<Option<String>, String> {
    let account = account(profile)?;
    let result = match operation {
        Operation::Load => store.get(&account).map(Some),
        Operation::Save if profile.api_key.trim().is_empty() => {
            return Err("Enter an API key before saving it.".into());
        }
        Operation::Save => store.set(&account, &profile.api_key).map(|_| None),
        Operation::Delete => match store.delete(&account) {
            Ok(()) | Err(StoreError::Missing) => Ok(None),
            Err(error) => Err(error),
        },
    };
    result.map_err(|error| match (operation, error) {
        (Operation::Load, StoreError::Missing) => "No saved key was found for this provider and URL. Enter a key to use this session.".into(),
        (Operation::Delete, _) => format!("Could not remove the saved key from {}. Unlock or enable the store and retry Forget API key; the saved copy may still exist.", store_name()),
        _ => format!("Could not access {}. It may be locked, unavailable, or access was denied. Your key remains session-only; unlock or enable the store and retry.", store_name()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::Provider;
    use std::cell::RefCell;
    use std::collections::HashMap;
    #[derive(Default)]
    struct Memory(RefCell<HashMap<String, String>>);
    impl Store for Memory {
        fn get(&self, account: &str) -> Result<String, StoreError> {
            self.0
                .borrow()
                .get(account)
                .cloned()
                .ok_or(StoreError::Missing)
        }
        fn set(&self, account: &str, key: &str) -> Result<(), StoreError> {
            self.0.borrow_mut().insert(account.into(), key.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<(), StoreError> {
            self.0
                .borrow_mut()
                .remove(account)
                .map(|_| ())
                .ok_or(StoreError::Missing)
        }
    }
    struct Locked;
    impl Store for Locked {
        fn get(&self, _: &str) -> Result<String, StoreError> {
            Err(StoreError::Unavailable)
        }
        fn set(&self, _: &str, _: &str) -> Result<(), StoreError> {
            Err(StoreError::Unavailable)
        }
        fn delete(&self, _: &str) -> Result<(), StoreError> {
            Err(StoreError::Unavailable)
        }
    }
    #[test]
    fn saved_keys_are_isolated_by_provider_host_and_path_but_not_model() {
        let store = Memory::default();
        let mut p = Profile::new(Provider::OpenAi);
        p.api_key = "test-secret".into();
        perform(&store, Operation::Save, &p).unwrap();
        p.model = "future-model".into();
        p.base_url = "https://API.OPENAI.COM:443/v1/".into();
        assert_eq!(
            perform(&store, Operation::Load, &p).unwrap().as_deref(),
            Some("test-secret")
        );
        for (provider, url) in [
            (Provider::Compatible, "https://api.openai.com/v1"),
            (Provider::OpenAi, "https://other.example/v1"),
            (Provider::OpenAi, "https://api.openai.com/other"),
        ] {
            p.provider = provider;
            p.base_url = url.into();
            assert!(perform(&store, Operation::Load, &p).is_err());
        }
    }
    #[test]
    fn save_update_delete_and_missing_delete_work_without_a_plaintext_fallback() {
        let store = Memory::default();
        let mut p = Profile::default();
        p.api_key = "first-secret".into();
        perform(&store, Operation::Save, &p).unwrap();
        p.api_key = "replacement-secret".into();
        perform(&store, Operation::Save, &p).unwrap();
        assert_eq!(
            perform(&store, Operation::Load, &p).unwrap().as_deref(),
            Some("replacement-secret")
        );
        perform(&store, Operation::Delete, &p).unwrap();
        assert!(perform(&store, Operation::Load, &p).is_err());
        perform(&store, Operation::Delete, &p).unwrap();
        for op in [Operation::Load, Operation::Save, Operation::Delete] {
            let error = perform(&Locked, op, &p).unwrap_err();
            assert!(!error.contains("replacement-secret"));
        }
        assert!(store.0.borrow().is_empty());
    }
    #[test]
    #[ignore = "Opt-in: writes only a disposable test credential to the real OS store and removes it"]
    fn native_store_round_trip() {
        let mut p = Profile::new(Provider::Compatible);
        p.base_url = format!("https://keyring-test-{}.invalid/v1", std::process::id());
        p.api_key = "Honeydew disposable credential test; not an API key".into();
        perform(&NativeStore, Operation::Save, &p).expect("native store save");
        let loaded = perform(&NativeStore, Operation::Load, &p);
        let deleted = perform(&NativeStore, Operation::Delete, &p);
        assert_eq!(loaded.unwrap().as_deref(), Some(p.api_key.as_str()));
        deleted.unwrap();
        assert!(perform(&NativeStore, Operation::Load, &p).is_err());
    }
}

/// Called before displaying a connection. A key loaded/typed for one URL cannot follow an endpoint edit.
pub fn prepare(profile: &mut Profile) -> bool {
    let current = endpoint(profile).unwrap_or_else(|_| profile.base_url.trim().to_owned());
    if profile.key_state.endpoint.is_empty() {
        profile.key_state.endpoint = current;
        return false;
    }
    if profile.key_state.endpoint == current {
        return false;
    }
    profile.api_key.clear();
    profile.remember_key = false;
    profile.key_state = super::KeyState {
        endpoint: current,
        notice: "API URL changed. Enter a key for this endpoint, or load its saved key. The old endpoint's saved key is unchanged.".into(),
        ..Default::default()
    };
    true
}

/// Apply a worker's result only to the same provider/endpoint. Failed deletes never pretend the stored copy is gone.
pub fn complete(
    profile: &mut Profile,
    source: &Profile,
    operation: Operation,
    result: Result<Option<String>, String>,
) {
    if profile.provider != source.provider || endpoint(profile).ok() != endpoint(source).ok() {
        return;
    }
    profile.key_state.attempted_load = true;
    match result {
        Ok(key) => match operation {
            Operation::Load => {
                if let Some(key) = key {
                    profile.key_state.saved_hash = Some(fingerprint(&key));
                    profile.api_key = key;
                    profile.remember_key = true;
                    profile.key_state.notice = format!("Loaded securely from {}.", store_name());
                }
            }
            Operation::Save => {
                profile.remember_key = true;
                profile.key_state.saved_hash = Some(fingerprint(&source.api_key));
                profile.key_state.notice = format!("API key saved securely in {}.", store_name());
            }
            Operation::Delete => {
                profile.remember_key = false;
                profile.key_state.saved_hash = None;
                profile.key_state.notice =
                    "Saved copy removed. Any key still entered is session-only.".into();
            }
        },
        Err(error) => {
            // An update failure can leave the previous saved version in the store.
            // An explicit opt-out must stop future automatic loads even if the OS refused deletion.
            profile.remember_key = operation != Operation::Delete && source.remember_key;
            profile.key_state.notice = if operation == Operation::Save && source.remember_key {
                format!(
                    "Could not update the saved key. The previous saved version may still exist; the edited key is session-only. {error}"
                )
            } else {
                error
            };
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::ai::{Config, Provider};
    #[test]
    fn url_changes_clear_active_keys_without_copying_them_to_new_endpoints() {
        let mut p = Profile::default();
        prepare(&mut p);
        p.api_key = "bound-secret".into();
        p.remember_key = true;
        let original = account(&p).unwrap();
        p.model = "new-model".into();
        assert!(!prepare(&mut p));
        assert_eq!(p.api_key, "bound-secret");
        p.base_url = "https://another.example/v1".into();
        assert!(prepare(&mut p));
        assert!(p.api_key.is_empty());
        assert!(!p.remember_key);
        assert_ne!(account(&p).unwrap(), original);
    }
    #[test]
    fn successful_load_and_save_are_remembered_but_failed_save_is_session_only() {
        let mut p = Profile::default();
        prepare(&mut p);
        p.api_key = "typed-key".into();
        let source = p.clone();
        complete(
            &mut p,
            &source,
            Operation::Save,
            Err("store is locked".into()),
        );
        assert!(!p.remember_key);
        assert_eq!(p.api_key, "typed-key");
        complete(&mut p, &source, Operation::Save, Ok(None));
        assert!(p.remember_key);
        assert!(p.key_state.saved_hash.is_some());
        let mut loaded: Profile =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        prepare(&mut loaded);
        let source = loaded.clone();
        complete(
            &mut loaded,
            &source,
            Operation::Load,
            Ok(Some("typed-key".into())),
        );
        assert_eq!(loaded.api_key, "typed-key");
        assert!(loaded.remember_key);
    }
    #[test]
    fn failed_delete_stops_auto_load_but_reports_the_saved_copy_may_remain() {
        let mut p = Profile::default();
        prepare(&mut p);
        p.api_key = "current-key".into();
        p.remember_key = true;
        let source = p.clone();
        complete(
            &mut p,
            &source,
            Operation::Delete,
            Err("access denied; saved key may still exist".into()),
        );
        assert!(!p.remember_key);
        assert!(p.key_state.notice.contains("may still exist"));
        complete(&mut p, &source, Operation::Delete, Ok(None));
        assert!(!p.remember_key);
        assert_eq!(p.api_key, "current-key");
        assert!(p.key_state.saved_hash.is_none());
    }
    #[test]
    fn delayed_load_cannot_overwrite_another_provider_or_endpoint() {
        let source = Profile::default();
        let mut p = Profile::new(Provider::Compatible);
        complete(
            &mut p,
            &source,
            Operation::Load,
            Ok(Some("stale-key".into())),
        );
        assert!(p.api_key.is_empty());
        p.provider = source.provider;
        p.base_url = "https://other.example/v1".into();
        complete(
            &mut p,
            &source,
            Operation::Load,
            Ok(Some("stale-key".into())),
        );
        assert!(p.api_key.is_empty());
    }
    #[test]
    fn preferences_keep_consent_but_never_secret_values_or_runtime_state() {
        let mut config = Config::default();
        let p = config.profile();
        p.remember_key = true;
        p.api_key = "never-write-this".into();
        prepare(p);
        let value = serde_json::to_value(&config).unwrap();
        assert!(!value.to_string().contains("never-write-this"));
        let p = &value["profiles"][0];
        assert_eq!(p["remember_key"], true);
        assert!(p.get("api_key").is_none());
        assert!(p.get("key_state").is_none());
    }
}
