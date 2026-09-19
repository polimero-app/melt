.PHONY: build test lint ci contract bambu-evidence-check help license-check run install-desktop uninstall-desktop release-cli-check release-gate release-qualification release-evidence cargo-build cargo-test cargo-lint ui-install ui-build ui-test
.DEFAULT_GOAL := help

build: cargo-build ui-build ## Build the Rust workspace and UI

test: cargo-test ui-test ## Run the Rust and UI test suites

lint: cargo-lint ## Check Rust formatting and linting

ci: license-check bambu-evidence-check ui-install lint test build contract ## Run the continuous-integration checks

license-check: ## Verify license declarations and files
	test -f LICENSE
	grep -Fqx 'license = "AGPL-3.0-only"' Cargo.toml
	grep -Fq '"license": "AGPL-3.0-only"' ui/package.json
	grep -Fq '"../../LICENSE"' crates/melt-desktop/tauri.conf.json
	! git grep -nE 'license = "MIT"|"license": "MIT"' -- ':!Cargo.lock' ':!Makefile'

contract: ## Run CLI contract tests
	cargo test -p melt-cli --test contract_fixtures --locked

bambu-evidence-check: ## Run Bambu evidence tests
	cargo test -p melt-core --test bambu_evidence --locked

run: ## Start the desktop app in development mode
	@set -e; \
	bun run --cwd ui dev & vite_pid=$$!; \
	trap 'kill $$vite_pid 2>/dev/null || true' EXIT INT TERM; \
	until curl --fail --silent http://127.0.0.1:1420 >/dev/null; do sleep 1; done; \
	if test "$$XDG_SESSION_TYPE" = wayland; then \
		echo "Disabling WebKit DMA-BUF rendering for this Wayland session"; \
		WEBKIT_DISABLE_DMABUF_RENDERER="$${WEBKIT_DISABLE_DMABUF_RENDERER:-1}" cargo run -p melt-desktop --bin melt; \
	else \
		cargo run -p melt-desktop --bin melt; \
	fi

XDG_DATA_HOME ?= $(HOME)/.local/share

# A development build installs nothing, so a Wayland compositor has no desktop
# entry to resolve the window icon from. Package builds carry their own entry;
# these targets only mirror it for `make run`. The entry must keep
# StartupWMClass matching the GTK application id, which Tauri leaves as the
# binary name unless app.enableGTKAppId is set.
install-desktop: ## Install a development desktop entry so the window icon resolves
	@set -e; \
	install -Dm644 crates/melt-desktop/icons/icon.svg \
		"$(XDG_DATA_HOME)/icons/hicolor/scalable/apps/melt.svg"; \
	install -Dm644 crates/melt-desktop/icons/32x32.png \
		"$(XDG_DATA_HOME)/icons/hicolor/32x32/apps/melt.png"; \
	install -Dm644 crates/melt-desktop/icons/128x128.png \
		"$(XDG_DATA_HOME)/icons/hicolor/128x128/apps/melt.png"; \
	install -Dm644 'crates/melt-desktop/icons/128x128@2x.png' \
		"$(XDG_DATA_HOME)/icons/hicolor/256x256/apps/melt.png"; \
	install -d "$(XDG_DATA_HOME)/applications"; \
	printf '%s\n' \
		'[Desktop Entry]' \
		'Type=Application' \
		'Name=Melt (development)' \
		'Comment=Local-first 3D printer control' \
		'Exec=$(CURDIR)/target/debug/melt' \
		'Icon=melt' \
		'StartupWMClass=melt' \
		'Terminal=false' \
		'Categories=Utility;' \
		> "$(XDG_DATA_HOME)/applications/melt-dev.desktop"; \
	update-desktop-database "$(XDG_DATA_HOME)/applications" 2>/dev/null || true; \
	gtk-update-icon-cache -qtf "$(XDG_DATA_HOME)/icons/hicolor" 2>/dev/null || true; \
	echo "Installed melt-dev.desktop and hicolor icons under $(XDG_DATA_HOME)"

uninstall-desktop: ## Remove the development desktop entry and icons
	@set -e; \
	rm -f "$(XDG_DATA_HOME)/applications/melt-dev.desktop"; \
	rm -f "$(XDG_DATA_HOME)"/icons/hicolor/*/apps/melt.png; \
	rm -f "$(XDG_DATA_HOME)/icons/hicolor/scalable/apps/melt.svg"; \
	update-desktop-database "$(XDG_DATA_HOME)/applications" 2>/dev/null || true; \
	gtk-update-icon-cache -qtf "$(XDG_DATA_HOME)/icons/hicolor" 2>/dev/null || true; \
	echo "Removed melt-dev.desktop and hicolor icons"

release-gate: bambu-evidence-check contract cargo-build ## Run required release checks
	$(MAKE) release-cli-check

release-cli-check: ## Verify the release CLI output
	MELT_CONFIG_DIR="$(CURDIR)/fixtures/cli-contract/config/empty" \
		./target/debug/melt version --output json

release-qualification: ci release-cli-check ## Run the full release qualification suite

release-evidence: ## Validate release evidence documents
	test -n "$(RELEASE_TAG)"
	for file in release-packaging-checklist.md updater-verification-report.md; do \
		path="docs/releases/$(RELEASE_TAG)/$$file"; \
		test -f "$$path"; \
		grep -Fqx "Release tag: $(RELEASE_TAG)" "$$path"; \
		grep -Eq "^Commit: .+" "$$path"; \
		grep -Eq "^Workflow: .+" "$$path"; \
		grep -Fqx "## Automated checks" "$$path"; \
		grep -Fqx "## External/manual evidence" "$$path"; \
		grep -Fq -- "- [x]" "$$path"; \
		! grep -Fq -- "- [ ]" "$$path"; \
	done

cargo-build: ## Build the Rust workspace
	cargo build --workspace --locked

cargo-test: ## Run the Rust test suite
	cargo test --workspace --locked

cargo-lint: ## Check Rust formatting and linting
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings

ui-install: ## Install locked UI dependencies
	bun install --cwd ui --frozen-lockfile

ui-build: ## Build the UI
	bun run --cwd ui build

ui-test: ## Run the UI test suite
	bun test --cwd ui

help: ## Show this help
	@printf "\033[37mUsage:\033[0m\n"
	@printf "  \033[37mmake [target]\033[0m\n\n"
	@printf "\033[34mAvailable targets:\033[0m\n"
	@grep -E '^[0-9a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[0;36m%-25s\033[m %s\n", $$1, $$2}'
	@printf "\n"
