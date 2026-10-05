use crate::files::{base_dir, instance_dir};
use serde::{Deserialize, Serialize};
use tauri::Emitter;

const CSL_PROJECT_ID: &str = "customskinloader";

fn emit(app: &tauri::AppHandle, stage: &str) {
    let _ = app.emit(
        "download-progress",
        serde_json::json!({"stage": stage, "current": 0, "total": 1}),
    );
}

fn skins_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = base_dir(app)?.join("skins");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn check_username(username: &str) -> Result<(), String> {
    let u = username.trim();
    if u.len() < 3 || u.len() > 16 || !u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Username must be 3-16 chars (a-z, 0-9, _)".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkinUser {
    pub username: String,
    pub has_skin: bool,
    pub has_cape: bool,
    pub has_elytra: bool,
    pub model: String, // "classic" | "slim"
}

#[tauri::command]
pub async fn list_skinned_users(app: tauri::AppHandle) -> Result<Vec<SkinUser>, String> {
    let dir = skins_dir(&app)?;
    let mut out = vec![];
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(vec![]);
    };
    for e in entries.flatten() {
        if !e.path().is_dir() {
            continue;
        }
        let username = e.file_name().to_string_lossy().to_string();
        let d = e.path();
        let meta_path = d.join("meta.json");
        let model = std::fs::read_to_string(&meta_path)
            .ok()
            .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
            .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| "classic".into());
        out.push(SkinUser {
            username,
            has_skin: d.join("skin.png").exists(),
            has_cape: d.join("cape.png").exists(),
            has_elytra: d.join("elytra.png").exists(),
            model,
        });
    }
    out.sort_by(|a, b| a.username.cmp(&b.username));
    Ok(out)
}

