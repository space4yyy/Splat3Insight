# Repository guide

## Project shape

- `desktop/src-tauri/`: Tauri host, commands, persistence, local OBS HTTP service.
- `desktop-core/`: account/authentication, SplatNet data client, domain models, and overlay selection/calculation.
- `desktop/ui/`: static desktop interface and the shared OBS card stylesheet.
- The Tauri frontend is static; `desktop/src-tauri/tauri.conf.json` points to `desktop/ui`.
- Keep development and runtime behavior in the desktop app and shared core crates.

## Important invariants

- Keep account tokens and credentials in the platform credential store. Never print or log credentials, login callback URLs, or token values.
- Preserve original battle and Salmon Run JSON in the compressed SQLite archive. Do not silently prune user history or app-data exports.
- Battle power deltas must only compare records from the same track: X Match by rule, Anarchy Challenge by weapon, and Anarchy Open by team identity. Missing/unplaced power remains unavailable rather than borrowing another track's score.
- The app preview and `/overlay` should use the same card markup conventions and `desktop/ui/overlay-card.css`; keep the card proportional and transparent outside its bounds.
- Keep the OBS card catalog in `desktop/ui/obs-cards.json`; account-scoped extension tables use the `extension_` prefix and an `account_id` column so account deletion can clean them generically.
- Treat account data and local exports as private; never stage local exports.

## Working practices

- Inspect existing changes before editing; preserve unrelated user modifications.
- Use `cargo fmt` for Rust formatting and run the relevant crate tests after behavior changes.
- Useful repository checks: `make fmt-check`, `make test`, and `make check`.
- Keep user-facing UI text in Simplified Chinese unless the feature requires a locale-specific string.
- Avoid committing unless the user explicitly asks for a commit.
