# Python モジュール import 機能 — 要件定義書

このドキュメントはコンテキスト圧縮後の実装再開用。

---

## 構文

```
import[py] module_name as alias
import[py] os.path as p            # サブモジュール
from py_module import[py] ClassName
from py_module import[py] ClassA, ClassB as CB
from os.path import[py] join, exists
```

- `[]` なしの `import` はまだ未実装
- `from ... import *` は非サポート
- `import[py]` の `[py]` は将来の他言語（`[js]` 等）拡張を見越した共通仕様

---

## Cargo.toml に追加

```toml
rustpython-parser = "0.4"
```

Python レキサー・パーサーは **このクレートで全て置き換える**。
`src/python_lexer.rs` / `src/python_parser.rs` は実装不要。

---

## Token / Lexer（変更なし）

- `Token::Import` / `Token::As` / `Token::From` はすでに存在する
- `[py]` は既存の `LBracket` + `Ident("py")` + `RBracket` で対応
- デコレーター（`@`）は字句解析のみ行い、構文解析で未実装エラーを出す

---

## AST 変更（`src/ast.rs`）

```rust
// Stmt enum に追加
Import {
    lang: String,                            // "py"（将来: "js" 等）
    module: Vec<String>,                     // ["os", "path"] for "os.path"
    alias: Option<String>,                   // Some("m") from "as m"
    body: Vec<Stmt>,                         // パース時に変換済みの tl AST
},
FromImport {
    lang: String,
    module: Vec<String>,
    names: Vec<(String, Option<String>)>,    // (元の名前, as エイリアス)
    body: Vec<Stmt>,                         // 型検査とキャッシュ用の完全な tl AST
},
```

`body` はパース時に Python ファイルを読み込み変換したもの。
型検査器がモジュール内クラス・関数を静的に把握するために必要。

---

## Parser 変更（`src/parser.rs`）

- `Parser` 構造体にソースファイルのディレクトリパスを保持するフィールドを追加
- `parse_stmt()` に `Token::Import` / `Token::From` のケースを追加
- `parse_import_stmt()` — `import[py] ...` を処理
- `parse_from_import_stmt()` — `from ... import[py] ...` を処理
- 両関数の中で Python ファイルを読み込み → `python_converter` を呼んで `body` を生成
- 複数名の同時 import をサポート: `from m import[py] A, B as BB`

---

## 新規ファイル: `src/python_converter.rs`

`rustpython_parser::ast::Mod` を受け取り `Vec<Stmt>`（tl AST）を返す変換層。

### 関数定義の変換

```python
def func(x: int, y, *args, **kwargs) -> str:
    ...
```
↓
```rust
Stmt::FnDef {
    name: "func",
    params: [
        Param { name: "x",        mutable: true, type_ann: Some("int") },
        Param { name: "y",        mutable: true, type_ann: Some("Any") },  // ヒントなし
        Param { name: "*args",    mutable: true, type_ann: Some("list[Any]") },
        Param { name: "**kwargs", mutable: true, type_ann: Some("dict[str, Any]") },
    ],
    return_type: Some("str"),
    ...
}
```

- 全パラメータに `mutable: true`
- 型ヒントなし → `type_ann: Some("Any")`
- `*args` → 名前を `"*args"` として格納（`is_varargs` フラグなし、名前で判別）
- `**kwargs` → 名前を `"**kwargs"` として格納

### クラス定義の変換

```python
class Foo(Base):
    class_var = 42          # クラス変数 → const
    typed_var: int = 0      # クラス変数（型ヒントあり）→ const

    def __init__(self, x: int) -> None:
        self.x = x          # インスタンス変数 → mut
        if True:
            self.y = 1      # 分岐内も含めてすべて探索 → mut
```
↓
```rust
Stmt::ClassDef {
    name: "Foo",
    bases: ["Base"],
    body: [
        Field { name: "class_var", kind: FieldKind::Const, type_ann: "Any", default: Some(Int(42)) },
        Field { name: "x",        kind: FieldKind::Mut,   type_ann: "int", default: None },
        Field { name: "y",        kind: FieldKind::Mut,   type_ann: "Any", default: None },
        // __init__ は FnDef として保持
    ]
}
```

