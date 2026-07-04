# C ABI 相互運用 — 設計仕様と実装フェーズ

外部言語（C/C++ DLL）との値・構造体受け渡しを、AST 作成時の型解決に基づいて
ゼロコピー（または最小コピー）で行うための確定仕様。

## 確定仕様（ユーザー決定事項）

1. **C ABI 準拠型の導入**: `int8/int16/int32/int64/uint8/uint16/uint32/uint64/float32/float64`
   を型注釈として導入する。
2. **準拠クラスの直接ポインタ渡し**: クラス構造が AST 作成時に解決済みで、全メンバが
   C ABI 型で記述されている場合、そのインスタンスはポインタ（raw ブロック先頭 = `ptr+8`）で
   外部関数に直接渡せる。**対象は C 側がポインタ渡し・参照渡しの関数のみ**。
3. **非準拠クラスの変換渡し**: クラス構造は解決済みだが C ABI 準拠でない場合
   （例: フィールドが Arrow `int` = i64 で C 側は `int` = i32）、AST 展開時に
   **C ABI 準拠のシャドウクラスを生成**し、インスタンス変換 → ポインタ渡しする。
   このオーバーヘッドは許容する。変換不能なフィールド（str, list 等）を含むクラスは
   コンパイルエラー。
4. **write-back**: C 側関数がミュータブルポインタ（`T*`）かつ渡す変数が `mut` の場合、
   関数を抜けた後にシャドウインスタンスを読み戻して元変数に反映する。AST 作成時に展開。
   オーバーヘッド許容。（準拠クラスの直接渡しでは C の書き込みが raw ブロックに
   直接反映されるため write-back 不要 = ゼロコピー）
5. **フィールド順序**: 宣言順。**継承（trait）フィールドは継承順で先頭に配置**し、
   own フィールドがその後に続く。
6. **C/C++ クラスの取り込み**: C ABI に準拠して Arrow クラスへ読み替え可能とする。
   friend 等の機能を持たないシンプルな構造体/クラス（standard-layout かつ
   trivially-copyable 相当）に限定。仮想関数（vtable ポインタ）を持つクラスは対象外。
7. **Arrow 内部は従来どおり typed ABI**: int32 への切り詰め等は行わない
   （Arrow 内の実行時値は常に i64/f64）。C ABI 型はフィールド格納時・境界通過時のみ
   幅変換される（blittable モデル: 書き込み時変換、渡す時はポインタのみ）。
8. **プリミティブはコピー後に直接渡す**（値がスロット/レジスタに乗る — 常に安全）。

## 設計上の解決事項

### C ABI 型の意味論（storage 型方式）
`int32` 等は**独立した実行時値型ではない**。実行時値は従来どおり `Value::Int`(i64) /
`Value::Float`(f64) であり、C ABI 型は以下の場面でのみ意味を持つ:
- クラスフィールド注釈 → raw ブロック内の格納幅・オフセットを決定（書き込み時に切り詰め、
  読み出し時に符号拡張 / f32↔f64 変換）
- 型検査・codegen 上は基底型（int / float）の別名として扱う（`c_abi_base_type()`）

切り詰めは現行 cpp ブリッジの `as i32` と同じ暗黙 truncate を既定とする
（他言語比較: ctypes/LuaJIT/Go と同派閥）。ErrSlot による checked 変換
（OverflowError）への切り替えは将来オプション。

### 値渡し構造体は常に防御コピー
Win64 ABI では 8B 超の by-value 構造体は「呼び出し側が一時コピーを作りポインタを渡す」
規約であり、**callee はその領域を書き換えてよい**。よって raw ブロックを直接渡すと
let/mut を問わずインスタンスが破壊されうる。値渡しは常にスタック一時領域への
memcpy（40–80B で ~2–5ns、C→C の同呼び出しと同額 = Arrow 固有ペナルティなし）。

- `const T*` → raw ブロック直渡し（コピー不要）
- `T*` + mut インスタンス → 直渡し（ゼロコピー write-back）
- `T*` + let インスタンス → **型チェッカーが静的に拒否**
- by-value → 常に一時コピー

