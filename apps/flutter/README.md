# Flutter app

One codebase for iOS and Android. Not started -- depends on
`bindings/flutter` (Dart bindings via `flutter_rust_bridge`) and,
transitively, on `crates/sync` / `crates/merge` having real implementations.

No native (Kotlin/Swift) app is planned. If one is ever needed later,
`crates/sync` and `crates/merge` don't need to change for it -- they don't
know or care what sits on top of `bindings/*` -- so there is nothing to
keep open here now.

Frontend design (note list/editor UI, the conflict-resolution screen)
has not started; none of it is decided yet.
