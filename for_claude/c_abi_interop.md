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

## 実装フェーズ

| フェーズ | 内容 | 状態 |
|---|---|---|
| **P0a** | C ABI 型注釈の受理（type_check / interpreter / codegen で基底型に解決） | ✅ 実装済み |
| **P0b** | フィールド順序: trait 継承順先頭 + own 宣言順（`build_field_index`） | ✅ 実装済み |
| **P0c** | フラットレイアウトの宣言順統一（`collect_flat_leaves` / `FlatLayout` のアルファベット順ソート除去）。**既存 .arc は要再コンパイル** | ✅ 実装済み |
| **P1** | InstanceData raw ブロック化（`Box<[u64]>` + RawLayout 記述子 + アクセサ書き換え） | ✅ 実装済み |
| **P2** | cpp ヘッダパーサの C++ simple class 対応 + CStructDef → Arrow クラス生成（RawLayout 付き） | ✅ 実装済み |
| P3 | codegen: 準拠クラスの直接ポインタ渡し / by-value 防御コピー / シャドウクラス変換 + write-back の AST 展開 | 未着手 |
| P4 | typed ABI スロットへのポインタ型追加（AbiTy::Ptr）+ dispatch / ar_call_fn 対応 | 未着手 |
| P5 | `T*` + let の静的拒否（型チェッカー） / checked 変換オプション | 未着手 |
