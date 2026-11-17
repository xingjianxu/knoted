//! Projection and tuple seek only; never acquire structural or row write locks.
use super::{Cursor, EntityType, TrashItem};
use sqlx::PgConnection;

pub(super) async fn page(
    c: &mut PgConnection,
    filter: Option<EntityType>,
    after: Option<&Cursor>,
    limit: u32,
) -> Result<Vec<TrashItem>, sqlx::Error> {
    sqlx::query_as::<_, TrashItem>(
        r#"
        WITH trash AS (
            SELECT 'note'::text AS entity_type, id, title, deleted_at, parent_id,
                   content_version AS version FROM notes WHERE deleted_at IS NOT NULL
            UNION ALL
            SELECT 'project', id, title, deleted_at, NULL::uuid, version
                FROM projects WHERE deleted_at IS NOT NULL
            UNION ALL
            SELECT 'task', id, title, deleted_at, parent_id, version
                FROM tasks WHERE deleted_at IS NOT NULL
        )
        SELECT *, true AS can_restore, NULL::text AS reason FROM trash
        WHERE ($1::text IS NULL OR entity_type=$1)
          AND ($2::timestamptz IS NULL OR deleted_at < $2
               OR (deleted_at=$2 AND (entity_type,id)>($3,$4)))
        ORDER BY deleted_at DESC, entity_type ASC, id ASC LIMIT $5
        "#,
    )
    .bind(filter.map(EntityType::as_str))
    .bind(after.map(|c| c.deleted_at))
    .bind(after.map(|c| c.entity_type.as_str()))
    .bind(after.map(|c| c.id))
    .bind(i64::from(limit) + 1)
    .fetch_all(c)
    .await
}
