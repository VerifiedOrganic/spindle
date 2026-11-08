use super::*;

impl Repository {
    // =========================================================================
    // Project
    // =========================================================================

    pub async fn list_projects(&self) -> Result<Vec<Project>> {
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {PROJECT_COLUMNS} FROM project ORDER BY created_at DESC");
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([], |r| Project::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn get_project(&self, id: &str) -> Result<Project> {
        let id = id.to_string();
        let project = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {PROJECT_COLUMNS} FROM project WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Project::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("project not found"))?;
        Ok(project)
    }

    /// Create a project plus the initial main branch, book 1, and chapter 1.
    /// Returns the (project, book, chapter) triple matching the SurrealDB
    /// repository's contract.
    pub async fn create_project(
        &self,
        input: &CreateProjectInput,
    ) -> Result<(Project, BibleBranch, Book, Chapter)> {
        let project_id = mint_id("project");
        let branch_id = mint_id("bible_branch");
        let book_id = mint_id("book");
        let chapter_id = mint_id("chapter");
        let now = timestamp_to_micros(chrono::Utc::now());
        let stored_reader_contract: super::records::StoredReaderContract =
            input.reader_contract.clone().into();
        let reader_contract = serde_json::to_string(&stored_reader_contract)
            .context("serializing reader_contract")?;
        let name = input.name.clone();
        let project_type = input.project_type.clone();
        let genre = input.genre.clone();

        let (project_id_returned, branch_id_returned, book_id_returned, chapter_id_returned) = self
            .inner
            .pool
            .write(move |conn| {
                let tx = conn.savepoint()?;
                tx.execute(
                    "INSERT INTO project (id, name, project_type, genre, reader_contract, \
                     active_branch_id, notes, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?7)",
                    rusqlite::params![
                        &project_id,
                        &name,
                        &project_type,
                        &genre,
                        &reader_contract,
                        &branch_id,
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO bible_branch (id, project_id, parent_branch_id, name, status, \
                     branch_type, description, created_from_save_point_id, created_at, updated_at) \
                     VALUES (?1, ?2, NULL, 'main', 'active', NULL, NULL, NULL, ?3, ?3)",
                    rusqlite::params![&branch_id, &project_id, now],
                )?;
                tx.execute(
                    "INSERT INTO book (id, project_id, book_number, title, created_at, updated_at) \
                     VALUES (?1, ?2, 1, NULL, ?3, ?3)",
                    rusqlite::params![&book_id, &project_id, now],
                )?;
                tx.execute(
                    "INSERT INTO chapter (id, project_id, book_id, book_number, chapter_number, \
                     title, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, 1, 1, NULL, ?4, ?4)",
                    rusqlite::params![&chapter_id, &project_id, &book_id, now],
                )?;
                tx.commit()?;
                Ok((project_id, branch_id, book_id, chapter_id))
            })
            .await?;

        let project = self.get_project(&project_id_returned).await?;
        let branch = self.get_branch(&branch_id_returned).await?;
        let book = self.get_book(&book_id_returned).await?;
        let chapter = self.get_chapter(&chapter_id_returned).await?;
        Ok((project, branch, book, chapter))
    }

    /// Persist (or clear) the project's narrator-voice directive. Passing an
    /// empty voice clears the column back to NULL.
    pub async fn set_narrator_voice(
        &self,
        project_id: &str,
        narrator_voice: spindle_core::style::NarratorVoice,
    ) -> Result<Project> {
        let project_id_owned = project_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        let stored_json: Option<String> = if narrator_voice.is_empty() {
            None
        } else {
            let stored: super::records::StoredNarratorVoice = narrator_voice.into();
            Some(serde_json::to_string(&stored).context("serializing narrator_voice")?)
        };
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE project SET narrator_voice = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&stored_json, now, &project_id_owned],
                )?;
                Ok(())
            })
            .await?;
        self.get_project(project_id).await
    }

    // =========================================================================
    // Book
    // =========================================================================

    pub async fn get_book(&self, id: &str) -> Result<Book> {
        let id = id.to_string();
        let book = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {BOOK_COLUMNS} FROM book WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Book::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("book not found"))?;
        Ok(book)
    }

    // =========================================================================
    // Chapter
    // =========================================================================

    pub async fn get_chapter(&self, id: &str) -> Result<Chapter> {
        let id = id.to_string();
        let chapter = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {CHAPTER_COLUMNS} FROM chapter WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Chapter::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("chapter not found"))?;
        Ok(chapter)
    }

    pub async fn list_books_by_project(&self, project_id: &str) -> Result<Vec<Book>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BOOK_COLUMNS} FROM book \
                     WHERE project_id = ?1 ORDER BY book_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Book::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Create a new book in a project, auto-incrementing `book_number` past
    /// the highest existing one.
    pub async fn create_book(&self, project_id: &str, title: Option<String>) -> Result<Book> {
        let id = mint_id("book");
        let project_id_owned = project_id.to_string();
        let id_for_return = id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.savepoint()?;
                let next_book_number: i64 = tx.query_row(
                    "SELECT COALESCE(MAX(book_number), 0) + 1 FROM book WHERE project_id = ?1",
                    [&project_id_owned],
                    |r| r.get(0),
                )?;
                tx.execute(
                    "INSERT INTO book (id, project_id, book_number, title, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    rusqlite::params![&id, &project_id_owned, next_book_number, &title, now],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.get_book(&id_for_return).await
    }

    /// Return the (book, chapter) pair for (book_number, chapter_number),
    /// creating either if missing. Used by save_scene_draft and plan_chapter
    /// when callers provide numeric placement that may not exist yet.
    pub async fn ensure_chapter(
        &self,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<Chapter> {
        if let Some(chapter) = self
            .find_chapter_by_number(project_id, book_number, chapter_number)
            .await?
        {
            return Ok(chapter);
        }
        let book = self.ensure_book(project_id, book_number).await?;
        let chapter_id = mint_id("chapter");
        let chapter_id_lookup = chapter_id.clone();
        let project_id_owned = project_id.to_string();
        let book_id_owned = book.id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO chapter (id, project_id, book_id, book_number, chapter_number, \
                     title, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?6)",
                    rusqlite::params![
                        &chapter_id,
                        &project_id_owned,
                        &book_id_owned,
                        book_number,
                        chapter_number,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_chapter(&chapter_id_lookup).await
    }

    pub async fn ensure_book(&self, project_id: &str, book_number: i32) -> Result<Book> {
        if let Some(book) = self.find_book_by_number(project_id, book_number).await? {
            return Ok(book);
        }
        let book_id = mint_id("book");
        let book_id_lookup = book_id.clone();
        let project_id_owned = project_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO book (id, project_id, book_number, title, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, NULL, ?4, ?4)",
                    rusqlite::params![&book_id, &project_id_owned, book_number, now],
                )?;
                Ok(())
            })
            .await?;
        self.get_book(&book_id_lookup).await
    }

    /// Option-returning lookup of a book by (project_id, book_number).
    pub async fn find_book_by_number(
        &self,
        project_id: &str,
        book_number: i32,
    ) -> Result<Option<Book>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BOOK_COLUMNS} FROM book \
                     WHERE project_id = ?1 AND book_number = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&project_id, book_number], |r| {
                    Book::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    /// Option-returning lookup of a chapter by (project, book#, chapter#).
    pub async fn find_chapter_by_number(
        &self,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<Option<Chapter>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE project_id = ?1 AND book_number = ?2 AND chapter_number = ?3 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&project_id, book_number, chapter_number],
                    |r| Chapter::try_from(r),
                )
                .optional_inner()
            })
            .await
    }

    pub async fn list_chapters_by_book(&self, book_id: &str) -> Result<Vec<Chapter>> {
        let book_id = book_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE book_id = ?1 ORDER BY chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&book_id], |r| Chapter::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Variant of `list_chapters_by_book` that takes a `(project_id,
    /// book_number)` natural key instead of a `book_id`. Used by the
    /// import-hydration pass which walks manuscript book numbers directly.
    pub async fn list_chapters_by_book_number(
        &self,
        project_id: &str,
        book_number: i32,
    ) -> Result<Vec<Chapter>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE project_id = ?1 AND book_number = ?2 ORDER BY chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, book_number], |r| {
                        Chapter::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Scene write paths (save_scene_draft + helpers)
    // =========================================================================

    pub async fn get_book_by_number(&self, project_id: &str, book_number: i32) -> Result<Book> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BOOK_COLUMNS} FROM book \
                     WHERE project_id = ?1 AND book_number = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&project_id, book_number], |r| {
                    Book::try_from(r)
                })
                .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("book {book_number} not found for project"))
    }

    pub async fn get_chapter_by_number(
        &self,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<Chapter> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE project_id = ?1 AND book_number = ?2 AND chapter_number = ?3 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&project_id, book_number, chapter_number],
                    |r| Chapter::try_from(r),
                )
                .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("chapter {book_number}.{chapter_number} not found for project"))
    }

    // =========================================================================
    // Additional list_* helpers backing assemble_subject_snapshot
    // =========================================================================

    /// List every chapter row in a project, ordered by book + chapter number.
    /// Used by the snapshot batch loader; mirrors the SurrealDB scoped
    /// `SELECT * FROM chapter WHERE project_id = $project_id ORDER BY ...`.
    pub async fn list_chapters_by_project(&self, project_id: &str) -> Result<Vec<Chapter>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE project_id = ?1 ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Chapter::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn update_project_reader_contract(
        &self,
        project_id: &str,
        reader_contract: &spindle_core::models::ReaderContract,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let stored: super::records::StoredReaderContract = reader_contract.clone().into();
        let serialized = serde_json::to_string(&stored)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE project SET reader_contract = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&serialized, now, &project_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}
