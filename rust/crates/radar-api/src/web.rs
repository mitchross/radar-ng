//! The web radar lane: the static Vite build of `web/` under `WEB_ROOT`.
//!
//! It runs from the top-level fallback, so every API and static route wins first. An unset or
//! empty `WEB_ROOT` turns the lane off and the fallback answers 404 as before.
//!
//! - `/assets/*`: content-hashed bundles, `immutable` for a year. A missing bundle is a 404, never
//!   the page, so a stale tab cannot load HTML as a script after a deploy.
//! - Every other GET or HEAD path: the file when it exists, otherwise `index.html`, with
//!   `Cache-Control: no-cache` so a deploy shows up at once.
//! - Paths under the API and static lanes ([`is_reserved`]) keep their own 404.
//!
//! Every response carries the security headers, errors included. `.gz` files next to the build
//! output are served to clients that accept gzip (the image build writes them).

use std::path::Path;

use anyhow::{Result, bail};
use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

use crate::config::Config;
use crate::respond;

const ASSET_CC: HeaderValue = HeaderValue::from_static("public, max-age=31536000, immutable");
const SHELL_CC: HeaderValue = HeaderValue::from_static("no-cache");

/// Path prefixes owned by the API and the static lanes. The web lane never answers under them.
const RESERVED_PREFIXES: [&str; 6] = ["/api", "/v1", "/tiles", "/basemap", "/replays", "/level2"];

/// True when `path` is a reserved prefix or lies under one.
#[must_use]
pub fn is_reserved(path: &str) -> bool {
    RESERVED_PREFIXES.iter().any(|prefix| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// The Content-Security-Policy for the page. `analytics_origin` (PostHog) is the only third-party
/// origin; empty allows none.
fn content_security_policy(analytics_origin: &str) -> String {
    let extra = if analytics_origin.is_empty() {
        String::new()
    } else {
        format!(" {analytics_origin}")
    };
    format!(
        "default-src 'self'; script-src 'self'{extra}; connect-src 'self'{extra}; \
         img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; worker-src 'self' blob:; \
         font-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"
    )
}

/// The services and headers of the web lane. Cloning is cheap.
#[derive(Clone)]
pub struct WebLane {
    assets: ServeDir,
    shell: ServeDir<ServeFile>,
    csp: HeaderValue,
}

impl WebLane {
    /// The lane for `WEB_ROOT`, or `None` when it is unset.
    ///
    /// # Errors
    /// Fails when the root has no `index.html`, or `WEB_ANALYTICS_ORIGIN` is not one origin.
    pub fn from_config(config: &Config) -> Result<Option<Self>> {
        let Some(root) = &config.web_root else {
            return Ok(None);
        };
        Self::new(root, &config.web_analytics_origin).map(Some)
    }

    fn new(root: &Path, analytics_origin: &str) -> Result<Self> {
        let index = root.join("index.html");
        if !index.is_file() {
            bail!("WEB_ROOT has no index.html: {}", index.display());
        }
        let origin = analytics_origin.trim();
        // One origin only: a separator would add a second source or a second directive.
        if origin.contains([';', ',']) || origin.contains(char::is_whitespace) {
            bail!("WEB_ANALYTICS_ORIGIN is not a single origin: {analytics_origin:?}");
        }
        let csp = HeaderValue::from_str(&content_security_policy(origin)).map_err(|_| {
            anyhow::anyhow!("WEB_ANALYTICS_ORIGIN has characters a header cannot carry")
        })?;
        Ok(Self {
            assets: ServeDir::new(root).precompressed_gzip(),
            shell: ServeDir::new(root)
                .precompressed_gzip()
                .fallback(ServeFile::new(index).precompressed_gzip()),
            csp,
        })
    }

    /// Answer one GET or HEAD request for a path outside the reserved prefixes.
    pub async fn serve(&self, req: Request) -> Response {
        let is_asset = req.uri().path().starts_with("/assets/");
        let (served, cache_control) = if is_asset {
            (self.assets.clone().oneshot(req).await, ASSET_CC)
        } else {
            (self.shell.clone().oneshot(req).await, SHELL_CC)
        };
        let mut resp = match served {
            Ok(resp) => resp.into_response(),
            Err(never) => match never {},
        };
        if resp.status().is_client_error() || resp.status().is_server_error() {
            resp = respond::caddy_error(resp.status());
        } else {
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, cache_control);
        }
        let headers = resp.headers_mut();
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        );
        headers.insert(
            HeaderName::from_static("content-security-policy"),
            self.csp.clone(),
        );
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_prefixes_match_whole_segments() {
        for path in [
            "/api",
            "/api/nope",
            "/v1/x",
            "/tiles/a/b",
            "/basemap/tiles/x",
            "/level2",
        ] {
            assert!(is_reserved(path), "{path}");
        }
        for path in ["/", "/apix", "/tilesets", "/some/deep/link", "/assets/a.js"] {
            assert!(!is_reserved(path), "{path}");
        }
    }

    #[test]
    fn csp_adds_the_analytics_origin_to_script_and_connect_only() {
        let none = content_security_policy("");
        assert!(none.contains("script-src 'self';"));
        assert!(none.contains("connect-src 'self';"));
        let with = content_security_policy("https://ph.example.com");
        assert!(with.contains("script-src 'self' https://ph.example.com;"));
        assert!(with.contains("connect-src 'self' https://ph.example.com;"));
        assert!(with.contains("img-src 'self' data: blob:;"));
    }
}
