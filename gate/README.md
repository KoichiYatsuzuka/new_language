# gate/ — ゲートが実行するテスト

言語を変えるたびに走らせるゲート（`scripts/`）が実行するテストを置く。計画:
[gate_speed_plan.md](../implementation_plans/gate_speed_plan.md)。

- **`examples/` とは役割が違う。** `examples/` は利用者向けの見本で、ゲートはここだけを見る（D-13）。
  ここのテストは `examples/` のファイルを import・参照しない。例題のコードを流用するときは写して持つ。
- 新しい文法・新しい禁止を足したら、`examples/` の見本に加えてここにもテスト（正常系と禁止の形）を書く。
- GUI は置かない（窓を出さない）。C / C++ / C# とのやり取りは GUI を使わずに確かめる（D-10）。

## 1. 置き場所と名前

| ディレクトリ | 中身 |
|---|---|
| `lib/` | 確かめ用のモジュール（`check.ar`）。テストではない（ランナーは実行しない） |
| `check/` | `lib/check.ar` そのものの確かめ（合わない値・期待した例外が出ない・別の例外が出る、で確かめ落ちになること） |
| `lang/<主題>/` | 言語の中核（`examples/` の `basics` / `collections` / `classes` / `typing` / `exceptions` に当たる範囲） |
| `lang/kinds/` | `examples/` に無かった静的な誤りと警告の種類を 1 件ずつ出す禁止の形（`scripts/gate_kinds.ps1` が、全部の種類に期待を持つテストがあるかを数える・5-2） |
| `async/` | `async` / イベントループ / シグナル |
| `modules/` | import とモジュール（パッケージ・相対 import・再エクスポート）。`ffi/` と同じく、テストは直下だけで、下のディレクトリ（`namespace_modules/` など）は補助（名前は import のパスと型の名前に出るので `examples/` と同じにした）。`.arc` を使うテストは `ffi/` の `native_*` など（補助を共有するため） |
| `ffi/` | 外部言語（C / C++ / C# / JS / Python / Rust）とネイティブの経路（`.arc`）。テストは `ffi/` の**直下だけ**に置き、名前の頭で言語を分ける（`cpp_` / `cs_` / `js_` / `py_` / `rs_` / `native_` …）。下のディレクトリ（`test_modules/` / `py_libs/` / `rs_crates/` / `cs_*/` …）は共有の補助で、ランナーは実行しない（`examples/interop/` と同じ名前で写したので import を書き換えていない・4-4）。例外: 名前が `_tests` で終わるディレクトリの下のテストは実行する（`py_subdir_tests/`: サブディレクトリから祖先の `ar_config.json` が見えること） |
| `bench/` | ベンチ（合否だけを見る。速さは `ab_bench` / `speed_modes` が測る） |
| `repl/` | 対話 REPL の入力と期待する出力 |
| `debugger/` | デバッガのコマンド列と期待する出力 |

- テストは 1 ファイル 1 主題。名前は `<主題>.ar`（正常系）と `<主題>_error.ar`（禁止の形・3 節）、
  `<主題>_warning.ar`（警告だけを期待し、最後まで走る正常系・3.4 節）。
  - ⚠ **`_error.ar` で終わる名前は禁止の形だけに使う。** 主題が例外の正常系は `eof_exception.ar` のように名付ける
    （`examples/` には `eof_error.ar` など、名前が `_error` で終わるのに正常に終わる例題が 5 件ある・2-1）。
- **作業ディレクトリは実行するファイルのディレクトリ**（ランナーがそう走らせる）。
- テストが使う補助のモジュール（import されるだけのもの）は、そのテストの隣の `<テスト名>_mods/` に置く。
  ランナーはディレクトリ名が `_mods` で終わる下を実行しない。

## 2. 正常系: 結果をコードの中で確かめる（D-15）

結果が正しいかはテスト自身が確かめ、違えば `GateCheckFailed` で止まる。stdout は照らさない。

```ar
from ..lib.check import check, check_eq, fail

check_eq(1 + 2, 3, "int add")                 # 違えば GateCheckFailed: int add: expected 3, got …
check_eq(repr([1, "a"]), "[1, 'a']", "repr of a list")   # 表示の形も文字列にして比べる
check(len(xs) > 0, "xs is not empty")
```

- `lib/check.ar` は `gate/` からの相対 import で読む（`from ..lib.check import …`・深さに合わせて `.` を足す）。
- `check_eq(actual, expected, what)` は `==` で比べ、違えば説明・期待・実際の値（`repr`）つきで `GateCheckFailed` を
  `raise` する。`check(cond, what)` は `cond` が偽なら、`fail(what)` は必ず `raise` する。
