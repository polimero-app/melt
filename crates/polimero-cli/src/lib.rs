//! CLI rendering for shared Polimero operations.

use std::{
    collections::BTreeMap,
    io::{IsTerminal, Write},
};

use polimero_core::{
    AppError, app_info,
    config::{Config, ConfigError, NamedProfile, config_dir},
    drivers::{self, DriverError},
    keychain::{SERVICE, SecretError, SecretStore, SystemKeychain, account},
    profiles::{self, ProfileError},
};
use serde::Serialize;

#[derive(Clone, Copy)]
enum OutputFormat {
    Human,
    Json,
}

struct Invocation<'a> {
    command: Vec<&'a String>,
    format: OutputFormat,
}

#[derive(Serialize)]
struct Meta<'a> {
    command: &'a str,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
    details: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct Envelope<'a, T: Serialize> {
    ok: bool,
    data: Option<T>,
    error: Option<ErrorDetail<'a>>,
    meta: Meta<'a>,
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let invocation = match Invocation::parse(args) {
        Ok(invocation) => invocation,
        Err(error) => return write_error("polimero", OutputFormat::Human, error, out, err),
    };
    let command = command_name(&invocation.command);

    match invocation.command.as_slice() {
        [version] if version.as_str() == "version" => {
            let info = app_info();
            write_success(
                "version",
                invocation.format,
                info,
                |out| writeln!(out, "polimero version {}", app_info().version),
                out,
            )
        }
        [printer, list] if printer.as_str() == "printer" && list.as_str() == "list" => {
            match Config::load() {
                Ok(config) => write_printer_list(invocation.format, config.sorted_profiles(), out),
                Err(error) => write_error(
                    "printer list",
                    invocation.format,
                    config_error(error),
                    out,
                    err,
                ),
            }
        }
        [printer, drivers] if printer.as_str() == "printer" && drivers.as_str() == "drivers" => {
            write_drivers(invocation.format, out)
        }
        [printer, add, name, flags @ ..]
            if printer.as_str() == "printer" && add.as_str() == "add" =>
        {
            add_profile(invocation.format, name, flags, out, err)
        }
        [status, name] if status.as_str() == "status" => {
            printer_status(invocation.format, name, out, err)
        }
        [printer, remove, name] if printer.as_str() == "printer" && remove.as_str() == "remove" => {
            remove_profile(invocation.format, name, false, out, err)
        }
        [printer, remove, name, yes]
            if printer.as_str() == "printer"
                && remove.as_str() == "remove"
                && yes.as_str() == "--yes" =>
        {
            remove_profile(invocation.format, name, true, out, err)
        }
        _ => write_error(
            &command,
            invocation.format,
            AppError::usage(format!("unknown command {command:?}")),
            out,
            err,
        ),
    }
}

fn add_profile(
    format: OutputFormat,
    name: &str,
    flags: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let request = match parse_add_request(name, flags) {
        Ok(request) => request,
        Err(error) => return write_error("printer add", format, AppError::usage(error), out, err),
    };
    let dir = match config_dir() {
        Ok(dir) => dir,
        Err(error) => {
            return write_error(
                "printer add",
                format,
                AppError {
                    exit_code: 1,
                    code: "internal-error",
                    message: error.to_string(),
                },
                out,
                err,
            );
        }
    };
    match profiles::create(dir, &SystemKeychain, request) {
        Ok(profile) => {
            let name = profile.name.clone();
            write_success(
                "printer add",
                format,
                AddData { profile },
                |out| writeln!(out, "Printer profile added: {name}"),
                out,
            )
        }
        Err(error) => write_error("printer add", format, profile_error(error), out, err),
    }
}

fn parse_add_request(name: &str, flags: &[&String]) -> Result<profiles::CreateRequest, String> {
    let mut request = profiles::CreateRequest {
        name: name.into(),
        driver: String::new(),
        host: String::new(),
        serial: String::new(),
        timeout: "10s".into(),
        insecure: false,
        access_code: String::new(),
    };
    let mut index = 0;
    while index < flags.len() {
        let flag = flags[index].as_str();
        match flag {
            "--insecure" => request.insecure = true,
            "--driver" | "--host" | "--serial" | "--timeout" | "--access-code-file" => {
                let value = flags
                    .get(index + 1)
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                match flag {
                    "--driver" => request.driver = (*value).clone(),
                    "--host" => request.host = (*value).clone(),
                    "--serial" => request.serial = (*value).clone(),
                    "--timeout" => request.timeout = (*value).clone(),
                    "--access-code-file" => {
                        request.access_code = read_access_code_file(value)?;
                    }
                    _ => unreachable!("recognized flags are exhaustive"),
                }
                index += 1;
            }
            _ => return Err(format!("unknown printer add option {flag:?}")),
        }
        index += 1;
    }
    if request.driver.is_empty() {
        return Err("--driver is required".into());
    }
    if request.host.is_empty() {
        return Err("--host is required".into());
    }
    Ok(request)
}

