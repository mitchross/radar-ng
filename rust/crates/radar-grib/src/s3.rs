//! Anonymous S3 access over HTTPS: ListObjectsV2 and object GET (spec-ingest-mrms-hrrr §4.1, §17.2).
//!
//! The base URL is configurable so tests can point at a local server. Query parameters go through
//! reqwest's query encoder, so the prefix and continuation token are percent-encoded (PLAN defect D6).
//! Python interpolates them raw. There are no in-process retries, as in the Python.

use std::time::Duration;

use bytes::Bytes;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{GribError, Result};
use crate::idx::ByteRange;

/// Page size used by the Python listing (`max-keys=1000`, the S3 maximum).
pub const MAX_KEYS: u32 = 1000;

/// One ListObjectsV2 response page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListPage {
    /// Keys on this page, in listing order (S3 returns them sorted by key).
    pub keys: Vec<String>,
    /// `IsTruncated == "true"` (exact string, as the Python compares it).
    pub is_truncated: bool,
    /// `NextContinuationToken`, when present and non-empty.
    pub next_token: Option<String>,
}

/// Parses a ListObjectsV2 XML document. Elements are matched by local name, so the S3 namespace
/// (`http://s3.amazonaws.com/doc/2006-03-01/`) is not required.
pub fn parse_list_objects(xml: &str) -> Result<ListPage> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut page = ListPage::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                stack.push(local_name(e.name().as_ref()));
                text.clear();
            }
            Ok(Event::Empty(e)) => {
                // An empty element has no text; `<Key/>` contributes nothing (Python: content.text None).
                let _ = local_name(e.name().as_ref());
                text.clear();
            }
            Ok(Event::Text(t)) => {
                let s = t
                    .unescape()
                    .map_err(|e| GribError::ListingParse(format!("text: {e}")))?;
                text.push_str(&s);
            }
            Ok(Event::CData(t)) => {
                text.push_str(&String::from_utf8_lossy(&t));
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref());
                let parent = stack
                    .len()
                    .checked_sub(2)
                    .and_then(|i| stack.get(i))
                    .map(String::as_str);
                match (parent, name.as_str()) {
                    (Some("Contents"), "Key") => {
                        if !text.is_empty() {
                            page.keys.push(text.clone());
                        }
                    }
                    (Some("ListBucketResult"), "IsTruncated") => page.is_truncated = text == "true",
                    (Some("ListBucketResult"), "NextContinuationToken") => {
                        page.next_token = if text.is_empty() {
                            None
                        } else {
                            Some(text.clone())
                        };
                    }
                    _ => {}
                }
                stack.pop();
                text.clear();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(GribError::ListingParse(e.to_string())),
        }
    }
    Ok(page)
}

fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    match s.rfind(':') {
        Some(i) => s[i + 1..].to_string(),
        None => s.into_owned(),
    }
}

/// Anonymous S3 client over a shared reqwest client.
#[derive(Debug, Clone)]
pub struct S3Client {
    http: reqwest::Client,
    base: String,
}

impl S3Client {
    /// Creates a client for `base_url` (no trailing slash), e.g. `https://noaa-mrms-pds.s3.amazonaws.com`.
    pub fn new(http: reqwest::Client, base_url: impl Into<String>) -> Self {
        let base = base_url.into().trim_end_matches('/').to_string();
        Self { http, base }
    }