- ⚠ `print` は確かめにならない（stdout を照らさないので）。表示が主題なら `str` / `repr` / f-string で作った文字列を
  `check_eq` で比べるか、2.2 の `emit` で貯めて照らす。
  - `print` そのものの引数（`sep` / `end`）を確かめるテスト（`lang/io/print_keywords.ar`）も `emit` に写している。
    `print` の `file=` は `None` しか受けない（標準出力の外へ書けない）ので、`print` 自身の出力はコードの中で
    照らせない。⚠ `print` そのものが壊れても `gate/` は気づかない（REPL の golden・4-9 が `print` を通る）。

### 2.1 正しく誤りになる形は、捕まえて正常に終わる

```ar
mut raised = False
try:
    let d: dict[str, int] = {"a": 1}
    let v = d["b"]
except KeyError as e:                         # ⚠ 期待する例外のクラスだけを捕まえる
    raised = True
    check_eq(str(e), "'b'", "KeyError message")
check(raised, "d['b'] raises KeyError")       # ⚠ try の外で確かめる
```

- **誤りが出なかったら止める。** そのための確かめ（上の `check(raised, …)`）は `try` の**外**に置く。`try` の中に
  `fail(..)` を置くと、`except` が自分の `GateCheckFailed` を捕まえて通ってしまうことがある。
- **`except Exception` / `except BaseException` で捕まえない**（別の理由の誤りまで捕まえて通る）。期待するクラスを
  書く。階層そのもの（`except LookupError` が `KeyError` を捕まえること）が主題のときだけ親のクラスを書く。
- 文面を確かめるときは `str(e)` を `check_eq` する（一部だけなら `check(".." in str(e), ..)`）。

### 2.2 出力を貯めて照らす（`emit` / `check_output`）

値を並べて出すテスト（`examples/` から写したものの多く）は、`print` の代わりに `emit` で出力を貯め、最後に
`check_output` で行ごとに照らす。

```ar
from ...lib.check import emit, check_output

emit([str(1 + 2)])                            # print(1 + 2) と同じ行を貯める
emit([str("a"), str("b")], sep=", ", end="")  # print("a", "b", sep=", ", end="")
check_output(["3", "a, b"], "what")           # 違えば GateCheckFailed: what: output line N: expected …, got …
```

- `check_output(lines, what)` は貯めた出力を `\n` で分けた並びと `lines` を比べる。出力が改行で終わるときは最後の
  要素が `""`。照らした後は貯めた出力を空にする。
- ⚠ **`async` のタスクの中では `emit` を使わない。** `emit` は貯めた文字列を束縛し直すので、タスクから呼ぶと呼び出し元に
  伝わらない（タスクは `mut` の束縛し直しを伝えない・言語の決まり）。タスクの結果は `block_return` で返して照らす。
- ⚠ 実行したファイルの絶対パスを含む出力（トレースバックの文など）は照らさない（worktree や別の環境で変わる）。
  `async/async_demo.ar` は誤りの文の最初と最後の行だけを照らしている。
- `examples/` から写したテスト（4-1）は、写したときの出力を期待にしている。⚠ 期待の値は「写した時点の exe の出力」。
  写したときに次の 2 つで正しさを確かめた（2026-11-23・4-1）:
  - 例題の行末のコメントに書かれた期待（`print(x)  # 15`）と、その行が出した 1 行を突き合わせた。1 回だけ出る行の
    1,244 件のうち、説明文のコメント 12 件を除く 1,232 件で合った。表記が違うだけ（`["a"]` と `['a']` など）でないものは
    1 件で、コードを読むと出力が正しく、コメントが古かった（`classes/polymorphism.ar` の `Box[MyInt]`）。
  - 参照実装（`impl_python`）の stdout とは 202 件中 95 件が一致した。残りは `impl_python` が止まっている（100 コミット
    以上前に同期・D-9）ため、根拠にはしていない。
  - 写すときに直したもの: 例題のパス（作業ディレクトリが違う）・実行ごとに変わる値（アドレス・`id`）・写しの見出しで
    ずれた行番号。テストの冒頭に、もとの例題の名前を書いている。

### 2.3 外部言語の側の出力

`emit` で貯まるのは Arrow の側の出力だけ。外部言語の側（Python のモジュール・ネイティブのコード）の `print` は
stdout へ直接出る。

