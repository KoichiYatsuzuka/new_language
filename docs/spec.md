# Arrow 言語仕様の概要

Arrow の**言語機能**を日本語でまとめた文書。⚠ **もとは古い `CLAUDE.md` のコピー**で、
H1 と副題がそのまま残っていた（2026-08-27 に修正）。運用上の指針は
[CLAUDE.md](../CLAUDE.md)、文法の詳細は [docs/grammar/](grammar/) を見ること。

## プロジェクト概要

**Arrow** は独自スクリプト言語。
Python の構文を基盤としつつ、静的型チェックや部分コンパイルを備えた、静的に型付けの強い、実行可能なスクリプト言語（コンパイルによって別言語のスクリプト言語を生成するわけではない）。

- ファイル拡張子: `.ar`
- 実装言語: **Rust**（メイン）、 Python（頒布用）
- インデントによるブロック構造（Python スタイル）
- Rustのように人間を信用しない値、型システム

## めぼしい機能

### 構文解析時の型検査
syntaxエラーを見つけるときに、型検査も同時に行う。静的な型エラーはその時点でエラーとして挙げ、見つかればプログラムが実行されない。
AnyやOptionalなどはtypeguard節内で使わなければ属性へのアクセスができないため、極力具体的な型が静的に分かるようなコードの方が楽に書ける。

動的エラーと異なり、静的エラーはコードの最後まですべて検査した後に列挙する。そのため、一度の解析で複数個所の静的エラーの解決が可能。

---
### let/mut/const
- let: 非可変属性。初期化時のみ代入可能で、それ以降は再代入、メンバへの代入、値を変更しうるメンバの呼び出し、mut属性引数への使用ができない。メンバがlet属性の場合、__init__で初期化した後は変更できない。非可変であることが保証されているので値渡しされることがないため、let属性を使うとコピーによるオーバーヘッドが発生しない。
- mut: 可変。初期化以降の再代入、メンバの書き換え、値を変更しうるメンバの呼び出し、mut属性への引数の使用ができる。mut属性の変数は意図しない値書き換えを防ぐため、他の変数への代入やlet属性関数への使用で値がコピーされる。そのため、速度低下の原因になりうる。freezeによってlet属性に変更可能。
- const: 定数。構文解析時に値が分かる必要がある。それ以外はlet属性と同じ。

- 引数のlet属性: その変数は値が変更されないことが保証されている。C++でいうところのtype const valueに似た機能。let属性の引数を関数内で書き換えようとすると静的にエラーとなる。
- 引数のmut属性: その変数は値が変更されることを述べている。mut属性の変数の参照を渡して、関数ないで書き換えることを前提とする。let属性の値は使えない。また、メンバ引数のselfがmutの場合、そのメンバ関数はインスタンスの内容を変化させることを告知する。

---
### 制御構文の返り値
Rustのように制御構文は返り値を持てる。block_returnを使うことでその制御構文を抜ける。また、制御構文は返り値の型を付記できる。

forとwhile文はloop_yieldを使うことで、各ループ内で返した値を一時保存し、ループを抜ける際に纏めてlistにして返す。

条件分岐を持たず、制御構文と同等のブロックを提供するblckが利用可能。使い捨てのラムダ式や寿命の短い変数（後述）を作るのに利用可能。

---
### スコープ
Pythonではif文などの制御構文内で定義された値はそのまま関数内で継続して保持されますが、この言語では破棄します。スコープ内の変数宣言を活用することで、コードの読み手に与える負担を軽減できます。

---
### デフォルト__init__
__init__を手動で定義しなかった場合、すべての引数を初期化するための__init__を自動的に生成します。

### 継承とtrait
クラスの継承は**許可されていません**。その代わり、インスタンス化不可な抽象仮想クラス、あるいはインターフェースのような役割を担うtraitを継承可能です（Rustのtaritとは役割が異なる）。traitは純粋仮想メンバ関数などを定義可能です。
trait内のメンバは::演算子でアクセス可能。

---
### アクセス指定子
名前マングリングではなく、明示的にpublic, private, protectedを指定可能。C++と一緒。デフォルトはpublic。

---

### テンプレート
TypeVar不要。型チェックもされる。

---
### キャスト特殊メソッド
任意の型へのキャスト特殊メソッドを定義可能。 プリミティブ以外の型へもキャスト可能。

独自演算子=>を用いることで右結合の演算子を実現し、C++のキャストよりも見やすさ・書きやすさを向上。

let属性の引数への代入時、可能ならキャスト演算子を呼び出して自動的にキャストする。

