# Android app

Not started. Depends on `bindings/mobile` (UniFFI Kotlin bindings) and,
transitively, on `core/sync` / `core/merge` having real implementations.

`CredentialProvider` (see `core/sync`'s module doc) will be implemented
here against the Android Keystore / `EncryptedSharedPreferences`.
