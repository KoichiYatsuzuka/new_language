# 外部言語との値の受け渡しを作り直す（文字列・配列・構造体・ポインタ）

`import[cpp-dll]` のライブラリから `str` が返ってこない、という報告を受けて、全ブリッジ（C/C++・Rust・
Python・JS・C#）の値の受け渡しを検査した。文字列・コレクション・インスタンスの多くが、**型検査を通った
あとで黙って別の値になる**（`None`・`null`・整数）か、**プロセスごと落ちる**ことが分かった。

本書は、ポインタの受け渡しに共通のプロトコル（`PtrProtocol`）を定め、Arrow 側では文字列（バイト列）・配列・
構造体を展開してコピーした Arrow の値として扱い、ポインタのまま扱うものはポインタクラス `Ptr[T]` にする、
という形に作り直す計画書である。

- 起票: 2026-10-12（利用者の報告「cpp-dll から str が返らない」から）
- 調査: Arrow `2aa53ed`（release ビルド）
- 状態: フェーズ 1 完了（1-1〜1-6・§5）。フェーズ 2 着手（2-1〜2-4 完了）
- 改訂: 2026-10-12 未決事項 U-1〜U-6 を利用者の回答で決定（D-7〜D-12）。フェーズを依存関係に合わせて組み直した
- 改訂: 2026-10-12 U-5（再）・U-7〜U-9 を決定（D-10〜D-13）。U-8 を (b) にしたため展開は Arrow 側で明示的に行う
- 改訂: 2026-10-12 U-10・U-11 を決定（D-14・D-15）。未決事項なし

## 0. 何が起きているか

検査の詳細は付録 A。ここには決定に効いたものだけを書く。

### 0.1 C/C++（`cpp-dll` / `cpp-lib`）

| 症状 | 重さ | 箇所 |
|---|---|---|
| `char*` / `const char*` の戻り値が常に `None`（型スタブ上は `str`） | 黙って誤り | `cpp_bridge/types.rs:63` |
| 構造体ポインタ・`void*` の戻り値が `None`。`void*` は往復できない（引数には Arrow 内部のハンドル番号が渡る） | 黙って誤り | `cpp_bridge/types.rs:44` / `:60` |
| 文字列フィールドを持つ構造体を C へ渡すと segfault（ハンドル番号をポインタとして渡す） | 落ちる | `cpp_bridge/codegen.rs:127` |
| cpp-dll で by-value の構造体を返す関数を呼ぶと segfault（シムが無く、構造体戻りをポインタ戻りとして呼んでいる） | 落ちる | `cpp_bridge/codegen.rs:307` |
| `wchar_t*` を UTF-8 の `char*` として渡す（cpp-lib ではその関数が黙って消える） | 黙って誤り | `header_parser/decls.rs:398` |
| `char*` の出力バッファに書き戻さない | 黙って誤り | − |
| 中身を公開しない構造体のポインタ（`Db*`）が「書き込み用の出力引数」とみなされ、`let` の変数を渡せない | 偽の静的エラー | `parser/imports/cpp.rs:140` |

### 0.2 他の言語

| 言語 | 症状 | 箇所 |
|---|---|---|
| cs-proc / cs-dll | 配列・`List`・`Dictionary`・C# オブジェクトの戻り値が整数ハンドルになる（スタブは `list[str]` / `Person` と宣言） | `cs_proc_runtime.rs:183`・`cs_dll_runtime.rs:196`・`ArrowPipeHost.cs:252` |
| cs-proc / cs-dll | Arrow のリストを渡すと C# 側に `null` が届く | `cs_proc_runtime.rs:161` |
| cs-proc | Arrow → C# の非 ASCII 文字列が化ける（受信バイトを `(char)b` で 1 バイトずつ文字にしている） | `ArrowPipeHost.cs:143` |
| js-proc | JS のオブジェクトが `"k=v"` の文字列リストになる。Arrow の dict / tuple / set を渡すと `null` | `js_proc_runtime.rs:170` / `:150` |
| js-proc | 境界の型照合: `-> float` の関数が整数値を返すと誤って FfiTypeError。`list[int]` などコンテナ宣言の戻り値は照合されない | `ffi_boundary.rs:260` |
| js-proc | 実行ごとに node プロセスが残る | `js_proc_runtime.rs:48` |
| rs | `pub String` フィールドを持つ構造体でラッパーがコンパイルエラー（import 全体が失敗） | `rs_loader/codegen.rs:260` |
| rs | private フィールドを持つ構造体を import できない（`new()` を優先する経路が一度も通らない） | `rs_crate/parse.rs:248` / `:503` |
| rs | `Vec<String>`・`Option<T>`・`&[i64]`・タプルを使う関数が黙って消える | `rs_crate/parse.rs:552` |
| rs | ネイティブ側が書き換えたあとも `obj.field` が古い値のまま | − |
| py-int | Arrow の set / インスタンスを渡すと TypeError（型名が `'unknown'`）。Python の set を受け取れない | `py_interop.rs:69` |

`import[py]`（変換して同じインタープリタで実行）は境界が無く、値はそのまま届く。

### 0.3 非同期タスクから外部関数を呼べない（D-6 で別タスク）

| ブリッジ | 結果 | 原因 |
|---|---|---|
| cpp-* | `native library not loaded`。メインスレッドで先に一度呼んでいれば成功する | 非同期用のインタープリタが DLL の表を空で作る（`interpreter.rs:746`） |
| cs-proc / cs-dll | `bridge not launched` | ブリッジの登録表がスレッドローカル（`cs_proc_runtime.rs:42`・`cs_dll_runtime.rs:28`） |
| js-proc | 動く | 登録表がグローバル |

## 1. 原因と、設計を決めた実測

### 1.1 原因

1. **扱えないものを黙って置き換える分岐**: 各ブリッジの `_ =>` / `other =>` が `None` / `null` / 整数ハンドル /
   `"k=v"` に変換する。⇒ D-4 と、規約「掬いきれなかった選択肢はエラーにする」（`.claude/rules/regulations.md`）
2. **境界の型照合が静的型付け言語を除外している**（`ffi_boundary.rs:66`）。「向こう側が型を守る」前提だが、
   ブリッジ自身が値を落とすので成り立たない
3. **ハンドル経路が Arrow 内部のハンドル番号をポインタとして扱う**（`c-abi-interop` P7 の残課題）
4. **C のポインタには長さも所有者も無い**のに、ブリッジがそれを補う手段を持たない

### 1.2 実測で分かったこと（詳細は付録 B）

- **配列の長さはメモリのどこにも無い。** 先頭の直前に要素数があるのは「C++ の `new T[n]` で `T` が非自明な
  デストラクタを持つ」ときだけ（配列クッキー）。ブリッジが扱う数値・POD 構造体の配列には無い。COM の
  `BSTR` だけは仕様で長さが前置されている
- **確保の方法は見分けられない。** `new` と `malloc` は同じプロセスヒープの同じブロックになる。DLL イメージ
  （`.rdata` / `.data`）とスタックは `VirtualQuery` で見分けられる
- **ヒープへの問い合わせは安全でない。** `HeapValidate` は debug CRT のブロックの途中を指すポインタに `TRUE` を
  返し、続く `HeapSize` がアクセス違反で落ちた。`_msize` は不正なポインタで `0xC0000374` の強制終了。
  `HeapWalk` は安全だがブロック数に比例して遅い
- **`std::vector` / `std::string` のレイアウトはビルド設定で変わる**（debug は先頭にポインタが 1 つ増える）。
  仮定が外れると `string` は「長さ 0 の空文字列」として黙って通った
- **展開してコピーするコストは、ふつうの使い方では誤差。** 外部呼び出し 1 回が約 300 ns、Arrow のループが
  40 ns/要素に対し、文字列・構造体 1 個は 40〜150 ns、数値配列は 1.3〜7 ns/要素。重いのは構造体の配列
  （約 100 ns/要素）と、Arrow で触らない大きなバッファ（`Value` 32 バイトへの膨張で uint8 は 32 倍）

