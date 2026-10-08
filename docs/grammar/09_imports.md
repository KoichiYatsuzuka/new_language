# インポートシステム

モジュールの呼び出し方は **CPython と同じ**です（2026-10-02）。探索規則の唯一の定義は
[src/module_path.rs](../../src/module_path.rs)、パッケージの扱いは
[src/parser/imports/packages.rs](../../src/parser/imports/packages.rs) にあります。
仕様を決めた経緯は [implementation_logs/IMPORT_RESOLUTION_PLAN.md](../../implementation_logs/IMPORT_RESOLUTION_PLAN.md)。

---

## 基本構文

```ar
import module.submodule              # 先頭の `module` を束縛する（`module.submodule.f()` と呼ぶ）
import module.submodule as alias     # `alias` に `module.submodule` を束縛する
from module import Name1, Name2
from module import Name as Alias
from . import sibling                # 相対: このファイルのディレクトリのモジュール
from ..pkg import Name               # 相対: 1 つ上のディレクトリの pkg から
import ..pkg.mod as m                # 相対（Arrow の拡張・CPython には無い書き方）
```

- `import` / `from … import` は**モジュールの最上位にだけ**書けます。関数・`if` / `for` などの中では
  `ParseError` です。
- `[lang]` タグは `import` の直後（`import[py] m`）と `from` 文の `import` の直後（`from m import[py] f`）に書きます。

AST:

```
Stmt::Import     { lang, module, source_module, alias, body, origin, bind }
Stmt::FromImport { lang, module, source_module, names, body, origin }
```

| フィールド | 内容 |
|---|---|
| `lang` | 言語タグ（`"ar-auto"` / `"ar"` / `"arc"` / `"py"` / `"py-int"` / `"rs"` / `"cpp-dll"` …） |
| `module` | モジュールの名前。Arrow / py は**ファイルごとに一意な名前**（後述「モジュールの同一性」）、cpp は解決済みヘッダパス、それ以外は書いた綴り |
| `source_module` | `module` が書いた綴りと違うときの、書いた綴り（`..util` など。エディタ用スタブの鍵） |
| `alias` | `as` の別名 |
| `body` | パース時に読み込んだモジュールの AST |
| `origin` | 探索の起点（`level` = 先頭のドットの数、`base_dir` = 起点ディレクトリ、`span` = 文の位置）。実行時の探索もこれを使う |
| `bind` | この文が束縛する名前と、そこへ入るモジュール（後述「束縛」）。`None` は束縛しない文（パッケージの連鎖を読み込むためにパーサが足した文） |
| `names` | `(元の名前, 別名)` のリスト |

---

## 言語タグ

インポート時に `[lang]` タグで読み込む形式を指定します。

```ar
import[ar]      mod          # .ar ファイルを強制読み込み
import[arc]     mod          # .arc (コンパイル済み) を強制読み込み
import          mod          # 自動選択 (.arc 優先、なければ .ar)
import[py]      mylib        # Python ファイルをコンバータで変換して読み込み
import[py-int]  numpy as np  # Python インタープリタ (PyO3) 経由で読み込み
import[rs]      regex        # Rust crate を直接読み込み
import[cpp-dll] DxLib.DxLib as dx            # C/C++ DLL を読み込み（ヘッダ DxLib/DxLib.h）
import[cpp-lib] MyLib.MyLib as ml            # C/C++ 静的ライブラリを読み込み
import[cs-dll]  MyLib.MyBridge as my         # .NET NativeAOT DLL を読み込み
import[cs-proc] MyLib.MyService as my        # .NET IPC サブプロセス経由で呼び出し
import[js-proc] out_debug.analysis as ana    # Node.js IPC サブプロセス経由で呼び出し
```

---

## 探索規則（全言語共通）

**どの言語タグでも同じ規則**で探します（`.ar` / `.arc` / py / py-int / cpp / cs / js）。
構文解析時の読み込みだけでなく、実行時の探索（`import[py-int]` の `sys.path`・C# のブリッジ DLL /
ホスト・js-proc の設定）も同じ起点を使います。

| 書き方 | 探す場所 |
|---|---|
| `import a.b` / `from a.b import x` | **エントリのディレクトリ**（CPython の `sys.path[0]`）→ 言語ごとの外部の探索先 |
| `import .a.b` / `from . import x` / `from .m import y` | import 文を書いたファイルのディレクトリ**だけ** |
| `import ..a.b` / `from .. import x` / `from ..m import y` | 1 つ上のディレクトリ**だけ**（ドットが 1 つ増えるごとにさらに 1 つ上） |

- ドット無しの書き方は、**書いたファイルのディレクトリを探しません**（CPython と同じく暗黙の相対
  import はありません）。パッケージの中から同じディレクトリのモジュールを指すときは
  `from . import util` か `import pkg.util` と書きます。
- エントリのディレクトリは、実行したファイル（`arrow main.ar` の `main.ar`）のあるディレクトリです。
  REPL などファイルから読んでいないときはカレントディレクトリです。
- `import a.b.c` は `a/b/c.arc` → `a/b/c.ar` → `a/b/c/__init__.ar` の順に探します（タグ無しの場合）。

**言語ごとの外部の探索先**（ドット無しの書き方のときだけ。ドット付きでは見ません）:

| 言語 | 外部の探索先 |
|---|---|
| `py` / `py-int` | `ar_config.json` の `python.search_paths` → `PYTHONPATH` → `$PYTHONHOME/Lib/site-packages` → Python の標準ライブラリ・site-packages |
| `cs-dll` / `cs-proc` | `ar_config.json` の `csharp.lib_paths` |
| `js-proc` | ブリッジ側の解決（`bridge_root`・npm パッケージ・Node.js 組み込み） |
| `.ar` / `.arc` / `cpp-*` | なし |

