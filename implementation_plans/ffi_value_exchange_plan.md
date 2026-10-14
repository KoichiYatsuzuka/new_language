# 外部言語との値の受け渡しを作り直す（文字列・配列・構造体・ポインタ）

`import[cpp-dll]` のライブラリから `str` が返ってこない、という報告を受けて、全ブリッジ（C/C++・Rust・
Python・JS・C#）の値の受け渡しを検査した。文字列・コレクション・インスタンスの多くが、**型検査を通った
あとで黙って別の値になる**（`None`・`null`・整数）か、**プロセスごと落ちる**ことが分かった。

本書は、ポインタの受け渡しに共通のプロトコル（`PtrProtocol`）を定め、Arrow 側では文字列（バイト列）・配列・
構造体を展開してコピーした Arrow の値として扱い、ポインタのまま扱うものはポインタクラス `Ptr[T]` にする、
という形に作り直す計画書である。

- 起票: 2026-10-12（利用者の報告「cpp-dll から str が返らない」から）
- 調査: Arrow `2aa53ed`（release ビルド）
- 状態: フェーズ 1〜4 完了（1-1〜1-6・2-1〜2-6・3-1〜3-3・4-1〜4-7・§5）・7-12・7-15 完了。フェーズ 5 着手（5-1）
- 改訂: 2026-10-14 C# / Rust の配列の受け取り方を利用者の判断で決定（D-16）。5-1 を export の生成器（5-1）と値の受け渡し（5-5）に分け、生成器は Arrow の CLI で作る（利用者の判断）
- 改訂: 2026-10-13 フェーズ 1・2 の実装中に見つけた、値の受け渡しの外の問題をフェーズ 7 に起票した（利用者の指示）
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

- **D-16**（2026-10-14・利用者の判断）: C# / Rust の配列・コレクションの受け取り方は、**メモリ上のイメージで決める**。
  C/C++ と同じく先頭アドレスと長さで表せるものは `Ptr[T]`（D-10 と同じく、使う側が `to_list()` で展開）、
  先頭アドレスと長さだけでは表せないものは境界でその都度 `list` に展開する。`byte[]` / `Vec<u8>` は D-9 のとおり
  `bytes`。実測は付録 B（C# / Rust のコレクション）:
  | 型 | メモリ上のイメージ | Arrow |
  |---|---|---|
  | C# の数値の `T[]` | GC ヒープのオブジェクト（型ポインタ・長さの後に要素が連続）。固定しないと GC が動かす | `Option[Ptr[T]]` |
  | C# の `List<T>` | 内部の配列 `_items` と要素数 `_size` を持つ別のオブジェクト。増えると配列を作り直す | `list[T]` |
  | C# の `string[]`・クラスの配列 | 要素は参照（GC が動かすオブジェクトへのポインタ） | `list[T]` |
  | Rust の数値の `Vec<T>` | (ポインタ, 長さ, 容量)。要素はヒープに連続し、`Vec` を動かしてもバッファは動かない | `Option[Ptr[T]]` |
  | Rust の `Vec<String>` など | 要素がそれぞれ (ポインタ, 容量, 長さ) を持つ | `list[T]` |
  - C# は、C# 側が持ち続ける配列（フィールドなど）を返すことがあるので、ブリッジがアンマネージドのメモリへ
    複製して `PtrProtocol` で渡す（`release` で解放）。共有したまま C# が書き換えると Arrow の `Ptr` の中身が
    変わる（D-11 に反する）
  - 別プロセスのブリッジ（cs-proc・js-proc）は D-5 のとおりコピーで運ぶので、どれも `list` / `bytes`
  - 引数は `Ptr[T]`・`list`・（`byte[]` / `Vec<u8>` なら）`bytes` を受ける（呼び出しの間だけのコピー・D-15）

### 未決事項

なし（2026-10-12 時点）。

## 3. 実装予定