⇒ 長さ・所有権・型は**プロトコルから得る**（D-7）。メモリを覗いて推測しない。

## 2. 決定

- **D-1**: 同じプロセス内のブリッジ（C/C++・cs-dll・py-int・rs）は、ポインタを次の C の構造体
  `PtrProtocol` で受け渡す。C/C++・C#・Arrow（ブリッジ内部）で同じ形にする。
  ```c
  typedef struct {
      uint64_t    address;     /* 先頭アドレス（NULL は 0） */
      const char* elem_type;   /* 要素の型名（"int32" / "float64" / "uint8" / 構造体名 …・静的な文字列・D-12） */
      uint32_t    flags;       /* const・NUL 終端・STATIC（D-15） など */
      int64_t     length;      /* 要素数（単体は 1） */
      void      (*release)(void* ctx, uint64_t address);  /* 解放関数。NULL は STATIC のときだけ（D-15） */
      void*       release_ctx; /* 解放に必要な情報（ヒープ・GCHandle・vector 本体など） */
  } PtrProtocol;
  ```
  - 解放責任は `bool` ではなく `release` で表す（解放の方法を知っているのは確保した側だけ）
  - 長さ・所有権・解放の責任は、このプロトコルにだけ書く（D-7）
- **D-2**: Arrow で値として扱うもの（バイト列・配列・構造体）は、**展開してコピー**した Arrow の値にする
  （Arrow が所有する値にする。他のインスタンスと同じ扱いにして仕様を一貫させる・利用者の判断）。展開した値は
  元のポインタの情報を持たない（C のメモリとはつながらない）。
  - いつ展開するか: by-value の構造体と、生の `char*` の戻り値（D-7・借用なので）は境界で展開する。
    `PtrProtocol` で届いたものは `Ptr[T]` で受け取り、Arrow 側で明示的に展開する（D-10）
- **D-3**: `PtrProtocol` で届いたもの・展開できる中身を持たないポインタ（`void*`・前方宣言だけの構造体）は
  **ポインタクラス `Ptr[T]`** として Arrow に見せ、C へ渡し返せるようにする。
  - できる: 比較・C へ渡す・（所有していれば）`release()`・長さと型が分かるものの展開
    （`to_list() -> list[T]`、文字データの `to_bytes() -> bytes`。どちらもコピーを作る）
  - できない: アドレスの取得・加算、整数からの生成、中身の書き込み（D-11）
  - `Ptr[Db]` と `Ptr[FILE]` は別の型。`void*` の引数はどの `Ptr` でも受ける。`Ptr[void]` → `Ptr[T]` は
    明示的な変換で、元の型と一致するときだけ成功する
  - 値が要らなくなったら `Drop` で 1 回だけ解放する（ファイルの値 `objects.rs:152` と同じ形）。コピーは共有。
    元の DLL への参照を持ち、`Ptr` が残っている間は DLL をアンロードしない
  - C 関数の側で解放する場合は `PtrProtocol*` で受け取り、解放したら `address` を 0 に書き戻す。Arrow は
    呼び出し後にそれを見て `Ptr` を無効にする（以後の使用はエラー）。生のヘッダから作った `Ptr` は借用
    （Arrow は解放しない）
- **D-4**: **掬いきれなかった選択肢はエラーにする**（利用者の指示・規約に追加）。黙って `None` / `null` / `0` /
  整数ハンドル / `"k=v"` にしない。対応していないシグネチャは黙って捨てずに報告する
- **D-5**: 別プロセスのブリッジ（cs-proc・js-proc）ではアドレスに意味が無い。D-2 に従い中身をコピーして運び
  （値表現を拡張する）、`Ptr` を渡したら D-4 に従いエラーにする
- **D-6**: 非同期タスクから外部関数を呼べない件（0.3）は、別タスクとして後で解決する（利用者の判断・フェーズ 6）
- **D-7**（U-1）: 長さ・所有権・解放の責任は**プロトコルにだけ書かせる**。SAL 注釈や補足宣言は読まない。
  プロトコルを使っていない既存のヘッダ（Win32・DxLib など）の生のポインタは、固定の規則で扱う:
  | 生のポインタ | 扱い |
  |---|---|
  | `char*` / `wchar_t*` の戻り値 | NUL 終端の借用バイト列（D-9）。解放しない |
  | それ以外のポインタの戻り値（`void*`・構造体ポインタ・`int*`） | `Ptr[T]`（借用） |
  | 引数の構造体ポインタ・プリミティブの書き込みポインタ | 今の仕組みのまま（ゼロコピー・書き戻し。skill `c-abi-interop` の P3〜P7） |
  | 引数の `char*` | `bytes`（D-13） |
  - ⚠ 配列や所有権付きのメモリを受け渡したい既存のライブラリには、`PtrProtocol` を返す薄い C のラッパーを書く
  - ⚠ 古いライブラリが malloc した `char*` を返す場合は、解放されず漏れる（落ちはしない）
- **D-8**（U-2）: ポインタ由来の値は**すべて `Option[...]` で受け取る**（NULL は `None`）。引数にも `None`（NULL）を
  渡せる。by-value の構造体と数値は対象外。⚠ 今は `char*` の引数に `None` を渡すと空文字列になる（黙った変換）
- **D-9**（U-3）: `char*` は**バイト列として受け取り、文字列への変換は Arrow 側で後から行う**。Arrow にはバイト列の
  型が無いので、組み込み型 `bytes` を新設する（`len`・添字・スライス・反復・比較・`decode(encoding)`、
  `str.encode(encoding)`）。`wchar_t*` も UTF-16LE のバイト列として受け取る。
  - 文字列型を持つ言語（C#・Python・JS）の文字列は `str` のまま。Python の `bytes`・C# の `byte[]`・
    Rust の `Vec<u8>`（今は 16 進文字列）は `bytes` に揃える（フェーズ 5）
- **D-10**（U-4・U-8）: `PtrProtocol` で届く配列は**常に `Ptr[T]` で受け取り**、配列として扱うときに Arrow 側で
  `p.to_list()` を呼んで展開する（利用者の判断・(b) 案）。ポインタのまま扱うなら、そのまま C へ渡し返す。
  展開した `list` は元の情報（アドレス・型・解放）を保持しない。C の幅で詰めた配列型は作らない（バイト列は
  D-9 の `bytes`）。所有権付きのメモリは `Ptr` の `Drop`（または `release()`）で解放する
- **D-11**（U-5）: **外部ライブラリとの境界を越えて、値の書き換えを前提としたポインタ渡しはできない**こととする
  （利用者の判断）。ライブラリが持ち続けるメモリを指し、呼び出し側の書き換えがライブラリに伝わる、または
  ライブラリの更新を呼び出し側が読み直す API は扱わない。`Ptr` に書き込みの手段を持たせないことで守る。
  呼び出しの間だけ C が Arrow 側のメモリへ書き込む引数は対象外（D-14）
- **D-12**（U-6・U-9）: 要素の型は、Arrow 側では `type` の値として見せる（`Ptr[T]` の静的な型と一致するかを
  実行時に照合する）。ABI 側は型名の文字列（`const char*`・静的な文字列。`"int32"` / `"float64"` / `"uint8"` /
  ヘッダの構造体名など）で、境界で Arrow の `type` に引き当てる。知らない名前はエラー（D-4）
- **D-13**（U-7）: `const char*` の引数は `bytes` だけを受け付ける。`str` を渡したら静的エラーにして
  `.encode(...)` を促す（UTF-8 で黙って渡すと、Shift-JIS 前提のライブラリで文字化けが黙って起きるため）。
  既存の例題（`dxlib_string.ar` など）は書き換える
