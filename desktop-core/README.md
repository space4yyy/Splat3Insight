# Splat 3 widget desktop core

This crate owns account metadata, native credential storage, Nintendo/SplatNet
authentication, the history data client, and the current-battle cursor used by
the Tauri app.

## Storage boundary

- `AccountMetadata` is saved as JSON and contains no tokens.
- `AccountSecret` is stored under a per-account credential entry through the
  platform's native credential store.
- macOS uses Keychain Services; Windows uses Windows Credential Manager.
- The `MemorySecretStore` is only for tests and development.

## History archive

`SplatNetDataClient` requests the complete history window exposed by SplatNet
for the first sync, then checks the latest 50 battle ids and latest 50 Salmon
Run ids during incremental syncs. It fetches the detail records and merges the
result into one per-account archive under the platform application-data
directory.

Records are keyed by their stable SplatNet detail id. Each unique record keeps
both a compact parsed form and the raw detail JSON, so later parser changes can
recover fields without requesting old records again. The desktop app stores
these records in `history/history.sqlite3`: the primary key prevents duplicate
rows and raw detail JSON is gzip-compressed. Incremental syncs only upsert the
newly fetched window, leaving older rows untouched. The app does not import old
per-account JSON archives. When an account is deleted, any stale JSON archive or
export files for that account are removed as part of the account-data cleanup.

Each detail is committed in its own SQLite transaction while it is being
fetched. The per-account `history_meta.sync_complete` checkpoint is set to
`0` before a sync and only set to `1` after the whole fetch finishes. If the
app or network stops part-way through, the already fetched records remain
available and the next run automatically resumes with a full backfill for
that account, skipping detail requests for records already committed before
returning to incremental sync.

At startup the app runs SQLite `PRAGMA integrity_check` and writes a clear
error to the timestamped application log if the database is damaged. Sync
status and the in-progress lock are tracked per account, so changing accounts
does not mix progress or start two syncs for the same account.

The desktop app exports a complete account archive as a `.json.gz` file under
`history/exports/`. The app does not delete historical records: the compressed
raw JSON is retained permanently, while repeated syncs only update the parsed
index fields for the same stable record ID. This keeps the original payload
available for future parser changes without accumulating duplicate copies.

## Nintendo Account login

The desktop flow uses a manual Nintendo Account authorization flow:

1. Generate a Nintendo Account authorization URL and open it in the system browser.
2. The user signs in and copies the link address of the “Select this account” button.
3. Paste that URL into the desktop app.
4. Validate the state and exchange the one-time `de` code for a session token.

No password is entered into the widget. The `npf...://auth#...` value is the
original Nintendo redirect link copied from the “Select this account” button;
the widget only parses it and does not register or own that protocol.

After the Nintendo identity exchange, the account and long-lived
`session_token` are saved immediately. SplatNet authorization is a separate,
repeatable operation: `splatnet_auth` follows the token chain of Coral
`Account/Login`, Coral `Game/GetWebServiceToken`, and SplatNet
`/api/bullet_tokens`. The resulting `game_web_token` and `bullet_token` are
kept with the account secret in the native credential store. A failed
f-generation request therefore does not require another Nintendo login.
If Nintendo rejects the saved session token, the account is marked
`login_expired` and keeps its history; signing in to the same Nintendo account
again replaces its credential in place, so the account never has to be removed
and added back.
The desktop app uses nxapi for f generation. Its public OAuth client id is
embedded in the application configuration, so users do not need to configure
an f-generation provider or client id manually.

Authentication failures are also written to the desktop app's `auth.log`
inside its platform application-config directory. The log records stages and
HTTP-safe error text only; it never records login links, session tokens, or
SplatNet tokens. Console output and file logs use local timestamps with the
current timezone offset. Logs rotate at 5 MiB and retain three backups as
`auth.log.1` through `auth.log.3`.
