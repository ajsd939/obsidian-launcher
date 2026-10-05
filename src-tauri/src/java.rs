use crate::files::base_dir;
use tauri::Emitter;

/// Which Java major a version json needs.
/// Uses `javaVersion.majorVersion`; versions without it predate Java 16 → need 8.
pub fn required_java_major(version_json: &serde_json::Value) -> u32 {
    version_json
        .pointer("/javaVersion/majorVersion")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(8)
}

/// Parse `java -version` output into a major version.
/// Handles `openjdk version "21.0.2"` and legacy `java version "1.8.0_392"`.
fn parse_java_major(output: &str) -> Option<u32> {
    let first = output.lines().next().unwrap_or("");
    let quoted = first.split('"').nth(1)?;
    if let Some(rest) = quoted.strip_prefix("1.") {
        return rest.split('.').next()?.parse().ok();
    }
    quoted.split('.').next()?.parse().ok()
}

fn java_major_of(bin: &str) -> Option<u32> {
    let out = std::process::Command::new(bin).arg("-version").output().ok()?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    parse_java_major(&combined)
}

fn exe() -> &'static str {
    if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    }
}

/// All java binary candidates on this machine, best first.
/// Order: settings override → our managed runtimes → JAVA_HOME → well-known
/// vendor dirs → official Minecraft launcher runtimes → PATH.
fn probe_candidates(app: &tauri::AppHandle) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut push = |p: String| {
        if !p.is_empty() && !out.contains(&p) {
            out.push(p);
        }
    };

    push(read_settings_java(app));

    // our managed runtimes (any version dir)
    if let Ok(base) = base_dir(app) {
        let rt = base.join("runtimes");
        if let Ok(entries) = std::fs::read_dir(&rt) {
            for e in entries.flatten() {
                let bin = e.path().join("bin").join(format!("java{}", exe()));
                if bin.exists() {
                    push(bin.to_string_lossy().to_string());
                }
            }
        }
    }

    if let Ok(jh) = std::env::var("JAVA_HOME") {
        push(format!("{jh}/bin/java{}", exe()));
    }

    // vendor install dirs
    #[cfg(target_os = "windows")]
    {
        for root in [
            "C:\\Program Files\\Eclipse Adoptium",
            "C:\\Program Files (x86)\\Eclipse Adoptium",
            "C:\\Program Files\\Microsoft",
            "C:\\Program Files\\Java",
            "C:\\Program Files (x86)\\Java",
        ] {
            if let Ok(entries) = std::fs::read_dir(root) {
                for e in entries.flatten() {
                    let bin = e.path().join("bin").join("java.exe");
                    if bin.exists() {
                        push(bin.to_string_lossy().to_string());
                    }
                }
            }
        }
        // official Minecraft launcher runtimes (Mojang-shipped JREs!)
        if let Ok(appdata) = std::env::var("APPDATA") {
            let rt = std::path::Path::new(&appdata).join(".minecraft").join("runtime");
            collect_runtime_bins(&rt, &mut push);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        for root in ["/usr/lib/jvm", "/usr/java", "/opt"] {
            if let Ok(entries) = std::fs::read_dir(root) {
                for e in entries.flatten() {
                    for sub in ["bin/java", "jre/bin/java"] {
                        let bin = e.path().join(sub);
                        if bin.exists() {
                            push(bin.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let rt = std::path::Path::new(&home).join(".minecraft").join("runtime");
            collect_runtime_bins(&rt, &mut push);
        }
    }

    push(if cfg!(target_os = "windows") {
        "java.exe".to_string()
    } else {
        "java".to_string()
    });
    out
}

#[cfg(target_os = "windows")]
fn collect_runtime_bins(rt: &std::path::Path, push: &mut impl FnMut(String)) {
    // layout: runtime/<component>/<os>/<component>/bin/javaw.exe
    let mut stack = vec![rt.to_path_buf()];
    let mut depth = 0;
    while let Some(dir) = stack.pop() {
        depth += 1;
        if depth > 8 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().map(|n| n == "javaw.exe" || n == "java.exe").unwrap_or(false) {
                push(p.to_string_lossy().to_string());
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn collect_runtime_bins(rt: &std::path::Path, push: &mut impl FnMut(String)) {
    let mut stack = vec![rt.to_path_buf()];
    let mut depth = 0;
    while let Some(dir) = stack.pop() {
        depth += 1;
        if depth > 8 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().map(|n| n == "java").unwrap_or(false) {
                push(p.to_string_lossy().to_string());
            }
        }
    }
}

/// Pick the best installed java for `required`.
/// Old versions (requirement <= 8, i.e. MC <= 1.16) ONLY run on Java 8
/// (LaunchWrapper casts to URLClassLoader + legacy libs break on 9+).
fn pick_installed(app: &tauri::AppHandle, required: u32) -> Option<String> {
    // settings override wins outright if it runs at all
    let configured = read_settings_java(app);
    if !configured.is_empty() && java_major_of(&configured).is_some() {
        return Some(configured);
    }
    let acceptable = |major: u32| {
        if required <= 8 {
            major == 8
        } else {
            major >= required
        }
    };
    let mut best: Option<(u32, String)> = None;
    for c in probe_candidates(app) {
        if c == configured {
            continue;
        }
        if let Some(major) = java_major_of(&c) {
            if acceptable(major) && best.as_ref().map(|b| major < b.0).unwrap_or(true) {
                best = Some((major, c));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Download a Temurin JRE for `major` into our runtimes dir. Returns java bin path.
async fn download_temurin_jre(app: &tauri::AppHandle, major: u32) -> Result<String, String> {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let url = format!("https://api.adoptium.net/v3/binary/latest/{major}/ga/{os}/x64/jre/hotspot/normal/eclipse");
    let _ = app.emit(
        "download-progress",
        serde_json::json!({"stage": format!("Downloading Java {major} (Temurin, one-time)…"), "current": 0, "total": 1}),
    );
    let base = base_dir(app)?;
    let dl_dir = base.join("runtimes").join("_dl");
    std::fs::create_dir_all(&dl_dir).map_err(|e| e.to_string())?;
    let archive = dl_dir.join(if cfg!(target_os = "windows") {
        format!("jre-{major}.zip")
    } else {
        format!("jre-{major}.tar.gz")
    });
    if !archive.exists() {
        let label = format!("temurin-jre-{major}");
        let stage = format!("Downloading Java {major}");
        crate::files::download_file_report(&url, &archive, Some((app, stage.as_str(), 0, 1)), &label).await?;
    }
    let dest = base.join("runtimes").join(format!("jre-{major}"));
    if !dest.exists() {
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        #[cfg(target_os = "windows")]
        extract_zip(&archive, &dest)?;
        #[cfg(not(target_os = "windows"))]
        extract_tar_gz(&archive, &dest)?;
    }
    let _ = std::fs::remove_file(&archive);
    find_java_bin(&dest).ok_or_else(|| "downloaded JRE has no bin/java".into())
}

#[cfg(target_os = "windows")]
fn extract_zip(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    zip.extract(dest).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn extract_tar_gz(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let gz = flate2::read::GzDecoder::new(f);
    let mut tar = tar::Archive::new(gz);
    tar.unpack(dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// Archives contain a top-level dir; find bin/java under it.
fn find_java_bin(root: &std::path::Path) -> Option<String> {
    let direct = root.join("bin").join(format!("java{}", exe()));
    if direct.exists() {
        return Some(direct.to_string_lossy().to_string());
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            let bin = e.path().join("bin").join(format!("java{}", exe()));
            if bin.exists() {
                return Some(bin.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// Resolve a working java for this version json, downloading one if needed.
/// Call this from launch paths (async). Never returns bare "java" that doesn't exist.
pub async fn ensure_java(app: &tauri::AppHandle, version_json: &serde_json::Value) -> Result<String, String> {
    let required = required_java_major(version_json);
    if let Some(p) = pick_installed(app, required) {
        return Ok(p);
    }
    // nothing suitable → fetch Temurin (8/17/21 cover all MC versions)
    let want = if required <= 8 {
        8
    } else if required <= 17 {
        17
    } else {
        21
    };
    download_temurin_jre(app, want).await
}

#[tauri::command]
pub async fn get_java_info(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let mut found = vec![];
    for c in probe_candidates(&app) {
        if let Some(major) = java_major_of(&c) {
            found.push(serde_json::json!({ "path": c, "major": major }));
        }
    }
    if found.is_empty() {
        return Err("No Java found at all — the launcher will auto-download Temurin on first launch.".into());
    }
    Ok(serde_json::json!({ "runtimes": found }))
}

/// Back-compat sync resolver: best-effort only (launch path uses ensure_java).
pub fn resolve_java(app: &tauri::AppHandle, version_json: &serde_json::Value) -> Result<String, String> {
    let required = required_java_major(version_json);
    if let Some(p) = pick_installed(app, required) {
        return Ok(p);
    }
    Err(format!(
        "No Java {required}+ found. Install Temurin {required} or let the launcher auto-download it (restart the game launch)."
    ))
}

fn read_settings_java(app: &tauri::AppHandle) -> String {
    let Ok(base) = base_dir(app) else {
        return String::new();
    };
    let p = base.join("settings.json");
    if let Ok(raw) = std::fs::read_to_string(p) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            return v.get("java_path").and_then(|s| s.as_str()).unwrap_or("").to_string();
        }
    }
    String::new()
}

#[tauri::command]
pub async fn save_settings(app: tauri::AppHandle, settings: serde_json::Value) -> Result<(), String> {
    let base = base_dir(&app)?;
    std::fs::write(base.join("settings.json"), serde_json::to_string_pretty(&settings).unwrap())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn load_settings(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let base = base_dir(&app)?;
    let p = base.join("settings.json");
    if !p.exists() {
        return Ok(serde_json::json!({ "java_path": "", "ram_default_mb": 2048 }));
    }
    let raw = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    Ok(serde_json::from_str(&raw).unwrap_or(serde_json::json!({})))
}
