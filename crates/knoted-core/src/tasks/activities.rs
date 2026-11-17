//! Manual follow-up notes, their history and append-only system status events.
//!
//! A manual note is mutable through CAS; every superseded body is copied to
//! `activity_revisions` inside the same transaction, and deleting a note keeps
//! only a marker whose content lives on in that history. System events are
//! appended by the owning task transaction and can never be edited or removed.
use super::service::{DomainError, TaskStatus};
use crate::context::AppContext;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

/// Maximum follow-up body length, matching the published `Activity` schema.
pub const MAX_BODY: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ActivityAuthorKind {
    Manual,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Activity {
    pub id: Uuid,
    pub task_id: Uuid,
    pub author_kind: ActivityAuthorKind,
    pub body: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct ActivityRevision {
    pub id: Uuid,
    pub activity_id: Uuid,
    pub version: i64,
    pub author_kind: ActivityAuthorKind,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct AppendActivity {
    pub task_id: Uuid,
    pub body: String,
}
pub struct EditActivity {
    pub task_id: Uuid,
    pub id: Uuid,
    pub expected_version: i64,
    pub body: String,
}
pub struct DeleteActivity {
    pub task_id: Uuid,
    pub id: Uuid,
    pub expected_version: i64,
}
#[derive(Default)]
pub struct ListActivities {
    pub task_id: Uuid,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}
pub struct ListRevisions {
    pub task_id: Uuid,
    pub activity_id: Uuid,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Serialize)]
pub struct ActivityPage {
    pub items: Vec<Activity>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize)]
pub struct ActivityRevisionPage {
    pub items: Vec<ActivityRevision>,
    pub next_cursor: Option<String>,
}

fn required_nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    scope: String,
    #[serde(deserialize_with = "required_nullable")]
    task_id: Option<Uuid>,
    #[serde(deserialize_with = "required_nullable")]
    activity_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    id: Uuid,
}
impl Cursor {
    fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("cursor fields serialize"))
    }
    fn decode(raw: &str) -> Result<Cursor, DomainError> {
        if raw.len() > 2048 {
            return Err(DomainError::InvalidQuery);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_| DomainError::InvalidQuery)?;
        serde_json::from_slice(&bytes).map_err(|_| DomainError::InvalidQuery)
    }
}
/// Opaque keyset cursor: `(sort_value,id)` descending, bound to its own scope so
/// a cursor from another task, activity or collection is a 400, not a page.
fn cursor_after(
    raw: &str,
    scope: &str,
    task_id: Uuid,
    activity_id: Option<Uuid>,
) -> Result<Option<(DateTime<Utc>, Uuid)>, DomainError> {
    let c = Cursor::decode(raw)?;
    if c.v != 1 || c.scope != scope || c.task_id != Some(task_id) || c.activity_id != activity_id {
        return Err(DomainError::InvalidQuery);
    }
    Ok(Some((c.created_at, c.id)))
}
fn limit(limit: Option<u32>) -> Result<u32, DomainError> {
    let limit = limit.unwrap_or(50);
    if (1..=100).contains(&limit) {
        Ok(limit)
    } else {
        Err(DomainError::InvalidQuery)
    }
}
fn validate_body(body: &str) -> Result<(), DomainError> {
    if body.is_empty() || body.chars().count() > MAX_BODY || body.contains('\0') {
        return Err(DomainError::InvalidField("body"));
    }
    Ok(())
}
fn expected_version(v: i64) -> Result<(), DomainError> {
    if v < 1 {
        Err(DomainError::InvalidField("expected_version"))
    } else {
        Ok(())
    }
}
async fn finish<T>(
    tx: sqlx::Transaction<'_, sqlx::Postgres>,
    r: Result<T, DomainError>,
) -> Result<T, DomainError> {
    match r {
        Ok(v) => {
            tx.commit().await?;
            Ok(v)
        }
        Err(e) => {
            tx.rollback().await?;
            Err(e)
        }
    }
}

