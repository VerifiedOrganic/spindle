# ──────────────────────────────────────────────────────────────────────────────
# Spindle – local-first MCP server for fiction authoring
# ──────────────────────────────────────────────────────────────────────────────

BINARY       := spindle-mcp
INSTALL_DIR  := $(HOME)/bin
RELEASE_BIN  := target/release/$(BINARY)
DEBUG_BIN    := target/debug/$(BINARY)
HTTP_ADDR    := 127.0.0.1:8787
DATA_DIR     := $(shell echo $${SPINDLE_DATA_DIR:-$$(dirs=$$(command -v dirs 2>/dev/null) && echo "$$($$dirs 2>/dev/null || echo $$HOME/.local/share)/spindle" || echo "$$HOME/.local/share/spindle")})

# MCP config locations
CLAUDE_LOCAL_MCP  := .claude/mcp.json
CLAUDE_GLOBAL_MCP := $(HOME)/.claude/mcp.json

.PHONY: help build release install uninstall \
        test test-core test-adapters test-mcp test-skills test-integration test-verbose \
        lint fmt fmt-check check validate ci \
        run run-http run-release run-release-http \
        clean clean-data reset-data \
        install-mcp uninstall-mcp show-mcp \
        deps doc loc

# ── Help ─────────────────────────────────────────────────────────────────────

help: ## Show this help menu
	@echo ""
	@echo "  \033[1mSpindle\033[0m — local-first MCP server for fiction authoring"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; { \
			split($$2, parts, /\|/); \
			if (parts[2]) { \
				printf "  \033[36m%-20s\033[0m \033[33m%-12s\033[0m %s\n", $$1, parts[1], parts[2]; \
			} else { \
				printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2; \
			} \
		}'
	@echo ""

# ── Build ────────────────────────────────────────────────────────────────────

build: ## Build debug binary
	cargo build -p $(BINARY)

release: ## Build optimised release binary (LTO + strip)
	cargo build -p $(BINARY) --release

check: ## Fast type-check (no codegen)
	cargo check --workspace --all-targets

# ── Install / Uninstall ─────────────────────────────────────────────────────

install: release ## Build release and install to ~/bin
	@mkdir -p $(INSTALL_DIR)
	@if pgrep -xf "$(INSTALL_DIR)/$(BINARY)" >/dev/null 2>&1; then \
		echo "Stopping running $(BINARY)…"; \
		pkill -xf "$(INSTALL_DIR)/$(BINARY)" 2>/dev/null || true; \
		sleep 1; \
	fi
	cp $(RELEASE_BIN) $(INSTALL_DIR)/$(BINARY)
	@echo "\033[32m✓\033[0m Installed $(BINARY) to $(INSTALL_DIR)/$(BINARY)"
	@echo "  Binary size: $$(du -h $(INSTALL_DIR)/$(BINARY) | cut -f1)"

uninstall: ## Remove installed binary from ~/bin
	@rm -f $(INSTALL_DIR)/$(BINARY)
	@echo "\033[32m✓\033[0m Removed $(BINARY) from $(INSTALL_DIR)"

# ── Tests ────────────────────────────────────────────────────────────────────

test: ## Run all workspace tests
	cargo test --workspace

test-core: ## Run spindle-core tests
	cargo test -p spindle-core

test-adapters: ## Run spindle-adapters tests
	cargo test -p spindle-adapters

test-mcp: ## Run spindle-mcp tests
	cargo test -p spindle-mcp

test-skills: ## Run spindle-skills tests
	cargo test -p spindle-skills

test-integration: ## Run integration tests only
	cargo test -p spindle-mcp --test '*' -- --include-ignored

test-verbose: ## Run all tests with output shown
	cargo test --workspace -- --nocapture

# ── Lint & Format ───────────────────────────────────────────────────────────

lint: ## Run clippy with deny warnings
	cargo clippy --workspace --all-targets -- -D warnings

fmt: ## Format all Rust source files
	cargo fmt --all

fmt-check: ## Check formatting without modifying files
	cargo fmt --all -- --check

# ── Validation ───────────────────────────────────────────────────────────────

validate: fmt-check lint test ## Full validation: fmt + clippy + tests

ci: validate ## Alias for validate (CI-style gate)

# ── Run ──────────────────────────────────────────────────────────────────────

run: ## Run MCP server (stdio transport, debug build)
	cargo run -p $(BINARY)

run-http: ## Run MCP server in HTTP mode (debug build)
	SPINDLE_HTTP_ADDR=$(HTTP_ADDR) cargo run -p $(BINARY)

run-release: release ## Run MCP server from release binary
	$(RELEASE_BIN)

