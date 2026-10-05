use crate::files::{base_dir, download_file};
use crate::loaders::maven_to_path;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::Emitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeVersion {
    pub version: String,      // forge part, e.g. "51.0.33"
    pub full: String,         // "1.21-51.0.33"
    pub installer_url: String,
}

fn emit(app: &tauri::AppHandle, stage: &str) {
    let _ = app.emit("download-progress", serde_json::json!({ "stage": stage, "current": 0, "total": 1 }));
}

fn parse_maven_versions(xml: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = xml;
    while let Some(s) = rest.find("<version>") {
        rest = &rest[s + 9..];
        if let Some(e) = rest.find("</version>") {
            out.push(rest[..e].trim().to_string());
            rest = &rest[e + 10..];
        } else {
            break;
        }
    }
    out
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("obsidian-launcher/0.1 (offline-mc-launcher)")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

#[tauri::command]
pub async fn list_forge_versions(game_version: String) -> Result<Vec<ForgeVersion>, String> {
    let url = "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml";
    let xml = http_client()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("forge metadata failed: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let mut out = vec![];
    for v in parse_maven_versions(&xml).into_iter().rev() {
        if let Some((mc, forge)) = v.split_once('-') {
            if mc == game_version {
                let full = v.clone();
                out.push(ForgeVersion {
                    version: forge.to_string(),
                    installer_url: format!(
                        "https://maven.minecraftforge.net/net/minecraftforge/forge/{full}/forge-{full}-installer.jar"
                    ),
                    full,
                });
                if out.len() >= 30 {
                    break;
                }
            }
        }
    }
    if out.is_empty() {
        return Err(format!("no Forge builds for {game_version}"));
    }
    Ok(out)
}

fn neo_major(mc: &str) -> String {
    // "1.21" / "1.21.1" -> "21", "1.20.1" -> "20", "26.1" -> "26"
    if mc.starts_with("1.") {
        mc.split('.').nth(1).unwrap_or("").to_string()
    } else {
        mc.split('.').next().unwrap_or("").to_string()
    }
}

#[tauri::command]
pub async fn list_neoforge_versions(game_version: String) -> Result<Vec<ForgeVersion>, String> {
    let url = "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml";
    let xml = http_client()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("neoforge metadata failed: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let major = neo_major(&game_version);
    let mut out = vec![];
    for v in parse_maven_versions(&xml).into_iter().rev() {
        // neo versions: "21.1.255" or "20.4.167" or "26.3.0.51-beta"
        let v_major = v.split('.').next().unwrap_or("");
        if v_major == major {
            out.push(ForgeVersion {
                version: v.clone(),
                installer_url: format!(
                    "https://maven.neoforged.net/releases/net/neoforged/neoforge/{v}/neoforge-{v}-installer.jar"
                ),
                full: v.clone(),
            });
            if out.len() >= 30 {
                break;
            }
        }
    }
    if out.is_empty() {
        return Err(format!("no NeoForge builds for {game_version} (major {major})"));
    }
    Ok(out)
}

/// Download installer jar into cache. Returns path.
async fn fetch_installer(app: &tauri::AppHandle, loader: &str, installer_url: &str, cache_name: &str) -> Result<PathBuf, String> {
    let base = base_dir(app)?;
    let dest = base.join("installers").join(cache_name);
    if !dest.exists() {
        emit(app, &format!("Downloading {loader} installer…"));
        crate::files::download_file_report(installer_url, &dest, Some((app, "Downloading installer", 0, 1)), cache_name).await?;
    }
    Ok(dest)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in walkdir_simple(src) {
        let rel = entry.strip_prefix(src).unwrap();
        let target = dst.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(p) = target.parent() {
                std::fs::create_dir_all(p)?;
            }
            // skip if same size exists
            let copy = match (std::fs::metadata(&entry), std::fs::metadata(&target)) {
                (Ok(a), Ok(b)) => a.len() != b.len(),
                _ => true,
            };
            if copy {
                std::fs::copy(&entry, &target)?;
            }
        }
    }
    Ok(())
}

fn walkdir_simple(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p.clone());
            }
            out.push(p);
        }
    }
    out
}