---
### 部分型
- Pythonと異なり、Anyはどんな属性もアクセス可能ではなく、**いかなる属性もアクセス不可**。そのため、Typeguardで明示的ダウンキャストをする必要がある。
同じく、Union, Optionalもダウンキャストの必要がある。Unionは共通traitのみアクセス可能。

- 極力使用を避け、具体的な型の利用を促しつつも、Pythonのライブラリからのグルーコードを容易に書けるようにしている。

---
### Self
- インスタンスと同じ型を明示する特殊型。インスタンス生成時に型が確定する。

---
### new_type
- 同じ仕様の型定義をコピーして別名保存する。構成が同じだが名前が異なるクラスを作れる。PythonのNewTypeに似ている。
- これはSelf型を活用することで、異なる型同士の演算は防ぐことができる。
- premetive型へ使えば値オブジェクトになる。

---
### Pythonモジュールの利用
- Pythonのモジュール利用を標準で実装している。利用方法は二種類。
- Pythonのソースコードを本言語の文法に置き換えながら読み込む方法。並列化などが可能。その代わり、コンパイル済みPython、pycが利用できない。
- Pythonインタープリタを呼んで、コードを実行させる方法。インタープリタは引数をPython与えたり、結果をPythonから受け取ったりする。.pycも含め、大抵のものが利用できるが、Global interpreter lockにより並列化不可。

### .dll, .libの読み込み
- Cコンパイラで作ったdllを読み込んで利用可能。ただし、.hファイルをスタブファイルとして読み込んで型検査を行うため、hファイルが無ければ呼び出せない。hファイルはC++でもOK。ただし、名前マングリングが必要なオーバーロード、テンプレート、名前空間は対応できていない。
- 普通の値渡しとconstなポインタ・参照渡しはlet属性として、constでないポインタと参照渡しはmut属性として扱われる。
- ポインタは内部変換している。そのため、ポインタ渡しは利用可能。ただし、ポインタを直接触る機能はこの言語には露出していない（id()を使ってメモリアドレスを知ることは可能だが、それを利用する機能がない）。
- libファイルはdllにリコンパイルすることで利用可能。
- dllコンパイラはユーザーが用意する必要あり。コンパイラのパスはconfigファイルで指定。
- コンパイルに必要なマクロなどはconfigファイルで指定可能。

### Rust crateの読み込み
- 実装言語であるRustが直接読み込む。Python実装はPyO3を利用。
- crate内の.rsファイルを直接スタブファイルとして読み込んでいる。crateのライブラリフォルダはconfigファイルで指定する必要あり。

--- 
### 部分コンパイル
モジュールなどを部分的に機械語へと変換することで、動作を速くする。型検査はスタブファイルで行う。

現状、bool, int, floatの演算のみが最適化され、それ以外は抽象木をそのまま入れている。それでも数値計算が主なモジュールは30-50倍ほどの加速が可能。

---
### 二種類の実装
- Rust: 速度優先。実行は環境依存。メインの開発言語。
- Python: 頒布優先。実行はinterpreter依存。Rustの実装を再現するようにサブで実装。

### そのうち実装するかもしれないもの
- ポインタ: 諸悪の根源なので極力実装しない。Any型が使えてしまうため、(void*)と同等のことができてしまう。現状、インスタンスの同一性チェックのみに使える。
- Python interpreter経由の並列処理: Pythonインタープリタを呼び出すとき、global interpreter lockにより、シングルスレッドのみ許可されている。Pythonコードを直接読みだす場合には影響なし。
- コンパイルによるclass, collection, str関連の最適化: 今はint, bool, floatの比較、算術演算のみ最適化していて、それ以外は抽象木をそのまま入れている。

## ディレクトリ構成

```
Arrow/
├── src/
│   ├── main.rs          # エントリーポイント・引数パース
│   ├── token.rs         # Token enum・Span・Spanned 定義
│   ├── lexer.rs         # 字句解析器（Vec<Spanned> を返す）
│   ├── ast.rs           # AST ノード定義（Span 埋め込み済み）
│   ├── parser.rs        # 再帰下降パーサー（Vec<Spanned> 入力）
│   ├── type_check.rs    # 静的型検査（パース後・実行前）
│   └── interpreter.rs   # ツリーウォークインタープリタ
├── spec/
│   ├── general.md       # 言語全般仕様
│   ├── keywords.md      # キーワード一覧
│   └── operator.md      # 演算子一覧・優先順位
├── examples/            # コード例
│   ├── XXX.tl           # 実装済み機能の動作例
│   ├── XXX_errors.tl    # 静的なエラーとして許可されないコード例
│　 ...
└── vscode-extension/    # VS Code 拡張（型推論インレイヒント）
    └── src/
        ├── extension.ts
        └── type_infer.ts
```

## 実行フロー

