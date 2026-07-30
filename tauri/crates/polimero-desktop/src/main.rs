use polimero_core::{AppInfo, config::Config};
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
        .invoke_handler(tauri::generate_handler![app_info, configured_printers])
        .run(tauri::generate_context!())
        .expect("error while running Polimero");
}