- フェーズ 1: C/C++ の既存の不具合を止める（黙って誤る・落ちる → エラー。プロトコルと独立）
- フェーズ 2: 他言語ブリッジの既存の不具合を止める（同上）
- フェーズ 3: 新しい値の型（`bytes`・`Ptr[T]`）
- フェーズ 4: `PtrProtocol` の定義と、境界での変換（C/C++）
- フェーズ 5: 他言語への波及
- フェーズ 6: 非同期タスクからの外部関数呼び出し（別タスク・D-6）
- フェーズ 7: フェーズ 1・2 の実装中に見つけた、値の受け渡しの外の問題（2026-10-13 起票・どれも独立に着手できる）

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
| **2-5** | ✅ rs: 対応していないシグネチャを import 時に報告する。`pub String` フィールドのセッター、`new()` を使う構築、ネイティブ側の書き換え後のフィールド値を直す | 1-1 | 中 |
| **2-6** | ✅ py-int: Python の set を Arrow の set に、Arrow の set を Python の set に変換する。変換できない値のエラーに型名を出す | なし | 小 |
| **3-1** | ✅ 組み込み型 `bytes` を足す（D-9・§5）: 値の種類・型検査・`len` / 添字 / スライス / 反復 / 比較・`decode(encoding)`・`str.encode(encoding)`。リテラル構文（`b"..."`）は含めない | なし | 中 |
| **3-2** | ✅ 値の種類 `Ptr` を足す（D-3・§5）: `Arc`・`Drop` で `release` を 1 回・DLL への参照・無効化の状態。網羅的な match の全箇所、等値とハッシュは同一性で | なし | 中 |
| **3-3** | ✅ 型検査（§5）: `Ptr[T]` を組み込みのジェネリック型として宣言（コンストラクタなし・`fixed_list[T]` と同じ置き方）。`to_list() -> list[T]`、`to_bytes() -> bytes`（要素が文字型のときだけ。それ以外は静的エラー）。ハンドルの引数に `mut` を要求しない。`Ptr[void]` → `Ptr[T]` の検査付き変換 | 3-2 | 中 |
| **4-1** | ✅（§5）`PtrProtocol` を定める: 共有ヘッダ（`ar_ptr_protocol.h`）・型名（D-12）と flags（const・NUL 終端・STATIC）の表・解放の規約（D-15）・C# の `[StructLayout(Sequential)]` の写し・仕様の文書。`bool` は使わない（C# の `UnmanagedCallersOnly` と Arrow の C 互換レイアウトに載らない） | なし | 小 |
| **4-2** | ✅（§5）ヘッダ解析が `PtrProtocol`（引数・戻り値・構造体のフィールド）を認識する | 4-1 | 小 |
| **4-3** | ✅（§5）生のポインタの既定の規則（D-7・D-8）を型スタブと実行時の両方に入れる: `char*` / `wchar_t*` の戻り値 → `Option[bytes]`、それ以外のポインタの戻り値 → `Option[Ptr[T]]`、`void*` の引数 → `Ptr`（今は `int`）、`char*` の引数 → `bytes`（D-13）、`None` → NULL。既存の例題（DxLib の実用例など）で影響する箇所を洗い出して移行する | 1-2・1-4・1-5・3-1・3-3 | 中 |
| **4-4** | ✅（§5）typed ABI に `Ptr`・`bytes`・`PtrProtocol` の受け渡しを足す（4 つの呼び出し経路と `cpp_typed_eligible` / `build_cpp_typed_sig`）。`PtrProtocol*` の引数で `address` が 0 に書き戻されたら `Ptr` を無効にする | 4-2・4-3 | 中 |
| **4-5** | ✅（§5）`PtrProtocol` の戻り値を `Option[Ptr[T]]` で受け取り、`to_list()` / `to_bytes()` で展開できるようにする（D-10）。`release` は `Ptr` の `Drop` / `release()` で 1 回だけ呼ぶ。STATIC の無い NULL の `release` はエラー（D-15） | 4-4 | 中 |
| **4-6** | ✅（§5）構造体を展開する: by-value の構造体は境界で（C 互換レイアウトのものは memcpy）、ポインタ型フィールドは D-7 / `PtrProtocol` の規則で、構造体の配列は `Ptr[構造体].to_list()` で。今の「コンストラクタにフィールドを 1 つずつ渡す」再構築をやめる | 1-3・4-5 | 中 |
| **4-7** | ✅（§5）Arrow → C: `list` → `PtrProtocol` の引数（呼び出しの間だけ有効なコピーを渡す・D-15）、`bytes` → `char*`。`mut` の `list` と出力バッファは呼び出しの後に書き戻す（D-14） | 4-5 | 中 |
| **5-1** | ✅（§5）cs-dll: `[UnmanagedCallersOnly]` の export（今は利用者が手書き）を生成する（`arrow --gen-cs-exports X.dll`。Arrow の CLI が型スタブと同じ読み方で C# を書く・利用者の判断）。今の ABI（数値・文字列・クラスのハンドル・nullable）を生成し、C# の例外を Arrow の例外にする（今はプロセスが落ちる）・整数の幅を確かめる（今は黙って切る）。例題の手書きの export を置き換える | 2-1 | 中 |
| **5-2** | cs-proc / js-proc: 値表現に dict / tuple / set / 配列 / `bytes` を足してコピーで運ぶ。`Ptr` を渡したらエラー | 2-1・2-3・3-1・3-2 | 中 |
| **5-3** | py-int: Python の `bytes` / `bytearray` / `memoryview` ↔ Arrow の `bytes` | 2-6・3-1 | 小 |
| **5-4** | rs: `Vec<u8>` → `bytes`（今は 16 進文字列）。数値の `Vec<T>` → `Option[Ptr[T]]`、`Vec<String>` など → `list[T]`（D-16）。`Option<T>`・`&[T]`・`String` フィールドを受け渡す（ラッパーは生成した Rust なので素直に書ける） | 2-5・3-1・4-5 | 中 |
| **5-5** | cs-dll: `byte[]` ↔ `bytes`、`IntPtr` / `nint` ↔ `Ptr`、数値の `T[]` ↔ `Option[Ptr[T]]`（アンマネージドへ複製して `PtrProtocol` で渡す）、`List<T>` / `string[]` → `list[T]`（D-16）。引数は `Ptr`・`list`・`bytes` を受ける。cs-proc の型スタブは D-5 のとおり `list` / `bytes`（5-2） | 3-1・4-5・5-1 | 中 |
| **6-1** | 非同期タスクのインタープリタにも読み込んだ DLL を見せる（`native_libs`） | なし | 小 |
| **6-2** | cs-proc / cs-dll のブリッジの登録表をスレッドを越えて使えるようにする | なし | 小 |
| **7-1** | 型検査: 要素型の無いコンテナ（`x mustbe set` / `mustbe list` の結果・Python のスタブの `set`）と要素型付きの値の `==` が「決して真にならない」という偽の静的エラーになる（`let s = a mustbe set` の後の `s == {1}`）。要素型の無いものは要素型付きと比べられるとみなす（2-6 で発見） | なし | 小 |
| **7-2** | `len(str)` が UTF-8 のバイト数を返す（`len("日本語")` は 9）のに、添字・反復は文字単位で食い違う（参照実装は 3）。文字数に揃える。VM の特化命令・ネイティブ codegen・既存の例題への影響を測ってから（2-2 で発見） | 3-1（バイト数が要るときの書き方 `len(s.encode("utf-8"))` を先に用意する） | 中 |
| **7-3** | 例外の `str(e)` / `print(e)` がメッセージではなく `<ValueError object at 0x…>` になる（アドレスが毎回変わり、出力の比較もずれる）。Python と同じくメッセージにする（1-6 で発見） | なし | 小 |
| **7-4** | `FfiTypeError` と、C# 側で投げられた例外（`CsProc: …` / `CsDll: …`）・js-proc の `remote` の誤りを `except` で捕まえられない（例外クラスとして登録されていない・`make_internal_raised_error`）。登録して捕まえられるようにする（2-1・2-3 で発見） | なし | 小 |
| **7-5** | 字句解析が UTF-8 の BOM（先頭の U+FEFF）を受け付けない。PowerShell 5.1 の `Set-Content -Encoding UTF8` で書いたファイルが ParseError になる（2-4 で発見） | なし | 小 |
| **7-6** | 外部言語のスタブのクラスを型の位置で名前空間付きに書けない（`x is cp.Person` が静的エラー。今は `Person` と書く）。Arrow のモジュールのクラス（`tags.Tag`・10-8）と同じく引けるようにする（2-1 で発見） | なし | 中 |
| **7-7** | 評価コアのビルド（`cargo check --no-default-features`）が通らない。`parser/imports` が `native` 専用の `python_converter` / `partial_compiler::rs_loader` を参照している（2-1 で発見・既存） | なし | 小 |
| **7-8** | Python のスタブの型の変換が要素型を落とす: `frozenset[T]` → `Any`、`set[tuple[int, int]]` → `set[tuple]`（2-6 で発見） | なし | 小 |
| **7-9** | 境界で `int` に入らない整数の扱いを揃える: C / C# / Rust は OverflowError（1-6・2-1・2-5）だが、`-> int` と宣言した Python の関数が i64 に入らない int を返すと FfiTypeError になる（2-6 で発見） | なし | 小 |
| **7-10** | 参照実装（`impl_python`）に `bytes` と `str.encode` / `bytes.decode`、`Ptr[T]` の型を足す。今は `bytes` の名前も `str.encode` も無く、`Ptr[int32]` の注釈も構文解析できないので、`compare_python_impl` がどちらの意味論も守れない（3-1 の 3 本と 3-3 の `ptr_types.ar` は既知の差分）。文字コードの名前と失敗の扱い（置き換えない・`shift_jis` は CP932 の表）を Rust 側と揃える（3-1・3-3 で発見） | 3-1・3-3 | 小 |
| **7-11** | バイト単位で開いたファイル（`open(path, mode, StartPoint.top, ByteRecognizingMode.byte)`）の `read()` / `write()` が `bytes` ではなく `list[int]` を受け渡す（Python の `"rb"` / `"wb"` は `bytes`）。`bytes` に揃える。既存の例題（`built_in.ar`）と型検査の戻り値の型を移行する（3-1 で発見） | 3-1 | 小 |
| **7-12** | ✅（§5）C のハンドル経路の数値の取り出し（`native_api/callbacks.rs` の `ar_to_int` / `ar_to_float`）が、数値でない値（`str`・`list`・`Ptr` など）を黙って `0` / `0.0` にする（`_ => 0`）。引数は静的検査と `CheckBefore` が先に止めることが多いが、`Any` と注釈した引数（構造体ポインタ・skill `c-abi-interop` の P6）など止めない経路が残るかは未確認。4-3 で `Ptr` を C へ渡す経路を作る前に、届きうる経路を洗ってエラーにする（3-2 で発見） | なし | 小 |
| **7-13** | DxLib の実用例（`examples/practical_examples/DxLib`）を動く状態に戻す。⚠ ゲートの対象外で、4-3 の時点で次の理由で型検査の前・型検査で止まっている: ヘッダの置き場所が合わない（`DxLib/DxLib.h` を探すが `examples/DxLib/` にある）、`const TCHAR*` の `TCHAR`（Windows の `tchar.h` の typedef）を解決できず `DrawString` / `SetWindowTextDX` などの文字列の関数が使えない（非 Unicode ビルドの `char` として解決する手段が要る）、`DEFAULTPARAM` の既定値つきの引数が数に入る（`DrawString` が 5 個を要求）、`RECT*` などの Windows の構造体。文字列の引数は 4-3 で `.encode("cp932")` に書き換えてある（4-3 で発見） | 4-3 | 中 |
| **7-14** | ポインタのフィールドを持つ構造体の配列を `Ptr[構造体].to_list()` で展開する。4-6 では正確なレイアウト（フィールドがすべて数値）の構造体の配列だけを展開し、ポインタのフィールドを持つもの（`Named` の `const char* name` など）は TypeError にしている。C のレイアウト（ポインタの幅・境界）を実行時に知り、要素ごとに D-7 の規則で変換する（4-6 で発見） | 4-6 | 小 |
| **7-15** | ✅（§5）型検査: Union の値をより広い Union へ渡せない。`Option[int]` → `Union[int, str, None]`、C の関数が返した `Option[Ptr[Db]]` → `void*` の引数（`Option[Ptr[void]]`）が静的エラーになる。期待の構成型 1 つで実引数の Union を丸ごと照合していた。参照実装も同じ（4-7 で発見） | なし | 小 |
| **7-16** | C# の `ref` / `out` 引数が書き戻されない（cs-dll・cs-proc）。型スタブは `mut` を求める（P6）が、どちらのブリッジも呼び出しの後で変数へ戻さないので、`out` の結果が黙って失われる。書き戻すか、未対応として静的エラーにする（5-1 で発見） | なし | 小 |

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
  解決されない（既存・`is cp.Person` は静的エラー・⇒ 7-6）。C# の例外は `CsProc: …` のままで `except` で捕まえられない（既存・⇒ 7-4）
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
  文字単位（参照実装の `len` は 3）。外部言語とは関係しない言語の意味論の問題。⇒ 7-2 として起票
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
- ⚠ 残したもの: FfiTypeError は例外クラスとして登録されておらず `except` で捕まえられない（既存・⇒ 7-4）。node のプロセスが
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


### 2-5 実装

- **呼べないシグネチャの報告**（`src/rs_crate/parse.rs`）: 関数・メソッドを「呼べる」「呼べない（理由つき・`RsUnsupported`）」に
  分け、呼べないものも印付きで型スタブに載せる。型検査が使った箇所を `FfiUnsupported` にする（モジュールの関数は
  "Rust function"、クラスのメソッドは "Rust method"、構築は "Rust constructor"）。⚠ 以前は import 時に黙って消え、
  使うと「そんな名前は無い」になっていた。対象: コレクション・`Option`・タプル・スライス・`&mut` の引数・ジェネリック・
  `self` の無い関連関数（`new` 以外）・`self` を消費するメソッド・作れない構造体の構築
