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
import[ar]      mod          # .ar ファイルを強制読み込み
import[arc]     mod          # .arc (コンパイル済み) を強制読み込み
import          mod          # 自動選択 (.arc 優先、なければ .ar)
import[py]      os.path      # Python ファイルをコンバータで変換して読み込み
import[py-int]  numpy as np  # Python インタープリタ (PyO3) 経由で読み込み
import[rs]      regex        # Rust crate を直接読み込み
import[cpp-dll] DxLib.DxLib with stub as dx  # C++ DLL を読み込み
import[cpp-lib] MyLib.MyLib with stub as ml  # C++ 静的ライブラリを読み込み
import[cs-dll]  MyLib.MyBridge as my         # .NET NativeAOT DLL を読み込み
import[cs-proc] MyLib.MyService as my        # .NET IPC サブプロセス経由で呼び出し
import[js-proc] out_debug.analysis as ana   # Node.js IPC サブプロセス経由で呼び出し
```

---

## モジュール検索パス

1. **`source_dir`**: 現在のソースファイルのディレクトリ
2. **`root_dir`**: メインエントリポイントのディレクトリ

`import foo.bar.baz` → `foo/bar/baz.ar` または `foo/bar/baz/` (パッケージ) を検索

---

## パッケージ

ディレクトリに `__init__.ar` を置くことでパッケージになります。

```
geometry/
├── __init__.ar
├── point.ar
└── vector.ar
```

```hv
import geometry               # geometry/__init__.ar を読み込む
import geometry.point         # geometry/point.ar を読み込む
from geometry import Vector   # geometry/__init__.ar 内の Vector を取得
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

## .ar / .arc 自動選択

`lang = "tl-auto"` (タグなし) のとき:

1. `.arc` ファイルが存在すれば `.arc` を優先
2. なければ `.ar` を読み込む

`.arc` には埋め込みソーステキストが含まれており、実行は通常どおり行われます。  
ネイティブコンパイル済み関数は DLL から呼び出されます。

---

## Python モジュール (`[py]`)

Python ソースファイルを Arrow の AST に変換してインポートします。

```hv
import[py] math as m
from[py] os.path import join, exists
```

**変換の制限**:
- Python の `class` → `fn __init__` を持つ Arrow クラスに変換
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

`ar_config.json` で `rust.crates_path` を設定すると Rust crate を直接読み込めます。

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
- `ar_config.json` でコンパイラパス・追加フラグを設定する必要があります

---

## .NET NativeAOT DLL (`[cs-dll]`)

C# の NativeAOT でコンパイルしたネイティブ DLL を Arrow から直接呼び出します。

```hv
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

管理 DLL (`{Name}.dll`) はスクリプトのディレクトリ (`source_dir`) またはモジュールサブディレクトリから検索されます。  
ネイティブ DLL (`{Name}_native.dll`) は同じ検索パスで探されます。

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

```hv
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

```hv
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

**型スタブ DLL** (`import[cs-dll]` と共通):
1. `source_dir / path / to / {Name}.dll`
2. `source_dir / {Name}.dll`
3. `source_dir / {Name} / {Name}.dll` (単一セグメント時、パッケージディレクトリ規約)

**proc ホスト exe**:
1. `{Name}_proc.exe` (専用ホスト)
2. `{Name}.exe` (自己ホスト exe)
上記を source_dir → CWD の順で検索

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

```hv
import[js-proc] path as js_path

let base: str  = js_path.basename("examples/file.ar")   # → "file.ar"
let ext:  str  = js_path.extname("file.ar")             # → ".ar"
let joined: str = js_path.join("a", "b", "c")           # → "a\b\c"
```

```hv
import[js-proc] out_debug.analysis as analysis

let stripped: str = analysis.stripComment("let x = 1  # comment")
let parts: List[str] = analysis.splitComma("int, str, bool")
```

```hv
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

### モジュール解決

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

```hv
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

```hv
import[js-proc] my_tool as tool
print(tool.greet("Arrow"))   # Hello, Arrow!
print(tool.add(3, 4))        # 7
```

非同期関数も透過的に使えます — `async function` が返す `Promise` はブリッジ側で `await` されます。

### LaTeX Workshop MathJax の流用例

LaTeX Workshop VS Code 拡張 (`james-yu.latex-workshop`) は `mathjax-full` をバンドルしています。`bridge/lw_math.cjs` はこれを動的ロードして hover preview と同じパイプラインで数式を SVG に変換します。

```hv
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