- **D-14**（U-10・暫定）: 呼び出しの間だけ C が Arrow 側のメモリへ書き込む引数は D-11 の対象外とし、扱う
  （利用者の判断・いったん推奨どおり）。`V3* out` のゼロコピー書き戻し・`double*` の書き戻し（skill
  `c-abi-interop` の P3〜P7）・`char*` の出力バッファ・`mut` の `list` の書き戻し。呼び出しが終わればメモリは
  共有されず、書き戻した後は Arrow の値なので D-2 と矛盾しない
- **D-15**（U-11）: `PtrProtocol` には**解放の責任を必ず明記させ、Arrow の値と同じ規約で解放させる**（利用者の判断）。
  - ライブラリが `PtrProtocol` で Arrow に渡したメモリは、Arrow が参照している間は有効でなければならない。
    Arrow は最後の参照がなくなったとき（`Drop`）か `release()` で、`release` を 1 回だけ呼ぶ。⇒ 後から
    `to_list()` / `to_bytes()` しても常に安全で、期限切れの検査は要らない
  - `release` が NULL でよいのは、flags に STATIC（DLL のアンロードまで有効で、内容も変わらない）があるときだけ。
    STATIC の無い NULL はエラー（D-4）
  - 次の呼び出しで上書きされる一時的なバッファを `PtrProtocol` で渡すのは規約違反。ライブラリ側でコピーして
    `release` を付けて渡す（違反は C 側の誤りで、Arrow からは検知できない）
  - 逆向き（Arrow → C の引数）は呼び出しの間だけ有効。C が保持したい場合は C 側でコピーする（C の慣習・今の
    `char*` の引数と同じ）

### 未決事項

なし（2026-10-12 時点）。

## 3. 実装予定

- フェーズ 1: C/C++ の既存の不具合を止める（黙って誤る・落ちる → エラー。プロトコルと独立）
- フェーズ 2: 他言語ブリッジの既存の不具合を止める（同上）
- フェーズ 3: 新しい値の型（`bytes`・`Ptr[T]`）
- フェーズ 4: `PtrProtocol` の定義と、境界での変換（C/C++）
- フェーズ 5: 他言語への波及
- フェーズ 6: 非同期タスクからの外部関数呼び出し（別タスク・D-6）

| タスク | 内容 | 前提 | 規模 |
|---|---|---|---|
| **1-1** | 再現用のテストライブラリを置く: C/C++（文字列・構造体・ポインタ・配列を返す/受ける関数。DLL と `_x64.lib`）・Rust crate・C# プロジェクトと、各ビルドスクリプト（`build_vec_math.ps1` と同じ形） | なし | 小 |
| **1-2** | C/C++ の黙って誤る経路をエラーにする: `char*` / `T*` / `void*` / 構造体ポインタの戻り値、ポインタ型フィールドを持つ構造体の受け渡し、`wchar_t*`、`char*` の出力バッファ、`char*` の引数への `None`（今は空文字列）。typed 経路に合わないポインタ引数をハンドル経路へ落とさない | 1-1 | 中 |
| **1-3** | cpp-dll の by-value 構造体（引数・戻り値）の呼び出し規約を直す（ラッパーに `#[repr(C)]` の型を生成して値で受け渡す。typed 経路と同じ形） | 1-1 | 中 |
| **1-4** | `ar_make_str` の UTF-8 検証（`from_utf8_unchecked` をやめる） | なし | 小 |
| **1-5** | cpp-lib の恒久キャッシュ DLL を、ラッパー生成が変わったら作り直す（版の刻印） | なし | 小 |
| **1-6** | （1-3 で発見・2026-10-13 利用者が採用）C の整数型の幅と符号を区別する。`char` / `short` / `int` / `unsigned` / `uint32_t` などを `CType::Int`（符号付き 4 バイト）にまとめていて、`unsigned` の 2^31 以上が負になる・`unsigned char` の戻り値でレジスタの上位のごみまで読む・`short*` などの書き込みポインタが 4 バイト書く、などが起きていた（構造体のフィールドだけは 1-3 で「レイアウト不明」として止めていた）。Windows の `long`（4 バイト）・`bool`（1 バイト）も同じ | 1-3 | 中 |
| **2-1** | ✅ C#: 配列・`List`・`Dictionary` の戻り値と、リストの引数をエラーにする。スタブにあるクラスを返すメソッドは `CsObject` に包む。`str` 以外の注釈（`str \| None` など）の戻り値を正しく扱う（C# の nullable 注釈を読む・§5） | 1-1 | 中 |
| **2-2** | ✅ cs-proc ホスト（`ArrowPipeHost.cs`）の受信を UTF-8 で復号する | なし | 小 |
| **2-3** | ✅ js-proc: オブジェクトの戻り値（`"k=v"`）と、dict / tuple / set / インスタンスの引数をエラーにする。境界の型照合で float 宣言への整数値を通し、コンテナ宣言の戻り値も照合する | なし | 小 |
| **2-4** | ✅ js-proc の node プロセスが終了時に残る問題を直す | なし | 小 |
| **2-5** | rs: 対応していないシグネチャを import 時に報告する。`pub String` フィールドのセッター、`new()` を使う構築、ネイティブ側の書き換え後のフィールド値を直す | 1-1 | 中 |
| **2-6** | py-int: Python の set を Arrow の set に、Arrow の set を Python の set に変換する。変換できない値のエラーに型名を出す | なし | 小 |
| **3-1** | 組み込み型 `bytes` を足す（D-9）: 値の種類・型検査・`len` / 添字 / スライス / 反復 / 比較・`decode(encoding)`・`str.encode(encoding)`。リテラル構文（`b"..."`）は含めない | なし | 中 |
| **3-2** | 値の種類 `Ptr` を足す（D-3）: `Arc`・`Drop` で `release` を 1 回・DLL への参照・無効化の状態。網羅的な match の全箇所、等値とハッシュは同一性で | なし | 中 |
| **3-3** | 型検査: `Ptr[T]` を組み込みのジェネリック型として宣言（コンストラクタなし・`fixed_list[T]` と同じ置き方）。`to_list() -> list[T]`、`to_bytes() -> bytes`（要素が文字型のときだけ。それ以外は静的エラー）。ハンドルの引数に `mut` を要求しない。`Ptr[void]` → `Ptr[T]` の検査付き変換 | 3-2 | 中 |
| **4-1** | `PtrProtocol` を定める: 共有ヘッダ（`ar_ptr_protocol.h`）・型名（D-12）と flags（const・NUL 終端・STATIC）の表・解放の規約（D-15）・C# の `[StructLayout(Sequential)]` の写し・仕様の文書。`bool` は使わない（C# の `UnmanagedCallersOnly` と Arrow の C 互換レイアウトに載らない） | なし | 小 |
| **4-2** | ヘッダ解析が `PtrProtocol`（引数・戻り値・構造体のフィールド）を認識する | 4-1 | 小 |
| **4-3** | 生のポインタの既定の規則（D-7・D-8）を型スタブと実行時の両方に入れる: `char*` / `wchar_t*` の戻り値 → `Option[bytes]`、それ以外のポインタの戻り値 → `Option[Ptr[T]]`、`void*` の引数 → `Ptr`（今は `int`）、`char*` の引数 → `bytes`（D-13）、`None` → NULL。既存の例題（DxLib の実用例など）で影響する箇所を洗い出して移行する | 1-2・1-4・1-5・3-1・3-3 | 中 |
| **4-4** | typed ABI に `Ptr`・`bytes`・`PtrProtocol` の受け渡しを足す（4 つの呼び出し経路と `cpp_typed_eligible` / `build_cpp_typed_sig`）。`PtrProtocol*` の引数で `address` が 0 に書き戻されたら `Ptr` を無効にする | 4-2・4-3 | 中 |
| **4-5** | `PtrProtocol` の戻り値を `Option[Ptr[T]]` で受け取り、`to_list()` / `to_bytes()` で展開できるようにする（D-10）。`release` は `Ptr` の `Drop` / `release()` で 1 回だけ呼ぶ。STATIC の無い NULL の `release` はエラー（D-15） | 4-4 | 中 |
| **4-6** | 構造体を展開する: by-value の構造体は境界で（C 互換レイアウトのものは memcpy）、ポインタ型フィールドは D-7 / `PtrProtocol` の規則で、構造体の配列は `Ptr[構造体].to_list()` で。今の「コンストラクタにフィールドを 1 つずつ渡す」再構築をやめる | 1-3・4-5 | 中 |
| **4-7** | Arrow → C: `list` → `PtrProtocol` の引数（呼び出しの間だけ有効なコピーを渡す・D-15）、`bytes` → `char*`。`mut` の `list` と出力バッファは呼び出しの後に書き戻す（D-14） | 4-5 | 中 |
| **5-1** | cs-dll: C# 側の `PtrProtocol` で配列・`List`・`byte[]` を受け渡す（export の生成器が要る。今は手書き）。`IntPtr` / `nint` ↔ `Ptr`、`byte[]` ↔ `bytes` | 2-1・3-1・4-5 | 大 |
| **5-2** | cs-proc / js-proc: 値表現に dict / tuple / set / 配列 / `bytes` を足してコピーで運ぶ。`Ptr` を渡したらエラー | 2-1・2-3・3-1・3-2 | 中 |
| **5-3** | py-int: Python の `bytes` / `bytearray` / `memoryview` ↔ Arrow の `bytes` | 2-6・3-1 | 小 |
| **5-4** | rs: `Vec<u8>` → `bytes`（今は 16 進文字列）。`Vec<T>`・`Option<T>`・`&[T]`・`String` フィールドを `PtrProtocol` で受け渡す（ラッパーは生成した Rust なので素直に書ける） | 2-5・3-1・4-5 | 中 |
| **6-1** | 非同期タスクのインタープリタにも読み込んだ DLL を見せる（`native_libs`） | なし | 小 |
| **6-2** | cs-proc / cs-dll のブリッジの登録表をスレッドを越えて使えるようにする | なし | 小 |

