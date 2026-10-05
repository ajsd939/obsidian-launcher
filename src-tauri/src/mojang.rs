use crate::models::{Account, VersionEntry, VersionManifest};
use uuid::Uuid;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

fn offline_uuid(username: &str) -> String {
    // Minecraft offline UUID = UUID v3 (MD5) of "OfflinePlayer:<name>"
    let name = format!("OfflinePlayer:{}", username);
    let uuid = Uuid::new_v3(&Uuid::NAMESPACE_DNS, name.as_bytes());
    uuid.hyphenated().to_string()
}

#[tauri::command]
pub fn create_offline_account(username: String) -> Result<Account, String> {
    let name = username.trim();
    if name.is_empty() || name.len() < 3 || name.len() > 16 {
        return Err("Username must be 3-16 characters".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' ) {
        return Err("Only a-z, 0-9 and _ allowed".into());
    }
    Ok(Account {
        username: name.to_string(),
        uuid: offline_uuid(name),
    })
}

#[tauri::command]
pub async fn fetch_version_manifest() -> Result<VersionManifest, String> {
    let resp = reqwest::get(MANIFEST_URL)
        .await
        .map_err(|e| format!("manifest fetch failed: {e}"))?;
    let manifest: VersionManifest = resp
        .json()
        .await
        .map_err(|e| format!("manifest parse failed: {e}"))?;
    Ok(manifest)
}

#[tauri::command]
pub async fn fetch_version_list(filter: Option<String>) -> Result<Vec<VersionEntry>, String> {
    let manifest = fetch_version_manifest().await?;
    let out: Vec<VersionEntry> = match filter.as_deref() {
        Some("release") => manifest.versions.into_iter().filter(|v| v.version_type == "release").collect(),
        Some("snapshot") => manifest.versions.into_iter().filter(|v| v.version_type != "release").collect(),
        _ => manifest.versions,
    };
    Ok(out.into_iter().take(200).collect())
}
