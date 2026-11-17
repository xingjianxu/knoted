//! Direct service checks use the same isolated PostgreSQL migrations as HTTP.
use knoted_core::{
    context::AppContext,
    db::testing::TestDb,
    links::service::{self, ChangeEntityTag, CreateLink, CreateTag, Entity, EntityType, ListTags},
    notes::{
        service::{self as notes, CreateNote, DeleteNote},
        tree::{self, RestoreNote},
    },
};

#[tokio::test]
async fn t32_services_preserve_tag_identity_through_note_delete_and_restore() {
    let db = TestDb::create().await.unwrap();
    let ctx = AppContext::new(db.pool.clone());
    let note = notes::create(
        &ctx,
        CreateNote {
            title: "note".into(),
            parent_id: None,
            document_json: r#"{"blocks":[]}"#.into(),
        },
    )
    .await
    .unwrap();
    let tag = service::create_tag(
        &ctx,
        CreateTag {
            name: " 研究 ".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(tag.name, "研究");
    assert!(tag.created_at <= ctx.now());
    let entity = Entity {
        entity_type: EntityType::Note,
        id: note.id,
    };
    for _ in 0..2 {
        service::assign_tag(
            &ctx,
            ChangeEntityTag {
                entity,
                tag_id: tag.id,
            },
        )
        .await
        .unwrap();
    }
    notes::delete(
        &ctx,
        DeleteNote {
            id: note.id,
            expected_version: 1,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        service::list_tags(
            &ctx,
            ListTags {
                entity: Some(entity),
                limit: None,
                cursor: None
            }
        )
        .await,
        Err(service::DomainError::NotFound)
    ));
    tree::restore(
        &ctx,
        RestoreNote {
            id: note.id,
            expected_version: 2,
            restore_parent: None,
        },
    )
    .await
    .unwrap();
    let tags = service::list_tags(
        &ctx,
        ListTags {
            entity: Some(entity),
            limit: None,
            cursor: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(tags.items.len(), 1);
    assert_eq!(tags.items[0].id, tag.id);
    let link = service::create_link(
        &ctx,
        CreateLink {
            source_type: EntityType::Note,
            source_id: note.id,
            target_type: EntityType::Note,
            target_id: note.id,
        },
    )
    .await
    .unwrap();
    assert_eq!(link.source_id, note.id);
    assert_eq!(link.target_id, note.id);
    db.drop().await;
}