⚠ フェーズ 1・2 で「エラーにする」としたものは、フェーズ 4・5 で順に正しく動くようになる。先に黙って
誤るのを止める（D-4）ために、エラーにする段を分けている。

## 4. ゲート

| タスク | 走らせるもの |
|---|---|
| 全タスク | `scan_examples` / `force_gate` / `compare_python_impl`（`test_build_gate` は前段で自動） |
| 外部言語の受け渡しを触るもの（1-2〜2-6・4-*・5-*） | `compare_import_paths -A`・`compare_outputs -A`。例題は**期待値と違えば `raise` する**形で書く（ゲートの多くは exit code しか見ない）。エラーにする変更はエラー例題（`_error`）を足す |
| 型スタブ・型検査を変えるもの（1-2・3-1・3-3・4-3・4-5） | `compare_wasm_frontend`・`type_obligations`。拡張の表示が変わるので VSIX を作り直す |
| 値の種類を足すもの（3-1・3-2） | `hash_eq_identity`（同一性で扱う型の一覧）・`stale_doc_refs`。新しい型の例題と、誤った使い方のエラー例題（`_error`）を足す |
| ファイルを足したもの | `generate-codebase-map` |

各タスクで関係する文書（skill `c-abi-interop`・`importation`、`docs/grammar/09_imports.md`）を更新する。
⚠ 例題ゲートは外部ライブラリが無い環境では失敗するので、1-1 のライブラリはビルド済みのものもリポジトリに置く
（`vec_math_x64.lib` と同じ扱い）。

## 5. 実施記録

着手順は 1-1 → 1-5 → 1-4 → 1-3 → 1-2 → 1-6。cpp-lib の生成物（`test_modules/ar_vec_math.dll` など）は
リポジトリに入っていて恒久キャッシュとして読まれるので、先に 1-5 を入れないとラッパー生成の変更
（1-2・1-3）が既存の例題に効かない。

### 1-1 実装

- C/C++: `examples/interop/test_modules/ffi_probe.{h,c,def}` と `build_ffi_probe.ps1`。同じソースから
  `ffi_probe.dll`（cpp-dll）と `ffi_probe_x64.lib`（cpp-lib）を作る。文字列・値渡し/値返しの構造体（8・12・16 バイト）・
  ポインタ型フィールドを持つ構造体・`void*`・中身を公開しない構造体のポインタ・配列を含む
  - ⚠ ヘッダとソースは ASCII だけにした。MSVC は既定で CP932 として読み、cpp-lib のシムもこのヘッダを
    include する。UTF-8 のコメントは次の行を食う（調査で実際に起きた）
  - DLL の export は `.def` で行う（ソースに `__declspec(dllexport)` を書くと静的ライブラリ側にも付き、
    シムの `/EXPORT:name=ar_shim_name` と衝突する）。import ライブラリは作らない（`/NOIMPLIB`）
- Rust: `examples/interop/rs_crates/ffi_probe_rs`（数値だけの構造体 `Counter`、`pub String` フィールドの
  `Person`、private フィールドと `new()` の `Account`、対応していないシグネチャ）。⚠ `Person` があるため
  2-5 の前はこの crate の import 自体が失敗する
- C#: `examples/interop/cs_probe`（`Probe.cs`・`Program.cs`・`build_cs_probe.ps1`）。パイプのホストは
  `cs_proc_test/ArrowPipeHost.cs` を `<Compile Include>` で共有する（2-2 の修正が両方に効く）。
  `bin/` `obj/` は追跡しない
- 確認: cpp-dll で `fp_greet()` / `fp_handle_new()` / `fp_db_open()` が `None`、cs-proc で `Probe.words()` が
  整数（`1`）になる（§0 の症状が再現する）。例題は直す各タスクで足す

### 1-5 実装

- `cpp_bridge/compiler.rs`: `.syms` の 1 行目に刻印（`cache_stamp`。シムと中継 DLL の**生成ソース**と、ビルドに効く設定の
  FNV-1a）を書き、恒久キャッシュは刻印が一致するときだけ使う。ヘッダ・ラッパー生成・設定のどれかが変われば作り直す。
  刻印の無い（1-5 より前の）`.syms` も作り直しになる。`.syms` の書き込み失敗を黙って捨てていた所はエラーにした
- ⚠ **発見**: 中継 DLL はシムを**絶対パス**で埋め込んでいた。コミット済みの `ar_vec_math.dll` は
  `D:\repository\new_language\...\ar_vec_math_shim.dll`（**別のチェックアウト**）を読んでいて、このリポジトリで
  シムを作り直しても使われず、別の場所に clone すると読み込めなかった。⇒ 生成ローダー（`codegen.rs` の
  `PLATFORM_LOADER`）が相対パスを中継 DLL 自身のディレクトリから引くようにし、シムはファイル名だけで指す
- `test_modules/ar_vec_math.*` を作り直した（刻印あり・絶対パスなし）。MSVC の設定の無いスクラッチ領域へ
  コピーしても（同じ `precompile_macros` を置けば）作り直さずに読めることを確かめた
- ⚠ 刻印は実装ライブラリ（`*_x64.lib`）の中身を含まない。ライブラリだけを作り直したら `ar_*` を消す
  （skill `importation` に追記）。DxLib の実用例（追跡外）は初回に作り直しになる（MSVC が要る）
- 単体テスト: `cpp_bridge::compiler::tests`（刻印が決定的・入力で変わる・`.syms` の読み取り）
- ⚠ **退行（1-3 で発見・修正）**: 相対パスを中継 DLL の場所から引くようにしたため、cpp-dll（中継 DLL は `%TEMP%`）で
  ヘッダを相対パスで import すると対象の DLL を読み込めず、中継 DLL の初期化が panic してプロセスが落ちていた。
  ゲートに cpp-dll の例題が 1 本も無く気づけなかった。⇒ 対象の DLL を絶対パスにして渡し、先に読み込めるかを確かめて
  普通のエラーにする（`exec/modules.rs` の cpp-dll の分岐）
