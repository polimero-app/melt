use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[test]
fn package_configuration_keeps_the_native_artifact_and_cli_contracts() {
    let config: Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("valid Tauri config");

    assert_eq!(config["productName"], "Melt");
    assert_eq!(config["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(config["identifier"], "app.polimero.melt");
    assert_eq!(config["bundle"]["active"], true);
    assert_eq!(config["bundle"]["targets"], "all");
    assert_eq!(env!("CARGO_PKG_LICENSE"), "AGPL-3.0-only");
    assert_eq!(
        config["bundle"]["resources"],
        serde_json::json!(["../../LICENSE"])
    );
    assert_eq!(
        config["bundle"]["linux"]["deb"]["depends"],
        serde_json::json!(["ffmpeg"])
    );
    assert_eq!(
        config["bundle"]["linux"]["rpm"]["depends"],
        serde_json::json!(["ffmpeg"])
    );
    assert_eq!(config["app"]["windows"][0]["minWidth"], 1280);
    assert_eq!(config["app"]["windows"][0]["minHeight"], 800);

    // A declared but missing icon only fails on the macOS/Windows release
    // runners, so resolve every path here instead.
    let icons = config["bundle"]["icon"]
        .as_array()
        .expect("declared bundle icons");
    assert_eq!(
        config["bundle"]["icon"],
        serde_json::json!([
            "icons/32x32.png",
            "icons/128x128.png",
            "icons/128x128@2x.png",
            "icons/icon.icns",
            "icons/icon.ico"
        ])
    );
    for icon in icons {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(icon.as_str().expect("icon path"));
        assert!(path.is_file(), "missing bundle icon: {}", path.display());
    }

    let output = Command::new(env!("CARGO_BIN_EXE_melt"))
        .args(["version", "--output", "json"])
        .output()
        .expect("run packaged CLI entry point");
    assert!(output.status.success());
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("JSON version envelope");
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["meta"]["command"], "version");
    assert_eq!(
        envelope["data"]["modes"],
        serde_json::json!(["gui", "headless"])
    );
}