### InstanceData raw ブロック（Case C レイアウト）— P1 実装済み
```
Box<[u64]>（8B アラインメント保証）
slot 0      : [class_id: u32][flags: u32]   ← リトルエンディアン u64 パッキング
slot 1..    : フィールド（C ABI レイアウト: 宣言順 + C アラインメント規則）
```
- Arrow コンパイル済みコード → `ptr+0` の class_id でポリモーフィック分岐
- 外部ライブラリ → `ptr+8` を構造体先頭として使用
- `INST_HAS_RAW_LAYOUT` フラグ + `field_init_bitmap`（bits 23-0、最大24スロット）は
  既存の flags 設計をそのまま使用
- アラインメント ≥16 の C 構造体（SSE 型）は対象外（+8 は 8B 境界まで）

**P1 実装詳細**（`src/interpreter/value.rs`）:
- `InstanceData { raw: Box<[u64]>, class, boxed_fields }` — 全インスタンスが raw ブロックを持ち、
  slot 0 がヘッダ。`class_id()` / `flags()` / `set_flags()` アクセサ経由で読み書きする
- `RawLayout::from_fields(&[(name, type_ann)])` — 宣言順 + C アラインメント規則で
  `RawFieldDesc { byte_offset, width }` を計算（`RawWidth`: I8〜F64）
- **適格条件**（`exec_class_def`）: trait 継承なし + 全フィールドがプリミティブ
  （int/float/C ABI 型）+ ≤24 フィールド。適格クラスのフィールドは raw ブロックに格納され、
  `boxed_fields` は空。非適格クラス・組み込みクラス（例外・enum・new_type ラッパー）は従来の
  boxed 形式のまま
- 統一アクセサ: `field_value(idx)` / `store_field(idx, val, mutable)` /
  `slot_initialized(idx)` / `field_mutable(idx)` — raw / boxed を透過的に処理。
  raw 書き込みは幅変換（int32 切り詰め・f32 縮小等）込み、raw 読み出しは符号拡張込み
- 可変性: raw クラスは `field_mutability_vec` + `INST_IMMUTABLE` フラグで表現
  （boxed のスロット別 bool と同義）。初期化追跡は `field_init_bitmap`
- deep_copy / copy(): raw ブロックは **memcpy 1回**（bench_field_access 4.6s → 3.2s）

### P2 実装詳細（cpp ヘッダ → RawLayout 付き Arrow クラス）

- `CStructDef.complete: bool`（`cpp_bridge/types.rs`）: フィールドリストが C/C++ 側の
  レイアウトメンバを**すべて**含むときのみ `true`。配列・ビットフィールド・ネスト構造体・
  未解決型のスキップ、union、継承付きクラスは `false`
- `CStructDef::raw_layout()`: `complete` かつ全フィールドが幅確定プリミティブ
  （`int`→i32 / `float`→f32 / `double`→f64）のとき `RawLayout` を返す。
  C の `long`（環境依存幅）と `bool`（1B だが既存ミラーは i32 仮定）は対象外
- `header_parser.rs` の simple class 判定（`classify_member_segment`）:
  - `virtual` / `friend` を含むクラスは**構造体リストから除外**（vtable / 非 simple）
  - メソッド宣言・`static`・`typedef`/`using`・ネスト型定義はレイアウト非寄与として無視
  - ビットフィールドは `complete=false`
- **既存バグ修正**: `parse_struct_bodies` が `namespace X { … }` / `extern "C" { … }` を
  不透明ブロックとしてスキップしていた（DxLib.h は全体が `namespace DxLib` のため
  構造体抽出が常に 0 件だった）。スコープブロックには降下するよう修正 →
  DxLib から VECTOR 等 63+ 構造体が Arrow クラス（raw f32/f64/i32 レイアウト付き）として
  登録されるようになった
- 格納は書き込み時変換（blittable モデル）: `VECTOR.x = 0.1` は f32 に縮小されて格納され、
  読み出しは f32 丸め済みの値を返す（C から見える値と常に一致）
