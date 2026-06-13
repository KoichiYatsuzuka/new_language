# VS Code Extension TODO

VS Code extension updates for `Arrow` (`.ar`) files.

## Mouse Over
- [x] マウスオーバーによって型と、その行でのconst/let/mut属性が表示されるようにする。
- [x] マウスオーバーによってdocstringが表示されるようにする。
- [x] マウスオーバーによって型の継承元traitを表示させる

## Syntax Highlighting

- [x] ハイライトする予約語の更新
- [x] クラスのデフォルトコンストラクタをクラス名と同じ色にする

### 予約語チェックリスト

`CLAUDE.md` の言語仕様と `src/lexer.rs` のキーワード定義を VS Code 拡張の TextMate grammar に反映する。

- [x] 変数宣言: `let`, `mut`, `const`, `freeze`
- [x] リテラル: `True`, `False`, `None`
- [x] 論理・比較キーワード: `and`, `or`, `not`, `in`, `not in`, `is`, `is not`
- [x] 制御構文: `if`, `elif`, `else`, `match`, `for`, `while`, `block`
- [x] ジャンプ文: `break`, `continue`, `pass`, `return`, `yield`, `yield from`, `block_return`, `block_yield`
- [x] 例外処理: `try`, `except`, `finally`, `raise`
- [x] 定義: `fn`, `gen`, `class`, `trait`, `lambda`, `template`
- [x] import: `import`, `from`, `as`
- [x] スコープ・コンテキスト: `del`, `global`, `nonlocal`, `with`
- [x] async: `async`, `await`
- [x] assertion: `assert`
- [x] 型キーワード: `Self`, `new_type`, `Any`, `Union`, `Option`

### 現在の grammar との差分メモ

- [x] `freeze` を追加する
- [x] `gen` を追加する
- [x] `trait` を定義キーワードとして追加する
- [x] `Self`, `new_type`, `Any`, `Union`, `Option` を型・言語キーワードとして追加する
- [x] `yield from`, `not in`, `is not` の複合キーワードが単語境界で正しくハイライトされるか確認する

### 型推論
- [x] クラスの型を認識させる
- [x] 自動生成されるデフォルトコンストラクタの型を認識させる。