fn read_access_code_file(path: &str) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot read --access-code-file: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4096 {
        return Err("--access-code-file must be a regular file no larger than 4 KiB".into());
    }
    #[cfg(unix)]
    if std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o077 != 0 {
        return Err("--access-code-file must not grant group or other access".into());
    }
    let mut code = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read --access-code-file: {error}"))?;
    if let Some(stripped) = code
        .strip_suffix("\r\n")
        .or_else(|| code.strip_suffix('\n'))
    {
        code = stripped.into();
    }
    Ok(code)
}

fn printer_status(
    format: OutputFormat,
    name: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => return write_error("status", format, config_error(error), out, err),
    };
    let profile = match config.get_profile(name) {
        Some(profile) => profile,
        None => {
            return write_error(
                "status",
                format,
                AppError::usage(format!("printer profile {name:?} not found")),
                out,
                err,
            );
        }
    };
    let driver_profile = match drivers::profile(profile) {
        Ok(profile) => profile,
        Err(error) => return write_error("status", format, driver_error(error), out, err),
    };
    if !driver_profile.driver().supports(drivers::Operation::Status) {
        return write_error(
            "status",
            format,
            driver_error(DriverError::UnsupportedOperation(
                driver_profile.driver(),
                drivers::Operation::Status,
            )),
            out,
            err,
        );
    }
    let access_code =
        match SystemKeychain.get(SERVICE, &account(&profile.driver, name, "access-code")) {
            Ok(access_code) => Some(access_code),
            Err(SecretError::NotFound) => None,
            Err(SecretError::Unavailable(_)) => {
                return write_error(
                    "status",
                    format,
                    AppError {
                        exit_code: 3,
                        code: "secret-store-failed",
                        message: "keychain operation failed".into(),
                    },
                    out,
                    err,
                );
            }
        };
    match drivers::status(&driver_profile, access_code.as_deref()) {
        Ok(status) => {
            let state = status.state;
            write_success(
                "status",
                format,
                status,
                |out| writeln!(out, "STATE\t{state:?}"),
                out,
            )
        }
        Err(error) => write_error("status", format, driver_error(error), out, err),
    }
}

fn driver_error(error: DriverError) -> AppError {
    match error {
        DriverError::Unknown(_) | DriverError::InvalidProfile(_) | DriverError::InvalidTimeout => {
            AppError::usage(error.to_string())
        }
        DriverError::UnsupportedOperation(_, _) => AppError {
            exit_code: 2,
            code: "unsupported-operation",
            message: error.to_string(),
        },
        DriverError::Moonraker(polimero_core::moonraker::Error::Authentication) => AppError {
            exit_code: 3,
            code: "authentication-failed",
            message: "printer authentication failed".into(),
        },
        DriverError::Moonraker(_) => AppError {
            exit_code: 1,
            code: "printer-unavailable",
            message: "printer status request failed".into(),
        },
    }
}

fn remove_profile(
    format: OutputFormat,
    name: &str,
    yes: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if !yes && !std::io::stdin().is_terminal() {
        return write_error(
            "printer remove",
            format,
            AppError::usage("non-interactive mode requires --yes"),
            out,
            err,
        );
    }
    if !yes {
        let _ = write!(
            err,
            "Remove printer profile {name} and its stored secrets? Type 'yes' to continue: "
        );
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            return write_error(
                "printer remove",
                format,
                AppError {
                    exit_code: 1,
                    code: "internal-error",
                    message: "cannot read confirmation".into(),
                },
                out,
                err,
            );
        }
        if answer.trim_end() != "yes" {
            return write_error(
                "printer remove",
                format,
                AppError::usage("confirmation declined; profile not removed"),
                out,
                err,
            );
        }
    }

    let dir = match polimero_core::config::config_dir() {
        Ok(dir) => dir,
        Err(error) => {
            return write_error(
                "printer remove",
                format,
                AppError {
                    exit_code: 1,
                    code: "internal-error",
                    message: error.to_string(),
                },
                out,
                err,
            );
        }
    };
    match profiles::remove(dir, &SystemKeychain, name) {
        Ok(result) => write_success(
            "printer remove",
            format,
            RemoveData { removed: result },
            |out| writeln!(out, "Printer profile removed: {name}"),
            out,
        ),
        Err(error) => write_error("printer remove", format, profile_error(error), out, err),
    }
}

