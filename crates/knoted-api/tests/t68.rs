use std::process::Command;

#[test]
fn t68_admin_backup_documents_its_entry_point() {
    let result = Command::new(env!("CARGO_BIN_EXE_knoted-api"))
        .args(["admin", "backup", "--help"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("admin backup --output"));
}

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use knoted_api::admin::backup::{ASSETS, BackupManifest, DATABASE_DUMP, FORMAT_VERSION, MANIFEST};
use knoted_core::context::AppContext;
use knoted_core::db::{self, testing};
use knoted_core::files::staging::Sha256Hex;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{ConnectOptions, PgPool};
use tempfile::TempDir;
use uuid::Uuid;

struct Fixture {
    directory: TempDir,
    control: PgPool,
    context: AppContext,
    name: String,
    config: PathBuf,
    asset_root: PathBuf,
    owner: Uuid,
}

impl Fixture {
    async fn create() -> Self {
        let options: PgConnectOptions = testing::test_database_url().parse().unwrap();
        assert!(options.get_database().unwrap().ends_with("_test"));
        let control = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .unwrap();
        let name = format!("knoted_t68_{}_test", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE {name} TEMPLATE template0"))
            .execute(&control)
            .await
            .unwrap();
        // On the local NixOS test instance pgvector's paths are per database.
        // Copy only those extension lookup settings to this fresh fixture.
        let extension_settings: Vec<String> = sqlx::query_scalar(
            "SELECT format('ALTER DATABASE %I SET %I TO %L', $1::text, name, setting)
             FROM pg_settings WHERE name IN ('extension_control_path','dynamic_library_path')
             AND source='database'",
        )
        .bind(&name)
        .fetch_all(&control)
        .await
        .unwrap();
        for setting in extension_settings {
            sqlx::query(&setting).execute(&control).await.unwrap();
        }
        let options = options.database(&name);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .unwrap();
        db::migrate(&pool).await.unwrap();
        let owner = Uuid::new_v4();
        sqlx::query("INSERT INTO users(id) VALUES($1)")
            .bind(owner)
            .execute(&pool)
            .await
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("backup.toml");
        let asset_root = directory.path().join("live-assets");
        fs::create_dir(&asset_root).unwrap();
        let connection = options.to_url_lossy().to_string();
        fs::write(
            &config,
            format!(
                "[server]\nbind = \"127.0.0.1:17685\"\n[database]\nurl = {:?}\n[assets]\nroot = {:?}\n",
                connection,
                asset_root.to_str().unwrap()
            ),
        )
        .unwrap();
        Self {
            context: AppContext::new(pool).with_asset_root(&asset_root),
            directory,
            control,
            name,
            config,
            asset_root,
            owner,
        }
    }

    async fn close(self) {
        self.context.admission_pool().close().await;
        self.context.pool().close().await;
        sqlx::query(&format!("DROP DATABASE {}", self.name))
            .execute(&self.control)
            .await
            .unwrap();
    }

    fn backup(&self, output: &Path) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_knoted-api"));
        command
            .args(["admin", "backup", "--config"])
            .arg(&self.config)
            .arg("--output")
            .arg(output)
            .kill_on_drop(true);
        command
    }

    async fn register_asset(&self, id: Uuid, bytes: &[u8]) {
        sqlx::query(
            "INSERT INTO assets(id,owner_id,storage_key,mime,size,sha256,state,created_at)
             VALUES ($1,$2,$3,'image/png',$4,$5,'ready',now())",
        )
        .bind(id)
        .bind(self.owner)
        .bind(id.to_string())
        .bind(bytes.len() as i64)
        .bind(Sha256Hex::digest(bytes).to_string())
        .execute(self.context.pool())
        .await
        .unwrap();
    }
}

const IMAGE: &[u8] = include_bytes!("../../../tests/fixtures/ocr/sample_zh_en.png");

#[tokio::test]
async fn t68_1_missing_image_fails_and_t68_3_releases_write_admission() {
    let fixture = Fixture::create().await;
    let image = Uuid::new_v4();
    fixture.register_asset(image, IMAGE).await;
    let output = fixture.directory.path().join("missing");
    let failed = fixture.backup(&output).output().await.unwrap();
    let incomplete = !output.join(MANIFEST).exists();
    let reopened = fixture.context.try_write().await.unwrap();
    let admitted = reopened.is_some();
    if let Some(guard) = reopened {
        guard.finish().await.unwrap();
    }

    fs::write(fixture.asset_root.join(image.to_string()), IMAGE).unwrap();
    let success_path = fixture.directory.path().join("complete");
    let succeeded = fixture.backup(&success_path).output().await.unwrap();
    let retained = fs::read(success_path.join(MANIFEST));
    let duplicate = fixture.backup(&success_path).output().await.unwrap();
    let unchanged = retained
        .as_ref()
        .is_ok_and(|data| fs::read(success_path.join(MANIFEST)).is_ok_and(|again| *data == again));
    let overlapping_path = fixture.asset_root.join("archive");
    let overlap = fixture.backup(&overlapping_path).output().await.unwrap();
    let source_unchanged = !overlapping_path.exists();
    fixture.close().await;

    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr)
            .contains("could not read and verify registered image")
    );
    assert!(incomplete && admitted);
    assert!(
        succeeded.status.success(),
        "{}",
        String::from_utf8_lossy(&succeeded.stderr)
    );
    assert!(!duplicate.status.success() && unchanged);
    assert!(!overlap.status.success() && source_unchanged);
}