- ゲート: `scan_examples`（基準と同じ: `bench_ab_native.ar` のタイムアウトのみ）・`force_gate`（`VmForceError` 0）・
  `compare_python_impl`・`compare_import_paths -A`・`compare_outputs -A`（基準 `7ffafa7`）

### 1-4 実装

- `native_api/callbacks.rs`: ネイティブ側から受け取る `(ptr, len)` を `native_str` で検証してから `str` にする。
  不正な UTF-8・負の長さ・長さのある NULL は呼び出しのエラー（`NativeError: … is not valid UTF-8`）にする
  （以前は検証せずに `str` にしていて未定義動作）
- `ar_make_str` に加え、名前を受け取るコールバック 7 か所（属性名・グローバル名・型名・メソッド名・フィールド名）も
  同じ検査を通す（生成コードの固定の名前なので実害は出にくいが、同じ未定義動作の形だった）
- 単体テスト: `native_api::callbacks::tests`（正しい UTF-8・日本語・長さ 0 の NULL は受け取る、CP932・負の長さ・
  NULL はエラー、名前のコールバックもエラー）。今の Arrow の例題からは不正なバイト列を渡す手段が無いため、例題は無い

### 1-3 実装

- 値渡し・値返しの構造体を、シムも中継 DLL も**値のまま**受け渡す。中継 DLL は C と同じレイアウトの
  `_Struct_{name}` を値で使い（呼び出し規約は Rust の `extern "C"` が合わせる）、cpp-lib のシムは実際の型で
  受け取り・返す（`c_type_str` が構造体の型名を返す）。シムの値返しの静的バッファ（スレッド安全でない）は無くした
- 実測で分かった以前の症状: cpp-dll は値返し（8・12・16 バイトすべて）で segfault。cpp-lib は **8 バイトの値渡しで
  segfault**（シムはアドレスで受け取るのに typed 経路は値で渡していた。12・16 バイトは x64 で隠しポインタになるので
  偶然動いていた）
- 値で受け渡せるのは C と同じレイアウトが分かる構造体（`CStructDef::raw_layout`）だけ。それ以外を値で受け渡す関数は
  生成しない（呼べばその関数が無いというエラー。静的エラーにするのは 1-2）
- ⚠ **発見**: ヘッダ解析は `char` / `short` / `int8_t` / `uint8_t` / `int16_t` / `uint16_t` / `wchar_t` も
  `CType::Int`（4 バイト）にまとめていて、`unsigned char` × 4 の構造体（DxLib の `COLOR_U8` など）を 16 バイトとして
  読み書きしていた（ゼロコピー渡しも含む）。⇒ `is_narrow_int_type` で見分け、そうしたフィールドを持つ構造体を
  不完全（`complete = false`）とみなす。引数・戻り値の整数の幅と符号は 1-6（⇒ 1-6 で幅を区別し、この判定は外した）
- 検査用ライブラリに `Rgba8`（`unsigned char` × 4）と `fp_make_rgba` / `fp_rgba_sum` を足した
- 例題: `cpp_dll_struct_by_value.ar`・`cpp_lib_struct_by_value.ar`（8・12・16 バイトを最上位・関数の中・ループの中から。
  期待値と違えば raise）・`cpp_struct_by_value_layout_error.ar`（負例）。`test_modules/ar_ffi_probe.*` を追加
- 単体テスト: `header_parser::tests`（4 バイト未満の整数のフィールド・typedef で解決した幅・ポインタは対象外）、
  `cpp_bridge::codegen::tests`（値のまま受け渡す・レイアウト不明なら生成しない）、`compiler::shim_tests`
- この例題で 1-5 の退行（cpp-dll の相対パス）が見つかり、別のコミットで直した（1-5 の記録）
- ゲート（基準 `c28b4c9`）: `scan_examples`・`force_gate` は基準と同じ（新しい例題も通る）。`compare_python_impl` は
  新しい 3 本を既知の差分に登録した（参照実装は値渡しの構造体を扱わない）。`compare_import_paths` 13/13。
  `compare_outputs` の差は新しい 3 本だけ（基準では落ちる・期待どおり）。負の対照で `cpp_lib_struct_by_value.ar` が
  ずれたのは、新旧で刻印が違い、交互に走らせると生成物の作り直しの表示が回によって変わるため。
  `compare_wasm_frontend` 492/492・`type_obligations` 退行なし

### 1-2 実装

- **判定を 1 か所に置いた**: `CFnSig::unsupported_reason`（`src/cpp_header/types.rs`）が、ヘッダから分かる時点で
  「今の Arrow から正しく呼べない」関数を理由つきで判定する。型スタブ（パーサ）と中継 DLL（実行時）の両方が使う。
  対象: ポインタを返す（`char*`・`wchar_t*`・`T*`・`void*`・構造体ポインタ・関数ポインタ）、`void*` / 関数ポインタ /
  `wchar_t*` / 非 const の `char*`（出力バッファ）/ `const T*`（数値。値 1 個か配列か分からない）を受ける、
  レイアウトの分からない構造体を受け渡す、構造体ポインタと typed 経路に載らない型（`const char*`・`bool`）が混ざる
- **静的エラー**: パーサは未対応の関数に理由を書いた文字列のデコレータ（`FFI_UNSUPPORTED_MARK`）を付け、`mut` も
  要求しない。型検査は `import` / `from … import` の時点でそれを集め、使った箇所を `TypeErrorKind::FfiUnsupported`
  （`C function 'fp.fp_greet' cannot be used from Arrow yet: it returns a C string`）にする。拡張も同じ wasm で出す
- **実行時**: 未対応の関数は中継 DLL にもシムにも入れない（`gen_dll_fn` が空を返す・`compile_tl_dll` が除く）。
  typed 経路の適格条件を `CFnSig::typed_abi_eligible` に移した（`cpp_typed_eligible` はこれに委ねる）
- **ハンドル経路へ落とさない**（c-abi-interop P7 の残課題）: `TypedArgs::marshal` が、構造体ポインタへのインスタンス以外・
  数値への書き込みポインタへの数値以外を `Err` にする（数値どうしの食い違いは従来どおりハンドル経路で変換）
- **`const char*` の引数**（`ar_to_cstr`）: str 以外・NUL を含む str は、エラーを設定して NULL を返す。中継コードは NULL
  なら C 関数を呼ばずに戻り、呼び出しの後でエラーが例外になる
- `CType::CharPtr { wide, mutable }`（`wchar_t*` と非 const の `char*` を区別。以前は同一視）
- 実測した以前の症状: `fp_rec_score(5)`（構造体ポインタに整数）は segfault、`fp_len("a\0b")` は黙って 1
- 例題: `cpp_unsupported_signature_error.ar`（8 種類の未対応がそれぞれ静的エラー）・`cpp_struct_ptr_arg_error.ar`・
  `cpp_cstring_nul_error.ar`（実行時エラー）・`cpp_cstring_arg.ar`（`const char*` の正例・UTF-8）。
  1-3 の `cpp_struct_by_value_layout_error.ar` も静的エラーになった。`ar_ffi_probe.*` を作り直した（未対応を除いた）
- 単体テスト: `cpp_header::types::tests`（呼べる・呼べない・正確なレイアウト）、`value::native::marshal_tests`、
  `native_api::callbacks::tests`（`to_cstr`）
- VSIX を作り直した（拡張の診断が増えるため）
- ⚠ 残したもの: ハンドル経路の数値の変換（`int` の引数に str が来ると 0）。省略された引数を 0 で埋める仕組み
  （`DEFAULTPARAM`）がこの変換に依存しており、静的な型でほぼ防がれるので触っていない。`bool` の幅は 1-6（⇒ 1 バイトにした）