```
ソースファイル
  → Lexer         Vec<Spanned> を生成（各トークンにファイル名・行・列を付与）
  → Parser        AST（Vec<Stmt>）を生成。Expr::BinOp / Stmt::Assign などに Span を埋め込む
  → TypeChecker   StaticTypeError を収集。1 件でもあれば全件表示して exit(1)
  → Interpreter   ツリーウォーク実行
```

---

### 字句解析（`src/lexer.rs`）
- 全キーワード・演算子・リテラル対応
- インデント追跡（INDENT / DEDENT トークン生成）
- 括弧内での改行無視
- 複合キーワード: `not in`, `is not`, `yield from`
- 数値リテラル: 10進・16進・8進・2進・アンダースコア区切り
- 文字列: シングル・ダブル・トリプルクォート・エスケープ
- 各トークンに `Span`（ファイル名・行番号・列番号）を付与
- `Token::SelfType`（`Self`）、`Token::NewType`（`new_type`）

### 構文解析（`src/parser.rs`）
- 変数宣言: `let`（不変）、`mut`（可変）、`const`（不変）
- 代入: `x = expr`、複合代入: `x += expr` など全種
- 式: 演算子優先順位を仕様通りに実装（右結合 `**` 含む）
- 関数呼び出し: `f(args)`、属性アクセス: `obj.attr`
- リストリテラル: `[a, b, c]`
- **タプルリテラル**: `(val, val, ...)` — 2 要素以上または単要素（末尾カンマ必須 `(val,)`）。`(expr)` は括弧グループ式（タプルではない）。空タプル `()` も有効。コンマの後に改行可
- **辞書リテラル**: `{key: value, ...}`（コンマの後に改行可）、空の `{}` は `dict[Any, Any]` 型
- **サブスクリプト演算子**: `expr[index]`（`Expr::Subscript`）。`expr[...]` 直後に `(` が続く場合はテンプレート呼び出しとして解釈（先読みで判別）
- 制御構文: `if` / `elif` / `else`、`while`、`for ... in ...`、`block`
- ジャンプ文: `break`、`continue`、`pass`、`return`、`block_return`
- 関数定義: `fn name(params) -> RetType:`
- クラス定義: `class Name(Base):` （基底クラス複数可）
- **クラスメンバ変数**: クラス本体では必ず型アノテーション付きで宣言する（`[mut|let|const|static mut] name: Type`）。`const` / `static mut` のみ `= default` で初期値を設定可能。`mut`/`let` への初期値指定は静的型エラー。型アノテーションのない宣言はパースエラー
- **デフォルトコンストラクタ自動生成**: `mut`/`let` フィールドが 1 つ以上あるとき、パーサーが `__init__(mut self, field1: Type1, ...)` を AST に挿入する。既存の `__init__` と引数の型・個数が完全一致する場合は生成しない（override）。型・個数が異なる場合は共存する（overload）
- **関数オーバーロード**: 同名関数を引数の型・個数で区別して複数定義可能
- **テンプレート**: 関数・クラスの定義時に型変数を宣言し、呼び出し時に具体的な型を指定する。構文: `fn f[T: Trait]()` / `class C[T: Trait]:`。テンプレート呼び出し構文: `f[Type](args)` / `C[Type](args)`
- **`Self` 型**: クラス・trait 本体でのみ使用可能な型キーワード。型アノテーション・戻り値型・`Self(...)` コンストラクタとして利用可能。クラス外での使用はパースエラー
- **`new_type`**: `new_type NewName: OriginalType` 構文。元のクラス／プリミティブ型と構造的に同一の新しい型を定義する。バインドは常に const（再代入はパースエラー）

### 静的型検査（`src/type_check.rs`）
パース後・実行前に AST を走査し、`StaticTypeError` を収集してまとめて報告する。
エラーメッセージには `ファイル名:行:列: StaticTypeError: ...` の形式で位置情報を付与。

現在検査するエラー種別（`TypeErrorKind`）:

| 種別 | 説明 |
|---|---|
| `AssignToImmutable` | `let` / `const` 変数への代入・複合代入 |
| `IncompatibleComparison` | `<` `>` `<=` `>=` で互換性のない型の比較（例: `str < int`） |
| `CallArgCountMismatch` | 関数呼び出し時の引数個数不一致 |
| `CallArgTypeMismatch` | 引数型の不一致 |
| `MissingParamTypeAnn` | パラメータの型アノテーション欠如（`self` は除外） |
| `MissingReturnTypeAnn` | 戻り値型アノテーション欠如 |
| `UnknownKeywordArg` | 存在しないキーワード引数名 |
| `NoMatchingOverload` | オーバーロード候補のどれにも引数の個数が合わない |
| `SelfTypeMismatch` | `Self` 型パラメータに異なるクラス／new_type のインスタンスを渡した |

