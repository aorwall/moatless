//! Serde support for tri-state PATCH fields.
//!
//! A clearable field is `Option<Option<T>>`: `None` is "omitted, leave alone",
//! `Some(None)` is "explicit null, clear the column", `Some(Some(v))` is "set".

use serde::{Deserialize, Deserializer};

/// Deserialize a tri-state PATCH field, keeping an explicit `null` distinct
/// from an absent key.
///
/// Serde's derive collapses the two into `None`, making the clear case
/// unreachable; pair this with `#[serde(default, skip_serializing_if =
/// "Option::is_none")]` or every omitted field serializes as `null` and clears
/// on arrival.
#[allow(clippy::option_option)]
pub fn deserialize_optional_nullable_field<'de, D, T>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