`ar_config.json` は**エントリのディレクトリから祖先へ**遡って最初に見つかったものを使います
（`python.search_paths` / `csharp.lib_paths` / `rust.crates_path` / `javascript`）。

> ⚠ 2026-10-02 午前の版では「ドット無しも書いたファイルのディレクトリから探し、エントリの
> ディレクトリは探さない」でしたが、CPython 準拠に改めました。それ以前は「書いたファイルの
> ディレクトリ → エントリのディレクトリ」の順で、サブディレクトリのファイルをエントリにすると
> 上の階層を指す手段がありませんでした。

**型の出所が見つからない import はエラー**です（言語によらず・構文解析の誤り）。
黙って（または警告だけ出して）型なしで続けることはしません。

| 言語 | 型の出所 | 見つからないとき |
|---|---|---|
| `.ar` / `.arc` | ソース・`.ars` | `cannot find module` |
| `py` / `py-int` | `.pyi` / `.py`（`py-int` は無ければ同梱スタブ）。`__init__` の無いディレクトリは名前空間パッケージ | `cannot find its types` — `.pyi` を置く |
| `cpp-dll` / `cpp-lib` | ヘッダ（`.h`） | `cannot read header` |
| `cs-dll` / `cs-proc` | DLL の .NET メタデータ | `cannot find '…dll'` — `csharp.lib_paths` に足す |
| `js-proc` | `.ars`（Node.js 組み込みのモジュールも含め、使う関数を `.ars` に書く） | `cannot find the type stub` |
| `rs` | crate のソース | `crate directory not found` など |

> ⚠ 2026-10-08 より前は `py-int` / `cpp-*` / `cs-*` / `js-proc` で見つからないと型なしで続けていたため、
> モジュールのメンバーがすべて型の分からないまま通り、間違いが実行時まで分からなかった
> （`implementation_plans/editor_import_resolution_plan.md` 1-2）。

---

## 相対 import

先頭のドットで、import 文を書いたファイルのディレクトリから数えた場所を指します。

```ar
# proj/pkg/sub/leaf.ar
from . import util             # proj/pkg/sub/util.ar
from .util import which        # proj/pkg/sub/util.ar の which
from .. import base            # proj/pkg/base.ar
from ..base import Unit        # proj/pkg/base.ar の Unit
import ..base                  # proj/pkg/base.ar（`base` を束縛・Arrow の拡張）
import ...lib.helper as h      # proj/lib/helper.ar（2 つ上）
```

- `.` が書いたファイルのディレクトリ、ドットが 1 つ増えるごとに 1 つ上です。
  字句解析器は `...` を 1 つのトークンにしますが、ドット 3 つとして数えます。
- `from . import x` の `x` は、このディレクトリのモジュール `x`（`x.ar` など）です。モジュールが無ければ、
  このディレクトリのパッケージ（`__init__.ar`）の中の名前 `x` を取り込みます。
- ドットの後にモジュール名の無い書き方は `from` の形だけです（`import .` は誤り）。
- `import[rs]` はクレート名を取るので、相対の書き方は誤りです。
- **CPython との差（Arrow の拡張）**:
  - `import .x` / `import ..x.y` も書けます（束縛は `import x.y` と同じく先頭の `x`）。
  - 相対 import は**エントリのファイルからも**使え、**エントリのディレクトリより上へも**遡れます
    （CPython ではどちらも誤り）。サブディレクトリのファイルをエントリにして上の階層を読むためです。

```ar
# proj/app/entry.ar をエントリにして実行する
from ..pkg import core         # proj/pkg/core.ar
import ..pkg.wrapper as w      # proj/pkg/wrapper.ar
```

---

## 束縛（`import a.b` は `a`）

CPython と同じく、`as` の無い `import a.b.c` は**先頭の `a`** を束縛します。

```ar
import pkg.sub.mod             # `pkg` を束縛する
print(pkg.sub.mod.f())         # 属性でたどる
print(mod.f())                 # ⛔ name 'mod' is not defined

import pkg.sub.mod as m        # `m` に pkg.sub.mod を束縛する
print(m.f())
```

- `from a.b import x` は `x` を束縛します（`a` / `b` は束縛しません）。
- 外部言語（`cpp-dll` / `cpp-lib` / `cs-dll` / `cs-proc` / `js-proc` / `rs`）のドット区切りは**ファイルの場所**で
  あってパッケージの階層ではないので、束縛は従来どおり**別名か最後の部分**（cpp はヘッダのファイル名）です。
- 束縛名はパーサが 1 か所（`Stmt::Import::bind`）で決め、実行時・型検査・型レジストリ・展開器はそれを読みます。

---

## パッケージ

ディレクトリがパッケージです。`__init__.ar` があればそれがパッケージの本体で、無ければ
**空の名前空間パッケージ**です（CPython の namespace package と同じ）。

```
geometry/
├── __init__.ar
├── point.ar
└── vector.ar
```

```ar
import geometry               # geometry/__init__.ar を実行し、`geometry` を束縛
import geometry.point         # geometry/__init__.ar → geometry/point.ar の順に読み込み、`geometry` を束縛
print(geometry.point.Point(1, 2))
from geometry import Vector   # geometry/__init__.ar の中の名前 Vector
from geometry import point    # point が __init__.ar の名前でなければ、サブモジュール geometry/point.ar を読み込む
```

- `import a.b.c` は `a` → `a.b` → `a.b.c` の順に読み込みます。各パッケージの `__init__` は 1 回だけ実行されます。
  パーサは、束縛しない import 文（`bind: None`）を元の文の**手前に**足して連鎖を表します。
