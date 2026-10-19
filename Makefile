.PHONY: install dev dev-signing build release-signing release test fmt-check check clean

install:
	npm --prefix desktop install

dev:
	npm --prefix desktop run tauri -- dev

# macOS：创建本机开发签名证书（只需一次），重新编译后不再反复要求钥匙串密码。
dev-signing:
	desktop/scripts/dev-signing.sh setup

build:
	npm --prefix desktop run tauri -- build

# 发布签名：生成自动更新密钥和 macOS 发布证书（只需一次），内容填进 GitHub Actions secrets。
release-signing:
	desktop/scripts/release-signing.sh setup

# 发布：改版本号、提交并打标签（不推送），例如 make release VERSION=0.3.0
release:
	desktop/scripts/release.sh $(VERSION)

test:
	cargo test --manifest-path desktop-core/Cargo.toml
	cargo test --manifest-path desktop/src-tauri/Cargo.toml
	npm --prefix desktop test

fmt-check:
	cargo fmt --manifest-path desktop-core/Cargo.toml -- --check
	cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check

check:
	cargo check --manifest-path desktop-core/Cargo.toml
	cargo check --manifest-path desktop/src-tauri/Cargo.toml

clean:
	cargo clean --manifest-path desktop-core/Cargo.toml
	cargo clean --manifest-path desktop/src-tauri/Cargo.toml
