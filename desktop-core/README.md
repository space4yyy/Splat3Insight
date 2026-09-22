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

`SplatNetDataClient` requests the latest 50 battle ids and latest 50 Salmon Run
ids from SplatNet, then fetches their detail records. The Tauri command merges
the result into one per-account archive under the platform application-data
directory.

Records are keyed by their stable SplatNet detail id. Each unique record keeps
both a compact parsed form and the raw detail JSON, so later parser changes can
recover fields without requesting old records again. Repeating a sync only
updates overlapping records; it does not append duplicates. The on-disk JSON is
compact to avoid formatting overhead. A future SQLite/blob-compression
migration can reduce storage further without changing the client contract.

## Current-only monitoring

`CurrentBattleMonitor::establish_baseline` records the newest visible battle
without emitting it. Later calls to `observe` return only battles newer than
the persisted cursor. This monitor is separate from the historical archive and
is reserved for the later live-overlay path.

The crate does not read or depend on the `s3s` export directory.

## Nintendo Account login

The desktop flow follows the manual flow used by `s3s`:

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
The desktop app uses nxapi for f generation. Its public OAuth client id is
embedded in the application configuration, so users do not need to configure
an f-generation provider or client id manually.

Authentication failures are also written to the desktop app's `auth.log`
inside its platform application-config directory. The log records stages and
HTTP-safe error text only; it never records login links, session tokens, or
SplatNet tokens.
