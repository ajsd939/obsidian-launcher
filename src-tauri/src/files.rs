use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::Manager;
use tokio::io::AsyncWriteExt;

/// Jeden współdzielony klient HTTP (keep-alive) dla wszystkich pobrań.

/// App data layout:
/// <base>/obsidian-launcher/
///   accounts.json, instances.json, settings.json
///   meta/          (cached version jsons)
///   libraries/     (shared Mojang libraries)
///   assets/        (shared assets)
///   runtimes/      (java runtimes)
///   instances/<id>/ (.minecraft per instance: client.jar, mods/, resourcepacks/, saves/)

pub fn base_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir failed: {e}"))?;
    // app_data_dir already includes app identifier; keep as-is
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn instance_dir(app: &tauri::AppHandle, instance_id: &str) -> Result<PathBuf, String> {
    let dir = base_dir(app)?.join("instances").join(instance_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("obsidian-launcher/0.1 (offline-mc-launcher)")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

pub async fn download_file(url: &str, dest: &PathBuf) -> Result<(), String> {
    download_file_report(url, dest, None, "").await
}

/// Streaming download with byte-level progress events.
/// `reporter`: (app handle, stage label, file index, file total). Emits
/// `download-progress` {stage, current, total, file, done_bytes, total_bytes}.
/// Throttled to ~4 events/sec to avoid flooding the webview.
pub async fn download_file_report(
    url: &str,
    dest: &PathBuf,
    reporter: Option<(&tauri::AppHandle, &str, usize, usize)>,
    file_label: &str,
) -> Result<(), String> {
    use futures_util::StreamExt;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let resp = http()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("GET {url} failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GET {url} -> {}", resp.status()));
    }
    let total_bytes: Option<u64> = resp.content_length();
    let mut f = tokio::fs::File::create(dest)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut done: u64 = 0;
    let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(10);
    let mut last_pct: u64 = 101;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        f.write_all(&chunk).await.map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        if let Some((app, stage, current, total)) = reporter {
            let pct = total_bytes.map(|t| if t > 0 { done * 100 / t } else { 100 }).unwrap_or(101);
            if last_emit.elapsed().as_millis() >= 250 || pct != last_pct && pct % 5 == 0 {
                last_emit = std::time::Instant::now();
                last_pct = pct;
                emit_file_progress(app, stage, current, total, file_label, done, total_bytes);
            }
        }
    }
    if let Some((app, stage, current, total)) = reporter {
        emit_file_progress(app, stage, current, total, file_label, done, total_bytes.or(Some(done)));
    }
    Ok(())
}

fn emit_file_progress(
    app: &tauri::AppHandle,
    stage: &str,
    current: usize,
    total: usize,
    file: &str,
    done_bytes: u64,
    total_bytes: Option<u64>,
) {
    use tauri::Emitter;
    let _ = app.emit(
        "download-progress",
        serde_json::json!({
            "stage": stage,
            "current": current,
            "total": total,
            "file": file,
            "done_bytes": done_bytes,
            "total_bytes": total_bytes,
        }),
    );
}

pub fn sha1_matches(path: &PathBuf, expected: &str) -> bool {
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    let mut h = sha1::Sha1::new();
    use sha1::Digest;
    h.update(&data);
    let hex = format!("{:x}", h.finalize());
    hex.eq_ignore_ascii_case(expected)
}