- **`new()` を使う構築**: `new` を探す処理が `self` の無い関数を捨てていたので、`new` は一度も使われず、private フィールドの
  ある構造体は構造体リテラルでビルドに失敗して **import 全体が落ちていた**。`new` → 構造体リテラル（全フィールドが `pub` で
  渡せる型のとき）→ どちらも無理なら構築を未対応（メソッドの戻り値としては使える）。`pub` フィールドの無い構造体も拾う
- **`pub String` フィールドのセッター**: 値を構造体へ移した後に読んでいてビルドに失敗していた（同じく import 全体が落ちた）。
  Arrow 側へ写す値を移す前に作る
- **フィールドの値**: `&mut self` のメソッドの後に Rust の値を Arrow 側のフィールドへ写し直す（`_sync_fields_*`）。逆向きに、
  Arrow から `obj.field = v` と代入すると生成したセッターを呼ぶ（`attr_assign_evaled`）。⚠ 以前は構築時の値のまま、代入は
  Arrow 側の写しだけが変わった
- **戻り値の構造体**: 戻り値をスレッドローカルの枠（`ADOPT_*`）に置いてクラスを呼び、`__init__` がそれを引き取る。⚠ 以前は
  戻り値のフィールドから**構築関数を通して作り直していて**、`new` の引数がフィールドと違うと誤り、private フィールドでビルド失敗
- **ほかに直したもの**: 複数行のシグネチャ（rustfmt の折り返し）を読む（以前は黙って消えた）。`-> Self` のメソッドを構造体の
  戻り値として扱う（以前は消えた）。整数の範囲を検査する（`try_from`・Arrow の int に入らない戻り値・Rust の型に入らない引数は
  OverflowError。以前は `as` で黙って折り返した）。引数にしか使えない型（`&[u8]`）・戻り値にしか使えない型（`Vec<u8>` /
  `[u8; N]`）を分けた（以前は `Vec<u8>` の引数でビルドに失敗した）
- 実測した以前の症状（`552548b`）: `import[rs] ffi_probe_rs` が `cannot construct Account with struct literal syntax due to private
  fields` と `borrow of moved value: _val` でビルドに失敗した
- 検査用: `ffi_probe_rs` に `-> Self`・作れない構造体（`Sealed`）・`pub` フィールドの無い構造体（`Secret`）・複数行のシグネチャ・
  整数の範囲・呼べないシグネチャを足した
- 例題: `rs_values.ar`（期待値と違えば raise）・`rs_unsupported_signature_error.ar`（9 か所の静的エラー）・`rs_u64_overflow_error.ar`
- 単体テスト: `rs_crate::parse::probe_tests`（構築の方法・メソッドの分類・関数の分類・型の位置）
- 文書: `docs/grammar/09_imports.md` の Rust、skill `importation` の Rust の節。VSIX を作り直した
- ⚠ 残したもの: `Vec<String>` などの受け渡し（5-4）。`pub` フィールドのうち渡せない型のものは型スタブに載らない（使うと「無い」）
- ゲート（基準 `552548b`・制限時間つき）: `scan_examples`・`force_gate`（`VmForceError` 0）は基準と同じ。`compare_python_impl` clean
  （新しい 3 本は cargo に依存するので比較から外した）。`compare_outputs` の差 3 本は新しい例題（基準では中継 DLL のビルドに失敗）。
  負の対照（基準どうし）でも同じ 3 本がずれたのは、基準が構造体を `HashMap` の順で並べて生成するため cargo のエラーの行番号が
  回ごとに変わるから（今は名前順に並べるので、新しい exe の出力は毎回同じ）。`compare_import_paths` 13/13・
  `compare_wasm_frontend` 509/509・`type_obligations` 退行なし・`stale_doc_refs` 0・`cargo test` 1087 件すべて成功


### 2-6 実装

- `py_interop.rs`: Arrow の set → Python の set、Python の set / frozenset → Arrow の set。Arrow の set に入れられない要素
  （list など・PyObject）を含む set は、dict と同じく要素を捨てずに丸ごと PyObject で渡す
- 変換できない値のエラーに型名を出す（インスタンスはクラス名・それ以外は `runtime_type_name`。以前は `'unknown'`）
- ⚠ ついでに直した黙った変換: i64 に入らない Python の int は f64 への変換に落ちて**黙って float**（精度を失う）になっていた →
  PyObject のまま。`__float__` を持つ値（`Decimal` / `Fraction`）も黙って float になっていた → `float` のインスタンスだけ float に
- 実測した以前の症状（`121218a`）: `2**70` → `1180591620717411300000`（float）、`Decimal("0.1")` → `0.1`（float）、
  Arrow の set を渡す → `cannot convert tl value of type 'unknown' to Python`、Python の set → PyObject（`set[int]` と宣言した
  関数は FfiTypeError）
- 検査用: `examples/interop/py_int_probe.py`。例題: `py_int_values.ar`（期待値と違えば raise）・`py_int_instance_arg_error.ar`
- 文書: `docs/grammar/09_imports.md` の py-int、skill `importation`
- ⚠ 残したもの: `-> int` と宣言した Python の関数が i64 に入らない int を返すと FfiTypeError（宣言と違う値として。C / C# / Rust の
  OverflowError とは違う・⇒ 7-9）。Python のスタブの型の変換は `frozenset[T]` を知らず `Any`、`set[tuple[int, int]]` を `set[tuple]` に
  する（既存・⇒ 7-8）
- ⚠ 見つけたもの（範囲外・既存の型検査の不具合）: 要素型の無いコンテナ（`x mustbe set` / `mustbe list` の結果・Python の
  スタブの `set`）と要素型付きのリテラルの `==` が「決して真にならない」という偽の静的エラーになる
  （`let s = a mustbe set` の後の `s == {1}`）。⇒ 7-1 として起票
- ゲート（基準 `121218a`・制限時間つき）: `scan_examples`・`force_gate`（`VmForceError` 0）は基準と同じ。`compare_python_impl` は
  新しい 2 本を既知の差分に登録して clean（参照実装は Arrow の set / インスタンスを内部の値のまま Python へ渡す）。
  `compare_outputs` の差 2 本は新しい例題（基準では set が FfiTypeError・型名が 'unknown'）、負の対照 424/424。
  `compare_import_paths` 13/13。型検査・拡張は変えていない

### 3-1 実装

- 値の種類 `Value::Bytes(Rc<[u8]>)`（不変）と型 `InferredType::Bytes`。`bytes()` / `bytes(b)` / `bytes(list[int])`（0〜255 の外は
  ValueError・int 以外の要素は TypeError）。`bytes(str)` は TypeError（Python と同じく文字コードを書かせる）
- 操作: `len`・添字（`int`）・スライス（`bytes`）・反復（`int`）・`+`（連結）・`==` と順序比較（バイトの辞書順）・`in`（部分列か
  1 バイト）・dict のキーと set の要素（値で等値・ハッシュ）・`is bytes` / `mustbe bytes`・真偽値（空が偽）。表示は Python の repr
  （`b'hi\xff'`）
- 文字コード（`src/interpreter/encoding.rs`）: `s.encode(enc="utf-8") -> bytes`・`b.decode(enc="utf-8") -> str`。`utf-8` / `utf-16-le` /
  `utf-16-be` / `ascii` / `latin-1` は自前で Python と同じ意味、`shift_jis`（= `cp932` / `sjis` / `windows-31j`。表は CP932 と同じ）と
  そのほかの WHATWG の名前は encoding_rs。変換できなければ位置つきの ValueError（置き換え文字や `&#…;` にしない）。BOM は足さず
  読まない（`utf-16` は曖昧なのでエラー）。知らない名前も ValueError
- 型検査: `bytes` と `str` を混ぜる（代入・`+`・`in`）と静的エラー。添字は `int`、`str.encode` → `bytes`、`bytes.decode` → `str`。
  型検査の組み込み名の表（`RUNTIME_BUILTIN_NAMES`）に `bytes`
- ⚠ 変えた挙動: `str.encode()` は以前、文字コードの引数を無視して UTF-8 の `list[int]` を返していた → `bytes`（`built_in.ar` の出力が
  `[72, 105]` から `b'Hi'` に）。`bytes` が組み込みの名前になり、変数名に使うと静的エラーなので、使っていた例題 2 本
  （`built_in.ar`・`archived/file_io.ar`）を改名
- ⚠ 範囲外: リテラル `b"..."`（計画どおり）。バイト単位のファイルの `read()` / `write()` はまだ `list[int]`（⇒ 7-11）。参照実装に
  `bytes` が無く、新しい例題 3 本は `compare_python_impl` の既知の差分（⇒ 7-10）
- 例題: `typing/bytes_basics.ar`（期待値と違えば raise）・`bytes_decode_error.ar`（実行時 ValueError）・`bytes_str_mix_error.ar`
  （静的エラー 4 件）
