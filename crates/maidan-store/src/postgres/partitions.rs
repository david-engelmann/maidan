//! Monthly range partitions for the append-only tables (Postgres only).
//!
//! A partitioned table here has one partition per calendar month (UTC) of its
//! time column, named `<table>_pYYYYMM`, plus a DEFAULT partition that catches
//! a row no month covers (a timestamp far ahead, or before the oldest month
//! still kept). The migration that partitions a table keeps its old rows as
//! one partition, `<table>_legacy`, covering everything before its first
//! month. SQLite has one table and nothing here.
//!
//! [`maintain`] keeps the current month and [`MONTHS_AHEAD`] more, so a
//! normal row never lands in DEFAULT. It runs at boot after the migrations
//! and on every retention sweep. When it adds a month that DEFAULT already
//! holds rows for (the sweep had not run for months, or a row came in dated
//! ahead), it moves those rows into the new partition first. It also keeps
//! each table's autovacuum settings on every partition, since a partitioned
//! parent takes none itself.
//!
//! Retention (`postgres/retention.rs`) drops a partition whole only when
//! every row in it is one the batched DELETE would have removed, and deletes
//! inside the partition otherwise; see [`list`] and [`drop_partition`].

use chrono::{DateTime, Datelike, TimeZone, Utc};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::error::StoreError;

/// How many months past the current one [`maintain`] keeps ready.
pub const MONTHS_AHEAD: u32 = 3;

/// A table partitioned by month.
#[derive(Debug)]
pub struct PartitionedTable {
    /// The partitioned parent.
    pub parent: &'static str,
    /// The time column it is partitioned on.
    pub key: &'static str,
    /// Storage parameters set on every partition. A partitioned parent
    /// cannot carry autovacuum settings, so they live on each partition.
    pub reloptions: &'static [&'static str],
}

/// The event log. Rows are only inserted (and deleted by retention or an
/// erasure), so the insert threshold is what triggers vacuum: a lower one
/// keeps the visibility map current for index-only scans, and freezes a month
/// soon after it fills instead of in one anti-wraparound pass much later.
/// Analyze runs sooner than the default too, because the newest month is the
/// one every cursor read hits and its statistics go stale fastest.
pub const EVENTS: PartitionedTable = PartitionedTable {
    parent: "maidan_events",
    key: "occurred_at",
    reloptions: &[
        "autovacuum_vacuum_scale_factor=0.05",
        "autovacuum_vacuum_insert_scale_factor=0.05",
        "autovacuum_analyze_scale_factor=0.02",
    ],
};

/// Every partitioned table [`maintain`] looks after.
pub const TABLES: &[&PartitionedTable] = &[&EVENTS];

/// One partition of a [`PartitionedTable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// The partition's name, quoted as SQL needs it.
    pub name: String,
    /// Inclusive lower bound; `None` for `MINVALUE` (and for DEFAULT).
    pub lower: Option<DateTime<Utc>>,
    /// Exclusive upper bound; `None` for DEFAULT.
    pub upper: Option<DateTime<Utc>>,
    pub is_default: bool,
}

impl Partition {
    /// Every row's time is before `cutoff`: the whole range ends by then.
    pub fn ends_by(&self, cutoff: DateTime<Utc>) -> bool {
        !self.is_default && self.upper.is_some_and(|upper| upper <= cutoff)
    }

    /// Some row's time may be before `cutoff`. DEFAULT can hold any time.
    pub fn starts_before(&self, cutoff: DateTime<Utc>) -> bool {
        self.is_default || self.lower.is_none_or(|lower| lower < cutoff)
    }
}

/// The partitions of `table`, oldest first, DEFAULT last.
pub async fn list<'e, E>(
    executor: E,
    table: &PartitionedTable,
) -> Result<Vec<Partition>, StoreError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // `pg_get_expr` prints the bounds as literals, which Postgres parses back
    // itself, so the session time zone does not matter.
    let rows = sqlx::query(
        "SELECT c.oid::regclass::text AS name,
                pg_get_expr(c.relpartbound, c.oid) = 'DEFAULT' AS is_default,
                (regexp_match(pg_get_expr(c.relpartbound, c.oid), 'FROM \\(''([^'']*)''\\)'))[1]::timestamptz AS lower,
                (regexp_match(pg_get_expr(c.relpartbound, c.oid), 'TO \\(''([^'']*)''\\)'))[1]::timestamptz AS upper
         FROM pg_inherits i
         JOIN pg_class c ON c.oid = i.inhrelid
         WHERE i.inhparent = $1::regclass
         ORDER BY is_default, upper NULLS LAST",
    )
    .bind(table.parent)
    .fetch_all(executor)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Partition {
                name: row.try_get("name")?,
                lower: row.try_get("lower")?,
                upper: row.try_get("upper")?,
                is_default: row.try_get("is_default")?,
            })
        })
        .collect()
}

