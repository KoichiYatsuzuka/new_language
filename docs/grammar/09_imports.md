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
cpp の設定（`cpp.*` と最上位の `"os"`）だけは、**ヘッダのディレクトリから祖先へ**（最後にカレントディレクトリ）見つかった
ものをすべて重ね、近いものがキーごとに勝ちます（ヘッダの隣の `ar_config.json` で上書きできる・`cpp_header::load_cpp_config`）。

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
- ⚠ 外部言語のスタブ（cpp / cs / js / rs）では、無いメンバーを誤りにしません。

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

**組み込み**（[12_builtins.md](12_builtins.md)）:
- Arrow に無い組み込み（`format` / `vars` / `exit` …）を参照するモジュールは、**読み込む時点で変換の誤り**です
  （実行の前・呼ばれない関数の中でも）。名前ごとに最初の行を並べます:
  `m.py: Python builtin(s) not supported by Arrow: 'format' (line 8), 'exit' (line 12) (...)`。
  モジュールのどこかで束縛した名前（`def format(..)` / 代入 / 仮引数 / `import` の別名）・デコレータ・クラスの基底・
  `super().m(..)` の `super` は通します（`from m import *` のあるモジュールは調べません）。実装は
  `src/python_converter/builtins_check.rs`。
- モジュールの最上位で組み込みの名前を束縛し直せます（`id = 7` / `def len(x): ..`。そのモジュールの中だけ）。

**例外**（[07_exceptions.md](07_exceptions.md)）:
- `except m.Err:`（修飾名）・`except (A, B) as e:`（型ごとに同じ本体の節を並べる）を変換します。
  組の入れ子 `except (A, (B, C)):` は変換の誤り（CPython 3.12 も実行時の `TypeError`）。
- 組み込みの例外を継承したクラスの `super().__init__(..)` は組み込みの例外へ届き、`args` / `str(e)` が CPython と同じになります。

**ファイル**: `open(path, "w", encoding="utf-8")` など Python の形の `open` を受けます。`with open(p) as f: return f.read()`
（`with` の中の `return`）も動きます。

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
  - 注釈は要素型まで Arrow の型にします: `set[tuple[int, int]]`・`dict[str, list[int]]`・`frozenset[str]`（→ `set[str]`）・
    `Optional[T]` / `T | None`（→ `Option[T]`）。戻り値の要素も境界で照合します（宣言に反すれば `FfiTypeError`）。
    Arrow が表せない型（`Sequence[T]`・`Callable[...]`・`tuple[int, ...]` の要素）は照合しません
    （`implementation_plans/ffi_value_exchange_plan.md` 7-8。以前は `dict[K, V]` → `dict`・`frozenset[T]` → `Any` と要素型を落としていました）
- **型を付けるのは注釈・スタブファイル（`.pyi`）があるものと、クラスのコンストラクタの戻り値だけ**。それ以外は `Any`
  （使うには明示のダウンキャストが要る）。注釈の無い関数の戻り値も `Any`（7-27。以前は `return` のリテラルから推し量っていた）
- **関数の引数**（7-30）: 注釈のある引数はその型、注釈の無い引数は `Any`、既定値のある引数は省略できる。引数の数も検査する。
  `*args` / `**kwargs` を持つ関数は引数を検査しない。PEP 484 の数の塔で、`float` と注釈した引数は `int` も受け、
  `-> float` の関数が `int` を返せば `float` にする。このファイルがクラスとして宣言していない型の名前（`numpy.typing.ArrayLike`
  などの別名・ほかのモジュールのクラス）は `Any`（`examples/interop/py_int_functions.ar`）。⚠ 以前は仮引数を持たず、
  引数の数も型も検査しなかった