- 単体テスト: `header_parser.rs::tests`（13本 — virtual/friend 除外、ビットフィールド、
  継承、union、アラインメント、実 DxLib.h スニペット + 実ファイル）

### Windows LLP64 の注意
C `long` は Windows で 4B、Linux で 8B。現行 `CType::Long → i64` マッピングは
LP64 前提。`long` 戻り値は x64 で上位 32bit がゼロ埋めされるため負値が壊れる —
ラッパーで `as i32 as i64`（符号拡張正規化）が必要（既知の未修正問題）。

### P3/P4 実装詳細（typed ABI へのポインタ引数統合）

P3（codegen 側の直接渡し／シャドウ変換／write-back）と P4（typed ABI への
`AbiTy::Ptr` 追加）は、cpp ブリッジの呼び出し規約が「AST 生成時ではなく
`NativeFnRef` 解決時（ライブラリロード時）に C 関数シグネチャが確定する」
アーキテクチャのため、**両方ともインタープリタ側（Rust）のみで実装**した
（LLVM codegen 側の変更は不要 — Arrow 内部の typed ABI は従来どおり）。

**`AbiTy::Ptr { mutable, by_value, layout: Arc<RawLayout> }`**（`value.rs`）:
- `layout` は `Arc`（`Rc` ではない）— `NativeFnRef` が AST の `NativeCallCache`
  （`Arc<dyn Any + Send + Sync>`）に格納されるため `Send + Sync` が要求される
- `mutable`: C 側が `T*`（非 const）で書き込みうるか。`by_value` が真のときは無関係
- `by_value`: by-value 構造体引数。C の値渡し意味論上、呼び出し先は常に独立した
  コピーしか見ないため write-back の対象には決してならない

**適格性判定**（`cpp_bridge/codegen.rs::cpp_typed_eligible` /
`exec.rs::build_cpp_typed_sig` — 両者は一致させること）:
`CType::OpaqueStructPtr`（`VECTOR*` 等）・`CType::ByValueStruct`（`VECTOR` 直接）
のうち、対象の `CStructDef` が `raw_layout().is_some()`（P2 参照）のものだけが対象。
戻り値の構造体（by-value 返却）は非対応（ハンドル経路のまま）。

**Rust ラッパー生成**（`gen_dll_fn_typed`）: ポインタ引数は u64 スロットの値を
そのまま `*mut/*const _Struct_{name}` にキャスト、by-value 引数はそのアドレスから
`*_Struct_{name}` としてデリファレンス・コピーしてから値渡しする
（Rust の Copy セマンティクスが Win64/SysV いずれの by-value 呼び出し規約とも
自動的に整合する — 呼び出し先は常にこの関数のスタック上のコピーだけを見る）。

**解決ロジック**（`value.rs::resolve_typed_ptr_arg` / `finish_ptr_arg_cleanup`
/ `PtrArgCleanup`）— 全呼び出し経路で共有する中核関数:
1. 値が `Value::Instance` でなければ `Ok(None)`（typed 経路を諦めてハンドル経路へ）
2. `mutable && !by_value && named_mut == Some(false)`（`let` 変数を書き込み用
   ポインタへ渡そうとした）なら `Err`（既存のハンドルベース `PtrParam::MutPtr` と
   同じ規則・同じ趣旨のエラー）
3. インスタンスの `class.raw_layout` が対象 `layout` と**構造的に完全一致**
   （`raw_layouts_compatible` — オフセット・幅・個数の Vec 比較）すれば
   **ゼロコピー**: `raw.as_ptr()+8` を直接渡す。書き戻しは不要（同一メモリ）
4. 一致しなければ**シャドウ変換**: フィールドを**宣言順の位置**で対応付けて
   一時バイト列を構築（`build_shadow_raw`）。`T*`（mutable）かつ名前付き
   `mut` 変数のときのみ、呼び出し後にバイト列の内容を元インスタンスへ
   読み戻す（`apply_shadow_raw`）。by-value・const・非名前付き引数は書き戻しなし

