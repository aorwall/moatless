//! Decoding a closed-set enum out of the text column that stores it.
//!
//! A required column decodes through `#[sqlx(try_from = "String")]` on the row
//! field. A nullable one cannot, since `Option<T>: TryFrom<Option<String>>` is
//! not an impl this crate may write — these helpers cover that case, and refuse
//! an unrecognised value rather than reading it as absent: the columns hold a
//! closed set this application writes, so an unnamed value is corruption, not a
//! default.

/// Parse a nullable enum column. `None` stays `None`; an unrecognised value is
/// a decode error naming the column.
pub fn decode_optional<T>(
    column: &'static str,
    value: Option<String>,
) -> Result<Option<T>, sqlx::Error>
where
    T: TryFrom<String, Error = String>,
{
    value
        .map(T::try_from)
        .transpose()
        .map_err(|error| decode_error(column, error))
}

/// Parse a required enum column read as text.
pub fn decode_required<T>(column: &'static str, value: String) -> Result<T, sqlx::Error>
where
    T: TryFrom<String, Error = String>,
{
    T::try_from(value).map_err(|error| decode_error(column, error))
}

fn decode_error(column: &'static str, error: String) -> sqlx::Error {
    sqlx::Error::ColumnDecode {
        index: column.to_string(),
        source: error.into(),
    }
}
