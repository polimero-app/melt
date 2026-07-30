use polimero_core::{
    AppInfo,
    config::{Config, config_dir},
    drivers,
    keychain::SystemKeychain,
    profiles,
};
use serde::Serialize;

#[tauri::command]
fn app_info() -> AppInfo {
    polimero_core::app_info()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrinterSummary {
    name: String,
    driver: String,
    host: String,
}

#[tauri::command]
fn configured_printers() -> Result<Vec<PrinterSummary>, String> {
    Config::load()
        .map(|config| {
            config
                .sorted_profiles()
                .into_iter()
                .map(|profile| PrinterSummary {
                    name: profile.name,
                    driver: profile.profile.driver,
                    host: profile.profile.host,
                })
                .collect()
        })
        .map_err(|_| "Unable to read printer configuration.".into())
}

#[tauri::command]
fn registered_drivers() -> Vec<drivers::DriverInfo> {
    drivers::registered().to_vec()
}

#[tauri::command]
fn remove_configured_printer(name: String) -> Result<profiles::RemoveResult, String> {
    let dir = config_dir().map_err(|_| "Unable to read printer configuration.".to_string())?;
    profiles::remove(dir, &SystemKeychain, &name).map_err(|error| error.to_string())
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(polimero_cli::run(
            &args,
            &mut std::io::stdout(),
            &mut std::io::stderr(),
        ));
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            app_info,
            configured_printers,
            registered_drivers,
            remove_configured_printer
        ])
        .run(tauri::generate_context!())
        .expect("error while running Polimero");
}