- 文書: `docs/grammar/08_type_system.md`、skill `type-checking` / `interpreter-internals` / `architecture-overview`
- ゲート（基準 `a1897bd`・制限時間つき）: `scan_examples`（基準と同じ。`bench_ab_native` のタイムアウトだけ）・`force_gate`
  （`VmForceError` 0）。`compare_python_impl` は新しい 3 本を既知の差分に登録して clean（170/170）。`compare_outputs` の差 4 本は
  `built_in.ar`（`"Hi".encode()` の表示が `b'Hi'` に）と新しい例題 3 本、負の対照 427/427。`compare_import_paths` 13/13。
  `compare_wasm_frontend` 514/514・`type_obligations` 退行なし・`hash_eq_identity` 一致・`stale_doc_refs` 0 件。`cargo test` 1092 件・
  拡張の wasm クレートの 676 件が通る。VSIX を作り直した

### 3-2 実装

- 値の種類 `Value::Ptr(Arc<PtrHandle>)`（`src/interpreter/value/ptr.rs`）。持つもの: アドレス（0 では作れない。NULL は `None`・
  D-8）・要素の型名（D-12 の ABI 側の名前。Arrow の型への対応は 3-3）・flags（`PTR_FLAG_CONST` / `PTR_FLAG_NUL_TERMINATED` /
  `PTR_FLAG_STATIC`。⚠ 4-1 の共有ヘッダと同じ値にする）・長さ（生のポインタは分からないので無し）・所有（`Owned` は解放関数と
  ctx / `Static` / `Borrowed`）・状態（使える / 解放済み / C の側で解放済み）・DLL への参照（`PtrKeepAlive`。ブリッジごとに
  ライブラリの型が違うので `Arc<dyn Any + Send + Sync>`）
- 解放: 最後の参照がなくなったとき（`Drop`）か `p.release()` で 1 回だけ。コピーも `deep_clone`（非同期タスクへ渡すとき）も
  共有する。`release()` は借用・STATIC・解放済み・C の側で解放済みのものには ValueError（黙って何もしない、にしない）。
  `invalidate()`（4-5 で使う）は解放関数を呼ばずに使えなくする
- 等値・ハッシュは同一性（`Arc::ptr_eq` / `Arc::as_ptr`）。同じアドレスを指す別の `Ptr` は等しくない。表示は
  `<Ptr[int32] len=4>`（アドレスを出さない・D-3）、型名は `Ptr`、真偽値は常に真。網羅的な match は 4 箇所（型名・真偽値・
  表示・ハッシュ）、`_` で受けている箇所は従来どおりエラーか「等しくない」
- インタプリタの `native_libs` を `Arc<NativeLibWrapper>` にした（`Ptr` が DLL を生かしておけるように）
- ⚠ Arrow から `Ptr` を作る手段はまだ無い（作るのは 4-3・4-5 のブリッジ）。そのため例題は無く、単体テスト（`value/ptr.rs` 7 本・
  `interpreter/tests/ptr.rs` 5 本）で確かめた。`value/ptr.rs` の dead_code の抑止は 4-3 で外す
- ⚠ 実装中に踏んだもの: `release()` の 2 回目が、錠を持ったまま `address()` で同じ `Mutex` を取り直して止まった（テストが
  待ち続けた）。状態を引数で渡す形にした
- ⚠ 見つけたもの: C のハンドル経路の数値の取り出しが数値でない値を黙って 0 にする（⇒ 7-12。4-3 の前に要る）
- ゲート（基準 `338ef86`・制限時間つき）: `scan_examples`（基準と同じ）・`force_gate`（`VmForceError` 0）・`compare_python_impl`
  clean（170/170）・`compare_outputs` 427/427 一致（負の対照 427/427）・`compare_import_paths` 13/13・`hash_eq_identity` 一致・
  `stale_doc_refs` 0 件。`cargo test` 1104 件・拡張の wasm クレートの 688 件が通る。型検査・拡張の表示は変えていない

### 3-3 実装

- 型 `InferredType::Ptr(Box<PtrElem>)`（`src/type_check/types.rs`）。要素は `PtrElem`: C の幅つきの数値（`Num("int32")`）・
  `Void`・構造体（`Type(NamedInstance)`）・要素型不明（素の `Ptr`。型テストだけ）。⚠ `InferredType` とは別に持つのは C の幅が
  要るから（`int32` も `int64` も `InferredType` では `Int`）
- 規則: 同じ要素型どうしと、どの `Ptr` から `Ptr[void]` へは暗黙に渡せる。`Ptr[void]` → `Ptr[T]` は `p mustbe Ptr[T]`
  （実行時に**作られたときの要素型**と照合・`ptr_ann_matches`）。要素型の分かっている `Ptr` どうしの `mustbe` は
  `MustBeCanNeverSucceed`。`is Ptr[T]` も実行時に判定できるので `GuardTypeNotTestable` にしない
- 注釈: 要素は C の型・構造体・`void` だけ（`Ptr[int]` / `Ptr[str]` は新設の `PtrElemNotCType`）。素の `Ptr` は
  `BareContainerAnnotation`。構造体の名前は存在検査の対象
- メソッド: `release() -> None`・`to_list() -> list[T]`（数値は `int` / `float`）・`to_bytes() -> bytes`（`int8` / `uint8` /
  `uint16` だけ）。それ以外のメソッド・`Ptr[void]` の `to_list()`・文字型でない `to_bytes()`・引数つきは新設の
  `PtrMethodUnavailable`（アドレスを取り出す手段を作らない・D-3）
- `mut` の仮引数（`T*` の引数）へ `let` の `Ptr` を渡せる（`mut_arg_exempt`。Arrow から中身を書き換えられないので
  関数値と同じ扱い）
- 実行時: `to_list()` は要素型の幅で読む（符号拡張・`float32` は広げる・`uint64` の 2^63 以上は OverflowError）。
  `to_bytes()` は長さか NUL 終端（flags）まで。構造体の要素・`void`・長さの無い生のポインタは誤り（構造体は 4-6）。
  型の照合は `primitive_ann_matches` と `value_matches_type`（2 つ目の表）と FFI 境界の検査が同じ `ptr_ann_matches` を使う
- 型の木を歩く処理を `InferredType::for_each_type` 1 本にした（`collect_type_names` と `Ptr` の要素の検査が共有）
- ついでに直したもの: `mustbe` の成否の検査が `bytes` を知らなかった（`b mustbe str` が素通し）
- 例題: `typing/ptr_types.ar`（受け取る側の型検査と `is Ptr` の実行時判定・期待値と違えば raise）・
  `ptr_method_error.ar`・`ptr_type_error.ar`（静的エラー 3 件・5 件）。⚠ 値を作る例題は 4-3 / 4-5 で足す
- 拡張: 構文の色付けに `Ptr` を足した。文書: `docs/grammar/08_type_system.md`、skill `type-checking`
- ゲート（基準 `f7e92a5`・制限時間つき）: `scan_examples`（基準と同じ）・`force_gate`（`VmForceError` 0）・`compare_python_impl`
  clean（172/172。`ptr_types.ar` は既知の差分、エラー例題 2 本は標準出力が空で一致）・`compare_outputs` の差 3 本は新しい
  例題だけ（負の対照 430/430）・`compare_import_paths` 13/13・`compare_wasm_frontend` 517/517・`type_obligations` 退行なし・
  `hash_eq_identity` 一致・`stale_doc_refs` 0 件。`cargo test` 1118 件・拡張の wasm クレートの 692 件が通る。VSIX を作り直した

### 7-12 実装

- 実測（`7f37f84`）: 型検査は `int` の引数へ `Any` / `str` を渡すのを止めるが、**素の `function` 型の変数・仮引数を通すと
  引数は照合されない**。そこで C の `int` の引数へ `"abc"` / `None` / `[1]` を渡すと黙って 0、`bool` の引数は `True` 以外が
  黙って偽になり、その値で C の関数が呼ばれていた（`fp_is_even("abc")` → `True`）。Rust の crate も同じで、`&str` の引数は
  `None` で空文字列のまま関数を呼び、呼んだ後でエラーになっていた
- `ar_to_int` / `ar_to_float`: 数値でない値（`None`・`str`・`list`・`Ptr` …）はエラーを設定して 0（最初の誤りを残す）。
  数値どうしは C の暗黙の変換のまま（`float` の切り捨て・`bool` の 0 / 1・`int` → `double`）
- コールバックを 2 つ足した: `to_bool`（`True` / `False` だけ）・`pending_error`。cpp / rs の中継コードは引数を全部変換した
  後に `pending_error` を見て、**関数を呼ばずに**戻る（呼び出しの後でエラーが例外になる）。値渡しの構造体のフィールドの
  取り出し・Rust の `String` / `&str` の引数も同じ網に入る
