# Python 互換の外部ライブラリ読み込みに向けた検討 ToDo

将来的に Python のような外部ライブラリを読み込めるようにするため、言語仕様と実装を Python 互換に寄せておくための検討項目です。

## 1. import 構文

- `import module`
- `import package.module`
- `import module as alias`
- `from module import name`
- `from module import name as alias`
- `from package.module import name1, name2`
- `from module import *`
- 相対 import の扱いを決める
  - `from . import x`
  - `from ..pkg import y`
- 複数行 import の構文を Python と互換にする
  - 括弧付き import
  - バックスラッシュ継続を許すかどうか

## 2. モジュール探索ルール

- Python の `sys.path` に相当する探索パスを設計する
- 実行ファイルのディレクトリを探索対象に含める
- カレントディレクトリを探索対象に含めるか決める
- 標準ライブラリ用の探索パスを決める
- 外部ライブラリ用の探索パスを決める
- 環境変数で探索パスを追加できるようにするか決める
- `.tl` モジュールと Python モジュールの優先順位を決める
- 同名モジュールの衝突解決ルールを決める

## 3. パッケージ仕様

- ディレクトリをパッケージとして扱う条件を決める
- Python 互換にするなら `__init__.py` 相当をどう扱うか決める
- `.tl` 用に `__init__.tl` を導入するか決める
- namespace package 相当をサポートするか決める
- パッケージ初期化の実行タイミングを決める
- 循環 import の途中状態をどう扱うか決める

## 4. モジュールオブジェクト

- import されたモジュールを値として扱えるようにする
- `module.name` による属性アクセスを定義する
- モジュール内の公開名・非公開名の扱いを決める
- `__name__`、`__file__`、`__package__` 相当を持たせるか決める
- `__all__` 相当を `from module import *` に使うか決める
- モジュールキャッシュを導入する
- 同じモジュールを複数回 import しても一度だけ実行する

## 5. 名前解決とスコープ

- import した名前を `let` / `mut` / `const` のどれにするか決める
- `import module` の束縛は原則 `const` にする
- `from module import name` の束縛も原則 `const` にする
- ローカルスコープ内 import を許すか決める
- import 名と既存変数名が衝突した場合のルールを決める
- `as` alias と既存の identifier ルールを揃える

## 6. Python ライブラリとの接続方式

- CPython を埋め込む方式にするか検討する
- Python C API / PyO3 / pyo3-ffi などの利用可否を検討する
- プロセス分離で Python を呼ぶ方式も検討する
- 純粋な `.tl` ライブラリと Python ライブラリを同じ import 構文で扱うか決める
- Python オブジェクトを `test_lang` の `Value` として包む型を用意する
- Python 例外を `test_lang` 側のエラーに変換する方針を決める
- Python の GIL やライフタイム管理をどう隠蔽するか検討する

## 7. 型システムとの接続

- Python ライブラリ由来の値をどの型として扱うか決める
- 型が分からない外部値用に `Any` 相当を導入するか検討する
- `.pyi` stub を読み込んで静的型検査に使うか検討する
- `typing` の `Optional` / `Union` / `Callable` / generic をどう対応させるか決める
- `test_lang` の `Union[...]` / `Option[...]` と Python typing の対応を決める
- Python クラスを `test_lang` の class / trait とどう対応させるか決める
- Python 関数の overload 情報をどう扱うか検討する
- 型情報がない場合は実行時チェックへ委ねるルールを明文化する

## 8. 呼び出し互換性

- 位置引数・キーワード引数を Python と同じ順序ルールにする
- default argument をサポートするか決める
- `*args` / `**kwargs` をサポートするか決める
- keyword-only argument をサポートするか決める
- positional-only argument をサポートするか決める
- Python 関数呼び出し時のエラーメッセージをどう寄せるか決める
- 戻り値の変換ルールを決める

## 9. 値変換

- `int` / `float` / `str` / `bool` / `None` の相互変換
- `list` / `tuple` / `dict` / `set` の対応方針
- `bytes` / `bytearray` をサポートするか決める
- Python iterator / generator を `test_lang` の generator と接続する
- Python object の属性アクセス・メソッド呼び出しを扱う
- Python 側の mutable object と `let` / `mut` / `freeze` の関係を決める
- 参照共有するかコピーするかを型ごとに決める

## 10. 未実装構文との整合

- 辞書リテラル `{k: v}` を Python 互換で設計する
- セットリテラル `{a, b}` を Python 互換で設計する
- tuple リテラル `(a, b)` を導入するか検討する
- slice 構文 `x[a:b:c]` を導入するか検討する
- subscript `x[i]` と代入 `x[i] = v` を設計する
- unpacking assignment を導入するか検討する
- comprehension を導入するか検討する
- decorator を導入するか検討する
- `with` 文を導入するか検討する
- lambda を導入するか検討する

## 11. 例外処理

- `try` / `except` / `finally` / `raise` を Python 互換に寄せる
- Python 例外クラスをどう表現するか決める
- `except SomeError as e` をサポートするか決める
- traceback / stack trace の表示方針を決める
- import 失敗時の例外種別を決める

## 12. 標準ライブラリ方針

- Python 標準ライブラリをそのまま呼べるようにするか決める
- `stdlib/` の `.tl` 標準ライブラリとの住み分けを決める
- `math`、`json`、`os`、`pathlib` など主要モジュールの扱いを決める
- 安全性のために利用可能モジュールを制限するモードを用意するか検討する

## 13. パッケージ管理

- `pip` で入れたライブラリを探索対象に含めるか決める
- 仮想環境 `.venv` を検出するか決める
- `pyproject.toml` / `requirements.txt` との連携を検討する
- `.tl` 用パッケージ管理を別に作るか検討する
- lockfile を持つか検討する

## 14. 実装上の追加が必要なもの

- `Token` に `Import`、`From`、`As` などを追加する
- `Stmt` に import 系 AST を追加する
- parser に import 文の構文解析を追加する
- interpreter に module loader を追加する
- type checker に import された名前の型登録を追加する
- モジュールキャッシュを `Interpreter` か別コンポーネントに持たせる
- ファイル単位で lexer/parser/type checker/interpreter を再帰的に呼べるようにする
- エラー表示に import chain を含める
- examples に正常系・エラー系 import サンプルを追加する
- spec に import / module / package の章を追加する

## 15. 先に決めたい設計判断

- `.tl` と `.py` の両方を同じ `import` で読めるようにするか
- Python 互換性を優先して構文を広げるか、`test_lang` の静的安全性を優先して制限するか
- Python オブジェクトを静的型検査でどこまで追うか
- `Any` を入れる場合、型安全性の境界をどう表示するか
- import は compile/type-check 時点で解決するか、実行時に解決するか
- 外部 Python 実行環境がない場合の挙動をどうするか
- セキュリティ制限が必要な実行モードを用意するか

## 16. 最初の実装候補

1. `.tl` ファイル同士の `import module` のみ実装する
2. モジュール探索パスとモジュールキャッシュを作る
3. `from module import name` を追加する
4. `as` alias を追加する
5. `.tl` パッケージと `__init__.tl` を追加する
6. 型検査で import 先の公開名を参照できるようにする
7. CPython 連携の小さな実験を別 feature として始める
8. Python の `.pyi` / `typing` 連携を検討する
