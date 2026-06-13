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

---

## 関連ドキュメント

| ファイル | 内容 |
|---|---|
| [for_claude/parser.md](../../for_claude/parser.md) | パーサー実装リファレンス |
| [for_claude/interpreter.md](../../for_claude/interpreter.md) | インタープリタ実装リファレンス |
| [for_claude/importation.md](../../for_claude/importation.md) | インポートシステム実装詳細 |
| [for_claude/typing.md](../../for_claude/typing.md) | 型システム実装詳細 |
| [for_claude/partial_compile.md](../../for_claude/partial_compile.md) | 部分コンパイル実装詳細 |
| [spec.md](../../spec.md) | 言語仕様概要 (日本語) |
| [docs/language_comparison.md](../language_comparison.md) | Rust/Python/Arrow 比較表 |
