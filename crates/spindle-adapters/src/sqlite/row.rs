//! Row-parsing helpers shared across every record's `TryFrom<&rusqlite::Row>`.
//!
//! Centralizing these here keeps the per-record TryFrom impls short and means
//! the conversions for ID strings, JSON columns, timestamps, and booleans are
//! tested and tuned in one place.
//!
//! The Spindle schema (V0001) uses:
//!   * IDs:        `TEXT` in `table:ulid` format. We return `String`.
//!   * Timestamps: `INTEGER` unix microseconds. We return `chrono::DateTime<Utc>`.
//!   * Booleans:   `INTEGER` 0/1. We return `bool`.
//!   * JSON cols:  `TEXT` constrained by `CHECK(json_valid(col))`. We return
//!     the deserialized `T`.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Row;
use rusqlite::types::ValueRef;
use serde::de::DeserializeOwned;

pub type Timestamp = DateTime<Utc>;

/// Read a non-null `TEXT` column as `String`.
#[inline]
pub fn text(row: &Row<'_>, idx: usize) -> rusqlite::Result<String> {
    row.get::<_, String>(idx)
}

/// Read an `option<TEXT>` column.
#[inline]
pub fn opt_text(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<String>> {
    row.get::<_, Option<String>>(idx)
}

/// Read a non-null `INTEGER` column as `i64`.
#[inline]
pub fn int(row: &Row<'_>, idx: usize) -> rusqlite::Result<i64> {
    row.get::<_, i64>(idx)
}

/// Read an `option<INTEGER>` column.
#[inline]
pub fn opt_int(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<i64>> {
    row.get::<_, Option<i64>>(idx)
}

/// Read a non-null `REAL` column as `f64`.
#[inline]
pub fn real(row: &Row<'_>, idx: usize) -> rusqlite::Result<f64> {
    row.get::<_, f64>(idx)
}

/// Read an `option<REAL>` column.
#[inline]
pub fn opt_real(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<f64>> {
    row.get::<_, Option<f64>>(idx)
}

/// Read a `BOOL` column stored as `INTEGER 0/1`.
#[inline]
pub fn boolean(row: &Row<'_>, idx: usize) -> rusqlite::Result<bool> {
    Ok(row.get::<_, i64>(idx)? != 0)
}

/// Read a `datetime` stored as `INTEGER` unix microseconds.
pub fn time(row: &Row<'_>, idx: usize) -> rusqlite::Result<Timestamp> {
    let micros = row.get::<_, i64>(idx)?;
    micros_to_timestamp(micros).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })
}

/// Read an `option<datetime>` (unix microseconds, nullable).
pub fn opt_time(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<Timestamp>> {
    match row.get_ref(idx)? {
        ValueRef::Null => Ok(None),
        ValueRef::Integer(micros) => Some(micros_to_timestamp(micros).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                idx,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        }))
        .transpose(),
        other => Err(rusqlite::Error::InvalidColumnType(
            idx,
            "expected INTEGER or NULL".into(),
            other.data_type(),
        )),
    }
}

/// Read a non-null JSON `TEXT` column as a typed `T`.
pub fn json<T: DeserializeOwned>(row: &Row<'_>, idx: usize) -> rusqlite::Result<T> {
    let raw: String = row.get(idx)?;
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// Read a `BLOB` column of packed `f32` values and return them as `f64`s.
///
/// The on-disk layout matches sqlite-vec's vec0 format: little-endian IEEE-754
/// `float32` values, no header. Length must be a multiple of 4. The returned
/// `Vec<f64>` preserves the existing `Vec<f64>` API in `crate::ai`; the
/// conversion is lossless going from f32 → f64.
pub fn blob_f32_as_f64(row: &Row<'_>, idx: usize) -> rusqlite::Result<Vec<f64>> {
    let bytes: Vec<u8> = row.get(idx)?;
    if !bytes.len().is_multiple_of(4) {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "embedding blob length {} is not a multiple of 4",
                    bytes.len()
                ),
            )),
        ));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64)
        .collect())
}

/// Pack a `&[f64]` into the on-disk vec0 BLOB layout (`f32` LE bytes).
/// Use this when binding an `embedding` parameter for INSERT/UPDATE.
pub fn pack_embedding(embedding: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(embedding.len() * 4);
    for v in embedding {
        out.extend_from_slice(&(*v as f32).to_le_bytes());
    }
    out
}

/// Read an `option<JSON TEXT>` column. `NULL` yields `None`; non-null is parsed.
pub fn opt_json<T: DeserializeOwned>(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<T>> {
    let raw: Option<String> = row.get(idx)?;
    let Some(raw) = raw else { return Ok(None) };
    serde_json::from_str(&raw).map(Some).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// Convert a `chrono::DateTime<Utc>` to the unix-microsecond representation
/// used in the database.
#[inline]
pub fn timestamp_to_micros(ts: Timestamp) -> i64 {
    ts.timestamp_micros()
}

fn micros_to_timestamp(micros: i64) -> Result<Timestamp, String> {
    Utc.timestamp_micros(micros)
        .single()
        .ok_or_else(|| format!("ambiguous or out-of-range timestamp micros: {micros}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn round_trip_timestamp_micros() {
        let now = Utc::now();
        let micros = timestamp_to_micros(now);
        let restored = micros_to_timestamp(micros).expect("micros within range");
        // Drop nanoseconds — micros precision.
        assert_eq!(restored.timestamp_micros(), micros);
    }

    #[test]
    fn helpers_read_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (
                id      TEXT,
                n       INTEGER,
                f       REAL,
                b       INTEGER,
                ts      INTEGER,
                tags    TEXT,
                maybe   TEXT
            );
            INSERT INTO t VALUES ('x', 7, 3.125, 1, 1700000000000000, '[\"a\",\"b\"]', NULL);",
        )
        .unwrap();

        let parsed = conn
            .query_row("SELECT id, n, f, b, ts, tags, maybe FROM t", [], |r| {
                Ok((
                    text(r, 0)?,
                    int(r, 1)?,
                    real(r, 2)?,
                    boolean(r, 3)?,
                    time(r, 4)?,
                    json::<Vec<String>>(r, 5)?,
                    opt_text(r, 6)?,
                ))
            })
            .unwrap();

        assert_eq!(parsed.0, "x");
        assert_eq!(parsed.1, 7);
        assert!((parsed.2 - 3.125).abs() < 1e-9);
        assert!(parsed.3);
        assert_eq!(parsed.4.timestamp_micros(), 1700000000000000);
        assert_eq!(parsed.5, vec!["a".to_string(), "b".to_string()]);
        assert!(parsed.6.is_none());
    }
}