fn find_produced_version_json(dotmc: &Path) -> Option<(PathBuf, Value)> {
    let versions_dir = dotmc.join("versions");
    let Ok(entries) = std::fs::read_dir(&versions_dir) else { return None };
    // pick most recently modified json
    let mut best: Option<(PathBuf, Value, std::time::SystemTime)> = None;
    for e in entries.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let name = dir.file_name()?.to_string_lossy().to_string();
        // skip vanilla numeric-only? forge ids contain "forge"
        if !name.contains("forge") {
            continue;
        }
        let json_path = dir.join(format!("{name}.json"));
        if !json_path.exists() {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&json_path) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else { continue };
        let mtime = std::fs::metadata(&json_path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().map(|b| mtime > b.2).unwrap_or(true) {
            best = Some((json_path, v, mtime));
        }
    }
    best.map(|(p, v, _)| (p, v))
}

/// Run installer headless into a temp dot-minecraft, then harvest version json + libraries.
/// Returns the loader profile json.
pub async fn ensure_forge_like_installed(
    app: &tauri::AppHandle,
    loader: &str, // "forge" | "neoforge"
    game_version: &str,
    loader_version: &str,
    installer_url: &str,
) -> Result<Value, String> {
    let base = base_dir(app)?;
    let cache_name = if loader == "forge" {
        format!("forge-{game_version}-{loader_version}-installer.jar")
    } else {
        format!("neoforge-{loader_version}-installer.jar")
    };
    let installer = fetch_installer(app, loader, installer_url, &cache_name).await?;

    // Installer itself needs Java 17+ for modern loaders; auto-provision if missing.
    let java_bin = crate::java::ensure_java(app, &serde_json::json!({"javaVersion": {"majorVersion": 17}})).await?;
    let work_dotmc = base.join("install_work").join(format!("{loader}-{game_version}-{loader_version}"));
    std::fs::create_dir_all(&work_dotmc).map_err(|e| e.to_string())?;

    // Run installer. Forge: --installClient <dir>. NeoForge newer: --install-client <dir>.
    let arg_sets: Vec<Vec<String>> = if loader == "forge" {
        vec![vec!["--installClient".into(), work_dotmc.to_string_lossy().to_string()]]
    } else {
        vec![
            vec!["--install-client".into(), work_dotmc.to_string_lossy().to_string()],
            vec!["--installClient".into(), work_dotmc.to_string_lossy().to_string()],
        ]
    };

    let mut last_err = String::new();
    let mut ok = false;
    for args in &arg_sets {
        emit(app, &format!("Running {loader} installer (may take 2-5 min)…"));
        let mut cmd = tokio::process::Command::new(&java_bin);
        cmd.arg("-jar")
            .arg(&installer)
            .args(args)
            .current_dir(&work_dotmc);
        match tokio::time::timeout(std::time::Duration::from_secs(600), cmd.output()).await {
            Ok(Ok(out)) => {
                if out.status.success() || find_produced_version_json(&work_dotmc).is_some() {
                    ok = true;
                    break;
                }
                last_err = format!(
                    "exit {}: {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).chars().take(500).collect::<String>()
                );
            }
            Ok(Err(e)) => last_err = e.to_string(),
            Err(_) => last_err = "installer timed out after 10 min".into(),
        }
    }
    if !ok && find_produced_version_json(&work_dotmc).is_none() {
        return Err(format!("{loader} installer failed: {last_err}. Try a different {loader} build or check Java 17+."));
    }

    let (produced_path, mut profile) =
        find_produced_version_json(&work_dotmc).ok_or_else(|| format!("{loader} installer produced no version json"))?;

    // Merge installer libraries into shared libraries dir
    let src_libs = work_dotmc.join("libraries");
    if src_libs.exists() {
        emit(app, "Copying loader libraries…");
        copy_dir_recursive(&src_libs, &base.join("libraries")).map_err(|e| e.to_string())?;
    }

    // Ensure any additional profile libraries with maven coords get downloaded (forge slim/extra)
    ensure_profile_libraries(app, &profile).await?;

    // Cache profile under stable name
    let meta_dir = base.join("meta");
    std::fs::create_dir_all(&meta_dir).map_err(|e| e.to_string())?;
    let produced_id = produced_path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{game_version}-{loader}-{loader_version}"));
    profile["id"] = Value::String(produced_id.clone());
    let cache = meta_dir.join(format!("{loader}-{game_version}-{loader_version}.json"));
    std::fs::write(&cache, serde_json::to_string_pretty(&profile).unwrap()).map_err(|e| e.to_string())?;
    Ok(profile)
}

