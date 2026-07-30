//! CLI rendering for shared Polimero operations.

use std::io::Write;

use polimero_core::{AppError, app_info};
use serde::Serialize;

#[derive(Serialize)]
struct Meta<'a> {
    command: &'a str,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
}

#[derive(Serialize)]
struct Envelope<'a, T: Serialize> {
    ok: bool,
    data: Option<T>,
    error: Option<ErrorDetail<'a>>,
    meta: Meta<'a>,
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match parse(args) {
        Ok((command, json)) => {
            let info = app_info();
            if json {
                let envelope = Envelope {
                    ok: true,
                    data: Some(info),
                    error: None,
                    meta: Meta { command },
                };
                let _ = writeln!(
                    out,
                    "{}",
                    serde_json::to_string(&envelope).expect("serializable envelope")
                );
            } else {
                let _ = writeln!(out, "polimero version {}", info.version);
            }
            0
        }
        Err(error) => write_error(args, error, out, err),
    }
}

fn parse(args: &[String]) -> Result<(&str, bool), AppError> {
    let (command, flags) = args
        .split_first()
        .ok_or_else(|| AppError::usage("a command is required"))?;
    if command != "version" {
        return Err(AppError::usage(format!("unknown command {command:?}")));
    }

    match flags {
        [] => Ok(("version", false)),
        [flag, value] if flag == "--output" && value == "json" => Ok(("version", true)),
        [flag, value] if flag == "--output" => {
            Err(AppError::usage(format!("invalid output format {value:?}")))
        }
        _ => Err(AppError::usage("version accepts only --output json")),
    }
}

fn write_error(args: &[String], error: AppError, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let json = args
        .windows(2)
        .any(|pair| pair[0] == "--output" && pair[1] == "json");
    if json {
        let envelope: Envelope<()> = Envelope {
            ok: false,
            data: None,
            error: Some(ErrorDetail {
                code: error.code,
                message: &error.message,
            }),
            meta: Meta {
                command: args.first().map_or("polimero", String::as_str),
            },
        };
        let _ = writeln!(
            out,
            "{}",
            serde_json::to_string(&envelope).expect("serializable envelope")
        );
    } else {
        let _ = writeln!(err, "Error: {}", error.message);
    }
    error.exit_code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_json_is_an_envelope() {
        let args = ["version".into(), "--output".into(), "json".into()];
        let mut out = Vec::new();
        assert_eq!(run(&args, &mut out, &mut Vec::new()), 0);
        assert!(String::from_utf8(out).unwrap().contains(r#""ok":true"#));
    }

    #[test]
    fn invalid_command_uses_the_json_error_envelope() {
        let args = ["unknown".into(), "--output".into(), "json".into()];
        let mut out = Vec::new();
        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains(r#""code":"config-error""#)
        );
    }
}