**`named_mut: Option<bool>`（呼び出し元の式が named mut/let かの判定）が
利用可能な経路とできない経路**:
| 呼び出し経路 | named_mut 判定 | 備考 |
|---|---|---|
| `call_native_function`（AST 経由、MutPtr パラメータあり） | CallArg から判定 | let 変数は事前チェックループで既にエラー化される |
| `dispatch_native_typed_exprs`（インラインキャッシュ経路） | CallArg から判定 | |
| `dispatch_native_evaled`（**const-only ポインタ関数の主経路**） | 常に `None` | CallArg 情報がない層。write-back 条件 `named_mut==Some(true)` を満たせないため常に安全側（書き戻ししない）に倒れる |
| `ar_call_fn`（ネイティブ→C、コンパイル済み Arrow から） | 常に `None` | アリーナハンドルのみで named-variable の概念がない層 |

**重要な考慮点（実装中に発覚）**: `has_writeback`（`fn_ref.ptr_params` に
`MutPtr` が1つでもあるか）が false の関数（= 全パラメータが const ポインタ、
例: `VectorInnerProduct(const VECTOR*, const VECTOR*)`）は `call_native_function`
から直接 `dispatch_native_evaled` に渡る。当初この経路には Ptr 対応を入れて
いなかったため、const-only 構造体ポインタ関数が**既存の壊れたハンドル経路**
（`OpaqueStructPtr::from_handle` がアリーナハンドルの値をそのままポインタとして
ビットキャストする — 実際のメモリアドレスではないため確実にクラッシュする）に
フォールバックしてしまい、`VectorInnerProduct` 呼び出しが access violation
（0xC0000005）を起こした。`dispatch_native_evaled` にも同じ Ptr 解決ロジック
（named_mut=None）を追加することで解決。**cpp ブリッジで構造体ポインタ引数を
扱う場合は、const-only 関数が必ずこの経路を通ることを踏まえて実装すること**。

また `sig_to_ptr_param_fn`（`exec.rs`）を拡張し、`OpaqueStructPtr` も通常の
`Ptr` と同じ規則で `PtrParam::MutPtr`/`ConstPtr` に分類するようにした。これにより
mutable 構造体ポインタを持つ関数が正しく write-back パス（→ typed Ptr 高速経路）
に入るようになる（以前は `OpaqueStructPtr` が `PtrParam::None` 扱いで
`has_writeback` が常に false になり、構造体の書き込みは考慮されていなかった）。

**検証**（DxLib VECTOR / `VectorAdd` / `VectorScale` / `VectorInnerProduct` /
`DrawPixel3D` で確認、テストスクリプトは検証後削除）:
- ゼロコピー直接渡し（mut 出力・const 入力）— 正しい書き戻しを確認
- const-only 関数（`VectorInnerProduct`）— 修正後に正常動作（140.0 = 内積の正解値）
- by-value 構造体引数（`DrawPixel3D`）— 正常呼び出し
- `let` 変数を mutable pointer へ渡す → `TypeError` で正しく拒否
- シャドウ変換（Arrow 独自クラス `MyVec`（float=f64 フィールド）を VECTOR
  期待関数へ渡す）— mut 変数への書き戻し・const の非書き戻しとも正しく動作

### P5 実装詳細（`T*` + let の静的拒否）

書き込み用ポインタ（`T*` / `VECTOR*` 等）へ不変（`let`）変数を渡す誤りを、
**実行時 TypeError から型検査時の静的エラーへ前倒し**した。既存の型検査機構
（`CallMutParamWithImmutableArg` — `mut` パラメータへ不変値を渡した検査）を
そのまま再利用しており、追加の検査ロジックは不要。

**変更点は 2 つ**（いずれも `src/parser/imports.rs::parse_cpp_import`）:

