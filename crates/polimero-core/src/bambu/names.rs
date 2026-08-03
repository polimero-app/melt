//! Naming rules shared by uploads, print commands, and user-facing job data.

use std::path::Path;

use serde::Serialize;
use thiserror::Error;

const MAX_NAME_BYTES: usize = 99;
const ILLEGAL_REMOTE: &[char] = &['<', '>', '[', ']', ':', '/', '\\', '|', '?', '*', '"'];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobNames {
    pub source_filename: String,
    pub remote_filename: String,
    pub display_name: String,
    pub project_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plate_name: Option<String>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum NameError {
    #[error("file name is empty")]
    Empty,
    #[error("file name contains unsupported characters")]
    InvalidCharacters,
    #[error("file name may not start or end with whitespace")]
    SurroundingWhitespace,
    #[error("file name exceeds the 99-byte printer limit")]
    TooLong,
}

/// Derives stable wire and display identities without mutating the source name.
pub fn derive_job_names(
    source: &Path,
    requested_display: Option<&str>,
    plate_name: Option<&str>,
) -> JobNames {
    let source_filename = source
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("print.gcode.3mf")
        .to_owned();
    let stem = print_stem(&source_filename);
    let display_name = normalize_display(requested_display.unwrap_or(stem));
    let project_name = normalize_display(stem);
    let extension = print_extension(&source_filename);
    let remote_stem = sanitize_remote_stem(&display_name);
    let remote_filename = fit_remote_name(&remote_stem, extension);
    JobNames {
        source_filename,
        remote_filename,
        display_name,
        project_name,
        plate_name: plate_name
            .map(normalize_display)
            .filter(|name| !name.is_empty()),
    }
}

pub fn validate_remote_filename(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.trim() != name {
        return Err(NameError::SurroundingWhitespace);
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(NameError::TooLong);
    }
    if name
        .chars()
        .any(|character| character.is_control() || ILLEGAL_REMOTE.contains(&character))
    {
        return Err(NameError::InvalidCharacters);
    }
    Ok(())
}

fn print_stem(filename: &str) -> &str {
    filename
        .strip_suffix(".gcode.3mf")
        .or_else(|| filename.strip_suffix(".3mf"))
        .or_else(|| filename.strip_suffix(".gcode"))
        .unwrap_or(filename)
}

fn print_extension(filename: &str) -> &str {
    if filename.ends_with(".gcode.3mf") {
        ".gcode.3mf"
    } else if filename.ends_with(".3mf") {
        ".3mf"
    } else if filename.ends_with(".gcode") {
        ".gcode"
    } else {
        ".gcode.3mf"
    }
}

fn normalize_display(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        "print".into()
    } else {
        truncate_utf8(&normalized, MAX_NAME_BYTES)
    }
}

fn sanitize_remote_stem(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for character in value.chars() {
        let replace = character.is_whitespace()
            || character.is_control()
            || ILLEGAL_REMOTE.contains(&character);
        if replace {
            separator = !output.is_empty();
        } else {
            if separator && !output.ends_with('_') {
                output.push('_');
            }
            separator = false;
            output.push(character);
        }
    }
    let output = output.trim_matches(['_', '.', ' ']);
    if output.is_empty() {
        "print".into()
    } else {
        output.into()
    }
}

fn fit_remote_name(stem: &str, extension: &str) -> String {
    let stem_limit = MAX_NAME_BYTES.saturating_sub(extension.len());
    let truncated = truncate_utf8(stem, stem_limit);
    let stem = truncated.trim_end_matches(['_', '.', ' ']);
    format!(
        "{}{extension}",
        if stem.is_empty() { "print" } else { stem }
    )
}

fn truncate_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_distinct_display_and_remote_names() {
        let names = derive_job_names(
            Path::new("/tmp/original.gcode.3mf"),
            Some("  Customer's   bracket: red  "),
            Some("Front plate"),
        );
        assert_eq!(names.source_filename, "original.gcode.3mf");
        assert_eq!(names.display_name, "Customer's bracket: red");
        assert_eq!(names.remote_filename, "Customer's_bracket_red.gcode.3mf");
        assert_eq!(names.project_name, "original");
        assert_eq!(names.plate_name.as_deref(), Some("Front plate"));
    }

    #[test]
    fn truncates_unicode_on_a_character_boundary_with_extension_preserved() {
        let names = derive_job_names(Path::new("model.3mf"), Some(&"é".repeat(100)), None);
        assert!(names.remote_filename.len() <= MAX_NAME_BYTES);
        assert!(names.remote_filename.ends_with(".3mf"));
        assert!(
            names
                .remote_filename
                .is_char_boundary(names.remote_filename.len())
        );
    }

    #[test]
    fn validates_explicit_remote_names() {
        assert_eq!(
            validate_remote_filename(" part.3mf"),
            Err(NameError::SurroundingWhitespace)
        );
        assert_eq!(
            validate_remote_filename("part?.3mf"),
            Err(NameError::InvalidCharacters)
        );
        assert!(validate_remote_filename("part.gcode.3mf").is_ok());
    }
}