- `==` / `!=` は異なる型間でも許容（実行時に `False` を返す想定）
- 型が静的に不明（fn パラメータ等）な場合は実行時に委ねる
- キーワード引数（`f(a=1, b=2)`）の名前・型・個数を検査（`collect_fn_sigs` 前処理で前方参照も対応）
- オーバーロード: 同名関数が複数ある場合、引数個数が一致する候補のみ型検査。複数候補が個数一致する場合は型検査をスキップ
- `Self` 型パラメータ検査: メソッド呼び出し時にレシーバのクラス名（`NamedInstance`）と引数の型を照合し、不一致なら `SelfTypeMismatch` を発行
- コンストラクタ呼び出し（`ClassName(args)`）は `InferredType::NamedInstance(ClassName)` を返す。これにより `new_type` で作成した型を静的に区別できる
- `Expr::Dict(_)` → `InferredType::Dict`。`Expr::Subscript` → `InferredType::Unknown`（実行時委譲）
- `Expr::Tuple(exprs)` → `InferredType::Tuple(Vec<InferredType>)`（各要素を再帰的に推論）。型注釈では `tuple[T1, T2, ...]` 形式を認識し `InferredType::from_ann` で解析
- `dict[K, V](literal)` でリテラルの各キー・値の型を検査。不一致があれば `StaticTypeError` を即時送出（インタープリタ層で実施）

**拡張方法**: `TypeErrorKind` に variant を追加 → `check_stmt` / `check_binop` に arm を追加。

### インタープリタ（`src/interpreter.rs`）
- 変数の不変/可変チェック（実行時）
- 算術・比較・論理・ビット演算（全演算子）
- 文字列連結 (`+`)
- `and` / `or` の短絡評価
- ゼロ除算エラー
- `if` / `elif` / `else`（スコープ分離済み）
- `while`（`break` / `continue` 対応）
- `for ... in ...`（`break` / `continue` 対応、イテレータプロトコル経由）
- `block:` スコープ（`block_return` / `block_yield` 対応）
- リストリテラル評価
- 組み込み関数: `print()`、`range(stop)` / `range(start, stop)` / `range(start, stop, step)`、`len()`
- **関数の実行**: `fn` 定義・呼び出し（位置引数・キーワード引数）・`return`・再帰
- **引数の渡し方（値渡し vs 参照渡し）**:
  - `let` パラメータ（不変）: 参照型（`Instance` / `Dict` / `List`）は **ディープコピー** して渡す。元の `mut` 変数は関数内の操作で一切変更されない
  - `mut` パラメータ（可変）: 参照型はそのまま共有参照として渡す。関数内の変更は呼び出し元に反映される
  - プリミティブ型（`int` / `float` / `str` / `bool` / `None`）: 常に値渡し（Rust の clone）
  - `self`（不変レシーバ）も同様に、`let self` ならディープコピーを受け取る
  - 実装: `bind_args` 内の `deep_copy_value` が `Instance` / `Dict` / `List` のフィールド・要素を再帰コピー
- **クラスの実行**: インスタンス化・フィールド・メソッド呼び出し・`self`・継承（`lookup_method_in_class`）
- `self.x = v` / `self.x += v`（`AttrAssign` / `AttrCompoundAssign`）
- **クラスメンバの種別**:
  - `mut name: Type` — ミュータブルなインスタンス変数。初期値不可。コンストラクタで必ず初期化する
  - `let name: Type` — イミュータブルなインスタンス変数。初期値不可。コンストラクタ（`__init__`）での初回代入のみ許可
  - `const name: Type = default` — クラス変数（全インスタンスで共有）。必ず初期値が必要。`instance.name` および `ClassName.name` でアクセス可能。代入は不可
  - `static mut name: Type [= default]` — 共有可変クラス変数（全インスタンスで共有）。初期値は任意。インスタンス経由・クラス名経由どちらでもアクセス・代入可能