- **クラス**（7-27）: `pc.Counter(10)` の型は `pc.Counter`。メソッド（引数と戻り値は注釈どおり、無ければ `Any`）と属性
  （クラス本体の `name: T` / `name = …`、メソッドの中の `self.name: T = …` / `self.name = …`。注釈の無いものは `Any`。
  `@property` も属性）の表で静的に検査し、表に無いメンバーは静的エラー。同じファイルの基底クラスのメンバーも持つ。
  ほかのファイル・ライブラリのクラスを継承するクラスと、デコレータ（`@dataclass` など）の付いたクラスはメンバーの顔ぶれが
  分からないので、書いたメンバーにだけ型を付け、ほかの名前は検査しない。コンストラクタの引数は `__init__` で検査する
  （7-31。`__init__` を書かない派生は同じファイルの基底のものを受け継ぎ、基底が分からないクラスは引数を問わない）。
  `x is pc.Counter` / `x mustbe pc.Counter` は Python の `isinstance` と同じく基底も見る
  （`examples/interop/py_int_classes.ar`）。⚠ 以前はコンストラクタの値が `Any` で、属性もメソッドも使えなかった
- 実行時は Python インタープリタを呼び出す。このとき import 文の探索先（ドット無しはエントリの
  ディレクトリと `python.search_paths`、相対は起点ディレクトリ）を `sys.path` の先頭に足す
- `import[py-int] os.path`（`as` 無し）は CPython と同じく `os` を束縛する
- GIL (Global Interpreter Lock) により並列化は不可
- `Value::PyObject` として扱われる

**値の対応**: `int` / `float` / `str` / `bool` / `None` / list / tuple / dict / set（`frozenset` も Arrow の set）・
bytes（Python の `bytes` / `bytearray` と、要素が 1 バイトの `memoryview` を Arrow の `bytes` にコピー・5-3。注釈の
`bytearray` / `memoryview` も `bytes` と読む。int32 などの `memoryview` は Python のオブジェクトのまま）は
双方向に変換します。i64 に入らない Python の `int` と、`Decimal` など `float` 以外の数値は Python のオブジェクトのまま
受け取ります（以前は黙って `float` に変換し精度を失っていました）。Python が投げた例外は `ForeignError`（`message` は
`ValueError: bad` のように Python の例外の種類とメッセージ）、Arrow の値を Python へ渡せない（Arrow のインスタンスなど）のは
`TypeError` です（7-24。以前は呼び出しの種類で決まる例外で、`ValueError` を投げるメソッドが `AttributeError` になっていました）。ただし `-> int` など `int` を宣言した場所に i64 に入らない
`int` が返れば、C / C# / Rust と同じく `OverflowError` です（7-9。以前は `FfiTypeError`）。Arrow の set に入れられない要素を含む set は、dict と
同じく丸ごと Python のオブジェクトのまま受け取ります。Arrow のクラスのインスタンスは Python へ渡せません（`TypeError`。
型名はクラス名）。

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
- `i*`/`u*` → `int`（Arrow の `int` に入らない値・Rust の型に入らない引数は `OverflowError`）
- `f32`/`f64` → `float`
- `bool` → `bool`
- `String`/`&str` → `str`
- `&[u8]` / `Vec<u8>` / `[u8; N]`（戻り値）→ `bytes`（5-4。以前は 16 進の `str`）
- 数値の `Vec<T>`（戻り値）→ 所有の `Ptr[T]`（`Vec<i32>` は `Ptr[int32]`。要素は連続したメモリなので `Ptr` で受け取り、
  `to_list()` で展開する・D-16。最後の参照か `release()` で Rust 側が 1 回だけ解放する）。数値の `&[T]` / `Vec<T>` の引数は
  同じ要素型の `Ptr[T]` か `list`（要素ごとに型と範囲を確かめる。int と float は暗黙に変換しない）
- `Vec<String>` / `Vec<bool>` → `list[str]` / `list[bool]`（要素がそれぞれ別の値なので展開する）。`list[str]` → `Vec<String>` /
  `&[String]` / `&[&str]`
- `Option<T>` → `Option[T]`（`None` か `T`）

クレートの `src/lib.rs` から `pub fn` と `pub struct` を自動検出して中継 DLL を生成します（複数行に折り返した
シグネチャも読みます）。

