use knoted_core::{
    config::FilesConfig,
    context::AppContext,
    files::{ObjectId, StagingStore, capture::MAX_INPUT_BYTES, text::extract_text},
};
fn fixture(
    bytes: &[u8],
) -> (
    tempfile::TempDir,
    AppContext,
    knoted_core::files::StagedObject,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = StagingStore::new(dir.path().join("input"), MAX_INPUT_BYTES).unwrap();
    let object = store
        .write(&ObjectId::parse("closed").unwrap(), bytes)
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/knoted_test")
        .unwrap();
    let ctx = AppContext::new(pool).with_files(FilesConfig {
        staging_root: Some(dir.path().into()),
        ..Default::default()
    });
    (dir, ctx, object)
}
#[tokio::test]
async fn t46_1_empty_success() {
    let (_dir, ctx, input) = fixture(b"");
    assert!(extract_text(&ctx, input).await.unwrap().blocks.is_empty());
}
#[tokio::test]
async fn t46_2_illegal_encoding_rejected() {
    for bytes in [&b"\xff"[..], &b"\xff\xfe\x00\xd8"[..], b"a\0b"] {
        let (_dir, ctx, input) = fixture(bytes);
        assert!(extract_text(&ctx, input).await.is_err());
    }
}
#[tokio::test]
async fn t46_3_special_characters_preserved() {
    let original = "  <>&\"中文😀\r\n\nprint('no execution')\n";
    let (_dir, ctx, input) = fixture(original.as_bytes());
    let doc = extract_text(&ctx, input).await.unwrap();
    assert_eq!(
        doc.blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<String>(),
        original
    );
    assert_eq!(doc.blocks[0].structure["line_start"], 1);
}

#[tokio::test]
async fn t46_unicode_boms_and_explicit_errors() {
    use knoted_core::files::text::DomainError;
    for bytes in [
        vec![0xef, 0xbb, 0xbf, b'a'],
        vec![0xff, 0xfe, 0x2d, 0x4e, 0x3d, 0xd8, 0x00, 0xde],
        vec![0xfe, 0xff, 0x4e, 0x2d, 0xd8, 0x3d, 0xde, 0x00],
    ] {
        let (_dir, ctx, input) = fixture(&bytes);
        let doc = extract_text(&ctx, input).await.unwrap();
        assert_eq!(
            doc.blocks[0].text,
            if bytes[0] == 0xef { "a" } else { "中😀" }
        );
    }
    for bytes in [
        vec![0xff],
        vec![0xff, 0xfe, 0x00],
        vec![0xff, 0xfe, 0x00, 0xd8],
        vec![0xc0, 0x80],
    ] {
        let (_dir, ctx, input) = fixture(&bytes);
        assert!(matches!(
            extract_text(&ctx, input).await,
            Err(DomainError::Encoding)
        ));
    }
    for bytes in [vec![0], vec![0xff, 0xfe, 0, 0]] {
        let (_dir, ctx, input) = fixture(&bytes);
        assert!(matches!(
            extract_text(&ctx, input).await,
            Err(DomainError::Nul)
        ));
    }
}
#[tokio::test]
async fn t46_staging_rejects_digest_size_partial_symlink_and_fifo() {
    let (dir, ctx, mut input) = fixture(b"safe");
    input.sha256 = knoted_core::files::Sha256Hex::digest(b"evil");
    assert!(extract_text(&ctx, input.clone()).await.is_err());
    input.sha256 = knoted_core::files::Sha256Hex::digest(b"safe");
    input.size += 1;
    assert!(extract_text(&ctx, input.clone()).await.is_err());
    input.size -= 1;
    let leaf = dir.path().join("input/closed");
    std::fs::rename(&leaf, dir.path().join("input/open.part")).unwrap();
    input.object_id = ObjectId::parse("open.part").unwrap();
    assert!(extract_text(&ctx, input.clone()).await.is_err());
    input.object_id = ObjectId::parse("closed").unwrap();
    std::os::unix::fs::symlink(dir.path().join("input/open.part"), &leaf).unwrap();
    assert!(extract_text(&ctx, input.clone()).await.is_err());
    std::fs::remove_file(&leaf).unwrap();
    rustix::fs::mkfifoat(rustix::fs::CWD, &leaf, rustix::fs::Mode::RUSR).unwrap();
    assert!(extract_text(&ctx, input).await.is_err());
    assert!(ObjectId::parse("../escape").is_err());
}
#[tokio::test]
async fn t46_100mib_many_lines_bounded_and_exact_locations() {
    let bytes = vec![b'\n'; MAX_INPUT_BYTES as usize];
    let (_dir, ctx, input) = fixture(&bytes);
    drop(bytes);
    let doc = extract_text(&ctx, input).await.unwrap();
    assert_eq!(doc.blocks.len(), 1600);
    assert_eq!(
        doc.blocks.iter().map(|b| b.text.len()).sum::<usize>(),
        MAX_INPUT_BYTES as usize
    );
    assert_eq!(doc.blocks[1].structure["line_start"], 65537);
    assert_eq!(
        doc.blocks.last().unwrap().structure["line_end"],
        MAX_INPUT_BYTES + 1
    );
}
#[tokio::test]
async fn t46_locations_whitespace_and_code_are_data() {
    let code = "one\r\n \t\r\ntwo\n<script>throw 'never'</script>\n";
    let (_dir, ctx, input) = fixture(code.as_bytes());
    let doc = extract_text(&ctx, input).await.unwrap();
    let b = &doc.blocks[0];
    assert_eq!(b.text, code);
    assert_eq!(b.structure["paragraph_end"], 2);
    assert_eq!(b.structure["line_end"], 5);
    assert_eq!(b.structure["column_end"], 1);
    for name in [
        "README",
        "Makefile",
        "Dockerfile",
        "Code.RS",
        "x.HTML",
        "x.xml",
    ] {
        assert!(knoted_core::files::text::supports(name));
    }
    for name in ["x.pdf", "x.PNG", "x.docx", "x.xlsx", "x.pptx", "x.bin"] {
        assert!(!knoted_core::files::text::supports(name));
    }
}