- ⚠ コールバックの表（`ArCallbacks`）には末尾に足すこと。cpp / rs の中継コードは表の先頭部分だけを宣言し、ネイティブ
  コンパイルの生成コードは添字で引く。3 つの写しの欄の並びが一致することを確かめた（skill `partial-compile` の表）
- ネイティブコンパイルの生成コードも `to_int` / `to_float` で値を開くので、同じ規則になる（数値でない値を黙って 0 にしない）
- 検査用に `ffi_probe` へ `fp_count_*` / `fp_call_count`、`ffi_probe_rs` へ `count_*` / `call_count` を足した（呼ばれた回数で
  「誤った引数では呼ばれていない」を確かめる）
- 例題: `interop/cpp_native_arg_check.ar`（cpp-dll・期待値と違えば raise）・`cpp_native_arg_error.ar`（cpp-lib・捕まえない
  TypeError）・`rs_native_arg_check.ar`。単体テスト 3 本（`callbacks.rs` 2 本・`cpp_bridge/codegen.rs` 1 本）
- ⚠ 残したもの: 素の `function` 型を通すと引数が静的に照合されないこと自体（型検査の設計どおり。実行時の境界で止める）。
  `const char*` の引数のエラーの文言は 1-2 のまま（Rust の `&str` の引数でも C の文字列と言う）
- ゲート（基準 `7f37f84`・制限時間つき）: `scan_examples`（基準と同じ）・`force_gate`（`VmForceError` 0）・`compare_python_impl`
  clean（172/172）・`compare_outputs` の差は新しい例題 3 本と `cpp_lib_int_widths.ar`、`compare_import_paths` は
  `cpp_struct_ptr.ar` が差。⚠ 後者と負の対照の差（`cpp_default_arg_native_call.ar` など 3 本）は、中継コードの生成が変わった
  ので新旧のバイナリを交互に走らせるたびに中継 DLL を作り直し、その表示が混ざったもの。各バイナリで 2 回目（作り直し
  なし）の出力を比べると 3 本とも同一だった。`stale_doc_refs` 0 件。`cargo test` 1121 件・拡張の wasm クレートの 692 件が通る

### 4-1 実装

- 共有ヘッダ `bridge/ptr_protocol/ar_ptr_protocol.h`（`ArPtrProtocol`・flags は `AR_PTR_CONST` / `AR_PTR_NUL_TERMINATED` /
  `AR_PTR_STATIC`・解放関数の型 `ArPtrRelease`）、C# の写し `bridge/ptr_protocol/ArPtrProtocol.cs`（`ArrowInterop` 名前空間・
  `[StructLayout(Sequential)]`・関数ポインタ `delegate* unmanaged`）、Rust の写し `PtrProtocolRaw`（`value/ptr.rs`）、
  仕様 `docs/ptr_protocol.md`（型名の表・flags・解放の約束・例）
- ⚠ 名前は `ArPtrProtocol`（D-1 の `PtrProtocol` に接頭辞を付けた。利用者のコードの名前とぶつからないように）
- ⚠ D-1 の並びの `flags`（4 バイト）と `length`（8 バイト）の間には詰め物が入るので、`_reserved`（0）として明示した
  （C# と並びを取り違えないため）。64 ビットで 48 バイト。`length` は「NUL 終端で数えない」ときに -1 を許す
- 型名（D-12）: `int8` … `uint64`・`float32` / `float64`・構造体の名前・`void`。文字データは `uint8`（`char`）と `uint16`
  （`wchar_t` / `char16_t`）。`bool` は使わない（真偽値の配列は `uint8`）
- 一致の網: 単体テスト 2 本（flags の値と欄の順序を C のヘッダ・C# のソースから読んで突き合わせる、Rust の写しの大きさと
  欄の位置）と `scripts/check_ptr_protocol.ps1`（C・C++・C# を実際にコンパイルして大きさ・欄の位置・flags を出し、Rust の
  期待値と比べる。3 つとも一致）
- 文書: `docs/grammar/09_imports.md` の cpp の節（7-12 の挙動も書き足した）・`08_type_system.md` の `Ptr[T]` から案内
- ゲート（基準 `1132ded`・制限時間つき）: 実行時の挙動は変えていない（定数・型を足しただけ）。`compare_outputs` 433/433 一致
  （負の対照 433/433）・`scan_examples`（基準と同じ）・`force_gate`（`VmForceError` 0）・`compare_python_impl` clean（172/172）・
  `compare_import_paths` 13/13・`stale_doc_refs` 0 件。`cargo test` 1123 件・拡張の wasm クレートの 694 件が通る

### 4-2 実装

- `CType::PtrProtocol { by_pointer, mutable }`（`src/cpp_header/types.rs`）。`ArPtrProtocol`（値）・`ArPtrProtocol*`・
  `const ArPtrProtocol*`（`struct` 付きも）を、引数・戻り値・構造体のフィールドで認識する。⚠ ヘッダ解析は `#include` を
  辿らないので**名前で**見分ける（利用者の対応表の後・typedef の表の前）。共有ヘッダの定義そのもの（取り込んだ・写した
  もの）は Arrow のクラスにしない
- 受け渡しは 4-4・4-5・4-7 で入るので、使う関数は理由つきで使えない（`unsupported_reason`。例:
  ``it returns `ArPtrProtocol`, which Arrow cannot receive yet``）。以前は知らない構造体として扱われ、「レイアウトが
  分からない」という理由で止まっていた。ポインタを返す（`ArPtrProtocol*`）関数は今後も使えない（値で返す）
- 型スタブ: 戻り値（値）は `Option[Ptr[void]]`（要素の型は実行時に届くので `p mustbe Ptr[T]` で照合して戻す・
  D-10・D-12）、ポインタの引数は `Ptr[void]`。値・`const` ポインタの引数（Arrow → C）の型は 4-7 で決める
- `ArPtrProtocol` のフィールドを持つ構造体は正確なレイアウトにならない（展開は 4-6）
- ⚠ 4-1 の共有ヘッダを ASCII にした（英語のコメント）。利用者の C のコードに取り込まれ、MSVC は既定で CP932 として
  読むので、UTF-8 のコメントが次の行を飲み込みうる（`ffi_probe.h` の注意と同じ）。単体テストが ASCII を見る
- 検査用の `ffi_probe` に `ArPtrProtocol` を受け渡す関数を足した（`fp_squares` / `fp_static_text` / `fp_no_release` /
  `fp_released` / `fp_protocol_sum` / `fp_protocol_free`。使うのは 4-4・4-5・4-7）。`ffi_probe.h` が共有ヘッダを取り込み、
  cpp-dll・cpp-lib（シム）とも既存の例題が通ることを確かめた
- 例題: `interop/cpp_ptr_protocol_pending_error.ar`（静的エラー 2 件。⚠ 4-5 で正例に置き換える）。単体テスト 1 本
- ゲート（基準 `d21894c`・制限時間つき）: `compare_outputs` の差は新しい例題だけ（負の対照 434/434）・`scan_examples`（基準と
  同じ）・`force_gate`（`VmForceError` 0）・`compare_python_impl` clean（172/172・新しい例題は既知の差分）・`compare_import_paths`
  13/13・`compare_wasm_frontend` 521/521・`type_obligations` 退行なし・`check_ptr_protocol` 一致・`stale_doc_refs` 0 件。
  `cargo test` 1124 件が通る。VSIX を作り直した

### 4-3 実装

- 型スタブ（`ctype_to_tl_str` が位置（引数・戻り値・フィールド）を受けるようにした）: `char*` / `wchar_t*` → `Option[bytes]`、
  ほかのポインタの戻り値・`void*` の引数・レイアウトの分からない構造体ポインタの引数 → `Option[Ptr[T]]`（`T` は
  `CType::ptr_elem_name`: `void` / 構造体名 / `int32` など）。数値の書き込みポインタの引数は従来どおり指す先の型、正確な
  レイアウトの構造体ポインタの引数は `Any`（インスタンス）
- 実行時（ハンドル経路の中継コード）: コールバックを 4 つ足した（`make_bytes` / `make_ptr` / `bytes_arg` / `ptr_arg`・表の
  40〜43）。`char*` の戻り値は NUL までをコピーした `bytes`（`wchar_t*` は UTF-16LE）、ほかのポインタの戻り値は借用の
  `Ptr[T]`（Arrow は解放しない）、NULL は `None`。`const char*` の引数は `bytes` か `None`（`str`・NUL を含む `bytes` は
  エラーで C の関数を呼ばない）、`void*` / 構造体ポインタの引数は要素型の合う `Ptr` か `None`。⚠ 型つき経路での
  `Ptr` / `bytes` の受け渡しは 4-4（`char*` / `void*` を含む関数は型つき経路に載らないのでハンドル経路で動く）