- **Python の側**は `lib/pycap.ar` で貯めて照らす。import した時点で CPython の `sys.stdout` を `io.StringIO` に
  差し替えるので、テストの import の**いちばん初め**に置く（モジュールを読み込む時の `print` も貯まる）。

  ```ar
  from ..lib.pycap import py_output   # いちばん初めの import
  from ..lib.check import emit, check_output, check_text
  import[py] test_modules.py_del as m
  ...
  check_output([..], "py_del")                          # Arrow の側
  check_text(py_output(), [..], "py_del (python side)") # Python の側
  ```

- ネイティブのコード（`.arc` / C）の `print` は照らせない。照らせない行の数をテストの末尾のコメントに書いている
  （`ffi/native_builtins.ar` の 1 行）。メタ関数が置いたコードの `print`（`modules/metafn_module.ar` の 4 行）も
  照らしていない。
- Arrow のモジュールの `print` は、写したモジュールの側で `emit` に置き換える（`modules/let_arg_sharing_probe.ar`）。
- 写すとき（4-4〜4-6）は、`emit` の出力に印を付けて走らせ、印の付いた行（Arrow の側）と付かない行（外部言語の側）に
  分けて期待にした。Arrow の側の行は、もとの例題の出力の中に同じ順で現れることを確かめた。

## 3. 禁止の形: 期待した誤りで止まること（D-14）

禁止されていることを書き、**期待した行で、期待した段階・種類の誤りが出て、ほかの誤りが出ないこと**を照らす。

```ar
let a: int = "s"            # expect: static VarTypeMismatch
```

### 3.1 期待の書き方

- 誤りが出る行の末尾に `# expect: <段階> <種類> ["文面の断片"]`。1 行に 2 つ以上なら `;` で区切る
  （`# expect: static A; static B`）。
- 位置を持たない誤り（実行時の一部・展開の一部）は、ファイルの先頭のコメントの並びに `# expect-file: <段階> <種類> ["断片"]`。
- 段階と種類:

  | 段階 | 種類に書くもの | 例 |
  |---|---|---|
  | `parse` | `-`（構文の誤りに種類は無い。文面の断片を必ず書く） | `# expect: parse - "cannot inherit"` |
  | `expand` | `MetaError` / `TemplateError` | `# expect: expand MetaError "recursion"` |
  | `static` | `TypeErrorKind` の名前（`src/type_check/errors.rs`） | `# expect: static VarTypeMismatch` |
  | `warning` | `TypeWarningKind` の名前 | `# expect: warning ProtocolReturnType` |
  | `runtime` | 例外のクラスの名前 | `# expect: runtime ZeroDivisionError` |

- 行は**誤りが報告される行**（文が複数行にまたがるときは、診断が指す行）。実行時の誤りは、トレースバックのうち**テストの
  ファイル自身の、いちばん内側の行**（`check_eq` の中で止まっても、`check_eq` を呼んだテストの行）。

### 3.2 判定（`scripts/gate_expect.ps1`・3-4）

- 出た誤り（警告を含む）の集まりと、期待の集まりが一致すれば合格。行・段階・種類で照らし、断片を書いたときは文面に
  その断片が含まれることも見る。
  - **足りない＝禁止が通ってしまった。余分＝別の理由で落ちた。どちらも不合格。**
- 期待が 1 つでもあるファイルは、exit が 0 なら不合格。期待が無いファイルは、誤りや警告が 1 つでも出たら不合格。
- `_error` の名前と期待の有無が食い違うファイル（`_error` なのに期待が無い・期待があるのに `_error` でない）も不合格。

### 3.3 1 ファイルに入れる数

- **静的な誤りと警告**は 1 回の実行で全部出るので、主題ごとに 1 ファイルにまとめる。各々を別の関数に入れ、
  互いに影響しないようにする（推論が `Unknown` に落ちて後ろの誤りが消える・連鎖する形は、集まりの一致で見つかる）。
- **構文の誤り・展開の誤り・捕まえない実行時の誤り**は最初の 1 つで止まるので 1 件 1 ファイル。
- 捕まえられる例外は 2.1 のとおり正常系の中で確かめる。捕まえない形（止まり方・exit・トレースバック）は
  段階ごとに数件だけ置く。

### 3.4 警告だけを期待するテスト（`_warning.ar`）

警告（`StaticTypeWarning`）は実行を止めない。警告が出る正常系は、警告の行に `# expect: warning <種類>` を書き、
名前を `<主題>_warning.ar` にする。判定は「期待した警告の集まりが一致し、**exit 0 で最後まで走った**こと」
（途中の `check_eq` / `check_output` で止まれば、期待に無い誤りとして不合格）。
誤りと警告の両方を期待するテストは `_error.ar`（exit が 0 でないこと）。

## 4. 外部言語の生成物（cpp-lib の中継 DLL）

