//! Environment-driven configuration (spec-tiler section 14).
//!
//! The pure `parse_*` / `resolve_*` functions take their inputs explicitly so tests do not
//! mutate the process environment (`std::env::set_var` is `unsafe` in edition 2024). The
//! `*_from_env` wrappers read the real environment at call time.

use crate::error::{Result, TilerError};

/// Default geometry cache byte budget, in MiB.
pub const DEFAULT_GEOMETRY_CACHE_MB: i64 = 256;
/// Default geometry cache entry budget.
pub const DEFAULT_GEOMETRY_CACHE_ENTRIES: i64 = 4096;
/// Default indexed PNG zlib level. Part of the indexed identity (spec H15).
pub const DEFAULT_PNG_COMPRESS_LEVEL: i64 = 6;

/// Roles that select a tile renderer.
pub const RENDERER_ROLES: [&str; 4] = ["mrms", "nowcast", "hrrr", "aux"];

/// Parse an integer environment value with the Python clamping rules: a missing, empty or
/// non-integer value yields `default`; the result is clamped to `[lo, hi]`.
pub fn parse_bounded_int(raw: Option<&str>, default: i64, lo: i64, hi: i64) -> i64 {
    let value = raw
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(default);
    value.clamp(lo, hi)
}

fn bounded_int_env(name: &str, default: i64, lo: i64, hi: i64) -> i64 {
    parse_bounded_int(std::env::var(name).ok().as_deref(), default, lo, hi)
}

/// `TILE_GEOMETRY_CACHE_MB` in `[0, 4096]`.
pub fn geometry_cache_mb() -> i64 {
    bounded_int_env("TILE_GEOMETRY_CACHE_MB", DEFAULT_GEOMETRY_CACHE_MB, 0, 4096)
}

/// `TILE_GEOMETRY_CACHE_ENTRIES` in `[1, 1_000_000]`.
pub fn geometry_cache_entries() -> i64 {
    bounded_int_env(
        "TILE_GEOMETRY_CACHE_ENTRIES",
        DEFAULT_GEOMETRY_CACHE_ENTRIES,
        1,
        1_000_000,
    )
}

/// `TILE_PNG_COMPRESS_LEVEL` in `[0, 9]`.
pub fn png_compress_level() -> u32 {
    bounded_int_env("TILE_PNG_COMPRESS_LEVEL", DEFAULT_PNG_COMPRESS_LEVEL, 0, 9) as u32
}

/// Renderer selected for one role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileRenderer {
    /// The legacy RGBA renderer. Not implemented in Rust; callers route this to Python.
    Legacy,
    /// The indexed (class-index) renderer implemented by this crate.
    Indexed,
}

impl TileRenderer {
    /// Canonical lowercase name, as used in `TILE_RENDERER`.
    pub fn as_str(self) -> &'static str {
        match self {
            TileRenderer::Legacy => "legacy",
            TileRenderer::Indexed => "indexed",
        }
    }
}

/// Resolve the renderer for `role` from explicit `TILE_RENDERER` and
/// `TILE_RENDERER_CANARY_ROLE` values (spec section 14, `tile_renderer_for_role`).
///
/// `TILE_RENDERER=indexed` alone cannot enable the new renderer for every role: a canary
/// role must also be named, and only that role gets `Indexed`.
pub fn resolve_tile_renderer(
    role: &str,
    renderer: Option<&str>,
    canary: Option<&str>,
) -> Result<TileRenderer> {
    if !RENDERER_ROLES.contains(&role) {
        return Err(TilerError::Invalid(format!(
            "unknown tile renderer role {role:?}"
        )));
    }
    let renderer = renderer.unwrap_or("legacy").trim().to_ascii_lowercase();
    match renderer.as_str() {
        "legacy" => Ok(TileRenderer::Legacy),
        "indexed" => {
            let canary = canary.unwrap_or("").trim().to_ascii_lowercase();
            if !RENDERER_ROLES.contains(&canary.as_str()) {
                return Err(TilerError::Invalid(format!(
                    "TILE_RENDERER=indexed requires TILE_RENDERER_CANARY_ROLE=<one of {RENDERER_ROLES:?}>"
                )));
            }
            Ok(if role == canary {
                TileRenderer::Indexed
            } else {
                TileRenderer::Legacy
            })
        }
        other => Err(TilerError::Invalid(format!(
            "unknown TILE_RENDERER={other:?}; expected 'legacy' or 'indexed'"
        ))),
    }
}

/// `tile_renderer_for_role` reading `TILE_RENDERER` and `TILE_RENDERER_CANARY_ROLE`.
pub fn tile_renderer_for_role(role: &str) -> Result<TileRenderer> {
    let renderer = std::env::var("TILE_RENDERER").ok();
    let canary = std::env::var("TILE_RENDERER_CANARY_ROLE").ok();
    resolve_tile_renderer(role, renderer.as_deref(), canary.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_int_parses_clamps_and_defaults() {
        assert_eq!(parse_bounded_int(None, 6, 0, 9), 6);
        assert_eq!(parse_bounded_int(Some(""), 6, 0, 9), 6);
        assert_eq!(parse_bounded_int(Some("abc"), 6, 0, 9), 6);
        assert_eq!(parse_bounded_int(Some(" 3 "), 6, 0, 9), 3);
        assert_eq!(parse_bounded_int(Some("42"), 6, 0, 9), 9);
        assert_eq!(parse_bounded_int(Some("-4"), 6, 0, 9), 0);
    }

    #[test]
    fn indexed_renderer_requires_one_named_canary_role() {
        // Missing canary raises and names the variable.
        let err = resolve_tile_renderer("mrms", Some("indexed"), None).unwrap_err();
        assert!(err.to_string().contains("CANARY_ROLE"));
        // Canary mrms makes mrms indexed and hrrr legacy.
        assert_eq!(
            resolve_tile_renderer("mrms", Some("indexed"), Some("mrms")).unwrap(),
            TileRenderer::Indexed
        );
        assert_eq!(
            resolve_tile_renderer("hrrr", Some("indexed"), Some("mrms")).unwrap(),
            TileRenderer::Legacy
        );
    }

    #[test]
    fn legacy_renderer_ignores_stale_canary_role() {
        for role in RENDERER_ROLES {
            assert_eq!(
                resolve_tile_renderer(role, Some("legacy"), Some("typo")).unwrap(),
                TileRenderer::Legacy
            );
        }
        // Default (unset) is legacy too.
        assert_eq!(
            resolve_tile_renderer("aux", None, None).unwrap(),
            TileRenderer::Legacy
        );
    }

    #[test]
    fn unknown_role_and_renderer_fail_closed() {
        assert!(resolve_tile_renderer("bogus", None, None).is_err());
        assert!(resolve_tile_renderer("mrms", Some("typo"), Some("mrms")).is_err());
        // Case-insensitive, stripped.
        assert_eq!(
            resolve_tile_renderer("mrms", Some(" INDEXED "), Some("MRMS")).unwrap(),
            TileRenderer::Indexed
        );
    }
}