#[tokio::test]
async fn t68_2_real_dump_contains_work_admitted_before_backup() {
    let fixture = Fixture::create().await;
    let image = Uuid::new_v4();
    let writer = fixture.context.try_write().await.unwrap().unwrap();
    let output = fixture.directory.path().join("snapshot");
    let mut backup = fixture.backup(&output);
    let task = tokio::spawn(async move { backup.output().await });
    let queued = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                    SELECT FROM pg_locks
                    WHERE locktype='advisory' AND NOT granted AND mode='ExclusiveLock'
                    AND database=(SELECT oid FROM pg_database WHERE datname=current_database())
                )",
            )
            .fetch_one(fixture.context.pool())
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok();
    let new_write = fixture.context.try_write().await.unwrap();
    let rejected = new_write.is_none();
    if let Some(guard) = new_write {
        guard.finish().await.unwrap();
    }
    fs::write(fixture.asset_root.join(image.to_string()), IMAGE).unwrap();
    fixture.register_asset(image, IMAGE).await;
    writer.finish().await.unwrap();
    let completed = task.await.unwrap().unwrap();

    // Keep the database name, manifest and archive in memory through cleanup.
    let database_name = fixture.name.clone();
    let manifest = fs::read(output.join(MANIFEST));
    let dump = fs::read(output.join(DATABASE_DUMP));
    let permissions = fs::metadata(&output).map(|stat| stat.permissions().mode() & 0o777);
    let copied = fs::read(output.join(ASSETS).join(image.to_string()));
    let restored_rows = Command::new("pg_restore")
        .args(["--data-only", "--table=assets", "--file=-"])
        .arg(output.join(DATABASE_DUMP))
        .output()
        .unwrap();
    fixture.close().await;

    assert!(queued && rejected);
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let manifest: BackupManifest = serde_json::from_slice(&manifest.unwrap()).unwrap();
    let dump = dump.unwrap();
    assert_eq!(manifest.format_version, FORMAT_VERSION);
    assert_eq!(manifest.source.database, database_name);
    assert_eq!(manifest.migrations.len(), db::embedded_migration_count());
    assert_eq!(manifest.database_dump.size, dump.len() as u64);
    assert_eq!(manifest.database_dump.sha256, Sha256Hex::digest(&dump));
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(manifest.assets[0].id, image);
    assert_eq!(manifest.assets[0].sha256, Sha256Hex::digest(IMAGE));
    assert_eq!(copied.unwrap(), IMAGE);
    assert_eq!(permissions.unwrap(), 0o700);
    assert!(restored_rows.status.success());
    assert!(String::from_utf8_lossy(&restored_rows.stdout).contains(&image.to_string()));
}
