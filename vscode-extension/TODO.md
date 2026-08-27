# VS Code Extension TODO

VS Code extension updates for `Arrow` (`.ar`) files.

## 解析基盤（2026-08-28 に置き換え済み）

行単位の正規表現による近似解析（`analysis.ts` / `type_infer.ts` ほか計 5,471 行）を廃止し、
Rust 実装そのもの（`crates/arrow-frontend` を wasm32 化したもの）に置き換えた。
`src/lexer` / `src/parser` / `src/type_check` を `#[path]` で取り込んでいるので、
**文法を追加すれば再ビルドだけで拡張が追随する**。

- [x] Diagnostics — 型検査器のエラー・警告をそのまま表示
- [x] Hover — 型・const/let/mut 属性・docstring・継承元 trait・アクセス指定
- [x] Inlay hints — 型注釈が無い宣言に推論型（初期化式の node-id 経由）
- [x] Semantic tokens — 宣言種別に基づく色分け
- [x] Completion — スコープ木に基づく可視名、`.` アクセスはメンバ表
- [x] Signature help — 関数・自動生成コンストラクタ
- [x] Go to definition — 宣言位置

置き換えで直った既知の不具合:
- `protocol` が宣言として認識されず、メソッドがトップレベル関数として誤登録されていた
- 無関係な同名変数に対する偽の `Variable 'x' is already declared`
- `.` 補完がスコープを無視してファイル内の全シンボル（128 件）を返していた
- `MyEnum.a.value` のような enum メンバのチェーンアクセスが空だった
- Rust の lexer が持つ 55 キーワードのうち 16 個を解析側が知らなかった

## 残っている手作業

- [ ] `syntaxes/arrow.tmLanguage.json` — TextMate の色付けは解析前に走る別系統なので、
      キーワード追加時は引き続き手で追随する。現時点で未反映: `alias` `case` `off` `on`
      `once` `protocol`
- [ ] `src/parser/imports_editor.rs` — import 構文を変えたときのみ追随が必要
      （`scripts/compare_wasm_frontend.ps1` がずれを検出する）

## 検証

| 目的 | コマンド | 期待値 |
|---|---|---|
| 拡張と `arrow.exe` の診断一致 | `./scripts/compare_wasm_frontend.ps1` | `INVENTED: 0` / `parse mismatch: 0` |
| プロバイダの回帰 | `stress.js`（VS Code 同梱 Node で実行） | `threw: 0` / `hover misses: 0` / `def misses: 0` |
| 単一ファイルの詳細 | `node run_debug.js <file.ar>` | 7 機能すべての出力 |