#[derive(Serialize)]
struct RemoveData {
    removed: profiles::RemoveResult,
}

#[derive(Serialize)]
struct AddData {
    profile: profiles::CreateResult,
}

fn profile_error(error: ProfileError) -> AppError {
    match error {
        ProfileError::MissingName
        | ProfileError::InvalidName
        | ProfileError::InvalidHost
        | ProfileError::InvalidAccessCode
        | ProfileError::NotFound(_) => AppError::usage(error.to_string()),
        ProfileError::Driver(error) => driver_error(error),
        ProfileError::Secret(_) => AppError {
            exit_code: 3,
            code: "secret-store-failed",
            message: "keychain operation failed".into(),
        },
        ProfileError::Config(ConfigError::ProfileAlreadyExists) => {
            AppError::usage("profile already exists")
        }
        ProfileError::Config(_) | ProfileError::RollbackFailed => AppError {
            exit_code: 1,
            code: "internal-error",
            message: error.to_string(),
        },
    }
}

fn write_drivers(format: OutputFormat, out: &mut dyn Write) -> i32 {
    let drivers = drivers::registered();
    match format {
        OutputFormat::Json => write_success(
            "printer drivers",
            format,
            DriverList {
                drivers: drivers.to_vec(),
            },
            |_| Ok(()),
            out,
        ),
        OutputFormat::Human => {
            let _ = writeln!(out, "DRIVER\tDESCRIPTION");
            for driver in drivers {
                if writeln!(out, "{}\t{}", driver.name, driver.description).is_err() {
                    return 1;
                }
            }
            0
        }
    }
}

impl<'a> Invocation<'a> {
    fn parse(args: &'a [String]) -> Result<Self, AppError> {
        let mut command = Vec::new();
        let mut format = OutputFormat::Human;
        let mut index = 0;
        while index < args.len() {
            if args[index] == "--output" {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| AppError::usage("--output requires a value"))?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(AppError::usage(format!(
                            "invalid output format {value:?}: must be human or json"
                        )));
                    }
                };
                index += 2;
                continue;
            }
            command.push(&args[index]);
            index += 1;
        }
        if command.is_empty() {
            return Err(AppError::usage("a command is required"));
        }
        Ok(Self { command, format })
    }
}

fn command_name(command: &[&String]) -> String {
    command
        .iter()
        .map(|part| part.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_success<T: Serialize>(
    command: &str,
    format: OutputFormat,
    data: T,
    human: impl FnOnce(&mut dyn Write) -> std::io::Result<()>,
    out: &mut dyn Write,
) -> i32 {
    match format {
        OutputFormat::Human => human(out).map_or(1, |_| 0),
        OutputFormat::Json => {
            let envelope = Envelope {
                ok: true,
                data: Some(data),
                error: None,
                meta: Meta { command },
            };
            write_json(out, &envelope).map_or(1, |_| 0)
        }
    }
}

#[derive(Serialize)]
struct PrinterProfile<'a> {
    name: &'a str,
    driver: &'a str,
    host: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    serial: &'a str,
    timeout: &'a str,
    insecure: bool,
}

#[derive(Serialize)]
struct PrinterList<'a> {
    profiles: Vec<PrinterProfile<'a>>,
}

#[derive(Serialize)]
struct DriverList {
    drivers: Vec<drivers::DriverInfo>,
}

fn write_printer_list(
    format: OutputFormat,
    profiles: Vec<NamedProfile>,
    out: &mut dyn Write,
) -> i32 {
    let data = profiles
        .iter()
        .map(|named| PrinterProfile {
            name: &named.name,
            driver: &named.profile.driver,
            host: &named.profile.host,
            serial: &named.profile.serial,
            timeout: &named.profile.timeout,
            insecure: named.profile.insecure,
        })
        .collect::<Vec<_>>();
    match format {
        OutputFormat::Json => write_success(
            "printer list",
            format,
            PrinterList { profiles: data },
            |_| Ok(()),
            out,
        ),
        OutputFormat::Human if data.is_empty() => {
            writeln!(out, "No printer profiles configured.").map_or(1, |_| 0)
        }
        OutputFormat::Human => {
            let _ = writeln!(out, "NAME\tDRIVER\tHOST\tSERIAL\tTIMEOUT\tINSECURE");
            for profile in data {
                if writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    profile.name,
                    profile.driver,
                    profile.host,
                    profile.serial,
                    profile.timeout,
                    profile.insecure
                )
                .is_err()
                {
                    return 1;
                }
            }
            0
        }
    }
}

