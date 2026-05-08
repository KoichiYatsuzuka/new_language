# リファクタリング候補リスト

調査日: 2026-05-08

---

## 1. コードの重複（DRY違反）

### 1-1. 複合代入演算子のトークン→BinOpマッピングが重複
- **ファイル**: [src/parser.rs](src/parser.rs)（247-278行付近）
- **問題**: `PlusEq`, `MinusEq` などの演算子トークンから `BinOp` への変換が、変数代入と属性代入で別々に書かれている
- **改善方針**: `fn token_to_compound_op(token: &Token) -> Option<BinOp>` を抽出して共通化

### 1-2. `check_binop` 内での演算子文字列変換が重複
- **ファイル**: [src/type_check.rs](src/type_check.rs)（895-925行付近）
- **問題**: `Any`チェック用と`Union`チェック用で、`BinOp`を文字列に変換するコードが2回繰り返されている
- **改善方針**: `fn binop_to_string(op: &BinOp) -> &'static str` ヘルパーを抽出

### 1-3. キーワード定義情報の二重管理
- **ファイル**: [src/lexer.rs](src/lexer.rs)（359-412行付近）
- **問題**: キーワードの列挙と `Display` 実装でほぼ同じ情報が重複している
- **改善方針**: マクロで一元的に定義するか、`Token::as_keyword_str()` を唯一の真実源にする

---

## 2. 長すぎる関数・メソッド

### 2-1. `parse_stmt` が肥大化している
- **ファイル**: [src/parser.rs](src/parser.rs)（109-285行付近、約176行）
- **問題**: 複数の異なる文法構造を一度に処理しており、特に識別子で始まる文（代入・複合代入・属性代入）の処理が複雑に絡み合っている
- **改善方針**: `fn parse_assign_or_call()`, `fn parse_attr_assign()` を抽出して分割

### 2-2. `parse_class_def` が多責務
- **ファイル**: [src/parser.rs](src/parser.rs)（537-669行付近、約132行）
- **問題**: trait必須フィールドの収集、自動`__init__`生成、フィールド検証が1つの関数に混在している
- **改善方針**: `fn collect_trait_required_fields()` と `fn generate_auto_init()` を分離

### 2-3. `infer` が巨大（型推論）
- **ファイル**: [src/type_check.rs](src/type_check.rs)（591-868行付近、約278行）
- **問題**: 関数呼び出し推論（625-809行）が特に長く、Selfチェック・引数検証・戻り値型推論がすべて混在している
- **改善方針**: `fn infer_call()` を独立メソッドに分離し、Selfチェックを `fn check_self_type_call()` として抽出

### 2-4. `lex_number` に複数の数値リテラルが集中
- **ファイル**: [src/lexer.rs](src/lexer.rs)（279-348行付近、約70行）
- **問題**: 16進数・8進数・2進数・浮動小数点数・指数表記をすべて1関数で処理している
- **改善方針**: `fn lex_radix_int(base: u32)` を共通化し、各形式のデコードを分離

---

## 3. 複雑すぎるmatch式

### 3-1. `parse_primary` 内のmatchが巨大
- **ファイル**: [src/parser.rs](src/parser.rs)（1179-1258行付近、約79行）
- **問題**: リテラル・括弧グループ・タプル・リスト・辞書の解析が1つのmatchに混在している
- **改善方針**: `fn parse_tuple_or_group()`, `fn parse_list_literal()`, `fn parse_dict_literal()` を独立メソッド化

### 3-2. テンプレート呼び出しとsubscriptの判定が混在
- **ファイル**: [src/parser.rs](src/parser.rs)（1150-1172行付近）
- **問題**: `Token::LBracket` の処理内でテンプレート呼び出しとsubscriptアクセスの判定が同居している
- **改善方針**: `fn parse_bracket_suffix()` として抽出し、先読みロジックを明確に分離

### 3-3. `check_binop` のAny/Unionチェックが巨大match
- **ファイル**: [src/type_check.rs](src/type_check.rs)（892-934行付近）
- **問題**: 複数のmatchブロックで演算子の文字列変換が繰り返されており、条件分岐が複雑
- **改善方針**: `fn check_comparison_operands()` として分離

---

## 4. 責務の分離が不十分な箇所

