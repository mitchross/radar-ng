//! Bearer-token authentication for the `/v1` routes (spec 6.2).
//!
//! Token format: `base64url_nopad(json) + "." + base64url_nopad(HMAC-SHA256(key, payload_part_text))`.
//! Only `sub` and `exp` are checked. The decoder is strict: a token with `=` padding or characters
//! outside the URL-safe alphabet is rejected. The Python code accepted padding, but no issuer
//! produces it (spec 6.2, which recommends strict decoding).
//!
//! Deviation: a non-finite `exp` (`NaN`, `inf`) is rejected. Python's `nan <= now` is false, so it
//! accepted such a token.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use radar_core::pycompat::py_str;
use serde_json::Value;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::respond;

const BEARER: &str = "Bearer ";

fn invalid() -> Response {
    respond::detail(StatusCode::UNAUTHORIZED, "invalid bearer token")
}

/// Decode one base64url segment without padding. `None` on any character or padding error.
fn decode_segment(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text).ok()
}

/// HMAC-SHA256 of the payload text (the base64url text itself, as the issuer signs it).
fn sign(key: &[u8], payload_text: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(payload_text.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Verify the `Authorization` header and return the subject.
///
/// # Errors
/// Returns the 503 response when no signing key is configured, or the 401 response for a missing
/// or invalid token.
#[allow(
    clippy::result_large_err,
    reason = "the Err is the ready-made HTTP response; boxing it would add a deref at about 50 call sites for no gain on this cold path"
)]
pub fn authenticate(headers: &HeaderMap, key: &[u8]) -> Result<String, Response> {
    if key.is_empty() {
        return Err(respond::detail(
            StatusCode::SERVICE_UNAVAILABLE,
            "workflow API authentication is not configured",
        ));
    }
    let header_value = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let Some(raw) = header_value.filter(|h| h.starts_with(BEARER)) else {
        return Err(respond::detail(
            StatusCode::UNAUTHORIZED,
            "missing bearer token",
        ));
    };
    let token = raw[BEARER.len()..].trim();
    let Some((payload_part, signature_part)) = token.split_once('.') else {
        return Err(invalid());
    };
    let expected = sign(key, payload_part);
    let Some(signature) = decode_segment(signature_part) else {
        return Err(invalid());
    };
    if !bool::from(expected.ct_eq(&signature)) {
        return Err(invalid());
    }
    let Some(payload_bytes) = decode_segment(payload_part) else {
        return Err(invalid());
    };
    let Ok(Value::Object(payload)) = serde_json::from_slice::<Value>(&payload_bytes) else {
        return Err(invalid());
    };
    let Some(sub) = payload.get("sub") else {
        return Err(invalid());
    };
    let subject = py_str(sub);
    let expires_at = match payload.get("exp") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    let Some(expires_at) = expires_at.filter(|e| e.is_finite()) else {
        return Err(invalid());
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    if subject.is_empty() || expires_at <= now {
        return Err(invalid());
    }
    Ok(subject)
}

/// Constant-time owner check (spec 6.2). A mismatch is the same 404 as a missing resource.
///
/// # Errors
/// Returns the 404 `{"detail":"not found"}` response when the subject is not the requested user.
#[allow(
    clippy::result_large_err,
    reason = "the Err is the ready-made HTTP response; boxing it would add a deref at about 50 call sites for no gain on this cold path"
)]
pub fn require_owner(authenticated: &str, requested: &str) -> Result<(), Response> {
    if bool::from(authenticated.as_bytes().ct_eq(requested.as_bytes())) {
        Ok(())
    } else {
        Err(respond::detail(StatusCode::NOT_FOUND, "not found"))
    }
}

