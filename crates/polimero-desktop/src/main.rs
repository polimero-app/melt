use polimero_core::{
    AppInfo,
    config::{Config, ConfigError, Profile, config_dir},
    diagnostics,
    drivers::{self, Capabilities, DriverError, Operation},
    keychain::{SERVICE, SecretError, SecretStore, SystemKeychain, account},
    moonraker, profiles,
};
use serde::{Deserialize, Serialize};

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

struct DesktopPrinter {
    driver: drivers::Profile,
    access_code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrinterCapabilities {
    name: String,
    capabilities: Capabilities,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorEntry {
    name: String,
    driver: String,
    status: Option<moonraker::Status>,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobActionRequest {
    name: String,
    action: String,
    #[serde(default)]
    device_path: Option<String>,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemperatureRequest {
    name: String,
    nozzle_celsius: Option<f64>,
    bed_celsius: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FanRequest {
    name: String,
    fan: String,
    speed_percent: u8,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MotionHomeRequest {
    name: String,
    confirmed: bool,
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
fn printer_capabilities(name: String) -> Result<PrinterCapabilities, String> {
    let config = Config::load().map_err(|_| "Unable to read printer configuration.".to_string())?;
    let profile = config
        .get_profile(&name.to_ascii_lowercase())
        .ok_or_else(|| "Printer profile not found.".to_string())?;
    let driver = drivers::profile(profile).map_err(|_| "Invalid printer profile.".to_string())?;
    Ok(PrinterCapabilities {
        name,
        capabilities: driver.driver().capabilities(),
    })
}

#[tauri::command]
fn monitored_printers() -> Result<Vec<MonitorEntry>, String> {
    let config = Config::load().map_err(|_| "Unable to read printer configuration.".to_string())?;
    Ok(config
        .sorted_profiles()
        .into_iter()
        .map(|named| monitor_printer(named.name, named.profile))
        .collect())
}

fn monitor_printer(name: String, profile: Profile) -> MonitorEntry {
    let driver = profile.driver.clone();
    let driver_profile = match drivers::profile(&profile) {
        Ok(profile) => profile,
        Err(_) => {
            return MonitorEntry {
                name,
                driver,
                status: None,
                error: Some("Invalid printer profile.".into()),
            };
        }
    };
    let kind = driver_profile.driver();
    if !kind.supports(Operation::Status) {
        return MonitorEntry {
            name,
            driver,
            status: None,
            error: Some("Status monitoring is unavailable for this driver.".into()),
        };
    }
    let access_code = match access_code(&profile.driver, &name, kind) {
        Ok(access_code) => access_code,
        Err(error) => {
            return MonitorEntry {
                name,
                driver,
                status: None,
                error: Some(error),
            };
        }
    };
    match drivers::status(&driver_profile, access_code.as_deref()) {
        Ok(status) => MonitorEntry {
            name,
            driver,
            status: Some(status),
            error: None,
        },
        Err(error) => MonitorEntry {
            name,
            driver,
            status: None,
            error: Some(operation_error(error, "status monitoring")),
        },
    }
}

#[tauri::command]
fn create_configured_printer(
    request: profiles::CreateRequest,
) -> Result<profiles::CreateResult, String> {
    let dir = config_dir().map_err(|_| "Unable to read printer configuration.".to_string())?;
    profiles::create(dir, &SystemKeychain, request).map_err(create_error)
}

fn create_error(error: profiles::ProfileError) -> String {
    match error {
        profiles::ProfileError::MissingName
        | profiles::ProfileError::InvalidName
        | profiles::ProfileError::InvalidHost
        | profiles::ProfileError::InvalidAccessCode => error.to_string(),
        profiles::ProfileError::Config(ConfigError::ProfileAlreadyExists) => {
            "A printer profile with this name already exists.".into()
        }
        profiles::ProfileError::Driver(DriverError::UnsupportedOperation(
            _,
            drivers::Operation::Verify,
        )) => "This driver cannot verify printer profiles yet.".into(),
        profiles::ProfileError::Driver(error) => operation_error(error, "profile verification"),
        profiles::ProfileError::Secret(_) | profiles::ProfileError::RollbackFailed => {
            "Keychain operation failed.".into()
        }
        profiles::ProfileError::Config(_) => "Unable to save printer configuration.".into(),
        profiles::ProfileError::NotFound(_) => "Printer profile not found.".into(),
    }
}

#[tauri::command]
fn printer_status(name: String) -> Result<polimero_core::moonraker::Status, String> {
    let printer = desktop_printer(&name, Operation::Status)?;
    drivers::status(&printer.driver, printer.access_code.as_deref())
        .map_err(|error| operation_error(error, "status"))
}

#[tauri::command]
fn printer_files(name: String) -> Result<moonraker::FileList, String> {
    let printer = desktop_printer(&name, Operation::FileList)?;
    drivers::file_list(&printer.driver, printer.access_code.as_deref(), "/", false)
        .map_err(|error| operation_error(error, "file listing"))
}

#[tauri::command]
fn printer_job_action(request: JobActionRequest) -> Result<moonraker::JobResult, String> {
    if !request.confirmed {
        return Err("Confirm this printer action before sending it.".into());
    }
    let operation = match request.action.as_str() {
        "start" => Operation::JobStart,
        "pause" => Operation::JobPause,
        "resume" => Operation::JobResume,
        "cancel" => Operation::JobCancel,
        _ => return Err("Unsupported printer action.".into()),
    };
    let printer = desktop_printer(&request.name, operation)?;
    let allowed = match request.action.as_str() {
        "start" => &[moonraker::PrinterState::Idle][..],
        "pause" => &[moonraker::PrinterState::Printing][..],
        "resume" => &[moonraker::PrinterState::Paused][..],
        "cancel" => &[
            moonraker::PrinterState::Printing,
            moonraker::PrinterState::Paused,
        ][..],
        _ => unreachable!("action is validated above"),
    };
    ensure_state(&printer, allowed, &request.action)?;
    match request.action.as_str() {
        "start" => {
            let device_path = request
                .device_path
                .as_deref()
                .filter(|path| !path.is_empty())
                .ok_or_else(|| "Choose a printer file before starting a job.".to_string())?;
            drivers::job_start(&printer.driver, printer.access_code.as_deref(), device_path)
        }
        "pause" => drivers::job_pause(&printer.driver, printer.access_code.as_deref()),
        "resume" => drivers::job_resume(&printer.driver, printer.access_code.as_deref()),
        "cancel" => drivers::job_cancel(&printer.driver, printer.access_code.as_deref()),
        _ => unreachable!("action is validated above"),
    }
    .map_err(|error| operation_error(error, &request.action))
}

#[tauri::command]
fn printer_temperature_set(
    request: TemperatureRequest,
) -> Result<moonraker::TemperatureResult, String> {
    let targets = moonraker::TemperatureTargets {
        nozzle_celsius: request.nozzle_celsius,
        bed_celsius: request.bed_celsius,
        chamber_celsius: None,
    };
    if targets.nozzle_celsius.is_none() && targets.bed_celsius.is_none() {
        return Err("Choose a temperature target first.".into());
    }
    let printer = desktop_printer(&request.name, Operation::TemperatureSet)?;
    ensure_state(
        &printer,
        &[moonraker::PrinterState::Idle],
        "set temperature",
    )?;
    drivers::temperature_set(&printer.driver, printer.access_code.as_deref(), targets)
        .map_err(|error| operation_error(error, "set temperature"))
}

#[tauri::command]
fn printer_fan_set(request: FanRequest) -> Result<moonraker::FanResult, String> {
    if !request.confirmed {
        return Err("Confirm this printer action before sending it.".into());
    }
    let printer = desktop_printer(&request.name, Operation::FanSet)?;
    ensure_state(
        &printer,
        &[
            moonraker::PrinterState::Idle,
            moonraker::PrinterState::Printing,
            moonraker::PrinterState::Paused,
            moonraker::PrinterState::Error,
        ],
        "set fan speed",
    )?;
    drivers::fan_set(
        &printer.driver,
        printer.access_code.as_deref(),
        &request.fan,
        request.speed_percent,
    )
    .map_err(|error| operation_error(error, "set fan speed"))
}

#[tauri::command]
fn printer_motion_home(request: MotionHomeRequest) -> Result<moonraker::MotionResult, String> {
    if !request.confirmed {
        return Err("Confirm this printer action before sending it.".into());
    }
    let printer = desktop_printer(&request.name, Operation::MotionHome)?;
    ensure_state(&printer, &[moonraker::PrinterState::Idle], "home axes")?;
    drivers::motion_home(
        &printer.driver,
        printer.access_code.as_deref(),
        &[moonraker::Axis::X, moonraker::Axis::Y, moonraker::Axis::Z],
    )
    .map_err(|error| operation_error(error, "home axes"))
}

#[tauri::command]
fn printer_emergency_stop(name: String) -> Result<(), String> {
    let printer = desktop_printer(&name, Operation::EmergencyStop)?;
    drivers::emergency_stop(&printer.driver, printer.access_code.as_deref())
        .map_err(|error| operation_error(error, "emergency stop"))
}

#[tauri::command]
fn diagnostics_report() -> Result<diagnostics::Report, String> {
    Config::load()
        .map(|config| {
            diagnostics::report(
                config
                    .sorted_profiles()
                    .into_iter()
                    .map(|profile| profile.profile),
            )
        })
        .map_err(|_| "Unable to read printer configuration.".into())
}

fn desktop_printer(name: &str, operation: Operation) -> Result<DesktopPrinter, String> {
    let name = name.to_ascii_lowercase();
    let config = Config::load().map_err(|_| "Unable to read printer configuration.".to_string())?;
    let profile = config
        .get_profile(&name)
        .cloned()
        .ok_or_else(|| "Printer profile not found.".to_string())?;
    let driver = drivers::profile(&profile).map_err(|_| "Invalid printer profile.".to_string())?;
    let driver_kind = driver.driver();
    if !driver_kind.supports(operation) {
        return Err(format!(
            "This driver does not support {}.",
            operation_name(operation)
        ));
    }
    let access_code = access_code(&profile.driver, &name, driver_kind)?;
    Ok(DesktopPrinter {
        driver,
        access_code,
    })
}

fn access_code(
    driver: &str,
    name: &str,
    driver_kind: drivers::Driver,
) -> Result<Option<String>, String> {
    match SystemKeychain.get(SERVICE, &account(driver, name, "access-code")) {
        Ok(access_code) => Ok(Some(access_code)),
        Err(SecretError::NotFound) => Ok(None),
        Err(SecretError::Unavailable(_)) if driver_kind == drivers::Driver::Moonraker => Ok(None),
        Err(SecretError::Unavailable(_)) => Err("Keychain operation failed.".into()),
    }
}

fn ensure_state(
    printer: &DesktopPrinter,
    allowed: &[moonraker::PrinterState],
    action: &str,
) -> Result<(), String> {
    let status = drivers::status(&printer.driver, printer.access_code.as_deref())
        .map_err(|error| operation_error(error, action))?;
    if allowed.contains(&status.state) {
        return Ok(());
    }
    Err(format!(
        "The printer is {:?}; it cannot {} now.",
        status.state, action
    ))
}

fn operation_name(operation: Operation) -> &'static str {
    match operation {
        Operation::Status => "status",
        Operation::FileList => "file listing",
        Operation::JobPause => "pausing jobs",
        Operation::JobResume => "resuming jobs",
        Operation::JobCancel => "cancelling jobs",
        Operation::JobStart => "starting jobs",
        Operation::EmergencyStop => "emergency stop",
        Operation::TemperatureSet => "temperature control",
        Operation::FanSet => "fan control",
        Operation::MotionHome => "motion control",
        _ => "this operation",
    }
}

fn operation_error(error: DriverError, operation: &str) -> String {
    match error {
        DriverError::Unknown(_) | DriverError::InvalidProfile(_) | DriverError::InvalidTimeout => {
            "Invalid printer profile.".into()
        }
        DriverError::UnsupportedOperation(_, _) => {
            format!("This driver does not support {operation}.")
        }
        DriverError::Moonraker(polimero_core::moonraker::Error::Authentication) => {
            "Printer authentication failed.".into()
        }
        DriverError::Moonraker(polimero_core::moonraker::Error::Timeout) => {
            format!("Printer {operation} timed out.")
        }
        DriverError::Moonraker(polimero_core::moonraker::Error::Unsupported(_)) => {
            format!("This driver does not support {operation}.")
        }
        DriverError::Moonraker(_) => format!("Printer {operation} failed."),
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
            printer_capabilities,
            monitored_printers,
            create_configured_printer,
            printer_status,
            printer_files,
            printer_job_action,
            printer_temperature_set,
            printer_fan_set,
            printer_motion_home,
            printer_emergency_stop,
            diagnostics_report,
            remove_configured_printer
        ])
        .run(tauri::generate_context!())
        .expect("error while running Polimero");
}