- ゲート（基準 `0c6c54e`）: `scan_examples`・`force_gate` は基準と同じ（新しい例題も期待どおり）。`compare_python_impl` は
  負例 2 本を既知の差分に登録した（参照実装は直す前と同じ壊れ方: NUL で黙って切れる・アクセス違反）。
  `compare_import_paths` 13/13。`compare_outputs` の差 5 本はすべて意図したもの（新しい負例 3 本・1-3 の負例が静的
  エラーになった・`cpp_lib_struct_by_value.ar` は 1-3 と同じく生成物の作り直しの表示）。`const char*` の正例は基準と一致。
  `compare_wasm_frontend` 496/496（新しい静的エラーも拡張と一致）・`type_obligations` 退行なし

### 1-6 実装

- `CType::Int { width: IntWidth, signed }`（`src/cpp_header/types.rs`）。`IntWidth` は `W8`〜`W64` と、環境で幅の
  変わる `CLong`（Windows 4 バイト）・`WChar`。ヘッダの型名からの対応は `decls.rs::c_int_type`、中継 DLL の Rust の型は
  `int_rust_type`、`RawLayout` の注釈は `int_raw_ann` の各 1 か所。`CType::Long` は無くした。`bool` は 1 バイト（`u8`）
- 受け取るときは C の幅と符号どおりに広げる。`unsigned long long` の 2^63 以上は `OverflowError`（戻り値・値返しの
  構造体のフィールド・書き込みポインタの書き戻しの全部。typed 経路は `_ErrSlot` と `decode_out_ptr`）。渡すときは
  C の幅に切り詰める（C の暗黙の変換と同じ）。書き込みポインタも C の幅で書く
- 1-3 の `is_narrow_int_type`（4 バイト未満の整数のフィールドを持つ構造体を不完全とみなす）は外した。`unsigned char` × 4
  の `Rgba8` は値で受け渡せるようになった。`signed char*` / `unsigned char*` は文字列ではなく数値のポインタにした
- 実測した以前の症状（`2090589`・cpp-dll）: `unsigned int` 4294967295 → -1、`long` -5 → 4294967291、
  `unsigned char` の (200 + 100) mod 256 = 44 → 300・`signed char` -5 → 251（レジスタの上位のごみまで読んでいた）、
  `unsigned long long` の 2^64-1 → -1（戻り値も書き込みポインタも）。`short` / `unsigned short` は MSVC が 4 バイトへ
  広げて返すので偶然合っていた。`long long` は以前から 8 バイトで正しい
- ⚠ 途中で見つけて直した: 値返しの構造体のフィールドや書き込みポインタの書き戻しで変換が例外を返したとき、生成コードが
  例外のハンドルを値として使い、呼び出しは正常に終わって例外が次の呼び出しまで残る形だった。⇒ 生成コードが
  `TL_EXCEPTION` と比べてすぐ返す（`CType::to_handle_may_raise`）
- 検査用ライブラリに整数の幅の関数 16 本と `Mat2`（配列のフィールド）・`U64Box`（`unsigned long long` のフィールド）を
  足した。ヘッダに残っていた非 ASCII のコメント（1-1 から）を ASCII にした
- 例題: `cpp_dll_int_widths.ar`・`cpp_lib_int_widths.ar`（期待値と違えば raise）・`cpp_u64_overflow_error.ar`（負例）。
  `cpp_{dll,lib}_struct_by_value.ar` に `Rgba8` を足し、`cpp_struct_by_value_layout_error.ar` は `Mat2` に替えた
- 単体テスト: `header_parser::tests`（狭い整数・`long` の幅）、`cpp_bridge::codegen::int_width_tests`（幅と符号・`long`・
  `bool`・2^63 以上の例外・文言の一致）、`value::native::tests`（書き込みポインタの読み出し）
- `docs/grammar/09_imports.md` に C の値の対応を書いた。VSIX を作り直した（`unsigned char*` の型が変わるため）
- ⚠ 残したもの: 渡すときの範囲外を静的に弾くこと。C ABI 型 `uint64` の**フィールド**（raw ブロック）が 2^63 以上を
  負として読むこと（P1 の格納型の規約。構造体ポインタのゼロコピー渡しで C が書いた値も同じ・skill `c-abi-interop`）
- ゲート（基準 `2090589`）: `scan_examples`・`force_gate`（`VmForceError` 0）は基準と同じ。`compare_python_impl` は新しい
  3 本を既知の差分に登録した（参照実装は直す前の Rust と同じく `unsigned int` の最大値を -1・2^64-1 を -1 として読む）。
  `compare_outputs` の差 6 本のうち 5 本は意図したもの（新しい 3 本と、`Rgba8` を足した値渡しの 2 本は基準では静的エラー）。
  残る `cpp_default_arg_native_call.ar` と `compare_import_paths` の `cpp_struct_ptr.ar`（12/13）は、負の対照（基準どうし）でも
  ずれる生成物の作り直しの表示で、`Cpp` の行を除いた出力は基準と一致した（1-3 と同じ）。`compare_wasm_frontend` 499/499・
  `type_obligations` 退行なし・`cargo test` 1077 件すべて成功
- `stale_doc_refs` で、1-2 で消した `by_value_ok` を指したままのコメント 2 か所（`codegen.rs`）が見つかり、今の判定（`CFnSig::unsupported_reason`）に直した

### 2-1 実装

- **型スタブ**（`parser/cs_assembly/`）:
  - C# の nullable 注釈（`NullableAttribute` / `NullableContextAttribute`）を読み、`string?` / `Person?` を `Option[str]` /
    `Option[Person]` にした（`parse.rs::read_nullable`・`NullableInfo`）。⚠ メタデータのシグネチャには `string` と `string?` の
    区別が無く、読まないと null が `str` の値に `None` として黙って入る。`str | None` の戻り値を正しく扱うには、これが要った
  - 受け渡せない型（コレクション・タプル・デリゲート・型変数・他のアセンブリの型・インターフェース）を使うメソッドに印を
    付け（`stub_gen.rs::cs_unsupported_reason`）、型検査が使った箇所を `FfiUnsupported` にする（クラスのメソッドへ広げた:
    `TypeRegistry::ffi_unsupported_method`・`call_check.rs`）。⚠ 印は 1-2 のデコレータではなく**メソッドの本体の先頭の
    文字列**: C# のスタブはクラスとして実行され、メソッドのデコレータが適用される
  - `HasCustomAttribute` の表の並びを規格どおりにした（以前は幅の計算にしか使っておらず、表が重複・欠落していた）
- **実行時**（`interpreter/cs_values.rs` を cs-proc / cs-dll で共有）: 戻り値は型スタブの宣言で読む。クラスのハンドルは
  `CsObject` に包む（ブリッジごとにクラスを控える）。null は `Option[T]` / `None` / `Any` のときだけ `None`、それ以外は
  TypeError。`object` の値でクラスの分からないオブジェクトは TypeError。運べない引数・別のブリッジの `CsObject` はエラー。
  例外の種類が付いたエラーは `CsProc:` を付けず、`except` で捕まえられるようにした（`tag_error`）
  - cs-proc: 宣言とタグの食い違い・壊れた応答はエラー（以前は `""` / `0`）。`ulong` の 2^63 以上は OverflowError（タグ `o`）
  - cs-dll: 引数を**宣言された型で**並べる（`Option[str]` の `None` は `(NULL, -1)`）。以前は値の種類で並べ、`string?` に
    `None` を渡すと後ろの引数が 1 つずつずれた。8 スロットを超えたら黙って捨てずにエラー。文字列の戻り値は UTF-8 を検査
    （以前は `from_utf8_lossy`）。`object` と値型の nullable の戻り値は取り決めが無いので呼ぶ前にエラー
- **C# ホスト**（`ArrowPipeHost.cs`・cs_probe と cs_proc_test が共有）: `char` / `byte` / `short` / `ushort` / `sbyte` /
  `ulong` の戻り値をハンドルにせず値で返す。`long?` などの引数を受ける（`Convert.ChangeType` は `Nullable<T>` を扱えない）。
  `char` の引数に長さ 1 の文字列。タグの無い引数・表に無いハンドルはエラー（以前は黙って `null`）