/// Save a skin/cape/elytra PNG (base64, with or without data: prefix).
#[tauri::command]
pub async fn save_skin_file(
    app: tauri::AppHandle,
    username: String,
    kind: String,
    data_base64: String,
    model: Option<String>,
) -> Result<(), String> {
    check_username(&username)?;
    if !["skin", "cape", "elytra"].contains(&kind.as_str()) {
        return Err("kind must be skin, cape or elytra".into());
    }
    let b64 = data_base64.split(',').last().unwrap_or("").trim();
    if b64.is_empty() {
        return Err("empty file".into());
    }
    // base64 decode without new deps
    let bytes = base64_decode(b64).ok_or("invalid base64")?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("file too large (max 4MB)".into());
    }
    // PNG magic check
    if bytes.len() < 8 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("only PNG files are supported".into());
    }
    let dir = skins_dir(&app)?.join(username.trim());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{kind}.png")), &bytes).map_err(|e| e.to_string())?;
    if kind == "skin" {
        let m = model.unwrap_or_else(|| "classic".into());
        let m = if m == "slim" { "slim" } else { "classic" };
        std::fs::write(dir.join("meta.json"), format!("{{\"model\":\"{m}\"}}"))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_skin_file(app: tauri::AppHandle, username: String, kind: String) -> Result<String, String> {
    check_username(&username)?;
    let p = skins_dir(&app)?.join(username.trim()).join(format!("{kind}.png"));
    let bytes = std::fs::read(&p).map_err(|_| format!("no {kind} saved for {username}"))?;
    Ok(format!("data:image/png;base64,{}", base64_encode(&bytes)))
}

#[tauri::command]
pub async fn delete_skin_file(app: tauri::AppHandle, username: String, kind: String) -> Result<(), String> {
    check_username(&username)?;
    let p = skins_dir(&app)?.join(username.trim()).join(format!("{kind}.png"));
    if p.exists() {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Resource-pack pack_format per release (from minecraft.wiki Pack_format).
/// Returns (major, use_min_max_scheme). 1.21.9+ uses min_format/max_format.
fn pack_format_for(version_id: &str) -> Option<(u32, bool)> {
    let v = version_id.trim();
    // 26.x style (new versioning): 26.1->84, 26.2->88, 26.3->97
    if let Some(rest) = v.strip_prefix("26.") {
        let minor: u32 = rest.split('.').next()?.parse().ok()?;
        return match minor {
            1 => Some((84, true)),
            2 => Some((88, true)),
            m if m >= 3 => Some((97, true)),
            _ => None,
        };
    }
    let parts: Vec<u32> = v
        .split('.')
        .map(|p| p.parse::<u32>().unwrap_or(9999))
        .collect();
    if parts.len() < 2 || parts[0] != 1 {
        return None;
    }
    let (minor, patch) = (parts[1], *parts.get(2).unwrap_or(&0));
    let fmt: u32 = match (minor, patch) {
        (0..=5, _) | (6, 0) | (6, 1) => return None, // ancient; skip
        (6, _) => 1,  // 1.6.x approx (really 1.6.1+)
        (7, _) | (8, _) => 1,
        (9, _) | (10, _) => 2,
        (11, _) | (12, _) => 3,
        (13, _) | (14, _) => 4,
        (15, _) | (16, 0) | (16, 1) => 5,
        (16, _) => 6,
        (17, _) => 7,
        (18, _) => 8,
        (19, 0) | (19, 1) | (19, 2) => 9,
        (19, 3) => 12,
        (19, _) => 13,
        (20, 0) | (20, 1) => 15,
        (20, 2) => 18,
        (20, 3) | (20, 4) => 22,
        (20, _) => 32,
        (21, 0) | (21, 1) => 34,
        (21, 2) | (21, 3) => 42,
        (21, 4) => 46,
        (21, 5) => 55,
        (21, 6) => 63,
        (21, 7) | (21, 8) => 64,
        (21, 9) | (21, 10) => return Some((69, true)),
        (21, _) => return Some((75, true)),
        _ => return None,
    };
    Some((fmt, false))
}

/// All default-skin texture paths we override (old + modern layouts).
fn default_skin_paths() -> Vec<&'static str> {
    let mut out = vec![
        // pre-1.19.3 layout
        "assets/minecraft/textures/entity/steve.png",
        "assets/minecraft/textures/entity/alex.png",
    ];
    // modern layout: wide + slim x 9 variants
    for model in ["wide", "slim"] {
        for name in ["steve", "alex", "ari", "efe", "kai", "makena", "noor", "sunny", "zuri"] {
            out.push(match (model, name) {
                ("wide", "steve") => "assets/minecraft/textures/entity/player/wide/steve.png",
                ("wide", "alex") => "assets/minecraft/textures/entity/player/wide/alex.png",
                ("wide", "ari") => "assets/minecraft/textures/entity/player/wide/ari.png",
                ("wide", "efe") => "assets/minecraft/textures/entity/player/wide/efe.png",
                ("wide", "kai") => "assets/minecraft/textures/entity/player/wide/kai.png",
                ("wide", "makena") => "assets/minecraft/textures/entity/player/wide/makena.png",
                ("wide", "noor") => "assets/minecraft/textures/entity/player/wide/noor.png",
                ("wide", "sunny") => "assets/minecraft/textures/entity/player/wide/sunny.png",
                ("wide", "zuri") => "assets/minecraft/textures/entity/player/wide/zuri.png",
                ("slim", "steve") => "assets/minecraft/textures/entity/player/slim/steve.png",
                ("slim", "alex") => "assets/minecraft/textures/entity/player/slim/alex.png",
                ("slim", "ari") => "assets/minecraft/textures/entity/player/slim/ari.png",
                ("slim", "efe") => "assets/minecraft/textures/entity/player/slim/efe.png",
                ("slim", "kai") => "assets/minecraft/textures/entity/player/slim/kai.png",
                ("slim", "makena") => "assets/minecraft/textures/entity/player/slim/makena.png",
                ("slim", "noor") => "assets/minecraft/textures/entity/player/slim/noor.png",
                ("slim", "sunny") => "assets/minecraft/textures/entity/player/slim/sunny.png",
                _ => "assets/minecraft/textures/entity/player/slim/zuri.png",
            });
        }
    }
    out
}

/// Build/update `obsidian-skin.zip` resource pack + enable it in options.txt.
/// Works on vanilla AND modded (profile textures from CSL take precedence when present).
/// Returns a note when the skin couldn't be applied.
pub async fn ensure_vanilla_skin_pack(
    app: &tauri::AppHandle,
    instance: &crate::models::Instance,
    username: &str,
) -> Result<Option<String>, String> {
    let src = skins_dir(app)?.join(username).join("skin.png");
    if !src.exists() {
        return Ok(None);
    }
    let skin_bytes = std::fs::read(&src).map_err(|e| e.to_string())?;
    let (format, ranged) = pack_format_for(&instance.version_id)
        .ok_or_else(|| format!("no skin-pack format known for {}", instance.version_id))?;

    let game_dir = instance_dir(app, &instance.id)?;
    let rp_dir = game_dir.join("resourcepacks");
    std::fs::create_dir_all(&rp_dir).map_err(|e| e.to_string())?;
    let pack_path = rp_dir.join("obsidian-skin.zip");

    // rebuild only when skin changed (size check is a cheap proxy)
    let rebuild = match std::fs::metadata(&pack_path) {
        Ok(m) => m.len() < 1000 || std::fs::metadata(&src).ok().and_then(|s| s.modified().ok()).zip(std::fs::metadata(&pack_path).ok().and_then(|p| p.modified().ok())).map(|(a, b)| a > b).unwrap_or(true),
        Err(_) => true,
    };
    if rebuild {
        let mcmeta = if ranged {
            format!("{{\"pack\":{{\"description\":\"Obsidian offline skin for {username}\",\"min_format\":{format},\"max_format\":{format}}}}}")
        } else {
            format!("{{\"pack\":{{\"description\":\"Obsidian offline skin for {username}\",\"pack_format\":{format}}}}}")
        };
        let f = std::fs::File::create(&pack_path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("pack.mcmeta", opts).map_err(|e| e.to_string())?;
        use std::io::Write;
        zip.write_all(mcmeta.as_bytes()).map_err(|e| e.to_string())?;
        for path in default_skin_paths() {
            zip.start_file(path, opts).map_err(|e| e.to_string())?;
            zip.write_all(&skin_bytes).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
    }

    enable_pack_in_options(&game_dir)?;
    Ok(None)
}

fn enable_pack_in_options(game_dir: &std::path::Path) -> Result<(), String> {
    const PACK_ENTRY: &str = "\"file/obsidian-skin.zip\"";
    let path = game_dir.join("options.txt");
    if !path.exists() {
        std::fs::write(&path, format!("resourcePacks:[\"vanilla\",{PACK_ENTRY}]\n")).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    if raw.contains("obsidian-skin.zip") {
        return Ok(());
    }
    let mut lines: Vec<String> = raw.lines().map(|l| l.to_string()).collect();
    let mut done = false;
    for line in lines.iter_mut() {
        if let Some(rest) = line.strip_prefix("resourcePacks:[") {
            if rest.trim_end().ends_with(']') {
                let inner = rest.trim_end();
                let inner = &inner[..inner.len() - 1];
                *line = if inner.trim().is_empty() {
                    format!("resourcePacks:[{PACK_ENTRY}]")
                } else {
                    format!("resourcePacks:[{inner},{PACK_ENTRY}]")
                };
                done = true;
                break;
            }
        }
    }
    if !done {
        lines.push(format!("resourcePacks:[\"vanilla\",{PACK_ENTRY}]"));
    }
    std::fs::write(&path, lines.join("\n")).map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub async fn instance_csl_present(app: tauri::AppHandle, instance_id: String) -> Result<bool, String> {
    Ok(csl_jar_in(&instance_dir(&app, &instance_id)?.join("mods")).is_some())
}

fn csl_jar_in(mods: &std::path::Path) -> Option<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(mods) else {
        return None;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if name.contains("customskinloader") && name.ends_with(".jar") && !name.ends_with(".disabled") {
            return Some(e.path());
        }
    }
    None
}

/// Ensure CSL mod + LocalSkin files for the given user in a modded instance.
/// Non-fatal for the game launch: returns a warning string on soft failure.
pub async fn ensure_csl_for_instance(
    app: &tauri::AppHandle,
    instance: &crate::models::Instance,
    username: &str,
) -> Result<Option<String>, String> {
    if instance.loader == "vanilla" {
        return Ok(Some("vanilla instances can't show custom skins — use a Fabric/Forge/etc. instance".into()));
    }
    let game_dir = instance_dir(app, &instance.id)?;
    let mods_dir = game_dir.join("mods");
    std::fs::create_dir_all(&mods_dir).map_err(|e| e.to_string())?;

    // 1. CSL jar (via Modrinth, matched to game version + loader)
    if csl_jar_in(&mods_dir).is_none() {
        emit(app, "Downloading CustomSkinLoader…");
        let versions = crate::modrinth::versions_inner(
            CSL_PROJECT_ID,
            Some(&instance.version_id),
            Some(&instance.loader),
        )
        .await
        .map_err(|e| format!("CustomSkinLoader not available for {} {}: {e}", instance.version_id, instance.loader))?;
        let ver = versions.into_iter().next().ok_or_else(|| {
            format!("no CustomSkinLoader build for {} {}", instance.version_id, instance.loader)
        })?;
        let file = ver
            .files
            .iter()
            .find(|f| f.primary)
            .or(ver.files.first())
            .ok_or("CustomSkinLoader version has no files")?;
        let dest = mods_dir.join(&file.filename);
        let fname = file.filename.clone();
        crate::files::download_file_report(&file.url, &dest, Some((app, "Downloading skin mod", 0, 1)), &fname).await?;
    }

    // 2. LocalSkin files for this user
    let store = skins_dir(app)?.join(username);
    let mut synced = vec![];
    for (kind, sub) in [("skin", "skins"), ("cape", "capes"), ("elytra", "elytras")] {
        let src = store.join(format!("{kind}.png"));
        if src.exists() {
            let dst_dir = game_dir.join("CustomSkinLoader").join("LocalSkin").join(sub);
            std::fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
            std::fs::copy(&src, dst_dir.join(format!("{username}.png"))).map_err(|e| e.to_string())?;
            synced.push(kind);
        }
    }
    // 3. loadlist: put LocalSkin first so offline names skip Mojang lookup fast
    write_loadlist_first_local(&game_dir)?;

    if synced.is_empty() {
        return Ok(Some(format!("no saved skin for {username} — upload one in the Skins tab")));
    }
    Ok(None)
}

/// Rewrite CSL config loadlist with LocalSkin first (keeps other entries).
fn write_loadlist_first_local(game_dir: &std::path::Path) -> Result<(), String> {
    let cfg_dir = game_dir.join("CustomSkinLoader");
    std::fs::create_dir_all(&cfg_dir).map_err(|e| e.to_string())?;
    let cfg_path = cfg_dir.join("CustomSkinLoader.json");
    let local_entry = serde_json::json!({"name": "LocalSkin", "type": "LocalSkin"});
    if cfg_path.exists() {
        if let Ok(raw) = std::fs::read_to_string(&cfg_path) {
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(list) = v.get_mut("loadlist").and_then(|l| l.as_array_mut()) {
                    list.retain(|e| e.get("name").and_then(|n| n.as_str()) != Some("LocalSkin"));
                    list.insert(0, local_entry);
                    std::fs::write(&cfg_path, serde_json::to_string_pretty(&v).unwrap())
                        .map_err(|e| e.to_string())?;
                    return Ok(());
                }
            }
        }
    }
    let v = serde_json::json!({
        "loadlist": [local_entry, {"name": "Mojang", "type": "MojangAPI"}],
        "version": 1,
    });
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits = 0;
    for c in input.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            '=' => break,
            _ if c.is_whitespace() => continue,
            _ => return None,
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8 & 0xFF);
        }
    }
    Some(out)
}

fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        s.push(T[(b[0] >> 2) as usize] as char);
        s.push(T[((b[0] & 3) << 4 | b[1] >> 4) as usize] as char);
        s.push(if chunk.len() > 1 { T[((b[1] & 15) << 2 | b[2] >> 6) as usize] as char } else { '=' });
        s.push(if chunk.len() > 2 { T[(b[2] & 63) as usize] as char } else { '=' });
    }
    s
}
