//! Deterministic workflow ID derivation.

use sha2::{Digest, Sha256};

/// Prefix of every watch workflow ID.
pub const WATCH_ID_PREFIX: &str = "watch:";

/// Watch workflow ID: `"watch:" + lowercase hex(sha256(user_id + "\0" + storm_cell_id))`.
///
/// This is the production scheme (`routes_workflows._watch_id`). The docstring
/// in `watch_storm.py` (`watch:{user_id}:{storm_cell_id}`) is stale.
///
/// The NUL separator keeps `("ab", "c")` and `("a", "bc")` distinct. The
/// digest is 64 hex characters, so the full ID is 70 characters long.
pub fn watch_workflow_id(user_id: &str, storm_cell_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(storm_cell_id.as_bytes());
    format!("{WATCH_ID_PREFIX}{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_values_match_python_hashlib() {
        // Expected values computed with python3 hashlib:
        // "watch:" + sha256(f"{user_id}\0{storm_cell_id}").hexdigest()
        assert_eq!(
            watch_workflow_id("synthetic-user", "synthetic-cell"),
            "watch:e7a9325e8825bbd1565a0947d867872059e559b5bcdaceeae798d127e55a07e0"
        );
        assert_eq!(
            watch_workflow_id("user-1", "storm-42"),
            "watch:1c6b41b9449463f2de7991a97b8f740798bfe7d25c7450bd7533a026be64e8be"
        );
        assert_eq!(
            watch_workflow_id("", ""),
            "watch:6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d"
        );
        assert_eq!(
            watch_workflow_id("ué", "c"),
            "watch:b7f7cca5398a3414d6751dffb56684208c8d768c4e3fa173ee08a9d1a9833e90"
        );
    }

    #[test]
    fn separator_prevents_boundary_collisions() {
        assert_ne!(watch_workflow_id("ab", "c"), watch_workflow_id("a", "bc"));
    }

    #[test]
    fn id_is_70_chars_lowercase_hex() {
        let id = watch_workflow_id("u", "s");
        assert_eq!(id.len(), 70);
        assert!(id.starts_with("watch:"));
        assert!(
            id[6..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }
}