- `x is Person` / `mustbe Person` が C# のオブジェクトで真にならなかった（`value_is_type` にアームが無かった）のを直した。
  `Option[Person]` を絞り込むのに要る
- 実測した以前の症状（`9fccf26`）: `make_person("bob")` → `1`、`words("a b")` → `2`（どちらもハンドル番号）、`string?` の null と
  約束違反の null → `str` の変数に `None`
- 検査用: `cs_probe/Probe.cs` に nullable・小さな値型・`object`・未対応のシグネチャのメンバーを足した。cs-dll は
  `cs_interop_test` に `Clone()`（オブジェクトの戻り値）・`FindWord()`（`string?`）と export を足し、NativeAOT で作り直した。
  `build_bridge.ps1` が cargo の進捗表示（stderr）で必ず失敗していたのを直した
- 例題: `cs_proc_values.ar`（期待値と違えば raise）・`cs_proc_unsupported_signature_error.ar`（8 か所の静的エラー）・
  `cs_proc_null_contract_error.ar`。`cs_interop_test.ar` に cs-dll の確認を足した
- 単体テスト: `cs_assembly::parse::probe_tests`（実際の `cs_probe.dll` で nullable と未対応の印）、`cs_values::tests`
- 文書: `docs/grammar/09_imports.md` に「C# の値の対応」、skill `importation` に C# の節。VSIX を作り直した
- ⚠ 残したもの: nullable 注釈の無い（古い）アセンブリの null は TypeError で、逃げ道が無い（Kotlin のプラットフォーム型のような
  扱いは必要になったら検討）。外部言語のスタブのクラスは型検査では素の名前（`Person`）で、`cp.Person` は型の位置で
  解決されない（既存・`is cp.Person` は静的エラー）。C# の例外は `CsProc: …` のままで `except` で捕まえられない（既存）
- ゲート（基準 `9fccf26`）: `scan_examples`・`force_gate`（`VmForceError` 0）は基準と同じ。`compare_python_impl` は新しい 2 本を
  既知の差分に登録した（参照実装は `cs_probe/cs_probe.dll` を見つけられず ParseError。静的エラーの例は一致）。
  `compare_outputs` の差 4 本はすべて意図したもの（新しい 3 本と、確認を足した `cs_interop_test.ar`。基準では `string?` の
  引数に `None` を渡すと静的エラー）。負の対照は 415/415。`compare_import_paths` の差は同じ `cs_interop_test.ar` だけ。
  `compare_wasm_frontend` 502/502（拡張のクレートのテストも、例題の DLL をリポジトリの直下から探すよう直して通過）・
  `type_obligations` 退行なし・`stale_doc_refs` 0・`cargo test` 1082 件すべて成功


### 2-2 実装

- `ArrowPipeHost.cs` の `ReadPipeLine` が受け取ったバイトを 1 つずつ文字にしていた（`(char)b`）のを、1 行のバイト列を
  まとめて UTF-8 として復号するようにした（不正なバイト列は例外）。送る向き（`WritePipeLine`）は以前から UTF-8
- 実測した以前の症状（`4c39493`）: `echo("日本語")` → 18 文字の化けた文字列、`byte_len("日本語")` → 18（正しくは 9）
- ホストは cs_probe と cs_proc_test が共有する（1-1）ので、両方を作り直した
- 例題: `cs_proc_utf8.ar`（日本語・アクセント・絵文字・空文字列の往復、C# 側のバイト数、オブジェクトに入れた名前。
  期待値と違えば raise）。Arrow の文字列は常に正しい UTF-8 なので、エラーの例題は無い
- ⚠ 見つけたもの（範囲外・既存）: Arrow の `len(str)` は UTF-8 のバイト数を返す（`len("日本語")` は 9）が、添字と反復は
  文字単位（参照実装の `len` は 3）。外部言語とは関係しない言語の意味論の問題
- ゲート（基準 `4c39493`）: `scan_examples`・`force_gate` は基準と同じ。`compare_python_impl` は新しい 1 本を既知の差分に登録して
  clean。`compare_outputs` 416/416・`compare_import_paths` 13/13（Rust は変えていないので両方の exe が直したホストを使う。
  直す前の値は上の実測）


### 2-3 実装

- **境界の照合**（`ffi_boundary::JavaScriptChecker`）: 「数値は常に Float で届く」前提で書かれていたが、ブリッジは整数値を
  `"i"` で送っていた。実際の表現に合わせて書き直した（`js_coerce`）: `float` 宣言に整数値 → `Float`（JS の値はもともと f64
  なので正確）、`int` 宣言に整数値の `Float` → `Int`（int64 に入るときだけ）、`list[float]` / `list[list[int]]` /
  `Option[float]` は要素・選択肢へ再帰。⚠ 以前は `-> float` の関数が `2.0` を返すと FfiTypeError になっていた
- **コンテナの戻り値の照合**: js-proc の型スタブ（`.ars`・Arrow の構文）の戻り値を Arrow の型として読むようにした
  （`collect_module_types` に言語を渡す）。⚠ 以前はプリミティブしか読まず、`-> list[int]` の関数の型は「分からない」になり、
  静的な検査も境界の照合も効かなかった（`[1, "two"]` が `list[int]` の変数に入った）。Python のスタブは綴りが Arrow の型と
  限らないので従来どおり
- **運べない値はエラー**（`js_proc_runtime.rs`）: dict / tuple / set / インスタンスの引数は TypeError（以前は JS に `null`）。
  オブジェクトの戻り値（`"o"`）・関数 / Symbol / BigInt（新しいタグ `"x"`）は TypeError（以前は `"k=v"` の文字列のリスト・
  文字列）。読めない値・知らないタグはエラー（以前は `""` / `0` / 整数）。TypeError は `except` で捕まえられる
- **ブリッジ**（`bridge/js_bridge.cjs`）: 整数は安全な整数（2^53 未満）だけ `"i"`、それ以外は `"f"`（`JSON.stringify` は
  2^53 以上で最短の往復表現を出し、double の持つ整数と違う桁になる）。NaN / ±Infinity は文字列で送る（以前は JSON の
  `null` → 0.0）。知らない引数のタグはエラー。受信を `StringDecoder` で復号する（チャンクの境目で UTF-8 が割れると壊れていた）
- 実測した以前の症状（`81b5688`）: `whole_float()` → FfiTypeError、`floats()`（`list[float]`）→ `[1, 2.5]`、
  `mixed()`（`list[int]`）→ `[1, 'two']`、`1e20`（`int`）→ 0、NaN / -Infinity → 0.0、オブジェクト → `['a=1', 'b=x']`、関数 →
  文字列、`type_of({"a": 1})` / tuple / set → `"null"`
- 検査用: `examples/interop/js_probe.js` / `js_probe.ars`（1-1 では作っていなかった）
- 例題: `js_proc_values.ar`（期待値と違えば raise。運べない値 6 種を `except TypeError` で捕まえる）・
  `js_proc_container_return_error.ar`（FfiTypeError）・`js_proc_dict_arg_error.ar`（TypeError）
- 単体テスト: `ffi_boundary::tests`（int → float・要素への再帰・`Option[float]`・2^63 以上と NaN の拒否）
- 文書: `docs/grammar/09_imports.md` の js-proc の型タグ、skill `importation` の js-proc の型エンコーディング
- ⚠ 残したもの: FfiTypeError は例外クラスとして登録されておらず `except` で捕まえられない（既存）。node のプロセスが
  実行ごとに残る（2-4）
