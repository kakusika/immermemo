# Task: Input Toolbar (EditorToolbar)

## Overview
immermemoのエディタ入力中に表示されるコンテキスト連動型ツールバーを実装する。
ボタンは基本的にピル形状（Pill style）を採用し、左・中央・右の3分割構成で進める。

- 左: Undo / Redo（合体ピル）
- 中央: 状態に応じたコンテキスト領域（合体ピル：`@`, リスト, 見出し）
- 右: キーボード格納 / 完了（円形Doneピル）

## Steps
- [x] 1. 既存のピルコンポーネント（PillButton / NavPill）のスタイル・仕様確認
- [x] 2. `EditorToolbar` コンポーネントを設計・実装（Slint）
- [x] 3. `editor.slint` に `EditorToolbar` を組み込み、レイアウト・配置を調整
- [x] 4. コールバック（undo, redo, done, アクション）の配線
- [x] 5. Linux デスクトップ環境でビルド＆動作確認（`cargo check` / `cargo test`）
- [x] 6. フィードバック反映（第1弾）：
  - 中央の3アクションを1つの合体ピル（`ContextActionsPill`）に統合
  - 右のDoneボタンが縦長になっていたアスペクト比を修正（`width: self.height` の正円ピル）
  - ヘッダー（TopBar）にあった二重のUndo/Redoボタン（`UndoRedoBubble`）を削除
- [x] 7. フィードバック反映（第2弾）：
  - 編集モード前の鉛筆FABをツールバーDoneピルと同じ大きさ（`root.pill-height` = 48px）に統一
  - カーソル位置でのスマート挿入：
    - `@` はカーソル位置（または選択テキスト置換）に挿入し、直後にキャレット移動
    - `- `（リスト）や `# `（見出し）は、カーソルがある行の先頭（行頭）にスマート挿入
    - 挿入後もエディタのフォーカス（`editor.focus()`）を維持