- 構造体は `pub fn new(..) -> Self` があればそれで作ります。無ければ全フィールドが `pub` のときだけ構造体リテラルで
  作ります。どちらも使えない構造体は Arrow から作れません（構築は静的エラー）が、メソッドの戻り値としては使えます。
- フィールド（`obj.field`）は Rust の値と一致します。`&mut self` のメソッドが書き換えれば反映され、`obj.field = v` と
  代入すれば Rust の値が変わります。`-> Self` を返すメソッドも使えます。
- 今の Arrow から呼べない関数・メソッド（`Vec<String>` などのコレクション・`Option`・タプル・`&mut` の引数・
  ジェネリック・`self` の無い関連関数（`new` 以外）・`self` を消費するメソッド）は、使った箇所が静的エラーになります
  （理由つき）。以前は import のときに黙って消えていました。

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

**値の対応**:
- C の整数型（`char` / `short` / `int` / `long` / `long long` / `intN_t` / `uintN_t` / `size_t` …）は Arrow の `int`
  です。受け取るときは C の型の幅と符号どおりに広げ（Windows の `long` は 4 バイト・下の「OS」）、渡すときは C の型の幅に
  切り詰めます（C の暗黙の変換と同じ）
- `unsigned long long`（`uint64_t` / `size_t`）の 2^63 以上は Arrow の `int` に入らないので `OverflowError` になります
- `bool` は `bool`、`float` / `double` は `float` です
- **生のポインタ**（`implementation_plans/ffi_value_exchange_plan.md` D-7・D-8・D-9・D-13）:

  | C | Arrow |
  |---|---|
  | `char*` / `wchar_t*` の戻り値 | `Option[bytes]`（NUL 終端の借用をコピー。`wchar_t` は UTF-16LE。文字列にするには `b.decode("utf-8")` など） |
  | ほかのポインタの戻り値（`void*`・構造体ポインタ・`int*`） | 借用の `Option[Ptr[T]]`（`void*` は `Ptr[void]`、構造体は `Ptr[構造体名]`、`int*` は `Ptr[int32]`。Arrow は解放しない） |
  | `const char*` の引数 | `bytes` か `None`（NUL を含むと `ValueError`）。**`str` は静的エラー**: ライブラリの文字コードで `s.encode("utf-8")` / `s.encode("cp932")` |
  | `void*` の引数 | どの `Ptr` でも、または `None` |
  | 構造体ポインタの引数 | 正確なレイアウトの構造体はインスタンス（書き戻しあり）か `Ptr[構造体名]`。前方宣言だけの構造体は `Ptr[構造体名]` か `None` |
  | 構造体の値返し | その構造体のクラスのインスタンス（コピー）。正確なレイアウトはバイト列をそのまま写し、ポインタのフィールドは上の規則（`char*` → `bytes`、ほか → `Ptr`、`ArPtrProtocol` → 所有の `Ptr`）。コンストラクタは呼ばない |
  | 構造体の値渡し | インスタンス。ポインタのフィールドも上の規則で組み立てる（`bytes` は呼び出しの間だけのコピー）。書き込み用の `char*` と `wchar_t*`（`const` も）のフィールド・関数ポインタ・入れ子の構造体・`ArPtrProtocol*`・ヘッダが構造体と宣言していない名前へのポインタのフィールドは渡せない |
  | 構造体の配列（`ArPtrProtocol`） | `Ptr[構造体].to_list()` で `list[構造体]`。ポインタのフィールドは値返しと同じ規則（`char*` / `wchar_t*` → `bytes`、ほかのポインタ → 借用の `Ptr`、NULL → `None`・7-14）。`ArPtrProtocol` のフィールドを持つ構造体の配列は TypeError（展開するたびに持ち主が増えるため） |
  | `ArPtrProtocol` の戻り値（値返し） | `Option[Ptr[void]]`（所有なら最後の参照か `release()` で 1 回だけ解放・STATIC は解放しない）。`p mustbe Ptr[int32]` で要素型を照合してから `to_list()` / `to_bytes()` |
  | `ArPtrProtocol`（値・`const` ポインタ・ポインタ）の引数 | `Ptr` か `None`。`ArPtrProtocol*` を受けた関数が解放して `address` を 0 に書き戻すと、その `Ptr` は使えなくなる（`docs/ptr_protocol.md`）。`list` / `bytes` も渡せる（呼び出しの間だけのコピー。要素の型は int → `int64`・float → `float64`・C の構造体 → 構造体名・bytes → `uint8`。`ArPtrProtocol*` へ `mut` の list を渡すと C が書いた中身が書き戻される） |
  | `char*`（書き込み用・出力バッファ）の引数 | `mut` の `bytes`（`bytes(n)` で用意する）。C が書いた中身が同じ長さの `bytes` として変数へ書き戻される |
  | 数値の書き込みポインタ（`int*` / `double*`）の引数 | 名前つきの `mut` 変数（呼び出しの後で書き戻す） |
  | NULL | `None` |

  ⚠ 4-3 より前は `const char*` が `str` を受けて黙って UTF-8 で渡し、ポインタを返す関数・`void*` を受ける関数は使えなかった
  （`examples/interop/cpp_raw_pointers.ar`・`cpp_cstring_arg.ar`）。4-4 から、正確なレイアウトの構造体ポインタと C の文字列・
  `bool` が混ざった関数も呼べる（`cpp_typed_pointers.ar`）。前方宣言だけの構造体（`typedef struct Db Db;`）は、
  型の名前としてだけ使える（Arrow から作ることはできない）
