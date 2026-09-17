# Flutter bindings

Exposes `immermemo-sync` and `immermemo-merge` to Dart via
[`flutter_rust_bridge`](https://cjycode.com/flutter_rust_bridge/), not
UniFFI -- UniFFI's own Dart support is community-maintained and not the
mainstream path for a Flutter app, where `flutter_rust_bridge` is the de
facto standard.

Not started: there is no `core` implementation to bind yet
(`core/sync`, `core/merge` are `todo!()`). The binding surface follows
`Vault` and its public methods directly -- `open`, `set_remote`,
`sync` -- plus `CredentialProvider` as a Dart-side callback, backed by
`flutter_secure_storage` (which itself wraps iOS Keychain / Android
Keystore), so this crate never touches OS-specific secret storage
directly.

`set_remote` takes a plain URL and is host-agnostic -- GitHub, Gitea, or
a self-hosted server all go through the same code path. Nothing here
special-cases which one the user pointed the app at.