- **読み込んだサブモジュールは親パッケージの属性になります**。どのファイルで読み込まれたかに関係ありません
  （CPython の `sys.modules` と同じ）。`import pkg` だけのファイルでも、別のところで `pkg.core` が
  読み込まれていれば `pkg.core` が見えます。
- `from a import b` は、`b` が `a` の名前に無ければ**サブモジュール `a.b` を読み込んで**束縛します。
- パッケージの `__init__.ar` の中から、そのパッケージのサブモジュールを読めます
  （`from . import core` / `from pkg.core import X`）。
  - ⚠ ただし `__init__.ar` の中の `import pkg.core`（`pkg` 自身を束縛する形）は、実行時に
    「パッケージが読み込まれていない」という誤りになります（CPython は初期化途中のパッケージを
    束縛しますが、そこまでは再現していません）。

---

## 名前空間のメンバー

モジュールの名前空間のメンバーは、CPython と同じく次のすべてです。

1. モジュールの最上位で宣言した名前（`let` / `const` / `mut` / `fn` / `class` / `enum` …）
2. **モジュールの中の `import` / `from … import` で束縛した名前（再エクスポート）**
3. 読み込まれたサブモジュール

```ar
# pkg/wrapper.ar
from . import core
from .core import Item
fn wrap(let v: int) -> Item:
    return core.make_item(v)
```

```ar
import pkg.wrapper
print(pkg.wrapper.core.LABEL)        # 再エクスポート（wrapper が import した core）
from pkg.wrapper import Item         # 再エクスポート（wrapper が core から取り込んだ Item）
print(pkg.core.LABEL)                # サブモジュール（wrapper が読み込んだ core は pkg の属性）
```

これらのどれでもない名前は**静的エラー**です（実行まで進めば `AttributeError` / `ImportError` / `NameError`）。

| 書き方 | 誤り |
|---|---|
| `m.nothing` | `module 'm' has no member 'nothing'`（`ModuleHasNoMember`） |
| `from m import nothing` | `cannot import 'nothing' from 'm'`（`CannotImportName`） |
| `let t: util.Tag`（`util` を束縛していない） | `type 'util.Tag' belongs to module 'util', which is not imported here; import it to use its types`（`UnimportedModuleType`） |

- モジュールで宣言した型は `モジュール名.型名` の型です（`tags.ar` の `Tag` は `tags.Tag`）。
  注釈には束縛した名前を通して書きます（`import tags as t` → `let x: t.Tag`、`import a.b` → `let x: a.b.Tag`）。
  `from tags import Tag` した `Tag` も同じ型です。再エクスポートで取り込んだ型は、元の宣言の型に解決されます。
- 型の表はプログラム全体で 1 つですが、**このファイルが束縛していないモジュールの型名は書けません**
  （CPython でも束縛していない名前は `NameError`）。別名で束縛していれば、使うべき綴りを案内します。
- ⚠ 外部言語のスタブ（cpp / cs / js / rs）と、VS Code 拡張では、無いメンバーを誤りにしません
  （拡張は 3-3 で CLI と同じにする・editor_import_resolution_plan.md）。

### グローバル変数の型と属性

import した後も、モジュールのグローバル変数の**型と属性**（`const` / `let` / `mut`）はそのまま分かります。
型は注釈が無くても推論した型です（`let LABEL = "x"` の `g.LABEL` は `str`）。
**書き換えられるかは型ではなく属性で決まります。**

| モジュールでの宣言 | `g.X = ..` | `g.X[0] = ..` / `g.X.f = ..` / `g.X.append(..)` |
|---|---|---|
| `const X` | 静的エラー `cannot assign to const 'g.X'` | 静的エラー `cannot modify the contents of const 'g.X'` |
| `let X` | 静的エラー `cannot assign to immutable variable 'g.X'` | 静的エラー（`let` の中身も書き換えられない） |
| `mut X` | **代入できる**（モジュールの大域そのものを書き換える。モジュールの関数も新しい値を見る） | 書き換えられる |
| 関数・クラス・`import` で束縛した名前 | 静的エラー（付け替えられない） | — |

- `mut` に代入する値は変数の型で検査します（`mut total = 0` に `g.total = "s"` は静的エラー）。
- `from g import X` で取り込んだ名前は、**付け替えられません**（`X = ..` は静的エラー）。
  **中身は元の宣言の属性に従います**: 元が `mut` のリストなら `X.append(..)` でき（モジュールと共有）、
  `let` / `const` ならできません。`int` などの値は import した時点の写しです（CPython と同じ）。
- VS Code 拡張も CLI と同じ処理で import 先を読むので、同じ型と属性が分かります。

---

## モジュールの同一性

モジュールは**ファイルごとに 1 つ**です。パーサが `module` を、エントリのディレクトリからの相対パスの
名前（`pkg.util` ＝ CPython のモジュール名）に書き換えます。

- 同じファイルは、どの綴りで import しても同じモジュールです（`import util` と、`pkg/` の中の
  `from .. import util` は同じ util.ar）。本体は 1 回だけ実行され、型も同じです。
- 違うファイルが同じ名前になることはありません（`util.ar` と `pkg/util.ar` は `util` と `pkg.util`）。
  名前を取り合ったら明示エラーです。
- エントリのディレクトリより上のディレクトリは `__parent__` で表します（`__parent__.util`）。型の名前にも
  使われます（`__parent__.util.UTag`）。
- Python の外部の探索先（site-packages など）で見つけたモジュールの名前は、書いた綴りのままです。

---

## モジュールキャッシュと循環 import 検出

