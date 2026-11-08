use super::*;

#[tokio::test]
async fn pre_v0024_database_upgrades_additively() {
    // A DB migrated only through V0023 (no canon_delta table) must upgrade
    // to V0024 cleanly with its existing rows intact — the migration is a
    // pure addition (ADR reversal-cost: additions are additive).
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("legacy.db");

    // Open a throwaway pool first so sqlite-vec's `vec0` module is
    // registered process-globally (via sqlite3_auto_extension) before we
    // run the raw-connection migration — V0002 declares a vec0 table.
    let _warm = SqlitePool::open(&tmp.path().join("warm.db")).await.unwrap();

    // Stage 1: run migrations up to V0023 only, on a raw rusqlite conn.
    {
        let mut conn = rusqlite::Connection::open(&db_path).unwrap();
        crate::sqlite::migrations::runner()
            .set_target(refinery::Target::Version(23))
            .run(&mut conn)
            .unwrap();
        // canon_delta does not exist yet at V0023.
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='canon_delta'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "canon_delta must not exist before V0024");
        // Seed a project row so we can prove it survives the upgrade.
        let now = timestamp_to_micros(chrono::Utc::now());
        conn.execute(
            "INSERT INTO project (id, name, project_type, genre, reader_contract, \
                 created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "project:legacy",
                "Legacy",
                "novel",
                "fantasy",
                r#"{"promise":"p","style_notes":[],"boundaries":[]}"#,
                now,
                now
            ],
        )
        .unwrap();
    }

    // Stage 2: open through the pool, which runs the full runner (incl.
    // V0024). Must succeed and the legacy row must still be there.
    let pool = SqlitePool::open(&db_path).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);

    let again = repo.get_project("project:legacy").await.unwrap();
    assert_eq!(again.name, "Legacy");

    // And canon_delta now exists and is queryable (empty).
    let listed = repo
        .list_canon_deltas("project:legacy", "bible_branch:missing", None, None)
        .await
        .unwrap();
    assert!(listed.is_empty());
}

#[tokio::test]
async fn pre_v0029_database_upgrades_additively() {
    // A DB migrated only through V0028 (no plan_amendment table, no
    // chapter_plan.plan_revision column) must upgrade to V0029 cleanly with
    // its existing rows intact — the migration is a pure addition (ADR
    // reversal-cost: additions are additive).
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("legacy29.db");

    // Warm sqlite-vec's vec0 module process-globally before the raw-conn run.
    let _warm = SqlitePool::open(&tmp.path().join("warm29.db"))
        .await
        .unwrap();

    // Stage 1: run migrations up to V0028 only, on a raw rusqlite conn.
    {
        let mut conn = rusqlite::Connection::open(&db_path).unwrap();
        crate::sqlite::migrations::runner()
            .set_target(refinery::Target::Version(28))
            .run(&mut conn)
            .unwrap();
        // plan_amendment does not exist yet at V0028.
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='plan_amendment'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "plan_amendment must not exist before V0029");
        // Seed a project row so we can prove it survives the upgrade.
        let now = timestamp_to_micros(chrono::Utc::now());
        conn.execute(
            "INSERT INTO project (id, name, project_type, genre, reader_contract, \
                 created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "project:legacy29",
                "Legacy29",
                "novel",
                "fantasy",
                r#"{"promise":"p","style_notes":[],"boundaries":[]}"#,
                now,
                now
            ],
        )
        .unwrap();
    }

    // Stage 2: open through the pool, which runs the full runner (incl.
    // V0029). Must succeed and the legacy row must still be there.
    let pool = SqlitePool::open(&db_path).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);

    let again = repo.get_project("project:legacy29").await.unwrap();
    assert_eq!(again.name, "Legacy29");

    // And plan_amendment now exists and is queryable (empty).
    let listed = repo
        .list_plan_amendments("project:legacy29", "bible_branch:missing", None, None)
        .await
        .unwrap();
    assert!(listed.is_empty());
}

