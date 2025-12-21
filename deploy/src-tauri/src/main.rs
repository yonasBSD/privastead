// prevents an extra console window on windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    secluso_deploy_lib::run()
}
