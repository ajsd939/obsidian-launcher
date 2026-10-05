use crate::files::base_dir;
use crate::models::Account;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStore {
    pub accounts: Vec<Account>,
    pub active: Option<String>,
}

fn store_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(base_dir(app)?.join("accounts.json"))
}

fn load_store(app: &tauri::AppHandle) -> Result<AccountStore, String> {
    let p = store_path(app)?;
    if !p.exists() {
        return Ok(AccountStore {
            accounts: vec![],
            active: None,
        });
    }
    let raw = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    Ok(serde_json::from_str(&raw).unwrap_or(AccountStore {
        accounts: vec![],
        active: None,
    }))
}

fn save_store(app: &tauri::AppHandle, store: &AccountStore) -> Result<(), String> {
    let p = store_path(app)?;
    std::fs::write(&p, serde_json::to_string_pretty(store).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn offline_uuid(username: &str) -> String {
    let name = format!("OfflinePlayer:{username}");
    Uuid::new_v3(&Uuid::NAMESPACE_DNS, name.as_bytes())
        .hyphenated()
        .to_string()
}

fn validate_username(username: &str) -> Result<String, String> {
    let name = username.trim();
    if name.is_empty() || name.len() < 3 || name.len() > 16 {
        return Err("Username must be 3-16 characters".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Only a-z, 0-9 and _ allowed".into());
    }
    Ok(name.to_string())
}

#[tauri::command]
pub async fn list_accounts(app: tauri::AppHandle) -> Result<AccountStore, String> {
    let mut store = load_store(&app)?;
    // drop active pointer if account vanished
    if let Some(active) = store.active.clone() {
        if !store.accounts.iter().any(|a| a.username == active) {
            store.active = store.accounts.first().map(|a| a.username.clone());
            save_store(&app, &store)?;
        }
    }
    Ok(store)
}

#[tauri::command]
pub async fn add_account(app: tauri::AppHandle, username: String) -> Result<AccountStore, String> {
    let name = validate_username(&username)?;
    let mut store = load_store(&app)?;
    if let Some(existing) = store.accounts.iter().find(|a| a.username.eq_ignore_ascii_case(&name)) {
        store.active = Some(existing.username.clone());
    } else {
        store.accounts.push(Account {
            username: name.clone(),
            uuid: offline_uuid(&name),
        });
        store.active = Some(name);
    }
    store.accounts.sort_by(|a, b| a.username.cmp(&b.username));
    save_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
pub async fn switch_account(app: tauri::AppHandle, username: String) -> Result<AccountStore, String> {
    let mut store = load_store(&app)?;
    if !store.accounts.iter().any(|a| a.username == username) {
        return Err("account not found".into());
    }
    store.active = Some(username);
    save_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
pub async fn remove_account(app: tauri::AppHandle, username: String) -> Result<AccountStore, String> {
    let mut store = load_store(&app)?;
    store.accounts.retain(|a| a.username != username);
    if store.active.as_deref() == Some(&username) {
        store.active = store.accounts.first().map(|a| a.username.clone());
    }
    save_store(&app, &store)?;
    Ok(store)
}
