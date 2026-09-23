.PHONY: install dev build test fmt-check check clean

install:
	npm --prefix desktop install

dev:
	npm --prefix desktop run tauri -- dev

build:
	npm --prefix desktop run tauri -- build

test:
	cargo test --manifest-path desktop-core/Cargo.toml
	cargo test --manifest-path desktop/src-tauri/Cargo.toml

fmt-check:
	cargo fmt --manifest-path desktop-core/Cargo.toml -- --check
	cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check

check:
	cargo check --manifest-path desktop-core/Cargo.toml
	cargo check --manifest-path desktop/src-tauri/Cargo.toml

clean:
	cargo clean --manifest-path desktop-core/Cargo.toml
	cargo clean --manifest-path desktop/src-tauri/Cargo.toml