```rust
module_cache: HashMap<(String, PathBuf), Vec<Stmt>>   // 鍵は絶対パスに正規化したファイル
loading:      HashSet<PathBuf>
```

- 同じモジュールを複数回 import しても 1 回しかパースされません。
- `loading` に現在ロード中のファイル（絶対パス・`..` を畳んだもの）を入れ、同じファイルが再度ロード
  されたら循環 import エラーです。パスを正規化するので、`..` を含む相互 import も検出できます。
- 実行時もモジュールの名前（上記の同一性）ごとに 1 回だけ本体を実行し、名前空間を共有します。
  読み込み中のモジュールを再び読み込もうとしたら実行時の循環 import エラーです。

---

## .ar / .arc 自動選択

`lang = "ar-auto"` (タグなし) のとき:

1. `.arc` ファイルが存在すれば `.arc` を優先
2. なければ `.ar` を読み込む

`.arc` には埋め込みソーステキストが含まれており、実行は通常どおり行われます。  
ネイティブコンパイル済み関数は DLL から呼び出されます。
⚠ `.arc` の埋め込みソースが隣の `.ar` と食い違っていたら、警告を出して `.ar` を使います
（`.ar` を直したのに古い `.arc` が使われ続けるのを防ぐため）。

---

## Python モジュール (`[py]`)

Python ソースファイルを Arrow の AST に変換してインポートします。

```ar
import[py] mylib as m
from mylib import[py] join_words, count
import[py] .py_pkg.top as pt          # 相対（このファイルのディレクトリの py_pkg/top.py）
```

- 探索は上記の規則どおりです（ドット無しはエントリのディレクトリ → `python.search_paths` → `PYTHONPATH` →
  site-packages）。
- **Python ファイルの中の import も同じ規則**です。ドット無しはエントリのディレクトリ（`sys.path[0]`）と
  外部の探索先から、`from .m import x` / `from . import m` は**その `.py` のディレクトリ**から数えます。
  `import a.b` の束縛・パッケージ（`__init__.py`・名前空間パッケージ）・サブモジュールの属性・
  再エクスポートも CPython と同じです。
- 標準ライブラリ（`os` / `sys` など）と C 拡張は変換できません（明示エラー）。CPython 経由で使うときは
  `import[py-int]` を使います。
- Python の関数の中の `import` は変換エラーです。

**変換の制限**:
- Python の `class` → Arrow のクラスに変換
- Python の `def` → `fn` に変換
- `*args` は可変長の仮引数、`**kwargs` はキーワード引数を受け取る仮引数として渡されます

関数本体内での変数ホイスト (if ブランチで代入された変数の前宣言) も自動で行われます。

---

## Python インタープリタ連携 (`[py-int]`)

PyO3 を介して Python インタープリタを呼び出します。

```ar
import[py-int] numpy as np
let arr = np.array([1, 2, 3, 4])
let mean = np.mean(arr)
```

**特徴**:
- `.pyi` スタブファイルがあれば型チェックに使用（探索は `[py]` と同じ順。`.pyi` → `.py`）
- 実行時は Python インタープリタを呼び出す。このとき import 文の探索先（ドット無しはエントリの
  ディレクトリと `python.search_paths`、相対は起点ディレクトリ）を `sys.path` の先頭に足す
- `import[py-int] os.path`（`as` 無し）は CPython と同じく `os` を束縛する
- GIL (Global Interpreter Lock) により並列化は不可
- `Value::PyObject` として扱われる

---
## Rust crate (`[rs]`)

`ar_config.json` で `rust.crates_path` を設定すると Rust crate を直接読み込めます。
`ar_config.json` はエントリのディレクトリから祖先へ遡って探します（見つからなければカレントディレクトリ）。
`import[rs]` はクレート名を取るので、相対の書き方（`import[rs] ..x`）は誤りです。

```json
{
  "rust": { "crates_path": "/path/to/registry/src/..." }
}
```

```ar
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

```ar
import[cpp-dll] DxLib.DxLib as dx
import[cpp-lib] .test_modules.vec_math as vm   # 相対（このファイルのディレクトリの test_modules/vec_math.h）
```

`Dir.Name` 形式 → `{探索の起点}/Dir/Name.h` のヘッダを読みます（ドット無しはエントリのディレクトリ、
相対は書いたファイルのディレクトリから数えた場所）。DLL（`cpp-dll`）はヘッダと同じディレクトリの `Name.dll` です。
`as` が無いときの束縛名はヘッダのファイル名（`Name`）です。

**制限**:
- C++ のオーバーロード・テンプレート・名前マングリングは非対応
- `ar_config.json` でコンパイラパス・追加フラグを設定する必要があります

---

## .NET NativeAOT DLL (`[cs-dll]`)

C# の NativeAOT でコンパイルしたネイティブ DLL を Arrow から直接呼び出します。

```ar
import[cs-dll] cs_form_test.FormBridge as forms

# 静的メソッド
let result = forms.FormApp.message_box("タイトル", "メッセージ", 0)

# コンストラクタ → C# オブジェクトハンドル
let tp = forms.TextProcessor("  Hello, World!  ")

# インスタンスメソッド
let upper = tp.ToUpper()