/// A follow-up write only targets a live task: a deleted task is gone, while an
/// archived task or an archived/deleted project refuses the write.
async fn live_task(c: &mut PgConnection, id: Uuid) -> Result<(), DomainError> {
    let t = super::repository::find(c, id)
        .await?
        .ok_or(DomainError::NotFound)?;
    if t.deleted_at.is_some() {
        return Err(DomainError::NotFound);
    }
    let conflict = DomainError::Conflict {
        current_version: Some(t.version),
    };
    if t.archived_at.is_some() {
        return Err(conflict);
    }
    if let Some(project_id) = t.project_id
        && !matches!(
            super::repository::project_state(c, project_id).await?,
            Some((false, false))
        )
    {
        return Err(conflict);
    }
    Ok(())
}
/// Follow the task write lock order: relation advisory lock, then the row.
async fn lock_task(c: &mut PgConnection, id: Uuid) -> Result<(), DomainError> {
    crate::projects::repository::lock_relations(c).await?;
    sqlx::query("SELECT id FROM tasks WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(c)
        .await?;
    Ok(())
}
async fn find(
    c: &mut PgConnection,
    task_id: Uuid,
    id: Uuid,
) -> Result<Option<Activity>, sqlx::Error> {
    sqlx::query_as::<_, Activity>("SELECT * FROM activities WHERE id=$1 AND task_id=$2")
        .bind(id)
        .bind(task_id)
        .fetch_optional(c)
        .await
}
async fn history(c: &mut PgConnection, old: &Activity) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO activity_revisions(id,activity_id,version,author_kind,body,created_at)
         VALUES($1,$2,$3,$4,$5,$6)",
    )
    .bind(Uuid::new_v4())
    .bind(old.id)
    .bind(old.version)
    .bind(old.author_kind)
    .bind(&old.body)
    .bind(old.updated_at)
    .execute(c)
    .await?;
    Ok(())
}
/// Append a manual follow-up note to a live task.
pub async fn append(ctx: &AppContext, input: AppendActivity) -> Result<Activity, DomainError> {
    validate_body(&input.body)?;
    let now = ctx.now();
    let mut tx = ctx.pool().begin().await?;
    let r = async {
        lock_task(&mut tx, input.task_id).await?;
        live_task(&mut tx, input.task_id).await?;
        let a = sqlx::query_as::<_, Activity>(
            "INSERT INTO activities(id,task_id,author_kind,body,version,created_at,updated_at)
             VALUES($1,$2,'manual',$3,1,$4,$4) RETURNING *",
        )
        .bind(Uuid::new_v4())
        .bind(input.task_id)
        .bind(&input.body)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        Ok(a)
    }
    .await;
    finish(tx, r).await
}

/// Revise a manual note: the superseded body moves to history in this transaction.
pub async fn edit(ctx: &AppContext, input: EditActivity) -> Result<Activity, DomainError> {
    expected_version(input.expected_version)?;
    validate_body(&input.body)?;
    let now = ctx.now();
    let mut tx = ctx.pool().begin().await?;
    let r = async {
        lock_task(&mut tx, input.task_id).await?;
        live_task(&mut tx, input.task_id).await?;
        let old = find(&mut tx, input.task_id, input.id)
            .await?
            .ok_or(DomainError::NotFound)?;
        if old.author_kind == ActivityAuthorKind::System {
            return Err(DomainError::Conflict {
                current_version: Some(old.version),
            });
        }
        if old.deleted_at.is_some() {
            return Err(DomainError::NotFound);
        }
        if old.version != input.expected_version {
            return Err(DomainError::Conflict {
                current_version: Some(old.version),
            });
        }
        // CAS before any history row: a loser of a concurrent edit must fail
        // with a conflict instead of colliding on the history uniqueness.
        let edited = sqlx::query_as::<_, Activity>(
            "UPDATE activities SET body=$3,updated_at=$4,version=version+1
             WHERE id=$1 AND task_id=$2 AND version=$5 AND author_kind='manual'
             AND deleted_at IS NULL RETURNING *",
        )
        .bind(input.id)
        .bind(input.task_id)
        .bind(&input.body)
        .bind(now)
        .bind(old.version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DomainError::Conflict {
            current_version: Some(old.version),
        })?;
        history(&mut tx, &old).await?;
        Ok(edited)
    }
    .await;
    finish(tx, r).await
}

/// Delete a manual note: the row stays as a marker and the final body becomes history.
pub async fn delete(ctx: &AppContext, input: DeleteActivity) -> Result<(), DomainError> {
    expected_version(input.expected_version)?;
    let now = ctx.now();
    let mut tx = ctx.pool().begin().await?;
    let r = async {
        lock_task(&mut tx, input.task_id).await?;
        live_task(&mut tx, input.task_id).await?;
        let old = find(&mut tx, input.task_id, input.id)
            .await?
            .ok_or(DomainError::NotFound)?;
        if old.author_kind == ActivityAuthorKind::System {
            return Err(DomainError::Conflict {
                current_version: Some(old.version),
            });
        }
        if old.deleted_at.is_some() {
            return Err(DomainError::NotFound);
        }
        if old.version != input.expected_version {
            return Err(DomainError::Conflict {
                current_version: Some(old.version),
            });
        }
        sqlx::query_as::<_, Activity>(
            "UPDATE activities SET deleted_at=$3,updated_at=$3,version=version+1
             WHERE id=$1 AND task_id=$2 AND version=$4 AND author_kind='manual'
             AND deleted_at IS NULL RETURNING *",
        )
        .bind(input.id)
        .bind(input.task_id)
        .bind(now)
        .bind(old.version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DomainError::Conflict {
            current_version: Some(old.version),
        })?;
        history(&mut tx, &old).await?;
        Ok(())
    }
    .await;
    finish(tx, r).await
}

