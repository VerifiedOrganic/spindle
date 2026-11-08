use super::*;
pub(crate) fn fixture() -> (tempfile::TempDir, TokenizerConfig) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokenizer.json");
    let data=br#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"WhitespaceSplit"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"SYS":1,"USER":2,"ASSISTANT":3,"hello":4,"snow":5},"unk_token":"[UNK]"}}"#;
    std::fs::write(&path, data).unwrap();
    let config = TokenizerConfig {
        tokenizer_json: path,
        tokenizer_sha256: sha256(data),
        model_revision: "fixture-r1".into(),
        framing: ChatFraming {
            prefix: "SYS ".into(),
            between: " USER ".into(),
            suffix: " ASSISTANT".into(),
            trim_system: false,
            trim_user: false,
        },
        calibration_file: dir.path().join("certificate.json"),
        cache_dir: dir.path().join("cache"),
        probe_mode: ProbeMode::LocalZeroOutput,
        tolerance_tokens: 0,
        verify_each_prompt: true,
    };
    (dir, config)
}
#[test]
fn pins_and_full_unicode_content_are_counted_without_truncation() {
    let (_dir, config) = fixture();
    let tokenizer = PinnedTokenizer::load("fixture", config.clone()).unwrap();
    assert_eq!(tokenizer.count_text("hello 雪 🧵").unwrap(), 3);
    assert_eq!(
        tokenizer
            .count_messages(&DraftMessages {
                system: "hello".into(),
                user: "snow".into()
            })
            .unwrap(),
        5
    );
    assert_eq!(tokenizer.count_text(&"snow ".repeat(1000)).unwrap(), 1000);
    let mut bad = config;
    bad.tokenizer_sha256 = "0".repeat(64);
    assert!(PinnedTokenizer::load("fixture", bad).is_err());
}
#[test]
fn framing_never_expands_placeholders_in_content() {
    let (_dir, config) = fixture();
    assert_eq!(
        config.framing.render(&DraftMessages {
            system: "{{user}}".into(),
            user: "{{system}}".into()
        }),
        "SYS {{user}} USER {{system}} ASSISTANT"
    );
}
#[test]
fn calibration_rejects_missing_mismatched_or_stale_samples() {
    let (_dir, config) = fixture();
    let encoder = PinnedTokenizer::load("fixture", config).unwrap();
    assert!(encoder.certificate("fixture-provider", vec![]).is_err());
    let mismatch = DraftMessages {
        system: "hello".into(),
        user: "snow".into(),
    };
    assert!(encoder.verify_count(&mismatch, 4).is_err());
    assert!(encoder.verify_count(&mismatch, 5).is_ok());
}
#[test]
fn nonzero_tolerance_is_explicit_and_bounded() {
    let (_dir, mut config) = fixture();
    config.tolerance_tokens = 1;
    let encoder = PinnedTokenizer::load("fixture", config.clone()).unwrap();
    let message = DraftMessages {
        system: "hello".into(),
        user: "snow".into(),
    };
    assert!(encoder.verify_count(&message, 6).is_ok());
    assert!(encoder.verify_count(&message, 7).is_err());
    config.tolerance_tokens = 17;
    assert!(PinnedTokenizer::load("fixture", config).is_err());
}

#[test]
fn certified_counts_work_offline_and_reject_changed_bindings() {
    let (_dir, config) = fixture();
    let encoder = PinnedTokenizer::load("fixture", config.clone()).unwrap();
    // Independently counted whitespace words, including three framing tokens.
    let samples = calibration_messages()
        .iter()
        .zip([9, 10, 8, 646])
        .map(|(m, n)| ProbeReceipt {
            model: "fixture".into(),
            messages_hash: messages_hash(m).unwrap(),
            input_tokens: n,
            output_tokens: 0,
            model_call_id: format!("probe-{n}"),
        })
        .collect();
    let cert = encoder.certificate("deployment-a", samples).unwrap();
    std::fs::write(&config.calibration_file, serde_json::to_vec(&cert).unwrap()).unwrap();
    let calibrated =
        PinnedTokenizer::load_verified("fixture", config.clone(), "deployment-a").unwrap();
    assert_eq!(
        calibrated
            .count_messages(&DraftMessages {
                system: "hello".into(),
                user: "snow".into()
            })
            .unwrap(),
        5
    );
    assert!(PinnedTokenizer::load_verified("fixture", config.clone(), "deployment-b").is_err());
    let mut changed = cert;
    changed.samples[0].messages_hash = "changed".into();
    assert!(
        PinnedTokenizer::load("fixture", config)
            .unwrap()
            .verify(&changed, "deployment-a")
            .is_err()
    );
}

#[test]
fn assets_with_padding_or_truncation_are_rejected() {
    for (key, value) in [
        (
            "padding",
            serde_json::json!({"strategy":{"Fixed":16},"direction":"Right","pad_to_multiple_of":null,"pad_id":0,"pad_type_id":0,"pad_token":"[UNK]"}),
        ),
        (
            "truncation",
            serde_json::json!({"direction":"Right","max_length":2,"strategy":"LongestFirst","stride":0}),
        ),
    ] {
        let (_dir, mut config) = fixture();
        let mut asset: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&config.tokenizer_json).unwrap()).unwrap();
        asset[key] = value;
        let data = serde_json::to_vec(&asset).unwrap();
        config.tokenizer_sha256 = sha256(&data);
        std::fs::write(&config.tokenizer_json, data).unwrap();
        assert!(PinnedTokenizer::load("fixture", config).is_err());
    }
}