- D-13: `const char*` へ `str` を渡すと静的エラーで、文面が `.encode(...)` を促す（`CallArgTypeMismatch` の文面）
- 構造体として宣言された名前だけを `Ptr[T]` にする: 前方宣言（`typedef struct Db Db;` / `struct Db;`）を集めて、中身の無い
  構造体の定義として持つ。宣言の無い名前（解決できない typedef）へのポインタは理由つきで使えない。⚠ 実装中に DxLib の
  `const TCHAR*` が「構造体 `TCHAR` へのポインタ」に見えて `Ptr[TCHAR]` を受ける関数になってしまうのを見つけて止めた
  （`TCHAR` は Windows の `tchar.h` の typedef で、実は C の文字列）。`struct Name*` の `struct` も外して突き合わせる
- 中身の見えない構造体は、型スタブにフィールドの無いクラスとして置く（`Ptr[Db]` の名前になる。Arrow から作ると理由つきの
  静的エラー）。実行時のクラスは作らない
- レイアウトの分からない構造体ポインタの引数は、`Ptr` を渡すので `mut` を要求しない（書き戻すのは正確なレイアウトの
  インスタンスだけ）。`Option[Ptr[T]]` の `let` も `mut` の仮引数へ渡せる（`mut_arg_exempt`）
- ⚠ 直ったもの: ハンドル経路は構造体ポインタの引数に Arrow 内部のハンドル番号をそのままポインタとして渡していた
  （skill `c-abi-interop` P7 の残課題・落ちうる）。今は `ptr_arg` が `Ptr` 以外をエラーにする
- 移行: `cpp_cstring_arg.ar` / `cpp_cstring_nul_error.ar`（`str` → `bytes`）、`cpp_unsupported_signature_error.ar`（使えるように
  なった関数を外した）、DxLib の実用例の文字列の引数を `.encode("cp932")` に（⚠ 実用例は `TCHAR` などのため今は動かない・
  ⇒ 7-13）
- 例題: `interop/cpp_raw_pointers.ar`（期待値と違えば raise）・`cpp_cstring_str_arg_error.ar`・`cpp_ptr_arg_error.ar`。
  単体テスト 6 本（コールバック 3・前方宣言 1・生成コード 1・要素の型名 1）と既存のテストの更新
- 文書: `docs/grammar/09_imports.md` の cpp の節（表）
- ゲート（基準 `d0737cc`・制限時間つき）: `compare_outputs` の差は変えた・足した例題 6 本と `cpp_lib_int_widths.ar` /
  `cpp_default_arg_native_call.ar`、`compare_import_paths` は `cpp_struct_ptr.ar` が差。⚠ 後者 3 本と負の対照の差は、中継コードの
  生成が変わったので新旧を交互に走らせるたびに中継 DLL を作り直した表示（2 回目の出力を比べると 3 本とも同一）。
  `scan_examples`（基準と同じ）・`force_gate`（`VmForceError` 0）・`compare_python_impl` clean（171/171・変えた例題 4 本は
  既知の差分）・`compare_wasm_frontend` 524/524・`type_obligations` 退行なし・`stale_doc_refs` 0 件。`cargo test` 1130 件・
  拡張の wasm クレートの 697 件が通る。VSIX を作り直した

### 4-4 実装

- `AbiTy` に `Bool` / `Bytes`（`const char*` の引数）/ `CStr`（`char*` / `wchar_t*` の戻り値）/ `PtrVal`（`void*`・構造体・数値の
  ポインタの引数と戻り値）/ `Protocol`（`ArPtrProtocol` の引数）を足し、`AbiTy::Ptr`（正確なレイアウトの構造体ポインタ）は
  構造体の名前を持って、インスタンスに加えて `Ptr[構造体]` も受ける（値渡しは `None` を受けない）。中継コードの `_typed` は
  純粋なキャストのまま（スロットはアドレスか 0/1）。適格の判定（`typed_abi_ok`）とシグネチャ（`build_cpp_typed_sig`）を揃えた
- 規則は `value/ptr.rs` の共通関数（`c_string_arg` / `ptr_arg_address` / `protocol_arg`）に置き、ハンドル経路のコールバック
  （`bytes_arg` / `ptr_arg`）も同じものを使う（2 か所に書くとずれるため）
- `ArPtrProtocol` の引数（値・`const` ポインタ・ポインタ）は `Ptr` / `None` から構造体を組み立てて渡す。解放関数を渡すのは
  C の側で解放できる `ArPtrProtocol*` のときだけ（値・`const` では NULL）。呼び出しの後で `address` が 0 なら、その `Ptr` を
  無効にする（以後の使用は ValueError・解放はもう呼ばない・D-3）。ハンドル経路もコールバック 2 つ（`protocol_arg` /
  `protocol_done`・表の 44〜45）で同じことをする（数値の食い違いで型つき経路から落ちたときに使う）
- ⚠ 実装中に見つけて直したもの: `invoke_typed_abi` は後片付け（引数のバッファの解放）の後で戻り値を読んでいた。C の関数が
  引数のバッファを指すポインタを返す（`fp_echo`）と解放済みの領域を読むので、後片付けの前にデコードするようにした
  （4-3 までは `char*` を含む関数が型つき経路に載らなかったので起きていなかった）。`ar_call_fn` の戻り値の自前の `match` も
  共通のデコードに寄せた
- これで、正確なレイアウトの構造体ポインタと C の文字列・`bool` が混ざった関数が呼べる（以前は「構造体ポインタの経路が
  運べない」として使えなかった）。値返しの構造体と混ざるものは引き続き使えない
- 型スタブ: `ArPtrProtocol` の引数はどの形でも `Option[Ptr[void]]`（`list` から組み立てるのは 4-7）
- 検査用の `ffi_probe` に `fp_protocol_type`（値で受けて要素の型名を返す）・`fp_rec_label`（構造体ポインタ＋`const char*`）を足した
- 例題: `interop/cpp_typed_pointers.ar`（期待値と違えば raise）・`cpp_ptr_protocol_arg_error.ar`。`cpp_ptr_protocol_pending_error.ar`
  は戻り値だけの負例にした。単体テスト 4 本（引数 2・戻り値 1・生成コード 1）と既存のテストの更新
- 文書: `docs/grammar/09_imports.md` の表、skill `c-abi-interop` / `partial-compile`（コールバックの表）
- ゲート（基準 `bd1f1ad`・制限時間つき）: `compare_outputs` の差は新しい例題 2 本・`cpp_ptr_protocol_pending_error.ar`（負例が
  1 件に）・`cpp_struct_ptr_arg_error.ar`（文面に「またはその Ptr」）と、中継 DLL の作り直しの表示 2 本（負の対照でも出る。
  2 回目の出力は同一）。`compare_import_paths` の `cpp_struct_ptr.ar` も同じ理由（2 回目は同一）。`scan_examples`（基準と同じ）・
  `force_gate`（`VmForceError` 0）・`compare_python_impl` clean（171/171）・`compare_wasm_frontend` 526/526・`type_obligations`
  退行なし・`stale_doc_refs` 0 件。`cargo test` 1134 件・拡張の wasm クレートの 700 件が通る。VSIX を作り直した

### 4-5 実装

- `ArPtrProtocol` の値返しを `Option[Ptr[void]]` として受け取る（型スタブは 4-2 のまま）。変換は `value/ptr.rs` の
  `ptr_from_protocol` 1 か所（型つき経路とハンドル経路が共有）: 要素の型名は数値・`void`・そのヘッダが宣言した構造体だけ
  （知らない名前は ValueError・D-12）、`length` は 0 以上か、NUL 終端のときだけ -1、解放関数があれば所有（最後の参照か
  `release()` で 1 回だけ呼ぶ）、NULL でよいのは STATIC のときだけ（D-15・違反は ValueError でメモリは解放しない）、
  アドレス 0 は `None`
- 型つき経路: `AbiTy::ProtocolRet`。`invoke_typed_abi` が受け取りの領域を用意して `*_ret` にそのアドレスを入れ、中継コードが
  そこへ構造体を書く（`_ret` は 1 語なので、48 バイトの構造体は呼び出し側の領域で受ける）。ハンドル経路: コールバック
  `protocol_ret`（表の 46）に構造体の名前の一覧を渡す。cpp-lib のシムは構造体の値返しと同じくキャストせずに返す
- ⚠ DLL への参照は持たせていない: 解放関数は C の DLL（cpp-lib はシムの DLL）の中にあり、中継 DLL が `LoadLibrary` した
  まま解放しないので、プロセスが終わるまでアンロードされない（D-3 の「`Ptr` が残っている間はアンロードしない」は満たす）
- 例題: `interop/cpp_ptr_protocol.ar`（展開・`release()` で 1 回・関数を抜けたときに 1 回・C の側で解放されたら使えない・
  STATIC の文字データ・NULL。期待値と違えば raise）・`cpp_ptr_protocol_release_error.ar`（D-15）。4-2 の負例
  `cpp_ptr_protocol_pending_error.ar` は正例に置き換えて消した。単体テスト 2 本と既存のテストの更新
