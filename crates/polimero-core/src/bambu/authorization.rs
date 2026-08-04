use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::AuthorizationMode;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthorizationEvidenceKind {
    ExplicitField,
    ProvisionalBit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationObservation {
    pub field: String,
    pub mode: AuthorizationMode,
    pub evidence_kind: AuthorizationEvidenceKind,
    pub active: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationResolution {
    pub effective: AuthorizationMode,
    pub conflict: bool,
    pub observations: Vec<AuthorizationObservation>,
}

pub fn resolve_authorization(report: &Value) -> AuthorizationResolution {
    let Some(print) = report.get("print").and_then(Value::as_object) else {
        return AuthorizationResolution::default();
    };
    let mut observations = Vec::new();
    for field in ["authorization_mode", "security_mode"] {
        if let Some(value) = print.get(field).and_then(Value::as_str)
            && let Some(mode) = explicit_mode(value)
        {
            observations.push(AuthorizationObservation {
                field: format!("print.{field}"),
                mode,
                evidence_kind: AuthorizationEvidenceKind::ExplicitField,
                active: true,
            });
        }
    }
    if let Some(required) = print
        .get("security")
        .and_then(|value| value.get("signing_required"))
        .and_then(Value::as_bool)
    {
        observations.push(AuthorizationObservation {
            field: "print.security.signing_required".into(),
            mode: if required {
                AuthorizationMode::SigningRequired
            } else {
                AuthorizationMode::DeveloperMode
            },
            evidence_kind: AuthorizationEvidenceKind::ExplicitField,
            active: true,
        });
    }
    if let Some(fun) = print.get("fun").and_then(Value::as_u64) {
        observations.push(AuthorizationObservation {
            field: "print.fun[29]".into(),
            mode: if fun & (1 << 29) == 0 {
                AuthorizationMode::DeveloperMode
            } else {
                AuthorizationMode::SigningRequired
            },
            evidence_kind: AuthorizationEvidenceKind::ProvisionalBit,
            active: false,
        });
    }
    let active = observations
        .iter()
        .filter(|observation| observation.active)
        .map(|observation| observation.mode)
        .collect::<Vec<_>>();
    let conflict = active
        .iter()
        .any(|mode| *mode == AuthorizationMode::DeveloperMode)
        && active
            .iter()
            .any(|mode| *mode == AuthorizationMode::SigningRequired);
    let effective = if conflict {
        AuthorizationMode::ConflictingEvidence
    } else {
        active
            .first()
            .copied()
            .unwrap_or(AuthorizationMode::Unknown)
    };
    AuthorizationResolution {
        effective,
        conflict,
        observations,
    }
}

fn explicit_mode(value: &str) -> Option<AuthorizationMode> {
    match value
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
        .as_str()
    {
        "developer" | "developer_mode" | "lan_only" => Some(AuthorizationMode::DeveloperMode),
        "signing_required" | "secured" | "secure" => Some(AuthorizationMode::SigningRequired),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_security_fields_resolve_authorization() {
        let resolution =
            resolve_authorization(&json!({"print":{"security":{"signing_required":true}}}));
        assert_eq!(resolution.effective, AuthorizationMode::SigningRequired);
        assert!(!resolution.conflict);
    }

    #[test]
    fn provisional_fun_bit_is_diagnostic_only() {
        let resolution = resolve_authorization(&json!({"print":{"fun":536870912}}));
        assert_eq!(resolution.effective, AuthorizationMode::Unknown);
        assert_eq!(
            resolution.observations[0].mode,
            AuthorizationMode::SigningRequired
        );
        assert!(!resolution.observations[0].active);
    }

    #[test]
    fn conflicting_explicit_fields_do_not_guess() {
        let resolution = resolve_authorization(
            &json!({"print":{"authorization_mode":"developer","security_mode":"secure"}}),
        );
        assert_eq!(resolution.effective, AuthorizationMode::ConflictingEvidence);
        assert!(resolution.conflict);
    }
}
