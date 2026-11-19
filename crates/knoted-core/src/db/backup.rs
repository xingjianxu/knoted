//! Inventory and exported PostgreSQL snapshot for the maintenance archive.

use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::files::staging::Sha256Hex;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("backup snapshot query failed")]
    Database(#[from] sqlx::Error),
    #[error("backup schema and embedded migrations differ")]
    Schema,
    #[error("registered image metadata is inconsistent")]
    AssetInventory,
    #[error("a current, historical or trash image reference is unavailable")]
    MissingReference,
}

/// An exported snapshot remains importable for as long as its transaction is open.
pub struct Snapshot {
    tx: Transaction<'static, Postgres>,
    pub id: String,
    pub database: String,
    pub migrations: Vec<(i64, String)>,
    pub assets: Vec<BackupAsset>,
}

pub struct BackupAsset {
    pub id: Uuid,
    pub size: u64,
    pub sha256: Sha256Hex,
}

impl Snapshot {
    /// Begin after maintenance exclusivity has been acquired on a separate pool.
    pub async fn open(pool: &PgPool) -> Result<Self, SnapshotError> {
        let mut tx = pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let (id, database) = sqlx::query_as("SELECT pg_export_snapshot(), current_database()")
            .fetch_one(&mut *tx)
            .await?;
        let applied: Vec<(i64, bool, Vec<u8>)> = sqlx::query_as(
            "SELECT version,success,checksum FROM _sqlx_migrations ORDER BY version",
        )
        .fetch_all(&mut *tx)
        .await?;
        if !super::migrations_match(&applied) {
            return Err(SnapshotError::Schema);
        }
        let migrations = applied
            .into_iter()
            .map(|(version, _, bytes)| {
                let checksum = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                (version, checksum)
            })
            .collect();

        let invalid_reference: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT FROM asset_refs r LEFT JOIN assets a ON a.id=r.asset_id
                WHERE a.state IS DISTINCT FROM 'ready'
            )",
        )
        .fetch_one(&mut *tx)
        .await?;
        if invalid_reference {
            return Err(SnapshotError::MissingReference);
        }
        let inventory: Vec<(Uuid, String, i64, String)> = sqlx::query_as(
            "SELECT id,storage_key,size,sha256 FROM assets WHERE state='ready' ORDER BY id",
        )
        .fetch_all(&mut *tx)
        .await?;
        let assets = inventory
            .into_iter()
            .map(|(id, key, size, sha256)| {
                if key != id.to_string() {
                    return Err(SnapshotError::AssetInventory);
                }
                Ok(BackupAsset {
                    id,
                    size: u64::try_from(size).map_err(|_| SnapshotError::AssetInventory)?,
                    sha256: Sha256Hex::parse(sha256).map_err(|_| SnapshotError::AssetInventory)?,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            tx,
            id,
            database,
            migrations,
            assets,
        })
    }

    /// Finish on the original connection. A disconnected snapshot cannot complete.
    pub async fn finish(self) -> Result<(), SnapshotError> {
        self.tx.commit().await?;
        Ok(())
    }
}

/// libpq settings for the application's host-user connection. The connection
/// string never crosses the subprocess argument or log boundary.
pub fn dump_environment(
    connection: &str,
    options: &PgConnectOptions,
    database: &str,
) -> Option<[(&'static str, String); 5]> {
    let url = url::Url::parse(connection).ok()?;
    if !matches!(url.scheme(), "postgres" | "postgresql") || url.password().is_some() {
        return None;
    }
    if url.query_pairs().any(|(name, _)| {
        !matches!(
            name.as_ref(),
            "host"
                | "port"
                | "user"
                | "dbname"
                | "sslmode"
                | "ssl-mode"
                | "application_name"
                | "statement-cache-capacity"
        )
    }) {
        return None;
    }
    let host = match options.get_socket() {
        Some(path) => path.to_str()?,
        None => options.get_host().trim_matches(['[', ']']),
    };
    let ssl_mode = match options.get_ssl_mode() {
        PgSslMode::Disable => "disable",
        PgSslMode::Allow => "allow",
        PgSslMode::Prefer => "prefer",
        PgSslMode::Require => "require",
        PgSslMode::VerifyCa => "verify-ca",
        PgSslMode::VerifyFull => "verify-full",
    };
    Some([
        ("PGHOST", host.to_owned()),
        ("PGPORT", options.get_port().to_string()),
        ("PGUSER", options.get_username().to_owned()),
        ("PGDATABASE", database.to_owned()),
        ("PGSSLMODE", ssl_mode.to_owned()),
    ])
}