### 4-1. ParserがTraitメタデータを直接管理
- **ファイル**: [src/parser.rs](src/parser.rs)（18-40行付近、`known_traits` フィールド）
- **問題**: Parserがtraitのフィールド・仮想メソッドのメタデータを保持・更新しており、字句構文解析の責務を超えている
- **改善方針**: `TraitRegistry` 構造体を別途実装し、Parserはそれを利用する形に変更

### 4-2. 自動`__init__`生成がパーサー内に組み込まれている
- **ファイル**: [src/parser.rs](src/parser.rs)（613-666行付近）
- **問題**: パースフェーズ内でAST変換（`__init__`の挿入）が行われており、単一責任の原則に反する
- **改善方針**: パース後に専用の`auto_init_pass`フェーズを設け、そこでAST変換を行う

### 4-3. インタープリタに組み込み例外クラス構築が混在
- **ファイル**: [src/interpreter.rs](src/interpreter.rs)（`Interpreter::new` 付近）
- **問題**: 組み込み例外クラスの生成がインタープリタ初期化コードに直接書かれている
- **改善方針**: `builtins.rs` または `exceptions.rs` モジュールに集約

### 4-4. lexerで複合キーワードを処理している
- **ファイル**: [src/lexer.rs](src/lexer.rs)（416-432行付近、`maybe_two_word`）
- **問題**: `not in`, `is not`, `yield from` のような2語キーワードをlexer内でスペースをスキップしながら判定しており、字句解析の責務を超えている
- **改善方針**: lexerは単語トークンとして出力し、parserで連続トークンを合成する方法を検討

---

## 5. エラーハンドリングの一貫性

### 5-1. `main.rs` のエラーハンドリングが統一されていない
- **ファイル**: [src/main.rs](src/main.rs)（30-82行付近）
- **問題**: `unwrap_or_else`, `expect`, `match` が混在しており、エラー処理のスタイルが不統一
- **改善方針**: `fn run_program(src: &str, filename: &str) -> Result<(), AppError>` を実装してエラーハンドリングを一元化

### 5-2. 数値パースエラーをサイレントに無視
- **ファイル**: [src/lexer.rs](src/lexer.rs)（291, 300, 309行付近）
- **問題**: `from_str_radix(...).unwrap_or(0)` により、不正な数値リテラルがサイレントに0になる
- **改善方針**: パース失敗時に診断情報（Span付きエラー）を返す

### 5-3. パースエラーメッセージが不十分
- **ファイル**: [src/parser.rs](src/parser.rs)（各所）
- **問題**: エラーメッセージが短すぎて（例: `"expected }"`）、何がどこで期待されたか不明
- **改善方針**: 期待トークン・実際のトークン・Spanを含む詳細なエラーメッセージに改善

---

## 6. 型設計の改善余地

### 6-1. `Expr` に `Span` が一貫して付与されていない
- **ファイル**: [src/ast.rs](src/ast.rs)（`Expr::BinOp` のみ span を持つ）
- **問題**: `Expr::BinOp` は span を持つが、他のExprバリアントは持っていないため、エラー位置報告に使えない場合がある
- **改善方針**: 全Exprにspanを含める、または `Spanned<Expr>` ラッパー型を導入

### 6-2. `InferredType::Unknown` と `Any` の意味が曖昧
- **ファイル**: [src/type_check.rs](src/type_check.rs)（28, 36行付近）
- **問題**: `Unknown`（推論失敗）と `Any`（意図的な動的型）の区別が名前に反映されておらず混同しやすい
- **改善方針**: `Unknown` → `Unresolved` に改名し、意味の違いを明確化

### 6-3. Token が Copy 非対応で clone コストが発生
- **ファイル**: [src/token.rs](src/token.rs), [src/parser.rs](src/parser.rs)
- **問題**: `Token` は `Clone` のみで `Copy` でないため、`self.current().clone()` のような呼び出しが多発している
- **改善方針**: `Ident`, `Str` などの文字列含有バリアントを `Arc<str>` 化し、全体を `Copy` にすることを検討

---

## 7. パフォーマンス改善の可能性

