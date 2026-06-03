# インポートシステム

---

## 基本構文

```hv
import module.submodule
import module.submodule as alias
from module import Name1, Name2
from module import Name as Alias
```

`Stmt::Import { lang, module, with_file, alias, body }`  
`Stmt::FromImport { lang, module, with_file, names, body }`

---

## 言語タグ

インポート時に `[lang]` タグで読み込む形式を指定します。

```hv
import[hv]      mod          # .hv ファイルを強制読み込み
import[hvc]     mod          # .hvc (コンパイル済み) を強制読み込み
import          mod          # 自動選択 (.hvc 優先、なければ .hv)
import[py]      os.path      # Python ファイルをコンバータで変換して読み込み
import[py-int]  numpy as np  # Python インタープリタ (PyO3) 経由で読み込み
import[rs]      regex        # Rust crate を直接読み込み
import[cpp-dll] DxLib.DxLib with stub as dx  # C++ DLL を読み込み
import[cpp-lib] MyLib.MyLib with stub as ml  # C++ 静的ライブラリを読み込み
```

---

## モジュール検索パス

1. **`source_dir`**: 現在のソースファイルのディレクトリ
2. **`root_dir`**: メインエントリポイントのディレクトリ

`import foo.bar.baz` → `foo/bar/baz.hv` または `foo/bar/baz/` (パッケージ) を検索

---

## パッケージ

ディレクトリに `__init__.hv` を置くことでパッケージになります。

```
geometry/
├── __init__.hv
├── point.hv
└── vector.hv
```

```hv
import geometry               # geometry/__init__.hv を読み込む
import geometry.point         # geometry/point.hv を読み込む
from geometry import Vector   # geometry/__init__.hv 内の Vector を取得
```

---

## モジュールキャッシュと循環 import 検出

```rust
module_cache: HashMap<(String, PathBuf), Vec<Stmt>>
loading:      HashSet<PathBuf>
```

- 同じモジュールを複数回 import しても1回しかパースされません
- `loading` に現在ロード中のパスを追加し、同じパスが再度ロードされたら循環 import エラー

---

## .hv / .hvc 自動選択

`lang = "tl-auto"` (タグなし) のとき:

1. `.hvc` ファイルが存在すれば `.hvc` を優先
2. なければ `.hv` を読み込む

`.hvc` には埋め込みソーステキストが含まれており、実行は通常どおり行われます。  
ネイティブコンパイル済み関数は DLL から呼び出されます。

---

## Python モジュール (`[py]`)

Python ソースファイルを Havakyrie の AST に変換してインポートします。

```hv
import[py] math as m
from[py] os.path import join, exists
```

**変換の制限**:
- Python の `class` → `fn __init__` を持つ Havakyrie クラスに変換
- Python の `def` → `fn` に変換
- `*args` / `**kwargs` → `AdditionalParam` dict として渡す

関数本体内での変数ホイスト (if ブランチで代入された変数の前宣言) も自動で行われます。

---

## Python インタープリタ連携 (`[py-int]`)

PyO3 を介して Python インタープリタを呼び出します。

```hv
import[py-int] numpy as np
let arr = np.array([1, 2, 3, 4])
let mean = np.mean(arr)
```

**特徴**:
- `.pyi` スタブファイルがあれば型チェックに使用
- 実行時は Python インタープリタを呼び出す
- GIL (Global Interpreter Lock) により並列化は不可
- `Value::PyObject` として扱われる

---

## Rust crate (`[rs]`)

`hv_config.json` で `rust.crates_path` を設定すると Rust crate を直接読み込めます。

```json
{
  "rust": { "crates_path": "/path/to/registry/src/..." }
}
```

```hv
import[rs] regex as re
let r = re.Regex("\\d+")
```

**対応する Rust 型**:
- `i*`/`u*` → `int`
- `f32`/`f64` → `float`
- `bool` → `bool`
- `String`/`&str` → `str`

クレートの `src/lib.rs` から `pub fn` と `pub struct` を自動検出して  
LLVM IR ラッパーを生成します。

---

## C++ DLL / 静的ライブラリ (`[cpp-dll]` / `[cpp-lib]`)

C/C++ ヘッダファイルを型スタブとして読み込みます。

```hv
import[cpp-dll] DxLib.DxLib with stub as dx
import[cpp-lib] MyMath.VecMath with stub as vm
```

`with stub` の後にヘッダパスを指定します。  
`Dir.Name` 形式 → `Dir/Name.h` のパスが検索されます。

**制限**:
- C++ のオーバーロード・テンプレート・名前マングリングは非対応
- `hv_config.json` でコンパイラパス・追加フラグを設定する必要があります

---

## import の実行タイミング

import 文はパース時に実行されます (`parse_import_stmt`):

1. モジュールファイルを読み込み
2. 字句解析・構文解析して AST を生成
3. 生成した AST を `Stmt::Import.body` に埋め込む

型検査・実行フェーズでは `body` を参照するだけです。  
これにより型検査でインポート先の型情報が利用できます。

---

## from import の動作

```hv
from geometry import Vector, Matrix as Mat
```

1. `geometry` モジュール全体の AST が `body` に格納される
2. 実行時に `body` を実行してモジュール名前空間を構築
3. `names` に列挙された名前をモジュール名前空間から取り出して現在スコープに登録

`Stmt::FromImport` と `Stmt::Import` はどちらも `body` にモジュール全体の AST を持ちます。
