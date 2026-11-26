//! T22 contract scenarios exercise domain operations without database I/O.

use knoted_core::context::AppContext;
use knoted_core::notes::document::{
    DomainError, ExtractPlainText, PlainTextBlock, PlainTextDocument, ValidateDocument,
    ValidatedDocument, extract_plain_text, validate_document,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;

const COMPLETE: &str = include_str!("../../../tests/fixtures/notes/complete.json");
const UNKNOWN: &str = include_str!("../../../tests/fixtures/notes/unknown-field.json");

fn context() -> AppContext {
    AppContext::new(
        PgPoolOptions::new()
            .connect_lazy("postgres://knoted_test@127.0.0.1:1/knoted_test")
            .unwrap(),
    )
}

fn validate(ctx: &AppContext, raw: &str) -> Result<ValidatedDocument, DomainError> {
    validate_document(
        ctx,
        ValidateDocument {
            format_version: 1,
            document_json: raw.into(),
        },
    )
}

fn extract(ctx: &AppContext, raw: &str) -> PlainTextDocument {
    let document = validate(ctx, raw).expect("valid document");
    extract_plain_text(ctx, ExtractPlainText { document }).unwrap()
}

fn paragraph(id: &str, text: &str) -> Value {
    json!({"id":id,"type":"paragraph","content":[{"type":"text","text":text}]})
}

fn document(block: Value) -> String {
    json!({"blocks":[block]}).to_string()
}

fn rejected(ctx: &AppContext, raw: &str, status: u16) {
    let error = validate(ctx, raw).expect_err("document must be rejected");
    assert_eq!(error.status_code(), status, "{error:?}");
}

#[tokio::test]
async fn table_cells_round_trip_and_extract_in_row_order() {
    let ctx = context();
    let table = json!({
        "id": "table",
        "type": "table",
        "table": {
            "rows": [
                {"cells": [[{"type":"text","text":"标题","marks":["bold"]}], []]},
                {"cells": [[{"type":"text","text":"左"}],
                    [{"type":"link","text":"右","href":"https://example.com/cell"}]]}
            ],
            "header_rows": 1,
            "header_columns": 1,
            "column_widths": [120, null]
        },
        "children": [paragraph("child", "子块")]
    });
    let raw = json!({"blocks": [paragraph("before", "前"), table]}).to_string();
    assert_eq!(validate(&ctx, &raw).unwrap().document_json(), raw);
    let plain = extract(&ctx, &raw);
    assert_eq!(plain.text, "前\n标题\t\n左\t右\n子块");
    assert_eq!(plain.blocks[1].block_id, "table");
    assert_eq!(plain.blocks[1].path, vec![1]);
    assert_eq!(plain.blocks[2].path, vec![1, 0]);
}

#[tokio::test]
async fn table_validation_rejects_invalid_shapes_and_links() {
    let ctx = context();
    for table in [
        json!(null),
        json!({"rows":[]}),
        json!({"rows":[{"cells":[]}]}),
        json!({"rows":[{"cells":[[]]},{"cells":[[],[]]}]}),
        json!({"rows":[{"cells":[[]]}],"header_rows":2}),
        json!({"rows":[{"cells":[[]]}],"header_columns":2}),
        json!({"rows":[{"cells":[[]]}],"column_widths":[1,2]}),
        json!({"rows":[{"cells":[[]]}],"column_widths":[0]}),
        json!({"rows":[{"cells":[[]]}],"column_widths":[10001]}),
        json!({"rows":[{"cells":[[]],"rowspan":2}]}),
        json!({"rows":[{"cells":[[{"type":"link","text":"bad","href":"/relative"}]]}]}),
    ] {
        rejected(
            &ctx,
            &document(json!({"id":"t","type":"table","table":table})),
            422,
        );
    }
    for block in [
        json!({"id":"t","type":"table"}),
        json!({"id":"p","type":"paragraph","table":{"rows":[{"cells":[[]]}]}}),
        json!({"id":"t","type":"table","table":{"rows":[{"cells":[[]]}]},
            "content":[{"type":"text","text":"ambiguous"}]}),
    ] {
        rejected(&ctx, &document(block), 422);
    }
}

#[tokio::test]
async fn table_dimensions_and_document_cell_count_are_bounded() {
    let ctx = context();
    let table = |id: &str, rows: usize, columns: usize| {
        json!({"id":id,"type":"table","table":{"rows":vec![
            json!({"cells":vec![json!([]); columns]}); rows
        ]}})
    };
    let first = table("first", 500, 10);
    let second = table("second", 50, 100);
    let exact = json!({"blocks": [first, second]});
    assert!(validate(&ctx, &exact.to_string()).is_ok());
    let mut oversized = exact;
    oversized["blocks"]
        .as_array_mut()
        .unwrap()
        .push(table("extra", 1, 1));
    rejected(&ctx, &oversized.to_string(), 413);
    rejected(&ctx, &document(table("wide", 1, 101)), 413);
    rejected(&ctx, &document(table("tall", 1001, 1)), 413);
}

#[tokio::test]
async fn t22_1_unknown_format_version_is_422() {
    let ctx = context();
    for version in [0, 2, u32::MAX] {
        let error = validate_document(
            &ctx,
            ValidateDocument {
                format_version: version,
                document_json: "{\"blocks\":[]}".into(),
            },
        )
        .unwrap_err();
        assert!(matches!(error, DomainError::UnsupportedVersion));
        assert_eq!(error.status_code(), 422);
    }
}

#[tokio::test]
async fn t22_2_duplicate_ids_across_levels_are_rejected() {
    let ctx = context();
    rejected(
        &ctx,
        &document(
            json!({"id":"same","type":"bulletListItem","children":[paragraph("same", "child")]}),
        ),
        422,
    );
    rejected(
        &ctx,
        &json!({"blocks":[paragraph("same", "a"),paragraph("same", "b")]}).to_string(),
        422,
    );
}

#[tokio::test]
async fn t22_3_fixture_preserves_chinese_code_and_preorder_locations() {
    let ctx = context();
    let result = extract(&ctx, COMPLETE);
    assert_eq!(
        result.text,
        "中文 <b>原文</b>\n前 链接 后\n一级\n二级\n待办\n\n图注\n  中文();\n\tlet x = 1;  \n"
    );
    let expected = [
        ("标题", vec![0], "中文 <b>原文</b>"),
        ("p", vec![1], "前 链接 后"),
        ("list", vec![2], "一级"),
        ("number", vec![2, 0], "二级"),
        ("check", vec![2, 0, 0], "待办"),
        ("empty", vec![2, 1], ""),
        ("image", vec![3], "图注"),
        ("code", vec![4], "  中文();\n\tlet x = 1;  \n"),
    ];
    assert_eq!(
        result.blocks,
        expected
            .into_iter()
            .map(|(id, path, text)| PlainTextBlock {
                block_id: id.into(),
                path,
                text: text.into()
            })
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn t22_empty_documents_and_blocks_preserve_empty_positions() {
    let ctx = context();
    assert_eq!(
        extract(&ctx, "{\"blocks\":[]}"),
        PlainTextDocument {
            text: String::new(),
            blocks: vec![]
        }
    );
    let result = extract(
        &ctx,
        &json!({"blocks":[paragraph("a", ""), paragraph("b", ""), paragraph("c", "x")]})
            .to_string(),
    );
    assert_eq!(result.text, "\n\nx");
    assert_eq!(result.blocks.len(), 3);
    assert_eq!(result.blocks[1].path, vec![1]);
}

#[tokio::test]
async fn t22_json_limit_counts_utf8_bytes_including_whitespace() {
    let ctx = context();
    let base = document(paragraph("p", "中"));
    let exact = format!("{base}{}", " ".repeat(2 * 1024 * 1024 - base.len()));
    assert!(validate(&ctx, &exact).is_ok());
    rejected(&ctx, &(exact.clone() + " "), 413);
    // The character count stays below the limit while UTF-8 bytes cross it.
    let oversized = document(paragraph("p", &"中".repeat(700_000)));
    assert!(oversized.chars().count() < 2 * 1024 * 1024);
    rejected(&ctx, &oversized, 413);
}

#[tokio::test]
async fn t22_block_count_includes_children_at_5000_boundary() {
    let ctx = context();
    let children = (1..5000)
        .map(|i| paragraph(&i.to_string(), ""))
        .collect::<Vec<_>>();
    let mut root = json!({"id":"root","type":"bulletListItem","children":children});
    assert_eq!(extract(&ctx, &document(root.clone())).blocks.len(), 5000);
    root["children"]
        .as_array_mut()
        .unwrap()
        .push(paragraph("overflow", ""));
    rejected(&ctx, &document(root), 413);
}

fn nested(depth: usize) -> String {
    let mut block = paragraph("leaf", "叶");
    for i in 1..depth {
        block = json!({"id":format!("parent{i}"),"type":"bulletListItem","children":[block]});
    }
    document(block)
}

#[tokio::test]
async fn t22_depth_eight_allowed_nine_rejected() {
    let ctx = context();
    let result = extract(&ctx, &nested(8));
    assert_eq!(result.blocks.len(), 8);
    assert_eq!(result.blocks[7].path, vec![0; 8]);
    rejected(&ctx, &nested(9), 413);
}

#[tokio::test]
async fn t22_extreme_nesting_is_rejected_without_stack_overflow() {
    let ctx = context();
    let raw = format!(
        "{{\"blocks\":[{}{{\"id\":\"leaf\",\"type\":\"paragraph\"}}{}]}}",
        "{\"id\":\"p\",\"type\":\"paragraph\",\"children\":[".repeat(20_000),
        "]}".repeat(20_000)
    );
    rejected(&ctx, &raw, 413);
}

#[tokio::test]
async fn t22_ids_count_unicode_characters() {
    let ctx = context();
    let result = extract(&ctx, &document(paragraph(&"中".repeat(64), "")));
    assert_eq!(result.blocks[0].block_id, "中".repeat(64));
    rejected(&ctx, &document(paragraph(&"中".repeat(65), "")), 422);
    rejected(&ctx, &document(paragraph("", "")), 422);
}

#[tokio::test]
async fn t22_heading_levels_and_image_assets_are_required_and_valid() {
    let ctx = context();
    for level in [1, 2, 3] {
        assert!(
            validate(
                &ctx,
                &document(json!({"id":"h","type":"heading","level":level}))
            )
            .is_ok()
        );
    }
    for level in [json!(0), json!(4), json!(-1), json!(1.5), Value::Null] {
        rejected(
            &ctx,
            &document(json!({"id":"h","type":"heading","level":level})),
            422,
        );
    }
    rejected(&ctx, &document(json!({"id":"h","type":"heading"})), 422);
    for asset in [
        json!("not-a-uuid"),
        json!("https://example.com/image.png"),
        Value::Null,
    ] {
        rejected(
            &ctx,
            &document(json!({"id":"i","type":"image","asset_id":asset})),
            422,
        );
    }
    rejected(&ctx, &document(json!({"id":"i","type":"image"})), 422);
    let result = extract(
        &ctx,
        &document(
            json!({"id":"i","type":"image","asset_id":"550e8400-e29b-41d4-a716-446655440000"}),
        ),
    );
    assert_eq!(result.text, "");
    assert_eq!(result.blocks.len(), 1);
}

#[tokio::test]
async fn t22_links_accept_only_explicit_absolute_safe_schemes() {
    let ctx = context();
    for href in [
        "https://example.com/a",
        "https://[2001:db8::1]/path?q=x#part",
        "HTTPS://example.com",
        "HtTp://example.com/path",
        "MaIlTo:person@example.com",
        "http://xn--fsqu00a.xn--0zwm56d/%E4%B8%AD%E6%96%87",
        "mailto:%E7%94%A8%E6%88%B7@example.com",
    ] {
        let raw = document(
            json!({"id":"p","type":"paragraph","content":[{"type":"link","text":"显示文本","href":href}]}),
        );
        assert_eq!(extract(&ctx, &raw).text, "显示文本");
    }
    for href in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "data:text/html,x",
        "file:///tmp/x",
        "ftp://example.com",
        "/relative",
        "//example.com",
        " https://example.com",
        "https://example.com ",
        "https://exa\nmple.com",
        "https://example.com\t",
        "https://example.com/\u{7f}",
        "https://example.com/\u{0085}",
        "https:example.com",
        "http:/example.com",
        "",
        "mailto:",
    ] {
        rejected(
            &ctx,
            &document(
                json!({"id":"p","type":"paragraph","content":[{"type":"link","text":"x","href":href}]}),
            ),
            422,
        );
    }
    rejected(
        &ctx,
        &document(json!({"id":"p","type":"paragraph","content":[{"type":"link","text":"x"}]})),
        422,
    );
}

#[tokio::test]
async fn t22_unknown_fields_nodes_and_marks_are_not_dropped() {
    let ctx = context();
    for raw in [
        UNKNOWN.to_owned(),
        "{\"blocks\":[],\"extra\":true}".into(),
        document(json!({"id":"b","type":"html","content":[]})),
        document(json!({"id":"b","type":"paragraph","content":[{"type":"html","text":"x"}]})),
        document(
            json!({"id":"b","type":"paragraph","content":[{"type":"text","text":"x","extra":1}]}),
        ),
        document(
            json!({"id":"b","type":"paragraph","content":[{"type":"text","text":"x","marks":["script"]}]}),
        ),
    ] {
        rejected(&ctx, &raw, 422);
    }
}

#[tokio::test]
async fn t22_required_fields_and_wrong_types_are_rejected() {
    let ctx = context();
    for raw in [
        "{}",
        "[]",
        "{\"blocks\":null}",
        "{\"blocks\":{}}",
        "{\"blocks\":[{}]}",
        "{\"blocks\":[{\"id\":\"b\"}]}",
        "{\"blocks\":[{\"type\":\"paragraph\"}]}",
        "{\"blocks\":[{\"id\":7,\"type\":\"paragraph\"}]}",
        "{\"blocks\":[{\"id\":\"b\",\"type\":\"paragraph\",\"content\":[{\"type\":\"text\"}]}]}",
        "{\"blocks\":[{\"id\":\"b\",\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":false}]}]}",
        "{\"blocks\":[]} {}",
        "{",
        "",
    ] {
        rejected(&ctx, raw, 422);
    }
}

