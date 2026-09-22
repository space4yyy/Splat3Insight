# Splat 3 widget desktop core

This crate is the first desktop-app slice. It owns account metadata, native
credential storage, and the current-only battle cursor that the future Tauri
commands will use.

## Storage boundary

- `AccountMetadata` is saved as JSON and contains no tokens.
- `AccountSecret` is stored under a per-account credential entry through the
  platform's native credential store.
- macOS uses Keychain Services; Windows uses Windows Credential Manager.
- The `MemorySecretStore` is only for tests and development.

## Current-only monitoring

`CurrentBattleMonitor::establish_baseline` records the newest visible battle
without emitting it. Later calls to `observe` return only battles newer than
the persisted cursor. The data client will fetch details only for those new
IDs, so adding an account never replays its old history.

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
The desktop app defaults to nxapi because the imink-compatible endpoint can
return a generic HTTP 500 while its service is unavailable. Configure the
nxapi OAuth client id in the app's “f 参数服务” panel; this id is unrelated to
Nintendo credentials. The imink-compatible endpoint remains available as a
manual fallback. The core crate exposes both provider URLs through
`SplatNetAuthConfig`.

Authentication failures are also written to the desktop app's `auth.log`
inside its platform application-config directory. The log records stages and
HTTP-safe error text only; it never records login links, session tokens, or
SplatNet tokens.