- ⚠ **ゲートが 7 時間止まった**: `compare_python_impl.ps1` が子プロセスの出力をパイプで読んでいて、新しい js-proc の例題が
  残した node（2-4）がパイプを握ったまま `.Result` が返らなかった。出力をファイルへ落とすよう直し（`compare_outputs.ps1` と
  同じ・skill `vm-pitfalls` §4）、制限時間を超えたら子プロセスごと止める。`scan_examples.ps1` の stderr の待ちも 10 秒までにした。
  新しい js-proc の例題は既存の js-proc の例題と同じく参照実装との比較から外した（外部プロセスに依存）
- ゲート（基準 `81b5688`・各ゲートに制限時間を付けた実行）: `scan_examples`・`force_gate`（`VmForceError` 0）は基準と同じ。
  `compare_python_impl` は clean（170/170。新しい 3 本は外部プロセス依存として比較から外した）。`compare_outputs` の差 3 本は
  新しい例題（基準では `whole_float()` の FfiTypeError などで違う）、負の対照 419/419。`compare_import_paths` 13/13。
  `compare_wasm_frontend` 506/506・`type_obligations` 退行なし・`stale_doc_refs` 0・`cargo test` 1083 件すべて成功


### 2-4 実装

- 原因: Arrow はブリッジを静的な表（`js_proc_runtime::global_bridges`）に持っていて、終了時に破棄されないので `PipeConn` の
  `Drop`（`quit` を送る）が走らない。ブリッジ（`bridge/js_bridge.cjs`）はパイプのサーバーを開いたまま待ち続けていた
- ブリッジが接続の `close` で終了するようにした（Arrow が異常終了・強制終了しても OS がパイプを閉じるので終わる）。Arrow が
  接続しないまま死んだときに備え、60 秒接続が無ければ終了する
- 実測: 以前は js-proc の例題 1 回ごとに node が 1 つ残った（2-3 のゲートの途中で 41・その前に 129 を手で止めた）。直した後は
  5 本の例題のどれの後も 0。実行中の `arrow.exe` だけを強制終了しても node は終わる
- ⚠ 残った node は親の stderr を握り続け、出力をパイプで読むゲート（`compare_python_impl.ps1`）が永久に待った（2-3 の記録）
- 例題は足していない（値の受け渡しは変わらない）。ゲートを走らせる手元のスクリプトが、各ゲートの後に残った node を数えて警告する
- ゲート（基準 `9f0aedb`・制限時間つき）: `scan_examples`・`force_gate`（`VmForceError` 0）は基準と同じ。`compare_python_impl`
  clean・`compare_outputs` 419/419（負の対照も 419/419）・`compare_import_paths` 13/13。どのゲートの後にも node は残らなかった

---

## 付録 A. 検査の記録（2026-10-12・`2aa53ed`）

各言語の検査用ライブラリを作り、実際に呼んで確かめた（cs-dll は既存の `examples/interop/cs_interop_test`）。

| 言語 | str を受け取る | str を渡す | コレクション | クラス・構造体 |
|---|---|---|---|---|
| cpp-dll | ❌ `None` | ✅ `char*`（UTF-8）／❌ `wchar_t*`・出力バッファ | ❌ 配列は渡せない | ❌ by-value の戻り値で segfault、ポインタの戻り値は `None` |
| cpp-lib | ❌ `None` | 同上（`wchar_t*` の関数は黙って消える） | ❌ | ⚠ by-value の戻り値は動くが、文字列フィールドは `None` |
| rs | ✅（UTF-8 も） | ✅ | ❌ `Vec<String>` などの関数は消える。`Vec<u8>` は 16 進文字列 | ⚠ 戻り値は動く。`pub String` / private フィールドで import 失敗。フィールド値が古い |
| py-int | ✅ | ✅ | ✅ list / dict / tuple、❌ set | ✅ Python → Arrow（属性・メソッド可）、❌ Arrow → Python |
| py | ✅ | ✅ | ✅ | ✅（同じインタープリタ内） |
| js-proc | ✅ | ✅ | ⚠ 配列のみ。オブジェクトは `"k=v"` | ❌ `"k=v"` |
| cs-proc | ✅（`null` → `None`） | ❌ 非 ASCII が化ける | ❌ 整数ハンドル | ⚠ コンストラクタで作ったものだけ使える。戻り値は整数 |
| cs-dll | ✅（UTF-8 も） | ✅ | ❌ スタブにはあるがブリッジに export が無い | 未検証（コード上は整数） |

- 文字列フィールドを持つ構造体を C へ渡す（cpp-dll・cpp-lib の両方）と、cpp-dll で by-value の構造体を
  返す関数を呼ぶと、exit 139（segfault）。
- 静的型 `str` の変数に `None` が入り、`g.upper()` で初めて `AttributeError` になる。

## 付録 B. 実測

**配列の直前のメモリ**（MSVC x64、release `/MD` と debug `/MDd`）

| 確保の方法 | 直前 8 バイト | 長さか |
|---|---|---|
| `new WithDtor[5]` / `[7]`（デストラクタあり） | `5` / `7` | ✅ 要素数（配列クッキー） |
| `new int[5]`・`new Trivial[5]`・`malloc`・`vector::data()` | release: 符号化されたヒープヘッダ、debug: `0xfdfdfdfd` + 通し番号 | ❌ |
| 静的配列・スタック上の配列・配列の途中 | 別の変数・戻りアドレス・手前の要素 | ❌ |
| `BSTR`（`L"hello"`） | `10`（バイト数） | ✅ 仕様 |

**ポインタだけから分かること**（DLL が返したポインタ、`/MD`・`/MT` で同じ結果）

| DLL での作り方 | 判別 |
|---|---|
| 文字列リテラル・`static const` の表 | DLL イメージの `.rdata`（読み取り専用） |
| `static` の配列 | DLL イメージの `.data` |
| `malloc` / `new int[]` | プロセスヒープ・同じサイズ（区別できない） |
| `malloc(256)` に `"hi"` | ヒープのサイズは 256（中身の長さではない） |
| `HeapCreate` した独自ヒープ | 別のヒープ |
| デストラクタ付き `new[]`・配列の途中・`VirtualAlloc` | 判定不能 |
| 関数内の配列 | スタック（関数から戻った時点で無効） |

**`std::vector<int>` / `std::string` を生のメモリから読む**（release は 24 / 32 バイト、debug は 32 / 40 バイト）

| DLL | 仮定したレイアウト | vector | string |
|---|---|---|---|
| release | release | ✅ 長さ 5・容量 10（ヒープ照合も一致） | ✅ |
| debug | release | 照合で弾けた | ❌ 長さ 0 の `""` として素通り |
| debug | debug | ✅ | ✅ |

**展開のコスト**（`Value` と同じ 32 バイトの enum を真似た Rust のベンチ。生成と破棄を含む。1 回の計測）

| C から受け取るもの | 時間 |
|---|---|
| 文字列 16〜256 バイト / 64 KB（ASCII） | 42〜65 ns / 7.4 µs（0.11 ns/バイト） |
| 文字列（日本語） | 0.64 ns/バイト |
| int32 / double の配列（要素数 4K まで / 64K〜1M） | 約 70 ns + 1.3 ns/要素 / 6.4〜7.1 ns/要素 |
| 参考: int32 配列の memcpy | 0.07 ns/要素 |
| 数値だけの構造体 / `int` と `char*` の構造体 → インスタンス | 66 ns / 151 ns |
| 構造体の配列 | 81〜99 ns/要素 |
| 逆方向: `list[int]` → int32 配列 / `str` → `char*` | 0.5〜0.65 ns/要素 / 38〜73 ns |

比較の基準（実際の Arrow）: C 関数 1 回（`bench_ab_cdll.ar` の `v3_add`）約 300 ns、`for x in xs` 40 ns/要素、
`xs[i]` のループ 164 ns/要素、Arrow で `P3(1.0, 2.0, 3.0)` を作る 1.2 µs。
メモリ（概算）: `list` の要素は 1 個 32 バイトなので、uint8 は 32 倍・int32 は 8 倍・double は 4 倍、
構造体の配列は約 10 倍に膨らむ。