- クラスレベル代入 → `const`（全インスタンス共有）
- `__init__` 内の `self.field = ...` → `mut`（ループ・分岐を含む全ノードを再帰探索）
- フィールドの型ヒントがあれば使用、なければ `"Any"`

### 未対応構文の扱い

| Python 構文 | 扱い |
|---|---|
| デコレーター `@decorator` | **ParseError**（後ほど実装） |
| 内包表記 `[x for x in y]` | ParseError |
| `with` 文 | ParseError |
| 多重代入 `a, b = c, d` | ParseError |
| `lambda` | ParseError |
| f-string `f"..."` | ParseError |
| `async def` / `await` | ParseError |
| `global` / `nonlocal` | 無視（サイレント） |
| `if __name__ == "__main__":` ブロック | 無視（ブロックごとスキップ） |

---

## Python 型ヒント → tl 型 マッピング

| Python 型ヒント | tl の型 |
|---|---|
| `int` | `int` |
| `str` | `str` |
| `float` | `float` |
| `bool` | `bool` |
| `None` / `-> None` | `None` |
| `list[T]` | `list[T]` |
| `dict[K, V]` | `dict[K, V]` |
| `tuple[T1, T2]` | `tuple[T1, T2]` |
| `Optional[T]` | `Option[T]` |
| `Union[T1, T2]` | `Union[T1, T2]` |
| `Any` | `Any` |
| カスタムクラス `Foo` | `NamedInstance("Foo")` |
| ヒントなし | `Any` |

---

## インタープリタ変更

### `Value` に追加（`src/interpreter.rs`）

```rust
Namespace(Rc<NamespaceData>)
```

```rust
struct NamespaceData {
    name: String,                     // モジュール名またはエイリアス
    members: HashMap<String, Value>,
}
```

属性アクセス（`.`）は既存の `Expr::Attr` の評価を流用。
`Value::Namespace` に対して `.attr` → `members.get(attr_name)` を返す。

**新演算子は不要**（`::` は既存の TraitAccess で使用中のため `::` は使わない）。

### モジュールキャッシュ

`Interpreter` 構造体に追加:
```rust
module_cache: HashMap<(String, PathBuf), ModuleState>

enum ModuleState {
    Loading,                 // 循環 import 検出用
    Loaded(Rc<NamespaceData>),
}
```

### `Stmt::Import` の実行フロー

1. `(lang, resolved_path)` をキーにキャッシュを検索
2. `Loading` → **RuntimeError**: `"circular import: <module_name>"`
3. `Loaded(ns)` → キャッシュから取得
4. 未登録の場合:
   - キャッシュに `Loading` をセット
   - `body`（変換済み tl AST）を孤立スコープで実行
   - トップレベル名を収集して `NamespaceData` を生成
   - キャッシュを `Loaded(ns)` に更新
5. `alias` で `Value::Namespace` をスコープにバインド

### `Stmt::FromImport` の実行フロー

1. 上記と同じ手順でモジュールをロード（キャッシュ利用）
2. `names` の各 `(name, alias)` を `namespace.members` からルックアップ
3. `alias.unwrap_or(name)` として現在スコープにバインド
4. 見つからない場合 → **RuntimeError**: `"cannot import name '<name>' from '<module>'"` 

### `*args` / `**kwargs` の呼び出し規約

インタープリタが関数呼び出し時にパラメータを処理する際：
- `name == "*args"` のパラメータがあれば、余った位置引数を `Value::List` にまとめて束縛
- `name == "**kwargs"` のパラメータがあれば、余ったキーワード引数を `Value::Dict` にまとめて束縛

---

## 型検査器変更（`src/type_check.rs`）

### `InferredType` に追加

```rust
Namespace(HashMap<String, InferredType>)
```

### `Stmt::Import` の型検査