run-release-http: release ## Run HTTP mode from release binary
	SPINDLE_HTTP_ADDR=$(HTTP_ADDR) $(RELEASE_BIN)

# ── MCP Config ───────────────────────────────────────────────────────────────

install-mcp: install ## Install binary + write global Claude MCP config
	@mkdir -p $(dir $(CLAUDE_GLOBAL_MCP))
	@if [ -f "$(CLAUDE_GLOBAL_MCP)" ]; then \
		if command -v jq >/dev/null 2>&1; then \
			jq '.mcpServers.spindle = {"command":"$(INSTALL_DIR)/$(BINARY)","args":[]}' \
				"$(CLAUDE_GLOBAL_MCP)" > "$(CLAUDE_GLOBAL_MCP).tmp" && \
				mv "$(CLAUDE_GLOBAL_MCP).tmp" "$(CLAUDE_GLOBAL_MCP)"; \
		else \
			echo '\033[33m⚠\033[0m  jq not found — cannot merge into existing config.'; \
			echo '  Add this to $(CLAUDE_GLOBAL_MCP) manually:'; \
			echo '    "spindle": {"command":"$(INSTALL_DIR)/$(BINARY)","args":[]}'; \
			exit 0; \
		fi; \
	else \
		printf '{\n  "mcpServers": {\n    "spindle": {\n      "command": "%s",\n      "args": []\n    }\n  }\n}\n' \
			"$(INSTALL_DIR)/$(BINARY)" > "$(CLAUDE_GLOBAL_MCP)"; \
	fi
	@echo "\033[32m✓\033[0m MCP config written to $(CLAUDE_GLOBAL_MCP)"

uninstall-mcp: ## Remove spindle from global Claude MCP config
	@if [ -f "$(CLAUDE_GLOBAL_MCP)" ] && command -v jq >/dev/null 2>&1; then \
		jq 'del(.mcpServers.spindle)' "$(CLAUDE_GLOBAL_MCP)" > "$(CLAUDE_GLOBAL_MCP).tmp" && \
			mv "$(CLAUDE_GLOBAL_MCP).tmp" "$(CLAUDE_GLOBAL_MCP)"; \
		echo "\033[32m✓\033[0m Removed spindle from $(CLAUDE_GLOBAL_MCP)"; \
	else \
		echo "Nothing to remove or jq not available."; \
	fi

show-mcp: ## Print current MCP config (local + global)
	@echo "\033[1m── Local (.claude/mcp.json) ──\033[0m"
	@if [ -f "$(CLAUDE_LOCAL_MCP)" ]; then cat "$(CLAUDE_LOCAL_MCP)"; else echo "  (not found)"; fi
	@echo ""
	@echo "\033[1m── Global (~/.claude/mcp.json) ──\033[0m"
	@if [ -f "$(CLAUDE_GLOBAL_MCP)" ]; then cat "$(CLAUDE_GLOBAL_MCP)"; else echo "  (not found)"; fi

# ── Data Management ─────────────────────────────────────────────────────────

clean: ## Remove build artifacts
	cargo clean

clean-data: ## Delete local Spindle data directory
	@if [ -d "$(DATA_DIR)" ]; then \
		echo "\033[33m⚠\033[0m  This will delete: $(DATA_DIR)"; \
		printf "  Continue? [y/N] "; read -r ans; \
		case "$$ans" in [yY]*) rm -rf "$(DATA_DIR)" && echo "\033[32m✓\033[0m Deleted $(DATA_DIR)";; *) echo "Aborted.";; esac; \
	else \
		echo "No data directory found at $(DATA_DIR)"; \
	fi

reset-data: clean-data run ## Delete data and start fresh server

# ── Utilities ────────────────────────────────────────────────────────────────

deps: ## Show dependency tree
	cargo tree --workspace --depth 1

doc: ## Generate and open rustdoc
	cargo doc --workspace --no-deps --open

loc: ## Count lines of Rust source code
	@echo "\033[1mLines of Rust code by crate:\033[0m"
	@for crate in spindle-core spindle-adapters spindle-mcp spindle-skills; do \
		count=$$(find crates/$$crate/src -name '*.rs' -exec cat {} + 2>/dev/null | wc -l | tr -d ' '); \
		printf "  \033[36m%-20s\033[0m %s lines\n" "$$crate" "$$count"; \
	done
	@total=$$(find crates -name '*.rs' -exec cat {} + 2>/dev/null | wc -l | tr -d ' '); \
	echo "  ────────────────────────────"; \
	printf "  \033[1m%-20s\033[0m %s lines\n" "total" "$$total"
