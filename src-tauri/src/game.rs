use crate::files::{base_dir, download_file, download_file_report, instance_dir};
use crate::models::{Account, Instance};
use serde_json::Value;
use std::path::PathBuf;
use tauri::Emitter;

#[derive(serde::Serialize, Clone)]
struct Progress {
    stage: String,
    current: usize,
    total: usize,
}

fn emit(app: &tauri::AppHandle, stage: &str, current: usize, total: usize) {
    let _ = app.emit(
        "download-progress",
        Progress {
            stage: stage.to_string(),
            current,
            total,
        },
    );
}

fn rule_allows(rules: Option<&Vec<Value>>) -> bool {
    let Some(rules) = rules else { return true };
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    };
    let mut allowed = true;
    // Mojang rules: last matching rule wins; default allow if no rules match.
    let mut result: Option<bool> = None;
    for r in rules {
        let action = r.get("action").and_then(|v| v.as_str()).unwrap_or("allow");
        let os_obj = r.get("os");
        let matches = match os_obj {
            None => true, // applies to all OS
            Some(os_v) => {
                if let Some(name) = os_v.get("name").and_then(|v| v.as_str()) {
                    name == os
                } else {
                    true
                }
            }
        };
        if matches {
            result = Some(action == "allow");
        }
    }
    if let Some(v) = result {
        allowed = v;
    }
    allowed
}

#[tauri::command]
pub async fn ensure_version_downloaded(
    app: tauri::AppHandle,
    version_id: String,
) -> Result<String, String> {
    let base = base_dir(&app)?;
    let meta_dir = base.join("meta");
    std::fs::create_dir_all(&meta_dir).map_err(|e| e.to_string())?;
    let version_json_path = meta_dir.join(format!("{version_id}.json"));

    // 1. Resolve version json URL from manifest
    emit(&app, "Resolving version", 0, 4);
    let manifest = crate::mojang::fetch_version_manifest().await?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| format!("version {version_id} not found"))?;

    // 2. Download version json
    if !version_json_path.exists() {
        emit(&app, "Downloading version json", 1, 4);
        download_file(&entry.url, &version_json_path).await?;
    }
    let raw = std::fs::read_to_string(&version_json_path).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    // 3. Client jar
    emit(&app, "Downloading client", 2, 4);
    let client_url = v
        .pointer("/downloads/client/url")
        .and_then(|u| u.as_str())
        .ok_or("version json missing downloads.client.url")?;
    let client_path = base.join("versions").join(&version_id).join(format!("{version_id}.jar"));
    if !client_path.exists() || client_path.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
        let label = format!("{version_id}.jar");
        download_file_report(client_url, &client_path, Some((&app, "Downloading client", 2, 4)), &label).await?;
    }

    // 4. Libraries
    emit(&app, "Downloading libraries", 3, 4);
    let libs_dir = base.join("libraries");
    let empty = vec![];
    let libs = v.get("libraries").and_then(|l| l.as_array()).unwrap_or(&empty);
    // zbierz brakujące pliki, potem równolegle (12 połączeń, współdzielony keep-alive)
    struct LibJob {
        url: String,
        dest: std::path::PathBuf,
        label: String,
    }
    let mut jobs: Vec<LibJob> = vec![];
    for lib in libs {
        if !rule_allows(lib.get("rules").and_then(|r| r.as_array())) {
            continue;
        }
        // artifact download
        if let Some(artifact) = lib.pointer("/downloads/artifact") {
            let path = artifact.get("path").and_then(|p| p.as_str()).unwrap_or("");
            let url = artifact.get("url").and_then(|u| u.as_str()).unwrap_or("");
            let sha1 = artifact.get("sha1").and_then(|s| s.as_str()).unwrap_or("");
            if path.is_empty() || url.is_empty() {
                continue;
            }
            let dest = libs_dir.join(path);
            let ok = dest.exists() && (sha1.is_empty() || crate::files::sha1_matches(&dest, sha1));
            if !ok {
                jobs.push(LibJob {
                    url: url.to_string(),
                    dest,
                    label: path.rsplit('/').next().unwrap_or(path).to_string(),
                });
            }
        }
        // natives: classifiers per OS
        if let Some(classifiers) = lib.pointer("/downloads/classifiers") {
            let key = if cfg!(target_os = "windows") {
                "natives-windows"
            } else if cfg!(target_os = "macos") {
                "natives-macos"
            } else {
                "natives-linux"
            };
            // also try natives-windows-64 etc fallback: pick first matching
            let mut picked: Option<&Value> = classifiers.get(key);
            if picked.is_none() {
                if let Some(obj) = classifiers.as_object() {
                    for (k, val) in obj {
                        if k.contains("windows") && cfg!(target_os = "windows")
                            || k.contains("linux") && !cfg!(target_os = "windows") && !cfg!(target_os = "macos")
                            || k.contains("macos") && cfg!(target_os = "macos")
                            || k.contains("osx") && cfg!(target_os = "macos")
                        {
                            picked = Some(val);
                            break;
                        }
                    }
                }
            }
            if let Some(nat) = picked {
                let path = nat.get("path").and_then(|p| p.as_str()).unwrap_or("");
                let url = nat.get("url").and_then(|u| u.as_str()).unwrap_or("");
                if !path.is_empty() && !url.is_empty() {
                    let dest = libs_dir.join(path);
                    if !dest.exists() {
                        jobs.push(LibJob {
                            url: url.to_string(),
                            dest,
                            label: path.rsplit('/').next().unwrap_or(path).to_string(),
                        });
                    }
                }
            }
        }
    }
    {
        use futures_util::stream::{self, StreamExt};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let total = jobs.len();
        let done = Arc::new(AtomicUsize::new(0));
        stream::iter(jobs)
            .map(|job| {
                let app = app.clone();
                let done = done.clone();
                async move {
                    let r = download_file_report(
                        &job.url,
                        &job.dest,
                        Some((&app, "Downloading libraries", 0, total)),
                        &job.label,
                    )
                    .await;
                    let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                    if n % 10 == 0 || n == total {
                        emit(&app, "Downloading libraries", n, total);
                    }
                    r
                }
            })
            .buffer_unordered(12)
            .collect::<Vec<Result<(), String>>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
    }

    // 5. Assets
    emit(&app, "Downloading assets", 4, 4);
    ensure_assets(&app, &v).await?;

    emit(&app, "Done", 4, 4);
    Ok(format!("version {version_id} ready"))
}

