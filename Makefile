.PHONY: build test lint ci cargo-build cargo-test cargo-lint ui-install ui-build ui-test

build: cargo-build ui-build

test: cargo-test ui-test

lint: cargo-lint

ci: ui-install lint test build

cargo-build:
	cargo build --workspace --locked

cargo-test:
	cargo test --workspace --locked

cargo-lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings

ui-install:
	npm --prefix ui ci

ui-build:
	npm --prefix ui run build

ui-test:
	npm --prefix ui test
