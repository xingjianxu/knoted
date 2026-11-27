//! Restore-time changes are executed against a private staging database only.

use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::str::FromStr;
use uuid::Uuid;

/// Retires cloned runtime capabilities and schedules fresh projections. IDs
/// referenced by domain history are retained, including jobs.
pub async fn retire_source_runtime(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(
        "UPDATE sessions SET revoked_at=now() WHERE revoked_at IS NULL;
         UPDATE ceremonies SET consumed_at=now() WHERE consumed_at IS NULL;
         UPDATE credential_tokens SET consumed_at=now() WHERE consumed_at IS NULL;
         DELETE FROM maintenance_external_submissions;
         DELETE FROM maintenance_external_targets;
         DELETE FROM maintenance_operations;
         UPDATE outbox SET published_at=now() WHERE published_at IS NULL;
         UPDATE jobs SET state='cancelled', lease_token=NULL, lease_expires_at=NULL,
             locked_by=NULL, updated_at=now()
           WHERE state IN ('queued','running','retry_wait','cancel_requested');
         UPDATE jobs SET idempotency_key=NULL WHERE kind IN (
             'fulltext.publish','fulltext.cleanup'
         ) AND idempotency_key IS NOT NULL;
         INSERT INTO outbox(job_id,topic,published_at)
             SELECT id,'restore.retired_job',now() FROM jobs j
             WHERE state IN ('failed','cancelled') AND NOT EXISTS (
               SELECT FROM outbox o WHERE o.job_id=j.id AND o.topic='restore.retired_job');
         DELETE FROM fulltext_publications;
         UPDATE file_sources SET input_object_id=NULL,captured_observation=NULL,
             captured_observation_version=NULL,captured_scan_id=NULL,content_fresh=false;
         UPDATE file_roots SET scan_token=NULL;
         INSERT INTO outbox(topic,payload)
           SELECT topic,jsonb_build_object('source_id',id,'source_version',version)
           FROM (
             SELECT 'note.changed' AS topic,id,content_version AS version FROM notes WHERE deleted_at IS NULL
             UNION ALL SELECT 'project.changed',id,version FROM projects WHERE deleted_at IS NULL
             UNION ALL SELECT 'task.changed',id,version FROM tasks WHERE deleted_at IS NULL
           ) source;"
    ).execute(&mut *tx).await?;
    tx.commit().await
}

pub struct RestoreDatabase {
    control: PgPool,
    options: PgConnectOptions,
    database: String,
    staging: String,
    pub environment: [(&'static str, String); 5],
}

impl RestoreDatabase {
    pub async fn preflight(url: &str, source_database: &str) -> Result<Self, &'static str> {
        let refused = "target database configuration refused";
        let options = PgConnectOptions::from_str(url).map_err(|_| refused)?;
        let name = options.get_database().ok_or(refused)?.to_owned();
        if name == source_database || name.is_empty() || name.len() > 63 || name.contains('\0') {
            return Err(refused);
        }
        let staging = format!("knoted_install_{}", Uuid::new_v4().simple());
        let environment =
            super::backup::dump_environment(url, &options, &staging).ok_or(refused)?;
        let control = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone().database("postgres"))
            .await
            .map_err(|_| refused)?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_database WHERE datname=$1)")
                .bind(&name)
                .fetch_one(&control)
                .await
                .map_err(|_| refused)?;
        if exists {
            return Err("target database already exists");
        }
        Ok(Self {
            control,
            options,
            database: name,
            staging,
            environment,
        })
    }

    pub async fn create(&self) -> Result<(), sqlx::Error> {
        // The staging identifier contains only the static prefix and a random hexadecimal UUID.
        sqlx::query(&format!(
            "CREATE DATABASE {} TEMPLATE template0 ALLOW_CONNECTIONS false",
            self.staging
        ))
        .execute(&self.control)
        .await?;
        sqlx::query(&format!(
            "REVOKE CONNECT ON DATABASE {} FROM PUBLIC",
            self.staging
        ))
        .execute(&self.control)
        .await?;
        sqlx::query(&format!(
            "ALTER DATABASE {} ALLOW_CONNECTIONS true",
            self.staging
        ))
        .execute(&self.control)
        .await?;
        Ok(())
    }

    pub fn staging_name(&self) -> &str {
        &self.staging
    }

    pub async fn pool(&self) -> Result<PgPool, sqlx::Error> {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_with(self.options.clone().database(&self.staging))
            .await
    }

    pub async fn publish(self) -> Result<(), sqlx::Error> {
        // PostgreSQL refuses the rename if another backend is connected or
        // another database has meanwhile claimed the requested name.
        let rename: String = sqlx::query_scalar(
            "SELECT format('ALTER DATABASE %I RENAME TO %I', $1::text, $2::text)",
        )
        .bind(&self.staging)
        .bind(&self.database)
        .fetch_one(&self.control)
        .await?;
        sqlx::query(&rename).execute(&self.control).await?;
        Ok(())
    }
}

/// The staging inventory and projection steps are serial. Acquire the single
/// backend after the final transaction and close it explicitly before rename.
pub async fn close_staging_pool(pool: PgPool) -> Result<(), sqlx::Error> {
    pool.acquire().await?.close().await?;
    pool.close().await;
    Ok(())
}

/// Accept a known migration prefix only after both the manifest and restored
/// migration table have been independently compared to the embedded checksums.
pub async fn migrate_verified(pool: &PgPool, expected: &[(i64, &str)]) -> Result<(), &'static str> {
    let mismatch = "archive migration history does not match this binary";
    let rows: Vec<(i64, bool, Vec<u8>, String)> = sqlx::query_as(
        "SELECT version, success, checksum, encode(checksum,'hex')
         FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| mismatch)?;
    let mut embedded = super::migrator().iter();
    if rows.is_empty() || rows.len() != expected.len() {
        return Err(mismatch);
    }
    for ((version, success, checksum, hex), archived) in rows.iter().zip(expected) {
        let migration = embedded.next().ok_or(mismatch)?;
        if !success
            || (*version, hex.as_str()) != *archived
            || *version != migration.version
            || checksum.as_slice() != migration.checksum.as_ref()
        {
            return Err(mismatch);
        }
    }
    super::migrate(pool)
        .await
        .map_err(|_| "compatible staging schema migration failed")
}
