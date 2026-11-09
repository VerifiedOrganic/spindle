# New-Book Setup

Load this unit only when the author is starting a new project. For an existing book, select the existing project instead of creating a replacement.

1. Establish title, genre, project type, reader promise, style notes, and boundaries from the author's supplied direction; ask only for missing creative choices.
2. Call `create_project` with the agreed `reader_contract`, then call `set_active_project` with the returned `project_id`.
3. Call `get_writer_state` to inspect the initial book and chapter created with the project. Call `create_book` or `create_chapter` only when additional books or chapters are needed beyond Book 1, Chapter 1.
4. For a full premise-to-ready planning flow, load `get_skill(name="new-book")`; it stages and ratifies planning levels through human gates before drafting. For smaller manual setup, load `get_skill` for `plot-architect`, `worldbuilder`, or `character-creator` as needed to establish initial canon, then load `scene-writer` to prepare and draft the opening scene.

Setup grants no permission to invent unrequested canon or auto-approve proposals.

<!-- tool: create_book -->
<!-- tool: create_chapter -->
<!-- tool: create_project -->
<!-- tool: get_skill -->
<!-- tool: get_writer_state -->
<!-- tool: set_active_project -->
