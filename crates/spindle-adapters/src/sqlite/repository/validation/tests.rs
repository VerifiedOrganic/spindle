use super::*;

#[tokio::test]
async fn deep_check_cache_hits_current_fingerprint_and_sweeps_stale() {
    let (_tmp, repo, _project, _branch, scene_id) = repo_with_scene().await;
    repo.put_deep_check_cache(&scene_id, "fp-a", "temporal_coherence", "output-a")
        .await
        .unwrap();
    assert_eq!(
        repo.get_deep_check_cache(&scene_id, "fp-a", "temporal_coherence")
            .await
            .unwrap()
            .as_deref(),
        Some("output-a")
    );

    repo.put_deep_check_cache(&scene_id, "fp-b", "temporal_coherence", "output-b")
        .await
        .unwrap();
    assert_eq!(
        repo.get_deep_check_cache(&scene_id, "fp-a", "temporal_coherence")
            .await
            .unwrap(),
        None,
        "an edit must miss the previous fingerprint"
    );
    assert_eq!(
        repo.get_deep_check_cache(&scene_id, "fp-b", "temporal_coherence")
            .await
            .unwrap()
            .as_deref(),
        Some("output-b")
    );
}