- **フィールド可変性の実行時チェック**: `let` フィールドへの再代入は `TypeError`、`const` クラス変数への代入も `TypeError`
- **関数オーバーロード**: 同名関数が複数あるとき `Value::OverloadedFn(Vec<Rc<FnValue>>)` として蓄積。呼び出し時に引数の個数→型で候補を絞り込む
- **テンプレート関数・クラス**: `Value::TemplateFn` / `Value::TemplateClass` として格納。呼び出し時に型引数の trait 制約を検証し、AST 内の型変数を具体型に置換して実行
- **`Self` 型**: メソッド実行時に `exec_fn_evaled()` がレシーバインスタンスのクラスを `"Self"` としてスコープに束縛。`Self(...)` は現在のクラスのコンストラクタとして機能し、`new_type` のインスタンスで呼び出すと正しく new_type 側のインスタンスを生成する
- **`new_type`**: `Stmt::NewTypeDef { name, original }` を実行。元の値が `Value::Class` の場合は `ClassValue` を `name` でコピーして `const` バインド、プリミティブ型の場合は `value` フィールドを持つラッパークラスを自動生成して束縛
- **イテレータプロトコル**: `for XX in YY:` は `YY.__iter__()` を呼んでジェネレータを取得し、`.next()` を繰り返す。終端で `EndOfIteration` エラーが発生するとループを終了する。`List` / `Str` は組み込みの `__iter__()` を持つ。クラスに `gen __iter__(self) -> T:` を定義することで任意のイテラブルを実装できる。`ClassValue` は `gen_methods: HashMap<String, Rc<GeneratorFnValue>>` フィールドを持ち、`eval_method_call` でジェネレータメソッドを優先的にディスパッチする
- **辞書型**: `Value::Dict(Rc<RefCell<DictData>>)`。内部は並列 `Vec`（`keys` / `items`）で管理。`dict[K, V]()` で空の型付き辞書を生成、`dict[K, V]({...})` でリテラルから型付き辞書を生成。`d[key]` でルックアップ、`d[key] = val` で追加・更新。書き込み時に型制約を検証（アップキャスト許可）。`d.key()` / `d.item()` でキー・値の `List` を返す。`is_truthy` は空なら偽、非空なら真
- **タプル型**: `Value::Tuple(Rc<TupleData>)`。`TupleData` は `values: Vec<Value>`（実値）と `types: Vec<String>`（各要素のランタイム型名）の並列 Vec で実装。公開 API（`get` / `len` / `is_empty` / `element_type` / `all_values` / `all_types`）を介してアクセスし、内部表現は将来変更可能。`is_truthy` は空なら偽、非空なら真。`==` は要素数と各要素を比較
- 値型: `Int`, `Float`, `Str`, `Bool`, `None`, `List`, `Dict(Rc<RefCell<DictData>>)`, `Tuple(Rc<TupleData>)`, `Function(Rc<FnValue>)`, `OverloadedFn(Vec<Rc<FnValue>>)`, `Class(Rc<ClassValue>)`, `Instance(Rc<RefCell<InstanceData>>)`, `TemplateFn(Rc<TemplateFnValue>)`, `TemplateClass(Rc<TemplateClassValue>)`, `Trait(String)`, `Namespace(Rc<NamespaceData>)`, `PyObject(Rc<PyObjHandle>)`
- **トレイトの実行時表現**: `Stmt::TraitDef` 実行時に `Value::Trait(name)` をスコープに束縛。`Error` トレイトは起動時に自動登録されるため常にスコープに存在する
- **`import[py]`**: `.py` ファイルを `python_converter.rs` で tl の AST に変換して実行する。Python モジュールの関数・クラスが tl の通常の値として利用可能
  - Python 関数は `is_python: true` フラグを持ち、引数リストにないキーワード引数を `kwargs` dict としてスコープに注入する（`**kwargs` 相当）
  - `dict.keys()` / `dict.values()` を `key()` / `item()` の別名として提供
- **`import[py-int]`**: PyO3 経由で Python ランタイムに接続し、実際の Python モジュールを実行時にロードする。Cython コンパイル済みモジュール（`.pyd`/`.so`）や標準ライブラリも扱える
  - 型シグネチャは `.pyi` または `.py` ファイルから取得して静的型検査に使用する（パーサーが処理）
  - 実行時は `Value::PyObject(Rc<PyObjHandle>)` で Python オブジェクトを opaque に保持
  - `src/interpreter/py_interop.rs` が tl↔Python の型変換・呼び出しを担当
  - 型変換: `int`/`float`/`str`/`bool`/`None`/`list`/`tuple`/`dict` はネイティブ tl 値に自動変換。それ以外は `PyObject` のまま保持
  - `PyObject` の属性アクセス（`obj.attr`）・メソッド呼び出し（`obj.method()`）・直接呼び出し（`obj()`）をすべて PyO3 経由でディスパッチ
  - `Interpreter::add_python_search_dir()` で `sys.path` に追加するディレクトリを登録可能（`main.rs` がソースファイルのディレクトリを自動登録）

### VS Code 拡張（`vscode-extension/`）
- `.ar` ファイルのシンタックスハイライト
- 型推論インレイヒント（変数名の右に `: int` などを表示）
- 対応型: `int`, `float`, `str`, `bool`, `None`, `unknown`

## 未実装の主な機能

