# Arrow 文法リファレンス

各文法要素についての字句解析・構文解析・AST 実行の解説。

---

## ファイル一覧

| ファイル | 内容 |
|---|---|
| [00_overview.md](00_overview.md) | パイプライン全体図・ファイル読み込み・CLI オプション |
| [01_lexer.md](01_lexer.md) | 字句解析・トークン種別・インデント処理・文字列/数値リテラル |
| [02_variables.md](02_variables.md) | 変数宣言 (let/mut/const/static)・スコープ・クロージャ |
| [03_expressions.md](03_expressions.md) | 演算子優先順位・各種式・型ガード・キャスト・ブロック式 |
| [04_control_flow.md](04_control_flow.md) | if/match/for/while/break/continue・制御構文の式としての使い方 |
| [05_functions.md](05_functions.md) | fn/gen・パラメータ・テンプレート・オーバーロード・デコレータ |
| [06_classes_traits.md](06_classes_traits.md) | クラス・trait・new_type・enum・アクセス制御 |
| [07_exceptions.md](07_exceptions.md) | try/except/finally/raise・組み込み例外クラス |
| [08_type_system.md](08_type_system.md) | 型アノテーション・静的型検査・型推論・型ガードナロイング |
| [09_imports.md](09_imports.md) | import・from import・言語タグ・モジュールキャッシュ |
| [10_special_features.md](10_special_features.md) | block_return/loop_yield/yield・async・break_point・数学文字列 |
| [11_events.md](11_events.md) | Signal[T]・on/once/off 購読・emit/emit_async・EventLoop・外部イベント |

---

## 関連ドキュメント

| ファイル | 内容 |
|---|---|
| [.claude/skills/parser-internals/SKILL.md](../../.claude/skills/parser-internals/SKILL.md) | パーサー実装リファレンス |
| [.claude/skills/interpreter-internals/SKILL.md](../../.claude/skills/interpreter-internals/SKILL.md) | インタープリタ実装リファレンス |
| [.claude/skills/importation/SKILL.md](../../.claude/skills/importation/SKILL.md) | インポートシステム実装詳細 |
| [.claude/skills/type-checking/SKILL.md](../../.claude/skills/type-checking/SKILL.md) | 型システム実装詳細 |
| [.claude/skills/partial-compile/SKILL.md](../../.claude/skills/partial-compile/SKILL.md) | 部分コンパイル実装詳細 |
| [.claude/skills/c-abi-interop/SKILL.md](../../.claude/skills/c-abi-interop/SKILL.md) | C ABI 相互運用設計仕様 |
| [spec.md](../../spec.md) | 言語仕様概要 (日本語) |
| [docs/language_comparison.md](../language_comparison.md) | Rust/Python/Arrow 比較表 |
