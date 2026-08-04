.PHONY: build test lint ci contract bambu-evidence-check license-check run release-cli-check release-gate release-qualification release-evidence cargo-build cargo-test cargo-lint ui-install ui-build ui-test

build: cargo-build ui-build

test: cargo-test ui-test

lint: cargo-lint

ci: license-check bambu-evidence-check ui-install lint test build contract

license-check:
	test -f LICENSE
	grep -Fqx 'license = "AGPL-3.0-only"' Cargo.toml
	grep -Fq '"license": "AGPL-3.0-only"' ui/package.json
	grep -Fq '"../../LICENSE"' crates/polimero-desktop/tauri.conf.json
	! git grep -nE 'license = "MIT"|"license": "MIT"' -- ':!Cargo.lock' ':!Makefile'

contract:
	cargo test -p polimero-cli --test contract_fixtures --locked

bambu-evidence-check:
	cargo test -p polimero-core --test bambu_evidence --locked

run:
	@set -e; \
	bun run --cwd ui dev & vite_pid=$$!; \
	trap 'kill $$vite_pid 2>/dev/null || true' EXIT INT TERM; \
	until curl --fail --silent http://127.0.0.1:1420 >/dev/null; do sleep 1; done; \
	if test "$$XDG_SESSION_TYPE" = wayland && test -n "$$DISPLAY" && test -z "$$GDK_BACKEND"; then \
		echo "Using X11 GTK fallback for this Wayland session"; \
		GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER="$${WEBKIT_DISABLE_DMABUF_RENDERER:-1}" cargo run -p polimero-desktop --bin polimero; \
	else \
		cargo run -p polimero-desktop --bin polimero; \
	fi

release-gate: bambu-evidence-check contract cargo-build
	$(MAKE) release-cli-check

release-cli-check:
	POLIMERO_CONFIG_DIR="$(CURDIR)/fixtures/cli-contract/config/empty" \
		./target/debug/polimero version --output json

release-qualification: ci release-cli-check

release-evidence:
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

cargo-build:
	cargo build --workspace --locked

cargo-test:
	cargo test --workspace --locked

cargo-lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings

ui-install:
	bun install --cwd ui --frozen-lockfile

ui-build:
	bun run --cwd ui build

ui-test:
	bun test --cwd ui