1. **cpp スタブの mutability マーキング拡張**: パーサが型検査用に生成する
   `Stmt::FnDef` スタブで、従来は `CType::Ptr { mutable: true }`（`int*` 等）のみを
   `mut` パラメータとしていたが、`CType::OpaqueStructPtr { mutable: true }`
   （`VECTOR*` 等の構造体ポインタ）も含めるよう修正。これにより型チェッカーが
   `dx.VectorAdd(let_var, ...)` を静的に `StaticTypeError` として報告する
   （メッセージ: `parameter 'Out' of 'VectorAdd' expects a mutable argument, ...`）。
   `const T*`（`OpaqueStructPtr { mutable: false }`）・by-value 構造体は
   `mut` にしないため、`let` 変数を渡しても正しく通る。

2. **非 UTF-8 ヘッダの読み込み修正（重要な副次バグ）**: `parse_cpp_import` は
   従来 `read_to_string`（厳格 UTF-8）でヘッダを読んでいたが、DxLib.h は
   Shift-JIS の日本語コメントを含むため読み込みが失敗し、**型検査用 body が
   常に空**になっていた（＝ cpp 関数呼び出しの静的型検査が完全にスキップされていた）。
   `read` + `from_utf8_lossy` に変更（実行時の `load_cpp_module` と一致）することで、
   非 UTF-8 ヘッダでも関数スタブが生成され、型検査が有効になる。この修正により
   P5 の静的検査だけでなく、cpp 関数呼び出し全般（引数個数・型）の静的検査も
   初めて機能するようになった。既存 DxLib サンプル 6 本は全て静的エラーなしを確認済み。

**cpp-lib/cpp-dll import の実行時は body を使わない**（`load_cpp_module` が
libloading で解決）ため、スタブの mutability 変更は実行時挙動に影響しない
（型検査専用）。

**動作意味論の差**: 静的検査は「名前付き不変変数」だけでなく一時式
（`VectorAdd(VECTOR(0,0,0), ...)`）も拒否する（`is_mutable_expr` が `Ident` のみ真）。
一方で実行時 `resolve_typed_ptr_arg` は名前付き `let` 変数（`named_mut==Some(false)`）
のみを拒否し一時式は許容する。この差は Arrow 通常関数の `mut` パラメータと同じ
挙動であり、出力ポインタへ一時式を渡すのは無意味（結果が破棄される）なため
静的拒否はむしろ有益。

**例**: `examples/cpp_struct_ptr_error.ar`（+ `examples/test_modules/vec_math.h`）
— `let` インスタンスを `v3_add(V3* out, ...)` へ渡して静的エラーになる自己完結例
（静的検査で停止するため DLL コンパイル不要、MSVC/clang なしで再現可能）。

## 実装フェーズ

| フェーズ | 内容 | 状態 |
|---|---|---|
| **P0a** | C ABI 型注釈の受理（type_check / interpreter / codegen で基底型に解決） | ✅ 実装済み |
| **P0b** | フィールド順序: trait 継承順先頭 + own 宣言順（`build_field_index`） | ✅ 実装済み |
| **P0c** | フラットレイアウトの宣言順統一（`collect_flat_leaves` / `FlatLayout` のアルファベット順ソート除去）。**既存 .arc は要再コンパイル** | ✅ 実装済み |
| **P1** | InstanceData raw ブロック化（`Box<[u64]>` + RawLayout 記述子 + アクセサ書き換え） | ✅ 実装済み |
| **P2** | cpp ヘッダパーサの C++ simple class 対応 + CStructDef → Arrow クラス生成（RawLayout 付き） | ✅ 実装済み |
| **P3** | codegen: 準拠クラスの直接ポインタ渡し / by-value 防御コピー / シャドウクラス変換 + write-back（インタープリタ側で実装、LLVM codegen 変更は不要） | ✅ 実装済み |
| **P4** | typed ABI スロットへのポインタ型追加（`AbiTy::Ptr`）+ 全 dispatch 経路（`call_native_function` / `dispatch_native_typed_exprs` / `dispatch_native_evaled` / `ar_call_fn`）対応 | ✅ 実装済み |
| **P5** | `T*` + let の静的拒否を型チェッカーへ前倒し（`OpaqueStructPtr` mutability マーキング + 非 UTF-8 ヘッダの lossy 読み込み修正）。checked 変換オプションは将来検討（現状は暗黙 truncate を維持） | ✅ 実装済み |
