# New-book setup

Load this unit only when the author wants a new project. For an existing book,
list and select the existing project instead of creating a replacement.
Establish title, genre, premise, reader promise, style, and boundaries from the
author's supplied direction; ask only for missing creative choices.

Use `create_project`, then `set_active_project`, `create_book`, and
`create_chapter` as needed. Use returned identifiers and the live schemas;
project creation may already create the initial book and chapter, so inspect
`get_writer_state` before adding them. Load planning, worldbuilding, or character
skills as needed. Setup grants no permission to invent canon or approve proposals.

<!-- tool: create_book -->
<!-- tool: create_chapter -->
<!-- tool: create_project -->
<!-- tool: get_writer_state -->
<!-- tool: set_active_project -->
