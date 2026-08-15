use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::Ipv4Addr,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u8,
    id: String,
    captured_at: String,
    transport: Transport,
    direction: Direction,
    printer: Printer,
    provenance: Provenance,
    source_url: Option<String>,
    source_revision: Option<String>,
    confidence: Confidence,
    sanitization: Vec<String>,
    assertion: String,
    qualification: String,
    removal_condition: String,
    #[serde(default)]
    conflicts_with: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Transport {
    Discovery,
    Mqtt,
    #[serde(rename = "tunnel-6000")]
    Tunnel6000,
    Ftps,
    Camera,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Direction {
    PrinterToClient,
    ClientToPrinter,
    Bidirectional,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Provenance {
    PhysicalCapture,
    ExecutableTest,
    Source,
    Documentation,
    Report,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Confidence {
    Provisional,
    Qualified,
    Confirmed,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Printer {
    model_raw: String,
    hardware_revision: Option<String>,
    modules: BTreeMap<String, String>,
}

#[test]
fn checked_in_bambu_evidence_is_complete_and_sanitized() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/bambu");
    let manifests = manifest_paths(&root);
    assert!(!manifests.is_empty(), "no Bambu evidence manifests found");

    let mut parsed = Vec::new();
    let mut ids = BTreeSet::new();
    for path in manifests {
        let bytes = fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let manifest: Manifest = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("{}: invalid manifest: {error}", path.display()));
        validate_manifest(&path, &manifest);
        assert!(
            ids.insert(manifest.id.clone()),
            "duplicate evidence ID {}",
            manifest.id
        );
        parsed.push((path, manifest));
    }

    for (path, manifest) in &parsed {
        for conflict in &manifest.conflicts_with {
            assert_ne!(
                conflict,
                &manifest.id,
                "{} conflicts with itself",
                path.display()
            );
            assert!(
                ids.contains(conflict),
                "{} references unknown conflict {conflict}",
                path.display()
            );
        }
        scan_sensitive(&path.with_file_name("manifest.json"));
        for relative in &manifest.files {
            let evidence = path.parent().expect("manifest directory").join(relative);
            assert!(
                evidence.is_file(),
                "{} references missing file {relative}",
                path.display()
            );
            assert!(
                !fs::symlink_metadata(&evidence)
                    .expect("evidence metadata")
                    .file_type()
                    .is_symlink(),
                "{} must not be a symlink",
                evidence.display()
            );
            scan_sensitive(&evidence);
        }
        let declared = manifest
            .files
            .iter()
            .map(PathBuf::from)
            .collect::<BTreeSet<_>>();
        let present = evidence_files(path.parent().expect("manifest directory"));
        assert_eq!(
            present,
            declared,
            "{} must declare every artifact in files and no missing artifact",
            path.display()
        );
    }
}

fn manifest_paths(root: &Path) -> Vec<PathBuf> {
    fn visit(directory: &Path, manifests: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).expect("read fixture directory") {
            let entry = entry.expect("read fixture entry");
            let path = entry.path();
            if entry.file_type().expect("fixture type").is_dir() {
                visit(&path, manifests);
            } else if entry.file_name() == "manifest.json" {
                manifests.push(path);
            }
        }
    }
    let mut manifests = Vec::new();
    visit(root, &mut manifests);
    manifests.sort();
    manifests
}

fn evidence_files(directory: &Path) -> BTreeSet<PathBuf> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeSet<PathBuf>) {
        for entry in fs::read_dir(directory).expect("read evidence directory") {
            let entry = entry.expect("read evidence entry");
            let path = entry.path();
            if entry.file_type().expect("evidence type").is_dir() {
                visit(root, &path, files);
            } else if path.file_name().is_none_or(|name| name != "manifest.json") {
                files.insert(
                    path.strip_prefix(root)
                        .expect("relative evidence path")
                        .to_owned(),
                );
            }
        }
    }
    let mut files = BTreeSet::new();
    visit(directory, directory, &mut files);
    files
}