1. `body` 内の `ClassDef` → `member_types[class_name] = InferredType::TypeVal`
2. `body` 内の `FnDef` → `member_types[fn_name] = InferredType::Unknown`（シグネチャ登録）
3. `alias → InferredType::Namespace(member_types)` をスコープに登録

### `Stmt::FromImport` の型検査

1. モジュールの `InferredType::Namespace` から各 `name` の型を取得
2. `alias.unwrap_or(name)` として直接スコープに登録

### Python クラスの型検査方針

- Python コードの**内部**（関数本体・クラス本体）は型検査しない
- Python クラスの**コンストラクタ呼び出し** (`pm.PyClass(args)`) → `InferredType::NamedInstance("PyClass")`
- tl 関数の引数として渡すときは型検査される（例: `Self` 型不一致など）

---

## モジュール解決（検索パス）

`import[py] mymodule` のファイル検索順：

1. `.tl` ソースファイルと**同じディレクトリ**（`mymodule.py`）
2. サブモジュール: `module = ["os", "path"]` → `<tl_dir>/os/path.py`

環境変数 `PYTHONPATH` による追加パスは未実装（後ほど追加予定）。

---

## テスト用 Python ライブラリ（リポジトリ内に作成）

デコレーターを使わない小規模ライブラリを `examples/` 内に配置。

```python
# examples/py_calculator.py

class Calculator:
    def __init__(self, initial: int) -> None:
        self.value = initial

    def add(self, x: int) -> int:
        self.value += x
        return self.value

    def reset(self) -> None:
        self.value = 0


def add_two(a: int, b: int) -> int:
    return a + b
```

---

## テスト用 .tl ファイル（作成予定）

### `examples/py_import.tl`（正常系）

```tl
import[py] py_calculator as calc

let c = calc.Calculator(10)
print(c.add(5))      # 15
print(c.add(3))      # 18
c.reset()
print(c.value)       # 0

let r = calc.add_two(3, 4)
print(r)             # 7

from py_calculator import[py] add_two as add
print(add(1, 2))     # 3
```

### `examples/py_import__errors.tl`（エラー系）

型検査エラー・循環 import エラーなどを確認するサンプル。

---

## 実装ファイル一覧（変更・新規）

| ファイル | 変更種別 | 内容 |
|---|---|---|
| `Cargo.toml` | 変更 | `rustpython-parser = "0.4"` 追加 |
| `src/ast.rs` | 変更 | `Stmt::Import` / `Stmt::FromImport` 追加 |
| `src/parser.rs` | 変更 | `parse_import_stmt()` / `parse_from_import_stmt()` 追加。`Parser` にソースディレクトリを保持 |
| `src/python_converter.rs` | **新規** | rustpython AST → tl AST 変換 |
| `src/interpreter.rs` | 変更 | `Value::Namespace` / `NamespaceData` 追加、モジュールキャッシュ追加 |
| `src/interpreter/exec.rs` | 変更 | `Stmt::Import` / `Stmt::FromImport` の実行処理 |
| `src/interpreter/eval.rs` | 変更 | `Value::Namespace` の属性アクセス対応 |
| `src/interpreter/ops.rs` | 変更 | `Value::Namespace` の `type_name` / `display` 対応 |
| `src/type_check.rs` | 変更 | `InferredType::Namespace` 追加、import 文の型検査 |
| `examples/py_calculator.py` | **新規** | テスト用 Python ライブラリ |
| `examples/py_import.tl` | **新規** | 正常系サンプル |
| `examples/py_import__errors.tl` | **新規** | エラー系サンプル |

---

## 実装しない機能（後回し・非サポート）

- `import`（`[]` なし）— 後ほど実装
- `from ... import *` — 非サポート（実装しない）
- デコレーター実行 — パースエラーのみ（後ほど実装）
- `PYTHONPATH` 環境変数によるパス追加 — 後ほど実装
- Python 以外の言語からの import（構文は共通だが変換層は未実装）
- tl ネイティブの `namespace` キーワード — `import` したモジュールが `Value::Namespace` を担うため不要
