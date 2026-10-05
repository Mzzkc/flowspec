use flowspec::{analyze, Config};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[test]
fn every_loss_id_binds_its_entire_actual_semantic_record() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("index.js"),
        "function left(x) { return x; }\nfunction right(x) { return x; }\n",
    )
    .unwrap();
    let mut policy_ids = std::collections::BTreeSet::new();
    for operation in ["source.js::stringReplaceAll", "source.rs::is_nonfinite"] {
        let config = Config { semantic_policy: Some(serde_json::from_value(json!({"version":1,"peers":[{"subjects":["index.js::left","index.js::right"],"required_operations":[operation]}]})).unwrap()), ..Default::default() };
        let result = analyze(d.path(), &config, &[]).unwrap();
        let authority = result.manifest.authority.unwrap();
        let losses = authority["losses"].as_array().unwrap();
        let policy: Vec<_> = losses
            .iter()
            .filter(|l| l["stage"] == "semantic_policy")
            .collect();
        assert_eq!(policy.len(), 2);
        assert_ne!(policy[0]["id"], policy[1]["id"]);
        assert_eq!(authority["execution"], "incomplete");
        for loss in &policy {
            assert_eq!(
                loss["policy"]["peers"][0]["required_operations"][0],
                operation
            );
            assert!(policy_ids.insert(loss["id"].as_str().unwrap().to_owned()));
        }
        for loss in losses {
            let mut meaning: Value = loss.clone();
            meaning.as_object_mut().unwrap().remove("id");
            let mut hash = Sha256::new();
            hash.update(b"flowspec/loss/v1\0");
            hash.update(serde_json::to_vec(&meaning).unwrap());
            assert_eq!(loss["id"], format!("sha256:{:x}", hash.finalize()));
        }
    }
}