# プロパティアクセス (ゼロ引数インスタンスメソッドとして dispatch)
let ok = tp.Confirmed
```

### 必要なファイル

| ファイル | 役割 |
|----------|------|
| `{Name}.dll` | 管理 DLL (ECMA-335 メタデータ、型スタブ生成用) |
| `{Name}_native.dll` | NativeAOT ネイティブ DLL (実際の実行時呼び出し先) |

管理 DLL (`{Name}.dll`) は探索の起点（ドット無しはエントリのディレクトリ、相対は書いたファイルから数えた場所）の
`path/to/{Name}.dll` → `{Name}.dll` → `{Name}/{Name}.dll`（単一セグメントのとき）の順に探し、ドット無しの書き方なら
最後に `ar_config.json` の `csharp.lib_paths` を探します。
ネイティブ DLL (`{Name}_native.dll`) は実行時に、同じ探索の起点（ドット無しなら `python.search_paths` も）の
`path/to/{Name}_native.dll` → `{Name}_native.dll` の順に探します。

### ブリッジ DLL の設計パターン

Arrow は管理 DLL の ECMA-335 メタデータを読んで Arrow 型スタブを生成します。このとき C# の戻り型 (`string` / `int` / `void` 等) が Arrow の `return_type` にマッピングされ、実行時の ABI dispatch を決定します。

そのため **スタブクラス** と **ブリッジエクスポートクラス** を分離する設計を推奨します:

```csharp
// ── スタブクラス (Arrow 型スタブ生成用) ────────────────────────────────
// C# の戻り型が Arrow の return_type になる。
// このクラスのメソッドは実際には呼ばれない。
public static class FormApp
{
    public static int    message_box(string title, string message, int buttons) => 0;
    public static long   input_box(string title, string prompt) => 0;
    public static string get_str(long handle) => "";
    public static void   release(long handle) { }
}

// ── ブリッジエクスポートクラス (NativeAOT 生ポインタ ABI) ──────────────
// [UnmanagedCallersOnly] で export 名を "FormApp_*" に揃える。
public static unsafe class FormBridgeExports
{
    [UnmanagedCallersOnly(EntryPoint = "FormApp_message_box")]
    public static long message_box(byte* title_ptr, int title_len,
                                   byte* msg_ptr, int msg_len, long buttons) { ... }

    [UnmanagedCallersOnly(EntryPoint = "FormApp_get_str")]
    public static void get_str(long handle, byte** out_ptr, int* out_len) { ... }

    [UnmanagedCallersOnly(EntryPoint = "FormApp_release")]
    public static void release(long handle) => ObjTable.Release(handle);

    // Arrow ランタイムが文字列バッファ解放に使う固定 export
    [UnmanagedCallersOnly(EntryPoint = "arrow_bridge_free_str")]
    public static void free_str(byte* ptr) { if (ptr != null) Marshal.FreeHGlobal((IntPtr)ptr); }
}
```

### ABI 規約

#### 引数

| Arrow 型 | ブリッジへの渡し方 |
|-----------|-------------------|
| `int` / `bool` | `i64` 直値 |
| `float` | `i64` ビットパターン (IEEE-754 reinterpret) |
| `str` | `(byte* ptr, int len)` の 2 引数ペア (UTF-8) |
| C# オブジェクトハンドル | `i64` ハンドル値 |

#### エクスポート名の命名規則

| 種別 | エクスポート名 |
|------|----------------|
| 静的メソッド | `{ClassName}_{method}` |
| インスタンスメソッド | `{ClassName}_inst_{method}` |
| コンストラクタ | `{ClassName}_new_{argc}` または `{ClassName}_new` |
| 文字列バッファ解放 | `arrow_bridge_free_str(byte*)` (固定名) |
| オブジェクト解放 | `arrow_bridge_release(i64)` (固定名) |

#### 戻り値

| C# 戻り型 → Arrow `return_type` | 変換方法 |
|----------------------------------|----------|
| `int` / `long` → `"int"` | `i64` 直値 |
| `float` / `double` → `"float"` | `i64` ビットパターンを `f64` に reinterpret |
| `bool` → `"bool"` | `raw != 0` |
| `string` → `"str"` | `(byte** out_ptr, int* out_len)` 出力引数、`arrow_bridge_free_str` で解放 |
| `void` → `"None"` | 無視 |
| オブジェクト → `"int"` | `i64` ハンドル (Arrow は `CsObject` として保持) |

文字列を返す関数は引数リストの末尾に `(byte** out_ptr, int* out_len)` の 2 引数を自動付加して呼び出されます。

### オブジェクトライフサイクル

C# 側では `Dictionary<long, object>` でオブジェクトをハンドル管理します:

```csharp
public static class ObjTable
{
    static readonly Dictionary<long, object> _table = new();
    static long _next = 1;

    public static long Store(object obj) { long h = _next++; _table[h] = obj; return h; }
    public static T Get<T>(long h) => (T)_table[h];
    public static void Release(long h) => _table.Remove(h);
}
```

Arrow から `release(handle)` を呼ぶと `ObjTable.Release(handle)` が実行されます。

### プロジェクト設定 (`.csproj`)

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Library</OutputType>
    <TargetFramework>net8.0-windows</TargetFramework>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <PublishAot>true</PublishAot>
  </PropertyGroup>
</Project>
```

ビルド:

```bash
dotnet publish -r win-x64 -c Release --self-contained
# 出力: bin/Release/net8.0-windows/win-x64/publish/{Name}.dll  (NativeAOT)
#        bin/Release/net8.0-windows/win-x64/{Name}.dll          (管理 DLL)
```

### パーサーでの処理

`import[cs-dll]` はパース時に次の処理を行います:

1. `{Name}.dll` (管理 DLL) を検索
2. ECMA-335 メタデータを解析 (`cs_assembly.rs` / `cs_assembly.py`)
3. 各 `TypeDef` から Arrow の `ClassDef` / `TraitDef` スタブを生成  
   — C# の戻り型が Arrow の `return_type` にマッピングされる
4. 生成した `Vec<Stmt>` を `Stmt::Import.body` に埋め込む