- 数値・`bool` の引数に合わない値（`None`・`str`・`list` …）が届いたら `TypeError` で、C の関数は呼ばれません。静的な
  検査が止めない経路（素の `function` 型を通した呼び出しなど）でも同じです（`examples/interop/cpp_native_arg_check.ar`）。
  `float` → `int` の切り捨てと `bool` → 0 / 1 は C の暗黙の変換のままです。Rust の crate（`[rs]`）も同じです
- **既定値**（7-13）: `int b DEFAULTPARAM( = -1 )`（DxLib の書き方）と C++ の `int b = -1` の既定値は、その引数の型で
  読みます（整数・`TRUE` / `FALSE`・浮動小数・`bool`・ポインタの `NULL`）。既定値を読めた末尾の引数は省略でき、省略した
  引数には宣言した既定値が入ります。既定値がマクロ名や式で読めない引数は省略できません（引数の数の静的エラー・
  `examples/interop/cpp_default_args.ar`）。⚠ 以前は省略を静的エラーにし、実行時は 0 / NULL で埋めていました
- **OS**（7-32）: C の `long` と `wchar_t` の幅は OS で決まる（Windows は 4 / 2 バイト、Linux / macOS は 8 / 4 バイト）。
  `ar_config.json` の最上位の `"os"`（`"windows"` / `"linux"` / `"macos"`）で決め、書いていなければ動いている OS を使う
  （エディタもホストの OS を使うので CLI と同じ型になる）。書いた OS が動いている OS と違えば、その OS の C のライブラリは
  読めないので import の誤り（`examples/interop/cpp_os_mismatch_error.ar`）。設定は祖先のディレクトリから重ねて読むので、
  ヘッダの隣の `ar_config.json` で上書きできる。`wchar_t*` はどの OS でも UTF-16LE の `bytes` で受け取る（UTF-32 の OS では
  符号化し直す）。⚠ 以前は Arrow を作ったときの OS で決めていて、エディタ（wasm）では常に Windows 以外の幅だった
- **システムヘッダの型**（7-13）: `ar_config.json` の `cpp.system_headers`（Windows SDK の `Windows.h` など）から typedef
  （`TCHAR` → `char`・`LONG` → `long`）と構造体の定義を読みます。ヘッダの関数が参照する構造体（`const RECT*` の `RECT`）は
  そこから足します（参照されたものだけ）。`const TCHAR*` の `const` は typedef を通しても残ります