#[tokio::test]
async fn t22_optional_fields_reject_explicit_null_and_wrong_types() {
    let ctx = context();
    for field in [
        "children", "content", "checked", "level", "asset_id", "language",
    ] {
        let mut block = paragraph("p", "");
        block[field] = Value::Null;
        rejected(&ctx, &document(block), 422);
    }
    for field in ["href", "marks"] {
        let mut inline = json!({"type":"text","text":"x"});
        inline[field] = Value::Null;
        rejected(
            &ctx,
            &document(json!({"id":"p","type":"paragraph","content":[inline]})),
            422,
        );
    }
    rejected(
        &ctx,
        &document(json!({"id":"p","type":"checkListItem","checked":"yes"})),
        422,
    );
}

#[tokio::test]
async fn t22_duplicate_keys_are_rejected_in_every_structure() {
    let ctx = context();
    for raw in [
        r#"{"blocks":[],"blocks":[]}"#,
        r#"{"blocks":[{"id":"a","id":"b","type":"paragraph"}]}"#,
        r#"{"blocks":[{"id":"a","type":"paragraph","type":"image"}]}"#,
        r#"{"blocks":[{"id":"a","type":"paragraph","children":[],"children":[]}]}"#,
        r#"{"blocks":[{"id":"a","type":"paragraph","level":null,"level":1}]}"#,
        r#"{"blocks":[{"id":"a","type":"paragraph","content":[{"type":"text","text":"a","text":"b"}]}]}"#,
        r#"{"blocks":[{"id":"a","type":"paragraph","content":[{"type":"text","text":"a","marks":[],"marks":[]}]}]}"#,
    ] {
        rejected(&ctx, raw, 422);
    }
}