⚠ **この節は 2026-08-27 に実測で洗い直した。** 以前ここに並んでいた
「セット型／例外処理／インポート／`match` 文／LLVM IR コード生成／Python 実装」は
**すべて実装済み**（例題の構文カバレッジと `--compile` の実行で確認）。
⚠ **「現在はツリーウォークインタープリタで動作」も誤り** — 解釈実行は**バイトコード VM 一本**
（詳細は [README.md](../README.md) の「実行方式」）。

- **`dataclass`**: 標準サポートは無い（`src/` に実装が存在しない）
- **値オブジェクト（型キャスト）による誤演算防止**: 未実装
- **戻り値型の実行時チェック**: 宣言型と実際の `return` 値の突き合わせ。
  ⚠ **未確認**（`mustbe` / `=>` と FFI 境界検査はあるが、戻り値の常時検査は見つからない）
- ⇒ **残タスクの正本は [implementation_logs/FUTURE_FEATURE.md](../implementation_logs/FUTURE_FEATURE.md)**。
  この節は言語機能の目安として読むこと。

## 言語仕様の要点（Python との違い）

- 変数宣言に `let` / `mut` / `const` が必要
- 関数定義は `def` ではなく `fn`
- パース後・実行前に静的型検査を行い `StaticTypeError` を出す
- テンプレートをサポート: `fn f[T: Trait]()` / `class C[T: Trait]:` で型変数を宣言し、`f[Type](args)` で呼び出す
- 変更しうる引数には `mut` を明示する必要がある
- 空のコレクションは型を明示しなければならない（`list[int]` など）
- `enum` は実装済み（`dataclass` 相当は未実装）
- 値オブジェクト（型キャスト）による誤演算防止（未実装）

### クラスメンバ変数の宣言規則

クラス本体で宣言できるメンバ変数は以下の 4 種類のみ。いずれも **型アノテーションが必須**。

```
mut        name: Type             # ミュータブルなインスタンス変数（初期値不可）
let        name: Type             # イミュータブルなインスタンス変数（初期値不可）
const      name: Type = default   # クラス変数（必ず初期値が必要）
static mut name: Type [= default] # 共有可変クラス変数（初期値は任意）
```

- `mut` / `let` フィールドは初期値を持てない（静的型エラー）。`__init__` などで明示的に初期化する必要がある
- `const` フィールドは初期値が必須。全インスタンスで共有される不変クラス変数。インスタンス経由・クラス名経由どちらでもアクセス可能。代入は不可
- `static mut` フィールドは初期値を省略できる。全インスタンスで共有される可変クラス変数。インスタンス経由・クラス名経由どちらでもアクセス・代入可能
- `let` フィールドは `__init__` 内での初回代入のみ許可。それ以降の代入は `TypeError`

### デフォルトコンストラクタ（auto-init）

`mut`/`let` フィールドが存在するとき、パーサーが自動的に `__init__` を生成する。

```
fn __init__(mut self, field1: Type1, field2: Type2, ...) -> None:
    self.field1 = field1
    self.field2 = field2
    ...
```

生成の可否は既存の `__init__` 定義と比較して決まる:

| 既存の `__init__` | 自動生成 | 備考 |
|---|---|---|
| なし | される | |
| 引数の型・個数が異なる | される | 既存定義とオーバーロードとして共存 |
| 引数の型・個数が完全一致 | されない | 既存定義が優先（override） |

- `const` / `static mut` フィールドは auto-init に含まれない

### 関数オーバーロード

同名の関数・メソッドを異なる引数の型・個数で複数定義できる。

```tl
fn add(a: int, b: int) -> int:   return a + b
fn add(a: str, b: str) -> str:   return a + b
```

- 呼び出し時は引数の個数 → 型の順で候補を絞り込み、最初にマッチしたものを実行
- どの候補にも一致しない場合は実行時エラー
- 静的型検査: 個数が合う候補がなければ `NoMatchingOverload` エラー

### テンプレート

関数・クラス・**trait** の定義時に型変数と trait 制約を宣言できる。

```tl
fn func[T1: Trait1, T2: Trait2](a: T1, b: T2) -> T1:
    ...

class MyClass[T: Trait1 and Trait2]:
    mut item: T
    fn get(self) -> T:
        return self.item

trait Container[T: Printable]:
    mut item: T
```

**呼び出し構文**: `func[ConcreteType](args)` / `MyClass[ConcreteType](args)`

**trait テンプレートの継承**: クラスは `class Foo(MyTrait[ConcreteType]):` の形式で template trait を具体化して継承できる。型変数はフィールドの型アノテーションに置換されて auto-init が生成される。