/// The first instant of `at`'s month, in UTC.
pub fn month_start(at: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(at.year(), at.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(at)
}

/// The first instant of the month after the one `month` starts.
pub fn next_month(month: DateTime<Utc>) -> DateTime<Utc> {
    let (year, month_no) = if month.month() == 12 {
        (month.year() + 1, 1)
    } else {
        (month.year(), month.month() + 1)
    };
    Utc.with_ymd_and_hms(year, month_no, 1, 0, 0, 0)
        .single()
        .unwrap_or(month)
}

/// `<table>_pYYYYMM` for the month starting at `month`.
pub fn partition_name(table: &PartitionedTable, month: DateTime<Utc>) -> String {
    format!("{}_p{:04}{:02}", table.parent, month.year(), month.month())
}

fn literal(at: DateTime<Utc>) -> String {
    format!(
        "'{}'",
        at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
    )
}

/// Serializes maintenance and partition drops on one table across replicas.
async fn lock_table(
    tx: &mut Transaction<'_, Postgres>,
    table: &PartitionedTable,
) -> Result<(), StoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 1162))")
        .bind(format!("maidan.partitions.{}", table.parent))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Keep every table in [`TABLES`] ready through [`MONTHS_AHEAD`] months past
/// `now`'s. Returns how many partitions it created.
pub async fn maintain(pool: &PgPool, now: DateTime<Utc>) -> Result<u64, StoreError> {
    let mut created = 0;
    for table in TABLES {
        created += ensure_months(pool, table, now, MONTHS_AHEAD).await?;
    }
    Ok(created)
}

/// Create `table`'s monthly partitions from the end of the newest one through
/// `ahead` months past `now`'s, recreate DEFAULT if it is gone, and set the
/// table's storage parameters on any partition that lacks them.
pub async fn ensure_months(
    pool: &PgPool,
    table: &PartitionedTable,
    now: DateTime<Utc>,
    ahead: u32,
) -> Result<u64, StoreError> {
    let mut tx = pool.begin().await?;
    lock_table(&mut tx, table).await?;
    // A table whose partitioning migration has not run (a database migrated
    // only partway, as a migration test does) has nothing to keep.
    let partitioned: bool = sqlx::query_scalar(
        "SELECT COALESCE((SELECT relkind = 'p' FROM pg_class WHERE oid = to_regclass($1)), FALSE)",
    )
    .bind(table.parent)
    .fetch_one(&mut *tx)
    .await?;
    if !partitioned {
        return Ok(0);
    }
    let parts = list(&mut *tx, table).await?;
    let default = match parts.iter().find(|p| p.is_default) {
        Some(p) => p.name.clone(),
        None => {
            let name = format!("{}_default", table.parent);
            sqlx::query(&format!(
                "CREATE TABLE {name} PARTITION OF {} DEFAULT",
                table.parent
            ))
            .execute(&mut *tx)
            .await?;
            name
        }
    };
    let mut target = month_start(now);
    for _ in 0..=ahead {
        target = next_month(target);
    }
    let mut from = parts
        .iter()
        .filter_map(|p| p.upper)
        .max()
        .unwrap_or_else(|| month_start(now));
    let mut created = 0;
    while from < target {
        let to = next_month(from);
        create_month(&mut tx, table, &default, from, to).await?;
        created += 1;
        from = to;
    }
    set_reloptions(&mut tx, table).await?;
    tx.commit().await?;
    Ok(created)
}

