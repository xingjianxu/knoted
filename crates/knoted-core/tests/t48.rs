use knoted_core::{
    files::{ObjectId, Sha256Hex, office},
    intelligence::{ExecutionKind, ExecutionRecord, ExecutionRequest},
};
use serde_json::{Value, json};
fn request() -> ExecutionRequest {
    ExecutionRequest {
        execution_id: uuid::Uuid::new_v4(),
        kind: ExecutionKind::Parse,
        input_object_id: ObjectId::parse("opaque-object").unwrap(),
        input_sha256: Sha256Hex::digest(b"Office"),
        source_version: 7,
        deadline: chrono::Utc::now() + chrono::Duration::seconds(900),
        options_revision: office::OPTIONS_REVISION.into(),
    }
}
fn artifact(req: &ExecutionRequest, format: &str) -> Value {
    let (pages, page, location) = match format {
        "docx" => (
            Value::Null,
            Value::Null,
            json!({"path":"body/table:2/row:1/cell:2/paragraph:1"}),
        ),
        "xlsx" => (
            Value::Null,
            Value::Null,
            json!({"sheet":"中文'原名","cell":"XFD1048576"}),
        ),
        _ => (
            json!(601),
            json!(601),
            json!({"path":"shape:1/shape:2/paragraph:1"}),
        ),
    };
    json!({"kind":"parse","input_sha256":req.input_sha256,"source_version":7,"options_revision":office::OPTIONS_REVISION,
        "child_pid":123,"truncated":false,"source_format":format,"title":null,"page_count":pages,"text_length":2,
        "parser":{"parser_revision":office::PARSER_REVISION,"options_revision":office::OPTIONS_REVISION,
        "limits":{"members":4096,"part_bytes":33554432,"expanded_bytes":134217728,"compression_ratio":200,"xml_nodes":500000,"xml_depth":64,"text_bytes":16777216,"cells":100000,"blocks":100000}},
        "blocks":[{"ordinal":0,"type":"paragraph","text":"中文","page":page,"bbox":null,"structure":location.to_string()}]})
}
#[test]
fn t48_1_sheet_name_and_sparse_cell_survive_closed_artifact() {
    let req = request();
    let doc = office::decode_artifact(&serde_json::to_vec(&artifact(&req, "xlsx")).unwrap(), &req)
        .unwrap();
    let loc: Value = serde_json::from_str(doc.blocks[0].structure.as_str().unwrap()).unwrap();
    assert_eq!(loc["sheet"], "中文'原名");
    assert_eq!(loc["cell"], "XFD1048576");
    assert!(office::supports("sheet.XLSX"));
    assert!(!office::supports("legacy.xls"));
}
#[test]
fn t48_2_slide_ordinal_above_pdf_limit_is_valid_but_bbox_is_not() {
    let req = request();
    let mut value = artifact(&req, "pptx");
    let doc = office::decode_artifact(&serde_json::to_vec(&value).unwrap(), &req).unwrap();
    assert_eq!(doc.blocks[0].page, Some(601));
    value["blocks"][0]["bbox"] = json!([0, 1, 2, 3]);
    assert!(office::decode_artifact(&serde_json::to_vec(&value).unwrap(), &req).is_err());
}
#[test]
fn t48_3_no_invented_pages_and_no_invalid_structures() {
    let req = request();
    for format in ["docx", "xlsx"] {
        let mut value = artifact(&req, format);
        assert!(office::decode_artifact(&serde_json::to_vec(&value).unwrap(), &req).is_ok());
        value["page_count"] = json!(1);
        assert!(office::decode_artifact(&serde_json::to_vec(&value).unwrap(), &req).is_err());
    }
    for format in ["docx", "xlsx", "pptx"] {
        let good = artifact(&req, format);
        for (field, bad) in [
            ("source_version", json!(8)),
            ("input_sha256", json!("0".repeat(64))),
            ("options_revision", json!("wrong")),
            ("source_format", json!("pdf")),
            ("text_length", json!(3)),
            ("truncated", json!(true)),
        ] {
            let mut value = good.clone();
            value[field] = bad;
            assert!(
                office::decode_artifact(&serde_json::to_vec(&value).unwrap(), &req).is_err(),
                "{field}"
            );
        }
        for (field, bad) in [
            ("ordinal", json!(1)),
            ("text", json!("")),
            ("page", json!(9999)),
            ("structure", json!("{}")),
            ("structure", json!("not-json")),
        ] {
            let mut value = good.clone();
            value["blocks"][0][field] = bad;
            assert!(
                office::decode_artifact(&serde_json::to_vec(&value).unwrap(), &req).is_err(),
                "{format}:{field}"
            );
        }
        let mut value = good.clone();
        value["parser"]["limits"]["cells"] = json!(100001);
        assert!(office::decode_artifact(&serde_json::to_vec(&value).unwrap(), &req).is_err());
    }
}
#[test]
fn t48_echo_exit_proof_and_expired_office_record_refuse_release() {
    let req = request();
    let value = json!({"execution_id":req.execution_id,"kind":"parse","state":"succeeded","input_sha256":req.input_sha256,"source_version":7,"options_revision":req.options_revision,"compute_exit_confirmed":true,"created_at":chrono::Utc::now(),"started_at":null,"finished_at":chrono::Utc::now(),"updated_at":chrono::Utc::now(),"error":null});
    let record: ExecutionRecord = serde_json::from_value(value.clone()).unwrap();
    record.verify_resource_exit(&req).unwrap();
    let mut late = value.clone();
    late["finished_at"] = json!(req.deadline + chrono::Duration::seconds(1));
    assert!(
        serde_json::from_value::<ExecutionRecord>(late)
            .unwrap()
            .verify_fresh(&req, req.deadline + chrono::Duration::seconds(2))
            .is_err()
    );
    for (key, invalid) in [
        ("compute_exit_confirmed", json!(false)),
        ("source_version", json!(8)),
        ("options_revision", json!("wrong")),
    ] {
        let mut bad = value.clone();
        bad[key] = invalid;
        assert!(
            serde_json::from_value::<ExecutionRecord>(bad)
                .unwrap()
                .verify_resource_exit(&req)
                .is_err()
        );
    }
}

#[test]
fn t48_cancelled_office_record_requires_confirmed_exit() {
    let req = request();
    for proof in [false, true] {
        let value = json!({"execution_id":req.execution_id,"kind":"parse","state":"cancelled","input_sha256":req.input_sha256,"source_version":7,"options_revision":req.options_revision,"compute_exit_confirmed":proof,"created_at":chrono::Utc::now(),"started_at":null,"finished_at":chrono::Utc::now(),"updated_at":chrono::Utc::now(),"error":null});
        let record: ExecutionRecord = serde_json::from_value(value).unwrap();
        assert_eq!(record.verify_resource_exit(&req).is_ok(), proof);
    }
}
