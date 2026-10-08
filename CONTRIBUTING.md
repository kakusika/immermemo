<!-- Generated from tmtroot/contributing.tmt. Edit that, then `tomet export .`. -->

# Immermemoへの貢献ガイド

## ドキュメントポリシー (SSoT)

- すべてのMarkdownファイル（`README.md`, `AGENTS.md`, `CONTRIBUTING.md` など）は自動生成される成果物です。
`.md` ファイルを直接編集してはいけません。
- ドキュメントの変更は、必ず `tmtroot/` 配下の `.tmt` ファイルを編集してください。
- 編集後は `just docs`（または `tomet export .`）でMarkdownを更新し、`just docs-check` で確認します。

## コミット規約 (Conventional Commits)

すべてのコミットは Conventional Commits 形式に従う必要があります: `type(scope): description`。

### Types

- `feat`: 新機能の追加
- `fix`: バグ修正
- `docs`: ドキュメントの変更（`.tmt` または生成ドキュメント）
- `refactor`: 挙動を変更しないコード構造の整理
- `perf`: パフォーマンス改善
- `test`: テストの追加・修正
- `chore`: ビルド設定、依存関係、ツール整備

### Scopes

変更対象のクレートまたはレイヤーをスコープとして指定します:

- **App**: `app`, `ui`, `android`
- **Crates**: `sync`, `merge`, `index`, `vault`
- **Vocab**: `vocab` (`mobile.vocabulary.tmt`)
- **Infra / Meta**: `nix`, `just`, `deps`, `docs`

> [!tip]
> 複数のクレートにまたがる変更の場合は、スコープを省略するか広域カテゴリ（例: `refactor(crates): ...`）を使用してください。

## クリーンコミット方針

Do not include AI session metadata or trailers (e.g. `Claude-Session:`, `Co-Authored-By:`) in commit messages.
Keep commit logs clean and focused on technical changes.

## 検証フロー

コミット前に以下のチェックを実行してください:

- `cargo check --workspace` / `cargo test --workspace`
- `just docs-check`: ドキュメントが最新かつ構文エラーがないことの確認