/// Build a token the way the issuer does. Used by tests and the HMAC test vectors.
#[cfg(test)]
pub fn issue(key: &[u8], payload_json: &str) -> String {
    let part = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(sign(key, &part));
    format!("{part}.{sig}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const KEY: &[u8] = b"test-signing-key";

    fn headers(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).expect("header"),
        );
        h
    }

    #[test]
    fn valid_token_returns_subject() {
        let token = issue(KEY, r#"{"sub":"user-42","exp":4102444800}"#);
        assert_eq!(authenticate(&headers(&token), KEY).expect("ok"), "user-42");
    }

    #[test]
    fn numeric_subject_is_coerced_and_string_exp_accepted() {
        let token = issue(KEY, r#"{"sub":42,"exp":"4102444800"}"#);
        assert_eq!(authenticate(&headers(&token), KEY).expect("ok"), "42");
    }

    #[test]
    fn unset_key_is_503_even_with_a_good_token() {
        let token = issue(KEY, r#"{"sub":"u","exp":4102444800}"#);
        let resp = authenticate(&headers(&token), b"").expect_err("503");
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn scheme_is_case_sensitive_and_header_must_exist() {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer abc.def"),
        );
        assert_eq!(
            authenticate(&h, KEY).expect_err("401").status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            authenticate(&HeaderMap::new(), KEY)
                .expect_err("401")
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn expired_empty_subject_wrong_key_and_bad_shapes_are_401() {
        for token in [
            issue(KEY, r#"{"sub":"u","exp":1000}"#),
            issue(KEY, r#"{"sub":"","exp":4102444800}"#),
            issue(b"other-key", r#"{"sub":"u","exp":4102444800}"#),
            issue(KEY, r#"{"exp":4102444800}"#),
            "nodot".to_string(),
            format!("{}.{}", URL_SAFE_NO_PAD.encode(b"{}"), "AAAA"),
        ] {
            assert_eq!(
                authenticate(&headers(&token), KEY)
                    .expect_err("401")
                    .status(),
                StatusCode::UNAUTHORIZED,
                "{token}"
            );
        }
    }

    #[test]
    fn padding_and_trailing_junk_on_the_signature_are_rejected() {
        let token = issue(KEY, r#"{"sub":"u","exp":4102444800}"#);
        assert!(authenticate(&headers(&format!("{token}==")), KEY).is_err());
        assert!(authenticate(&headers(&format!("{token}!")), KEY).is_err());
    }

    #[test]
    fn owner_check_is_404_on_mismatch() {
        assert!(require_owner("u1", "u1").is_ok());
        assert_eq!(
            require_owner("u1", "u2").expect_err("404").status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[cfg(test)]
mod vector_tests {
    //! Replays `testdata/hmac_vectors.json`, which holds the results of running the real Python
    //! `authenticated_user` (see the generator notes in the file). Each case checks the Rust
    //! result: `rust` in the file, which differs from `python` only where the port deliberately
    //! differs (strict padding, non-finite `exp`).

    use super::*;
    use axum::http::HeaderValue;
    use serde_json::Value;

    #[tokio::test]
    async fn python_vectors_replay_against_the_port() {
        let doc: Value = serde_json::from_str(include_str!("../../testdata/hmac_vectors.json"))
            .expect("vector file");
        let key = doc["signing_key"]
            .as_str()
            .expect("signing key")
            .as_bytes()
            .to_vec();
        let cases = doc["cases"].as_array().expect("cases");
        assert!(cases.len() >= 15, "expected the full vector set");
        for case in cases {
            let name = case["name"].as_str().unwrap_or("?");
            let mut headers = HeaderMap::new();
            if let Some(value) = case["authorization"].as_str() {
                headers.insert(
                    header::AUTHORIZATION,
                    HeaderValue::from_str(value).expect("header value"),
                );
            }
            let expected = &case["rust"];
            match authenticate(&headers, &key) {
                Ok(subject) => {
                    assert_eq!(
                        Some(subject.as_str()),
                        expected["ok"].as_str(),
                        "case {name}"
                    );
                }
                Err(resp) => {
                    assert_eq!(
                        expected["status"].as_u64(),
                        Some(u64::from(resp.status().as_u16())),
                        "case {name}: status"
                    );
                    let bytes = axum::body::to_bytes(resp.into_body(), 4096)
                        .await
                        .expect("body");
                    let body: Value = serde_json::from_slice(&bytes).expect("json body");
                    assert_eq!(body["detail"], expected["detail"], "case {name}: detail");
                }
            }
        }
    }
}
