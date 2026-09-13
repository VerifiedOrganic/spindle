//! Fiction anti-slop scanner (Phase 1).
//!
//! Loads the Phase 0 shelf pack (`references/anti-slop-shelf-pack.v0.toml`)
//! and the human catalog (`references/anti-slop.md`). Soft shelves stay
//! warnings and never increment [`AntiSlopReport::hard_count`]. Hard shelves
//! increment `hard_count` only when they exceed the pack limit.
//!
//! Fiction-only. Does not port Voices / BLUF / delve-as-tech-gate / Flesch /
//! detector / em-dash gates.
//!
//! Rewrite directions are rewrite-from-beats (≤1–2), not paraphrase-humanizer.
//! Persist path for [`AntiSlopReport`] is sketched; this phase does not write it.

mod pack;
mod packet;
mod scan;

pub use pack::{
    AntislopError, DEFAULT_CATALOG_MARKDOWN, DEFAULT_PACK_TOML, GenreOverride, PackPolicy,
    RewriteMode, Severity, ShelfLimit, ShelfPack, ShelfSpec,
};
pub use packet::{
    CompactShelfDigest, CompactShelfEntry, ProfileOverlay, SceneNegative, VoiceSample,
    WritingPacketHooks, assemble_writing_packet, compact_shelf_digest,
    genre_override_from_style_texts, render_compact_shelf_digest_markdown,
    render_writing_packet_hooks_markdown, writing_packet_hooks_from_guidance,
};
pub use scan::{AntiSlopHit, AntiSlopReport, ScanInput, ScanSurface, persist_path_sketch, scan};

