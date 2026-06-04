# Spindle Project-Local Workspaces

Spindle supports a project-local workspace convention designed to make it easy to manage separate books independently. This allows you to work in a book folder, connect an MCP client, and resolve all story bible state, database files, configuration, and authoring run artifacts local to that folder.

## Workspace Layout

When you run `spindle-mcp init` (or `spindle init` if aliased) in a folder, Spindle initializes a local workspace by creating a `.spindle/` folder:

```
my-book/
├── .spindle/
│   ├── spindle.db          # The SQLite database holding this book's story bible and runs
│   ├── config.toml         # Project-specific agent & routing configurations
│   ├── artifacts/          # Scene drafts, paces, and checkpoint reports
│   └── runtime/            # Temporary runtime files (e.g. spindle.addr)
```

## Resolution Semantics

When the Spindle MCP server starts or a Spindle CLI command runs:
1. It looks for the nearest ancestor `.spindle/` directory starting from the current working directory.
2. If found, all data, configuration (`.spindle/config.toml`), database (`.spindle/spindle.db`), and artifacts resolve to that local workspace.
3. If no `.spindle/` folder is found:
   - It falls back to the legacy platform-global directory (typically `~/.local/share/spindle` on Linux/macOS, with the database file named `spindle.sqlite`).
4. Explicit overrides (`SPINDLE_DATA_DIR` and `SPINDLE_CONFIG` environment variables) always win and bypass the workspace lookup.

This structure guarantees that two separate book folders will never share DB states or bleed context unless you explicitly configure them to.

## Git & Version Control Conventions

We recommend the following practices when using Spindle with Git:

### What to Ignore

You should **not** commit the binary SQLite database to Git. It changes with every run, leads to binary merge conflicts, and is not human-readable. Add `.spindle/` (or at least `.spindle/spindle.db` and `.spindle/runtime/`) to your `.gitignore`:

```gitignore
# Ignore local Spindle database and runtime lock/address files
.spindle/spindle.db
.spindle/runtime/
```

### What to Commit

- **Project Config**: Committing `.spindle/config.toml` is highly recommended. It defines the model routes, custom prompts, and target agents that make your book's compilation environment reproducible for other collaborators or machines.
- **Human-Readable Exports**: Use Spindle's export capabilities (like EPUB or Markdown exports of the manuscript/canon) and commit those human-readable outputs to Git. This allows you to track changes to your actual text and story bible in a clean, diffable way.
- **Durable Snapshots/Metadata**: If you generate JSON seed files or durable backup snapshots, commit them to your repository to serve as the source of truth for rebuilding the story bible when cloning the book to a new machine.
