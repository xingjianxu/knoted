use super::service::*;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

pub(super) async fn visible(
    c: &mut PgConnection,
    e: Entity,
    write: bool,
) -> Result<(), DomainError> {
    let registered: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM entity_registry WHERE id=$1 AND entity_type=$2)",
    )
    .bind(e.id)
    .bind(e.entity_type.as_str())
    .fetch_one(&mut *c)
    .await?;
    if !registered {
        return Err(DomainError::NotFound);
    }
    // The next entity cards own their business tables. No table means no capability.
    // Files follow the published file_sources present/missing contract, not pathname.
    let (table, predicate, archive) = match e.entity_type {
        EntityType::Note => ("notes", "deleted_at IS NULL", "false"),
        EntityType::File => ("file_sources", "status='present'", "false"),
        EntityType::Project => ("projects", "deleted_at IS NULL", "archived_at IS NOT NULL"),
        EntityType::Task => ("tasks", "deleted_at IS NULL", "archived_at IS NOT NULL"),
    };
    let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(table)
        .fetch_one(&mut *c)
        .await?;
    if !exists {
        return Err(DomainError::NotFound);
    }
    // Row locks serialize visibility checks with deletion/archival in actual modules.
    let archived: Option<bool> = sqlx::query_scalar(&format!(
        "SELECT {archive} FROM {table} WHERE id=$1 AND {predicate} FOR UPDATE"
    ))
    .bind(e.id)
    .fetch_optional(c)
    .await?;
    match archived {
        None => Err(DomainError::NotFound),
        Some(true) if write => Err(DomainError::Conflict {
            current_version: None,
        }),
        Some(_) => Ok(()),
    }
}
pub(super) async fn insert_tag(
    pool: &PgPool,
    id: Uuid,
    name: String,
    now: DateTime<Utc>,
) -> Result<Tag, sqlx::Error> {
    sqlx::query_as(
        "INSERT INTO tags(id,name,created_at,updated_at) VALUES($1,$2,$3,$3) RETURNING *",
    )
    .bind(id)
    .bind(name)
    .bind(now)
    .fetch_one(pool)
    .await
}
pub(super) async fn tag(c: &mut PgConnection, id: Uuid) -> Result<Option<Tag>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM tags WHERE id=$1 AND deleted_at IS NULL FOR UPDATE")
        .bind(id)
        .fetch_optional(c)
        .await
}
pub(super) async fn rename_tag(
    c: &mut PgConnection,
    id: Uuid,
    name: String,
    now: DateTime<Utc>,
) -> Result<Tag, sqlx::Error> {
    sqlx::query_as(
        "UPDATE tags SET name=$2,version=version+1,updated_at=$3 WHERE id=$1 RETURNING *",
    )
    .bind(id)
    .bind(name)
    .bind(now)
    .fetch_one(c)
    .await
}
pub(super) async fn delete_tag(
    c: &mut PgConnection,
    id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tags SET deleted_at=$2,updated_at=$2,version=version+1 WHERE id=$1")
        .bind(id)
        .bind(now)
        .execute(c)
        .await?;
    Ok(())
}
pub(super) async fn tags(
    c: &mut PgConnection,
    entity: Option<Entity>,
    limit: i64,
    after: Option<Uuid>,
) -> Result<Vec<Tag>, sqlx::Error> {
    if let Some(e) = entity {
        sqlx::query_as(&format!("SELECT t.* FROM tags t JOIN {} r ON t.id=r.tag_id WHERE r.entity_id=$1 AND t.deleted_at IS NULL AND ($2::uuid IS NULL OR t.id>$2) ORDER BY t.id LIMIT $3",e.entity_type.tag_table())).bind(e.id).bind(after).bind(limit).fetch_all(c).await
    } else {
        sqlx::query_as("SELECT * FROM tags WHERE deleted_at IS NULL AND ($1::uuid IS NULL OR id>$1) ORDER BY id LIMIT $2").bind(after).bind(limit).fetch_all(c).await
    }
}
pub(super) async fn change_tag(
    c: &mut PgConnection,
    input: ChangeEntityTag,
    assign: bool,
) -> Result<(), sqlx::Error> {
    let table = input.entity.entity_type.tag_table();
    let sql = if assign {
        format!("INSERT INTO {table}(entity_id,tag_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
    } else {
        format!("DELETE FROM {table} WHERE entity_id=$1 AND tag_id=$2")
    };
    sqlx::query(&sql)
        .bind(input.entity.id)
        .bind(input.tag_id)
        .execute(c)
        .await?;
    Ok(())
}
pub(super) async fn insert_link(
    c: &mut PgConnection,
    input: CreateLink,
    now: DateTime<Utc>,
) -> Result<EntityLink, sqlx::Error> {
    sqlx::query_as("INSERT INTO entity_links(id,source_type,source_id,target_type,target_id,created_at) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(source_id,target_id) DO UPDATE SET id=entity_links.id RETURNING *").bind(Uuid::new_v4()).bind(input.source_type.as_str()).bind(input.source_id).bind(input.target_type.as_str()).bind(input.target_id).bind(now).fetch_one(c).await
}
pub(super) async fn links(
    c: &mut PgConnection,
    source: Uuid,
    limit: i64,
    after: Option<Uuid>,
) -> Result<Vec<EntityLink>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM entity_links WHERE source_id=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3").bind(source).bind(after).bind(limit).fetch_all(c).await
}
pub(super) async fn link(
    c: &mut PgConnection,
    id: Uuid,
) -> Result<Option<EntityLink>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM entity_links WHERE id=$1")
        .bind(id)
        .fetch_optional(c)
        .await
}
pub(super) async fn remove_link(c: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM entity_links WHERE id=$1")
        .bind(id)
        .execute(c)
        .await?;
    Ok(())
}
