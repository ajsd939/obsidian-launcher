mod accounts;
mod files;
mod forge;
mod game;
mod java;
mod loaders;
mod models;
mod modrinth;
mod mojang;
mod skins;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            mojang::create_offline_account,
            accounts::list_accounts,
            accounts::add_account,
            accounts::switch_account,
            accounts::remove_account,
            mojang::fetch_version_manifest,
            mojang::fetch_version_list,
            game::ensure_version_downloaded,
            game::create_instance,
            game::list_instances,
            game::launch_instance,
            loaders::list_fabric_loaders,
            loaders::list_quilt_loaders,
            forge::list_forge_versions,
            forge::list_neoforge_versions,
            modrinth::modrinth_search,
            modrinth::modrinth_versions,
            modrinth::modrinth_install_file,
            modrinth::list_instance_mods,
            modrinth::delete_instance_mod,
            modrinth::install_mrpack,
            skins::list_skinned_users,
            skins::save_skin_file,
            skins::get_skin_file,
            skins::delete_skin_file,
            skins::instance_csl_present,
            java::get_java_info,
            java::save_settings,
            java::load_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