#[tokio::test]
async fn t22_errors_redact_body_and_input_can_be_fixed_and_revalidated() {
    let ctx = context();
    let raw = UNKNOWN.to_owned();
    let original = raw.clone();
    let error = validate(&ctx, &raw).unwrap_err();
    for message in [error.to_string(), format!("{error:?}")] {
        for secret in ["secret-body", "正文不应出现在错误里", "secret()", "script"] {
            assert!(!message.contains(secret));
        }
        assert!(message.contains('$'));
    }
    assert_eq!(raw, original);
    let mut corrected: Value = serde_json::from_str(&raw).unwrap();
    corrected["blocks"][0]
        .as_object_mut()
        .unwrap()
        .remove("html");
    assert_eq!(
        extract(&ctx, &corrected.to_string()).text,
        "正文不应出现在错误里"
    );
    let error = validate(&ctx, &document(json!({"id":"secret-body","type":"paragraph","content":[{"type":"javascript:secret()","text":"正文不应出现在错误里"}]}))).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("secret"));
}

#[tokio::test]
async fn t22_uri_spelling_must_match_contract_before_url_normalization() {
    let ctx = context();
    for href in [
        "https://例子.测试/中文",
        "mailto:用户@example.com",
        "https://example.com/a b",
        "https://example.com/<script>",
        "https://example.com/\\path",
        "https://example.com/{}",
        "https://example.com/|x",
        "https://example.com/^x",
        "https://example.com/`x",
        "https://example.com/%",
        "https://example.com/%A",
        "https://example.com/%ZZ",
        "https://example.com/a#b#c",
        "https://example.com/[path]",
        "https://example.com/?q=[value]",
        "https://user[name]@example.com/",
        "https://user@@example.com/",
        "https:///example.com/",
        "https:////example.com/",
        "mailto:person@example.com?subject=[value]",
    ] {
        rejected(
            &ctx,
            &document(
                json!({"id":"p","type":"paragraph","content":[{"type":"link","text":"中文显示文本","href":href}]}),
            ),
            422,
        );
    }
    assert_eq!(extract(&ctx, &document(json!({"id":"p","type":"paragraph","content":[{"type":"link","text":"中文显示文本","href":"https://example.com/a%20b?x=%E4%B8%AD#part"}]}))).text, "中文显示文本");
}

#[tokio::test]
async fn t22_asset_ids_require_schema_uuid_spelling() {
    let ctx = context();
    for asset in [
        "550e8400e29b41d4a716446655440000",
        "{550e8400-e29b-41d4-a716-446655440000}",
        "urn:uuid:550e8400-e29b-41d4-a716-446655440000",
        "550e8400-e29b-41d4-a716-44665544000z",
    ] {
        rejected(
            &ctx,
            &document(json!({"id":"i","type":"image","asset_id":asset})),
            422,
        );
    }
    let result = extract(
        &ctx,
        &document(
            json!({"id":"i","type":"image","asset_id":"550E8400-E29B-41D4-A716-446655440000","content":[{"type":"text","text":"图注"}]}),
        ),
    );
    assert_eq!(result.text, "图注");
}
