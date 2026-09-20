use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use thiserror::Error;

use super::CanonicalModel;

pub const MAX_PUBLIC_CATALOGUE_BYTES: usize = 2 << 20;
const PUBLIC_CATALOGUE_HOSTS: [&str; 2] = ["bambulab.com", "www.bambulab.com"];
/// Bambu ships at most one stable release per model per day and serves these
/// pages behind Cloudflare, so a short TTL would buy nothing and risk being
/// rate limited.
const PUBLIC_CATALOGUE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

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
    public_catalogue_urls(model).first().copied()
}

/// Returns the model-specific official page first, optionally followed by a
/// shared family page that publishes a release history for that family.
///
/// A fallback is only safe when the page is not another model's own page:
/// Bambu ships distinct, non-interchangeable firmware per model, so reading
/// `.../a1` for an A1 mini would report a different printer's version as this
/// printer's public-stable release. `public_catalogue_family_pages_are_shared`
/// locks that invariant in.
///
// ponytail: bambulab.com answers 403 to non-browser clients, so the a2l, x2d,
// and h2d-pro slugs cannot be checked from CI. A wrong slug degrades to
// `publicCatalogueUnavailable`, never to another model's firmware version, so
// the shared-page test covers the failure that matters. Confirm the slugs
// during physical qualification of those models.
pub fn public_catalogue_urls(model: CanonicalModel) -> &'static [&'static str] {
    match model {
        CanonicalModel::A1 => &["https://bambulab.com/en-us/support/firmware-download/a1"],
        CanonicalModel::A1Mini => &["https://bambulab.com/en-us/support/firmware-download/a1-mini"],
        CanonicalModel::A2L => &["https://bambulab.com/en-us/support/firmware-download/a2l"],
        CanonicalModel::P1P => &[
            "https://bambulab.com/en-us/support/firmware-download/p1p",
            "https://bambulab.com/en-us/support/firmware-download/p1",
        ],
        CanonicalModel::P1S => &[
            "https://bambulab.com/en-us/support/firmware-download/p1s",
            "https://bambulab.com/en-us/support/firmware-download/p1",
        ],
        CanonicalModel::P2S => &["https://bambulab.com/en-us/support/firmware-download/p2s"],
        CanonicalModel::X1 => &["https://bambulab.com/en-us/support/firmware-download/x1"],
        CanonicalModel::X1Carbon => &["https://bambulab.com/en-us/support/firmware-download/x1c"],
        CanonicalModel::X1E => &["https://bambulab.com/en-us/support/firmware-download/x1e"],
        CanonicalModel::X2D => &["https://bambulab.com/en-us/support/firmware-download/x2d"],
        CanonicalModel::H2D => &["https://bambulab.com/en-us/support/firmware-download/h2d"],
        CanonicalModel::H2DPro => &["https://bambulab.com/en-us/support/firmware-download/h2d-pro"],
        CanonicalModel::H2S => &["https://bambulab.com/en-us/support/firmware-download/h2s"],
        CanonicalModel::H2C => &["https://bambulab.com/en-us/support/firmware-download/h2c"],
        CanonicalModel::Unknown => &[],
    }
}

struct CachedRelease {
    release: PublicFirmwareRelease,
    fetched_at: Instant,
}

fn fresh(entry: &CachedRelease) -> bool {
    entry.fetched_at.elapsed() < PUBLIC_CATALOGUE_TTL
}

/// One slot per canonical model, shared by every profile of that model, so a
/// result is reused across printers instead of refetched per printer.
type Slot = Arc<Mutex<Option<CachedRelease>>>;

fn slot(model: CanonicalModel) -> Slot {
    static SLOTS: OnceLock<Mutex<HashMap<CanonicalModel, Slot>>> = OnceLock::new();
    let slots = SLOTS.get_or_init(Mutex::default);
    let mut slots = slots.lock().unwrap_or_else(|error| error.into_inner());
    slots.entry(model).or_default().clone()
}

/// Fetches the latest public stable release for a model, reusing a cached
/// result for [`PUBLIC_CATALOGUE_TTL`].
///
/// The per-model slot is held across the fetch, so concurrent callers for the
/// same model wait for the first request and share its result rather than
/// each issuing their own. A waiting caller can therefore block for longer
/// than its own `timeout`, which is the point: one request per model.
pub fn fetch_public_firmware(
    model: CanonicalModel,
    timeout: Duration,
) -> Result<PublicFirmwareRelease, PublicCatalogueError> {
    let slot = slot(model);
    let mut cached = slot.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(entry) = cached.as_ref().filter(|entry| fresh(entry)) {
        return Ok(entry.release.clone());
    }
    // ponytail: successes only, so a transient Cloudflare block or parse
    // failure retries on the next check instead of being pinned for a day.
    // Add negative caching with backoff if those retries ever read as
    // rate limiting.
    let release = fetch_uncached(model, timeout)?;
    *cached = Some(CachedRelease {
        release: release.clone(),
        fetched_at: Instant::now(),
    });
    Ok(release)
}