### インタープリターでの処理

実行時は次の手順で動作します:

1. `body` の実行 → `TlClass` スタブが名前空間に登録される
2. `{Name}_native.dll` (NativeAOT DLL) を検索・ロード
3. 名前空間内の全 `TlClass` に `__cs_bridge_path__` クラス変数を設定
4. メソッド呼び出し時に `__cs_bridge_path__` を検出 → cs-dll dispatch へ切り替え:
   - `TlClass.method(args)` → `call_static(bridge, ClassName, method, args, ret_type)`
   - `TlCsObject.method(args)` → `call_instance(bridge, ClassName, handle, method, args, ret_type)`
   - `TlClass(args)` (コンストラクタ) → `call_constructor(bridge, ClassName, args)` → `TlCsObject`

### WinForms の例

```ar
import[cs-dll] cs_form_test.FormBridge as forms

# MessageBox (ブロッキング)
let r = forms.FormApp.message_box("タイトル", "内容", 2)  # 2=YesNo
if r == 1:
    print("Yes が押されました")

# 入力ダイアログ → 文字列ハンドル → 文字列取得
let h = forms.FormApp.input_box("入力", "名前を入力してください:")
if h != 0:
    let name = forms.FormApp.get_str(h)
    forms.FormApp.release(h)
    print("入力:", name)

# TODO マネージャー
let todo_h = forms.FormApp.show_todo("タスク管理")
let n = forms.FormApp.todo_count(todo_h)
mut i = 0
while i < n:
    print("-", forms.FormApp.todo_get(todo_h, i))
    i += 1
forms.FormApp.release(todo_h)
```

---

## .NET IPC サブプロセス (`[cs-proc]`)

`import[cs-proc]` は通常の .NET アプリ（NativeAOT 不要）を子プロセスとして起動し、**Windows 名前付きパイプ**経由で JSON-RPC を行います。

```ar
import[cs-proc] cs_proc_test as svc

# 静的メソッド
let sum = svc.Calculator.add(10, 25)
print(sum)                            # 35

# コンストラクタ + インスタンスメソッド
let calc = svc.Calculator(100)
let v = calc.increment(50)
print(v)                              # 150
print(calc.get_formatted())          # "Value: 150"

# TextProcessor
let tp = svc.TextProcessor("Hello Arrow")
print(tp.to_upper())                 # "HELLO ARROW"
print(tp.word_count())               # 2
```

### cs-dll との比較

| | `cs-dll` | `cs-proc` |
|--|----------|-----------|
| C# コンパイル | NativeAOT 必須 | 通常 .NET (net8.0 等) |
| 呼び出しオーバーヘッド | 低 (DLL 直接) | 中 (名前付きパイプ IPC) |
| 安全なコード | unsafe 必須 | 不要 |
| WinForms / GUI | STA 手動管理 | 子プロセス内で自由 |

### 必要なファイル

| ファイル | 役割 |
|----------|------|
| `{Name}.dll` | 管理 DLL (ECMA-335 メタデータ → 型スタブ生成) |
| `{Name}_proc.exe` または `{Name}.exe` | 子プロセスホスト (IPC ループ) |
| `{Name}.runtimeconfig.json` | .NET ランタイム設定 |

### プロトコル

通信は**改行区切り JSON (NDJSON)**です。Arrow が要求を送り、ホストが応答します。

```
Request:  {"id":N,"op":"static"|"new"|"inst"|"quit","cls":"Name","mth":"method","hnd":handle,"args":[...]}
Response: {"id":N,"ok":<value>} | {"id":N,"err":"message"}
```

引数タグ: `"i"` = int64、`"f"` = float64、`"b"` = bool、`"s"` = string、`"h"` = ハンドル、`"n"` = null

### C# ホストの作成

`ArrowPipeHost` クラスを使ってホストを実装します：

```csharp
// Services.cs — public クラスのみが Arrow に公開される
public class Calculator
{
    private long _value;
    public Calculator(long initial = 0) => _value = initial;

    public static long add(long a, long b) => a + b;
    public long increment(long n) { _value += n; return _value; }
    public string get_formatted() => $"Value: {_value}";
}

// Program.cs — エントリポイント
var host = new ArrowPipeHost(typeof(Calculator).Assembly);
host.Run(args);  // args[0] = 名前付きパイプ名
```

- `ArrowPipeHost` はリフレクションでメソッドを dispatch する汎用クラス
- `public` クラス/メソッドのみ Arrow に公開される（ECMA-335 パーサーがフィルタ）
- `ArrowPipeHost` 自体は `internal` にしておくことを推奨

### プロジェクト設定 (`.csproj`)

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
  </PropertyGroup>

  <!-- ビルド後に exe / dll / runtimeconfig.json をプロジェクトルートへコピー -->
  <Target Name="CopyToProjectDir" AfterTargets="Build">
    <Copy SourceFiles="$(OutputPath)$(AssemblyName).exe"       DestinationFolder="$(ProjectDir)" SkipUnchangedFiles="true" Condition="Exists('$(OutputPath)$(AssemblyName).exe')" />
    <Copy SourceFiles="$(OutputPath)$(AssemblyName).dll"       DestinationFolder="$(ProjectDir)" SkipUnchangedFiles="true" Condition="Exists('$(OutputPath)$(AssemblyName).dll')" />
    <Copy SourceFiles="$(OutputPath)$(AssemblyName).runtimeconfig.json" DestinationFolder="$(ProjectDir)" SkipUnchangedFiles="true" Condition="Exists('$(OutputPath)$(AssemblyName).runtimeconfig.json')" />
  </Target>