fn config_error(error: ConfigError) -> AppError {
    match error {
        ConfigError::UnsupportedVersion { .. } => {
            AppError::usage("unsupported config schema version")
        }
        ConfigError::Malformed(_) => AppError::usage("config file is malformed"),
        _ => AppError {
            exit_code: 1,
            code: "internal-error",
            message: "failed to read config".into(),
        },
    }
}

fn write_error(
    command: &str,
    format: OutputFormat,
    error: AppError,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if matches!(format, OutputFormat::Json) {
        let envelope: Envelope<()> = Envelope {
            ok: false,
            data: None,
            error: Some(ErrorDetail {
                code: error.code,
                message: &error.message,
                details: BTreeMap::new(),
            }),
            meta: Meta { command },
        };
        let _ = write_json(out, &envelope);
    } else {
        let _ = writeln!(err, "Error: {}", error.message);
    }
    error.exit_code
}

fn write_json<T: Serialize>(out: &mut dyn Write, value: &T) -> serde_json::Result<()> {
    serde_json::to_writer_pretty(&mut *out, value)?;
    writeln!(out).map_err(serde_json::Error::io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use polimero_core::config::Profile;

    #[test]
    fn version_json_is_an_envelope() {
        let args = ["version".into(), "--output".into(), "json".into()];
        let mut out = Vec::new();
        assert_eq!(run(&args, &mut out, &mut Vec::new()), 0);
        assert!(String::from_utf8(out).unwrap().contains(r#""ok": true"#));
    }

    #[test]
    fn invalid_command_uses_the_json_error_envelope() {
        let args = ["unknown".into(), "--output".into(), "json".into()];
        let mut out = Vec::new();
        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains(r#""code": "config-error""#)
        );
    }

    #[test]
    fn printer_list_keeps_the_existing_json_data_shape() {
        let profiles = vec![NamedProfile {
            name: "garage".into(),
            profile: Profile {
                driver: "moonraker".into(),
                host: "printer.local".into(),
                serial: String::new(),
                timeout: "10s".into(),
                insecure: false,
                created: String::new(),
                updated: String::new(),
            },
        }];
        let mut out = Vec::new();

        assert_eq!(
            write_printer_list(OutputFormat::Json, profiles, &mut out),
            0
        );

        let output = String::from_utf8(out).unwrap();
        assert!(output.contains(r#""command": "printer list""#));
        assert!(output.contains(r#""profiles": ["#));
        assert!(output.contains(r#""name": "garage""#));
        assert!(!output.contains(r#""serial""#));
    }

    #[test]
    fn printer_drivers_keeps_the_existing_json_data_shape() {
        let mut out = Vec::new();

        assert_eq!(write_drivers(OutputFormat::Json, &mut out), 0);

        let output = String::from_utf8(out).unwrap();
        assert!(output.contains(r#""command": "printer drivers""#));
        assert!(output.contains(r#""drivers": ["#));
        assert!(output.contains("bambu-lan"));
    }

    #[test]
    fn unsupported_driver_status_has_a_stable_error_code() {
        let error = driver_error(DriverError::UnsupportedOperation(
            drivers::Driver::BambuLan,
            drivers::Operation::Status,
        ));

        assert_eq!(error.exit_code, 2);
        assert_eq!(error.code, "unsupported-operation");
    }

    #[test]
    fn printer_add_parses_the_existing_profile_flags() {
        let raw = [
            "--driver".into(),
            "moonraker".into(),
            "--host".into(),
            "printer.local".into(),
            "--serial".into(),
            "unused".into(),
            "--timeout".into(),
            "5s".into(),
            "--insecure".into(),
        ];
        let flags = raw.iter().collect::<Vec<_>>();

        let request = parse_add_request("Garage", &flags).unwrap();

        assert_eq!(request.name, "Garage");
        assert_eq!(request.driver, "moonraker");
        assert_eq!(request.host, "printer.local");
        assert_eq!(request.timeout, "5s");
        assert!(request.insecure);
    }

    #[test]
    fn profile_removal_requires_yes_when_not_interactive() {
        let args = [
            "printer".into(),
            "remove".into(),
            "garage".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("non-interactive mode requires --yes")
        );
    }
}