    /// The configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// One ListObjectsV2 page for `prefix`, optionally continuing from `continuation`.
    pub async fn list_page(
        &self,
        prefix: &str,
        continuation: Option<&str>,
        timeout: Duration,
    ) -> Result<ListPage> {
        let url = format!("{}/", self.base);
        let mut query: Vec<(&str, &str)> = vec![("prefix", prefix), ("list-type", "2")];
        let max_keys = MAX_KEYS.to_string();
        query.push(("max-keys", &max_keys));
        if let Some(token) = continuation {
            query.push(("continuation-token", token));
        }
        let resp = self
            .http
            .get(&url)
            .query(&query)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| GribError::transport(&url, e))?;
        let status = resp.status();
        let final_url = resp.url().to_string();
        if !status.is_success() {
            return Err(GribError::HttpStatus {
                status: status.as_u16(),
                url: final_url,
            });
        }
        let body = resp
            .text()
            .await
            .map_err(|e| GribError::transport(&url, e))?;
        parse_list_objects(&body)
    }

    /// Lists every key under `prefix`, following continuation tokens (Python `_list_keys_sync`).
    ///
    /// Stops when `IsTruncated` is not `"true"` or no continuation token is returned. Keys are
    /// returned unfiltered; callers filter by suffix.
    pub async fn list_all(&self, prefix: &str, timeout: Duration) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self.list_page(prefix, token.as_deref(), timeout).await?;
            keys.extend(page.keys);
            if !page.is_truncated {
                break;
            }
            match page.next_token {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        Ok(keys)
    }

    /// GET of a whole object. Any non-2xx status is an error (Python `raise_for_status`).
    pub async fn get_object(&self, key: &str, timeout: Duration) -> Result<Bytes> {
        let url = format!("{}/{}", self.base, key);
        let resp = self
            .http
            .get(&url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| GribError::transport(&url, e))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(GribError::HttpStatus {
                status: status.as_u16(),
                url,
            });
        }
        resp.bytes()
            .await
            .map_err(|e| GribError::transport(&url, e))
    }

    /// Ranged GET. Requires `206 Partial Content` (PLAN defect D5: a server that ignores Range and
    /// answers 200 would otherwise be written once per range).
    ///
    /// When a `Content-Range` header is present its start must equal the requested start. When the
    /// range has a known end, the body must be exactly that long.
    pub async fn get_range(&self, key: &str, range: ByteRange, timeout: Duration) -> Result<Bytes> {
        let url = format!("{}/{}", self.base, key);
        let header = range.header_value();
        let resp = self
            .http
            .get(&url)
            .header(reqwest::header::RANGE, &header)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| GribError::transport(&url, e))?;
        let status = resp.status();
        if status.as_u16() != 206 {
            return Err(GribError::RangeNotHonored {
                url,
                range: header,
                detail: format!("status {}", status.as_u16()),
            });
        }
        let content_range = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = resp
            .bytes()
            .await
            .map_err(|e| GribError::transport(&url, e))?;
        check_partial(&url, &header, range, content_range.as_deref(), body.len())?;
        Ok(body)
    }

    /// HEAD status code. Transport errors are returned as `Err`; any status is `Ok` (no redirects
    /// are followed, so a 3xx comes back as itself).
    pub async fn head_status(&self, key: &str, timeout: Duration) -> Result<u16> {
        let url = format!("{}/{}", self.base, key);
        let resp = self
            .http
            .head(&url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| GribError::transport(&url, e))?;
        Ok(resp.status().as_u16())
    }
}