- 文書: `docs/ptr_protocol.md`（C/C++ の対応状況）・`docs/grammar/09_imports.md` の表
- ゲート（基準 `14edb56`・制限時間つき）: `compare_outputs` の差は新しい例題 2 本と、中継 DLL の作り直しの表示 2 本（負の対照
  でも出る。2 回目の出力は同一。`compare_import_paths` の `cpp_struct_ptr.ar` も同じ）。`scan_examples`（基準と同じ）・
  `force_gate`（`VmForceError` 0）・`compare_python_impl` clean（171/171）・`compare_wasm_frontend` 527/527・`type_obligations`
  退行なし・`stale_doc_refs` 0 件。`cargo test` 1136 件・拡張の wasm クレートの 701 件が通る。VSIX を作り直した

### 4-6 実装

- 値返しの構造体: コンストラクタにフィールドを渡して作り直すのをやめた。正確なレイアウトの構造体は C のバイト列をそのまま
  インスタンスの raw ブロックへ写す（`InstanceData::from_c_bytes`・コールバック `make_raw_instance`）。ポインタのフィールドを
  持つ構造体は D-7 の規則（`char*` / `wchar_t*` → `bytes`、ほかのポインタ → 借用の `Ptr`、`ArPtrProtocol` → 所有の `Ptr`）で
  フィールドを変換し、宣言順に入れる（`make_instance`）。コールバックは表の 47〜48。型スタブの戻り値の型を構造体のクラスに
  した（以前は `Any` で、フィールドを読むのに毎回キャストが要った。引数は `Any` のまま）
- 値渡しの構造体: ポインタのフィールドも同じ規則で組み立てる（`const char*` は `bytes_arg`、ポインタは `ptr_arg`、
  `ArPtrProtocol` は `protocol_arg`）。⚠ 以前の生成コードはポインタのフィールドに Arrow 内部のハンドル番号をそのまま入れて
  いた（値渡しを止めていたので生成されなかった）
- 受け渡せるかの判定は `CStructDef::by_value_reason`（戻り値と引数で違う: 引数では C が書き込むかもしれない `char*` と
  `wchar_t*` のフィールドを渡せない）。不完全なレイアウト・関数ポインタ・入れ子の構造体は従来どおり使えない
- 構造体の配列: `Ptr[構造体].to_list()` で `list[構造体]`（要素ごとに `from_c_bytes`。要素の間隔は C の `sizeof` =
  `RawLayout::c_size`。⚠ `total_bytes` は 8 の倍数に切り上げてあるので別物）。クラスは名前で引く（中継コードの `get_global`
  と同じく大域に登録されているもの）。ポインタのフィールドを持つ構造体の配列は TypeError（⇒ 7-14）
- ⚠ 実装中に踏んだもの: バイト列を写す形にしたら、1-6 の「符号なし 64 ビットの 2^63 以上は境界で OverflowError」を通らなく
  なった（`U64Box` が黙って負になる）。`from_c_bytes` で確かめるようにした（例題 `cpp_dll_int_widths.ar` が止めた）
- 検査用の `ffi_probe` に `Tagged`（`ArPtrProtocol` のフィールド）・`fp_tagged` / `fp_tagged_sum`・`fp_pairs`（`Pair` の配列）・
  `fp_named_list`（`Named` の配列）を足した
- 例題: `interop/cpp_struct_expand.ar`（期待値と違えば raise）・`cpp_struct_list_error.ar`。`cpp_unsupported_signature_error.ar`
  から使えるようになった関数を外した。単体テスト 2 本（`c_size`・`from_c_bytes`）と生成コード・判定のテストの更新
- 文書: `docs/grammar/09_imports.md` の表
- ゲート（基準 `70eb811`・制限時間つき）: `compare_outputs` の差は新しい例題 2 本と、中継 DLL の作り直しの表示 2 本（負の対照
  でも出る。2 回目の出力は同一。`compare_import_paths` の `cpp_struct_ptr.ar` も同じ）。値返しの構造体を受け取る既存の例題
  （`cpp_dll_struct_by_value.ar` / `cpp_lib_struct_by_value.ar`）は基準と同じ出力。`scan_examples`（基準と同じ）・`force_gate`
  （`VmForceError` 0）・`compare_python_impl` clean（171/171）・`compare_wasm_frontend` 529/529・`type_obligations` 退行なし・
  `stale_doc_refs` 0 件。`cargo test` 1138 件・拡張の wasm クレートの 703 件が通る。VSIX を作り直した

### 7-15 実装

- 型の適合判定（`TypeChecker::type_matches_exact`）: Union の実引数は、**どの構成型も**期待に合えば受ける。以前は期待の
  Union の構成型 1 つで実引数の Union を丸ごと照合していたので、同じ Union にしか渡せなかった（`Option[int]` →
  `Union[int, str, None]`・C の関数が返した `Option[Ptr[Db]]` → `void*` の引数 `Option[Ptr[void]]`）。引数・変数・戻り値の
  すべてがこの関数を通る。構成型の 1 つでも合わなければ従来どおり弾く（`Union[int, str]` → `Option[int]`）
- 4-7 で cpp のスタブの戻り値の型を読むようにしたら、`fp.fp_handle_get(db)`（3-3 の「どの `Ptr` も `void*` へ渡せる」）と
  `ArPtrProtocol` の引数（`Union[Ptr[void], list[Any], bytes, None]`）へ C の戻り値を渡す形が静的エラーになって見つかった
- 参照実装（`impl_python/type_check/type_utils.py::_type_matches`）にも同じ規則を入れた（同じ穴があった・別のコミット）
- 例題: `typing/union_widening.ar`（期待値と違えば raise・4-6 の exe では 5 件の静的エラー）・`union_widening_error.ar`。
  単体テスト 3 本（`union_types.rs`）
- 文書: `docs/grammar/08_type_system.md` の Union・skill `type-checking` の適合判定
- ゲート: 4-7 と合わせて走らせた（4-7 の記録）

### 4-7 実装

- `list` / `bytes` → `ArPtrProtocol` の引数: 呼び出しの間だけ有効なコピーを作る（`copied_protocol_arg`・`value/ptr.rs`）。要素の型は
  中身で決める（int → `"int64"`・float → `"float64"`・C の構造体のインスタンス → 構造体名で `RawLayout::c_size` ずつ並べる・
  bytes → `"uint8"`）。空の list・種類の混ざった list は TypeError（C の側が読む形が決まらない）。flags は書き込み用でなければ
  CONST、解放関数は NULL（Arrow が持つ・D-15）。コピーは型つき経路では後片付け（`PtrArgCleanup::Protocol`）まで、ハンドル
  経路では `STATE.protocol_args` が最も外側の呼び出しを抜けるまで持つ（呼び出しの後で `protocol_done` が書き戻すため）
- 書き戻し（D-14）: `ArPtrProtocol*` へ `mut` の list を渡すと、`address` が変わっていなければ C が書いた中身を list へ戻す
  （`ListWriteBack`。構造体の要素はフィールドへ）。`address` が 0 に書き戻されたら `Ptr` を無効にする規則（4-4）はそのまま。
  書き込み用へ `bytes` を渡すのは TypeError（`bytes` は不変）
- `char*`（非 const）の出力バッファ: 未対応（1-2）から外した。`mut` の `bytes` のコピーを渡し（コールバック 49
  `bytes_buf_arg`）、呼び出しの後で同じ長さの `bytes` として変数へ書き戻す（ハンドル経路のみ。typed 経路には載せない）。
  バッファは `bytes(n)`（n バイトのゼロ・負は ValueError）で用意する
- 型スタブ: `ArPtrProtocol` の引数を `Union[Ptr[void], list[Any], bytes, None]` にした。`char*` の出力バッファと
  `ArPtrProtocol*` は `mut` を求める（`writable_ref`）ので、`let` を渡すと静的エラー。実行時の書き戻しの印（`PtrParam::MutPtr`）は
  実際に書き戻すものだけにした（数値の書き込みポインタ・正確なレイアウトの構造体ポインタ・出力バッファ）。⚠ レイアウトの
  分からない構造体ポインタは `Ptr` を渡すだけで何も書き戻さないので外した
- ⚠ 実装中に見つけた退行（4-3 から）: 型検査が cpp のスタブの戻り値の型をプリミティブしか読まず（`collect_module_types`）、
  `Option[bytes]` / `Option[Ptr[T]]` / 構造体を返す関数の結果が「分からない」になっていた。4-7 で `ArPtrProtocol*` の引数が
  `mut` を求めるようになり、`let h = fp.fp_handle_new()` を渡すと偽の静的エラーになって見つかった（`mut_arg_exempt` は
  `Ptr` を免除するが、型が分からないと効かない）。js-proc と同じく `InferredType::from_ann` で読むようにした（cpp のスタブは
  Arrow が Arrow の綴りで作る）。型が付いたことで、`char*` の戻り値（`Option[bytes]`）を `mustbe` せずに `==` で比べていた
  例題 3 本（`cpp_raw_pointers` / `cpp_struct_expand` / `cpp_typed_pointers`）を直し、より広い Union へ渡せない穴（⇒ 7-15・
  先に直した）も見つかった