async fn ensure_profile_libraries(app: &tauri::AppHandle, profile: &Value) -> Result<(), String> {
    let base = base_dir(app)?;
    let libs_dir = base.join("libraries");
    let empty = vec![];
    let libs = profile.get("libraries").and_then(|l| l.as_array()).unwrap_or(&empty);
    for lib in libs {
        // vanilla-style artifact
        if let Some(artifact) = lib.pointer("/downloads/artifact") {
            let path = artifact.get("path").and_then(|p| p.as_str()).unwrap_or("");
            let url = artifact.get("url").and_then(|u| u.as_str()).unwrap_or("");
            let sha1 = artifact.get("sha1").and_then(|s| s.as_str()).unwrap_or("");
            if path.is_empty() || url.is_empty() {
                continue;
            }
            let dest = libs_dir.join(path);
            if dest.exists() && (sha1.is_empty() || crate::files::sha1_matches(&dest, sha1)) {
                continue;
            }
            download_file(url, &dest).await?;
            continue;
        }
        // maven coords style
        if let Some(name) = lib.get("name").and_then(|n| n.as_str()) {
            let Some(rel) = maven_to_path(name) else { continue };
            let dest = libs_dir.join(&rel);
            if dest.exists() {
                continue;
            }
            let repo = lib.get("url").and_then(|u| u.as_str()).unwrap_or("");
            // try explicit repo, then well-known fallbacks
            let mut urls = vec![];
            if !repo.is_empty() {
                urls.push(format!("{}/{rel}", repo.trim_end_matches('/')));
            }
            urls.push(format!("https://maven.minecraftforge.net/{rel}"));
            urls.push(format!("https://maven.neoforged.net/releases/{rel}"));
            urls.push(format!("https://libraries.minecraft.net/{rel}"));
            urls.push(format!("https://repo1.maven.org/maven2/{rel}"));
            let mut done = false;
            for u in urls {
                if download_file(&u, &dest).await.is_ok() {
                    done = true;
                    break;
                }
                let _ = std::fs::remove_file(&dest);
            }
            if !done {
                // non-fatal: some optional libs may 404; launch will surface real errors
                continue;
            }
        }
    }
    Ok(())
}

/// Classpath entries from a forge-like profile (both artifact paths and maven coords).
pub fn forge_classpath_entries(base: &Path, profile: &Value) -> Vec<String> {
    let libs_dir = base.join("libraries");
    let mut out = vec![];
    if let Some(arr) = profile.get("libraries").and_then(|v| v.as_array()) {
        for lib in arr {
            // respect rules
            if let Some(rules) = lib.get("rules").and_then(|r| r.as_array()) {
                if !rules_allow(rules) {
                    continue;
                }
            }
            if let Some(path) = lib.pointer("/downloads/artifact/path").and_then(|p| p.as_str()) {
                out.push(libs_dir.join(path).to_string_lossy().to_string());
                continue;
            }
            if let Some(name) = lib.get("name").and_then(|n| n.as_str()) {
                if let Some(rel) = maven_to_path(name) {
                    out.push(libs_dir.join(rel).to_string_lossy().to_string());
                }
            }
        }
    }
    out
}

fn rules_allow(rules: &[Value]) -> bool {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    };
    let mut result: Option<bool> = None;
    for r in rules {
        let action = r.get("action").and_then(|v| v.as_str()).unwrap_or("allow");
        let matches = match r.get("os") {
            None => true,
            Some(os_v) => os_v.get("name").and_then(|v| v.as_str()).map(|n| n == os).unwrap_or(true),
        };
        if matches {
            result = Some(action == "allow");
        }
    }
    result.unwrap_or(true)
}