### 7-1. `Rc<RefCell<_>>` の過度な使用
- **ファイル**: [src/interpreter.rs](src/interpreter.rs)（`ClassValue`, `InstanceData` 定義付近）
- **問題**: シングルスレッドで同時変更が起きない箇所でも `Rc<RefCell<_>>` が使われており、実行時オーバーヘッドがある
- **改善方針**: 本当に内部可変性が必要な箇所を見極め、不要な `RefCell` を除去

### 7-2. 全文字位置の事前計算によるメモリ消費
- **ファイル**: [src/lexer.rs](src/lexer.rs)（6-21行付近、`positions` 配列）
- **問題**: ソース全文字に対して `(line, col)` を事前計算しており、大きなファイルではメモリ消費が大きい
- **改善方針**: オフセットから行・列をオンデマンド計算する方式に変更

### 7-3. 型推論で同じ引数式を複数回評価
- **ファイル**: [src/type_check.rs](src/type_check.rs)（625-809行付近）
- **問題**: 型チェック内で同一の引数式の型が複数回推論される可能性がある
- **改善方針**: 引数の型推論結果を一度だけ計算してキャッシュ

---

## 8. 命名の一貫性・明確性

### 8-1. `emit()` がエラー追加を意味している
- **ファイル**: [src/type_check.rs](src/type_check.rs)（383行付近）
- **問題**: `emit()` はコード生成の文脈で使われることが多く、エラー追加の意味として紛らわしい
- **改善方針**: `emit()` → `report_error()` または `add_error()` に改名

### 8-2. `is_virtual_body` / `is_virtual` の意味が反転しやすい
- **ファイル**: [src/parser.rs](src/parser.rs)（406-411行付近、`FnDef.is_virtual`）
- **問題**: 「仮想メソッドかどうか」と「実装が `...` だけかどうか」が混同されやすい命名
- **改善方針**: `is_ellipsis_body()` または `is_abstract_body()` に改名

### 8-3. スコープスタックの命名が意図を反映していない
- **ファイル**: [src/type_check.rs](src/type_check.rs)（`scopes` フィールド）
- **問題**: `Vec` の末尾が最内スコープという慣例が名前から読み取れない
- **改善方針**: `scope_stack` に改名し、`push_scope()` / `pop_scope()` メソッドで操作をラップ

### 8-4. 型引数と関数パラメータで `params` が使い回されている
- **ファイル**: [src/parser.rs](src/parser.rs)（`parse_type_expr` 内など）
- **問題**: 関数パラメータ（`Param`型）と型引数の両方に `params` という名前が使われており混同しやすい
- **改善方針**: 型引数を表す箇所は `type_params` または `type_args` に改名

---

## 優先度まとめ

| 優先度 | 番号 | 内容 |
|--------|------|------|
| 高 | 2-1 | `parse_stmt` を分割 |
| 高 | 2-2 | `parse_class_def` を分割 |
| 高 | 2-3 | `infer` から `infer_call` を抽出 |
| 中 | 1-1 | compound assign op の共通化 |
| 中 | 3-1 | `parse_primary` を分割 |
| 中 | 4-1 | `TraitRegistry` を分離 |
| 中 | 4-2 | auto-init 生成を別フェーズに |
| 中 | 5-1 | `main.rs` のエラー処理一元化 |
| 中 | 6-1 | 全 Expr に Span を付与 |
| 中 | 6-2 | `Unknown` → `Unresolved` 改名 |
| 中 | 7-1 | `Rc<RefCell<_>>` の見直し |
| 低 | 1-2 | `binop_to_string` 抽出 |
| 低 | 1-3 | キーワード定義マクロ化 |
| 低 | 2-4 | `lex_number` を分割 |
| 低 | 3-2 | `parse_bracket_suffix` 抽出 |
| 低 | 4-3 | builtins モジュール化 |
| 低 | 4-4 | lexer の複合キーワード処理をparserへ |
| 低 | 5-2 | 数値パースエラーの診断 |
| 低 | 6-3 | Token を Copy 化 |
| 低 | 7-2 | position 計算を遅延化 |
| 低 | 8-1 | `emit()` → `report_error()` 改名 |
| 低 | 8-2 | `is_virtual` → `is_ellipsis_body` 改名 |
| 低 | 8-3 | `scopes` → `scope_stack` 改名 |
| 低 | 8-4 | 型引数を `type_params` に改名 |