- 検査用の `ffi_probe` に `fp_sum64` / `fp_double64` / `fp_sumf` / `fp_pairs_total` / `fp_bytes_sum` を足した
- 例題: `interop/cpp_list_to_c.ar`（期待値と違えば raise）・`cpp_output_buffer_let_error.ar`。`cpp_ptr_protocol_arg_error.ar` は
  list が渡せるようになったので `str` / `int` / `dict` を渡す負例にした。`cpp_unsupported_signature_error.ar` から `fp_fill` を
  外した。単体テスト 3 本（list / bytes / 構造体の list のコピーと書き戻し・出力バッファの生成コード）と判定のテストの更新
- 文書: `docs/grammar/09_imports.md` の表・`docs/ptr_protocol.md`・`docs/grammar/08_type_system.md`（`bytes(n)`）・skill
  `c-abi-interop`・`partial-compile`（コールバック 49）
- ゲート（基準 `7508f34`・7-15 と合わせて・制限時間つき）: `compare_outputs` の差は新しい例題 3 本（`cpp_list_to_c` /
  `cpp_output_buffer_let_error`・7-15 の `union_widening`）・負例にし直した `cpp_ptr_protocol_arg_error`・`cpp_ptr_arg_error`（12 行目の
  `Ptr[void]` → `Db*` が静的エラーとして見つかるようになった。例題の説明どおりで、以前は戻り値の型が分からず素通りしていた）と、
  中継 DLL の作り直しの表示 2 本（負の対照でも出る。2 回目の出力は同一。`compare_import_paths` の `cpp_struct_ptr.ar` も同じ）。
  `mustbe` を足した既存の例題 3 本は基準と同じ出力。`scan_examples`（基準と同じ）・`force_gate`（`VmForceError` 0）・
  `compare_python_impl` clean（174/174）・`compare_wasm_frontend` 533/533・`type_obligations` 退行なし・`stale_doc_refs` 0 件。
  `cargo test` 1144 件・拡張の wasm クレートの 705 件が通る。VSIX を作り直した

### 5-1 実装

- 生成器: `arrow --gen-cs-exports X.dll [-o out.cs]`（`cs_assembly/export_gen.rs`）。マネージドの DLL のメタデータから、
  Arrow が呼ぶ `[UnmanagedCallersOnly]` の export を C# で書く（利用者の判断で Arrow の CLI に置いた）。名前と ABI は
  手書きのときと同じ（`{Class}_new_{個数}` / `{Class}_{name}` / `{Class}_inst_{name}`・整数は `long`・浮動小数は `long` の
  ビット列・文字列は `(byte*, int)`・クラスはハンドル）。生成したクラスは `internal` なので型スタブに出ない
- 型の読み方を 1 つにした: シグネチャのデコーダが C# の型（`CsTy`）を返し、型スタブの綴りは `CsTy::arrow()`（以前の
  綴りと同じ）。アセンブリの歩き方も 1 つにした（`cs_assembly/model.rs::assembly_model`）。⚠ 以前は型スタブと `.ars` が
  それぞれ歩いていた。生成器が 3 本目を持つと export の名前・型がスタブとずれるので、3 者が同じ一覧を読む。`.ars` は
  前後で `__init__` の行だけが変わることを 4 つの DLL で確かめた
- 生成した export は C# の例外と引数の誤りを捕まえ、`arrow_bridge_take_error` に預ける。Arrow は呼び出しのたびに
  引き取って例外にする（`BridgeLib::take_error`・`TypeError` / `ValueError` / `OverflowError` で始まればその例外、ほかは
  `CsDll:`）。⚠ 手書きの export は例外を捕まえておらず、`Calculator.Divide(1.0, 0.0)` でプロセスごと落ちた
- 整数を C# の型の幅で確かめる（`int` へ 2^31 は `OverflowError`。⚠ 手書きの export は `(int)a` で黙って切っていた）。
  `ulong` の戻り値の 2^63 以上も `OverflowError`。`char` は 1 文字の `str`（UTF-16 の 1 単位でなければ `ValueError`・
  文字列は置き換えずに例外にする UTF-8 で読み書き）。`Task<T>` は待って `T` を返す
- 作れないもの（型スタブが未対応としたもの・`object` の引数・`ref` / `out`（⇒ 7-16）・ジェネリック・演算子・抽象クラスの
  コンストラクタ・添字つきの setter・2 つ目以降のオーバーロード）は作らず、理由をコメントで残す
- ⚠ 見つけて直した不具合: cs-dll のコンストラクタは型スタブに引数の型が無く（`__init__(self)` だけ）、値の種類で並べて
  いた。手書きの export は `double` の引数をレジスタ（XMM0）で受けるので、`cs_interop_test.ar` の
  `Calculator(100.0)` は黙って 0 で始まっていた（例題は `expected 125.0` と書いて 25.0 を出していたが、値を確かめて
  いないのでゲートは緑だった）。生成した export は浮動小数を `long` のビット列で受けるので 125.0 になる。さらに型スタブに
  公開のコンストラクタごとの `__init__` を載せ、実行時は引数の個数で選んでその型で並べる（`cs_values::ctor_types`。
  その個数が無ければ `TypeError`。`double` を受けるコンストラクタへ `int` を渡すと、以前は整数のビット列が `double` と
  して読まれ、今は `TypeError`）。型検査は外部言語のコンストラクタの引数を
  見ないので静的な結果は変わらない。未対応の印は全部のコンストラクタが未対応のときだけ付ける（印はメソッドの名前
  ごとに集めるので、1 つに付けるとクラスを構築できなくなる）
- `[UnmanagedCallersOnly]` の付いたメソッド（手書きの export）を型スタブから外した（`NullableInfo::unmanaged_entries`）。
  ⚠ 以前は `ArrowExports` クラスがメソッドごと型スタブに出ていた
- 例題のプロジェクト（`cs_interop_test`）を生成した export に移した: `ArrowExports.cs` を消し、`build_bridge.ps1` を
  2 段（生成物を外したマネージドのビルド → 生成 → NativeAOT）にした。`ArrowBridge.csproj` は `-p:ArrowGenerated=false`
  のとき `ArrowExports.g.cs` を外す。C# に検査用のメンバー（`AddBytes` / `MaxUInt` / `MaxULong` / `AddAsync` /
  `FirstChar` / `RepeatChar`）を足した
- 参照実装（`impl_python/interpreter/cs_dll_runtime.py`）も同じ規則にした（エラーの引き取り・生成したブリッジで
  その個数のコンストラクタが無ければ `TypeError`）
- 例題: `interop/cs_dll_generated.ar`（期待値と違えば raise）・`cs_dll_exception_error.ar`。単体テスト 3 本（`__init__` の
  オーバーロード・`cs_probe.dll` から生成した export・C# の型のデコード）
- 文書: `docs/grammar/09_imports.md`（export の生成・ABI）・skill `importation`・CLAUDE.md のコマンド
- ゲート（基準 `7dae71f`・制限時間つき）: `compare_outputs` の差は新しい例題 2 本だけ（`cs_interop_test.ar` の
  125.0 は基準の exe でも新しいブリッジを読むので一致する）。`compare_import_paths` 13/13・`scan_examples`（基準と同じ）・
  `force_gate`（`VmForceError` 0）・`compare_python_impl` clean（175/175）・`compare_wasm_frontend` 535/535・
  `type_obligations` 退行なし・`stale_doc_refs` 0 件。`cargo test` 1147 件・拡張の wasm クレートの 707 件が通る。
  VSIX を作り直した

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

**C# / Rust のコレクションのメモリ上のイメージ**（2026-10-14・.NET 8 x64 release・rustc release。D-16）

| 型 | 測ったこと | 結果 |
|---|---|---|
| C# `int[]` | 先頭要素とオブジェクトの差・先頭要素の 8 バイト前・オブジェクトの先頭 | 16 バイト・長さ（int32）・型ポインタ（MethodTable）。要素は 4 バイト間隔で連続 |
| C# `int[]` | 固定しない配列の番地は、圧縮する GC の後で変わるか | 20 回中 20 回変わった |
| C# `int[]` | `GCHandle.Alloc(.., Pinned)` で固定した配列 | 20 回中 0 回 |
| C# `List<int>` | 中身 | 内部の配列 `_items`（長さ 4）と `_size`（3）。10 個足すと配列が別物になる（長さ 16） |
| C# `string[]` | 要素 | 文字列オブジェクトへの参照（8 バイト） |
| Rust `Vec<i32>` | 大きさと中身 | 24 バイト（ポインタ・長さ・容量。並びは Rust が定めない）。要素は 4 バイト間隔で連続 |
| Rust `Vec<i32>` | 容量を超えて足す・`Vec` を動かす | バッファが別物になる・バッファは動かない |
| Rust `&[i32]` / `String` | 大きさ | 16 バイト（ポインタ・長さ）/ 24 バイト（要素ごとにポインタ・容量・長さ） |