fn fetch_uncached(
    model: CanonicalModel,
    timeout: Duration,
) -> Result<PublicFirmwareRelease, PublicCatalogueError> {
    let urls = public_catalogue_urls(model);
    if urls.is_empty() {
        return Err(PublicCatalogueError::UnsupportedModel);
    }
    let client = Client::builder()
        .timeout(timeout)
        .redirect(Policy::none())
        .user_agent("Melt firmware availability/1.0")
        .build()?;
    let mut last_error = PublicCatalogueError::Status;
    for url in urls {
        let parsed = reqwest::Url::parse(url).map_err(|_| PublicCatalogueError::InvalidUrl)?;
        if parsed.scheme() != "https"
            || !parsed
                .host_str()
                .is_some_and(|host| PUBLIC_CATALOGUE_HOSTS.contains(&host))
        {
            return Err(PublicCatalogueError::InvalidUrl);
        }
        let mut response = match client.get(parsed).send() {
            Ok(response) => response,
            Err(error) => {
                last_error = PublicCatalogueError::Request(error);
                continue;
            }
        };
        if !response.status().is_success() {
            last_error = PublicCatalogueError::Status;
            continue;
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PUBLIC_CATALOGUE_BYTES as u64)
        {
            last_error = PublicCatalogueError::TooLarge;
            continue;
        }
        let mut body = Vec::new();
        if response
            .by_ref()
            .take((MAX_PUBLIC_CATALOGUE_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .is_err()
        {
            last_error = PublicCatalogueError::Read;
            continue;
        }
        if body.len() > MAX_PUBLIC_CATALOGUE_BYTES {
            last_error = PublicCatalogueError::TooLarge;
            continue;
        }
        let html = String::from_utf8_lossy(&body);
        if let Some(version) = parse_public_firmware_version(&html) {
            return Ok(PublicFirmwareRelease {
                model,
                version,
                url: (*url).to_owned(),
            });
        }
        last_error = PublicCatalogueError::VersionNotFound;
    }
    Err(last_error)
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
        assert_eq!(
            public_catalogue_url(CanonicalModel::A1Mini),
            Some("https://bambulab.com/en-us/support/firmware-download/a1-mini")
        );
        assert!(
            public_catalogue_urls(CanonicalModel::P1S)
                .contains(&"https://bambulab.com/en-us/support/firmware-download/p1")
        );
    }

    /// Every model ships its own firmware, so a fallback page must never be
    /// another model's own catalogue page. Without this, a failed scrape of
    /// `.../a1-mini` would silently report the A1's version for an A1 mini.
    #[test]
    fn public_catalogue_family_pages_are_shared() {
        let models = super::super::models()
            .iter()
            .map(|entry| entry.canonical)
            .chain([CanonicalModel::Unknown])
            .collect::<Vec<_>>();
        let primaries: Vec<&str> = models
            .iter()
            .filter_map(|model| public_catalogue_url(*model))
            .collect();
        for model in models {
            assert_eq!(
                public_catalogue_url(model).is_some(),
                model != CanonicalModel::Unknown,
                "{model:?} has no official catalogue page"
            );
            for fallback in public_catalogue_urls(model).iter().skip(1) {
                assert!(
                    !primaries.contains(fallback),
                    "{model:?} falls back to another model's page: {fallback}"
                );
            }
        }
    }

    /// A cache hit must answer without touching the network, and an entry
    /// older than the TTL must stop counting as one.
    #[test]
    fn public_catalogue_results_are_cached_per_model_until_the_ttl_expires() {
        let model = CanonicalModel::H2C;
        let release = PublicFirmwareRelease {
            model,
            version: "01.02.03.04".into(),
            url: public_catalogue_url(model).unwrap().to_owned(),
        };
        let slot = slot(model);
        *slot.lock().unwrap() = Some(CachedRelease {
            release: release.clone(),
            fetched_at: Instant::now(),
        });

        // A zero timeout makes any real request fail, so a success here can
        // only have come from the cache.
        assert_eq!(
            fetch_public_firmware(model, Duration::ZERO).unwrap(),
            release
        );

        if let Some(expired) =
            Instant::now().checked_sub(PUBLIC_CATALOGUE_TTL + Duration::from_secs(1))
        {
            assert!(!fresh(&CachedRelease {
                release,
                fetched_at: expired,
            }));
        }
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
