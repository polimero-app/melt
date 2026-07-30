use polimero_core::{
    AppInfo,
    config::{Config, config_dir},
    drivers::{self, DriverError},
    keychain::{SERVICE, SecretError, SecretStore, SystemKeychain, account},
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
fn printer_status(name: String) -> Result<polimero_core::moonraker::Status, String> {
    let config = Config::load().map_err(|_| "Unable to read printer configuration.".to_string())?;
    let profile = config
        .get_profile(&name)
        .ok_or_else(|| "Printer profile not found.".to_string())?;
    let driver_profile = drivers::profile(profile).map_err(status_error)?;
    if !driver_profile.driver().supports(drivers::Operation::Status) {
        return Err(status_error(DriverError::UnsupportedOperation(
            driver_profile.driver(),
            drivers::Operation::Status,
        )));
    }
    let access_code =
        match SystemKeychain.get(SERVICE, &account(&profile.driver, &name, "access-code")) {
            Ok(access_code) => Some(access_code),
            Err(SecretError::NotFound) => None,
            Err(SecretError::Unavailable(_)) => return Err("Keychain operation failed.".into()),
        };
    drivers::status(&driver_profile, access_code.as_deref()).map_err(status_error)
}

fn status_error(error: DriverError) -> String {
    match error {
        DriverError::Unknown(_) | DriverError::InvalidProfile(_) | DriverError::InvalidTimeout => {
            "Invalid printer profile.".into()
        }
        DriverError::UnsupportedOperation(_, _) => "This driver does not support status.".into(),
        DriverError::Moonraker(polimero_core::moonraker::Error::Authentication) => {
            "Printer authentication failed.".into()
        }
        DriverError::Moonraker(_) => "Printer status request failed.".into(),
    }
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
            printer_status,
            remove_configured_printer
        ])
        .run(tauri::generate_context!())
        .expect("error while running Polimero");
}
