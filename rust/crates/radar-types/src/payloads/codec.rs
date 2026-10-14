//! Crate-private serde helpers shared by the payload structs.

use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serializer};
use serde_json::{Map, Value};

/// Serializes a finite `f64`. Non-finite values fail the encode instead of
/// emitting `NaN`/`Infinity` (which serde_json would write as `null`, a value
/// Python cannot decode into a float).
pub(crate) fn finite_f64<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.is_finite() {
        serializer.serialize_f64(*value)
    } else {
        Err(S::Error::custom(
            "non-finite float cannot be encoded for Temporal JSON",
        ))
    }
}

/// `Option<f64>` variant of [`finite_f64`]; `None` encodes as `null`.
pub(crate) fn finite_opt_f64<S: Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(v) => finite_f64(v, serializer),
        None => serializer.serialize_none(),
    }
}

/// Deserializes a JSON object, mapping `null` (or absent, via `#[serde(default)]`)
/// to an empty object. Matches the Python `geometry or {}` normalisation.
pub(crate) fn null_as_empty_object<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Map<String, Value>, D::Error> {
    Ok(Option::<Map<String, Value>>::deserialize(deserializer)?.unwrap_or_default())
}

/// Python default `1` (for example `OpenMeteoSyncArgs.past_days`).
pub(crate) fn default_one() -> i64 {
    1
}

/// Python default for `IngestMrmsArgs.mrms_prefix`: env `MRMS_PREFIX` or the QC base product.
pub(crate) fn default_mrms_prefix() -> String {
    std::env::var("MRMS_PREFIX").unwrap_or_else(|_| super::mrms::DEFAULT_MRMS_PREFIX.to_owned())
}

/// Python default for `IngestMrmsArgs.layer_name`: env `LAYER_NAME` or `radar`.
pub(crate) fn default_layer_name() -> String {
    std::env::var("LAYER_NAME").unwrap_or_else(|_| super::mrms::DEFAULT_LAYER_NAME.to_owned())
}
