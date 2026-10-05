import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type VersionEntry = { id: string; type: string; url: string; releaseTime: string };
type Account = { username: string; uuid: string };
type AccountStore = { accounts: Account[]; active: string | null };
type Instance = { id: string; name: string; version_id: string; loader: string; loader_version?: string; ram_mb: number };
type LoaderVersion = { version: string; stable?: boolean; full?: string };
type ModHit = { project_id: string; title: string; description: string; downloads: number; project_type: string };
type ModVersion = { id: string; version_number: string; game_versions: string[]; loaders: string[]; files: { filename: string; url: string; primary: boolean; size: number }[] };
type Loader = "vanilla" | "fabric" | "quilt" | "forge" | "neoforge";
type DlEvent = { stage: string; current: number; total: number; file?: string; done_bytes?: number; total_bytes?: number };

function fmtMB(b?: number) {
  if (b === undefined || b === null) return "";
  return `${(b / 1048576).toFixed(1)} MB`;
}

function DownloadBar({ dl, busy, doneFlash }: { dl: DlEvent | null; busy: boolean; doneFlash: boolean }) {
  if (doneFlash) {
    return (
      <div className="dlbar done">
        <span className="dl-check">✓</span>
        <span>Downloaded</span>
      </div>
    );
  }
  if (!dl || (!busy && Date.now() - (dl as any)._at > 8000)) return null;
  const bytePct = dl.total_bytes && dl.total_bytes > 0 && dl.done_bytes !== undefined
    ? Math.min(100, (dl.done_bytes / dl.total_bytes) * 100) : null;
  const countPct = dl.total > 1 ? Math.min(100, (dl.current / dl.total) * 100) : null;
  const pct = bytePct ?? countPct;
  const sub = dl.file
    ? `${dl.file} ${dl.done_bytes !== undefined && dl.total_bytes ? `· ${fmtMB(dl.done_bytes)} / ${fmtMB(dl.total_bytes)}` : dl.total > 1 ? `· file ${dl.current}/${dl.total}` : ""}`
    : dl.total > 1 ? `${dl.current}/${dl.total}` : "";
  return (
    <div className="dlbar">
      <div className="dl-top"><span className="dl-stage">{dl.stage}</span><span className="dl-sub">{sub}{pct !== null ? ` · ${pct.toFixed(0)}%` : ""}</span></div>
      <div className="dl-track">{pct !== null ? <div className="dl-fill" style={{ width: `${pct}%` }} /> : <div className="dl-fill indeterminate" />}</div>
    </div>
  );
}
type Tab = "play" | "mods" | "skins" | "settings";