async fn ensure_assets(app: &tauri::AppHandle, version_json: &Value) -> Result<(), String> {
    let base = base_dir(app)?;
    let index_id = version_json
        .pointer("/assetIndex/id")
        .and_then(|v| v.as_str())
        .unwrap_or("legacy");
    let index_url = version_json
        .pointer("/assetIndex/url")
        .and_then(|v| v.as_str())
        .ok_or("missing assetIndex.url")?;
    let indexes_dir = base.join("assets").join("indexes");
    std::fs::create_dir_all(&indexes_dir).map_err(|e| e.to_string())?;
    let index_path = indexes_dir.join(format!("{index_id}.json"));
    if !index_path.exists() {
        download_file(index_url, &index_path).await?;
    }
    let raw = std::fs::read_to_string(&index_path).map_err(|e| e.to_string())?;
    let idx: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let objects = idx.get("objects").and_then(|o| o.as_object()).cloned().unwrap_or_default();
    let total = objects.len();
    // najpierw szybki skan co brakuje, potem równolegle (16 połączeń)
    struct AssetJob {
        url: String,
        dest: std::path::PathBuf,
        hash: String,
    }
    let mut jobs: Vec<AssetJob> = vec![];
    for (_name, obj) in objects.iter() {
        let hash = obj.get("hash").and_then(|h| h.as_str()).unwrap_or("");
        if hash.len() < 2 {
            continue;
        }
        let sub = format!("{}/{}", &hash[0..2], hash);
        let dest = base.join("assets").join("objects").join(&sub);
        if dest.exists() {
            continue;
        }
        jobs.push(AssetJob {
            url: format!("https://resources.download.minecraft.net/{sub}"),
            dest,
            hash: hash.to_string(),
        });
    }
    {
        use futures_util::stream::{self, StreamExt};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let total_missing = jobs.len();
        let done = Arc::new(AtomicUsize::new(0));
        stream::iter(jobs)
            .map(|job| {
                let app = app.clone();
                let done = done.clone();
                async move {
                    let r = download_file_report(
                        &job.url,
                        &job.dest,
                        Some((&app, "Downloading assets", 0, total)),
                        &job.hash,
                    )
                    .await
                    .map_err(|e| format!("asset {}: {e}", job.hash));
                    let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                    if n % 50 == 0 || n == total_missing {
                        emit(&app, "Downloading assets", n, total);
                    }
                    r
                }
            })
            .buffer_unordered(16)
            .collect::<Vec<Result<(), String>>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(())
}

/// Rozpakowuje natywne biblioteki (dll/so) dla wersji do shared dir
/// i zwraca ścieżkę do ustawienia jako -Djava.library.path.
/// Bez tego stare wersje (1.8.9 i okolice) padają na UnsatisfiedLinkError: no lwjgl64.
fn ensure_natives(app: &tauri::AppHandle, version_json: &Value, version_id: &str) -> Result<std::path::PathBuf, String> {
    let base = base_dir(app)?;
    let dir = base.join("natives").join(version_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let libs_dir = base.join("libraries");

    let mut jars: Vec<std::path::PathBuf> = vec![];
    if let Some(libs) = version_json.get("libraries").and_then(|l| l.as_array()) {
        for lib in libs {
            if !rule_allows(lib.get("rules").and_then(|r| r.as_array())) {
                continue;
            }
            let Some(classifiers) = lib.pointer("/downloads/classifiers") else { continue };
            let key = if cfg!(target_os = "windows") {
                "natives-windows"
            } else if cfg!(target_os = "macos") {
                "natives-macos"
            } else {
                "natives-linux"
            };
            let mut picked: Option<&Value> = classifiers.get(key);
            if picked.is_none() {
                if let Some(obj) = classifiers.as_object() {
                    for (k, val) in obj {
                        if k.contains("windows") && cfg!(target_os = "windows")
                            || k.contains("linux") && !cfg!(target_os = "windows") && !cfg!(target_os = "macos")
                            || k.contains("macos") && cfg!(target_os = "macos")
                            || k.contains("osx") && cfg!(target_os = "macos")
                        {
                            picked = Some(val);
                            break;
                        }
                    }
                }
            }
            if let Some(path) = picked.and_then(|n| n.get("path")).and_then(|p| p.as_str()) {
                let jar = libs_dir.join(path);
                if jar.exists() {
                    jars.push(jar);
                }
            }
        }
    }

    let marker = dir.join(".extracted");
    let want = jars.len().to_string();
    let have = std::fs::read_to_string(&marker).unwrap_or_default();
    if have.trim() != want.trim() || want == "0" {
        for jar in &jars {
            extract_natives(jar, &dir)?;
        }
        std::fs::write(&marker, &want).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

fn extract_natives(jar: &std::path::PathBuf, dir: &std::path::PathBuf) -> Result<(), String> {
    let f = std::fs::File::open(jar).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        if name.starts_with("META-INF") || entry.is_dir() {
            continue;
        }
        let leaf = name.rsplit('/').next().unwrap_or(&name);
        if !(leaf.ends_with(".dll") || leaf.ends_with(".so") || leaf.ends_with(".dylib")) {
            continue;
        }
        let dest = dir.join(leaf);
        let mut out = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
        use std::io::copy;
        copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn create_instance(
    app: tauri::AppHandle,
    name: String,
    version_id: String,
    loader: Option<String>,
    loader_version: Option<String>,
) -> Result<Instance, String> {
    let base = base_dir(&app)?;
    let path = base.join("instances.json");
    let mut list: Vec<Instance> = if path.exists() {
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).unwrap_or_default()
    } else {
        vec![]
    };
    let id = format!(
        "{}-{}",
        version_id.to_lowercase().replace('.', "_"),
        chrono::Utc::now().timestamp_millis() % 100000
    );
    let loader_norm = loader.unwrap_or_else(|| "vanilla".into()).to_lowercase();
    let loader_ver = match loader_version {
        Some(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    };
    if loader_norm != "vanilla" && loader_ver.is_none() {
        return Err(format!("loader version required for {loader_norm}"));
    }
    let inst = Instance {
        id: id.clone(),
        name: if name.trim().is_empty() {
            version_id.clone()
        } else {
            name.trim().to_string()
        },
        version_id: version_id.clone(),
        loader: loader_norm,
        loader_version: loader_ver,
        ram_mb: 2048,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let _ = instance_dir(&app, &id)?;
    list.push(inst.clone());
    std::fs::write(&path, serde_json::to_string_pretty(&list).unwrap()).map_err(|e| e.to_string())?;
    Ok(inst)
}

#[tauri::command]
pub async fn list_instances(app: tauri::AppHandle) -> Result<Vec<Instance>, String> {
    let path = base_dir(&app)?.join("instances.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

#[tauri::command]
pub async fn launch_instance(
    app: tauri::AppHandle,
    instance_id: String,
    account: Account,
) -> Result<String, String> {
    use std::process::Stdio;
    let base = base_dir(&app)?;
    let instances: Vec<Instance> = list_instances(app.clone()).await?;
    let inst = instances
        .into_iter()
        .find(|i| i.id == instance_id)
        .ok_or("instance not found")?;

    // ensure files present
    ensure_version_downloaded(app.clone(), inst.version_id.clone()).await?;

    // skins (non-fatal): vanilla fallback pack for every launch + CSL for modded
    let mut skin_notes: Vec<String> = vec![];
    match crate::skins::ensure_vanilla_skin_pack(&app, &inst, &account.username).await {
        Ok(None) => {}
        Ok(Some(note)) => skin_notes.push(note),
        Err(e) => skin_notes.push(format!("skin pack: {e}")),
    }
    if inst.loader != "vanilla" {
        match crate::skins::ensure_csl_for_instance(&app, &inst, &account.username).await {
            Ok(note) => {
                if let Some(n) = note {
                    skin_notes.push(n);
                }
            }
            Err(e) => skin_notes.push(format!("skins unavailable: {e}")),
        }
    }
    let loader_profile: Option<Value> = match inst.loader.as_str() {
        "fabric" | "quilt" => {
            let lv = inst.loader_version.clone().unwrap_or_default();
            emit(&app, &format!("Downloading {} loader {}", inst.loader, lv), 0, 1);
            Some(
                crate::loaders::ensure_loader_downloaded(
                    &app,
                    &inst.loader,
                    &inst.version_id,
                    &lv,
                )
                .await?,
            )
        }
        "forge" => {
            let lv = inst.loader_version.clone().unwrap_or_default();
            let full = format!("{}-{lv}", inst.version_id);
            let url = format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{full}/forge-{full}-installer.jar");
            // skip re-install if cached profile exists
            let cached = base_dir(&app)?.join("meta").join(format!("forge-{}-{lv}.json", inst.version_id));
            if cached.exists() {
                let raw = std::fs::read_to_string(&cached).map_err(|e| e.to_string())?;
                Some(serde_json::from_str(&raw).map_err(|e| e.to_string())?)
            } else {
                Some(crate::forge::ensure_forge_like_installed(&app, "forge", &inst.version_id, &lv, &url).await?)
            }
        }
        "neoforge" => {
            let lv = inst.loader_version.clone().unwrap_or_default();
            let url = format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{lv}/neoforge-{lv}-installer.jar");
            let cached = base_dir(&app)?.join("meta").join(format!("neoforge-{}-{lv}.json", inst.version_id));
            if cached.exists() {
                let raw = std::fs::read_to_string(&cached).map_err(|e| e.to_string())?;
                Some(serde_json::from_str(&raw).map_err(|e| e.to_string())?)
            } else {
                Some(crate::forge::ensure_forge_like_installed(&app, "neoforge", &inst.version_id, &lv, &url).await?)
            }
        }
        _ => None,
    };

    let meta_path = base.join("meta").join(format!("{}.json", inst.version_id));
    let raw = std::fs::read_to_string(&meta_path).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    let game_dir: PathBuf = instance_dir(&app, &inst.id)?;
    let client_jar = base
        .join("versions")
        .join(&inst.version_id)
        .join(format!("{}.jar", inst.version_id));

    // classpath: vanilla libs + client + loader libs
    let libs_dir = base.join("libraries");
    let mut cp_entries: Vec<String> = vec![client_jar.to_string_lossy().to_string()];
    if let Some(libs) = v.get("libraries").and_then(|l| l.as_array()) {
        for lib in libs {
            if !rule_allows(lib.get("rules").and_then(|r| r.as_array())) {
                continue;
            }
            if let Some(path) = lib.pointer("/downloads/artifact/path").and_then(|p| p.as_str()) {
                cp_entries.push(libs_dir.join(path).to_string_lossy().to_string());
            }
        }
    }
    let mut main_class = v
        .get("mainClass")
        .and_then(|m| m.as_str())
        .unwrap_or("net.minecraft.client.main.Main")
        .to_string();
    if let Some(profile) = loader_profile.as_ref() {
        if let Some(mc) = profile.get("mainClass").and_then(|m| m.as_str()) {
            main_class = mc.to_string();
        }
        if inst.loader == "fabric" || inst.loader == "quilt" {
            let extra = crate::loaders::loader_classpath_entries(&base, profile, &inst.loader);
            cp_entries.extend(extra);
        } else if inst.loader == "forge" || inst.loader == "neoforge" {
            let extra = crate::forge::forge_classpath_entries(&base, profile);
            cp_entries.extend(extra);
            // forge nub: version json may declare its own minecraftArguments additions;
            // for MVP keep vanilla game args (works for most modern forge).
        }
    }
    let sep = if cfg!(target_os = "windows") { ";" } else { ":" };
    let classpath = cp_entries.join(sep);

    let asset_index = v
        .pointer("/assetIndex/id")
        .and_then(|a| a.as_str())
        .unwrap_or("legacy");
    let assets_dir = base.join("assets").to_string_lossy().to_string();

    // java (auto-downloads Temurin if nothing suitable is installed)
    let java_bin = crate::java::ensure_java(&app, &v).await?;
    let mut args: Vec<String> = vec![
        format!("-Xmx{}M", inst.ram_mb),
        format!("-Xms{}M", 512.min(inst.ram_mb)),
    ];
    // natywne biblioteki (krytyczne dla starych wersji: lwjgl64.dll itd.)
    match ensure_natives(&app, &v, &inst.version_id) {
        Ok(natives) => args.push(format!("-Djava.library.path={}", natives.to_string_lossy())),
        Err(e) => skin_notes.push(format!("natives: {e}")),
    }
    args.extend(vec![
        "-cp".into(),
        classpath,
        main_class.into(),
        "--username".into(),
        account.username.clone(),
        "--version".into(),
        inst.version_id.clone(),
        "--gameDir".into(),
        game_dir.to_string_lossy().to_string(),
        "--assetsDir".into(),
        assets_dir,
        "--assetIndex".into(),
        asset_index.into(),
        "--uuid".into(),
        account.uuid.replace('-', ""),
        "--accessToken".into(),
        "0".into(),
        "--userType".into(),
        "legacy".into(),
        "--versionType".into(),
        "release".into(),
    ]);

    // spawn detached, pipe output to log file
    let log_path = game_dir.join("launcher.log");
    let log_file = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;

    // Use tauri shell? std Command is fine.
    let child = std::process::Command::new(&java_bin)
        .args(&args)
        .current_dir(&game_dir)
        .stdout(Stdio::from(log_file.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log_file))
        .spawn()
        .map_err(|e| format!("failed to launch java ({java_bin}): {e}"))?;

    let pid = child.id();
    // detach: don't wait
    std::mem::forget(child);
    let mut msg = format!("launched pid {pid}, log: {}", log_path.display());
    if !skin_notes.is_empty() {
        msg.push_str(&format!(" | skins: {}", skin_notes.join("; ")));
    }
    Ok(msg)
}