```tl
trait Printable:
    fn to_str(self) -> str:
        ...

class MyInt(Printable):
    mut value: int
    fn to_str(self) -> str: return "MyInt"

fn describe[T: Printable](item: T) -> str:
    return item.to_str()

let a = MyInt(42)
let s = describe[MyInt](a)   # → "MyInt"
```

**制約の仕様**:
- 型変数ごとに `: Trait` で trait 制約を指定する
- `and` で複数 trait を結合できる: `[T: TraitA and TraitB]`
- 呼び出し時に型変数に渡された具体型が制約を満たさない場合、`TemplateError` を送出して停止
- 具体型が trait を実装しているかは、クラス定義の `bases`（継承 trait リスト）で判定する
- 組み込み型（`int`, `str` 等）は trait を実装していないため、trait 制約を持つテンプレートには渡せない

**実装の仕組み**:
1. パース時: テンプレートパラメータ付きの `FnDef` / `ClassDef` を AST に生成し、インタープリタが `Value::TemplateFn` / `Value::TemplateClass` として格納
2. 呼び出し時: 型引数ごとに `type_satisfies_trait()` で制約を検証し、違反があれば即エラー
3. 制約通過後: `subst_*` 関数群で AST 内の型変数名を具体型名に置換した新しい AST を生成して実行

**現在の制限**:
- テンプレート関数のオーバーロードは未対応（同名テンプレート関数を複数定義すると後の定義で上書き）
- `list[T]` のような複合型に含まれる型変数は置換されない（`parse_type_expr` が基底型名のみ返すため）
- テンプレート呼び出しの静的型検査は未対応（制約チェックは実行時のみ）

### `Self` 型

クラス・trait 本体でのみ使用できる特殊な型キーワード。

```tl
class Vec2:
    mut x: int
    mut y: int
    fn clone(self) -> Self:          # 戻り値型として使用
        return Self(self.x, self.y)  # Self(...) でコンストラクタ呼び出し
    fn add(self, other: Self) -> Self:  # パラメータ型として使用
        return Self(self.x + other.x, self.y + other.y)
```

**仕様**:
- クラス・trait の外で `Self` を型アノテーションや式に使うとパースエラー
- 実行時: `exec_fn_evaled()` がレシーバインスタンスのクラスを `"Self"` としてスコープに束縛する
- `Self(...)` コンストラクタはレシーバの実際のクラスを使うため、`new_type` で派生した型でも正しく動作する
- 静的型検査: `other: Self` 型パラメータに異なるクラスのインスタンスを渡すと `SelfTypeMismatch` エラー

### `new_type`

既存のクラス／プリミティブ型と構造的に同一だが **名前が異なる** 新しい型を定義する。Python の `NewType` に近い。

```tl
class Meters:
    mut value: int
    fn add(self, other: Self) -> Self:
        return Self(self.value + other.value)

new_type Kilometers: Meters   # Meters と同じ構造だが別の型
new_type Celsius: int         # プリミティブのラッパー（.value フィールドを持つクラスが生成される）
```

**仕様**:
- 構文: `new_type NewName: OriginalType`
- バインドは常に `const`。再代入は**パースエラー**（型エラーではない）
- 元の型が `class` の場合: `ClassValue` を `name` だけ変えてコピー。メソッドは共有される
- 元の型がプリミティブ（`int`, `str` 等）の場合: `value` フィールドを持つラッパークラスを自動生成
- `Self` との相互作用: `new_type` 由来のインスタンスでメソッドを呼ぶと、`Self` はその new_type のクラスに解決される。同じ元クラスから派生した 2 つの new_type は互いに `Self` 型引数として渡せない
- 静的型検査: `a.method(b)` で `method` の引数が `Self` 型であり、`a` と `b` が異なるクラス／new_type のとき `SelfTypeMismatch` を発行

**エラー例**:
```tl
class Meters:
    mut value: int
    fn add(self, other: Self) -> Self: ...

new_type Kilometers: Meters

let m = Meters(100)
let km = Kilometers(5)
m.add(km)   # StaticTypeError: parameter 'other' of 'add' expects 'Self' = 'Meters' but got 'Kilometers'
```

### イテレータプロトコル

`for XX in YY:` 構文はイテレータプロトコルを通じて動作する。

**動作フロー**:
1. `YY` を評価する
2. イテレータ（`Value::Generator`）を取得する:
   - `Value::List` / `Value::Str` → 組み込みの `__iter__()` で Generator を生成
   - `Value::Generator` → そのまま使用
   - `Value::Instance` → `__iter__()` メソッドを呼び出してジェネレータを取得
3. `generator.next()` を繰り返し呼び出し、値を `XX` に束縛してループ本体を実行
4. `next()` が `EndOfIteration` エラーを返したらループを終了し、実行を継続する