- **構造体のコンストラクタ**（7-31）: `Pair(1, 2)` は全フィールドを宣言順に受け、フィールドの型で静的に検査する
  （`examples/interop/foreign_ctor_args.ar`）。中身の見えない構造体（前方宣言だけ）は作れない（静的エラー）
- **取り込んだヘッダ**（7-28）: `cpp-lib` は `cpp.precompile_macros` があるとき、ヘッダが `#include` する同じ場所のヘッダ
  （DxLib の `DxFunctionWin.h`）の関数・構造体も読みます。型スタブと実行時は同じ関数で集めるので、そうした関数にも型が
  付きます（`examples/interop/cpp_lib_included_header.ar`）。⚠ 以前は実行時だけが読み、型スタブには無かった（型が付かず検査されなかった）
- **中継 DLL に入らなかった関数**（7-28）: cpp-lib の C++ シムのコンパイルで外れた関数（Windows の同名のマクロと衝突する
  `SetWindowText` など）は、呼ぶと理由つきの `RuntimeError` になります（シムを作るまで分からないので静的には検査できない）。
  外すのは、コンパイラの誤りが名指しした名前のうちシムに入れた関数だけです（誤りの文面に出る型名・名前空間は外さない）。
  関数を 1 つも名指ししない誤りは、外しても直らないので import の誤りになります（7-29）
- 配列・文字データ・所有権のあるメモリは `PtrProtocol` で受け渡します（[ptr_protocol.md](../ptr_protocol.md)。
  共有ヘッダ `bridge/ptr_protocol/ar_ptr_protocol.h`）
- 今の Arrow から正しく受け渡せない関数は、使った箇所が理由つきの静的エラーになります（判定は `CFnSig::unsupported_reason`
  の 1 か所・`examples/interop/cpp_unsupported_signature_error.ar`・`cpp_callback_param_error.ar`）:
  `wchar_t*` の引数・`const int*` などの数値の `const` ポインタの引数（値 1 個か配列か分からない）・関数ポインタ（callback）を
  受ける／返す・レイアウトの分からない構造体を値で受け渡す・ヘッダが構造体と宣言していない名前（解決できない typedef）への
  ポインタ・`ArPtrProtocol*` を返す・正確なレイアウトの構造体ポインタと `char*` の出力バッファか構造体の値返しが混ざる など。
  ⚠ `char*` の出力バッファ（4-7）と `ArPtrProtocol` の引数・値返し（4-4・4-5）は上の表のとおり使える。
  ⚠ 関数ポインタの引数（`void (* cb )( void )`）を持つ関数は、以前はヘッダの解析が読めずに黙って捨てていた（7-29。
  名前空間が開いているので静的に通り、実行時に「属性が無い」で落ちた）

**制限**:
- C++ のオーバーロード・テンプレート・名前マングリングは非対応
- `ar_config.json` でコンパイラパス・追加フラグを設定する必要があります

---

## C# の値の対応（`[cs-dll]` / `[cs-proc]` 共通）

型スタブは C# のアセンブリ（`{Name}.dll`）のメタデータから作ります。メソッドの引数・戻り値はその宣言どおりに
受け渡し、宣言に合わない値はエラーにします（黙って `None` や整数にしません）。

| C# の型 | Arrow の型 | 備考 |
|---|---|---|
| `long` / `int` / `short` / `byte` / `ulong` … | `int` | `ulong` の 2^63 以上は `OverflowError` |
| `double` / `float` | `float` | |
| `bool` | `bool` | |
| `string` / `char` | `str` | `char` を渡すときは長さ 1 の `str` |
| `string?` / `Person?` / `long?` | `Option[str]` / `Option[Person]` / `Option[int]` | C# の nullable 注釈を読みます。null は `None` |
| このアセンブリの公開クラス | そのクラス（値は C# のオブジェクト） | `x is Person` / `mustbe Person` で絞り込めます。型の位置では別名つきの `cp.Person` とも書けます（7-6） |
| `object` | `Any` | 中身が数値・文字列ならその値。クラスが分からないオブジェクトは `TypeError` |
| `void` | `None` | |