/* ---------- skin head avatar ---------- */
function MiniHead({ username }: { username: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [has, setHas] = useState(false);
  useEffect(() => {
    let dead = false;
    (async () => {
      try {
        const data = await invoke<string>("get_skin_file", { username, kind: "skin" });
        if (dead) return;
        const img = new Image();
        img.onload = () => {
          const c = ref.current;
          if (!c) return;
          const ctx = c.getContext("2d");
          if (!ctx) return;
          ctx.imageSmoothingEnabled = false;
          ctx.clearRect(0, 0, 32, 32);
          ctx.drawImage(img, 8, 8, 8, 8, 0, 0, 32, 32);
          ctx.drawImage(img, 40, 8, 8, 8, 0, 0, 32, 32);
          setHas(true);
        };
        img.src = data;
      } catch {
        if (!dead) setHas(false);
      }
    })();
    return () => { dead = true; };
  }, [username]);
  if (!has) return <span className="avatar-fallback">{username.slice(0, 1).toUpperCase()}</span>;
  return <canvas ref={ref} width={32} height={32} className="avatar-canvas" />;
}

/* ---------- full body preview ---------- */
function drawPart(ctx: CanvasRenderingContext2D, img: HTMLImageElement, sx: number, sy: number, w: number, h: number, dx: number, dy: number, s: number, flip = false, overlay?: { sx: number; sy: number }) {
  ctx.save();
  if (flip) { ctx.translate(dx + w * s, dy); ctx.scale(-1, 1); ctx.drawImage(img, sx, sy, w, h, 0, 0, w * s, h * s); }
  else ctx.drawImage(img, sx, sy, w, h, dx, dy, w * s, h * s);
  ctx.restore();
  if (overlay) {
    ctx.save();
    if (flip) { ctx.translate(dx + w * s, dy); ctx.scale(-1, 1); ctx.drawImage(img, overlay.sx, overlay.sy, w, h, 0, 0, w * s, h * s); }
    else ctx.drawImage(img, overlay.sx, overlay.sy, w, h, dx, dy, w * s, h * s);
    ctx.restore();
  }
}

function SkinPreview({ dataUrl, slim }: { dataUrl: string | null; slim: boolean }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, 96, 144);
    if (!dataUrl) return;
    const img = new Image();
    img.onload = () => {
      const s = 4;
      ctx.imageSmoothingEnabled = false;
      drawPart(ctx, img, 8, 8, 8, 8, 32, 4, s, false, { sx: 40, sy: 8 });
      drawPart(ctx, img, 20, 20, 8, 12, 32, 36, s, false, { sx: 20, sy: 36 });
      drawPart(ctx, img, 44, 20, 4, 12, 12, 36, s, false, { sx: 44, sy: 36 });
      drawPart(ctx, img, 4, 20, 4, 12, 32, 84, s, false, { sx: 4, sy: 36 });
      if (img.height >= 64) {
        drawPart(ctx, img, 36, 52, 4, 12, 68, 36, s, false, { sx: 52, sy: 52 });
        drawPart(ctx, img, 20, 52, 4, 12, 48, 84, s, false, { sx: 4, sy: 52 });
      } else {
        drawPart(ctx, img, 44, 20, 4, 12, 68, 36, s, true, { sx: 44, sy: 36 });
        drawPart(ctx, img, 4, 20, 4, 12, 48, 84, s, true, { sx: 4, sy: 36 });
      }
      void slim;
    };
    img.src = dataUrl;
  }, [dataUrl, slim]);
  return <canvas ref={ref} width={96} height={144} className="skin-canvas" />;
}

