# Contributing

Spindle is a Rust workspace. Run commands from the repository root.

## Development Setup

Install a current stable Rust toolchain with `rustfmt` and `clippy`.

```bash
rustup component add rustfmt clippy
```

## Validation

Before opening a change, run the same checks as CI:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

For focused architecture-boundary feedback while editing crate ownership or
DTO placement, run:

```bash
cargo test -p spindle-core --test architecture_boundaries
```

Performance regression tests are opt-in:

```bash
cargo test -p spindle-core --features perf
cargo test -p spindle-adapters --features perf
```

## Local Runtime

Run the MCP server in stdio mode:

```bash
cargo run -p spindle-mcp
```

Set `SPINDLE_DATA_DIR` to choose the local SQLite data directory. Optional model
agent routing is configured with `SPINDLE_CONFIG` or a `spindle.toml` file; see
`docs/spindle-agent-config.md`.

## Change Guidelines

- Keep `spindle-core` free of transport and persistence details.
- Put public tool/resource DTOs in `spindle-core`; keep MCP handlers thin.
- Put repository orchestration, markdown/context assembly, and outbound runtime
  integrations in `spindle-adapters`.
- Keep behavior changes covered by focused tests.
- Update user-facing docs and embedded skills when public tool behavior changes.
- Do not commit local data directories, generated runtime state, API keys, or
  machine-specific client configuration.
