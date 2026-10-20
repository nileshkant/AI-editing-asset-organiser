//! Thin IPC adapter for SS-049 provider settings. Rules live in
//! soundshelf_core::ai_provider; this module adds the OS credential store.
//! Keys arrive from the renderer only as `set_key` input and are never returned.
use crate::service::AppState;
use soundshelf_core::ai_provider::{
    ProviderDraft, ProviderError, ProviderView, Secret, SecretStore, StoreChoice, TestReport, KEYCHAIN_SERVICE,
};

/// macOS Keychain, Windows Credential Manager or the Linux Secret Service.
pub struct OsKeychain;
fn keychain_error(error: keyring::Error) -> ProviderError {
    match error {
        keyring::Error::NoStorageAccess(_) => ProviderError::KeychainDenied,
        // Platform messages can name the account; keep only the category.
        keyring::Error::PlatformFailure(_) => ProviderError::KeychainUnavailable("the platform credential service failed".into()),
        _ => ProviderError::KeychainUnavailable("the stored credential could not be used".into()),
    }
}
impl SecretStore for OsKeychain {
    fn get(&self, account: &str) -> Result<Option<Secret>, ProviderError> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(keychain_error)?;
        match entry.get_password() {
            Ok(raw) => Secret::new(&raw).map(Some).map_err(|_| ProviderError::KeyMissing),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error(e)),
        }
    }
    fn set(&self, account: &str, secret: &Secret) -> Result<(), ProviderError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, account).and_then(|e| e.set_password(secret.expose())).map_err(keychain_error)
    }
    fn delete(&self, account: &str) -> Result<(), ProviderError> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(keychain_error)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error(e)),
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ProviderError> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|_| "Provider operation failed".to_owned())?.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ai_providers(state: tauri::State<'_, AppState>) -> Result<Vec<ProviderView>, String> {
    let p = state.providers.clone();
    blocking(move || p.list()).await
}
#[tauri::command]
pub async fn ai_provider_create(state: tauri::State<'_, AppState>, draft: ProviderDraft) -> Result<ProviderView, String> {
    let p = state.providers.clone();
    blocking(move || p.create(&draft)).await
}
#[tauri::command]
pub async fn ai_provider_update(state: tauri::State<'_, AppState>, id: String, revision: u64, draft: ProviderDraft) -> Result<ProviderView, String> {
    let p = state.providers.clone();
    blocking(move || p.update(&id, revision, &draft)).await
}
#[tauri::command]
pub async fn ai_provider_delete(state: tauri::State<'_, AppState>, id: String, revision: u64, confirmed: bool) -> Result<(), String> {
    if !confirmed {
        return Err("Confirm removing this provider".into());
    }
    let p = state.providers.clone();
    blocking(move || p.delete(&id, revision)).await
}
#[tauri::command]
pub async fn ai_provider_set_key(state: tauri::State<'_, AppState>, id: String, revision: u64, key: String, store: StoreChoice) -> Result<ProviderView, String> {
    let p = state.providers.clone();
    blocking(move || {
        let mut key = key;
        let result = p.set_key(&id, revision, &key, store);
        // Zero bytes are valid UTF-8; clear the IPC copy of the key.
        unsafe { key.as_mut_vec() }.fill(0);
        result
    })
    .await
}
#[tauri::command]
pub async fn ai_provider_clear_key(state: tauri::State<'_, AppState>, id: String, revision: u64) -> Result<ProviderView, String> {
    let p = state.providers.clone();
    blocking(move || p.clear_key(&id, revision)).await
}
/// Explicit, user-initiated connection test; the only command that contacts a provider.
#[tauri::command]
pub async fn ai_provider_test(state: tauri::State<'_, AppState>, id: String, revision: u64, embeddings: bool) -> Result<TestReport, String> {
    let p = state.providers.clone();
    blocking(move || p.test(&id, revision, embeddings)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The real OS store either round-trips and deletes a key, or reports an
    /// unavailable/denied keychain (headless CI without a secret service).
    #[test]
    fn os_keychain_round_trips_or_reports_a_typed_failure() {
        let account = format!("creativeshelf-test-{}", uuid::Uuid::new_v4());
        let secret = Secret::new("sk-keychain-test-0000").unwrap();
        match OsKeychain.set(&account, &secret) {
            Ok(()) => {
                assert_eq!(OsKeychain.get(&account).unwrap().as_ref().map(Secret::expose), Some("sk-keychain-test-0000"));
                OsKeychain.delete(&account).unwrap();
                assert_eq!(OsKeychain.get(&account).unwrap(), None);
                OsKeychain.delete(&account).unwrap();
            }
            Err(e) => {
                eprintln!("OS keychain unavailable here: {e}");
                assert!(matches!(e, ProviderError::KeychainUnavailable(_) | ProviderError::KeychainDenied), "{e:?}");
            }
        }
    }
}
