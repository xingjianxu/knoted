//! Conservative identity matching and atomic user metadata transfer.
use super::capture::SourceObservation;
use sqlx::{PgConnection, types::Json};
use uuid::Uuid;

/// Called only while capture holds the current complete root/scan lock. A hash
/// confirms content, never identity. All candidates and observations are scoped
/// to that root and scan; the opaque handle must also prove incarnation.
pub(crate) async fn match_move(
    conn: &mut PgConnection,
    input: &SourceObservation,
    sha256: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    let m = &input.metadata;
    let Some(handle) = &m.identity.handle else {
        return Ok(None);
    };
    if m.nlink != 1 {
        return Ok(None);
    }
    let candidates: Vec<Uuid> = sqlx::query_scalar(
        "SELECT s.id FROM file_sources s WHERE s.root_id=$1 AND s.move_scan_id=$2
         AND s.status='missing' AND s.device=$3 AND s.inode=$4 AND s.nlink=1
         AND s.opaque_handle=$5 AND s.sha256=$6
         AND NOT EXISTS(SELECT 1 FROM file_observations o WHERE o.scan_id=$2 AND o.relative_path=s.relative_path)
         AND (SELECT count(*) FROM file_observations o WHERE o.scan_id=$2
              AND (o.metadata->'identity'->>'device')::numeric=$3
              AND (o.metadata->'identity'->>'inode')::numeric=$4)=1
         AND (SELECT count(*) FROM file_sources other WHERE other.root_id=$1
              AND (other.status='present' OR other.move_scan_id=$2)
              AND other.device=$3 AND other.inode=$4)=1
         ORDER BY s.id FOR UPDATE")
        .bind(input.root_id).bind(input.scan_id).bind(m.identity.device as i64)
        .bind(m.identity.inode as i64).bind(Json(handle)).bind(sha256)
        .fetch_all(&mut *conn).await?;
    Ok(if candidates.len() == 1 {
        Some(candidates[0])
    } else {
        None
    })
}

use crate::context::AppContext;
pub use crate::links::service::DomainError;
use chrono::{DateTime, Utc};
use serde::Serialize;
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FileSource {
    pub id: Uuid,
    pub relative_path: String,
    pub name: String,
    pub size: i64,
    pub sha256: String,
    pub content_version: i64,
    pub version: i64,
    pub status: String,
    pub mime: Option<String>,
    pub note: Option<String>,
    pub tag_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
}
pub struct RelinkSource {
    pub id: Uuid,
    pub new_source_id: Uuid,
    pub expected_version: i64,
}
#[derive(sqlx::FromRow)]
struct RelinkRow {
    id: Uuid,
    root_id: Uuid,
    version: i64,
    status: String,
    note: Option<String>,
}

#[derive(sqlx::FromRow)]
struct RelinkRoot {
    id: Uuid,
    configured_path: String,
    identity: Json<super::safe_fs::Identity>,
}
impl RelinkRoot {
    fn validate(&self) -> Result<(), DomainError> {
        let actual = super::safe_fs::open_root(std::path::Path::new(&self.configured_path))
            .and_then(super::safe_fs::metadata)
            .map(|m| m.identity);
        if actual.ok().as_ref() != Some(&self.identity.0) {
            return Err(DomainError::Conflict {
                current_version: None,
            });
        }
        Ok(())
    }
}
pub async fn relink_source(
    ctx: &AppContext,
    input: RelinkSource,
) -> Result<FileSource, DomainError> {
    if input.id == input.new_source_id || input.expected_version < 1 {
        return Err(DomainError::InvalidField);
    }
    let mut tx = ctx.pool().begin().await?;
    let result=async {
        crate::notes::repository::lock_tree(&mut tx).await?;
        crate::projects::repository::lock_relations(&mut tx).await?;
        // Scanner/capture use root -> source. Never acquire a root after a file
        // row, even though relationship admission already serializes user writes.
        let root:Option<RelinkRoot>=sqlx::query_as("SELECT id,configured_path,identity FROM file_roots WHERE singleton AND identity IS NOT NULL AND paused_reason IS NULL AND scan_token IS NULL FOR UPDATE")
            .fetch_optional(&mut *tx).await?;
        let root=root.ok_or(DomainError::Conflict{current_version:None})?;
        root.validate()?;
        let rows:Vec<RelinkRow>=sqlx::query_as("SELECT id,root_id,version,status,note FROM file_sources WHERE id=ANY($1) ORDER BY id FOR UPDATE")
            .bind(vec![input.id,input.new_source_id]).fetch_all(&mut *tx).await?;
        let old=rows.iter().find(|r|r.id==input.id).ok_or(DomainError::NotFound)?;
        let new=rows.iter().find(|r|r.id==input.new_source_id).ok_or(DomainError::NotFound)?;
        if old.version!=input.expected_version {return Err(DomainError::Conflict{current_version:Some(old.version)});}
        if old.root_id!=root.id || new.root_id!=root.id || new.status!="present" {
            return Err(DomainError::Conflict{current_version:Some(old.version)});
        }
        if let (Some(a),Some(b))=(&old.note,&new.note) && a!=b {
            return Err(DomainError::Conflict{current_version:Some(old.version)});
        }
        let (old_relations,new_relations)=crate::links::transfer_file_metadata(&mut tx,old.id,new.id).await?;
        let old_changed=old_relations || old.note.is_some();
        let new_changed=new_relations || (old.note.is_some() && new.note.is_none());
        if old_changed {
            sqlx::query("UPDATE file_sources SET note=NULL,version=version+1,updated_at=$2 WHERE id=$1")
                .bind(old.id).bind(ctx.now()).execute(&mut *tx).await?;
        }
        if new_changed {
            sqlx::query("UPDATE file_sources SET note=COALESCE(note,$2),version=version+1,updated_at=$3 WHERE id=$1")
                .bind(new.id).bind(&old.note).bind(ctx.now()).execute(&mut *tx).await?;
        }
        let mut source:FileSource=sqlx::query_as("SELECT s.id,s.relative_path,''::text AS name,s.size,s.sha256,s.content_version,s.version,s.status,s.mime,s.note,s.created_at,s.updated_at,s.last_seen_at,ARRAY(SELECT ft.tag_id FROM file_tags ft JOIN tags t ON t.id=ft.tag_id WHERE ft.entity_id=s.id AND t.deleted_at IS NULL ORDER BY ft.tag_id) AS tag_ids FROM file_sources s WHERE s.id=$1")
            .bind(new.id).fetch_one(&mut *tx).await?;
        source.name=source.relative_path.rsplit('/').next().unwrap_or_default().to_owned();
        root.validate()?;
        Ok(source)
    }.await;
    match result {
        Ok(source) => {
            tx.commit().await?;
            Ok(source)
        }
        Err(error) => {
            tx.rollback().await?;
            Err(error)
        }
    }
}