fn validate_manifest(path: &Path, manifest: &Manifest) {
    assert_eq!(
        manifest.schema_version,
        1,
        "{} uses an unsupported schema",
        path.display()
    );
    assert!(
        valid_id(&manifest.id),
        "{} has an invalid evidence ID",
        path.display()
    );
    OffsetDateTime::parse(&manifest.captured_at, &Rfc3339)
        .unwrap_or_else(|error| panic!("{} has an invalid capturedAt: {error}", path.display()));
    for (name, value) in [
        ("modelRaw", manifest.printer.model_raw.as_str()),
        ("assertion", manifest.assertion.as_str()),
        ("qualification", manifest.qualification.as_str()),
        ("removalCondition", manifest.removal_condition.as_str()),
    ] {
        assert!(
            !value.trim().is_empty(),
            "{} has an empty {name}",
            path.display()
        );
    }
    assert!(
        !manifest.sanitization.is_empty(),
        "{} must document sanitization",
        path.display()
    );
    assert_unique(path, "sanitization", &manifest.sanitization);
    assert_unique(path, "conflictsWith", &manifest.conflicts_with);
    assert_unique(path, "files", &manifest.files);
    for relative in &manifest.files {
        let candidate = Path::new(relative);
        assert!(
            !candidate.is_absolute()
                && candidate
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            "{} contains unsafe evidence path {relative:?}",
            path.display()
        );
    }
    if matches!(
        manifest.provenance,
        Provenance::Source | Provenance::Documentation | Provenance::Report
    ) {
        assert!(
            manifest
                .source_url
                .as_deref()
                .is_some_and(|value| url::Url::parse(value).is_ok()),
            "{} requires a valid sourceUrl",
            path.display()
        );
    }
    if matches!(manifest.provenance, Provenance::Source) {
        assert!(
            manifest
                .source_revision
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            "{} requires a sourceRevision",
            path.display()
        );
    }
    let _ = (
        &manifest.transport,
        &manifest.direction,
        &manifest.confidence,
        &manifest.printer.hardware_revision,
        &manifest.printer.modules,
    );
}

fn valid_id(value: &str) -> bool {
    value.len() >= 2
        && (value.as_bytes()[0].is_ascii_lowercase() || value.as_bytes()[0].is_ascii_digit())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

fn assert_unique(path: &Path, field: &str, values: &[String]) {
    let unique = values.iter().collect::<BTreeSet<_>>();
    assert_eq!(
        unique.len(),
        values.len(),
        "{} has duplicate {field} values",
        path.display()
    );
}

fn scan_sensitive(path: &Path) {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let Ok(text) = std::str::from_utf8(&bytes) else {
        panic!(
            "{} is binary; add and reference a reproducible sanitized textual summary instead",
            path.display()
        );
    };
    let lowered = text.to_ascii_lowercase();
    for forbidden in [
        "begin private key",
        "begin rsa private key",
        "begin openssh private key",
        "\"access_code\"",
        "\"accesscode\"",
        "\"password\"",
        "\"cloudtoken\"",
        "\"wifi_password\"",
        "/home/",
        "/users/",
        "c:\\users\\",
    ] {
        assert!(
            !lowered.contains(forbidden),
            "{} contains forbidden sensitive marker {forbidden:?}",
            path.display()
        );
    }
    for token in text.split(|character: char| {
        !(character.is_ascii_hexdigit() || character == '.' || character == ':' || character == '-')
    }) {
        if let Ok(address) = token.parse::<Ipv4Addr>() {
            assert!(
                !address.is_private() && !address.is_loopback() && !address.is_link_local(),
                "{} contains private network address {address}",
                path.display()
            );
        }
        assert!(
            !looks_like_mac(token),
            "{} contains a MAC address",
            path.display()
        );
    }
    if let Ok(json) = serde_json::from_str::<Value>(text) {
        reject_sensitive_json_keys(path, &json);
    }
}

fn looks_like_mac(value: &str) -> bool {
    let separator = if value.contains(':') { ':' } else { '-' };
    let parts = value.split(separator).collect::<Vec<_>>();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn reject_sensitive_json_keys(path: &Path, value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
                assert!(
                    !matches!(
                        normalized.as_str(),
                        "accesscode" | "password" | "cloudtoken" | "wifipassword" | "privatekey"
                    ),
                    "{} contains forbidden JSON key {key:?}",
                    path.display()
                );
                reject_sensitive_json_keys(path, value);
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| reject_sensitive_json_keys(path, value)),
        _ => {}
    }
}
