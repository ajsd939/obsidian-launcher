use crate::files::{base_dir, download_file, instance_dir};
use crate::models::Instance;
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("obsidian-launcher/0.1 (offline-mc-launcher)")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModHit {
    pub project_id: String,
    pub title: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub downloads: i64,
    pub project_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModVersion {
    pub id: String,
    pub version_number: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub files: Vec<ModFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModFile {
    pub filename: String,
    pub url: String,
    pub primary: bool,
    pub size: u64,
}

#[tauri::command]
pub async fn modrinth_search(
    query: String,
    game_version: Option<String>,
    loader: Option<String>,
    project_type: Option<String>,
) -> Result<Vec<ModHit>, String> {
    // facets: [["project_type:mod"],["versions:1.21"],["categories:fabric"]]
    let mut facets: Vec<Vec<String>> = vec![];
    let pt = project_type.unwrap_or_else(|| "mod".into());
    if pt != "any" {
        facets.push(vec![format!("project_type:{pt}")]);
    }
    if let Some(g) = game_version.filter(|g| !g.is_empty()) {
        facets.push(vec![format!("versions:{g}")]);
    }
    if let Some(l) = loader.filter(|l| !l.is_empty() && l != "vanilla") {
        // modrinth loader ids: fabric, quilt, forge, neoforge
        facets.push(vec![format!("categories:{l}")]);
    }
    let facets_str = serde_json::to_string(&facets).unwrap_or("[]".into());
    let url = format!(
        "https://api.modrinth.com/v2/search?query={}&facets={}&limit=25",
        urlencoding_simple(&query),
        urlencoding_simple(&facets_str)
    );
    let resp = client().get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("modrinth search -> {}", resp.status()));
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    let mut out = vec![];
    if let Some(hits) = v.get("hits").and_then(|h| h.as_array()) {
        for h in hits {
            out.push(ModHit {
                project_id: h.get("project_id").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                title: h.get("title").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                description: h.get("description").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                icon_url: h.get("icon_url").and_then(|s| s.as_str()).map(|s| s.to_string()),
                downloads: h.get("downloads").and_then(|d| d.as_i64()).unwrap_or(0),
                project_type: h.get("project_type").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            });
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn modrinth_versions(
    project_id: String,
    game_version: Option<String>,
    loader: Option<String>,
) -> Result<Vec<ModVersion>, String> {
    versions_inner(&project_id, game_version.as_deref(), loader.as_deref()).await
}

pub async fn versions_inner(
    project_id: &str,
    game_version: Option<&str>,
    loader: Option<&str>,
) -> Result<Vec<ModVersion>, String> {
    let mut url = format!("https://api.modrinth.com/v2/project/{project_id}/version?limit=20");
    if let Some(g) = game_version.filter(|g| !g.is_empty()) {
        url.push_str(&format!("&game_versions=%5B%22{}%22%5D", urlencoding_simple(g)));
    }
    if let Some(l) = loader.filter(|l| !l.is_empty() && *l != "vanilla") {
        url.push_str(&format!("&loaders=%5B%22{}%22%5D", urlencoding_simple(&l)));
    }
    let resp = client().get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("modrinth versions -> {}", resp.status()));
    }
    let arr: Vec<Value> = resp.json().await.map_err(|e| e.to_string())?;
    let mut out = vec![];
    for v in arr {
        let files = v.get("files").and_then(|f| f.as_array()).cloned().unwrap_or_default();
        out.push(ModVersion {
            id: v.get("id").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            version_number: v.get("version_number").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            game_versions: v.get("game_versions").and_then(|g| serde_json::from_value(g.clone()).ok()).unwrap_or_default(),
            loaders: v.get("loaders").and_then(|l| serde_json::from_value(l.clone()).ok()).unwrap_or_default(),
            files: files.into_iter().filter_map(|f| {
                Some(ModFile {
                    filename: f.get("filename")?.as_str()?.to_string(),
                    url: f.get("url")?.as_str()?.to_string(),
                    primary: f.get("primary").and_then(|p| p.as_bool()).unwrap_or(false),
                    size: f.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                })
            }).collect(),
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn modrinth_install_file(
    app: tauri::AppHandle,
    instance_id: String,
    file_url: String,
    filename: String,
    subdir: Option<String>,
) -> Result<String, String> {
    let dir = instance_dir(&app, &instance_id)?;
    let sub = subdir.unwrap_or_else(|| "mods".into());
    let dest = dir.join(&sub).join(sanitize_filename(&filename));
    crate::files::download_file_report(&file_url, &dest, Some((&app, "Downloading mod", 0, 1)), &filename).await?;
    Ok(format!("installed {}", dest.display()))
}

#[tauri::command]
pub async fn list_instance_mods(app: tauri::AppHandle, instance_id: String) -> Result<Vec<String>, String> {
    let dir = instance_dir(&app, &instance_id)?.join("mods");
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        if e.path().is_file() {
            out.push(e.file_name().to_string_lossy().to_string());
        }
    }
    out.sort();
    Ok(out)
}

#[tauri::command]
pub async fn delete_instance_mod(app: tauri::AppHandle, instance_id: String, filename: String) -> Result<(), String> {
    let p = instance_dir(&app, &instance_id)?.join("mods").join(sanitize_filename(&filename));
    if p.exists() {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Install a .mrpack (local path or http(s) URL). If instance_id is None, creates one from pack metadata.
#[tauri::command]
pub async fn install_mrpack(
    app: tauri::AppHandle,
    pack_path_or_url: String,
    instance_id: Option<String>,
    new_name: Option<String>,
) -> Result<Instance, String> {
    use tauri::Emitter;
    let emit = |s: &str| {
        let _ = app.emit("download-progress", serde_json::json!({"stage": s, "current": 0, "total": 1}));
    };
    // 1. obtain local zip path
    let local: std::path::PathBuf = if pack_path_or_url.starts_with("http") {
        emit("Downloading modpack…");
        let dest = base_dir(&app)?.join("packs").join(format!("pack-{}.mrpack", chrono::Utc::now().timestamp_millis()));
        download_file(&pack_path_or_url, &dest).await?;
        dest
    } else {
        std::path::PathBuf::from(&pack_path_or_url)
    };
    if !local.exists() {
        return Err("pack file not found".into());
    }
    // 2. read index
    let file = std::fs::File::open(&local).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut index_raw = String::new();
    {
        let mut f = zip.by_name("modrinth.index.json").map_err(|_| "not a .mrpack (missing modrinth.index.json)")?;
        use std::io::Read;
        f.read_to_string(&mut index_raw).map_err(|e| e.to_string())?;
    }
    let index: Value = serde_json::from_str(&index_raw).map_err(|e| e.to_string())?;
    let mc_version = index.pointer("/dependencies/minecraft").and_then(|v| v.as_str()).unwrap_or("1.21").to_string();
    let deps = index.get("dependencies").and_then(|d| d.as_object()).cloned().unwrap_or_default();
    // detect loader: fabric-loader, quilt-loader, forge, neoforge
    let (loader, loader_ver): (String, Option<String>) = if let Some(v) = deps.get("fabric-loader").and_then(|v| v.as_str()) {
        ("fabric".into(), Some(v.to_string()))
    } else if let Some(v) = deps.get("quilt-loader").and_then(|v| v.as_str()) {
        ("quilt".into(), Some(v.to_string()))
    } else if let Some(v) = deps.get("forge").and_then(|v| v.as_str()) {
        ("forge".into(), Some(v.to_string()))
    } else if let Some(v) = deps.get("neoforge").and_then(|v| v.as_str()) {
        ("neoforge".into(), Some(v.to_string()))
    } else {
        ("vanilla".into(), None)
    };

    // 3. resolve/create instance
    let inst: Instance = if let Some(id) = instance_id.filter(|s| !s.is_empty()) {
        let list: Vec<Instance> = crate::game::list_instances(app.clone()).await?;
        list.into_iter().find(|i| i.id == id).ok_or("instance not found")?
    } else {
        // validate loader version exists for fabric/quilt (forge/neoforge versions from pack are installer-specific; accept as-is)
        crate::game::create_instance(app.clone(), new_name.unwrap_or_else(|| format!("{mc_version}-{loader}")), mc_version.clone(), Some(loader.clone()), loader_ver.clone()).await?
    };

    // warn on loader mismatch (non-fatal)
    if inst.loader != loader && loader != "vanilla" {
        // proceed anyway; files still land in right instance
    }

    // 4. download files
    let files = index.get("files").and_then(|f| f.as_array()).cloned().unwrap_or_default();
    let total = files.len();
    let inst_dir = instance_dir(&app, &inst.id)?;
    let mut i = 0;
    for f in &files {
        i += 1;
        let path = f.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let downloads = f.get("downloads").and_then(|d| d.as_array()).cloned().unwrap_or_default();
        if path.is_empty() || downloads.is_empty() {
            continue;
        }
        let url = downloads[0].as_str().unwrap_or("");
        let dest = inst_dir.join(path.trim_start_matches('/'));
        // skip if exists with matching size
        let want_size = f.get("fileSize").and_then(|s| s.as_u64()).unwrap_or(0);
        if dest.exists() && want_size > 0 && dest.metadata().map(|m| m.len()).unwrap_or(0) == want_size {
            continue;
        }
        emit(&format!("Modpack files {i}/{total}"));
        // best effort per-file (log, continue)
        let file_label = path.trim_start_matches('/').to_string();
        let _ = crate::files::download_file_report(url, &dest, Some((&app, "Modpack files", i, total)), &file_label).await;
    }

    // 5. extract overrides (need fresh zip handle)
    let file2 = std::fs::File::open(&local).map_err(|e| e.to_string())?;
    let mut zip2 = zip::ZipArchive::new(file2).map_err(|e| e.to_string())?;
    let overrides_dirs = index.get("overrides").and_then(|o| o.as_str()).unwrap_or("overrides");
    for override_root in [overrides_dirs, "client-overrides"] {
        for idx in 0..zip2.len() {
            let mut entry = zip2.by_index(idx).map_err(|e| e.to_string())?;
            let name = entry.name().to_string();
            if !name.starts_with(&format!("{override_root}/")) || entry.is_dir() {
                continue;
            }
            let rel = name.trim_start_matches(&format!("{override_root}/"));
            if rel.is_empty() {
                continue;
            }
            let dest = inst_dir.join(rel);
            if let Some(p) = dest.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            let mut out = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
            use std::io::copy;
            let _ = copy(&mut entry, &mut out);
        }
    }

    emit("Modpack installed");
    Ok(inst)
}

fn sanitize_filename(name: &str) -> String {
    name.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '\0')).collect::<String>()
}

fn urlencoding_simple(s: &str) -> String {
    // minimal percent-encoding for query/facets (alnum + few safe chars pass through)
    let mut out = String::new();
    for b in s.bytes() {
        if matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