</Project>
```

ビルド:
```bash
dotnet build -c Debug
# プロジェクトディレクトリに {Name}.dll / {Name}.exe / {Name}.runtimeconfig.json が生成される
```

### ファイル検索順

Arrow は以下の順で proc ホストと型スタブを探します：

`{起点}` はドット無しならエントリのディレクトリ、相対なら書いたファイルのディレクトリから数えた場所です。

**型スタブ DLL** (`import[cs-dll]` と共通):
1. `{起点} / path / to / {Name}.dll`
2. `{起点} / {Name}.dll`
3. `{起点} / {Name} / {Name}.dll` (単一セグメント時、パッケージディレクトリ規約)
4. `ar_config.json` の `csharp.lib_paths`（ドット無しの書き方のときだけ）

**proc ホスト exe**（実行時）:
1. `{Name}_proc.exe` (専用ホスト)
2. `{Name}.exe` (自己ホスト exe)

それぞれを `{起点}`（ドット無しなら `python.search_paths` も）の `path/to/` の下 → 直下 →
`{Name}/` の下（単一セグメント時）の順で探します。⚠ カレントディレクトリは探しません。

### `ArrowPipeHost` の dispatch 仕組み

```
Arrow                           C# Host (ArrowPipeHost)
  │                                   │
  │── {"op":"new","cls":"Calc"} ──────▶│ Activator.CreateInstance(type, args)
  │◀── {"id":1,"ok":{"t":"h","v":1}} ─│ → handle=1 を ObjTable に登録
  │                                   │
  │── {"op":"inst","hnd":1,"mth":"increment","args":[{"t":"i","v":50}]} ──▶│
  │                                   │ obj = ObjTable[1]
  │                                   │ method.Invoke(obj, [50L])
  │◀── {"id":2,"ok":{"t":"i","v":150}} ──────────────────────────────────│
```

戻り値の型変換（EncodeResult）：
- `string` → `{"t":"s","v":"..."}`
- `int`/`long` → `{"t":"i","v":N}`
- `double`/`float` → `{"t":"f","v":N}`
- `bool` → `{"t":"b","v":true/false}`
- `void`/`null` → `null`
- その他（参照型）→ ObjTable に登録し `{"t":"h","v":handle}`

---

## Node.js IPC サブプロセス (`[js-proc]`)

`import[js-proc]` は Node.js プロセスを子プロセスとして起動し、**Windows 名前付きパイプ**経由で JSON-RPC を行います。Node.js の任意のモジュール（npm パッケージ・組み込みモジュール・カスタムスクリプト）を Arrow から呼び出せます。

```ar
import[js-proc] path as js_path

let base: str  = js_path.basename("examples/file.ar")   # → "file.ar"
let ext:  str  = js_path.extname("file.ar")             # → ".ar"
let joined: str = js_path.join("a", "b", "c")           # → "a\b\c"
```

```ar
import[js-proc] out_debug.analysis as analysis

let stripped: str = analysis.stripComment("let x = 1  # comment")
let parts: List[str] = analysis.splitComma("int, str, bool")
```

```ar
import[js-proc] lw_math as math