/* ---------- app ---------- */
export default function App() {
  const [tab, setTab] = useState<Tab>("play");
  const [store, setStore] = useState<AccountStore | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuName, setMenuName] = useState("");
  const [onboardName, setOnboardName] = useState("");

  const active = store?.accounts.find((a) => a.username === store.active) ?? null;

  const [versions, setVersions] = useState<VersionEntry[]>([]);
  const [selectedVersion, setSelectedVersion] = useState("1.21");
  const [instances, setInstances] = useState<Instance[]>([]);
  const [instanceName, setInstanceName] = useState("");
  const [status, setStatus] = useState("");
  const [dl, setDl] = useState<DlEvent | null>(null);
  const [dlOk, setDlOk] = useState(false);
  const [busy, setBusy] = useState(false);
  const [javaPath, setJavaPath] = useState("");
  const [javaInfo, setJavaInfo] = useState("");
  const [showSnapshots, setShowSnapshots] = useState(false);
  const [loader, setLoader] = useState<Loader>("vanilla");
  const [loaderVersions, setLoaderVersions] = useState<LoaderVersion[]>([]);
  const [selectedLoaderVersion, setSelectedLoaderVersion] = useState("");

  const [modInstanceId, setModInstanceId] = useState("");
  const [searchQuery, setSearchQuery] = useState("sodium");
  const [searchResults, setSearchResults] = useState<ModHit[]>([]);
  const [expandedProject, setExpandedProject] = useState<string | null>(null);
  const [projectVersions, setProjectVersions] = useState<Record<string, ModVersion[]>>({});
  const [instanceMods, setInstanceMods] = useState<string[]>([]);
  const [mrpackInput, setMrpackInput] = useState("");

  const [skinUser, setSkinUser] = useState("");
  const [skinModel, setSkinModel] = useState<"classic" | "slim">("classic");
  const [skinUsers, setSkinUsers] = useState<{ username: string; has_skin: boolean; has_cape: boolean; has_elytra: boolean; model: string }[]>([]);
  const [skinPreview, setSkinPreview] = useState<string | null>(null);
  const [capePreview, setCapePreview] = useState<string | null>(null);
  const [cslStatus, setCslStatus] = useState<Record<string, boolean>>({});

  async function refreshAccounts() {
    try {
      const s = await invoke<AccountStore>("list_accounts");
      setStore(s);
      if (s.active) setSkinUser((prev) => prev || s.active || "");
      return s;
    } catch (e) { setStatus(String(e)); return null; }
  }

  async function refreshVersions() {
    try {
      const list = await invoke<VersionEntry[]>("fetch_version_list", { filter: showSnapshots ? null : "release" });
      setVersions(list);
      if (list.length > 0 && !list.find((v) => v.id === selectedVersion)) setSelectedVersion(list[0].id);
    } catch (e) { setStatus(String(e)); }
  }

  async function refreshInstances() {
    try {
      const list = await invoke<Instance[]>("list_instances");
      setInstances(list);
      if (list.length > 0 && !modInstanceId) setModInstanceId(list[0].id);
    } catch (e) { setStatus(String(e)); }
  }

  async function refreshJava() {
    try {
      const info: any = await invoke("get_java_info");
      const runtimes = (info.runtimes as { path: string; major: number }[]) || [];
      setJavaInfo(runtimes.map((r) => `Java ${r.major}: ${r.path}`).join("\n") || "none found");
    } catch (e) { setJavaInfo(String(e)); }
    try {
      const s: any = await invoke("load_settings");
      if (s.java_path) setJavaPath(s.java_path);
    } catch {}
  }

  async function refreshInstanceMods(id: string) {
    if (!id) return;
    try { setInstanceMods(await invoke<string[]>("list_instance_mods", { instanceId: id })); }
    catch { setInstanceMods([]); }
  }

  async function refreshSkins(forUser?: string) {
    const target = forUser ?? skinUser ?? store?.active ?? "";
    try {
      const users = await invoke<{ username: string; has_skin: boolean; has_cape: boolean; has_elytra: boolean; model: string }[]>("list_skinned_users");
      setSkinUsers(users);
      if (target) {
        for (const kind of ["skin", "cape"] as const) {
          try {
            const data = await invoke<string>("get_skin_file", { username: target, kind });
            if (kind === "skin") setSkinPreview(data); else setCapePreview(data);
          } catch {
            if (kind === "skin") setSkinPreview(null); else setCapePreview(null);
          }
        }
      }
    } catch (e) { setStatus(String(e)); }
    const st: Record<string, boolean> = {};
    for (const i of instances) {
      if (i.loader !== "vanilla") {
        try { st[i.id] = await invoke<boolean>("instance_csl_present", { instanceId: i.id }); }
        catch { st[i.id] = false; }
      }
    }
    setCslStatus(st);
  }

  useEffect(() => {
    refreshAccounts();
    refreshVersions();
    refreshInstances();
    refreshJava();
    const unlisten = listen<DlEvent>(
      "download-progress",
      (e) => {
        setDl({ ...e.payload, _at: Date.now() } as DlEvent);
        setDlOk(false);
        if (e.payload.stage === "Done") {
          setDlOk(true);
          setTimeout(() => { setDlOk(false); setDl(null); }, 4000);
        }
      }
    );
    return () => { unlisten.then((f) => f()); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showSnapshots]);

  useEffect(() => {
    async function load() {
      if (loader === "vanilla") { setLoaderVersions([]); return; }
      try {
        let list: LoaderVersion[] = [];
        if (loader === "fabric") list = await invoke<LoaderVersion[]>("list_fabric_loaders", { gameVersion: selectedVersion });
        else if (loader === "quilt") list = await invoke<LoaderVersion[]>("list_quilt_loaders", { gameVersion: selectedVersion });
        else if (loader === "forge") list = (await invoke<any[]>("list_forge_versions", { gameVersion: selectedVersion })).map((r) => ({ version: r.version, full: r.full }));
        else if (loader === "neoforge") list = (await invoke<any[]>("list_neoforge_versions", { gameVersion: selectedVersion })).map((r) => ({ version: r.version, full: r.full }));
        setLoaderVersions(list);
        if (list.length > 0) setSelectedLoaderVersion(list[0].version);
        setStatus("");
      } catch (e) { setStatus(String(e)); setLoaderVersions([]); }
    }
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loader, selectedVersion]);

  useEffect(() => { refreshInstanceMods(modInstanceId); }, [modInstanceId]);

  useEffect(() => {
    if (store?.active) setSkinUser((s) => s || store.active || "");
  }, [store?.active]);

  /* ----- accounts ----- */
  async function handleAddAccount(name: string, after?: () => void) {
    if (!name.trim()) return;
    try {
      const s = await invoke<AccountStore>("add_account", { username: name.trim() });
      setStore(s);
      setStatus(`Profile ${name.trim()} ready.`);
      after?.();
    } catch (e) { setStatus(String(e)); }
  }

  async function handleRemoveAccount(name: string) {
    try {
      const s = await invoke<AccountStore>("remove_account", { username: name });
      setStore(s);
    } catch (e) { setStatus(String(e)); }
  }

  /* ----- play ----- */
  async function handleCreateAndPlay() {
    if (!active) { setStatus("Create a profile first."); return; }
    if (loader !== "vanilla" && !selectedLoaderVersion) { setStatus("Pick a loader version first."); return; }
    setBusy(true);
    setStatus("");
    try {
      if (loader === "forge" || loader === "neoforge") setStatus(`Installing ${loader} ${selectedLoaderVersion} — official installer, may take minutes…`);
      else {
        setStatus(`Preparing ${selectedVersion} (${loader})…`);
        await invoke("ensure_version_downloaded", { versionId: selectedVersion });
      }
      const inst = await invoke<Instance>("create_instance", {
        name: instanceName || `${selectedVersion}${loader === "vanilla" ? "" : `-${loader}`}`,
        versionId: selectedVersion,
        loader,
        loaderVersion: loader === "vanilla" ? null : selectedLoaderVersion,
      });
      setInstanceName("");
      await refreshInstances();
      setModInstanceId(inst.id);
      setStatus(`Launching ${inst.name}…`);
      const msg = await invoke<string>("launch_instance", { instanceId: inst.id, account: active });
      setStatus(msg as string);
    } catch (e) { setStatus(`Error: ${e}`); }
    finally { setBusy(false); }
  }

  async function handleLaunch(id: string) {
    if (!active) { setStatus("Create a profile first."); return; }
    setBusy(true);
    try {
      setStatus("Launching…");
      const msg = await invoke<string>("launch_instance", { instanceId: id, account: active });
      setStatus(msg as string);
    } catch (e) { setStatus(`Error: ${e}`); }
    finally { setBusy(false); }
  }

  /* ----- mods ----- */
  async function handleSearch() {
    const inst = instances.find((i) => i.id === modInstanceId);
    try {
      setStatus("Searching Modrinth…");
      const res = await invoke<ModHit[]>("modrinth_search", {
        query: searchQuery,
        gameVersion: inst?.version_id ?? null,
        loader: inst?.loader ?? null,
        projectType: "mod",
      });
      setSearchResults(res);
      setStatus(`Found ${res.length} projects.`);
    } catch (e) { setStatus(String(e)); }
  }

  async function toggleProject(projectId: string) {
    if (expandedProject === projectId) { setExpandedProject(null); return; }
    setExpandedProject(projectId);
    if (projectVersions[projectId]) return;
    const inst = instances.find((i) => i.id === modInstanceId);
    try {
      const vers = await invoke<ModVersion[]>("modrinth_versions", { projectId, gameVersion: inst?.version_id ?? null, loader: inst?.loader ?? null });
      setProjectVersions((p) => ({ ...p, [projectId]: vers }));
    } catch (e) { setStatus(String(e)); }
  }

  async function installModFile(fileUrl: string, filename: string) {
    if (!modInstanceId) { setStatus("Select an instance first."); return; }
    try {
      setStatus(`Installing ${filename}…`);
      const msg = await invoke<string>("modrinth_install_file", { instanceId: modInstanceId, fileUrl, filename, subdir: "mods" });
      setStatus(msg as string);
      refreshInstanceMods(modInstanceId);
    } catch (e) { setStatus(`Error: ${e}`); }
  }

  async function handleMrpackInstall() {
    if (!mrpackInput.trim()) { setStatus("Paste a .mrpack path or URL."); return; }
    setBusy(true);
    try {
      setStatus("Installing modpack…");
      const inst = await invoke<Instance>("install_mrpack", { packPathOrUrl: mrpackInput.trim(), instanceId: null, newName: null });
      setMrpackInput("");
      await refreshInstances();
      setModInstanceId(inst.id);
      setStatus(`Modpack installed as "${inst.name}".`);
    } catch (e) { setStatus(`Error: ${e}`); }
    finally { setBusy(false); }
  }

  /* ----- skins ----- */
  async function handleSkinUpload(kind: "skin" | "cape" | "elytra", file: File | undefined) {
    const target = (skinUser || store?.active || "").trim();
    if (!target) { setStatus("Pick a profile first."); return; }
    if (!file) return;
    const dataUrl = await new Promise<string>((res, rej) => {
      const r = new FileReader();
      r.onload = () => res(r.result as string);
      r.onerror = rej;
      r.readAsDataURL(file);
    });
    try {
      await invoke("save_skin_file", { username: target, kind, dataBase64: dataUrl, model: skinModel });
      setStatus(`${kind} saved for ${target}. Applies on next launch.`);
      refreshSkins(target);
    } catch (e) { setStatus(String(e)); }
  }

  async function saveJavaPath() {
    await invoke("save_settings", { settings: { java_path: javaPath, ram_default_mb: 2048 } });
    refreshJava();
    setStatus("Settings saved.");
  }

  const modInst = instances.find((i) => i.id === modInstanceId);
  const needsOnboarding = store !== null && store.accounts.length === 0;

  /* ----- onboarding ----- */
  if (needsOnboarding) {
    return (
      <div className="onboard">
        <div className="onboard-card">
          <div className="logo big">⬢ Obsidian</div>
          <p className="muted">Offline Minecraft launcher. Create your first profile to start — no account needed, pick any name.</p>
          <div className="row center">
            <input
              value={onboardName}
              onChange={(e) => setOnboardName(e.target.value)}
              maxLength={16}
              placeholder="Username"
              onKeyDown={(e) => { if (e.key === "Enter") handleAddAccount(onboardName); }}
            />
            <button className="play" onClick={() => handleAddAccount(onboardName)}>Create</button>
          </div>
          {status && <pre className="status">{status}</pre>}
        </div>
      </div>
    );
  }

  return (
    <div className="shell">
      <header className="topbar">
        <div className="brand">⬢ Obsidian</div>
        <div className="account-wrap">
          <button className="account-chip" onClick={() => setMenuOpen((o) => !o)}>
            {active && <MiniHead username={active.username} />}
            <span>{active?.username ?? "…"}</span>
            <span className="caret">▾</span>
          </button>
          {menuOpen && store && (
            <div className="account-menu">
              {store.accounts.map((a) => (
                <div key={a.username} className={`account-row${a.username === store.active ? " current" : ""}`}>
                  <button
                    className="account-pick"
                    onClick={async () => {
                      const s = await invoke<AccountStore>("switch_account", { username: a.username });
                      setStore(s);
                      setMenuOpen(false);
                    }}
                  >
                    <MiniHead username={a.username} />
                    <span>{a.username}</span>
                  </button>
                  <button
                    className="account-del"
                    title={`Remove ${a.username}`}
                    onClick={() => handleRemoveAccount(a.username)}
                  >×</button>
                </div>
              ))}
              <div className="account-add">
                <input
                  value={menuName}
                  onChange={(e) => setMenuName(e.target.value)}
                  maxLength={16}
                  placeholder="New profile…"
                  onKeyDown={(e) => { if (e.key === "Enter") handleAddAccount(menuName, () => setMenuName("")); }}
                />
                <button onClick={() => handleAddAccount(menuName, () => setMenuName(""))}>Add</button>
              </div>
            </div>
          )}
        </div>
      </header>

      <div className="body">
        <nav className="rail">
          {(["play", "mods", "skins", "settings"] as Tab[]).map((t) => (
            <button key={t} className={tab === t ? "active" : ""} onClick={() => { setTab(t); if (t === "skins") refreshSkins(); }}>
              <span className="glyph">{t === "play" ? "▶" : t === "mods" ? "▤" : t === "skins" ? "◍" : "⚙"}</span>
              <span className="lbl">{t[0].toUpperCase() + t.slice(1)}</span>
            </button>
          ))}
        </nav>

        <main className="main">
          {tab === "play" && (
            <>
              <section className="card hero">
                <div>
                  <h1>{instanceName || selectedVersion} <span className="muted">{loader !== "vanilla" ? `· ${loader}` : ""}</span></h1>
                  <p className="muted">Playing as {active?.username}</p>
                </div>
                <button className="play big" disabled={busy} onClick={handleCreateAndPlay}>{busy ? "Working…" : "▶ Play"}</button>
              </section>

              <section className="card">
                <h2>Setup</h2>
                <div className="row">
                  <input value={instanceName} onChange={(e) => setInstanceName(e.target.value)} placeholder="Instance name" />
                  <select value={selectedVersion} onChange={(e) => setSelectedVersion(e.target.value)}>
                    {versions.slice(0, 80).map((v) => <option key={v.id} value={v.id}>{v.id}</option>)}
                  </select>
                  <select value={loader} onChange={(e) => setLoader(e.target.value as Loader)}>
                    <option value="vanilla">Vanilla</option>
                    <option value="fabric">Fabric</option>
                    <option value="quilt">Quilt</option>
                    <option value="forge">Forge</option>
                    <option value="neoforge">NeoForge</option>
                  </select>
                  {loader !== "vanilla" && (
                    <select value={selectedLoaderVersion} onChange={(e) => setSelectedLoaderVersion(e.target.value)}>
                      {loaderVersions.slice(0, 30).map((lv) => <option key={lv.version} value={lv.version}>{lv.version}{lv.stable === false ? " (unstable)" : ""}</option>)}
                    </select>
                  )}
                  <label className="check"><input type="checkbox" checked={showSnapshots} onChange={(e) => setShowSnapshots(e.target.checked)} /> snapshots</label>
                </div>
                {(loader === "forge" || loader === "neoforge") && <p className="muted">Runs the official installer headless (one-time, minutes).</p>}
                
              </section>

              <section className="card">
                <h2>Instances</h2>
                {instances.length === 0 && <p className="muted">No instances yet — hit Play above.</p>}
                {instances.map((i) => (
                  <div key={i.id} className="inst">
                    <div><b>{i.name}</b> <span className="muted">{i.version_id} · {i.loader}{i.loader_version ? ` ${i.loader_version}` : ""}{i.loader === "vanilla" ? " · skin ok, no cape" : ""}</span></div>
                    <button disabled={busy} onClick={() => handleLaunch(i.id)}>Play</button>
                  </div>
                ))}
              </section>
              {status && <pre className="status">{status}</pre>}
            </>
          )}

          {tab === "mods" && (
            <>
              <section className="card">
                <h2>Modrinth</h2>
                <div className="row">
                  <select value={modInstanceId} onChange={(e) => setModInstanceId(e.target.value)}>
                    {instances.map((i) => <option key={i.id} value={i.id}>{i.name} ({i.version_id} {i.loader})</option>)}
                  </select>
                  <input value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} placeholder="Search mods…" style={{ flex: 1 }} />
                  <button onClick={handleSearch}>Search</button>
                </div>
                {modInst && <p className="muted">Into {modInst.name} — {modInst.version_id} / {modInst.loader}</p>}
                {searchResults.map((h) => (
                  <div key={h.project_id} className="inst col">
                    <div className="row spread">
                      <div><b>{h.title}</b> <span className="muted">⬇ {h.downloads.toLocaleString()}</span>
                        <div className="muted">{h.description.slice(0, 120)}</div>
                      </div>
                      <button onClick={() => toggleProject(h.project_id)}>{expandedProject === h.project_id ? "Hide" : "Versions"}</button>
                    </div>
                    {expandedProject === h.project_id && (projectVersions[h.project_id] || []).map((v) => {
                      const primary = v.files.find((f) => f.primary) || v.files[0];
                      return primary ? (
                        <div key={v.id} className="row">
                          <span className="muted">{v.version_number} ({v.game_versions.slice(0, 3).join(", ")})</span>
                          <button onClick={() => installModFile(primary.url, primary.filename)}>Install {primary.filename}</button>
                        </div>
                      ) : null;
                    })}
                  </div>
                ))}
              </section>

              <section className="card">
                <h2>Installed ({instanceMods.length})</h2>
                {instanceMods.length === 0 && <p className="muted">Nothing here yet.</p>}
                {instanceMods.map((m) => (
                  <div key={m} className="inst">
                    <span>{m}</span>
                    <button onClick={async () => { await invoke("delete_instance_mod", { instanceId: modInstanceId, filename: m }); refreshInstanceMods(modInstanceId); }}>Delete</button>
                  </div>
                ))}
              </section>

              <section className="card">
                <h2>Modpack (.mrpack)</h2>
                <div className="row">
                  <input value={mrpackInput} onChange={(e) => setMrpackInput(e.target.value)} placeholder="Path or https:// URL" style={{ flex: 1 }} />
                  <button disabled={busy} onClick={handleMrpackInstall}>Install</button>
                </div>
                
              </section>
              {status && <pre className="status">{status}</pre>}
            </>
          )}

          {tab === "skins" && (
            <>
              <section className="card">
                <h2>Skins</h2>
                <p className="muted">Every instance type: skin via generated resource pack. Modded adds cape + elytra through CustomSkinLoader. Client-side only.</p>
                <div className="row">
                  <select value={skinUser} onChange={(e) => { setSkinUser(e.target.value); refreshSkins(e.target.value); }}>
                    {store?.accounts.map((a) => <option key={a.username} value={a.username}>{a.username}</option>)}
                  </select>
                  <select value={skinModel} onChange={(e) => setSkinModel(e.target.value as any)}>
                    <option value="classic">Classic</option>
                    <option value="slim">Slim</option>
                  </select>
                  <button onClick={() => refreshSkins()}>Refresh</button>
                </div>
                <div className="row top">
                  <SkinPreview dataUrl={skinPreview} slim={skinModel === "slim"} />
                  <div className="upcol">
                    {(["skin", "cape", "elytra"] as const).map((kind) => (
                      <div key={kind} className="row">
                        <b className="kw">{kind}</b>
                        <label className="upload">
                          <input type="file" accept=".png,image/png" onChange={(e) => { handleSkinUpload(kind, e.target.files?.[0]); e.target.value = ""; }} />
                          <span>Upload</span>
                        </label>
                        <button onClick={async () => {
                          try { await invoke("delete_skin_file", { username: skinUser || store?.active, kind }); refreshSkins(); }
                          catch (e) { setStatus(String(e)); }
                        }}>Delete</button>
                      </div>
                    ))}
                    {capePreview && <img src={capePreview} alt="cape" className="cape-thumb" />}
                  </div>
                </div>
              </section>

              <section className="card">
                <h2>Saved ({skinUsers.length})</h2>
                {skinUsers.length === 0 && <p className="muted">Nothing saved yet.</p>}
                {skinUsers.map((u) => (
                  <div key={u.username} className="inst">
                    <div><b>{u.username}</b> <span className="muted">{[u.has_skin && "skin", u.has_cape && "cape", u.has_elytra && "elytra"].filter(Boolean).join(" + ")} · {u.model}</span></div>
                    <button onClick={() => { setSkinUser(u.username); setSkinModel(u.model as any); refreshSkins(u.username); }}>Load</button>
                  </div>
                ))}
              </section>

              <section className="card">
                <h2>Skin mod</h2>
                {instances.filter((i) => i.loader !== "vanilla").length === 0 && <p className="muted">No modded instances.</p>}
                {instances.filter((i) => i.loader !== "vanilla").map((i) => (
                  <div key={i.id} className="inst">
                    <div><b>{i.name}</b> <span className="muted">{cslStatus[i.id] ? "installed" : "auto-installs on launch"}</span></div>
                  </div>
                ))}
              </section>
              {status && <pre className="status">{status}</pre>}
            </>
          )}

          {tab === "settings" && (
            <section className="card">
              <h2>Java</h2>
              <p className="muted">Auto-detected, Temurin downloaded as fallback. Manual path is a last resort.</p>
              <div className="row">
                <input value={javaPath} onChange={(e) => setJavaPath(e.target.value)} placeholder="Optional override…" style={{ flex: 1 }} />
                <button onClick={saveJavaPath}>Save</button>
              </div>
              <pre className="status">{javaInfo}</pre>
              <h2>About</h2>
              <ul className="muted compact">
                <li>Vanilla / Fabric / Quilt / Forge / NeoForge</li>
                <li>Modrinth + .mrpack · Skins + capes · Java auto-provision</li>
              </ul>
            </section>
          )}
        </main>
      </div>
      <DownloadBar dl={dl} busy={busy} doneFlash={dlOk} />
    </div>
  );
}
