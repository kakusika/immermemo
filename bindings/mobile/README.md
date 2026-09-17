# Mobile bindings

Exposes `immermemo-sync` and `immermemo-merge` to Kotlin and Swift via
[UniFFI](https://mozilla.github.io/uniffi-rs/), from one interface
definition.

Not started: there is no `core` implementation to bind yet
(`core/sync`, `core/merge` are `todo!()`). The binding surface will
follow `Vault` and its public methods directly -- `open`, `set_remote`,
`sync` -- plus `CredentialProvider` as a UniFFI callback interface so
`apps/ios` and `apps/android` can each supply their own OS keychain
access.
