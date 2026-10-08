<!-- Generated from tmtroot/readme.tmt. Edit that, then `tomet export .`. -->

# Immermemo

イマーメモ、今メモ!
日々のメモ体験と、**ノートの保有者がユーザーであること**を意識したモバイルメモアプリ。

Immermemoは、構造化マークアップ言語 [Tomet](https://github.com/tomet-lang/tomet) (`.tmt`) を使用してノートを保存します。
ノートは特定のアプリに閉じ込められず、**Immermemoを使用しなくてもTometファイルとして編集・利用できます**。

## 特徴 (Features)

> [!summary]
> **いつでも手軽に記録**でき、**書いたノートを自分の手元に置いておける**メモアプリ。
> **オフラインでも制限なく編集**でき、リッチテキスト編集によって**記法を意識せず書ける**ほか、
> **同期時の競合も作業を止めずに保持する**ことで、**環境や同期状態を意識せず**、思考をそのまま記録できます。
> **高度な検索**や**ライブラリ**によって、蓄積したノートも必要なときにすぐ見つけられます。

- 
  - ソースコードを公開しながら開発されています。
また、メモに必要のない広告は表示されません。
- 
  - 保存できるノートの量に、アプリ側の制限はありません。
利用可能な容量は、お使いのデバイスや同期サービスに依存します。
- 
  - インターネット接続がなくてもノートを編集できます。
- 
  - GitHub / Git / Google Drive に対応しています。
自分のノートを、自分が選んだ場所に保存・同期できます。
- 
  - 同期時に競合の解決を強制しません。
Tometの `@conflict` を使用し、競合状態そのものをノートとして保持します。
そのため、競合が発生しても作業を中断することなく、記録を続けられます。
- 
  - 高度な検索
ライブラリによるノートの探索・整理
- 
  - Tometの記法を意識せず、通常のエディタのようにノートを編集できます。
- 
  - 素早くノートにアクセス

## リポジトリ構成 (Layout)

```
immermemo/
├── crates/
│   ├── sync/    # git2-backed vault sync
│   ├── merge/   # AST-level three-way merge
│   ├── index/   # SQLite-backed note index for listing and search
│   └── vault/   # vault/note/credential domain logic, UI-agnostic
├── vocab/
│   └── mobile.vocabulary.tmt   # @mobile.conflict vocabulary
└── app/         # The app: desktop + Android, Slint UI
```

## 開発ガイド (Development)

```sh
nix develop         # enters the dev shell (or use direnv)
just avd-create     # 初回のみ: AVD作成 (Pixel 6, API34, x86_64)
just emulator       # エミュレータ起動
just run            # ビルド → インストール → 起動
```

### 便利なレシピ

- `just logs`: 実行中アプリの logcat を追跡
- `just screenshot [path]`: 画面キャプチャ取得
- `just stop`: アプリを強制終了
- `just emulator-stop`: エミュレータ終了