- `?` の無い参照型（`string` / `Person`）のメソッドが null を返したら `TypeError` です（C# 側の約束違反）。
  nullable 注釈を有効にしていない（`<Nullable>` の無い）アセンブリの参照型も同じ扱いです。
- コレクションと `byte[]` は cs-proc がコピーで運びます（5-2）: 配列・`List<T>` など → `list[T]`、
  `Dictionary<K, V>` → `dict[K, V]`、`HashSet<T>` → `set[T]`、`ValueTuple` → `tuple[…]`、`byte[]` → `bytes`。
  引数は C# のメソッドの引数の型に作り直して渡します。要素が C# のオブジェクトでもかまいません。
  `Task<T>` を返すメソッドは待ってから `T` を返します（cs-proc）。
- cs-dll（同じプロセス・5-5・D-16）: `byte[]` ↔ `bytes`、数値の配列（`int[]` / `double[]` …）→ `Ptr[int32]` などの所有の
  `Ptr`（C# の配列は GC が動かすので、アンマネージドのメモリへ複製して渡す。`to_list()` で展開）。引数は同じ要素型の
  `Ptr` か list（`Union[Ptr[int32], list[int]]`）。`List<T>` / `string[]` / `bool[]` → `list[T]`（境界で展開）、
  `IntPtr` / `nint` ↔ 借用の `Ptr[void]`（0 は None）。dict・set・タプルは cs-dll ではまだ運べません。
- 次の型を受け渡すメソッドは、使った箇所が静的エラーになります: デリゲート・型変数（ジェネリックなメソッド）・
  他のアセンブリの型・インターフェース（cs-dll では dict・set・タプル・入れ子のコレクションも）。
- `ref` / `out` の引数を持つメソッドも静的エラーです（Arrow は書き戻さないので・7-16）。⚠ 以前は型スタブを通り、実行すると
  cs-proc はホストが引数を変換できず（`InvalidCastException`）、cs-dll は「export が無い」エラーでした。
- C# のオブジェクトは、それを作ったブリッジ（同じ DLL / 同じホスト）へだけ渡せます。
- 公開のコンストラクタは型スタブの `__init__`（コンストラクタごとに 1 つ）になり、引数の個数で選んでその型で渡します。
  その個数のコンストラクタが無い呼び出しは静的エラー（7-31。型の見えない経路では実行時の `TypeError`）。

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

### export を生成する（`--gen-cs-exports`）

Arrow が呼ぶ `[UnmanagedCallersOnly]` の中継関数（export）は、管理 DLL のメタデータから生成できます
（`implementation_plans/ffi_value_exchange_plan.md` 5-1）。型スタブと同じ読み方で作るので、Arrow が呼ぶ名前と型の
export だけが並びます。

```bash
dotnet build {Name}.csproj -r win-x64 -c Release -p:ArrowGenerated=false     # 1 回目（生成物を除く）
arrow --gen-cs-exports bin/Release/net8.0/win-x64/{Name}.dll -o ArrowExports.g.cs
dotnet publish {Name}.csproj -r win-x64 -c Release --self-contained          # NativeAOT（生成物を含む）
```

1 回目のビルドは、古い生成物が消えたメソッドを呼んでいても通るように `ArrowExports.g.cs` を外します:

```xml
<ItemGroup Condition="'$(ArrowGenerated)' == 'false'">
  <Compile Remove="ArrowExports.g.cs" />
</ItemGroup>
```

一式は `examples/interop/cs_interop_test/build_bridge.ps1`。生成した export は次のことをします:

- C# の例外を捕まえて Arrow の `ForeignError` にする（`ForeignError: System.DivideByZeroException: …`・`except ForeignError` で
  捕まえられる。7-4）。⚠ 手書きの export が例外を漏らすとプロセスごと落ちます
- 整数を C# の型の幅で確かめる（`int` へ 2^31 を渡すと `OverflowError`・`ulong` の戻り値の 2^63 以上も `OverflowError`）。
  `char` は 1 文字の `str`（UTF-16 の 1 単位でなければ `ValueError`）、`Task<T>` は待ってから `T` を返す
