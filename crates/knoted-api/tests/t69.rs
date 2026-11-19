use knoted_api::admin::restore::RestoreCommand;

#[test]
fn t69_3_requires_explicit_archive_and_target() {
    assert!(RestoreCommand::parse(["--input", "/archive"]).is_err());
    assert!(RestoreCommand::parse(["--input", "/a", "--target", "/t", "--input", "/b"]).is_err());
    let command =
        RestoreCommand::parse(["--target", "/target.toml", "--input", "/archive"]).unwrap();
    assert_eq!(command.input.to_str(), Some("/archive"));
    assert_eq!(command.target.to_str(), Some("/target.toml"));
}

#[test]
fn t69_1_digest_mismatch_rejects_archive() {
    use knoted_api::admin::restore::verify_archive;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir(root.join("assets")).unwrap();
    std::fs::write(root.join("database.dump"), b"PGDMP").unwrap();
    let manifest = serde_json::json!({
        "format_version": 1, "completed_at": "2026-01-01T00:00:00Z", "app_version":"test",
        "artifact_revision":null, "migrations":[], "assets":[],
        "database_dump":{"size":5, "sha256":"0".repeat(64)},
        "source":{"database":"src","rp_id":"localhost","origin":"https://localhost",
        "asset_root":"/src/assets","staging_root":null,"file_root":null,
        "intelligence_origin":"http://127.0.0.1:15603",
        "search":{"enabled":true,"origin":"http://127.0.0.1:15605","document_index":"doc","chunk_index":"chunk"}}
    });
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(verify_archive(root).is_err());
}