`import[cpp-lib]` は、初めて走らせたときにヘッダの隣へ中継 DLL・シム・印（`ar_<stem>.dll` / `ar_<stem>_shim.*` /
`ar_<stem>.syms`）を作る。`examples/` と同じく**コミットする**（作り直しに MSVC が要り、`ar_config.json` の `cpp.msvc` が
環境によっては無いパスを指すため）。A/B で exe ごとに分ける仕組みは `cpp_cache_isolation.ps1`（D-11）で、A/B 系
（`gate_ab.ps1`）が走らせる exe ごとに温めてから走らせ、終わったら始める前に戻す（gate_speed_plan.md 6-2）。
⚠ 中継 DLL のキャッシュは共通ランナーの保存のキー（ディレクトリの指紋）に入れない。ヘッダ（指紋に入る）と exe（鍵に入る）から
作られるもので、入れると入れ替えのたびに保存が使われなくなるため。
⚠ 作るときにこの環境の MSVC を指す必要があれば、テストの `_mods/` に**一時的な** `ar_config.json`（`cpp.msvc` だけ）を置き、
作った後に消す（コミットしない。近いディレクトリの設定がキー単位で勝つ）。

## 5. ネイティブの経路（`.arc`）

コンパイル済みのモジュールを使うテストは、**本当にコンパイル済みのコードで走ったこと**も確かめる。陳腐化の警告
（`Warning: compiled module … is out of date …`）が出たら不合格（1-2 で、改行の違いだけで 36 個の `.arc` が使われず、
ネイティブを確かめるはずの例題がソースの解釈で走っていた）。

## 6. 並列に走らせる

ランナーは既定で並列に走らせる（論理コアの半分・3-5）。**今のところ逐次に回す規則は無い**（2-1 の実験の結果）。

- 9 カテゴリの例題 733 件を、逐次と 6 並列で同じ exe で走らせ、全件の exit が一致した（逐次 168 秒・6 並列 36 秒）。
- 以前 6 並列で落ちた 8 件（cpp-dll・`import[rs]`・`.arc`）の原因は、一時ディレクトリの**固定の名前**を同時に走る
  arrow.exe どうしが取り合っていたこと（cpp-dll の中継のビルドと読み込む DLL・`.arc` から取り出す DLL・`import[rs]` の
  cargo のプロジェクト）。プロセスごとの作業ディレクトリに直した（`partial_compiler::private_temp_dir`）。
- ⚠ **同じ cpp-lib の中継 DLL を同時に作り直すと取り合う**（出力先がヘッダの隣の 1 つのファイルなので）。ランナーは
  並列に走らせる前に、cpp-lib を使うテストを 1 本ずつ走らせてキャッシュを温める（`cpp_cache_isolation.ps1` と同じ）。
- ⚠ 速さを測るスクリプト（`ab_bench` / `speed_modes` など）は並列にしない（数字が変わる）。

## 7. ベンチ

- 合否だけを見る（結果の値を `check_eq` で確かめる）。1 本 1 秒以内に収まる回数にする。
- 速さは測らない（`examples/bench/` と `ab_bench` / `speed_modes` の担当）。
- `examples/bench/` から写したもの（4-8）は、回数を減らし（大文字の名前の定数 100 以上と、1 万以上の整数のリテラルを
  100 で割る）、`import[py] time` を `lib/fake_time.ar`（呼ぶたびに 1 ミリ秒ずつ進む決まった時計）に替えた。
  出力（経過時間から出す値を含む）は `emit` で貯めて照らす。経過時間の値そのものに意味は無く、形が変わらないこと・
  最後まで走ること・結果の値（`CHECKSUM` など）を見る。
  - `.arc` と対になるモジュール（`bench_ab_native_module` など）は書き換えずに写した（陳腐化させないため）。
  - `bench_ab_cdll` は cpp-lib の補助を共有するので `ffi/` に置いた。

## 8. 対話（REPL・デバッガ）

コードの中で確かめられないので、入力と期待する出力の対で照らす（今の `repl_session` / `debug_session` と同じ形）。

- `repl/<name>.in` と `<name>.out`: `scripts/repl_session.ps1` が全部の対を照らす（4-9 で `examples/repl/` から写し、
  1 本だけでなくディレクトリの中の対を全部見るようにした）。
- `debugger/<name>.ar` / `.in` / `.out`: `scripts/debug_session.ps1` が照らす。golden の中のパスは
  `<REPO>\gate\debugger\…` の形。共通ランナー（`gate_expect` など）はこのディレクトリを実行しない。
- どちらのスクリプトも `-Dir` で `examples/repl` / `examples/debugger` を指せる。
