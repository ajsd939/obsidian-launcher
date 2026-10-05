# Obsidian Launcher

Offline Minecraft: Java Edition launcher (Tauri v2 + React + Rust). No Mojang/Microsoft account needed — offline profiles with proper offline UUIDs.

> For legal play you should own Minecraft. Offline mode only; custom skins are client-side.

## Features

- **Offline profiles** — multiple saved accounts, onboarding on first boot, persisted `accounts.json`
- **Vanilla + loaders** — Vanilla, Fabric, Quilt, Forge (incl. legacy ≤1.12 via direct `install_profile.json`), NeoForge
- **Auto Java** — probes installed JDKs (Adoptium, Microsoft, Mojang runtimes), downloads Temurin JRE (8/17/21) as fallback; Java 8 forced for ≤1.16
- **Mods** — Modrinth search/browser with version filtering, `.mrpack` import, per-instance mod list
- **Skins everywhere** — generated `obsidian-skin.zip` resource pack overriding all 20 default skins (correct `pack_format` per version, incl. `min/max_format` for 1.21.9+); capes/elytra on modded instances via auto-installed CustomSkinLoader + LocalSkin
- **Fast downloads** — shared keep-alive HTTP client, 12–16 parallel file downloads, per-file + session MB progress, retries, corrupt-partial cleanup
- **Natives** — extracts `lwjgl64.dll`/`.so` per version, sets `-Djava.library.path` (required for old versions like 1.8.9)
- **Quick-join servers** — server list with `servers.dat` sync + `--server/--port` launch args
- **Installer** — NSIS setup (`npm run tauri build`)

## Run dev

```sh
npm install
npm run tauri dev
```

Requires Rust (stable) + Node. Java auto-provisions on first launch (or point to a JDK in Settings).

## Build installer

```sh
npm run tauri build   # -> src-tauri/target/release/bundle/nsis/*-setup.exe
```

## Layout (runtime data)

`%APPDATA%/com.obsidian.launcher/` (or platform equivalent):

- `accounts.json`, `instances.json`, `settings.json`, `servers.json`, `skins/`
- `meta/`, `versions/`, `libraries/`, `assets/`, `natives/`, `runtimes/`, `installers/`
- `instances/<id>/` — isolated `.minecraft` per instance