- 作れないメンバー（型スタブが未対応としたもの・`object` の引数・`ref` / `out`・ジェネリック・演算子）は作らず、
  理由をコメントで残す（Arrow から呼ぶと「export が無い」エラー）
- 数値の配列を受け渡すメソッドがあれば、`bridge/ptr_protocol/ArPtrProtocol.cs` をプロジェクトに入れる
  （`<Compile Include="…\bridge\ptr_protocol\ArPtrProtocol.cs" Link="ArPtrProtocol.cs" />`・5-5）。list は 5-2 と同じ
  タグつきの JSON を文字列で受け渡す（`Utf8JsonWriter` / `JsonDocument` なので NativeAOT でも動く）

生成したクラスは `internal` なので型スタブには出ません。`[UnmanagedCallersOnly]` の付いたメソッド（手書きの export）も
型スタブに出しません。以下は export を手で書く場合の設計です。

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
| `str` | `(byte* ptr, int len)` の 2 引数ペア (UTF-8)。`Option[str]` の `None` は `(NULL, -1)` |
| C# オブジェクトハンドル | `i64` ハンドル値。`Option[クラス]` の `None` は `0` |

引数の並べ方は**型スタブの宣言**で決めます（値の種類ではありません）。コンストラクタは、引数の個数で選んだ
`__init__` の宣言で並べます。全部で 8 スロットまで（文字列は 2、文字列の戻り値の出力引数がさらに 2）で、
超えるとエラーです。⚠ export の浮動小数の引数は `double` ではなく `long`（ビット列）で受けてください
（Arrow は整数のレジスタで渡します）。`Option[int]` / `Option[float]` / `Option[bool]`
の `None` を渡す取り決めはまだ無いので、渡すとエラーになります。

#### エクスポート名の命名規則

| 種別 | エクスポート名 |
|------|----------------|
| 静的メソッド | `{ClassName}_{method}` |
| インスタンスメソッド | `{ClassName}_inst_{method}` |
| コンストラクタ | `{ClassName}_new_{argc}` または `{ClassName}_new` |
| 文字列バッファ解放 | `arrow_bridge_free_str(byte*)` (固定名) |
| オブジェクト解放 | `arrow_bridge_release(i64)` (固定名) |
| `byte[]` / list（5-5） | 文字列と同じ `(byte*, int)`（list はタグつきの JSON）。戻り値は末尾の `(byte** out_ptr, int* out_len)` |
| 数値の配列（5-5） | 引数は `ArPtrProtocol*`（1 スロット）。戻り値は末尾の `ArPtrProtocol*` へブリッジが書く（解放関数つき） |
| `IntPtr`（5-5） | アドレスの `i64`（Arrow では借用の `Ptr[void]`・0 は None） |
| エラーの引き取り（任意） | `arrow_bridge_take_error(byte** out_ptr, int* out_len)`（固定名）。Arrow が呼び出しのたびに呼び、`out_ptr` が NULL でなければその文字列をエラーにする（`TypeError: ` / `ValueError: ` / `OverflowError: ` で始まればその例外）。生成した export が持つ |

#### 戻り値