let err: str = math.renderSVGToFile("\\frac{1}{2}", True, "out/frac.svg", 1.5, "#cdd6f4")
```

### `ar_config.json` の設定

```json
{
  "javascript": {
    "node_path":    "node",
    "bridge_script": "bridge/js_bridge.cjs",
    "bridge_root":  "vscode-extension"
  }
}
```

| キー | 説明 |
|------|------|
| `node_path` | Node.js 実行ファイルのパスまたはコマンド名 |
| `bridge_script` | IPC サーバースクリプト（通常 `bridge/js_bridge.cjs`）への相対パス |
| `bridge_root` | モジュール解決のルートディレクトリ。`import[js-proc] a.b` は `{bridge_root}/a/b.js` を探す |

`ar_config.json` は import 文の探索の起点（ドット無しはエントリのディレクトリ）から祖先へ遡って探し、
見つからなければカレントディレクトリを見ます。

### モジュール解決

まず探索の起点（ドット無しはエントリのディレクトリ、相対は書いたファイルから数えた場所）に
`a/b.js` / `a/b.cjs` / `a/b/` があれば、その**絶対パス**をブリッジへ渡します。
無ければ、ドット無しの書き方に限りブリッジ側の解決に任せます（相対の書き方で見つからなければ誤り）。
型検査用の `.ars` スタブは探索の起点の `a/b.ars` を読みます。

ブリッジスクリプト (`js_bridge.cjs`) は次の順でモジュールを探します:

1. `{bridge_root}/{module_path}` （拡張子なし）
2. `{bridge_root}/{module_path}.js`
3. `{bridge_root}/{module_path}.cjs`
4. ブリッジスクリプト自身のディレクトリ (`bridge/`) に対して同様に試行
5. 裸の `require(moduleName)` — npm パッケージ・Node.js 組み込みモジュールのフォールバック

**例**: `bridge_root = "vscode-extension"` のとき

| Arrow インポート | 解決されるパス |
|-----------------|---------------|
| `import[js-proc] path` | Node.js 組み込み `path` モジュール |
| `import[js-proc] out_debug.analysis` | `vscode-extension/out_debug/analysis.js` |
| `import[js-proc] lw_math` | `bridge/lw_math.cjs` |

### プロトコル

通信は**改行区切り JSON (NDJSON)**です。Arrow が要求を送り、ブリッジが応答します。

```
Request:  {"id":N,"op":"list"|"call"|"quit","module":"a/b","fn":"fnName","args":[...]}
Response: {"id":N,"ok":{t,v}} | {"id":N,"err":"message"}
```

引数・戻り値の型タグ:

| タグ | Arrow 型 | JavaScript 型 |
|------|----------|---------------|
| `"i"` | `int` | `number` (整数) |
| `"f"` | `float` | `number` (小数) |
| `"b"` | `bool` | `boolean` |
| `"s"` | `str` | `string` |
| `"n"` | `None` | `null` / `undefined` |
| `"a"` | `List` | `Array` |
| `"o"` | `List[str]` (`"k=v"` 形式) | `Object` |

### `list` 操作

`import` 実行時にブリッジへ `list` 要求を送り、モジュールがエクスポートする関数名を取得します。各関数は `Value::JsProcFn` として名前空間に登録されます。

### 起動フロー

```
Arrow runtime                              Node.js bridge (js_bridge.cjs)
    │                                                  │
    │── node js_bridge.cjs <pipe> <bridge_root> ──────▶│ パイプサーバー起動
    │◀── "READY\n" ─────────────────────────────────── │ 名前付きパイプに接続完了
    │                                                  │
    │── {"op":"list","module":"path"} ────────────────▶│ require('path')
    │◀── {"id":1,"ok":{"t":"a","v":[{"t":"s","v":"basename"},...]} │
    │                                                  │
    │── {"op":"call","module":"path","fn":"basename","args":[...]} ─▶│
    │◀── {"id":2,"ok":{"t":"s","v":"file.ar"}} ────── │
```

### `cs-proc` との比較

| | `cs-proc` | `js-proc` |
|--|-----------|-----------|
| ランタイム | .NET (net8.0) | Node.js |
| 型スタブ | ECMA-335 DLL パース | `list` 操作で動的取得（.ars あれば静的チェック可） |
| 呼び出し先 | リフレクション dispatch | 任意の JS モジュール関数 |
| Promise 対応 | 不要 | `await Promise.resolve()` で透過的に同期 |
| 用途 | .NET ライブラリ / GUI | npm パッケージ・VS Code 拡張機能の内部ロジック |

### AsyncManager との組み合わせ

JS 呼び出しは Arrow の `<-` async 構文と組み合わせられます。ブリッジはグローバルな `Mutex` で保護されているため、複数の async スレッドから安全に同時呼び出しできます（シリアル化）。

```ar
import[js-proc] lw_math as math

let mng = AsyncManager(2)

mng <- async->str:
    block_return math.renderSVGString("e^{i\\pi}+1=0", True, 1.5, "#cdd6f4")

mng <- async->str:
    block_return math.renderSVGString("\\sqrt{x^2+1}", True, 1.5, "#cdd6f4")

mng.wait_for_finish()
print(mng.results)
```

### カスタムブリッジモジュールの作成

`bridge/` ディレクトリに `.cjs` ファイルを置くことで、Arrow から呼び出せる独自モジュールを作成できます。

```javascript
// bridge/my_tool.cjs
'use strict';
function greet(name) { return 'Hello, ' + name + '!'; }
function add(a, b)   { return a + b; }
module.exports = { greet, add };
```

```ar
import[js-proc] my_tool as tool
print(tool.greet("Arrow"))   # Hello, Arrow!
print(tool.add(3, 4))        # 7
```

非同期関数も透過的に使えます — `async function` が返す `Promise` はブリッジ側で `await` されます。

### LaTeX Workshop MathJax の流用例

LaTeX Workshop VS Code 拡張 (`james-yu.latex-workshop`) は `mathjax-full` をバンドルしています。`bridge/lw_math.cjs` はこれを動的ロードして hover preview と同じパイプラインで数式を SVG に変換します。

```ar
import[js-proc] lw_math as math

fn render(name: str, formula: str) -> None:
    let err: str = math.renderSVGToFile(formula, True, "out/" + name + ".svg", 1.5, "#cdd6f4")
    if err != "": print("ERROR:", err)
    else: print("OK:", name + ".svg")

render("quadratic",  "x = \\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}")
render("euler",      "e^{i\\pi} + 1 = 0")
render("schrodinger","i\\hbar\\frac{\\partial}{\\partial t}\\Psi = \\hat{H}\\Psi")

let html_err: str = math.renderGalleryHTML("out")
```

---

## import の実行タイミング

import 文はパース時に実行されます (`parse_import_stmt`):

1. 探索規則に従ってモジュールファイルを探し、読み込む
2. 字句解析・構文解析して AST を生成（その中の import も同じ手順で再帰的に読み込む）
3. 生成した AST を `Stmt::Import.body` に埋め込み、`module` をファイルごとの名前に書き換え、束縛（`bind`）を決める
4. `import a.b.c` なら、パッケージ `a` / `a.b` を読み込む**束縛しない import 文**を元の文の手前に足す

型検査・実行フェーズでは `body` を参照するだけです（実行時に読むのは外部言語のブリッジ・DLL・Python の
モジュールだけ）。これにより型検査でインポート先の型情報が利用できます。

---

## from import の動作

```ar
from geometry import Vector, Matrix as Mat, point
```

1. `geometry` モジュール全体の AST が `body` に格納される（パッケージの連鎖があれば手前で読み込む）
2. 取り込む名前がモジュールの名前に無く、サブモジュール（`geometry/point.ar`）があれば、パーサがそれを
   先に読み込む文を手前に足す
3. 実行時に `body` を実行してモジュール名前空間を構築（読み込み済みなら共有）
4. `names` に列挙された名前を、モジュールのメンバー（宣言・再エクスポート）→ サブモジュールの順に引いて
   現在スコープに登録。どちらにも無ければ静的エラー（`CannotImportName`）

`Stmt::FromImport` と `Stmt::Import` はどちらも `body` にモジュール全体の AST を持ちます。