/// Tech / Voices gates that must not appear as fiction shelves.
pub const NON_PORTS: &[&str] = &[
    "bluf",
    "tech_buzzlist",
    "em_dash_gate",
    "flesch",
    "detector_percent",
    "faq_ending",
    "delve_as_tech_gate",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn fiction(prose: &str) -> ScanInput<'_> {
        ScanInput::fiction(prose)
    }

    #[test]
    fn default_pack_loads_twelve_fiction_shelves() {
        let pack = ShelfPack::load_default().expect("embedded v0 pack");
        assert_eq!(pack.schema_version, 0);
        assert_eq!(pack.pack_id, "fiction.default");
        assert_eq!(pack.domain, "fiction");
        assert_eq!(pack.shelves.len(), 12);
        assert!(pack.policy.fiction_only);
        assert!(pack.policy.said_bookism_soft_only_by_default);
        assert!(pack.policy.max_rewrite_from_beats <= 2);
        assert_eq!(
            pack.shelf("solitary_fade").map(|s| s.severity),
            Some(Severity::Soft)
        );
        assert_eq!(
            pack.shelf("said_bookism").map(|s| s.severity),
            Some(Severity::Soft)
        );
        assert_eq!(
            pack.shelf("contrast_not_x_but_y").map(|s| s.severity),
            Some(Severity::Hard)
        );
        assert!(DEFAULT_CATALOG_MARKDOWN.contains("solitary_fade"));
        assert!(DEFAULT_CATALOG_MARKDOWN.contains("Rewrite from the beat."));
    }

    #[test]
    fn said_bookism_stays_soft_under_product_lock_even_if_pack_says_hard() {
        let mut pack = ShelfPack::load_default().expect("pack");
        pack.shelf_mut("said_bookism").unwrap().severity = Severity::Hard;
        let prose =
            "\"Leave,\" she hissed.\n\"Please,\" he breathed.\n\"Impossible,\" she gasped.\n";
        let report = scan(&pack, &fiction(prose));
        let bookism: Vec<_> = report
            .hits
            .iter()
            .filter(|hit| hit.shelf_id == "said_bookism")
            .collect();
        assert!(
            !bookism.is_empty(),
            "expected over-limit said_bookism hits: {report:?}"
        );
        assert!(bookism.iter().all(|hit| hit.severity == Severity::Soft));
        assert_eq!(report.hard_count, 0);
    }

    #[test]
    fn solitary_fade_hits_are_soft_and_do_not_increment_hard_count() {
        let pack = ShelfPack::load_default().expect("pack");
        let prose = "She spent the afternoon thinking about what he'd said.\n\n\
                     Hours passed.\n\n\
                     Later, at the meeting, she told him everything.";
        let report = scan(&pack, &fiction(prose));
        let fades: Vec<_> = report
            .hits
            .iter()
            .filter(|hit| hit.shelf_id == "solitary_fade")
            .collect();
        assert!(
            !fades.is_empty(),
            "expected solitary_fade hits, got {report:?}"
        );
        assert!(fades.iter().all(|hit| hit.severity == Severity::Soft));
        assert_eq!(
            report.hard_count, 0,
            "solitary_fade must not contribute to hard_count: {report:?}"
        );
        assert!(report.soft_count >= 1);
        assert!(
            fades
                .iter()
                .all(|hit| hit.rewrite == RewriteMode::FromBeats)
        );
    }

    #[test]
    fn hard_over_limit_increments_hard_count() {
        let pack = ShelfPack::load_default().expect("pack");
        // contrast limit 1: two hinges => 1 hard finding
        // emotion_cocktail limit 0: one mix => 1 hard finding
        let prose = "It wasn't anger. It was disappointment.\n\
                     This wasn't a homecoming. It was a reckoning.\n\
                     She felt a mix of relief and dread when the letter came.";
        let report = scan(&pack, &fiction(prose));
        assert!(
            report.hits.iter().any(|hit| hit.shelf_id == "contrast_not_x_but_y"
                && hit.severity == Severity::Hard),
            "{report:?}"
        );
        assert!(
            report
                .hits
                .iter()
                .any(|hit| hit.shelf_id == "emotion_cocktail" && hit.severity == Severity::Hard),
            "{report:?}"
        );
        assert_eq!(report.hard_count, 2, "{report:?}");
    }

    #[test]
    fn one_allowed_contrast_does_not_increment_hard_count() {
        let pack = ShelfPack::load_default().expect("pack");
        let prose = "It wasn't anger. It was disappointment. He set the cup down.";
        let report = scan(&pack, &fiction(prose));
        assert!(
            report
                .hits
                .iter()
                .all(|hit| hit.shelf_id != "contrast_not_x_but_y"),
            "within-limit contrast should stay silent: {report:?}"
        );
        assert_eq!(report.hard_count, 0);
    }

    #[test]
    fn fishing_ending_is_hard_at_the_close() {
        let pack = ShelfPack::load_default().expect("pack");
        let prose = "She locked the till and stood in the doorway.\n\n\
                     She didn't know what tomorrow would bring.";
        let report = scan(&pack, &fiction(prose));
        assert!(
            report
                .hits
                .iter()
                .any(|hit| hit.shelf_id == "fishing_ending" && hit.severity == Severity::Hard),
            "{report:?}"
        );
        assert!(report.hard_count >= 1);
    }

    #[test]
    fn non_fiction_surface_is_skipped() {
        let pack = ShelfPack::load_default().expect("pack");
        let prose = "It wasn't a design doc. It was a reckoning.\n\
                     A mix of hope and terror. Only time would tell.";
        let report = scan(
            &pack,
            &ScanInput {
                prose,
                surface: ScanSurface::Other,
            },
        );
        assert!(report.skipped);
        assert_eq!(report.hard_count, 0);
        assert_eq!(report.soft_count, 0);
        assert!(report.hits.is_empty());
    }

    #[test]
    fn non_ports_are_not_shelves_and_delve_is_not_a_gate() {
        let pack = ShelfPack::load_default().expect("pack");
        for id in NON_PORTS {
            assert!(pack.shelf(id).is_none(), "{id} must not be a fiction shelf");
        }
        let prose = "She wanted to delve into the cellar ledgers.\n\
                     Leverage? Synergy? Unlock the FAQ.\n\
                     Readability grade: easy. Detector score: none.";
        let report = scan(&pack, &fiction(prose));
        assert!(
            report
                .hits
                .iter()
                .all(|hit| !NON_PORTS.contains(&hit.shelf_id.as_str())),
            "non-port gates must not fire: {report:?}"
        );
        assert!(
            report
                .hits
                .iter()
                .all(|hit| hit.shelf_id != "naming_watchlist"
                    || !hit.excerpt.to_lowercase().contains("delve")),
            "delve must not be a naming_watchlist tech gate: {report:?}"
        );
    }

    #[test]
    fn rewrite_is_from_beats_not_paraphrase() {
        let pack = ShelfPack::load_default().expect("pack");
        assert_eq!(pack.policy.max_rewrite_from_beats, 2);
        assert!(
            pack.shelves
                .iter()
                .all(|s| s.rewrite == RewriteMode::FromBeats)
        );
        let report = scan(
            &pack,
            &fiction("She felt a mix of relief and dread.\nHours passed."),
        );
        assert!(!report.hits.is_empty());
        assert!(
            report
                .hits
                .iter()
                .all(|hit| hit.rewrite == RewriteMode::FromBeats)
        );
        assert!(report.rewrite_max_passes <= 2);
        assert!(report.hits.iter().all(|hit| {
            hit.rewrite_hint.contains("beat") && !hit.rewrite_hint.contains("paraphrase")
        }));
    }

    #[test]
    fn persist_path_is_sketched_not_wired() {
        let path = persist_path_sketch("proj_1", "branch_main", "scene_9");
        assert!(path.contains("anti_slop_report.json"));
        assert!(path.contains("scene_9"));
    }

    #[test]
    fn compact_shelf_digest_lists_twelve_fiction_shelves_with_effective_severity() {
        let pack = ShelfPack::load_default().expect("pack");
        let digest = compact_shelf_digest(&pack, None);
        assert_eq!(digest.pack_id, "fiction.default");
        assert_eq!(digest.catalog_uri, "bible://references/anti-slop");
        assert_eq!(digest.rewrite_max_passes, 2);
        assert_eq!(digest.shelves.len(), 12);
        assert!(digest.shelves.iter().any(|s| s.id == "solitary_fade"
            && s.severity == Severity::Soft
            && s.enabled
            && s.profile_overlay.is_none()));
        assert!(
            digest
                .shelves
                .iter()
                .any(|s| s.id == "said_bookism" && s.severity == Severity::Soft && s.enabled)
        );
        assert!(
            digest
                .shelves
                .iter()
                .any(|s| s.id == "contrast_not_x_but_y" && s.severity == Severity::Hard)
        );
        for id in NON_PORTS {
            assert!(
                digest.shelves.iter().all(|s| s.id != *id),
                "{id} must not appear in the fiction digest"
            );
        }
        let markdown = render_compact_shelf_digest_markdown(&digest);
        assert!(markdown.contains("compact_shelf_digest"));
        assert!(markdown.contains("solitary_fade"));
        assert!(markdown.contains("bible://references/anti-slop"));
        assert!(markdown.contains("from-beats"));
        assert!(markdown.contains("not a paraphrase-humanizer"));
        assert!(!markdown.contains("BLUF"));
        assert!(!markdown.contains("Flesch"));
    }

    #[test]
    fn compact_shelf_digest_applies_profile_disable_soften_promote() {
        let pack = ShelfPack::load_default().expect("pack");
        let overlay = genre_override_from_style_texts(
            "style_profile:comedy",
            &[
                "disable fishing_ending".to_string(),
                "soften contrast_not_x_but_y".to_string(),
                "promote said_bookism".to_string(),
            ],
        );
        assert_eq!(overlay.profile, "style_profile:comedy");
        assert_eq!(overlay.disable, vec!["fishing_ending".to_string()]);
        assert_eq!(overlay.soften, vec!["contrast_not_x_but_y".to_string()]);
        assert_eq!(overlay.promote_to_hard, vec!["said_bookism".to_string()]);

        let digest = compact_shelf_digest(&pack, Some(&overlay));
        let fishing = digest
            .shelves
            .iter()
            .find(|s| s.id == "fishing_ending")
            .expect("fishing_ending");
        assert!(!fishing.enabled);
        assert_eq!(fishing.profile_overlay, Some(ProfileOverlay::Disabled));

        let contrast = digest
            .shelves
            .iter()
            .find(|s| s.id == "contrast_not_x_but_y")
            .expect("contrast");
        assert!(contrast.enabled);
        assert_eq!(contrast.severity, Severity::Soft);
        assert_eq!(contrast.profile_overlay, Some(ProfileOverlay::Softened));

        let bookism = digest
            .shelves
            .iter()
            .find(|s| s.id == "said_bookism")
            .expect("said_bookism");
        assert_eq!(bookism.severity, Severity::Hard);
        assert_eq!(bookism.profile_overlay, Some(ProfileOverlay::PromotedHard));
    }

    #[test]
    fn style_profile_guidance_supplies_voice_sample_and_scene_negative_hooks() {
        let pack = ShelfPack::load_default().expect("pack");
        let guidance = crate::style::StyleProfileGuidance {
            do_rules: vec!["Keep the mill ledger in the POV's hands.".into()],
            avoid_rules: vec![
                "a mix of relief and dread".into(),
                "Do not close on fishing_ending outlook slogans.".into(),
            ],
            prompt_snippet: "Short clauses. Concrete tools. No thesis hinges.".into(),
            narrator_voice: crate::style::NarratorVoice {
                notes: vec!["The narrator names the work, not the mood.".into()],
                ..Default::default()
            },
            ..Default::default()
        };

        let hooks = writing_packet_hooks_from_guidance(&guidance, &pack);
        assert!(
            hooks
                .voice_samples
                .iter()
                .any(|sample| sample.excerpt.contains("mill ledger")
                    && sample.source.contains("do_rules"))
        );
        assert!(
            hooks
                .voice_samples
                .iter()
                .any(|sample| sample.excerpt.contains("Short clauses")
                    && sample.source.contains("prompt_snippet"))
        );
        assert!(hooks.voice_samples.iter().any(
            |sample| sample.speaker == "narrator" && sample.excerpt.contains("names the work")
        ));
        assert!(
            hooks
                .scene_negatives
                .iter()
                .any(|neg| neg.shelf_id.as_deref() == Some("emotion_cocktail")
                    && neg.note.contains("mix of relief"))
        );
        assert!(
            hooks
                .scene_negatives
                .iter()
                .any(|neg| neg.shelf_id.as_deref() == Some("fishing_ending"))
        );
        assert!(
            hooks
                .voice_samples
                .iter()
                .all(|sample| sample.excerpt.split_whitespace().count() <= 40)
        );
        let rendered = render_writing_packet_hooks_markdown(&hooks);
        assert!(rendered.contains("voice_samples"));
        assert!(rendered.contains("scene_negatives"));
        assert!(!rendered.contains("100+"));
    }
}