| C# 戻り型 → Arrow `return_type` | 変換方法 |
|----------------------------------|----------|
| `int` / `long` → `"int"` | `i64` 直値 |
| `float` / `double` → `"float"` | `i64` ビットパターンを `f64` に reinterpret |
| `bool` → `"bool"` | `raw != 0` |
| `string` → `"str"` | `(byte** out_ptr, int* out_len)` 出力引数、`arrow_bridge_free_str` で解放。null は `out_ptr = NULL`（または `out_len < 0`） |
| `void` → `"None"` | 無視 |
| このアセンブリのクラス → クラス名 | `i64` ハンドル (Arrow は C# のオブジェクトとして保持)。null は `0` |

`object`（`Any`）と `long?` などの値型の nullable は、ハンドル・null の取り決めが無いので、呼ぶ前にエラーにします。

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

引数タグ: `"i"` = int64、`"f"` = float64、`"b"` = bool、`"s"` = string、`"h"` = ハンドル、`"n"` = null。
戻り値だけ `"o"`（int64 に入らない整数 = `ulong` の 2^63 以上。10 進の文字列）があり、Arrow は `OverflowError` にします。
戻り値は宣言された型で読みます（「C# の値の対応」）。
1 行は UTF-8 です（ホストは 1 行のバイト列をまとめて復号します。以前は 1 バイトずつ文字にしていて、Arrow から送った
非 ASCII の文字列が化けていました）。

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
- `string` / `char` → `{"t":"s","v":"..."}`
- `int`/`long`/`short`/`ushort`/`byte`/`sbyte`/`uint` → `{"t":"i","v":N}`
- `ulong` → 2^63 未満は `{"t":"i","v":N}`、以上は `{"t":"o","v":"18446744073709551615"}`
- `double`/`float` → `{"t":"f","v":N}`
- `bool` → `{"t":"b","v":true/false}`
- `void`/`null` → `null`
- その他（クラスのインスタンス）→ ObjTable に登録し `{"t":"h","v":handle}`

引数の変換（ConvertArg）は `long?` などの nullable な値型を受け、`char` には長さ 1 の文字列を受けます。
タグの無い・知らない引数、表に無いハンドルはエラーです（以前は黙って `null` にしていました）。

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
| `"i"` | `int` | `number`（安全な整数 = 絶対値 2^53 未満） |
| `"f"` | `float` | `number`（それ以外。NaN / ±Infinity は文字列 `"NaN"` / `"Infinity"` / `"-Infinity"`） |
| `"b"` | `bool` | `boolean` |
| `"s"` | `str` | `string` |
| `"n"` | `None` | `null` / `undefined` |
| `"a"` | `list` | `Array` |
| `"u"` | `tuple`（引数だけ） | `Array`（JS にタプルは無い。戻り値を `tuple[…]` と宣言すれば同じ長さの配列をタプルにする） |
| `"d"` | `dict` | キーがすべて `str` なら素のオブジェクト、それ以外は `Map`。`Map` の戻り値も `dict` |
| `"o"` | `dict[str, …]`（戻り値だけ） | 素のオブジェクト（プロトタイプが `Object.prototype` か `null`） |
| `"e"` | `set` | `Set` |
| `"y"` | `bytes` | `Buffer` / `Uint8Array`（base64 で運ぶ） |
| `"x"` | （エラー） | 関数・`Symbol`・`BigInt`・クラスのインスタンス（`Date` など）。受け取れない（`TypeError`） |

JS の数値には整数と小数の区別が無いので、戻り値は `.ars` に書いた型で読みます: `-> float` の関数が `2.0`
（整数値）を返せば `2.0`、`-> int` の関数が整数値の小数を返せば `int`。`list[float]` などコンテナの要素にも同じ
規則を当てます。宣言と食い違う値（`-> list[int]` が `[1, "two"]` を返す・小数を `int` で返す）は `FfiTypeError`（`TypeError`
の派生の組み込み例外）、整数値だが int64 に入らない数（`-> int` が `1e20` を返す）は `OverflowError`（7-9）、JS の側で投げられた例外は `ForeignError`（`message` は `Error: boom` のように `String(err)`）です。
cs-proc のホストで投げられた C# の例外も `ForeignError`（`System.InvalidOperationException: boom`）です（7-4）。
`dict[str, float]` の値・`set[float]` の要素・`tuple[int, float]` の要素にも同じ規則を当てます。
dict / tuple / set / bytes は中身ごとコピーして運びます（`implementation_plans/ffi_value_exchange_plan.md` 5-2）。
Arrow のクラスのインスタンス・関数・`Ptr`（別プロセスでは意味が無い）は `TypeError` です（以前は JS 側に `null` が
届き、オブジェクトの戻り値は `"k=v"` の文字列のリストになっていました）。

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
