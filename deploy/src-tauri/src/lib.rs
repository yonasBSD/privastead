// more tauri command info at https://tauri.app/develop/calling-rust/

mod pi_hub_provision;
mod provision_server;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            pi_hub_provision::check_docker,
            pi_hub_provision::build_image,
            pi_hub_provision::generate_user_credentials,
            provision_server::test_server_ssh,
            provision_server::provision_server,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