async fn create_month(
    tx: &mut Transaction<'_, Postgres>,
    table: &PartitionedTable,
    default: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<(), StoreError> {
    let parent = table.parent;
    let key = table.key;
    let name = partition_name(table, from);
    let (lo, hi) = (literal(from), literal(to));
    let with = table.reloptions.join(", ");
    // Attaching checks that DEFAULT holds no row of the new range, so rows
    // already there move first. The move is a delete from DEFAULT, which must
    // not fire the table's delete cascade: the rows are not going away.
    let in_default: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM {default} WHERE {key} >= {lo} AND {key} < {hi})"
    ))
    .fetch_one(&mut **tx)
    .await?;
    if !in_default {
        sqlx::query(&format!(
            "CREATE TABLE {name} PARTITION OF {parent} FOR VALUES FROM ({lo}) TO ({hi}) WITH ({with})"
        ))
        .execute(&mut **tx)
        .await?;
        return Ok(());
    }
    sqlx::query(&format!(
        "CREATE TABLE {name} (LIKE {parent} INCLUDING DEFAULTS) WITH ({with})"
    ))
    .execute(&mut **tx)
    .await?;
    sqlx::query("SELECT set_config('maidan.partition_move', 'on', true)")
        .execute(&mut **tx)
        .await?;
    sqlx::query(&format!(
        "WITH moved AS (
             DELETE FROM {default} WHERE {key} >= {lo} AND {key} < {hi} RETURNING *
         )
         INSERT INTO {name} SELECT * FROM moved"
    ))
    .execute(&mut **tx)
    .await?;
    sqlx::query("SELECT set_config('maidan.partition_move', 'off', true)")
        .execute(&mut **tx)
        .await?;
    sqlx::query(&format!(
        "ALTER TABLE {parent} ATTACH PARTITION {name} FOR VALUES FROM ({lo}) TO ({hi})"
    ))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Set `table.reloptions` on each partition missing one of them. A partition
/// that already has them is left alone, so a sweep does not queue behind a
/// running vacuum for nothing.
async fn set_reloptions(
    tx: &mut Transaction<'_, Postgres>,
    table: &PartitionedTable,
) -> Result<(), StoreError> {
    let wanted: Vec<String> = table.reloptions.iter().map(|o| o.to_string()).collect();
    let missing: Vec<String> = sqlx::query_scalar(
        "SELECT c.oid::regclass::text
         FROM pg_inherits i
         JOIN pg_class c ON c.oid = i.inhrelid
         WHERE i.inhparent = $1::regclass
           AND NOT (COALESCE(c.reloptions, '{}') @> $2::text[])",
    )
    .bind(table.parent)
    .bind(&wanted)
    .fetch_all(&mut **tx)
    .await?;
    let with = table.reloptions.join(", ");
    for name in missing {
        sqlx::query(&format!("ALTER TABLE {name} SET ({with})"))
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Begin a partition drop on `table`: a transaction holding the maintenance
/// lock and the parent's lock, so no row can enter or leave the partition
/// while the caller checks it. Waits at most `lock_wait` for the parent; a
/// sweep that cannot get it in time skips the drop and deletes instead.
pub async fn begin_drop(
    pool: &PgPool,
    table: &PartitionedTable,
    lock_wait: std::time::Duration,
) -> Result<Option<Transaction<'static, Postgres>>, StoreError> {
    let mut tx = pool.begin().await?;
    lock_table(&mut tx, table).await?;
    sqlx::query(&format!(
        "SET LOCAL lock_timeout = '{}ms'",
        lock_wait.as_millis()
    ))
    .execute(&mut *tx)
    .await?;
    match sqlx::query(&format!(
        "LOCK TABLE {} IN ACCESS EXCLUSIVE MODE",
        table.parent
    ))
    .execute(&mut *tx)
    .await
    {
        Ok(_) => Ok(Some(tx)),
        Err(sqlx::Error::Database(err)) if err.code().as_deref() == Some("55P03") => {
            tracing::warn!(
                table = table.parent,
                "retention: table busy; deleting instead of dropping"
            );
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

/// Drop `partition` inside a transaction from [`begin_drop`].
pub async fn drop_partition(
    tx: &mut Transaction<'_, Postgres>,
    partition: &Partition,
) -> Result<(), StoreError> {
    sqlx::query(&format!("DROP TABLE {}", partition.name))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 30, 0)
            .single()
            .expect("date")
    }

    #[test]
    fn months_roll_over_the_year() {
        assert_eq!(
            month_start(at(2026, 12, 9)),
            Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap()
        );
        assert_eq!(
            next_month(month_start(at(2026, 12, 9))),
            Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()
        );
        assert_eq!(
            partition_name(&EVENTS, month_start(at(2027, 3, 31))),
            "maidan_events_p202703"
        );
    }

    #[test]
    fn bounds_decide_drop_and_delete() {
        let month = Partition {
            name: "p".into(),
            lower: Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
            upper: Some(Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap()),
            is_default: false,
        };
        let default = Partition {
            name: "d".into(),
            lower: None,
            upper: None,
            is_default: true,
        };
        let feb = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
        assert!(month.ends_by(feb));
        assert!(!month.ends_by(feb - chrono::Duration::seconds(1)));
        assert!(month.starts_before(feb));
        assert!(!month.starts_before(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()));
        assert!(!default.ends_by(feb), "DEFAULT is never dropped");
        assert!(default.starts_before(feb), "DEFAULT can hold any time");
    }
}