**カスタムイテラブルの定義**:

```tl
class NumberRange:
    mut low: int
    mut high: int

    gen __iter__(self) -> int:
        mut i = self.low
        while i <= self.high:
            yield i
            i += 1

let r = NumberRange(1, 5)
for v in r:
    print(v)   # 1, 2, 3, 4, 5
```

**仕様**:
- `gen __iter__(self) -> YieldType:` の形式でクラスにイテレータを定義する
- `__iter__()` はジェネレータ（`Value::Generator`）を返さなければならない
- ジェネレータが終端に達した後に `.next()` を呼ぶと `EndOfIteration: generator is exhausted` エラーを送出する
- `for` ループはこの `EndOfIteration` を内部でキャッチしてループを終了する（エラーはループ外に伝播しない）
- `break` / `continue` はイテレータベースのループでも通常通り動作する
- `gen_methods` はクラスボディの `Stmt::GenDef` から収集され、`ClassValue` に格納される。`new_type` や テンプレートクラスのコピー時にも引き継がれる

### 辞書型（dict）

`dict[K, V]` 構文でキーと値の型を指定した辞書を扱う。

```tl
# 辞書リテラル → dict[Any, Any]
let scores = {"alice": 95, "bob": 82}
print(scores["alice"])   # 95

# 型付きコンストラクタ（空）
mut d = dict[str, int]()
d["x"] = 10

# 型付きコンストラクタ（リテラルから）
let m = dict[int, str]({1: "first", 2: "second"})
print(m[1])   # first

# key() / item()
let ks = d.key()    # キー一覧（List）
let vs = d.item()   # 値一覧（List）
```

**仕様**:
- `{}` または辞書リテラル単体 → `dict[Any, Any]` 型（型制約なし）
- `dict[K, V]()` → 空の型付き辞書を生成。以降の `d[k] = v` で型検証を行う
- `dict[K, V]({...})` → リテラルの各キー・値が K・V に適合するか検査。不一致なら `StaticTypeError` を送出
- アップキャストは許容（例: `dict[str, float]` に `int` 値を書き込める）
- `d[key]` でルックアップ。存在しないキーは `KeyError` を返す
- `d[key] = val` で追加・更新。型付き辞書では書き込み時に型制約を検証
- `d.key()` → キーの `List`、`d.item()` → 値の `List`
- `is_truthy`: 空なら偽、非空なら真
- 内部実装: `DictData { key_type, item_type, keys: Vec<Value>, items: Vec<Value> }`（並列リスト）

### タプル型（tuple）

`(val, val, ...)` 構文で生成する不変・固定長シーケンス。各要素は異なる型を持てる。

```tl
let empty  = ()                    # 空タプル
let single = (42,)                 # 単要素（末尾カンマ必須）
let pair   = (1, "hello")          # 複数要素
let nested = ((1, 2), (3, 4))      # ネスト

# (expr) だけはタプルではなくグループ式
let n = (10 + 20)   # → 30（int）

# 多行リテラル（括弧内では改行がトークンから除去される）
let triple = (
    100,
    200,
    300
)

# 等価比較：要素数と各要素の値で比較
print((1, 2) == (1, 2))   # True
print((1, 2) == (1, 3))   # False

# 真偽値：空なら偽、非空なら真
print(not ())       # True
print(not (1, 2))   # False
```

**仕様**:
- 構文: `()` / `(expr,)` / `(expr, expr, ...)`
- `(expr)` は括弧グループ式（タプルではない）
- リテラル生成時にその場で型解析が行われ、静的型は `tuple[T1, T2, ...]` となる
- タプルは不変（要素の追加・変更・削除は不可）
- 型注釈では `tuple[T1, T2, ...]` 形式を使用（例: `let t: tuple[int, str] = (1, "a")`）

**内部実装**（変更の可能性あり、公開 API は安定）:
- `TupleData { values: Vec<Value>, types: Vec<String> }` — 実値と型名の並列リスト
- 公開 API: `get(i)`, `len()`, `is_empty()`, `element_type(i)`, `all_values()`, `all_types()`
- `Value::Tuple(Rc<TupleData>)` として表現（`Rc` なので Clone が安価）

## 次に実装すべき機能

⚠ **ここにあった優先順リスト（セット型／例外処理／`match` 文／クロージャ／LLVM IR コード生成）は
5 件とも実装済み**だったので削除した（2026-08-27・実測）。

⇒ **残タスクは [implementation_logs/FUTURE_FEATURE.md](../implementation_logs/FUTURE_FEATURE.md)**
（意義・前提・実装時の留意点つき）。この文書に残タスクを書き足さないこと（二重管理になる）。
