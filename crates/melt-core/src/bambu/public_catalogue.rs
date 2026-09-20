use std::{io::Read, time::Duration};

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use thiserror::Error;

use super::CanonicalModel;

pub const MAX_PUBLIC_CATALOGUE_BYTES: usize = 2 << 20;
const PUBLIC_CATALOGUE_HOSTS: [&str; 2] = ["bambulab.com", "www.bambulab.com"];

#[derive(Debug, Error)]
pub enum PublicCatalogueError {
    #[error("the printer model is not allowlisted for public firmware lookup")]
    UnsupportedModel,
    #[error("public firmware catalogue URL is not allowlisted")]
    InvalidUrl,
    #[error("public firmware catalogue request failed")]
    Request(#[from] reqwest::Error),
    #[error("public firmware catalogue returned an unsuccessful status")]
    Status,
    #[error("public firmware catalogue response is too large")]
    TooLarge,
    #[error("public firmware catalogue did not contain a stable firmware version")]
    VersionNotFound,
    #[error("public firmware catalogue response could not be read")]
    Read,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicFirmwareRelease {
    pub model: CanonicalModel,
    pub version: String,
    pub url: String,
}

pub fn public_catalogue_url(model: CanonicalModel) -> Option<&'static str> {
    let slug = match model {
        CanonicalModel::A1 | CanonicalModel::A1Mini => "a1",
        CanonicalModel::A2 => "a2",
        // Bambu publishes one P-series catalogue for both P1 variants. Using
        // the model-specific paths returns a generic 404 page, so the parser
        // never gets a release even when the official catalogue has one.
        CanonicalModel::P1P | CanonicalModel::P1S => "p1",
        CanonicalModel::P2S => "p2s",
        // X1/X1C/X1E share the X1 catalogue and release history.
        CanonicalModel::X1 | CanonicalModel::X1Carbon | CanonicalModel::X1E => "x1",
        CanonicalModel::X2 => "x2",
        CanonicalModel::H2D => "h2d",
        CanonicalModel::H2S => "h2s",
        CanonicalModel::H2C => "h2c",
        CanonicalModel::Unknown => return None,
    };
    Some(match slug {
        "a1" => "https://bambulab.com/en-us/support/firmware-download/a1",
        "a2" => "https://bambulab.com/en-us/support/firmware-download/a2",
        "p1" => "https://bambulab.com/en-us/support/firmware-download/p1",
        "p2s" => "https://bambulab.com/en-us/support/firmware-download/p2s",
        "x1" => "https://bambulab.com/en-us/support/firmware-download/x1",
        "x2" => "https://bambulab.com/en-us/support/firmware-download/x2",
        "h2d" => "https://bambulab.com/en-us/support/firmware-download/h2d",
        "h2s" => "https://bambulab.com/en-us/support/firmware-download/h2s",
        "h2c" => "https://bambulab.com/en-us/support/firmware-download/h2c",
        _ => unreachable!(),
    })
}

pub fn fetch_public_firmware(
    model: CanonicalModel,
    timeout: Duration,
) -> Result<PublicFirmwareRelease, PublicCatalogueError> {
    let url = public_catalogue_url(model).ok_or(PublicCatalogueError::UnsupportedModel)?;
    let parsed = reqwest::Url::parse(url).map_err(|_| PublicCatalogueError::InvalidUrl)?;
    if parsed.scheme() != "https"
        || !parsed
            .host_str()
            .is_some_and(|host| PUBLIC_CATALOGUE_HOSTS.contains(&host))
    {
        return Err(PublicCatalogueError::InvalidUrl);
    }
    let client = Client::builder()
        .timeout(timeout)
        .redirect(Policy::none())
        .user_agent("Melt firmware availability/1.0")
        .build()?;
    let mut response = client.get(parsed.clone()).send()?;
    if !response.status().is_success() {
        return Err(PublicCatalogueError::Status);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PUBLIC_CATALOGUE_BYTES as u64)
    {
        return Err(PublicCatalogueError::TooLarge);
    }
    let mut body = Vec::new();
    response
        .by_ref()
        .take((MAX_PUBLIC_CATALOGUE_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| PublicCatalogueError::Read)?;
    if body.len() > MAX_PUBLIC_CATALOGUE_BYTES {
        return Err(PublicCatalogueError::TooLarge);
    }
    let html = String::from_utf8_lossy(&body);
    let version =
        parse_public_firmware_version(&html).ok_or(PublicCatalogueError::VersionNotFound)?;
    Ok(PublicFirmwareRelease {
        model,
        version,
        url: url.to_owned(),
    })
}

/// Extracts the highest stable-looking Bambu firmware version from the page's
/// rendered/embedded HTML. The parser intentionally ignores prerelease labels
/// next to a candidate and never treats arbitrary download URLs as evidence.
pub fn parse_public_firmware_version(html: &str) -> Option<String> {
    let bytes = html.as_bytes();
    let mut candidates = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        let mut parts = 0;
        let mut valid = true;
        while index < bytes.len() {
            let part_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if part_start == index || index - part_start > 4 {
                valid = false;
                break;
            }
            parts += 1;
            if index >= bytes.len() || bytes[index] != b'.' {
                break;
            }
            index += 1;
        }
        if !valid || !(3..=4).contains(&parts) {
            continue;
        }
        let end = index;
        let version = &html[start..end];
        let context_start = start.saturating_sub(96);
        let context = html[context_start..end.min(html.len())].to_ascii_lowercase();
        if context.contains("beta") || context.contains("alpha") || context.contains("candidate") {
            continue;
        }
        candidates.push((
            crate::bambu::FirmwareVersion::parse(version),
            version.to_owned(),
        ));
    }
    candidates
        .into_iter()
        .max_by(|left, right| left.0.numeric_cmp(&right.0))
        .map(|(_, version)| version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlists_known_models_and_rejects_unknown() {
        assert!(public_catalogue_url(CanonicalModel::P1S).is_some());
        assert!(public_catalogue_url(CanonicalModel::Unknown).is_none());
    }

    #[test]
    fn extracts_highest_stable_version_and_ignores_prereleases() {
        let html = r#"
          <script>latestVersion="01.08.00.00-beta";</script>
          <div>Firmware version 01.07.00.00</div>
          <div>Firmware version 01.09.00.00</div>
        "#;
        assert_eq!(
            parse_public_firmware_version(html).as_deref(),
            Some("01.09.00.00")
        );
    }
}
