use serde::{Deserialize, Serialize};

use super::ModelFamily;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CanonicalModel {
    A1,
    A1Mini,
    A2,
    P1P,
    P1S,
    P2S,
    X1,
    X1Carbon,
    X1E,
    X2,
    H2D,
    H2S,
    H2C,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelIdentity {
    /// Exact value advertised by the printer or supplied by the user.
    pub raw: String,
    /// Separator-insensitive value used only for matching.
    pub normalized: String,
    pub canonical: CanonicalModel,
    pub family: ModelFamily,
}

impl ModelIdentity {
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let normalized = raw.trim().to_ascii_uppercase().replace(['-', '_', ' '], "");
        let canonical = match normalized.as_str() {
            "A1" => CanonicalModel::A1,
            "A1MINI" => CanonicalModel::A1Mini,
            "A2" => CanonicalModel::A2,
            "P1P" => CanonicalModel::P1P,
            "P1S" => CanonicalModel::P1S,
            "P2S" => CanonicalModel::P2S,
            "X1" => CanonicalModel::X1,
            "X1C" | "X1CARBON" => CanonicalModel::X1Carbon,
            "X1E" => CanonicalModel::X1E,
            "X2" => CanonicalModel::X2,
            "H2D" => CanonicalModel::H2D,
            "H2S" => CanonicalModel::H2S,
            "H2C" => CanonicalModel::H2C,
            _ => CanonicalModel::Unknown,
        };
        let family = ModelFamily::from_model(&normalized);
        Self {
            raw,
            normalized,
            canonical,
            family,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unknown_internal_identifiers() {
        let identity = ModelIdentity::parse("  future_dev_42 ");
        assert_eq!(identity.raw, "  future_dev_42 ");
        assert_eq!(identity.normalized, "FUTUREDEV42");
        assert_eq!(identity.canonical, CanonicalModel::Unknown);
        assert_eq!(identity.family, ModelFamily::Unknown);
    }

    #[test]
    fn canonicalizes_only_explicit_aliases() {
        assert_eq!(
            ModelIdentity::parse("X1-C").canonical,
            CanonicalModel::X1Carbon
        );
        assert_eq!(ModelIdentity::parse("P1S").family, ModelFamily::P1);
    }
}