/// Validates a 206 response against the requested range (see [`S3Client::get_range`]).
pub(crate) fn check_partial(
    url: &str,
    header: &str,
    range: ByteRange,
    content_range: Option<&str>,
    body_len: usize,
) -> Result<()> {
    let fail = |detail: String| GribError::RangeNotHonored {
        url: url.to_string(),
        range: header.to_string(),
        detail,
    };
    if let Some(cr) = content_range {
        // "bytes START-END/TOTAL" (TOTAL may be "*")
        let spec = cr
            .strip_prefix("bytes ")
            .ok_or_else(|| fail(format!("Content-Range {cr:?}")))?;
        let (span, _total) = spec.split_once('/').unwrap_or((spec, "*"));
        let (start, end) = span
            .split_once('-')
            .ok_or_else(|| fail(format!("Content-Range {cr:?}")))?;
        let start: u64 = start
            .trim()
            .parse()
            .map_err(|_| fail(format!("Content-Range {cr:?}")))?;
        let end: u64 = end
            .trim()
            .parse()
            .map_err(|_| fail(format!("Content-Range {cr:?}")))?;
        if start != range.start {
            return Err(fail(format!("Content-Range starts at {start}")));
        }
        if matches!(range.end, Some(want_end) if end != want_end) {
            return Err(fail(format!("Content-Range ends at {end}")));
        }
    }
    if let Some(len) = range.len() {
        if body_len as u64 != len {
            return Err(fail(format!(
                "body has {body_len} bytes, range covers {len}"
            )));
        }
    } else if body_len == 0 {
        return Err(fail("empty body for an open-ended range".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE1: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>noaa-mrms-pds</Name>
  <Prefix>CONUS/MergedBaseReflectivityQC_00.50/20260930</Prefix>
  <KeyCount>2</KeyCount>
  <MaxKeys>1000</MaxKeys>
  <IsTruncated>true</IsTruncated>
  <Contents>
    <Key>CONUS/MergedBaseReflectivityQC_00.50/20260930/MRMS_MergedBaseReflectivityQC_00.50_20260930-000000.grib2.gz</Key>
    <Size>123</Size>
  </Contents>
  <Contents>
    <Key>CONUS/MergedBaseReflectivityQC_00.50/20260930/MRMS_MergedBaseReflectivityQC_00.50_20260930-000200.grib2.gz</Key>
  </Contents>
  <NextContinuationToken>1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=</NextContinuationToken>
</ListBucketResult>"#;

    const PAGE2: &str = r#"<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <IsTruncated>false</IsTruncated>
  <Contents><Key>a/b &amp; c_20260930-000400.grib2.gz</Key></Contents>
  <Contents><Key>a/MRMS.idx</Key></Contents>
</ListBucketResult>"#;

    #[test]
    fn parses_keys_truncation_and_token() {
        let p = parse_list_objects(PAGE1).unwrap();
        assert_eq!(p.keys.len(), 2);
        assert!(p.keys[0].ends_with("20260930-000000.grib2.gz"));
        assert!(p.is_truncated);
        assert_eq!(
            p.next_token.as_deref(),
            Some("1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=")
        );
    }

    #[test]
    fn unescapes_entities_and_handles_missing_namespace() {
        let p = parse_list_objects(PAGE2).unwrap();
        assert_eq!(
            p.keys,
            vec![
                "a/b & c_20260930-000400.grib2.gz".to_string(),
                "a/MRMS.idx".to_string()
            ]
        );
        assert!(!p.is_truncated);
        assert_eq!(p.next_token, None);
    }

    #[test]
    fn empty_listing_is_empty() {
        let p = parse_list_objects(
            "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>",
        )
        .unwrap();
        assert!(p.keys.is_empty());
    }

    #[test]
    fn malformed_xml_is_an_error() {
        assert!(parse_list_objects("<ListBucketResult><Contents><Key>x</Contents>").is_err());
    }

    #[test]
    fn partial_response_checks() {
        let r = ByteRange {
            start: 100,
            end: Some(199),
        };
        assert!(check_partial("u", "bytes=100-199", r, Some("bytes 100-199/5000"), 100).is_ok());
        assert!(check_partial("u", "bytes=100-199", r, None, 100).is_ok());
        assert!(check_partial("u", "bytes=100-199", r, Some("bytes 0-199/5000"), 100).is_err());
        assert!(check_partial("u", "bytes=100-199", r, None, 99).is_err());
        let open = ByteRange {
            start: 30000,
            end: None,
        };
        assert!(
            check_partial(
                "u",
                "bytes=30000-",
                open,
                Some("bytes 30000-30500/40000"),
                501
            )
            .is_ok()
        );
        assert!(check_partial("u", "bytes=30000-", open, None, 0).is_err());
    }
}