/// Newest first; deleted notes are gone from the list but stay in history.
pub async fn list(ctx: &AppContext, input: ListActivities) -> Result<ActivityPage, DomainError> {
    let limit = limit(input.limit)?;
    let after = input
        .cursor
        .as_deref()
        .map(|raw| cursor_after(raw, "task_activities", input.task_id, None))
        .transpose()?
        .flatten();
    let mut c = ctx.pool().acquire().await?;
    let task = super::repository::find(&mut c, input.task_id)
        .await?
        .ok_or(DomainError::NotFound)?;
    if task.deleted_at.is_some() {
        return Err(DomainError::NotFound);
    }
    let mut items = sqlx::query_as::<_, Activity>(
        "SELECT * FROM activities WHERE task_id=$1 AND deleted_at IS NULL
         AND ($2::timestamptz IS NULL OR (created_at,id) < ($2,$3))
         ORDER BY created_at DESC,id DESC LIMIT $4",
    )
    .bind(input.task_id)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1))
    .bind(i64::from(limit) + 1)
    .fetch_all(&mut *c)
    .await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|a| {
            Cursor {
                v: 1,
                scope: "task_activities".into(),
                task_id: Some(input.task_id),
                activity_id: None,
                created_at: a.created_at,
                id: a.id,
            }
            .encode()
        })
    } else {
        None
    };
    Ok(ActivityPage { items, next_cursor })
}

/// History of one note, including a deleted note's former bodies.
pub async fn list_revisions(
    ctx: &AppContext,
    input: ListRevisions,
) -> Result<ActivityRevisionPage, DomainError> {
    let limit = limit(input.limit)?;
    let after = input
        .cursor
        .as_deref()
        .map(|raw| {
            cursor_after(
                raw,
                "activity_revisions",
                input.task_id,
                Some(input.activity_id),
            )
        })
        .transpose()?
        .flatten();
    let mut c = ctx.pool().acquire().await?;
    let task = super::repository::find(&mut c, input.task_id)
        .await?
        .ok_or(DomainError::NotFound)?;
    if task.deleted_at.is_some() {
        return Err(DomainError::NotFound);
    }
    if find(&mut c, input.task_id, input.activity_id)
        .await?
        .is_none()
    {
        return Err(DomainError::NotFound);
    }
    let mut items = sqlx::query_as::<_, ActivityRevision>(
        "SELECT * FROM activity_revisions WHERE activity_id=$1
         AND ($2::timestamptz IS NULL OR (created_at,id) < ($2,$3))
         ORDER BY created_at DESC,id DESC LIMIT $4",
    )
    .bind(input.activity_id)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1))
    .bind(i64::from(limit) + 1)
    .fetch_all(&mut *c)
    .await?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if more {
        items.last().map(|r| {
            Cursor {
                v: 1,
                scope: "activity_revisions".into(),
                task_id: Some(input.task_id),
                activity_id: Some(input.activity_id),
                created_at: r.created_at,
                id: r.id,
            }
            .encode()
        })
    } else {
        None
    };
    Ok(ActivityRevisionPage { items, next_cursor })
}

/// Human-readable system body. The state names come from the same serialization
/// the public task DTO uses, so the two can never drift apart.
pub(super) fn status_event(old: TaskStatus, new: TaskStatus) -> String {
    fn name(s: TaskStatus) -> String {
        serde_json::to_value(s)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    }
    format!("status: {} -> {}", name(old), name(new))
}

/// Append-only system event, written by the task transaction that caused it.
/// It carries no CAS version of its own: version 1 is final.
///
/// Follow-up records deliberately write no outbox row: the search projection
/// (T53/T57) owns that protocol, and until it exists a new topic would only
/// manufacture unpublished events. Deleting a note already withdraws it from
/// every current read path and keeps it readable through history.
pub(super) async fn append_system(
    c: &mut PgConnection,
    task_id: Uuid,
    body: String,
    at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO activities(id,task_id,author_kind,body,version,created_at,updated_at)
         VALUES($1,$2,'system',$3,1,$4,$4)",
    )
    .bind(Uuid::new_v4())
    .bind(task_id)
    .bind(&body)
    .bind(at)
    .execute(&mut *c)
    .await?;
    Ok(())
}