#[tokio::test]
async fn main_v0047_database_applies_canon_migration() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("main47.db");
    let _warm = SqlitePool::open(&tmp.path().join("warm47.db"))
        .await
        .unwrap();
    {
        let mut conn = rusqlite::Connection::open(&db_path).unwrap();
        crate::sqlite::migrations::runner()
            .set_target(refinery::Target::Version(47))
            .run(&mut conn)
            .unwrap();
        let latest: i32 = conn
            .query_row(
                "SELECT MAX(version) FROM refinery_schema_history",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(latest, 47);
    }
    let pool = SqlitePool::open(&db_path).await.unwrap();
    pool.read(|conn| {
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM relationship_scene_delta", [], |r| {
                r.get(0)
            })?;
        assert_eq!(count, 0);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn pre_v0037_database_applies_later_migrations_with_secret_facts() {
    // HEAD already shipped V0036. This batch adds V0037 (aliases), V0038
    // (nullable canonical_fact.scene_id), and V0039 (deep_check_cache).
    // Refinery abort_missing would refuse a V0035 inserted below 36; this
    // test is the live upgrade: a V0036-era DB with a secret-linked
    // knowledge_fact must open, keep the link, and gain the new table.
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("legacy36.db");
    let _warm = SqlitePool::open(&tmp.path().join("warm36.db"))
        .await
        .unwrap();

    {
        let mut conn = rusqlite::Connection::open(&db_path).unwrap();
        crate::sqlite::migrations::runner()
            .set_target(refinery::Target::Version(36))
            .run(&mut conn)
            .unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();

        let cache_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='deep_check_cache'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cache_exists, 0, "deep_check_cache must not exist at V0036");

        let now = timestamp_to_micros(chrono::Utc::now());
        conn.execute(
            "INSERT INTO project (id, name, project_type, genre, reader_contract, \
                 created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "project:legacy36",
                "Legacy36",
                "novel",
                "fantasy",
                r#"{"promise":"p","style_notes":[],"boundaries":[]}"#,
                now,
                now
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO bible_branch (id, project_id, name, status, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "bible_branch:legacy36",
                "project:legacy36",
                "main",
                "active",
                now
            ],
        )
        .unwrap();
        conn.execute(
            "UPDATE project SET active_branch_id = ?1 WHERE id = ?2",
            rusqlite::params!["bible_branch:legacy36", "project:legacy36"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO book (id, project_id, book_number, created_at) \
                 VALUES (?1, ?2, 1, ?3)",
            rusqlite::params!["book:legacy36", "project:legacy36", now],
        )
        .unwrap();
        conn.execute(
                "INSERT INTO chapter (id, project_id, book_id, book_number, chapter_number, created_at) \
                 VALUES (?1, ?2, ?3, 1, 1, ?4)",
                rusqlite::params!["chapter:legacy36", "project:legacy36", "book:legacy36", now],
            )
            .unwrap();
        conn.execute(
            "INSERT INTO scene (id, project_id, branch_id, book_id, chapter_id, \
                 book_number, chapter_number, scene_order, full_text, summary, \
                 content_rating, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, 1, 'prose', 'sum', 'General', ?6, ?6)",
            rusqlite::params![
                "scene:legacy36",
                "project:legacy36",
                "bible_branch:legacy36",
                "book:legacy36",
                "chapter:legacy36",
                now
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO character (id, project_id, branch_id, name, normalized_name, \
                 summary, role, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            rusqlite::params![
                "character:legacy36",
                "project:legacy36",
                "bible_branch:legacy36",
                "Nate",
                "nate",
                "lead",
                "protagonist",
                now
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO canonical_fact (id, project_id, branch_id, scene_id, \
                 book_number, chapter_number, subject_table, predicate, value_kind, \
                 value_text, aliases, scope, created_at, updated_at, secret) \
                 VALUES (?1, ?2, ?3, ?4, 1, 1, 'character', 'true_name', 'string', \
                         'Nathaniel', '[]', 'invariant', ?5, ?5, 1)",
            rusqlite::params![
                "canonical_fact:legacy36",
                "project:legacy36",
                "bible_branch:legacy36",
                "scene:legacy36",
                now
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO knowledge_fact (id, project_id, branch_id, character_id, \
                 fact, normalized_fact, source_summary, tags, reader_visible, \
                 created_at, updated_at, secret_of_fact_id) \
                 VALUES (?1, ?2, ?3, ?4, 'knows the name', 'knows the name', 'reveal', \
                         '[]', 1, ?5, ?5, ?6)",
            rusqlite::params![
                "knowledge_fact:legacy36",
                "project:legacy36",
                "bible_branch:legacy36",
                "character:legacy36",
                now,
                "canonical_fact:legacy36"
            ],
        )
        .unwrap();
    }

    let pool = SqlitePool::open(&db_path).await.expect(
        "a V0036-era database with secret-linked knowledge_facts must migrate through V0038/V0039",
    );
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);

    let again = repo.get_project("project:legacy36").await.unwrap();
    assert_eq!(again.name, "Legacy36");

    let knowledge = repo
        .get_knowledge_fact("knowledge_fact:legacy36")
        .await
        .unwrap();
    assert_eq!(
        knowledge.secret_of_fact_id.as_deref(),
        Some("canonical_fact:legacy36"),
        "V0038 rebuild must restore secret_of_fact_id links"
    );

    let scene_id_notnull: i64 = repo
        .pool()
        .read(|conn| {
            conn.query_row(
                "SELECT \"notnull\" FROM pragma_table_info('canonical_fact') \
                     WHERE name = 'scene_id'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(scene_id_notnull, 0, "V0038 must make scene_id nullable");

    let cache_exists: i64 = repo
        .pool()
        .read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                     WHERE type='table' AND name='deep_check_cache'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(cache_exists, 1, "V0039 must create deep_check_cache");
}

#[tokio::test]
async fn pre_v0051_database_upgrades_additively() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("legacy50.db");
    let _warm = SqlitePool::open(&tmp.path().join("warm50.db"))
        .await
        .unwrap();

    {
        let mut conn = rusqlite::Connection::open(&db_path).unwrap();
        crate::sqlite::migrations::runner()
            .set_target(refinery::Target::Version(50))
            .run(&mut conn)
            .unwrap();
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='canon_event'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "canon_event must not exist before V0051");

        let now = timestamp_to_micros(chrono::Utc::now());
        conn.execute(
            "INSERT INTO project (id, name, project_type, genre, reader_contract, \
                 created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "project:legacy50",
                "Legacy50",
                "novel",
                "fantasy",
                r#"{"promise":"p","style_notes":[],"boundaries":[]}"#,
                now,
                now
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO bible_branch (id, project_id, name, status, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "bible_branch:legacy50",
                "project:legacy50",
                "main",
                "active",
                now
            ],
        )
        .unwrap();
    }

    let pool = SqlitePool::open(&db_path).await.unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let repo = Repository::new(pool, data_dir);

    let again = repo.get_project("project:legacy50").await.unwrap();
    assert_eq!(again.name, "Legacy50");

    let events = repo
        .list_canon_events_for_lineage("project:legacy50", "bible_branch:legacy50")
        .await
        .unwrap();
    assert!(events.is_empty());
}
