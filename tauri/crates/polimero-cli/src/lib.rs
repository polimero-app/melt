//! CLI rendering for shared Polimero operations.

use std::{collections::BTreeMap, io::Write};

use polimero_core::{
    AppError, app_info,
    config::{Config, ConfigError, NamedProfile},
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
        _ => write_error(
            &command,
            invocation.format,
            AppError::usage(format!("unknown command {command:?}")),
            out,
            err,
        ),
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
}
