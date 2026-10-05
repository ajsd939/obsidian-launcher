use crate::files::{base_dir, download_file};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
}

#[tauri::command]
pub async fn list_fabric_loaders(game_version: String) -> Result<Vec<LoaderVersion>, String> {
    let url = format!("https://meta.fabricmc.net/v2/versions/loader/{game_version}");
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("fabric meta failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("fabric: no loaders for {game_version} ({})", resp.status()));
    }
    let arr: Vec<Value> = resp.json().await.map_err(|e| e.to_string())?;
    let mut out = vec![];
    for entry in arr {
        if let Some(v) = entry.pointer("/loader/version").and_then(|v| v.as_str()) {
            let stable = entry
                .pointer("/loader/stable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            out.push(LoaderVersion {
                version: v.to_string(),
                stable,
            });
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn list_quilt_loaders(game_version: String) -> Result<Vec<LoaderVersion>, String> {
    let url = format!("https://meta.quiltmc.org/v3/versions/loader/{game_version}");
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("quilt meta failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("quilt: no loaders for {game_version} ({})", resp.status()));
    }
    let arr: Vec<Value> = resp.json().await.map_err(|e| e.to_string())?;
    let mut out = vec![];
    for entry in arr {
        if let Some(v) = entry.pointer("/loader/version").and_then(|v| v.as_str()) {
            // quilt has no stable flag; treat all as stable
            out.push(LoaderVersion {
                version: v.to_string(),
                stable: true,
            });
        }
    }
    Ok(out)
}

pub async fn fetch_loader_profile(
    loader: &str,
    game_version: &str,
    loader_version: &str,
) -> Result<Value, String> {
    let url = match loader {
        "fabric" => format!(
            "https://meta.fabricmc.net/v2/versions/loader/{game_version}/{loader_version}/profile/json"
        ),
        "quilt" => format!(
            "https://meta.quiltmc.org/v3/versions/loader/{game_version}/{loader_version}/profile/json"
        ),
        _ => return Err(format!("unknown loader {loader}")),
    };
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("loader profile fetch failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("loader profile -> {}", resp.status()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// Maven coords "group:artifact:version[:classifier][@ext]" -> relative path.
pub fn maven_to_path(coords: &str) -> Option<String> {
    let (coords, ext) = match coords.split_once('@') {
        Some((c, e)) => (c, e),
        None => (coords, "jar"),
    };
    let parts: Vec<&str> = coords.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let (group, artifact, version) = (parts[0], parts[1], parts[2]);
    let classifier = if parts.len() > 3 { Some(parts[3]) } else { None };
    let file = match classifier {
        Some(c) => format!("{artifact}-{version}-{c}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!(
        "{}/{artifact}/{version}/{file}",
        group.replace('.', "/")
    ))
}

pub async fn ensure_loader_downloaded(
    app: &tauri::AppHandle,
    loader: &str,
    game_version: &str,
    loader_version: &str,
) -> Result<Value, String> {
    let profile = fetch_loader_profile(loader, game_version, loader_version).await?;
    let base = base_dir(app)?;
    let libs_dir = base.join("libraries");

    // Collect all maven-style libs from both shapes:
    // fabric: launcherMeta.libraries.{client,common}[]
    // quilt: libraries[]
    let mut maven_libs: Vec<(String, String)> = vec![];
    if loader == "fabric" {
        for key in ["client", "common"] {
            if let Some(arr) = profile
                .pointer(&format!("/launcherMeta/libraries/{key}"))
                .and_then(|v| v.as_array())
            {
                for lib in arr {
                    if let (Some(name), Some(url)) = (
                        lib.get("name").and_then(|n| n.as_str()),
                        lib.get("url").and_then(|u| u.as_str()),
                    ) {
                        maven_libs.push((name.to_string(), url.to_string()));
                    }
                }
            }
        }
        // also top-level libraries if present
        if let Some(arr) = profile.get("libraries").and_then(|v| v.as_array()) {
            for lib in arr {
                if let (Some(name), Some(url)) = (
                    lib.get("name").and_then(|n| n.as_str()),
                    lib.get("url").and_then(|u| u.as_str()),
                ) {
                    maven_libs.push((name.to_string(), url.to_string()));
                }
            }
        }
    } else {
        if let Some(arr) = profile.get("libraries").and_then(|v| v.as_array()) {
            for lib in arr {
                if let Some(name) = lib.get("name").and_then(|n| n.as_str()) {
                    let url = lib
                        .get("url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("https://maven.quiltmc.org/repository/release/");
                    maven_libs.push((name.to_string(), url.to_string()));
                }
                // quilt may also embed vanilla-style downloads.artifact
                if let Some(artifact) = lib.pointer("/downloads/artifact") {
                    let path = artifact.get("path").and_then(|p| p.as_str()).unwrap_or("");
                    let aurl = artifact.get("url").and_then(|u| u.as_str()).unwrap_or("");
                    if !path.is_empty() && !aurl.is_empty() {
                        let dest = libs_dir.join(path);
                        if !dest.exists() {
                            download_file(aurl, &dest).await?;
                        }
                    }
                }
            }
        }
    }

    for (coords, repo) in maven_libs {
        let rel = maven_to_path(&coords)
            .ok_or_else(|| format!("bad maven coords {coords}"))?;
        let dest = libs_dir.join(&rel);
        if dest.exists() {
            continue;
        }
        let base_url = repo.trim_end_matches('/');
        let full = format!("{base_url}/{rel}");
        download_file(&full, &dest).await?;
    }

    // cache profile for launch
    let meta_dir = base.join("meta");
    std::fs::create_dir_all(&meta_dir).map_err(|e| e.to_string())?;
    let cache = meta_dir.join(format!("{loader}-{game_version}-{loader_version}.json"));
    std::fs::write(&cache, serde_json::to_string_pretty(&profile).unwrap())
        .map_err(|e| e.to_string())?;
    Ok(profile)
}

/// Classpath entries from a loader profile (maven coords -> local paths).
pub fn loader_classpath_entries(base: &std::path::Path, profile: &Value, loader: &str) -> Vec<String> {
    let libs_dir = base.join("libraries");
    let mut out = vec![];
    let push_coords = |coords: &str, acc: &mut Vec<String>| {
        if let Some(rel) = maven_to_path(coords) {
            acc.push(libs_dir.join(rel).to_string_lossy().to_string());
        }
    };
    if loader == "fabric" {
        for key in ["client", "common"] {
            if let Some(arr) = profile
                .pointer(&format!("/launcherMeta/libraries/{key}"))
                .and_then(|v| v.as_array())
            {
                for lib in arr {
                    if let Some(name) = lib.get("name").and_then(|n| n.as_str()) {
                        push_coords(name, &mut out);
                    }
                }
            }
        }
        if let Some(arr) = profile.get("libraries").and_then(|v| v.as_array()) {
            for lib in arr {
                if let Some(name) = lib.get("name").and_then(|n| n.as_str()) {
                    push_coords(name, &mut out);
                }
            }
        }
    } else if loader == "quilt" {
        if let Some(arr) = profile.get("libraries").and_then(|v| v.as_array()) {
            for lib in arr {
                if let Some(name) = lib.get("name").and_then(|n| n.as_str()) {
                    push_coords(name, &mut out);
                }
                if let Some(path) = lib.pointer("/downloads/artifact/path").and_then(|p| p.as_str()) {
                    out.push(libs_dir.join(path).to_string_lossy().to_string());
                }
            }
        }
    }
    out
}
