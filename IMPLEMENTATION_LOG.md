# 実装ログ — 完了した作業の詳細記録

[BYTECODE_VM_PLAN.md](BYTECODE_VM_PLAN.md) から切り出した**実装済み内容の詳細**。

> **通常の実装作業では読む必要はない。** 計画書側に各項目 1〜2 行の要約があり、
> 「なぜそう作ったか」「何を測ってどう判断したか」を遡りたいときだけここを見る。
>
> 特に価値があるのは **見積もりが外れた記録**（下記 #16 c-2 / c-3 / (b)(i) など）。
> 同じ轍を踏まないために残してある。

---

## 完了済みタスク一覧（詳細）

### Phase R / Phase V の完了項目
1. **Phase R** — R0 フレーム隔離（`frame_floor`）／ R1 ローカル slot 化（`LocalRef`）／ R3 属性 IC（`AttrCache`）／
   R4 呼び先解決 ／ メソッド IC ／ §7.4-2 `Value` サイズ削減（`JsProcFn` を Box 化・72→32B）。
2. **Phase V-A** — VM 骨格（op/chunk/run/disasm）・算術・ローカル slot・制御フロー（if/while）・関数呼び出し・ローカル宣言。
3. **Phase V-B** — メソッド本体の VM 化（`self` 対応）・属性書き込み `SET_ATTR`。
4. **Phase V-C** — break/continue ジャンプ ／ ネスト局所の平坦 slot 化 ／ match 文 ／
   **例外テーブル**（try/except/finally・raise・ハンドラスタック）／ **ブロック式**（block:/if/while/for/match 式 + block_return/loop_yield）。
5. **Phase V-D** — for ループ（GET_ITER/FOR_ITER）／ 組み込み print/range/len（`CallBuiltin`）／ **Chunk キャッシュ健全化**（`Weak<FnValue>` キー）。
6. **Phase V-E（実利部分）** — 呼び出し位置 span 伝搬でトレースバック行番号を復元（**全決定的例が off/auto byte-identical**）／
   デバッグ名テーブル（`Chunk.local_names`）／ **デバッガ REPL のバイトコード実行**（停止スコープ名前引き `LoadName`/`DeclareName`・フォールバック付き）。
7. **#5 添字・コレクションリテラル** — `obj[i]` 読み書き（`Subscript`/`SetIndex`）・list/tuple/set/dict リテラル
   （`BuildList`/`BuildTuple`/`BuildSet`/`BuildDict`）を VM 化。`for` 変数が外側をシャドウする関数は bail（flat-slot 非対応）。
8. **#4 メソッド呼び出し機構の軽量化** — VM 呼び出しの高速バインド経路（単純シグネチャは `bind_args` の
   Vec 確保・名前 clone・`params.clone()`・copy/cast パスを飛ばし直接バッファへ束縛）。method_hot 1.13→1.61x・
   method_body 1.22→1.64x。コピー意味論は不変（self deep_copy 省略は escape 健全性のため不採用）。
9. **#6 その他組み込みの VM 化** — 純粋組み込み `next`/`repr`/`id`/`enumerate`/`zip`/`getenv` を `CallBuiltin`
   （`eval_builtin_evaled`・enumerate/zip はコア共有）、登録済み型コンストラクタ（int/str/…）を通常の
   `LoadGlobal`+`Call`（`call_type_by_name_evaled` へ委譲・グローバル shadow に健全）で対象化。enumerate/zip 支配 1.49x。
10. **#7 テンプレート実体化の Chunk メモ化** — `(テンプレート, 型引数)` をキーに具体 `FnValue`/`GeneratorFnValue`
    をメモ化（`subst_stmts` の clone-walk と Chunk 再コンパイルを初回のみに）。テンプレート関数支配で auto 5.9x
    （従来は毎コール再コンパイルで VM がツリーウォークより遅い病理ケースだった）。テンプレートクラスは対象外（副作用）。
11. **#8 ジェネレータ本体の VM 化** — `Yield` op で本体をバイトコード実行し `yield` を `GENERATOR_YIELDS` へ
    eager 収集（意味論不変）。`vm_gen_chunks` キャッシュ。付随して `call_value_evaled` の `GeneratorFn` 呼び出し
    ギャップ（VM 関数から `gen()` を呼ぶと "not callable")も修正。generator 支配で ~3.2x。`Self`/クロージャは bail。
12. **#9 async の VM 対応（関数内）** — `AsyncSubmit` op ＋ `Chunk.async_blocks`。捕捉集合を「本体の参照名 ∩
    frame slot」に限定し、frame から値を読んで**一時スコープ経由で既存 `capture_env` を再利用**して env を組む
    （ツリーウォークと同一・D5 share-nothing 維持）。VM コンパイル済み関数内で `mng <- async` が使える。
13. **#16 AST 型解決層 — 段階(a)＋(b)第1増分**（2026-08-03〜05・コミット `#13-1`〜`#13-5`）— 型検査が全式の型を **node-id 索引の
    注釈テーブル**（解決型/検査指示/型インターン/CallInfo/binop_kind）に永続化（MustBe/BinOp/Attr/Subscript/Cast/IsType/Call。
    Ident はスキップ）。`check_program`→`interp.set_annotations` でランタイム配線。**plan A 第1増分**として VM が `binop_kind` を消費し
    **型特化二項演算 op**（IntBinLL 等・参照読み＋ディスパッチ省略・想定外型はフォールバックで健全）を emit → **int 1.30x / float 1.36x**。
    詳細と残り・git 状態は **#16 の「段階と実装状況」節**（本文後半）に集約。
14. **#16 完了**（2026-08-10）— 段階(a) 注釈永続化 → (b)(i)(ii)(iii) VM 消費 → (c-1)(c-2)(c-3) ネイティブ消費
    → D 型検査の解像度 → E テンプレート → F モジュール横断、＋ FFI 境界検査。
    **三経路（ツリーウォーク／VM／ネイティブ）が同一の型解決注釈を消費する**状態に到達。
    主な実測: ネイティブ `flat_bench` **1.73x**／属性オペランドの二項演算 **1.379x**（`GetAttrLocal`）／
    int div/mod/bit **1.478x**／`mustbe` を含む関数 **2.04x**（bail していたものが VM 化）／
    `for i in range(n)` **1.153x**。
15. **#18 順序比較の食い違い解消**（2026-08-10）— 型検査が許可していた `(int,float)` 混在の `<=`/`>=` と
    `str` 同士の 4 演算子が実行時に未実装で TypeError だったのを、実行時側を検査器に合わせて解消。
16. **#11 R2-a / R2-a′ / R2-b**（2026-08-11）— 「AST 展開の共通化」の観点で実施。
    ネイティブが R1 の解決済み AST を消費（`ident_name` で 28 箇所を両変種対応・`--compile` にリゾルバ追加）、
    さらに **slot 索引でローカルを管理**（`harvest_local_slots` がリゾルバの割り当てを収穫・codegen は採番しない）。
    グローバルは `Expr::GlobalRef { name, cache }` を新設しツリーウォークと VM が同一ノードを共有。
17. **#14 の一部**（2026-08-11）— `.arc` の陳腐化検査。埋め込みソースと隣の `.ar` を突き合わせ、
    食い違えば警告してソース側を使う。**リポジトリの `.arc` が実際に 3 件古く、ずっと古いコードを実行していた**。

---

## #16 AST 型解決層 — 三経路の挙動統一（完了 2026-08-10）

**【2026-08-10 ひと段落】** 段階(a) 注釈永続化／(b)(i)(ii)(iii) VM 消費／(c-1)(c-2) ネイティブ配線＋実測／
型検査の解像度向上（for 要素型・`infer_attr`）／FFI 境界検査まで完了。残りは下記「⬜ 残り」節。
型検査器が既に全式ぶん計算している型（`infer(&Expr)->InferredType`・[infer.rs:10](src/type_check/infer.rs#L10)）を
**node-id 別テーブルへ焼き込み**、可能な限り低レベルまで解決する（具象型・メソッド/フィールドのバイトオフセット・
呼び出しシグネチャ・検査要否）。解決点は決め打ち（直接オフセット/直接ディスパッチ・検査なし）、未解決点のみ検査指示を付す。

### 目的（速度ではない・承知の上）
(1) コンパイル時とツリーウォーク時の**挙動統一**。(2) バイトコード化・各最適化で生じがちな「型が確定しているか/どの経路か」
等の**例外的な条件分岐を AST 段階で解消**（各経路が単純化）。速度貢献は現段階で限定的（VM は R3/メソッド IC が緩衝。#3 実測:
overloaded 演算は「探索」~2-3% のみ解決可能・残りは呼び出し機構＋確保で不可避。**ネイティブは IC が無いぶん効果が桁違い**）。

### 注釈モデル（採用: node-id ＋ 2直交側テーブル ＋ 型インターン表）
- **node-id**: annotatable な Expr（`Call`/`Attr`/`BinOp`/`Cast`/`MustBe`/`IsType`/`Subscript`）にパース時
  `node_id: u32` を採番（**C1: グローバル採番**）。**非テンプレ関数から着手**（R1 と同じ段階戦略）。テンプレは注釈が型変数 `T`
  型のまま＝実体化時に型 subst が要るため**次段へ分離**。async はクローン AST だが注釈は**読み取り専用**なので同一 node-id 参照で安全。
- **`Ident`/`LocalRef` は node-id を付けない（採用 2026-08-03）**。理由: `Ident` は葉の名前参照だが、その**型は必ず消費側
  ノード（`BinOp`/`Call`/`Attr` 等）が推論過程で既に保持**する（narrowing 済み）ので個別注釈は冗長。加えて `Ident` は
  **タプル変種で 97 箇所**に及び node-id 化は極めて侵襲的。→ 型は消費側で捕捉する。
  **⚠️ 別途の再検討事項**: `Ident` の使用箇所が多いこと自体（タプル変種 `Ident(String)`・97 サイト）は、将来「Ident を
  構造体変種化して node-id/解決情報を持たせる」等の**AST 表現の再設計**の検討対象。本タスク #16 の範囲では扱わず、独立に評価する。
- **2直交テーブル（node-id 索引）**:
  1. **解決型テーブル** `node_id → 型idx`（具象 or `Dynamic`）。**型は per-module「型インターン表」への index**（`InferredType` を
     インライン展開せず index 化 ＝ AST 軽量・比較高速・**ネイティブの型記述子テーブル生成と直結**）。
  2. **検査指示テーブル** `node_id → 指示`（`None` / `CheckBefore(型idx)`）。境界・`mustbe`・`cast` の動的検査を表す。
- **呼び出し(Call)注釈**（点1・「解決済み CALL 注釈が引数検査指示を持つ」に**統合**）: `{ 呼び先=シンボル参照(名前＋解決index＋
  シグネチャ型idx), 各引数=(引数node_id, 型idx, 検査指示) }`。**生ポインタは持たず**「シンボル参照」（R4 の呼び先解決機構を再利用。
  各バックエンドが自前の関数ポインタ/シンボルへ写像）。
- 属性/メソッド: 解決時 `(クラス型idx, フィールド byte-offset / メソッド slot)`、未解決時 `Dynamic`。BinOp: `(左型idx, 右型idx, 結果型idx)`。

### 検査要否の決定（点2/3・調査で確定）
- **要検査 → `CheckBefore`**: `Expr::MustBe`（不一致で raise・[ast.rs:538](src/ast.rs#L538)）／`Expr::Cast`（`__cast__`/コンストラクタ
  ディスパッチ・[ast.rs:513](src/ast.rs#L513)）／他言語 FFI 境界／**非コンパイル Arrow ライブラリ境界**（コンパイル済みは typed ABI 保証で無検査）／
  型が `Any`/`Protocol`/`Union`/`Unresolved` の消費点。
- **無検査（点3）は自動的に満たされる**: 型ガードは [check.rs:376](src/type_check/stmt/check.rs#L376) が**絞り込んだ型で変数を分岐スコープに
  再宣言**して実現。したがって**分岐内の各出現で `infer` は絞り込み済み具象型を返す**＝そこへ焼けば自然に無タグ。**別途 narrowing 抽出は不要**。

### 消費者（三経路が同一注釈を消費）
- **ツリーウォーク**（`eval`）: 注釈があれば直接オフセット/直接ディスパッチ、`CheckBefore` があれば動的検査。
- **バイトコード**（plan A）: 解決点は特化 op、`CheckBefore` は検査 op。**`Value` は boxed 維持**（内省保持・unbox=解釈B は非採用）。
- **ネイティブ codegen**（#13）: `context.rs` の独自再導出を**この注釈の消費に置換**。typed 機械語を生成し**不要な型情報は消去**。
  **境界の動的検査は CALL 注釈の引数検査指示＋型インターン表から呼び出し前にインライン生成**
  （＝点4「ライブラリ内テーブルで呼び出し前に検査」を**この機構に畳み込み・別機構は不要＝削除**）。

### 入口コスト（調査済み・#1）
型は既に `infer` で計算済みだが `&Expr` を取り**破棄**、`check` はエラーのみ返す（書き戻し 0 件）。第一歩は
**「型検査の走査中に node-id テーブルへ型＋検査指示を書き込む」**（narrowing はスコープ再宣言で `infer` に既に反映済み＝
検査走査中に書くのが最適）。型推論の再実装は不要＝中規模。

### 段階と実装状況（2026-08-05 更新・**コンテクストなし再開用ハンドオフ**）
段階: **(a) 注釈永続化層** → **(b) ツリーウォーク/VM が消費（plan A）** → **(c) ネイティブ codegen が消費（#13）**。
関係: R1/R3/R4 の解決注釈を**型情報まで拡張・一本化**。#13 を包含し、plan A と #11 resolve-time R2 はこの層の**消費側**。

### ✅ 段階(a) 完了（注釈生成＋ランタイム配線）
- **注釈基盤**: [src/type_check/annotations.rs](src/type_check/annotations.rs)（新規）。`AstAnnotations` が
  node-id 索引の **① 解決型テーブル（`resolved`）・② 検査指示テーブル（`directives`: `None`/`CheckBefore(TypeId)`）**
  ＋ **③ 型インターン表（`intern`: `TypeId`→`InferredType`）** ＋ **④ Call 構造化表（`calls`: `CallInfo{callee, args:[ArgAnnotation{ty,directive}]}`）**
  ＋ **⑤ 二項演算オペランド種別（`binop_kind`: `BinOperandKind::Int/Float`）** を持つ。公開型: `AstAnnotations`/`TypeId`/
  `Directive`/`CallInfo`/`ArgAnnotation`/`BinOperandKind`（[mod.rs](src/type_check/mod.rs) で re-export）。
- **node-id**: パーサ（[src/parser/mod.rs](src/parser/mod.rs) の `node_counter`＋`next_node_id()`）が **per-module で 1 始まり採番**。
  annotatable な Expr 構造体変種に `node_id: u32` フィールドを追加済み: **`MustBe`/`BinOp`/`Attr`/`Subscript`/`Cast`/`IsType`/`Call`**
  （[src/ast.rs](src/ast.rs)）。テンプレ subst はコピー・py-converter/合成コードは `0`（＝未採番・注釈対象外）。
  **`Ident`/`LocalRef` は node-id を付けない**（下記「別途再検討事項」参照。型は消費側で捕捉）。
- **型検査での充填**: [src/type_check/infer.rs](src/type_check/infer.rs) の各 arm と
  [src/type_check/call_check.rs](src/type_check/call_check.rs) の `infer_call`/`infer_call_inner`（`node_id` を通した）が
  走査中に焼く。ノード別: MustBe/Cast=解決型＋`CheckBefore`／BinOp=結果型＋（int/int・float/float なら）`binop_kind`／
  Attr=フィールド型（**registry の `class_field_details` から実型を引く**・infer 戻り値は不変で下流無影響）／Subscript=要素型／
  IsType=`Bool`／Call=結果型＋`CallInfo`。**Call の引数検査指示**: 直接関数(単一sig)・関数型変数・インスタンスメソッド(単一sig・非static)で、
  param 具象×arg 動的(`Any`/`Unresolved`)のとき `CheckBefore(param型)`。overload/static/キーワード可変長は保守的 `None`。
- **ランタイム配線**: `TypeChecker::check_program(stmts) -> (errors, warnings, AstAnnotations)`（[mod.rs](src/type_check/mod.rs)・
  旧 `check_with_warnings` は削除）。[src/main.rs](src/main.rs) が生成 → `interp.set_annotations(Rc::new(ann))`。
  [Interpreter](src/interpreter.rs) が `pub(crate) annotations: Rc<AstAnnotations>`（既定空）を保持。crate 全体から
  `self.annotations.resolved_type(node_id)`/`.directive(node_id)`/`.call_info(node_id)`/`.binop_kind(node_id)` で参照可。
- **テスト**: [src/frontend_tests/type_check_tests/annotations.rs](src/frontend_tests/type_check_tests/annotations.rs)（14件・パイプライン検証）。
- **不変条件**: 注釈生成はランタイム挙動に無影響（`infer` の戻り値・エラー出力を変えない）＝**off/auto byte-identical 維持**。

### ✅ 段階(b) 第1増分 完了（plan A: 型特化二項演算）
- **注釈駆動の型特化 op**: `binop_kind` が int/int・float/float の二項演算（Add/Sub/Mul・比較のみ・Div/Mod/Pow/bit は汎用）を
  **`IntBinLL`/`IntBinLC`/`FloatBinLL`/`FloatBinLC`**（[src/vm/op.rs](src/vm/op.rs)・[run.rs](src/vm/run.rs)・[disasm.rs](src/vm/disasm.rs)）へ落とす。
  **オペランドを clone せず参照読み**＋op ディスパッチ省略。`Value` は **boxed 維持**（内省保持・unbox=解釈B は不採用）。
- **配線**: `compile_fn(params, body, annotations: Rc<AstAnnotations>)`（[src/vm/compiler.rs](src/vm/compiler.rs)・`Compiler.annotations`）。
  呼び元 `get_or_compile_chunk`/`get_or_compile_gen_chunk`（[execution.rs](src/interpreter/functions/execution.rs)）が `self.annotations.clone()` を渡す。
  `try_emit_bin_fused(.., node_id)` が `binop_kind` を見て特化 op を emit（#2 の superinstruction を拡張）。
- **健全性**: 特化 op は**実行時型が想定外なら汎用 `apply_bin_fast` へフォールバック**。よって注釈が古く/衝突していても**結果不変**
  （モジュール横断の node-id 衝突は perf の無駄フォールバックのみ・正しさは保たれる）。
- **実測（release・best-of-3・同一ビルドで特化 on/off を A/B）**: int 算術ループ **1.30x**（5994→4613ms）／pure float **1.36x**（5661→4148ms）／
  呼び出し支配の float は 1.04x（希釈）。効いた主因: **32B `Value` クローン2回の参照読み回避** ＋ op ディスパッチ省略。

### ✅ 段階(c-1)/(c-2) 完了（ネイティブ codegen への注釈配線＋実測）— 2026-08-10
- **c-1 配線**: `--compile` が `TypeChecker::check_and_annotate` を使い、注釈を
  `partial_compiler::compile` → `compile_native` → `generate_llvm_module` → `GenCtx.annotations`
  （[mod.rs](src/partial_compiler/llvm_codegen/mod.rs)・[context.rs](src/partial_compiler/llvm_codegen/context.rs)）へ渡す。
  **node-id 空間の一致を確認済み**: import 済みモジュール body は `Stmt::Import{body}` に入れ子で、
  型検査の注釈充填（`collect_module_types` は署名のみ読む）も codegen（トップレベル定義のみ走査）も踏み込まない
  ＝ `--compile` 対象モジュールのパーサ採番と 1:1。テンプレートは `llvm_codegen` が非対応なので
  「subst が node_id を複製する」問題の影響外（**ネイティブは VM と違いフォールバックが無い**ため、この確認が前提条件）。
- **c-2 実測（注釈は消費せず、自前導出と突き合わせるだけ）**: `AR_ANNOT_DIFF=1` で内訳を出力
  （[annot_diff.ps1](annot_diff.ps1)）。対象 6 モジュール（physics / swd_nested / typed_abi_module /
  geometry / flat_bench_module / partial_call_overhead_module）で **IR は全て byte-identical**
  （[dump_native_ir.ps1](dump_native_ir.ps1) ＋ `AR_DUMP_LL` フック）。

### 🔍 c-2 の結論（**プランの前提が実測で覆った・要判断**）
1. **`GenCtx::field_ty` は実質デッドコードだった**。属性読みの大半は関数入口の **preread 高速パス**
   （[function.rs:72-112](src/partial_compiler/llvm_codegen/function.rs#L72)）が処理し、`field_ty` に到達するのは
   「**本体が書き換えるクラス param 上の読み**」だけ。実測 6 モジュールで到達は 7 件のみ（うち解決成功 0 件）。
   → §4.4 が名指しした置換対象（`field_ty`/`param_classes` の自前再導出）を注釈へ置き換えても**得るものがない**。
2. **注釈テーブル自体は充填されている**（physics: `resolved=117` / `interned=4`）。空ではない。
3. **codegen が `Ty::Handle` へ落ちた式のうち、注釈が具象型を持つものは実測ほぼ 0**
   （6 モジュール合計で `call=2` のみ）。＝ 現状の注釈は codegen の適用範囲を広げられない。
4. **根本原因は型検査側の解像度不足**（codegen 側ではない）。決定的な例が `flat_bench_module.compute_mut`:
   `for p in pts:` の `p.x` 7 箇所が**自前導出・注釈ともに未解決**。理由は
   **型検査が for ループのターゲットを `InferredType::Unresolved` で宣言している**こと
   （[check.rs:99-101](src/type_check/stmt/check.rs#L99)・`let` 局所は `check_var_decl` で推論済みなのに for だけ落ちている）。
   受け手が `NamedInstance` に解決されないため `infer_attr` がフィールド型を焼けない。
→ **段階(c-3)「自前再導出を撤去して注釈へ移行」は、現状のまま実施しても効果ゼロ**。
  先に **for ターゲットの要素型推論**（`ListOf/FixedListOf/SetOf/DictOf/Tuple` の要素型で宣言）を入れるのが前提。
  これは型検査の**意味論変更**（今まで `Unresolved` で素通りしていた箇所に新規の静的エラーが出うる）なので、
  独立タスクとして判断を要する。VM 経路（`binop_kind`）にも同時に効く。

### ✅ `infer_attr` 戻り値の実型化 ＋ 段階(b)(iii) スタック版型特化 op（2026-08-10）
- **`infer_attr` 戻り値の実型化**（[infer.rs:333-355](src/type_check/infer.rs#L333)）: `NamedInstance` のフィールドは
  registry から引いた実型を**戻り値としても返す**（従来は注釈にだけ焼き戻り値は `Unresolved`）。
  これで `p.x * p.x` が Float×Float と判定され `binop_kind` が付く。
  **単体では速度効果ゼロ**（bench_field_access 0.98x＝誤差）。理由は下記。
- **段階(b)(iii)**（[op.rs](src/vm/op.rs)・[run.rs](src/vm/run.rs)・[compiler.rs](src/vm/compiler.rs)）:
  従来の型特化 op は `IntBinLL`/`FloatBinLC` など**超命令融合（`local <op> local` / `local <op> const`）専用**で、
  `try_emit_bin_fused` が `as_local(left)` に失敗すると即 `false` を返すため、
  **属性・添字・呼び出し結果をオペランドに持つ式には特化が乗らなかった**。
  → スタック上の2値を参照で見る **`IntBinSS`/`FloatBinSS`** を追加し、融合できない形でも
  `binop_kind` があれば特化 op に落とす。判定は `specialized_bin_kind` に共通化（Div/Mod/Pow/bit は従来どおり汎用）。
  想定外の実行時型は既存同様 `apply_bin_fast` へフォールバックするので健全。
- **実測**: 属性オペランドの二項演算を切り出したループ（呼び出し・`GET_ATTR` の希釈を抑えたもの）で
  **1.069x**（1.224s → 1.145s・同一ビルドで emit のみ A/B・best-of-3）。
  一方 `bench_field_access.ar` は 1.015x にとどまる（1 ケースあたり 100 万回の関数呼び出し＋7 回の `GET_ATTR` が支配的で、
  二項演算の占める割合が小さい）。**この2つの数字の差が「残る速度余地は呼び出し機構と属性読み」という §4.3 の知見と一致する**。
- **開発フック追加**: `AR_VM_DUMP=1` で `compile_fn` が生成 Chunk を逆アセンブルして stderr へ出す
  （[compiler.rs](src/vm/compiler.rs)）。`disasm.rs` はこれまで**呼び元ゼロ**だった。
  `kinetic` の本体が `FBIN_SS` ×6 になることを目視確認済み。
- **特化対象 op の拡大（(iii) 完遂・2026-08-10）**: `specialized_bin_kind` を「種別ごとに許可 op を持つ」形へ変更。
  - **int/int**: `Div`/`FloorDiv`/`Mod`/`Pow`/`BitAnd`/`BitOr`/`BitXor`/`LShift`/`RShift` を追加（従来は Add/Sub/Mul と比較のみ）。
    **ゼロ除算は特化側が `None` を返して汎用パスへ落とす**ので、`ZeroDivisionError` の 3 種の文言は
    `apply_binop` の一箇所に保たれる。`Pow` は指数非負なら整数冪・負なら float（既存アームと同一）。
  - **float/float**: `Div`/`Pow` を追加。**`//` と `%` は `apply_binop` に Float/Float アームが無い**（＝エラー）ため特化しない。
  - **int/float 混在**: op を6つ増やす代わりに **`apply_bin_fast` に混在アームを追加**（`Lt`/`Gt`/`Div`/`Pow`）。
    これで注釈の有無に関わらず `Bin`/`BinLocalLocal`/`BinLocalConst` の全経路が `apply_binop_dyn` への
    降下を避けられる。**`LtEq`/`GtEq` の混在は追加していない**（下記の言語仕様の穴を参照）。
  - **実測**: `(i % 7) + (i // 3) + (i & 255) + (i ^ 9) + (i | 4)` のループで **1.478x**（0.984s → 0.666s・
    同一ビルドで許可 op のみ A/B・best-of-3）。Add/Sub/Mul しか特化できなかった従来との差。
  - **同値性検証**: [examples/bench/numeric_ops_equivalence.ar](examples/bench/numeric_ops_equivalence.ar) を追加。
    int/float/混在の全演算＋境界値（負値・ゼロ・負の指数）＋ゼロ除算 3 種＋float の inf/NaN を網羅し、
    `--vm=off` と `--vm=auto` が **119 行 byte-identical** であることを確認する。

### ⚠️ 発見した言語仕様の穴 → **#18 として修正済み（2026-08-10）**
- `apply_binop` に int/float 混在の `<=` `>=` アームが無く、`i < f` は通るのに `i <= f` が TypeError だった。
  (b)(iii) の実装中に混在アームを足したところ VM だけ成功して off/auto が割れたため発覚。
  当時は「修正＝言語の挙動変更」として据え置き、後に **#18** へ昇格して修正した（str 比較の欠落も同時に）。

### ✅ 段階(b)(ii) `CheckBefore` 指示の消費（2026-08-10）
- **判明していた実態**: `Expr::MustBe` / `Expr::Cast` / `Expr::IsType` は `compile_expr` に arm が無く
  `_ => return None` に落ちていた。つまり **`mustbe` / `=>` / `is` を 1 つでも含む関数は丸ごと
  ツリーウォークへ bail** していた（検査が遅いのではなく、関数全体が VM 化されない）。
- **実装**: `Op::MustBe(name_idx, span_idx)` / `Op::Cast(name_idx)` を追加（`IsType` は既存 op を再利用、
  `negated` は `Op::Un(Not)` で表現）。コンパイラは `check_required(node_id)` で
  **`Directive::CheckBefore` を消費**して検査 op を出す。指示が無いノード（未採番・合成 AST・
  モジュール横断で注釈なし）は「検査が要るか判らない」ので**その関数の VM 化を諦める**
  ＝ 検査を省く方向へは決して倒さない。
- **意味論の一致は構造で担保**: 検査本体をコピーせず、ツリーウォークと VM が**同一メソッドを共有**する。
  `Interpreter::mustbe_check`（[eval/core.rs](src/interpreter/eval/core.rs)）を新設し `Expr::MustBe` アームと
  `Op::MustBe` の双方から呼ぶ。キャストも `eval_cast` を `eval_cast_evaled`（値を受ける版）へ分割し共有
  （[eval/calls.rs](src/interpreter/eval/calls.rs)）。`mustbe_outer_type` を `pub(crate)` 化。
- **実測**: `mustbe` を含むホット関数で **2.04x**（off 0.956s → auto 0.468s）。
  変更前はこの関数が bail していたので `auto` は `off` と同値だった＝**丸ごとの改善**。
- **注意（適用範囲）**: 既存例題（`mustbe.ar` / `polymorphism.ar` / `fixed_list.ar`）はこれらを
  **モジュール top-level** で使っており、VM は関数本体しかコンパイルしないため新 op は 0 個。
  効果が出るのは**関数内で使った場合**のみ。確認用に
  [examples/typing/runtime_checks_in_function.ar](examples/typing/runtime_checks_in_function.ar) を追加した。
- **未実施（意図的）**: `CallInfo` の**引数境界検査**（`call_check.rs` が param 具象 × arg 動的で付ける
  `CheckBefore`）。これは**現在どこでも実行されていない検査を新設する**ことになり、
  今まで通っていたコードが実行時エラーになりうる＝ off/auto byte-identical を壊す**言語の挙動変更**。
  段階(c) で境界検査をネイティブへインライン生成する設計と併せて判断すべきなので据え置いた。

### 🔬 引数境界検査の診断（2026-08-10・「設計が誤りか / 前提が未実装か」の切り分け）
**結論: 設計は誤っていない。生成側は意図どおり動いており、欠けているのは消費側だけ。**
- **計測手段**: `AstAnnotations::call_check_stats()`（[annotations.rs](src/type_check/annotations.rs)）＋
  `AR_ANNOT_DIFF=1` 時に [main.rs](src/main.rs) が `AnnotCalls: calls=N args_with_CheckBefore=M` を出す。
- **実測（例題全件）**: `calls=1650` に対し `args_with_CheckBefore=5`（`functions.ar` 2 件・`cpp_struct_ptr.ar` 3 件）。
- **なぜ少ないか（設計の穴ではなく前提の重複）**: 生成条件は
  「param 具象 × arg 動的（`Any` **または** `Unresolved`）」だが、**`Any` 側は静的型検査が既にハードエラーにする**
  （`argument 0 of 'f' expects 'int' but got 'Any'`）。したがって実行時まで到達しうるのは
  **`Unresolved` 側だけ**であり、件数が絞られるのは当然の帰結。
- **境界で本当に効くケースは存在する（実証済み）**:
  ```
  import[py-int] math as m
  fn takes_int(let x: int) -> str: ...
  let from_py = m.fabs(-3.5)   # 実体は float 3.5
  print(takes_int(from_py))    # → "got 3.5 / doubled 7.0" が**エラーなしで**出る
  ```
  型検査はこの引数に `CheckBefore(int)` を正しく生成している。にもかかわらず**誰も実行しない**ため、
  `x: int` と宣言したパラメータが実行時に float を保持したまま素通りする。
  ＝「外部言語ライブラリ境界を超えるのに必要」という当初の判断は妥当で、**未実装なのは消費側だけ**。
- **付随して判明した非一貫性**: 同じ py 呼び出しでも、`json.loads(...)` は `Any` を返すと**スタブが宣言している**ため
  静的エラーになり、`math.fabs(...)` はメンバ未知で呼び出し結果が `Unresolved` になるため無検査で通る。
  **スタブが当該メンバを宣言しているかどうかで「静的拒否」と「無検査素通り」が分かれる**。
  境界検査を実装する際はこの差を設計に織り込む必要がある（`Unresolved` を単に許すのではなく検査へ倒す）。

### 🔬 「スタブで潰しきれるか」の検証（2026-08-10）
**結論: 潰しきれない。しかも現行ルールは“スタブを整備するほど発火しなくなる”という逆向きの性質を持つ。**

**(A) スタブでは捕捉できず、動的境界検査なら捕捉できるケース — 存在する（実証済み）**
Python スタブは `.py` の型注釈をそのまま信用して `let f: function->int` を作る
（`extract_py_type_stubs`・[imports/mod.rs:112](src/parser/imports/mod.rs#L112)）。
**Python は注釈を実行時に強制しない**ので、注釈が嘘なら静的検査は素通りする。
```python
def get_int() -> int:  return "I am a string"    # 注釈は int、実体は str
```
```
let a = L.get_int()
print(takes_int(a))     # fn takes_int(let x: int)
```
→ 実測: `x=I am a string doubled=I am a stringI am a string`。
**エラーも警告も出ず、`x * 2` が文字列反復として実行され“静かに誤った答え”が出る**（最悪の失敗形）。
`-> int` が `None` を返す版は `TypeError: unsupported operand types for Mul: NoneType and int` と、
**境界ではなく使用箇所を責める分かりにくいエラー**になる。
コンテナ要素型も同様: `-> list` が `[1, "two", 3.0]` を返し `list[int]` で受けると、
ループ内部で `Add: int and str` として初めて露見する（`mustbe list[int]` が要素型を検査しない旨の
既存警告 `MustBeElemTypeUnchecked` と同じ構造の穴）。

**⚠️ 最重要: これらのケースで `args_with_CheckBefore = 0`。**
現行ルールは「param 具象 × arg 動的」で発火するが、スタブが整うと arg 型が具象（`int`）になるため
**検査指示が生成されなくなる**。＝ **スタブを整備するほど保護が消える**という逆向きの依存。
危険なのは「引数の静的型が動的なとき」ではなく「**静的型は具象に見えるが値が外部由来のとき**」。

**(B) スタブでも動的検査でも救えないケース — こちらも存在する**
C/C++ の `void*` は Arrow の **`int`** に落ちる（[imports/mod.rs:399](src/parser/imports/mod.rs#L399)）。
型タグは「int である」以上の情報を持たないので、動的型検査を入れても静的型と同じことしか言えず**無意味**。
ここを守るには型検査ではなく**ハンドルの出所・生存期間の追跡**が要る（別軸の課題）。

**(C) スタブが既に対処できているケース**
`OpaqueStructPtr`（`FILE*`/`HWND` 等）と `ByValueStruct` は Arrow の **`Any`** に落ちる。
`Any` を具象パラメータへ渡すのは現状ハードな静的エラーなので、ここは既に塞がっている
（やや過剰で、利用側に cast/`mustbe` を強制する）。

**→ 設計上の含意**: 境界検査は**引数の静的型ではなく「値が FFI 境界を越えてきた」という出所**を鍵にすべき。
具体的には「Arrow 呼び出しの引数を検査する」のではなく、
**外部呼び出しの戻り値をスタブ宣言型と突き合わせて Arrow へ入る瞬間に検査する**方が、
(A) を正面から捕まえられ、スタブ整備と矛盾しない（スタブが宣言した型が検査の根拠になる）。

### ✅ 段階(b)(i) 属性アクセスの高速化（2026-08-10）
- **当初の想定は外れた**: 「静的にクラスが確定していれば R3 IC のチェックを省く」つもりだったが、
  `GetAttr` のヒット経路（[run.rs](src/vm/run.rs)）を読むと IC ヒットは既に
  `class_id` の整数比較＋アクセス種別チェックだけで、**静的化しても削れるのはこの 2 つの比較のみ**。
  実際の支配項は別で、`local.attr` が `LoadLocal(slot); GetAttr(..)` に展開されるため
  **`LoadLocal` が `Value` を clone する＝`Rc` の refcount 増減が属性読みごとに発生**していた。
- **実装**: 二項演算の超命令と同じ手を属性へ適用。`Op::GetAttrLocal(slot, name_idx, cache_idx)` を追加し、
  レシーバを **frame から参照で読む**（clone・push/pop なし）。IC ミス・非 public・非インスタンスのときだけ
  clone してフルパス（`get_attr_val`）へ回すので意味論は `GetAttr` と同一。
- **実測**: 属性オペランドの二項演算ループ **1.1446s → 0.8298s = 1.379x**、
  `bench_field_access.ar`（呼び出し支配）**1.2217s → 1.1061s = 1.105x**。
  (b)(iii) までの積み上げと合わせ、属性読みが多いコードで効く。
- **検証**: [examples/classes/attr_access_paths.ar](examples/classes/attr_access_paths.ar) を追加。
  局所変数レシーバ／ネスト属性（外側は非局所）／public・private／テンプレート経由で同じ命令に
  別クラスが流れる場合（**IC ミス→再解決**）／存在しない属性（AttributeError）を網羅し off/auto 一致。
- **メソッド呼び出しの融合（`CallMethodLocal`）も実施 2026-08-10**:
  `local.method(args)` を `Op::CallMethodLocal(slot, name_idx, argc, mut_mask)` へ融合し、
  レシーバをスタックへ積まず frame から読む。**ただし属性読みと違い clone は消せない**
  （レシーバは呼び先の `self` として所有権が要る）。消えるのは `LoadLocal` の op ディスパッチと
  push/pop 1 組だけなので、**実測 1.042x**（1.375s → 1.320s・同一ビルドで emit のみ A/B・best-of-3）と小さい。
  §4.3 の「残る速度余地は呼び出し機構（bind/フレーム構築）」という知見どおりで、
  **レシーバの受け渡しは支配項ではなかった**。
  - **評価順の注意**: 融合版は引数評価後に frame を読む（融合前はレシーバが先）。
    VM がコンパイルするコードでは式の評価中に自フレームの slot が再束縛されない
    （再束縛は文＝`StoreLocal` のみ・クロージャ捕捉は VM 非対応で bail）ため観測は同一。
    [examples/classes/method_call_paths.ar](examples/classes/method_call_paths.ar) で
    引数自体がメソッド呼び出し／`mut self`／ネストしたレシーバ／非局所レシーバを網羅し off/auto 一致を確認。
  - **測定上の落とし穴**: `bench_method_call.ar` の hot loop は**モジュール top-level** にあり
    VM が一切コンパイルしないため、この融合の測定には使えない（当初これで測って誤った数字を出した）。
    関数内のループで測ること。

### ✅ 段階 F — モジュール横断の注釈（2026-08-10・**#16 完了**）
調べたところ欠けは **2 段階**あり、当初想定（「import 先の本体に注釈が付かない」）より広かった。

**(F-1) 型検査のレジストリが import 先の定義を収集していなかった**
（[registry/builder.rs](src/type_check/registry/builder.rs) の `collect` に Import アームが無かった）。
そのため import したクラスが `known_class_names` に載らず、**メインプログラム側でも**
`v.x`（`v: Vec2` が import 由来）の型が引けなかった。
実測: import クラスを使う算術 3 件が**すべて特化されず**（`Attr` 由来の `Unresolved` が 4 件）。
→ `Stmt::Import` / `Stmt::FromImport` の `body` へ再帰するようにした。
**`fn_sigs` は `push` で積むため二重収集すると偽のオーバーロードになる**（単一シグネチャ前提の
高速パスが崩れる）ので、`(lang, モジュールパス)` で重複を弾く。

**(F-2) import 先モジュールの関数本体が型検査の走査対象外だった**
（`Stmt::Import` は `collect_module_types`＝署名読みしか通らなかった）。
実測: 同じ式でもメイン側は `FBIN_SS`、import 先は `BIN` のままという非対称が起きていた。
→ `TypeChecker::annotate_module_body`（[stmt/resolve.rs](src/type_check/stmt/resolve.rs)）を追加し、
本体を**隔離スコープで検査して注釈だけ採取**する。
- **診断は捨てる**: モジュール自身の型エラーは、そのモジュールを直接実行/`--compile` した
  ときに報告されるべきもの。ここで出すと import 側に二重に出る（動作確認済み: 型エラーを含む
  モジュールを import しても import 側は正常終了する）。
- 重複走査は `annotated_modules` で防ぐ。

**結果**: 例題全件の特化 binop **235 → 248**。合成テストでは import クラスを使う算術が 0/3 → 3/3。
**FFI 境界検査も import 先で有効になった**（`lib2.ar` 内の py 呼び出しが `FfiTypeError` を出す）。
node-id の一意化（C1 の実装漏れ修正）と合わせ、**注釈がプログラム全体へ行き渡った**。
例題: [examples/interop/cross_module_annotation.ar](examples/interop/cross_module_annotation.ar)。

### ✅ 段階 E — テンプレート実体化での型特化（2026-08-10）
**問題**: テンプレートは実体化時に AST を複製して型変数を具体型へ置換するが、**node-id は原型から
コピーされる**ため、注釈テーブルは「型変数のままの原型」を指したままになる。
型検査は `x: T` を `NamedInstance("T")`（実在しないクラス扱い）として通すので、
注釈は**間違ってはいないが常に `Unresolved` 相当**で、`Foo[int]` でも型特化 op が一切出なかった。

**採らなかった案**: 実体化ごとに node-id を再採番して型検査を再実行する。
テンプレート実体化は**実行時**（`templates.rs` の `subst`）に起きるため、
型検査器（プログラム全体のレジストリを要する）を実行時に走らせる必要があり、
得られる効果に対して構造変更が大きすぎる。

**採った案**: 注釈が無いとき **実体化後の AST に書かれている型注釈から特化種別を導出する**
（`Compiler::local_operand_kind`・[compiler.rs](src/vm/compiler.rs)）。
`subst` は param の型注釈を具体型へ置換済み（`a: T` → `a: int`）で、VM コンパイラは
それを `slot_type` に持っている。両オペランドが同一プリミティブ注釈の局所変数
（数値リテラルは相方に合わせる）のときだけ種別を返す。
- **健全性**: 特化 op は実行時型が想定外なら汎用へフォールバックするので、
  導出が外れても**結果は変わらない**（速度の無駄が出るだけ）。
- **副次効果**: 同じ理由で**注釈テーブルが届かない箇所（import 先モジュール等）にも効く**
  ＝ 段階 F の一部を実質的に前倒しできている。
- **実測**: テンプレート実体化ループで **int 1.059x / float 1.086x**
  （同一ビルドで導出のみ A/B）。例題全件の特化件数は 231 → 235。
- **例題**: [examples/typing/template_specialization.ar](examples/typing/template_specialization.ar)
  （同一テンプレートを int/float/str で実体化＝ node-id 共有下での正しさを確認）。
- **付随して見つけた既存制約**: `str < str` は `apply_binop` が未対応で
  `TypeError: unsupported operand types for Lt: str and str` になる（テンプレートとは無関係・off/auto 同一挙動）。
  #18 の「混在 `<=`/`>=` が無い」と同じ系統の穴。

### ✅ 段階(c-3) — ネイティブ codegen が注釈を第一の根拠にする（2026-08-10・**#16 の目的達成**）
`field_ty_resolved` を `legacy` 返しから **`annotated.or(legacy)`** へ変更
（[context.rs](src/partial_compiler/llvm_codegen/context.rs)）。これで
**ツリーウォーク／VM／ネイティブの三経路が同じ AST 型解決注釈を根拠に動く**＝ #16 の当初目的。
自前導出 `field_ty` はフォールバックとして残す（実測で `legacy_only=0`・`conflict=0`＝
自前導出が注釈より広く解けるケースは 1 件も無いが、node-id が付かない合成 AST 用のゼロコストな保険）。

- **効果は c-2 時点の見積もり（`annot_only=7`）を大きく超えた**。属性が typed になると
  **その先の演算まで連鎖して typed になる**ため。`flat_bench_module.compute_mut` の IR で:
  - 変更前: `p.x` が `CB_GET_ATTR`（**ハンドル**を返す）→ 算術も `CB_BINOP`（ボックス化）→ 最後に `CB_TO_FLOAT`
  - 変更後: `p.x` が `CB_GET_FLOAT_FIELD`（**`double` を直返し**）→ **ボックス化二項演算コールバックが全て消滅**し
    ネイティブ float 演算に。関数内の `call` 命令が 25 → 13、IR 全体 12783 → 11313 バイト。
- **実測**: `flat_bench.ar` の mutable-list 経路（25M 要素アクセス）が
  **43.90s → 25.36s = 1.73x**（同一ビルドで `annotated.or(legacy)` ⇄ `legacy` を A/B）。
- **数値の一致を確認**: 同じ入力に対しツリーウォーク（`.arc` 無し）とネイティブ（`--compile` 後）が
  ともに `5008.9693499999985` を返す。
- **他 5 モジュールの IR は byte-identical**（注釈が自前導出を上回るのが flat_bench_module だけだったため）。
- **`CallInfo` の引数検査指示を境界インライン生成する話は取り下げ**: この役割は
  FFI 境界検査（戻り値方向・`ffi_boundary`）が担い、そちらの方が本質的だと判明したため
  （スタブ整備と同じ向きに強くなる／引数方向は向こうの型が不明なことが多い）。

### ✅ 段階 D — 型検査の解像度・第3弾（2026-08-10）
**推測せず計測から入った**。`binop_kind` が付かなかった二項演算を理由別に数える診断を追加
（`AstAnnotations::note_binop_miss` / `note_unresolved_source`・`AR_ANNOT_DIFF=1` で出力）。
- **初期値（例題全件）**: binop 559 件中 **specialized=214 / miss=345**。
  miss の内訳は `both_unresolved=101` / `one_unresolved=150` / `resolved_but_mixed=94`
  ＝ **miss の 73%（251/345）が `Unresolved` 絡み**で、「律速は型検査の解像度」という仮説を数字で確認。
- **`Unresolved` の発生源**（式の種類別）: `BinOp` 123 / `Call` 118 / `Ident` 95 / `Attr` 14 / `TraitAccess` 2。
  `BinOp` と `Ident` は**伝播**であり、**根は `Call`**（＝戻り値型が判らない呼び出し）と特定。
- **修正 1: `Expr::ForExpr` がループ変数を宣言していなかった**（[infer.rs](src/type_check/infer.rs)）。
  `Stmt::For` は先に直していたが**式の for が漏れていた**。本体では変数が未宣言＝`Unresolved` だった。
  なお修正直後は特化件数が**減った**。原因は、未宣言だったせいで**外側スコープの同名変数を拾って
  偶然型が付いていた**ケースがあったため。これが次の修正の必要性を露わにした。
- **修正 2: `range()` に戻り値型 `list[int]` を与えた**（[type_check/mod.rs](src/type_check/mod.rs)）。
  `range` は型検査のグローバルに登録が無く未知の識別子扱いで、`range(n)` が `Unresolved`
  → **`for i in range(n)` のループ変数が型無し**になっていた。最頻出のループ形なのに本体が一切特化されない。
- **結果**: specialized **214 → 231**（miss 345 → 328）。
  `for i in range(n)` のループで **1.153x**（0.460s → 0.399s・同一ビルドで A/B・best-of-3）。
- **`len`/`repr` は意図的に外した**: ここへ登録した名前はグローバルスコープを占めるため
  `let len = ...` が「already declared」の静的エラーになる（`int`/`str` と同じ扱い）。
  `let len = ...` は今まで通っていた書き方で**新たなエラーを増やす**割に、特化件数の伸びは **+1** しかなかった。
  `range` は変数名として使われることが稀なので残した。
- **例題**: [examples/basics/for_range_typing.ar](examples/basics/for_range_typing.ar)
  （文の for・入れ子・for 式・`len` 利用・外側と同名のループ変数）。
- **残る miss の主因**: `partial_call_overhead.ar`(69) と `bottleneck_bench.ar`(68) が突出しており、
  いずれも **`time.time()` など py 組み込みモジュールの呼び出しが `Unresolved`** を生んでいる。
  `time` は C 実装で `.py` ソースが無くスタブを抽出できないため、**型検査の推論規則ではこれ以上詰められない**。
  次に効くのは**組み込み py モジュール向けのスタブ整備**（#17-b と同じ「スタブで潰す」方向）。

### ✅ FFI 境界検査 — (A) への対応（2026-08-10）
**設計方針**: 検査の鍵を「引数の静的型が動的か」から「**値が FFI 境界を越えてきたか**」へ移した。
従来の `CheckBefore`（param 具象 × arg 動的）は**スタブが整うほど発火しなくなる**逆向きの性質を持つが、
本機構は**スタブが宣言した型を検査の根拠にする**ので、スタブ整備の方針と同じ向きに強くなる。

- **検査点**: 外部関数呼び出しの**戻り値**が Arrow へ入る瞬間。
  `Interpreter::check_ffi_return`（[eval/calls.rs](src/interpreter/eval/calls.rs)）。
  宣言型は **型検査が Call ノードへ焼いた解決型**（＝ #16 の注釈テーブル）から引く。段階(a) の基盤がそのまま効いた。
- **経路**: `mod.func()`（`Expr::Attr` → `eval_method_call` へ委譲）と、PyObject/JsProcFn を直接呼ぶ形の両方。
  前者は委譲先の署名を増やさぬよう、呼ぶ前に `foreign_call_lang` で呼び先の言語だけ覗く。
- **言語ごとの検査器**: [src/interpreter/ffi_boundary.rs](src/interpreter/ffi_boundary.rs)（新規）。
  `trait BoundaryChecker` ＋ 言語非依存の共通判定 `check_common`、言語登録は `checker_for` の 1 行。
  **言語を足すときの変更は「impl を書く」「`checker_for` に 1 行」の 2 箇所だけ**（呼び出し側・エラー生成・
  値の差し替えは共通実装）。
  - `PythonChecker`: 共通判定そのまま（Python は int/float を区別して届く）。
  - `JavaScriptChecker`: **JS は数値がすべて f64** で届く（`decode_result` の `"f"`）。
    素朴に「int か」を見ると正しいコードが落ちるので、**整数値の Float は `int` 宣言に適合として `Int` へ寄せる**
    （`Verdict::Coerce`）。小数を持つ値は本物の不一致。要素が int のリストも同じ緩和を適用。
  - 静的型付け言語（C/C++・C#・Rust）は登録しない＝無検査（向こう側が型を守るため）。
- **判定は 4 値**: `Ok` / `Coerce(値)` / `Mismatch` / `Unverifiable`。
  **判定できない宣言型（`Any`/`Unresolved`/関数型/`NamedInstance` 等）は `Unverifiable` で素通し**。
  誤検知で正しいコードを落とすより取りこぼす方に倒した。＝ スタブが無い箇所の挙動は変わらない。
- **コンテナは要素型まで検査する**（`list[int]` と宣言して `[1, "two", 3.0]` が返る、が実際の失敗例）。
  走査コストは `py_to_tl` が既に全要素を歩いているのと同オーダー。

### ✅ スタブ側の穴埋め（`Unresolved`/`Any` を減らす）
- **PEP 585 の小文字ジェネリクスに未対応だった**: `py_type_to_arrow`（[imports/mod.rs](src/parser/imports/mod.rs)）は
  `typing.List[T]` は見ていたが **`list[int]`（Python 3.9+ の標準表記）を catch-all で `Any` に落としていた**。
  その結果スタブが要素型を失い、境界検査も `Any` は検査不能として素通しするため機構が成立しなかった。
  `list[T]` / `set[T]` / `Set[T]` / `dict[...]` / `tuple[...]` を追加。
- **PEP 604 の `X | None` / `X | Y`** も `Option[T]` / `Union[T, U]` へ変換するようにした
  （角括弧を含む場合は入れ子の区切りと紛れるので対象外＝保守的）。
- これで **スタブを書けば書くほど静的にも動的にも締まる**（`Unresolved` はスタブで潰し、
  潰しきれない「スタブが嘘をつく」ケースは境界検査が動的に捕まえる）という二段構えが成立する。

### 🧪 検証
- `cargo test` **696 緑**（`ffi_boundary` の言語別ポリシー単体テスト 10 件を追加）。警告 0。
- 例題: [ffi_boundary_check.ar](examples/interop/ffi_boundary_check.ar)（正例・スタブどおりなら無干渉）／
  [ffi_boundary_check_error.ar](examples/interop/ffi_boundary_check_error.ar)（負例）
  ＋素材 [ffi_probe/](examples/interop/ffi_probe/)。
- 実測（`lying_py.py`）: `-> int` が str/None を返す、`-> list[int]` が `[1,"two",3.0]` を返す、の 3 例とも
  **境界の行を指す `FfiTypeError`** になった。以前は 1 つ目が `doubled=...` を静かに誤答し、
  2 つ目は使用箇所で `Mul: NoneType and int` という分かりにくいエラーになっていた。
- 例題スキャン FAIL 0・off/auto 35 例題 byte-identical（検査は解釈経路の共通部分に入るため両モード同一）。

### 🐛 node-id のモジュール横断衝突を修正（2026-08-10・境界検査導入で顕在化）
設計判断は **C1「グローバル採番」** だったが、実装は **per-module で 1 始まり**になっていた
（サブパーサが `node_counter: 0` から採番）。import 先モジュールの node-id がメインと衝突し、
**消費側が別モジュールの注釈を読む**状態だった。
VM の型特化のように実行時フォールバックを持つ消費者は結果が変わらないので今まで露見しなかったが、
**注釈を信頼する FFI 境界検査では誤検知**になる。実際に再現した:
別モジュール内の `P.give_int()`（正しく int を返す）が
`declared to return 'str' but returned 'int'` と報告された。
→ `Parser.node_counter` を `Rc<Cell<u32>>` にしてサブパーサへ共有し、プログラム全体で一意にした
（[parser/mod.rs](src/parser/mod.rs)・[imports/ar_modules.rs](src/parser/imports/ar_modules.rs)・
[imports/cs_js_modules.rs](src/parser/imports/cs_js_modules.rs)）。
import 先モジュールの関数本体は型検査の対象外なので**注釈が付かない＝検査がスキップされる**（安全側）。
そこまで検査を効かせるには下記「モジュール横断の注釈管理」が要る。

### ⚠️ この機構でも救えない範囲（→ #17 へ昇格）
- **C/C++ の `void*`** は Arrow の `int` に落ちる。型タグは「int である」以上を語らないので、
  動的検査を入れても静的型と同じことしか言えない → **#17-a（専用型の導入）**。
- **JS はスタブ（`.ars`）に型が書かれていて初めて効く**。現状の `.ars` は型を持たないため実質無検査
  → **#17-b（基本は `Any`・`.d.ts` があればそれを使ってスタブ生成）**。
- **引数方向**（Arrow → 外部）は対象外。向こう側の引数型が分からない場合が多く、
  分かる場合は静的検査で足りるため。

### ✅ #16 の全サブタスク（完了記録・2026-08-10）
- ~~**段階(b)**~~ 【✅ (i)(ii)(iii) すべて完了】
  （(i) `GetAttrLocal` ＋ `CallMethodLocal`、
  ~~(ii) `CheckBefore` 指示の消費~~ 【✅ 完了】、
  ~~(iii) 融合できない形への型特化~~ 【✅ 完了・`IntBinSS`/`FloatBinSS` ＋ Div/Mod/Pow/bit ＋ 混在】。
  **(ii) の残り**: Call 引数の境界検査（上記のとおり挙動変更なので要判断）。
- ~~for ループターゲットの要素型推論~~ 【✅ 完了 2026-08-10】（下記参照）。
- ~~`infer_attr` の戻り値をフィールド実型にする~~ 【✅ 完了 2026-08-10】（型検査の解像度・第2弾）。
- ~~型検査の解像度・第3弾~~ 【✅ 完了 2026-08-10】（下記「段階 D」節）。
- ~~**段階(c-3)**~~ 【✅ 完了 2026-08-10】（上記「段階(c-3)」節）。
  c-2 時点では「注釈が codegen を上回るのは 7 箇所だけ＝効果は小さい」と見積もっていたが、
  **属性が typed になると演算まで連鎖する**ため実際は 1.73x だった。見積もりが外れた理由も記録済み。
- ~~**テンプレート対応**~~ 【✅ 完了 2026-08-10・段階 E】（実体化後の型注釈から特化種別を導出）。
- ~~**モジュール横断（段階 F）**~~ 【✅ 完了 2026-08-10】（レジストリの import 収集＋本体の注釈採取）。

**→ #16 はこれで完了。以降に残るのは下記の「⚠️ 別途の再検討事項」と、番号付きリストの #17/#18。**

### ✅ for ループターゲットの要素型推論（c-2 結論 4 の前提・2026-08-10）
- **実装**: `TypeChecker::for_element_type`（[stmt/resolve.rs](src/type_check/stmt/resolve.rs)）＋
  `Stmt::For` の検査（[stmt/check.rs](src/type_check/stmt/check.rs)）。`ListOf`/`FixedListOf`/`ListLikeOf`/`SetOf` は要素型、
  `Str` は 1 文字ずつの `Str`、**全要素同型のタプル**はその型、それ以外は従来どおり `Unresolved`。
  分割代入（`for k, v in pairs`）は要素型が要素数一致の `Tuple` のときのみ各要素型を割り当てる。
  **`dict` は Arrow では反復不可**（`make_for_iterator` が `TypeError`）なので対象外。
- **効果（実測）**: `flat_bench_module.compute_mut` の `p.x` 等 7 箇所が `neither=7` → **`annot_only=7`**
  （＝注釈のみが解決＝ネイティブ codegen が新たに型特化できる箇所）。全モジュールで `conflict=0`。
- **意味論の変化（小さい）**: 欠落フィールドは元々静的検査対象外（`check_member_access_static` は
  `has_field` が false なら早期 return）なので**エラーは増えない**。増えるのは
  **ループ変数経由の private/protected アクセス**が `StaticTypeError` として捕捉されるケース（動作確認済み）。
- **検証**: `cargo test` **686 緑**・警告 0・off/auto **32 例題 byte-identical**・例題スキャン **FAIL 0**。
  FAIL 0 は本変更前（HEAD）でも同じ 5 件が失敗しており**回帰ではない**ことを、変更を退避して HEAD をビルドし直し
  同一スキャンで確認済み（下記「例題の修正」参照）。

### 🧹 例題の修正（上記の検証で判明した既存不具合・2026-08-10）
for 推論とは無関係に前から失敗していた 5 例題を修正した（言語仕様どおりのエラーで、例題側が古かったもの）。
- `built_in.ar` — `class Box` にフィールド宣言が無い（`mut val: int` を追加）。さらに
  `make_and_write` が**コミット済みの生成物 `_tmp_new.txt`** に阻まれるため、`import[py-int] os` ＋
  `try: os.remove(...)` で先に後始末する（`import[py-int] os.path` は**組み込みの `path` 型を隠す**ので不可）。
- `collection.ar` — `class Stack` にフィールド宣言が無い（`mut items: list[int]` を追加）。
- `functions.ar` — `mut count: int = 0` は仕様違反（`const`/`static mut` のみ既定値可）。`__init__` で初期化へ。
- `variable.ar` — freeze 後の書き込み TypeError が未捕捉でスクリプトが中断し、以降の約 6 割が未実行だった。try/except で捕捉。
- `importation.ar` — `import[rs] sha2` のクレートが `rust.crates_path` に無い**環境要因**。ソースは正しいので
  [run_examples.ps1](run_examples.ps1) / [compare_vm_modes.ps1](compare_vm_modes.ps1) の skip へ追加した。

---

## #15b `Ident` の AST 表現再設計（実装完了・消費者不在を確定 2026-08-11）

### やったこと
- `Expr::Ident(String)` → **`Expr::Ident { name: String, node_id: u32 }`**（struct 変種化）。
- `LocalRef`/`GlobalRef` にも `node_id` を追加し、**リゾルバの書き換えで引き継ぐ**
  （[resolver.rs](src/interpreter/resolver.rs)）。リゾルバは型検査の**後**に走るので、
  引き継がないと消費者（解決済み AST しか見ない）が注釈を引けなくなる。
- `subst_expr`（[templates.rs](src/interpreter/templates.rs)）も node_id を保存する既存規約（段階 E）に合わせた。
- 型検査 [infer.rs](src/type_check/infer.rs) が**参照サイトごとの型**を注釈テーブルへ焼く。
- 診断 `AnnotIdent`（`AR_ANNOT_DIFF=1`）を追加。

### なぜ「参照サイト単位」でなければならないか
型ガード絞り込み（`if x is int:`）は **分岐スコープでの再 `declare`** として実装されている
（[stmt/check.rs](src/type_check/stmt/check.rs) `narrow_by_type_guard` → `self.declare(...)`）。
したがって同じ変数でも参照位置によって `lookup` の答えが変わる。
**`(関数, 変数名)` をキーにした変数単位の表では表現できない**ため、node-id が必要になる。
（この確認をしないと「#11 の slot を流用すれば AST 改変ゼロで済む」という誤った結論に至る。）

### ⚠️ 消費者は 0 件（実測・本タスクの主要な成果）
代表 6 モジュールで識別子読みを分類した（`AnnotIdent`）:

| 分類 | 件数 | 意味 |
|---|---:|---|
| `slot_typed` + `name_typed` | **192** | codegen の自前導出が既に具象型を得ている（余地なし） |
| `annot_boxed` | **40** | `Ty::Handle` に落ちたが、注釈も「クラス／リスト／str 等」＝**ハンドル表現が正しい** |
| **`annot_only`** | **0** | **注釈なら型特化できたのに codegen が落としていた箇所** |
| `annot_none` | **0** | 注釈が無い／`Unresolved` |
| `global_ref` | 3 | 本質的にハンドル |

**`annot_only = 0`** ＝ 識別子読みで型情報を落としている箇所は 1 件も存在しない。
`annot_none = 0` は「注釈が全件解決している」＝**配線が正しいことの裏取り**にもなっている
（注釈テーブルは physics で 117→234 と約 2 倍に増えた）。

**判断: 実装は残すが消費側への配線は行わない。** #11 R2-c（`CB_GET_GLOBAL` 3 箇所）・
#14（モジュール間直リンクの辺が 0）と同じ「消費者不在」の判定基準を適用した。

### 「Ident が `Unresolved` の 27%」は表現の問題ではなかった
全例題 103 本の集計（[annot_unresolved.ps1](annot_unresolved.ps1)）:
`BinOp 128 (34%) / Call 125 (33%) / Ident 101 (27%) / Attr 14 / Subscript 7 / TraitAccess 2`。
Ident が 3 位だが、`infer` の Ident アームは `self.lookup(name)` の結果をそのまま返すだけなので、
**この 101 件は「検査器のスコープが型を持っていない」ことを意味する**。node-id を足しても 1 件も減らない。
→ 効くのは表現ではなく **スタブ整備・推論規則の側**（次候補が py 組み込みスタブなのはこの実測による）。

### コストと検証
- 入口コスト: A/B 実測で median **+1.9%**（parse+typecheck 支配・31 例題 × 5 回・ノイズ水準）。
- `cargo build` 警告 0 ／ `cargo test` **696 緑** ／ `compare_vm_modes.ps1` **identical 42 / differing 0** ／
  `scan_examples.ps1` **FAIL 0**（92 例題）／ `dump_native_ir.ps1` 代表 6 モジュール **IR byte-identical**。
- 変更規模: 実サイト 68（計画書の「97 サイト」は #11 の切り出し前の陳腐化した数字だった。実際は 79・うち約 11 はコメント）。

### 次にここを触るなら
残る本命は **`Ident`/`LocalRef`/`GlobalRef` の 3 変種統合**（1 概念に 3 変種・合計約 132 サイト）。
node-id 追加とは別物で、速度ではなく**コードの簡素化**が動機になる。大改造なので単独で判断すること。

---

## #15c 識別子 3 変種の統合（完了 2026-08-11）

### 動機
`Expr::Ident` / `Expr::LocalRef` / `Expr::GlobalRef` は**1 つの概念（識別子参照）に 3 変種**で、
各パスが同じ 3 アームを書かされていた。一方、名前だけ欲しい大多数のサイトはその区別を使っていない。
速度ではなく**コードの簡素化**が動機（#15b から切り出した項目）。

### 形
```rust
Expr::Ident { name: String, node_id: u32, res: Resolution }

pub enum Resolution {
    Unresolved,          // 名前でスコープを引く
    Local(u32),          // 関数 base スコープの slot 索引（R1）
    Global(SlotCache),   // 最上位スコープ＋実行時 index キャッシュ（R2-b）
}
```
**リゾルバは変種を差し替えるのではなく `res` を書く**（[resolver.rs](src/interpreter/resolver.rs)）。
`name` / `node_id` がそのまま残るので、書き換えで注釈やフォールバックを失わない。
`Resolution::Global` の `SlotCache::clone` は空を返すため、テンプレート実体化での再解決は自動的に働く。

### ⚠️ 機械的に統合すると壊れる（この作業の要点）
統合前に `Expr::Ident` **だけ**にマッチしていた箇所は「**未解決の**識別子」しか見ていなかった。
素直に 1 変種へ統合すると、これらが解決済みも拾う。実害の具体例:

- [builtins.rs](src/interpreter/eval/builtins.rs) の `eval_builtin_ident_call` は
  **名前だけで組み込みへ振り分け、シャドウ検査が無い**。
  → `len` / `print` という名のローカル変数が関数値を保持していると、**組み込みに横取りされる**。
- [calls.rs](src/interpreter/eval/calls.rs) の `call_name`（トレースバック表示名）は
  従来 `LocalRef`/`GlobalRef` を `"<anonymous>"` にしていた。広げると**出力が変わり byte-identical が壊れる**。
- [vm/compiler.rs](src/vm/compiler.rs) の呼び出し経路は `Ident` → `GlobalRef` → `LocalRef` の
  if-chain で、先頭を無条件 `Ident` にすると**後続 2 分岐が dead になる**。

対処: インタプリタ実行経路（`eval/`・`exec/`・`functions/`）の **18 サイトを
`res: Resolution::Unresolved` に限定**して旧挙動を保存。VM はアーム順（Unresolved → Global → Local）で
旧 if-chain と同一に保った。**広げる（解決済みも受ける）のは意味論の変更**なので別途判断とする。

型検査・パーサのサイトは限定していない — **リゾルバは型検査の後に走る**ので `res` は常に `Unresolved` であり、
限定しても意味論は同じだから。

### 効果と検証
- 3 変種 → 1 変種。3-way or-pattern を全廃（`ident_name` / `expr_eligible` / `annotatable_node_id` /
  `ast_value` / `infer` などが 1 アームに）。
- **`Expr` のサイズは 112 バイトで不変**（HEAD で実測して確認）。`Resolution` は 16 バイトで
  既存の最大変種に収まった。インタプリタ全体への波及なし。
- 差分 **28 ファイル・+177/−176 行**。
- `cargo build` 警告 0 ／ `cargo test` **696 緑** ／ clippy **増分 0**（HEAD と同じ 50 件）／
  `compare_vm_modes.ps1` **identical 42 / differing 0** ／ `scan_examples.ps1` **FAIL 0** ／
  `dump_native_ir.ps1` 代表 6 モジュール **IR byte-identical**。
- 旧変種名を指す doc コメント 35 箇所も `Resolution::*` へ更新済み（残存 0）。

---

## #15 `Value::Str` → `Rc<str>`（§7.4-1 完了・§7.4-3 は消費者不在で保留 2026-08-11）

### やったこと
`String` を持っていた 3 つの文字列表現を**まとめて** `Rc<str>` にした。1 つだけ変えても
境界で確保が復活するので効果が出ない（例: `Value` だけ変えると `d["k"]` が `DictKey` 生成で確保する）。

| 変更 | 効果 |
|---|---|
| `Value::Str(String)` → `Value::Str(Rc<str>)`（[value/core.rs](src/interpreter/value/core.rs)） | 変数読み・引数束縛・スタック push の `Value::clone` が参照カウント加算だけになる |
| `DictKey::Str(String)` → `Rc<str>`（[value/collections.rs](src/interpreter/value/collections.rs)） | `d["key"]` の索引でキーを作るたびの String 確保が消える |
| `Expr::Str(String)` → `Rc<str>`（[ast.rs](src/ast.rs)） | ツリーウォークのリテラル評価 `Value::Str(s.clone())` が確保しなくなる（リテラルの実体は AST に 1 本） |

- **`Value::str(impl Into<Rc<str>>)` に構築を集約**（[value/core.rs](src/interpreter/value/core.rs)）。
  `&str` / `String` / `Rc<str>` のどれからでも書け、`Rc<str>` を渡した場合は確保しない。
  304 サイトのうち構築側はほぼ全てこれ 1 本に寄った。
- `eval_str_method` は**レシーバを `Rc<str>` で受ける**ようにし（[classes/string_methods.rs](src/interpreter/classes/string_methods.rs)）、
  引数抽出マクロ（`arg_str!` / `arg_opt_str!`）は `String` を返すままにした。
  こうすると下流の `sep.as_str()` 等 23 箇所を触らずに済む（`Rc<str>` への `.as_str()` は unstable な `str_as_str`）。

### ⚠ この変更で唯一壊れうる所: `deep_clone`
`deep_clone` は **async のスレッド間送出**（[async_mgr.rs](src/interpreter/async_mgr.rs) `var.get_value().deep_clone()`）で使う。
`Rc` の参照カウントは**非アトミック**なので、素直に `s.clone()` にすると
**バッファを 2 スレッドで共有したまま送り出してカウンタが壊れる**。
`Value::Str(s) => Value::Str(Rc::from(&**s))` と書いて必ず独立バッファを作ること
（`DictKey::Str` 経由の復元も同様）。share-nothing（D5）はこの 1 行に依存している。

**負の対照で実在を確認した**（推測ではない）。この 1 行を `s.clone()`（＝共有）に戻すと
[async_string_share.ar](examples/async/async_string_share.ar) が **`Illegal instruction`（exit 132）で落ちる**。
正しい実装では 10 回連続で `results` が全て同値・exit 0。

⚠ ただし**接触回数を上げないと再現しない**。最初に書いた版（各タスクが捕捉文字列を 1 回だけ読む）は
壊れたビルドでも 6 回中 6 回とも正常終了した。`wait_for_finish()` 中の main は当該 `Rc` を触らないので、
競合するのは**ワーカースレッド同士**であり、各スレッドが捕捉文字列を**ループ内で繰り返し clone** して
初めてカウンタの取り合いが起きる。例題は 8 スレッド × 40000 回読みにしてある。
将来この例題を軽量化すると**検知力を失う**（落ちなくなるだけで、バグは残る）ので縮めないこと。

### 実測（同一マシン・release・best-of-3・[bench_string.ar](examples/bench/bench_string.ar)）
| ケース | HEAD | #15 | 倍率 |
|---|---:|---:|---:|
| 1. 文字列変数の読み | 0.2093 s | 0.1609 s | **1.30x** |
| 2. 文字列の引数束縛 | 0.1627 s | 0.1323 s | **1.23x** |
| 3. 文字列連結（対照群） | 0.1253 s | 0.1098 s | 1.14x |
| 4. 属性/メソッド経由 | 0.2125 s | 0.1745 s | **1.22x** |
| 5. dict の文字列キー | 0.1528 s | 0.1012 s | **1.51x** |

- **対照群（3. 連結）も 1.14x 速くなった**のは想定外。連結自体は `Rc<str>` で 1 回余分に
  コピーするので不利なはずだが、同じループ内のリテラル読み・`len()` 引数束縛の利得が上回った。
  ＝「文字列を作る」より「文字列を運ぶ」方が多いというワークロードの性質が出ている。
- **dict キーが最大（1.51x）**。`DictKey` を一緒に変えなければここは伸びなかった。
- 数値ベンチ（[bench_field_access.ar](examples/bench/bench_field_access.ar)）は HEAD 1.021s → 1.075s で
  ノイズ水準（§7.2 の投影どおり Value 操作以外には効かない）。
- `size_of::<Value>()` は **32 バイトのまま**（`String` 24B → `Rc<str>` 16B と縮んだが最大変種は別）。
  回帰防止に単体テスト `value_stays_32_bytes` を追加した。

### 🔬 §7.4-3 文字列インターンは**消費者 0 件**（実測して保留）
§7.4-3 は「属性名・メソッド名を `Rc<str>` + ポインタ比較」だが、**比較する相手が居ない**。

- **属性読み**: R3 の `AttrCache` 命中時は `idx` 直読みで、名前引きは既に無い
  （[eval/core.rs](src/interpreter/eval/core.rs) `eval_attr`。`field_index.get(attr)` は `debug_assert_eq!` の中だけ＝release では消える）。
- **メソッド呼び出し**: IC 命中時も `class.methods.get(method_name)` が残るが、これは
  `HashMap` 引きであってポインタ比較に置き換わる形ではない（潰すなら IC に解決済みメソッドを載せる別施策）。

**判定に使った実測**（[bench_name_hash.ar](examples/bench/bench_name_hash.ar)）:
名前の**長さだけ**を変えた同形状のコードを比べた。文字列ハッシュはキー長に比例するので、
名前引きが効いているなら長い名前が遅くなるはず。

| | 8 文字名 | 60 文字名 | 差 |
|---|---:|---:|---:|
| 属性読み | 0.0717 s | 0.0726 s | +1.3%（ノイズ） |
| メソッド呼び出し | 0.1709 s | 0.1714 s | +0.3%（ノイズ） |

**差が出ない ＝ 名前引きは支配項に含まれていない。** #11 R2-c・#14・#15b と同じ
「消費者が居るか」の基準で保留とした。メソッド呼び出しの残コストは名前ではなく
呼び出し機構（#12・~630ns/call）側にある。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **697 緑**（`value_stays_32_bytes` を 1 件追加）／
  clippy **増分 0**（50 件で HEAD と同数・内訳も同一）／
  `compare_vm_modes.ps1` **identical 42 / differing 0** ／ `scan_examples.ps1` **FAIL 0** ／
  `dump_native_ir.ps1` 代表 6 モジュール **IR byte-identical**。
- 変更規模: 47 ファイル。`Value::Str` 参照 304 サイト。
- 追加した例題: [bench_string.ar](examples/bench/bench_string.ar)（A/B 基準）／
  [bench_name_hash.ar](examples/bench/bench_name_hash.ar)（#15-3 の判定プローブ）／
  [async_string_share.ar](examples/async/async_string_share.ar)（`deep_clone` の回帰検知）。
  前 2 つは [bench.ps1](bench.ps1) に登録済み。

### 次にここを触るなら
- `Value::Type(String)` / `Value::Trait(String)` / `Value::Protocol(String)` も同じ形で `Rc<str>` にできるが、
  これらは**生成頻度も clone 頻度も低い**（型値・trait 値を変数に入れて回すコードは稀）。着手前に件数を測ること。
- 文字列連結が支配的なワークロードでは `Rc<str>` は 1 回余分にコピーする。
  もし将来そこが問題になるなら、対象は §7.4-3 ではなく「連結結果を `String` のまま持てる表現」の検討になる。

---

## #15d 実行経路で「解決済み」判定を活かす（完了 2026-08-11）

### 着手前に再現した「実際の誤り」
計画書の指示どおり**先に例題で再現**してから直した。結果、**想定より重い欠陥**だった。

**(1) 組み込みがユーザーの束縛を横取りする** — モジュール最上位で `let repr = my_fn` としても
組み込みが呼ばれる。実測で **6 件**（`print` / `next` / `zip` / `enumerate` / `getenv` / `repr`）。
`len` / `id` はグローバル登録済みで `let` 自体が弾かれるため対象外。

**(2) トレースバックが `<anonymous>`** — しかも **`--vm=off` と `--vm=auto` で出力が食い違っていた**
（off=`in <anonymous>` / auto=`in boom`）。単なる不親切ではなく**モード間の不一致**。

### ⚠ 根本原因は「`res` を見れば済む」ではなかった
計画書は「`res` で組み込み振り分けを判定できる」と書いていたが、**`res` だけでは判定できない**。

`Resolution::Unresolved` は「**シャドウが無い**」を意味しない。リゾルバが処理するのは
**トップレベル関数の本体だけ**で、モジュール最上位・テンプレート本体・合成 AST は常に `Unresolved`。
（#15 の調査で「最上位は丸ごと `Unresolved`」と実測済み。）
つまり `Unresolved` へ限定する既存のゲートは、**最も壊れている場所を素通しする**構造だった。
実際、関数本体（`Resolution::Local`）でのシャドウは修正前から正しく動いていた。

→ 判定は `res` ではなく**実際の束縛**を見る。VM の
`is_vm_builtin(name) && !slots.contains_key(name)` と同じ規則に揃えた。

### 実装
- `Interpreter::builtin_is_shadowed` / `builtin_is_shadowed_global` を新設（[scope.rs](src/interpreter/scope.rs)）。
  `get_var` はクローンしないので判定は参照だけで済む。
- ツリーウォーク: 組み込み振り分けの前に `builtin_is_shadowed` を見る（[eval/calls.rs](src/interpreter/eval/calls.rs)）。
- トレースバック名: `call_name` を `res` 非依存にした（同ファイル）。
- VM: `Op::CallBuiltin` がグローバル側のシャドウを実行時に見る（[vm/run.rs](src/vm/run.rs)）。
  ローカルはコンパイル時に `slots.contains_key` で除外済みなので、実行時はグローバルだけでよい。

### ⚠ `Value::Type` を除外しないと `len(py_obj)` が壊れる（途中で踏んだ罠）
`register_builtin_globals`（[built_in_types.rs](src/interpreter/built_in_types.rs)）は
**`len` を `Value::Type("len")` としてグローバルに置いている**（ネイティブの `cb_get_global("len")` 用）。
素直に「グローバルに束縛があればシャドウ」と判定すると、`len()` が組み込み経路から
`call_type_by_name_evaled` へ逸れる。そちらには **`Value::PyObject` のアームが無い**ので
`len(py_obj)` が `TypeError` になり、組み込み経路を保つ VM 側とも食い違う
（エラー文言も `takes exactly one argument` vs `takes exactly 1 argument` で違う）。
→ **`Value::Type(t)` で `t == name` のものはシャドウとみなさない**（＝組み込み登録そのもの）。

### 🐛 VM 側にも鏡像のバグがあった（例題を書いて初めて出た）
ツリーウォークだけ直した時点で `nested -> user:from-fn`(off) / `'from-fn'`(auto) と食い違った。
VM の `slots.contains_key(name)` は**コンパイル中の関数のローカル slot しか見ない**ため、
**グローバルのシャドウを取りこぼす**。上記の `Op::CallBuiltin` 側の検査で解消。

### 実測（シャドウ検査のコスト・best-of-3）
組み込み呼び出しだけのループ（`len(s)` ×3／400k 回・他に何もしない極端な形）:

| 経路 | 修正前 | #15d | 差 |
|---|---:|---:|---:|
| 最上位（ツリーウォーク） | 0.1573 s | 0.1728 s | **+9.8%** |
| 関数内（VM `CallBuiltin`） | 0.1373 s | 0.1507 s | **+9.8%** |

標準ベンチ（field_access・bench_string）は誤差水準で変化なし。
**この 10% は「組み込み呼び出しが実行時間のほぼ全て」という人工的な条件でのみ出る。**

### 💭 フラグでコストを消す案は採らなかった
「どこかで組み込み名が束縛されたか」を `bool` で持てば検査をほぼ無料にできるが、
**束縛を作る全経路（`declare_var`・`insert`・import・デバッガ宣言…）でフラグを立てる不変条件**が生まれ、
1 箇所の漏れが**沈黙する正しさのバグ**になる。標準ベンチで差が出ない以上リターンが小さく、
「高リスク低リターンは保留」の基準に従って見送った。組み込み呼び出し支配のワークロードが
実際に問題化したときに再評価する。

### 🔍 副産物: `compare_vm_modes.ps1` は stderr を比較していない
[compare_vm_modes.ps1](compare_vm_modes.ps1) は stderr を `$tmpErr` にリダイレクトしているが
**読み出して比較しているのは stdout だけ**。トレースバックは stderr に出るため、
(2) のモード不一致は 45 例題を回しても検出されなかった。
→ 新タスク候補: **stderr も byte-identical 比較の対象に含める**（既存の差分がどれだけ出るかは未調査）。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **697 緑** ／ clippy **50 件で増分 0** ／
  `compare_vm_modes.ps1` **identical 45 / differing 0** ／ `scan_examples.ps1` **FAIL 0** ／
  `dump_native_ir.ps1` 代表 6 モジュール **IR byte-identical**（codegen は非変更）。
- 例題: [builtin_shadow.ar](examples/basics/builtin_shadow.ar)（最上位・関数本体・非シャドウの 3 系統）／
  [traceback_frame_names.ar](examples/exceptions/traceback_frame_names.ar)。
  ⚠ 同じ名前を外側と内側の両方で宣言することはできない（`already declared in an accessible scope`）ので、
  例題は名前を分けてある。

---

## #20 off/auto 比較に stderr と `_error` 例題を追加（完了 2026-08-11）

### 動機
[compare_vm_modes.ps1](compare_vm_modes.ps1) は stderr を `$tmpErr` へリダイレクトしながら
**読み出していたのは stdout だけ**だった。トレースバックは stderr に出るので、
#15d-2 の「off=`<anonymous>` / auto=`boom`」というモード不一致が **45 例題を素通り**した。
この系列の不変条件（off/auto byte-identical）を検査するはずの仕組みに穴が開いていた。

### 2 段に分けた理由（着手前の実測）
| 測定 | 結果 |
|---|---|
| 45 例題の stderr は既に一致しているか | **45 一致 / 0 不一致**（＝スクリプト改修だけなら即通る） |
| そのうち stderr が**空でない**もの | **4 件のみ**（intersection / mustbe / cpp_struct_ptr / swd_nested_runner） |
| 除外されていた `_error` 例題 20 件の内訳 | 静的エラー 11 ／ **実行時トレースバック 9** |

→ **stderr 比較を足すだけ（20-a）では歯が立たない**（4 例題しか発火しない）。
トレースバックを実際に踏むのは `_error` の実行時系 9 件で、そこが除外されていたのが本質。
よって `_error` の取り込み（20-b）まで含めて 1 タスクとした。

### 実装
- `Invoke-Example` が `[PSCustomObject]@{ Out; Err }` を返すようにし、**両ストリームを個別に比較**。
  差分表示は食い違ったストリームだけ出す（`[DIFF:stderr] runtime_error.ar` の形）。
- 例題選択から `_error` / `__errors` の除外を外した。**静的エラー系も含めてよい** —
  型検査は実行前に走るので両モードで自明に一致し、誤検出しない。名前リストの保守も不要になる。
- 旧挙動へ戻す `-SkipErrorExamples` を用意（45 例題・stderr 4 件で従来どおり動作を確認）。
- **`examples that produced stderr: N` を常時表示**。0 なら stderr 比較が一度も発火していない＝
  検査に歯が無い状態なので、気づけるようにした（今回まさにそれを見落としていたため）。

### 効果
| | 変更前 | #20 |
|---|---:|---:|
| 比較例題数 | 45 | **68** |
| stderr を出した例題 | 4 | **27** |

### 🧪 負の対照で検知力を確認した（推測ではない）
修正を 1 つずつ戻して、**実際に落ちること**を確かめた:

| 戻した修正 | 検知結果 |
|---|---|
| #15d-2（traceback 名） | **`[DIFF:stderr] runtime_error.ar`** で検出 |
| #15d-1（組み込み横取り） | **`[DIFF:stdout] builtin_shadow.ar`** で検出 |

変更前のスクリプトは #15d-2 を検出できなかったので、**この 2 件はいずれも新規に獲得した検知力**。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **697 緑** ／
  `compare_vm_modes.ps1` **identical 68 / differing 0** ／ `-SkipErrorExamples` で **45 / 0**（旧挙動）／
  `scan_examples.ps1` **FAIL 0**。
- PS 5.1 のため BOM 付き UTF-8 で保存（日本語コメントあり）。構文は `PSParser::Tokenize` で確認。

### 次にここを触るなら
`examples that produced stderr` が減ったら、**エラー経路の例題が減った＝検知力が落ちた**サイン。
#22（呼び出しディスパッチ再構成）はエラー文言とトレースバックを最も壊しやすいので、
この数字を着手前後で比べること。

---

## #15e 実行経路の `res` ゲート一掃（完了 2026-08-11）

### 判定に使った原則
#15d で得た教訓を一文にすると:

> **`res` は「どの記憶域から読むか」の最適化ヒントであって、「それが何を意味するか」の根拠ではない。**

`Resolution::Unresolved` は「未解決」ではなく実質「**リゾルバの対象外＝どこに書かれたか**」を表す
（対象はトップレベル関数の本体のみ。最上位・テンプレート本体・合成 AST は常に `Unresolved`）。
これを意味論の判定に使うと、**同じコードが書かれた場所で挙動を変える**。

この原則で 18 サイトを 2 分類し、**意味論側 13 サイトから `res` 条件を外した**。

| 分類 | サイト | 対処 |
|---|---|---|
| **最適化ヒント（残す）** | R4 呼び先キャッシュ（[eval/calls.rs](src/interpreter/eval/calls.rs) 2 箇所） | そのまま。解決済みなら別経路で速く引けるので、条件付きで正しい |
| **意味論（`res` を外す）** | [eval/native.rs](src/interpreter/eval/native.rs) 6 ／ [exec/mod.rs](src/interpreter/exec/mod.rs) 2 ／ [exec/vars.rs](src/interpreter/exec/vars.rs) 1 ／ [functions/args.rs](src/interpreter/functions/args.rs) 2 ／ `callee_display_name` 2 | 名前が欲しいだけなので `res` を問わない |

### 🐛 実バグ 2 件（着手前に例題で再現・修正後に解消を確認）

**(1) `mut → let` がコピーされない**（[exec/vars.rs](src/interpreter/exec/vars.rs)）
```
top    a = [1,2,3,4]  b = [1,2,3]      ← 最上位: 正しい
in-fn  a = [1,2,3,4]  b = [1,2,3,4]    ← 関数内: b が a を共有（誤り）
```
`let b = a` は深いコピー＋freeze のはずが、関数本体では元変数の可変性を調べずに素通ししていた。
off/auto 両方で同じ誤りなので `compare_vm_modes` では捕まらず、例題スキャンも通っていた。

**(2) C/C++ の OutPtr 書き戻しが関数内で起きない**（[eval/native.rs](src/interpreter/eval/native.rs)）
```
top   n = 5.0      ← 最上位: 正しい
in-fn n = 0.0      ← 関数内: 書き戻しされない（誤り）
```
`double* out` へ渡した `mut` 変数が、**関数本体からだと書き戻し登録（`out_wb`）に積まれない**。
既存例題 [cpp_struct_ptr.ar](examples/interop/cpp_struct_ptr.ar) は (3) が最上位だったため通っていた。
FFI の out パラメータが黙って機能しないという、外から見えにくい種類の欠陥。

### 調べて「バグではなかった」もの（記録）
- **`let` を書き込みポインタへ渡す拒否**（native.rs:40）: 関数内でも
  **静的型検査が先に捕まえる**（`parameter 'out_len' of 'v3_norm' expects a mutable argument`）。
  実行時チェックは二重の網であり、`res` で飛んでいても表に出なかった。
- **クロージャ／async のキャプチャ**（`collect_refs_expr`）: `collect_referenced_names` は
  `capture_env`（[exec/blocks.rs](src/interpreter/exec/blocks.rs)）と VM の async ブロックが使う。
  `res` で名前を落とすとキャプチャ漏れになるが、**リゾルバが入れ子定義の本体と
  `Stmt::AsyncAssign` に踏み込まない**ので現状は安全だった（resolver.rs に明記あり）。
  ただしこれは**隠れた結合**で、#21 でリゾルバを広げると黙って壊れる。今回 `res` 条件を外して解いた。
- **引数の可変性判定**（[functions/args.rs](src/interpreter/functions/args.rs)）: 既定が
  `is_mutable = true`（＝保守的にコピー）なので取りこぼしても正しい。**最適化の取りこぼしのみ**。
  ついでに条件を外したので、解決済みでもコピー省略が効くようになった。

### 実測
- `bench_field_access`: 1.045〜1.083 s（#15d 時点 1.014〜1.055 s）。**ノイズ〜微減**。
  `res` を外したぶん名前引きが増えるのは「native 呼び出し・`mut→let` 代入・FFI エラー生成」に
  限られ、いずれもホットループの本体ではない。
- IR は **byte-identical**（codegen 非変更）。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **697 緑** ／ clippy **50 件で増分 0** ／
  `compare_vm_modes.ps1` **identical 69 / differing 0**（stderr 発火 27）／ `scan_examples.ps1` **FAIL 0**。
- 例題: [mut_to_let_copy.ar](examples/basics/mut_to_let_copy.ar)（新規・list/dict・最上位と関数内）／
  [cpp_struct_ptr.ar](examples/interop/cpp_struct_ptr.ar) にケース (4)「関数本体での OutPtr 書き戻し」を追加。
- 不要になった `use crate::ast::Resolution;` を 4 ファイルから削除。

### 次にここを触るなら
残した 2 サイト（R4 呼び先キャッシュ）は**最適化なので `res` に依存してよい**が、
「解決済みなら別経路で正しく処理される」という前提の上に立っている。#22-d で呼び先の同定を
1 段に括るときに、この前提ごと構造で表現するのが本筋。

---

## #21 最上位が丸ごと `Unresolved` な件 — 調査と判断（2026-08-11）

### 何が欠けているか（実現可能性は高い）
`resolve_program`（[resolver.rs](src/interpreter/resolver.rs)）はトップレベルの `FnDef`/`GenDef` に対して
`resolve_function` を呼ぶだけで、**最上位の文列そのものを書き換えていない**。
一方 `collect_program_globals` は**最上位の名前を既に全て集めており**、
`Resolution::Global(SlotCache)` も R2-b で実装済み。
つまり不足しているのは「最上位文列に対して rewrite を走らせる」ことだけで、新機構は要らない。

### 🔬 実測（`--vm=off`・best-of-3・1M 回ループ）

**(1) 最上位 vs 関数内の差（回収余地の上限）**
| | 時間 |
|---|---:|
| A. 最上位（全て `Unresolved`） | 0.262 s |
| C. 関数内ローカル（`Local(slot)`） | 0.222 s |

→ 上限 **1.18x**。

**(2) 最初の三点比較は誤読しかけた**
関数内からグローバルを読み書きする形（B）を測ると **0.293 s** で、
**最上位（A）より遅い**という結果が出た。これだけ見ると「#21 は逆効果」に見える。

**(3) 切り分け: 遅さの原因は解決方式ではなく「グローバルへの書き込み」だった**
ループ変数・累算器を両方ローカルに揃え、**読む対象だけ**を変えて測り直した:

| | 時間 |
|---|---:|
| `Global(SlotCache)` 読み | **0.450 s** |
| `Local(slot)` 読み | 0.456 s |

→ **読み取りは同速**（誤差内で Global がわずかに速い）。(B) の遅さは
グローバル代入（`SlotCell` = `Rc<RefCell>` 経由＋epoch 検証）のコストであって、
`Resolution::Global` のコストではない。

**この切り分けをしないと「#21 は効果なし」と誤って保留にしていた。**

### 判断: **実装する価値がある**（ただし優先度は #22 の後でよい）
- **効果は実在する**。最上位の書き込みは `Stmt::Assign` の `SlotCache`（R2）で既に索引化済みなので、
  残っている差は**読みが名前引きであること**に由来する。そこを `Global` にすれば (1) の 1.18x をほぼ回収できる。
- **機構は既存**（`collect_program_globals` ＋ `Resolution::Global`）。新しい概念を足さない。
- **#15e で意味論の結合は解いた**。以前なら最上位を解決対象にした瞬間に
  クロージャ／async のキャプチャが黙って壊れていた（`collect_refs_expr` が `res` を見ていたため）。

### ⚠ 実装時に必ず踏む注意点
- **最上位の入れ子ブロックで束縛される名前を `Global` にしてはいけない**。
  `for i in ...` のターゲットや `if:` 内の `let` は `scopes[0]` に居ない。
  関数側で使っている `collect_bound_names` の保守的収集をそのまま適用すること
  （resolver.rs に「`collection.ar` の set を for ターゲットが覆って落ちた」実例あり）。
- 効果が出るのは**最上位に重いループがある場合のみ**。
  リポジトリの実プログラムは**全て 150ms 未満（起動支配）**で、現状の受益者は
  `examples/bench/` のベンチ群だけ。ただしそれらは**プロジェクト自身が自然に最上位へループを書いた**
  結果でもあり、スクリプト言語として典型的な書き方でもある。

### 依存
- **#22-d をブロックしない**。#15e で「`res` は意味論の根拠にしない」原則が確定したので、
  #22-d は「最上位が解決済みかどうか」に依存しない設計になる。
  当初「#21 → #22-d」と依存を引いたが、**#15e の完了によりこの依存は解消された**。

---

## #22-a 呼び出しディスパッチの分類（完了 2026-08-11）

### 分類の枠組み（3 つの直交軸）
現在 [eval_call](src/interpreter/eval/calls.rs) は性質の違う 3 つの判断を 1 本の if-chain に潰している。

| 軸 | 内容 | 実装箇所 | 重複 |
|---|---|---|---|
| **A. 呼び先の同定** | AST の形（`Ident`/`Attr`/`TemplateInstantiate`）＋記憶域＋各種キャッシュ | ツリーウォーク `eval_call` 前段 5 ゲート ／ VM コンパイラの `res` 3 分岐×`Unresolved` 内 5 分岐 | **2 実装** |
| **B. 呼び先の正規化** | テンプレート実体化・オーバーロード選択・`Class`→`__init__`・`Instance`→`__call__` | A と C の間に散在 | — |
| **C. 実行方式** | ← ユーザー提案の分類 | **3 実装**（下記） | **3 実装** |

### C 軸は 3 箇所に別々のアーム集合で実装されている
| `Value` 変種 | `eval_call` | `call_value_evaled`（**VM が使う**） | `Namespace` アーム |
|---|:---:|:---:|:---:|
| `Function` / `OverloadedFn` / `Class` / `GeneratorFn` / `NativeFunction` / `PyObject` | ✅ | ✅ | ✅ |
| `Type`（型コンストラクタ） | ✅ | ✅ | ❌ |
| `Instance`（`__call__`） | ✅ | ✅ | ❌ |
| **`JsProcFn`** | ✅ | **❌→修正** | ✅ |
| `Protocol` / `TemplateFn`（エラー文言） | ✅ | ❌ | ❌ |
| `Namespace` | ❌ | ❌ | ✅ |

### 🐛 発見 1: `JsProcFn` 欠落による off/auto 不一致（修正済）
`call_value_evaled` に `Value::JsProcFn` アームが無く、**VM の `Op::Call` はこの関数を使う**ため:
```
let f = js_mod.func        # 関数値に束縛
fn g() -> str: return f(x) # VM 化された関数内から呼ぶ
```
が `--vm=off` では通り、`--vm=auto` では **`TypeError: 'function' object is not callable`**。
アームを追加して解消（実測で両モード `z.ar` 一致を確認）。

`compare_vm_modes.ps1` が検出できなかったのは、js-proc 例題が**スキップリストにある**ため
（Node ブリッジが常駐して終了しない）。#20 の強化でも届かない領域。

### 🐛 発見 2: VM 経路では FFI 境界検査が丸ごと効いていない（未修正）
`check_ffi_return`（#16 の成果）の呼び出しは **`eval_call` の 3 箇所だけ**で、
`call_value_evaled` からは一度も呼ばれない。検査は**型検査が Call ノードへ焼いた宣言型**を
`node_id` 索引で引くため、`node_id` を持たない `call_value_evaled` では**原理的に行えない**。
→ `PyObject` / `JsProcFn` を値経由で VM から呼ぶと、**スタブが宣言した型との突き合わせが働かない**。
C 軸を 1 本化し `node_id` を実行方式ディスパッチまで運ぶ #22-b で解消する。

### ユーザー提案の 5 分類への写像 — **4 分岐で閉じる**
| 提案分類 | 現在の `Value` 変種 | 判定 |
|---|---|---|
| 1. 組み込み | `Value::Type`（型コンストラクタ）＋ `eval_builtin_ident_call` | **2 箇所に分裂**。統合対象 |
| 2. 非コンパイルの Arrow 関数・メソッド | `Function` / `OverloadedFn` / `Class` / `GeneratorFn` / `Instance` | B 軸で 1 つへ正規化できる |
| 3. コンパイル済み Arrow | `NativeFunction` | **4 と同一表現** |
| 4. 直接読める外部ライブラリ | `NativeFunction` | **3 と統合済**（[NativeFnRef](src/interpreter/value/native.rs) が C ABI シンボル呼びを一本化） |
| 5. 翻訳機経由 | `PyObject` / `JsProcFn` / `CsObject` | 言語別だが「ブリッジを挟む」で 1 分岐 |

**結論: C 軸は 4 分岐（1/2/3+4/5）で閉じる。** 提案の 3 と 4 を分ける必要はない
（差は出自であって呼び出し経路ではない）。C# DLL は形式が DLL でも**ブリッジ経由なので 5**。

### 閉じない残余（C の下のサブ種別として持つ）
分類 2 の中に「**到達方法ではなく起きることが違う**」ものが残る。分岐は減らないが A/B から切り離せる:
- 通常関数: 本体を実行して戻り値
- ジェネレータ関数: 本体を eager 実行して `Value::Generator` を返す
- コンストラクタ: インスタンス確保 ＋ `__init__`

`Protocol` / `TemplateFn` のアームは**実行方式ではなく「B 軸が未実施」を検出しているだけ**なので、
B を独立段にすればここから消える。

### 22-b 以降への含意
- **C の 3 実装を 1 本化**すれば、発見 1（アーム集合のずれ）は構造的に起きなくなる。
- **`node_id` を C まで運ぶ**設計にすれば、発見 2（FFI 境界検査の欠落）も同時に解消する。
- VM は C 軸を**自前で持たず `call_value_evaled` に委譲済み**なので、統合先はそこで良い。
  VM が重複を持っているのは **A 軸だけ**。

---

## #22-b C 軸（実行方式）の統合（完了 2026-08-11）

### やったこと
実行方式ディスパッチの**3 重実装を 1 本化**した。統合先は `call_value_evaled`
（[eval/calls.rs](src/interpreter/eval/calls.rs)）— VM の `Op::Call` が既にここへ委譲していたため。

- `eval_call` の 11 アーム match を**委譲 1 行**へ置換。
- `check_ffi_return` を `&Expr` ではなく**表示名 `&str`** を取る形に変え、C 軸から呼べるようにした。
- `call_value_evaled` に `node_id` を追加し、`PyObject` / `JsProcFn` で FFI 境界検査を実行。
- **`Op::Call` に `node_id` フィールドを追加**（`(argc, mut_mask, name_idx, span_idx, node_id)`）。
  これが無いと VM 経路に宣言型が届かない。

### C 軸へ渡せない呼び先 — 3 つだけ残した（理由つき）
統合の要点は「**何が本当に C 軸ではないか**」を見極めること。以下は `eval_call` に残す:

| 呼び先 | 残す理由 |
|---|---|
| `Function` | **A 軸**の後処理（R4 グローバル slot の焼き込み）が「どこから呼ばれたか」を要する |
| `NativeFunction` | `mut` ポインタの **write-back に引数の式が要る**（評価済みの値では書き戻し先が判らない）。typed IC の焼き込みも引数が全て位置引数かを式で見る |
| `Type` | `AsyncManager(num_thread=3)` が**キーワード引数**、`Signal[T]()` がテンプレート形。評価済み値ベクタではキーワード名が落ちる |

### 🐛 統合中に 2 つの回帰を検出した（どちらも直した）
- **`Value::Type` を委譲したら壊れた**。`call_type_by_name_evaled` には `AsyncManager`/`Signal` のアームが無く、
  コンパイラが「**`Value::AsyncManager` is never constructed**」と警告して露見した。
  デッドコード警告が意味論の回帰を捕まえた形。
- **`TemplateFn` のエラー文言が 2 種類あった**。統合時に片方へ揃えた
  （`TemplateError: template must be called with explicit type arguments ...` を採用）。

### 🧪 負の対照で「VM 経路の FFI 境界検査欠落」を確認した
`Op::Call` が渡す `node_id` を一時的に `0` に戻すと:

| モード | 結果 |
|---|---|
| `--vm=off` | `FfiTypeError: ... declared to return int but returned str` |
| `--vm=auto` | **`I am a string` を素通しして継続** |

＝ **モードによって静かに誤った値が Arrow へ流れ込む**状態だった（例題の言う「最悪の失敗形」）。
修正後は両モードとも `FfiTypeError`。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **697 緑** ／ clippy **50 件で増分 0** ／
  `compare_vm_modes.ps1` **identical 70 / differing 0**（stderr 発火 28）／ `scan_examples.ps1` **FAIL 0**。
- 例題: [ffi_boundary_value_call_error.ar](examples/interop/ffi_boundary_value_call_error.ar)（新規・
  関数値経由の py 呼び出しで両モードとも境界検査が効くこと）。

### 22-c / 22-d への申し送り
- 残した 3 例外のうち **`NativeFunction` と `Type` は「引数を式のまま渡す必要がある」**という同じ理由。
  22-c（B 軸の分離）で「引数の正規化」を独立段にすれば、この 2 つも C 軸へ寄せられる可能性がある。
- `Function` の例外は純粋に A 軸（キャッシュ焼き込み）なので、22-d で A 軸を括れば解消する。
- `eval_method_call` の `Namespace` アームは**まだ独自のアーム集合を持っている**（未統合）。
  ここも `call_value_evaled` へ寄せるのが 22-c の一部。

---

## #22-c B 軸の分離 — C 軸の統合を完了（2026-08-12）

### やったこと
22-b で残っていた **C 軸の 3 例外のうち 2 つを解消**し、3 つ目の重複実装も畳んだ。

| 対象 | 変更 |
|---|---|
| `eval_method_call` の `Namespace` アーム | **C 軸へ委譲**（22-a が数えた 3 つ目の実行方式ディスパッチを解消） |
| `Value::Type` | `call_type_constructor_evaled` を新設して**キーワード引数を保持したまま**渡し、C 軸へ移動 |
| `instantiate` / `instantiate_evaled` | 2 実装 → **`instantiate` は引数を評価して `instantiate_evaled` へ委譲**する 1 実装へ |

`eval_type_constructor_call` / `make_async_manager`（CallArg 版）は**消費者が消えて削除**できた
＝ C 軸へ寄せられたことの機械的な裏付け。

### 🐛 委譲で回帰を 1 件出し、検査が捕まえた
`Namespace` アームを委譲した直後、`scan_examples.ps1` が
**`FAIL cs_interop_test.ar: TypeError: function takes 0 argument(s), got 1`** を検出。

原因は 22-b の `Value::Type` と**まったく同じ形**だった:
`instantiate`（CallArg 版）だけが **cs-dll / cs-proc ブリッジのコンストラクタ分岐**を持ち、
`instantiate_evaled` には無かった。委譲した結果、C# のコンストラクタ呼び出しが
引数を取らない Arrow 側 stub `__init__` へ流れて落ちた。
→ ブリッジ分岐を `instantiate_evaled` へ移植し、`instantiate` はその委譲に一本化。

**「同じ処理の CallArg 版と evaled 版がずれている」がこの系列の反復パターン**である
（#22-a の `JsProcFn`、22-b の `AsyncManager`、22-c の cs ブリッジ）。
片方を他方への委譲にして**実装を 1 つに畳む**のが唯一の恒久策。

### 残る唯一の例外: `NativeFunction`（意図的に据え置き）
`mut` ポインタの write-back が **引数の「元の変数名」**を要る
（[eval/native.rs](src/interpreter/eval/native.rs) の `writebacks.push((n.clone(), h))`）。
評価済み引数は `(Option<String> /*キーワード名*/, Value, bool /*可変か*/)` の 3 つ組で、
**元の変数名を持っていない**。

- **解くには**: この 3 つ組へ `source_name: Option<String>` を足す（B 軸の引数正規化）。
- **コスト**: この型は **27 箇所**の署名に現れる。
- **リターン**: 例外が 1 つ減るだけで速度効果は無い。しかも触るのは
  **#15e で実バグ（OutPtr 書き戻し漏れ）が出たまさにその経路**。
- → 「高リスク低リターンは保留」の基準で見送り。着手するなら**引数の 3 つ組を
  名前付き struct にする独立タスク**として切り出すこと。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **697 緑** ／ clippy **50 件で増分 0** ／
  `compare_vm_modes.ps1` **identical 70 / differing 0** ／ `scan_examples.ps1` **FAIL 0** ／
  IR **byte-identical**（codegen 非変更）。

---

## #22-d A 軸の整理（完了 2026-08-12）— #22 系列の締め

### やったこと 1: 組み込み振り分けから `res` を外した
```rust
// before
if let Expr::Ident { name, res: Resolution::Unresolved, .. } = func {
    if !self.builtin_is_shadowed(name) { ... }
// after
if let Expr::Ident { name, .. } = func {
    if !self.builtin_is_shadowed(name) { ... }
```
**この `res` 条件は実質無条件だった。** 組み込み名（`print`/`len`/…）は AST で宣言される名前ではないので
リゾルバの `globals` に載らず、`res` は常に `Unresolved` になる。にもかかわらず条件に書かれていたため
「`res` を見て組み込みか判定している」と読めてしまう — #15d でまさにその誤読をした箇所。
判定の根拠は `builtin_is_shadowed`（実際の束縛）だけで足りる。

なお `let repr = f` を最上位で宣言した場合、関数本体からの参照は `res == Global` になるが、
`builtin_is_shadowed` が束縛を見つけるので結果は同じ（[builtin_shadow.ar](examples/basics/builtin_shadow.ar) で確認）。

### やったこと 2: 畳めない A 軸の重複を**テストで固定**した
組み込み呼び出しの判断は VM コンパイラ（`is_vm_builtin`）とインタプリタ（`eval_builtin_evaled`）の
**2 箇所**にあり、コンパイル時と実行時なので**畳めない**。しかし集合がずれると
`CallBuiltin` を発行したのに実行側が `None` を返し **`NameError` で落ちる**（VM 経路だけ＝off/auto 不一致）。

→ 名前集合を `VM_BUILTIN_NAMES` 定数に切り出し、
`vm_builtin_names_are_all_handled`（[tests/mod.rs](src/interpreter/tests/mod.rs)）で
「VM が発行する全名前を `eval_builtin_evaled` が扱う」ことを検査する。

**負の対照で検知力を確認**: `VM_BUILTIN_NAMES` に `"open"`（`eval_builtin_ident_call` にはあるが
`eval_builtin_evaled` には無い名前）を足すとテストが落ち、修正方法まで示すメッセージが出た。

### この系列の結論 — 重複への対処は 2 通りしかない
| 状況 | 対処 | 本系列での適用 |
|---|---|---|
| 同じ入力から同じ判断をする 2 実装 | **片方を他方への委譲にして畳む** | 22-b（`eval_call`）/ 22-c（`Namespace`・`instantiate`）/ `Type` |
| 前提が違って畳めない（コンパイル時 vs 実行時） | **不変条件をテストで固定する** | 22-d（`VM_BUILTIN_NAMES`） |

「同じ処理の CallArg 版と evaled 版がずれている」は #22-a/-b/-c で **3 回**実バグを生んだ。
新しく `*_evaled` 版を足すときは、**必ず片方をもう片方の委譲にすること**。

### 残る `res` 参照（7 箇所）— すべて最適化ヒント
実行経路に残る `res: Resolution::Unresolved` は 7 箇所で、**意味論を決めているものは無い**:
- `eval/calls.rs` 2 箇所: R4 呼び先キャッシュの命中判定と充填（解決済みなら別経路で速く引ける）
- `vm/compiler.rs` 5 箇所: コンパイル時の slot 索引（`self.slots.get(name)`）。
  `res` は「リゾルバが slot を割り当てたか」の問い合わせそのもので、**A 軸の内部**に閉じている。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **698 緑**（不変条件テスト +1）／ clippy **50 件で増分 0** ／
  `compare_vm_modes.ps1` **identical 70 / differing 0** ／ `scan_examples.ps1` **FAIL 0** ／
  IR **byte-identical**。

---

## #21-b リゾルバを最上位文列へ広げる（完了 2026-08-12）

### 実装
`resolve_program` に `resolve_toplevel` を追加（[resolver.rs](src/interpreter/resolver.rs)）。
最上位の `Expr::Ident` に `Resolution::Global` を付ける。

**関数側（`resolve_function`）との決定的な違いは 2 つ**:
1. **base slot が無い**。最上位の宣言は `scopes[0]` へ入るので `Local` は決して付かない。
2. **直下宣言を差し引いてはいけない**。関数では「本体の宣言はグローバルを覆う」が、
   最上位では**直下宣言こそがグローバル**。`collect_bound_names` をそのまま使うと
   直下宣言まで拾って差集合が空になり、**何も解決されない**。

そこで `collect_toplevel_shadowing` を新設し、覆いうる名前だけを集める:
- 直下宣言（`Let`/`Mut`/`Const`/`Static`/`LetTuple`）→ **名前は拾わない**が初期化式の中は見る
- 入れ子定義（`FnDef`/`ClassDef`/…）→ 本体は**別フレーム**なので無視
- それ以外（for ターゲット・if/while/block/match の中）→ `collect_bound_names` で保守的に全部

### 効果（`--vm=off`・best-of-3・[toplevel_gap.ar]）
| | 21-a 測定 | #21-b |
|---|---:|---:|
| 最上位ループ | 0.262 s | **0.223 s** |

**1.19x** — 21-a が予測した上限（1.18x）どおり。
最上位の書き込みは `Stmt::Assign` の `SlotCache`（R2）で既に索引化済みだったので、
残っていた差は「読みが名前引きであること」だけ、という 21-a の分析が裏付けられた。

### 併せて入れた高速パス（純粋な無駄取り）
最上位が `Global` になると、呼び出しのたびに `builtin_is_shadowed` のスコープ走査が走る。
**`res` が解決済み ⟹ 必ずユーザーの束縛がある**（リゾルバが `Local`/`Global` を付けるのは
AST で宣言された名前だけで、組み込み名は宣言できない＝`let len = ...` は弾かれる）ので、
`builtin_is_shadowed(name) == true` と**同値**。よって解決済みなら検査を飛ばす。
意味論の判定ではなく `res` の最適化ヒントとしての正しい使い方（#15e の原則に適合）。

### ⚠ 退行の誤帰属を切り分けた（記録）
`bench_field_access` が #15e 時点の 1.045〜1.083 s から 1.10〜1.16 s に見えたため
21-b の退行を疑ったが、**`resolve_toplevel` の呼び出しをコメントアウトしても同じ値**だった
（1.103〜1.138 s）。＝ 21-b 由来ではない（#22 系列かマシン変動）。
一度コメントに「21-b が 1.05→1.16 に退行させたので直した」と書いたが、**事実に反するので訂正済み**。
**A/B は必ず当該変更だけを切り替えて取ること。**

### 受益者について（正直な範囲）
効果が出るのは**最上位に重いループがある場合のみ**。
`bench_string` / `bench_field_access` は誤差水準で変化なし（名前引きが支配項ではないため）。
リポジトリの実プログラムは全て 150ms 未満（起動支配）で、現状の受益者はベンチ群だけ。
ただしスクリプト言語として最上位にループを書くのは典型的で、プロジェクト自身のベンチも
自然にそう書かれている。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **698 緑** ／ clippy **50 件で増分 0** ／
  `compare_vm_modes.ps1` **identical 71 / differing 0** ／ `scan_examples.ps1` **FAIL 0** ／
  IR **byte-identical**。
- 例題: [toplevel_global_shadow.ar](examples/basics/toplevel_global_shadow.ar)（新規）—
  for ターゲット・if・while・ブロック式で**覆われた名前が内側の束縛を読む**ことを確認。
  ここを取り違えると「グローバルを読むべきでない位置でグローバルを読む」バグになる。

---

## #12 呼び出し機構の高速化（完了 2026-08-12）— **フレームスタック改修なしで達成**

### 計測が設計を覆した
計画書は #12 を「R0-A 明示フレームスタック（`Rc<Frame>` のスタック化）」と定義し、
支配項を「**フレーム構築** ~630ns/call」としていた。着手前に実測したところ:

1. **現在の呼び出しコストは 0.360 µs**（630ns は Phase 0 の値で、R 系列で既に削減済み）。
2. `--vm=auto`（0.360）と `--vm=off`（0.386〜0.404）の差が僅か
   ＝ **コストは本体実行ではなく呼び出し設定側**にある。
3. `run_vm_method` / `exec_fn_evaled` を読むと、**フレームとは無関係な per-call のヒープ確保**が 2 つあった。

**結果、フレームスタックには一切触らずに 2.57x を得た。**

### 原因 1: `build_caller_frame` を成功時にも作っていた（2.08x）
呼び出し元フレームは**エラー経路でしか使わない**のに、`match result` の前で無条件に構築していた。
その中身は per-call で:
- `call_stack.last().cloned()` — String 確保
- `span.file.to_string()` — String 確保
- `get_context_lines(file, line, 5)` — **ソース 5 行を `join("
")`** で String 確保

→ 各エラー分岐へ移動（遅延化）。**0.360 → 0.173 µs（2.08x）**。

### 原因 2: `call_stack.push(fn_name.to_string())`（さらに 1.24x）
呼び出しごとに関数名の String を確保していた（実測 ~43ns/call。
一時的に `String::new()` へ置換して 0.173 → 0.130 と切り分け）。

→ pop したバッファを `call_name_pool` に取っておき、push 時に `clear()` + `push_str()` で詰め直す。
**定常状態で確保 0**。`call_stack` の型と `len()` の意味は変えないので、
深さを見るデバッガ（`debugger.rs` 4 箇所）や例外フレーム生成に影響しない。

### 実測まとめ
| 指標 | 前 | #12 | 倍率 |
|---|---:|---:|---:|
| `fn call (no args)` 呼び出しオーバーヘッド | 0.360 µs | **0.138 µs** | **2.61x** |
| `let→let int` | 0.453 µs | 0.221 µs | 2.05x |
| `4-field read` | 0.570 µs | 0.351 µs | 1.62x |
| `bench_field_access`（E2E） | 1.10 s | **0.81 s** | **1.36x** |

### ⚠ トレースバックは壊していない
`call_stack` の内容と `build_caller_frame` はトレースバック生成の中核なので、
多段呼び出しの未捕捉例外で **off/auto とも従来と同一の 3 フレーム**が出ることを確認した
（`in <module>` → `in middle` → `in boom`）。
`compare_vm_modes.ps1` は #20 で **stderr も比較**するようになっているので、
71 例題の回帰検査にもトレースバックが含まれている。

### 「フレームスタック」本体はどうするか
**当初の #12（`Vec<Rc<Frame>>` 化）は未実施**。今回の計測で分かったのは:
- 呼び出しオーバーヘッドの支配項は**フレーム構築ではなかった**（per-call のヒープ確保だった）。
- 残る 0.138 µs の内訳は未分解。フレーム構築がどれだけ占めるかは**未計測**。

→ フレームスタック改修を再提案するなら、**まず残り 0.138 µs を分解**すること。
§4.2 の記述（「R0 ストレージ改修が Phase R の主作業」）は Phase 0 の測定に基づいており、
**現在の支配項を指していない可能性が高い**。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **698 緑** ／ clippy **50 件で増分 0** ／
  `compare_vm_modes.ps1` **identical 71 / differing 0**（stderr 込み）／ `scan_examples.ps1` **FAIL 0**。

---

## #12b / #2c 保留の判断（2026-08-12）

### 依存が循環している
`12b → 2c` は「12b が価値を解放する」関係ではない。§3.4-A 自身がこう書いている:

> **トレードオフ**: 呼び出しごとにフレームを**ヒープ確保**するため、呼び出しパスの伸びは ~2x
> **後段最適化（V-F）**: 非エスケープフレームだけフラット確保に落として**呼び出し速度を回収**

＝ **#12b が損失を作り、#2c がそれを取り戻す**。12b をやらなければ 2c は不要。
`← 12b` を前提に持つタスクは **#2c だけ**（実測: 計画書内で 1 件）。
「他タスクの前提だから着手する」という理由は、この循環のため成立しない。

### VM 経路は既にフラットで Scope を使わない
`run_vm_method` / `try_fast_bind` は `self.vm_stack`（共有 flat buf）を使い、
**`scopes` を一切参照しない**（grep で 0 件）。
＝ #2c が目指す「非エスケープフレームのフラット確保」は **VM 適格関数では既に達成済み**。
model A が影響するのは**ツリーウォーク側だけ**で、そこは D2（強制バイトコード）が実現すれば
通常経路から外れる層。

### 速度理由も消えた
#12 で呼び出しオーバーヘッドは 0.360 → 0.138 µs（2.61x）。
真因は per-call のヒープ確保 2 件であって**フレーム構築ではなかった**。
残 0.138 µs のうちフレーム構築が占める割合は**未計測**。

### 「クロージャだけフレーム化」（中間案）は成立しない
> クロージャにフレームごと掴ませるには、そのフレームが参照カウント管理されている必要がある。

`scopes: Vec<Scope>` は Scope を**値で所有**しているので一部だけ共有できない。
共有可能にした時点で `Vec<Rc<Scope>>` ＝ **A そのもの**になる。
無理に中間を作ると「クロージャ生成時に該当スコープだけ `Rc` へ退避、残りは値のまま」という混在になり、
§3.4-A が A を採る理由として挙げた「**open/closed upvalue 開閉処理**」「**直値/セル混在**」を
自分で作り直すことになる（現状の `Var::Cell` 昇格より悪い）。

### A 自体にも計画書に無い隠れコストがある
フレームは実行中ずっと可変で、`self.scopes` への参照は **50 箇所（うち可変 16 箇所）**。
`Rc<Frame>` にすると:

| 方式 | 代償 |
|---|---|
| `Rc<RefCell<Frame>>` | **ローカルアクセスのたびに borrow フラグ検査**＋入れ子 borrow の panic リスク |
| `Rc::get_mut`（refcount==1 のみ可変） | クロージャに掴まれた瞬間に経路が分かれる＝**open/closed 相当の分岐が復活** |

＝ A は `Var` 変種 4→2 と `capture_env` 53 行を減らす代わりに、
**別の場所に同等の分岐か実行時検査を持ち込む**。純粋な単純化ではない。

### 結論
| 観点 | 判定 |
|---|---|
| 速度 | **不要**（#12 で達成済み・VM 経路は無関係） |
| 依存 | **循環**（2c は 12b のためだけに存在） |
| 構造 | **中立〜微減**（変種は減るが borrow 検査か分岐が増える） |
| 対象範囲 | ツリーウォークのみ（D2 で通常経路から外れる層） |

→ **#12b・#2c とも保留**。再訪するなら、着手理由は「依存」ではなく
**「残 0.138 µs/call の分解でフレーム構築が支配的と出たとき」**か、
**「クロージャ関連のバグが実際に出て `Var::Cell` 昇格が原因と特定できたとき」**に限る。

---

## #2b V-F 単型算術命令（完了 2026-08-12）— **新 op ゼロ・穴は文の側にあった**

### 計測が着手先を変えた（2 回目）

計画書は #2b を「型注釈から**専用 op を emit**」と定義していた。着手前に `src/vm/op.rs` を読むと、
**型特化 op は #16 段階(b) で既に出揃っていた**（`IntBinLL`/`IntBinLC`/`FloatBinLL`/`FloatBinLC`/
`IntBinSS`/`FloatBinSS`）。しかも `int_binop_specialized` は Div/FloorDiv/Mod/Pow/bit まで対応済みで、
`op.rs` の doc コメント（「Add/Sub/Mul と比較のみ」）の方が陳腐化していた。

そこで「専用 op を足す」ではなく「**既にある特化 op に乗っていない経路はどれか**」を探した。
`AR_VM_DUMP=1` で算術ループを逆アセンブルしたところ、一目で出た:

```
   8  IBIN_LL 2 3 Add        ← acc = acc + k は特化済み
   9  STORE_LOCAL 2
  10  LOAD_LOCAL 1           ┐
  11  CONST 3 = Some(Int(1)) │ i += 1 だけが 4 命令＋汎用 Bin
  12  BIN Add                │
  13  STORE_LOCAL 1          ┘
```

### 原因: `Stmt::CompoundAssign` が型検査の注釈対象から漏れていた

- `Expr::BinOp` は `node_id` を持ち、型検査（[infer.rs](src/type_check/infer.rs)）が `binop_kind` を焼く。
- `Stmt::CompoundAssign` は **`node_id` を持っていなかった**ため注釈が焼けず、
  VM コンパイラ（[compiler.rs](src/vm/compiler.rs)）も融合を試みずに `LoadLocal; e; Bin; StoreLocal` を直に emit していた。
- 型検査の `CompoundAssign` アームは**左辺型（`lookup(name)`）と右辺型（`infer(value)`）を両方持っていた**のに、
  それを捨てていた。

⚠ この形は #22 系列で繰り返し出た「同じ判断が 2 箇所にあってずれる」の**変種**。
今回は「片方が判断を**そもそもしていない**」パターンで、off/auto 比較にも例題スイートにも映らない
（挙動は同じで速度だけが違うため）。

### 実装（新 op ゼロ）

| 層 | 変更 |
|---|---|
| `ast.rs` | `Stmt::CompoundAssign` に `node_id: u32` を追加 |
| `parser/stmts/assignment.rs` | `parse_compound` で `next_node_id()` を採番 |
| `templates.rs` / `python_converter` | 原型から引き継ぎ／`0`（未採番）で追従 |
| `type_check/annotations.rs` | 判断を **`BinOperandKind::of(lt, rt)` に集約**し `Expr::BinOp` 側もそこへ委譲 |
| `type_check/stmt/check.rs` | `CompoundAssign` で `binop_kind` を焼く |
| `vm/compiler.rs` | `emit_bin_fused_slot` / `specialized_bin_kind_slot` / `gate_bin_kind` を切り出し、`CompoundAssign` から**同じ経路へ委譲** |

`x <op>= e` は VM 上では `x = x <op> e` と**同じ命令列になる**（`StoreLocal` は deep_copy しない）ので、
`Expr::BinOp` 用の融合＋特化をそのまま通せた。**新しいオペコードは 1 つも要らなかった。**

⚠ 評価順: 融合後は「右辺を用意してから左辺 slot を読む」形になるが、融合対象の右辺は
局所変数読みか定数リテラルのみで副作用が無いため観測値は同一（`CallMethodLocal` と同じ根拠）。

### 実測

`x += e` と `x = x + e` は**意味論同一**なので、同一バイナリ内の A/B で切り分けた（マシン変動が入らない）。

| 指標 | 前 | #2b | 倍率 |
|---|---:|---:|---:|
| `int   x += e`（ns/assign） | 52.3 | **28.5** | **1.84x** |
| `float x += e`（ns/assign） | 54.2 | **28.9** | **1.88x** |
| `while i < n: i += 1`（ns/iter） | 64.4 | **39.0** | **1.60x** |
| `binop_kind` 特化件数（bench_arith） | 15 | 21 | +6 |

E2E は HEAD バイナリと**交互実行**で取った（[ab_bench.ps1](ab_bench.ps1) を新設）。best-of-3:

| ベンチ | A/B |
|---|---:|
| bench_arith | 1.333x |
| bench_control_flow | 1.167x |
| bench_block_expr | 1.148x |
| bench_for | 1.120x |
| bench_collections | 1.088x |
| bench_method_hot | 1.045x |
| bench_method_body | 1.002x |

⚠ `bench_field_access` / `bottleneck_bench` は **0% 変化**。ホットループが
**モジュール最上位**にあり VM 経路に乗らないため（VM は関数本体のみ）。
「VM の変更が既存ベンチに出ない」ときはまずループが関数の中にあるか確認すること。

### 属性複合代入 `obj.x += e` — 型特化だけ実施、命令列は #2a へ

同じ穴が `Stmt::AttrCompoundAssign` にもあった（汎用 `Bin`）。ただし変数版との差 ~30ns/assign を
分解すると、**大半は命令列側**だった:

```
self.fx = self.fx + 1.5 : LoadLocal, GetAttrLocal, Const, FloatBinSS, SetAttr        (5 命令)
self.fx += 1.5          : LoadLocal, Const, LoadLocal, GetAttr, Swap, Bin, SetAttr   (7 命令)
```

型特化（`Bin` → `*BinSS`）だけ入れた結果は **152.7〜164.2 → 145.6〜149.0 ns（約 4〜5%）**。
残差 ~22ns は `GetAttr`（スタックからレシーバを clone）→ `GetAttrLocal`（frame から参照読み）化と
`Swap` 除去で取れるが、これは **op 除去＝peephole（#2a）の領分**なので手を出さず申し送りにした。
型の入手経路は `annot_prim(attr_node_id)`（注釈テーブルの結果型）＋`expr_prim(value)`。

### 回帰テスト — 負の対照で検知力を確認した

「同じ演算が**書き方**で特化を失わない」を [tests/mod.rs](src/interpreter/tests/mod.rs) の
`bin_specialization_invariants` で固定した（`x += e` と `x = x + e` の**命令列が完全一致**すること）。

⚠ **最初のテストは検知力が無かった**。`Op::Bin` の出現数だけを見ていたため、
融合だけ効いて特化が落ちた状態（`BinLocalConst`）を素通りした。負の対照
（型検査の注釈記録を外す）を実際に走らせて初めて判った。
`BinLocalLocal`/`BinLocalConst` も数える形に直し、再度負の対照で FAIL を確認:

```
left:  "[..., IntBinLC(1, 1, Add), ...]"      ← 期待
right: "[..., BinLocalConst(1, 1, Add), ...]" ← 注釈を外すとこうなる
```

### 診断の母集団が変わった点（注意）

`AnnotBinop: specialized=N` は `binop_kind` の件数なので **`CompoundAssign` の分も含む**が、
`miss_*` は `Expr::BinOp` の失敗しか数えない（複合代入は失敗理由を分類できる情報を持たない）。
`specialized / (specialized + miss)` を成功率として読まないこと（`annotations.rs` にも明記）。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **699 緑**（+1 = 新規不変条件テスト）／
  `cargo clippy --all-targets` **HEAD と byte-identical（62 件・増分 0）**／
  `compare_vm_modes.ps1` **identical 71 / differing 0**（stderr 込み）／
  `scan_examples.ps1` **FAIL 0 / TIMEOUT 0**／
  `dump_native_ir.ps1` **6 モジュール byte-identical**（`binop_kind` の消費者は VM のみで
  ネイティブ codegen・インタプリタは非消費であることを grep で確認した上での裏取り）。

---

## #2a V-F peephole（完了 2026-08-13）— **速度より「索引再マップ済みの土台」が成果**

2 つの独立した作業からなる。**A/B は別々に取った**（まとめて測ると帰属を誤る・#21-b の教訓）。

### #2a-1 属性複合代入の命令列（#2b からの申し送り）

`obj.x += e` は #2b 時点で 7 命令だった。変数版との差 ~30ns のうち型特化で取れたのは 4〜5% だけで、
残りは命令列そのものだと #2b で分解済みだった。

```text
self.fx = self.fx + 1.5 : LoadLocal, GetAttrLocal, Const, FloatBinSS, SetAttr        (5)
self.fx += 1.5   (前)   : LoadLocal, Const, LoadLocal, GetAttr, Swap, Bin, SetAttr   (7)
self.fx += 1.5   (後)   : LoadLocal, GetAttrLocal, Const, FloatBinSS, SetAttr        (5)
```

2 点を直した:
1. **`LoadLocal; GetAttr` → `GetAttrLocal`**（レシーバを clone せず frame から参照読み。`Expr::Attr` と同じ融合）。
2. **`Swap` の除去**。ツリーウォークは value を先に評価するので素直に組むと `Swap` が要るが、
   **value が副作用を持たない**（局所変数読み or 定数リテラル）なら先に現在値を読んでも観測結果は同じ。
   純粋でない value（関数呼び出し等）では順序を保存して `Swap` を残す（6 命令）。

| `self.x += e` | ns/assign |
|---|---:|
| #2b 着手前 | 152.7〜164.2 |
| #2b（型特化のみ） | 145.6〜149.0 |
| **#2a-1（命令列）** | **121.5〜128.6** |
| 対照 `self.x = self.x + e` | 120.9〜129.3 |

→ **変数版と同速に到達**（差が消えた）。通算 ~1.26x。

### #2a-2 peephole パス（`src/vm/peephole.rs` 新設）

`compile_fn` の最後に 1 回だけ走る後処理。コンパイラ本体は「素直に出す」責務のままにして、
構造的に出る無駄をここで回収する分業にした。対象は 2 つ:

1. **ジャンプ連鎖の畳み込み** — `JUMP a` の先が `JUMP b` なら直接 `b` へ。
2. **次命令への JUMP の除去** — `i: JUMP i+1` を消す。

どちらも **`else` の無い `if`** で構造的に出る（`Stmt::If` は分岐ごとに無条件 `Jump(end)` を置き、
最後の分岐ではその `end` が直後の命令になる）。ループ内 `if` では「if の脱出 → ループ back-edge」の
連鎖も出る。実例（`for` 内 `if`、左が前・右が後）:

```text
 8  JUMP_IF_FALSE 12        8  JUMP_IF_FALSE 6
11  JUMP 12                11  JUMP 6
12  JUMP 6                 12  JUMP 6   ← 到達不能になる（除去はしていない・後述）
```

⚠ **コード索引を持つ op は飛び先だけではない**。`ForIter` の exit_ip と `SetupTry` の handler_ip も
コード索引なので、再マップから漏らすとループ脱出・例外ハンドラが壊れる。
`code_target_mut` を**唯一の窓口**にして、専用テストで固定した。

⚠ `Chunk::spans` は**プール**（op が索引を持つ）で per-op の行テーブルではないため、命令削除でずれない。
**#1（汎用行テーブル）を入れるときは peephole と同時に直すこと**（計画書の #1 行にも注記済み）。

### 実測 — 静的には効くが、命令ミックスに占める割合が小さい

| 指標 | 値 |
|---|---:|
| 除去された JUMP（basics/classes/errors/functions の 18 例題） | **14.3%** |
| 除去された命令 / 総命令数 | **0.31%** |

E2E（`ab_bench.ps1` で #2a-1 版と交互実行・best-of-5）:

| ベンチ | A/B |
|---|---:|
| bench_branch（分岐支配・#2a で新設） | **1.044x** |
| bench_control_flow | 1.023x |
| bench_arith / bench_for / bench_collections | 1.007〜1.026x |
| bench_method_body | 1.000x |

→ **分岐支配コードで +4.4%、それ以外はほぼ 0**。JUMP の 14.3% を消しても総命令の 0.31% にしかならない、
という静的な数字と整合する。

### ⚠ 「退行」を誤帰属しかけた（記録）

`bench_method_hot` が **0.972x（2.8% 退行）** を 2 回再現した。peephole のせいに見えたが、
**同ベンチの生成バイトコードを両バイナリでダンプすると md5 まで一致**した
（＝ peephole は何も削っていない・同じ命令列を同じだけ実行している）。

→ **バイナリのコード配置由来の計測アーティファクト**と判断した（モジュール追加で
インタプリタのホットコードの I-cache/分岐予測のアラインメントがずれる）。
#21-b と同じ罠で、**「差が出た＝その変更のせい」ではない**。
疑ったら**まず生成物が変わっているかを確認する**のが最短の切り分けだった。

### やらなかったこと

- **到達不能コードの除去**: 連鎖畳み込みの結果、誰も飛ばない `JUMP` が残る（上の例の 12）。
  実行されないので**実行時コストはゼロ**。到達可能性解析を足すリスクに見合わないので見送り、
  **#24（peephole パターンの追加）**として番号を付けて計画書に登録した。

### 検証
- `cargo build` 警告 0 ／ `cargo test` **703 緑**（+4 = `peephole.rs` の単体テスト）／
  `cargo clippy --all-targets` **HEAD と byte-identical（62 件・増分 0）**／
  `compare_vm_modes.ps1` **identical 71 / differing 0**（stderr 込み）／
  `scan_examples.ps1` **FAIL 0 / TIMEOUT 0**／`dump_native_ir.ps1` **6 モジュール byte-identical**。

---

## #1-a / #1-b（完了 2026-08-13）— **本体着手前に安全網を作り、代償の大半を安全網ごと消した**

### 着手前の調査で前提が 2 つ崩れた

1. **デバッガには自動テストが 1 件も無かった**。`compare_vm_modes.ps1` は stdin を与えないので
   `break_point` を含む例題（`debug_demo`）は skip リスト入りで、
   「デバッグ中は VM を無効化してツリーウォークに委ねる」という #1 の暫定対応は
   **一度も検証されたことがなかった**。#1 本体は**ホットループとデバッガの両方**に触るので、
   この状態で着手するのは高リスク。
2. **VM コンパイラは `Stmt::BreakPoint` を扱わない**（grep で 0 件）＝ `break_point` を含む関数は
   常に bail してツリーウォークになる。したがって暫定対応が実際に効いているのは
   **「別関数へステップインしたとき」だけ**で、停止した関数自身は元からツリーウォーク。

### 代償の実測（暫定対応の costs）

| ベンチ | `--vm=off` | `--vm=auto` | off/auto |
|---|---:|---:|---:|
| bench_control_flow | 10.27s | 2.44s | **4.22x** |
| bench_arith | 4.22s | 1.44s | 2.94x |
| bench_branch | 4.25s | 1.73s | 2.46x |
| bench_method_hot | 7.63s | 3.73s | 2.04x |

＝ ステップ実行中は **2.0〜4.2x 遅い**。ただし `q`（resume）で `DbgMode::Inactive` に戻るので、
遅いのは**ステップ中だけ**。人間が待たされるのは「重い関数を step-over で飛ばす」ときに限られる。

### #1-a 安全網（`compare_debug_modes.ps1` ＋ `examples/debugger/`）

`<name>.ar` と `<name>.in`（デバッガへ流すコマンド列）を対にして、両モードで実行し
**stdout+stderr を比較**する。4 シナリオ: step-into / step-over+step-out / dbg 変数と式評価 /
重い呼び出しの step-over。

⚠ **負の対照で検知力を確認した**。`vm_eligible` から `dbg_active()` ゲートを外すと:

```text
DIFFER  dbg_vars.ar / step_into_fn.ar / step_over_out.ar   (3/3、後に 4/4)
  => [dbg] Error: NameError: 'n' is not defined      ← ステップイン先の VM フレームが見えない
```

初版は `step_into_fn` しか検知しなかった（他 2 本は**別関数へステップインしていなかった**）。
上記 2. の理由で、**別関数へのステップインを含めないと検知力が出ない**。`.in` を書き直して 4/4 検知に。

### #1-b 代償の最小化 — `should_pause_at` を読んだら本体は要らなかった

`should_pause_at` の停止条件を表にすると、**呼び出し先で停止し得るのは StepInto だけ**だと分かる:

| モード | 停止条件 | 呼び出し先（深さ > 現在）で停止し得るか |
|---|---|---|
| `StepOver` | `call_stack.len() == entry` | **しない**（呼び出し先は常に `entry` より深い） |
| `StepOut { target }` | `call_stack.len() <= target` | **しない**（呼び出し先は常に `target` より深い） |
| `StepInto` | `depth > entry` で停止 | **する** |

→ `dbg_active()`（＝デバッグ中は全部ツリーウォーク）を **`dbg_blocks_vm()`（StepInto のときだけ）**
へ置換。**通常経路のコストはゼロ**（ホットループには何も足していない）。

| シナリオ | 前 | 後 | 倍率 |
|---|---:|---:|---:|
| 重い呼び出しを step-over（`step_over_heavy.ar`） | 1052〜1063 ms | **533〜544 ms** | **1.97x** |

`compare_debug_modes.ps1` は 4/4 identical のまま。

### ⚠ 計測ハーネスの不具合で「効果なし」と誤判定しかけた（記録）

最初 PowerShell で測ったとき **1.01x（差なし）** と出た。しかし
「VM 無しなら 3.0x 差がつく関数なのに、デバッグ経由だと差が消える」のは辻褄が合わない。
実測値の**絶対値が想定より小さすぎる**（0.48s ＝ ほぼ起動時間）ことに気付いて bash で測り直すと 1.97x。
原因は PowerShell 側の相対パス解決で、プログラムが即終了していた。

→ **「差が出なかった」も「差が出た」と同じくらい疑うこと**。まず**桁が想定と合うか**を見るのが最短だった
（#2a では逆に「差が出た」を疑って生成物の md5 一致で潰した。どちらも同じ作法）。

### #1 本体（VM 内の文単位ブレーク）→ **この後 完了**（次節を参照）

この時点では「残る代償は StepInto だけ」なので本体は保留と判断した。
その判断は**その後の調査で覆った**（#3 の前提であることを実コードで確認・既存バグの発見）。
⚠ ここで挙げた懸念のうち「**セッション開始時に既に走っている VM フレームが停止判定を持たない**」は、
**懸念ではなく既に起きていた実バグ**だった（step-out が効かない）。経緯と解決は次節。

---

## #1 本体（完了 2026-08-13）— VM 内の文単位ブレーク。**既存バグ 1 件を修正し、副産物で 1.3x**

### 着手理由が「実利」から「前提＋バグ」へ変わった

#1-b の時点では「残る代償は StepInto だけ」なので本体は保留と判断していた。
その後 2 点で判断が変わった:

1. **#3 の前提であることを実コードで確認した**。#3 が削除対象とする TLS 4 本
   （`GENERATOR_YIELDS`/`BLOCK_YIELDS`/`LOOP_DEPTH`/`BLOCK_RETURN_EXPECTED_TYPE`）と
   センチネル 2 種（`RAISE_SENTINEL`/`BREAK_SENTINEL`）は、**ツリーウォークが関数本体の制御フロー**
   （`break`・`block_return`・`loop_yield`・例外伝播）を回すための機構。
   デバッガが本体をツリーウォークに委ねる限り消せない。
   （§0.1 の「ツリーウォークは §2.3 用に残置」を根拠に保留したのは、削除対象の中身を見ていない判断だった。）
2. **既存バグを見つけた**（下記）。

### ⚠ 既存バグ: VM フレームへ step-out すると停止せず走り抜ける

`caller`（`break_point` を含まない＝VM）が `probe`（`break_point` を含む＝bail してツリーウォーク）を呼ぶ形で、
`probe` から step-out すると:

| モード | 挙動 |
|---|---|
| `--vm=off` | `caller` の次の文で停止し、`got`/`before` を参照できる |
| `--vm=auto` | **停止しない**。プログラムが最後まで走る（デバッガが制御を失う） |

**#2a のコミット時点から存在していた**（`arrow_1base.exe` で再現確認）。
原因は「デバッグ中は VM を無効化」というゲートが**新しく入るフレーム**にしか効かず、
**セッション開始前から走っている VM フレーム**を救えないこと。
暫定対応が正しく見えていたのは **一度も検証されていなかったから**で、
#1-a の安全網（`examples/debugger/step_out_to_vm_caller.ar`）が入って初めて可視化された。

### 実装（4 点）

| # | 内容 |
|---|---|
| 1-1 | `Chunk::stmt_spans`（**文境界行テーブル**・code と 1:1）。`compile_stmt` 入口で予約し `emit` が消費。位置を持たない文は `STMT_NO_SPAN` で、ツリーウォークの `best_span_for` と同じく `dbg_last_span` へフォールバック |
| 1-2 | `peephole::optimize` が行テーブルを**同じ写像で詰め直す**。さらに **文の先頭 op は除去対象から外す**（`break` のように Jump 1 個だけの文があり、消すと停止位置が 1 文ぶん消える） |
| 1-3 | `run_stepping`（停止判定つき専用ループ）。**通常ループには何も足さない**。入るのは ① `run` 入口でセッション中 ② `Flow::NextAfterCall`（呼び出しから戻ったらセッションが始まっていた＝上のバグの修正点） |
| 1-4 | 停止時に `chunk.local_names` から一時スコープを組み `frame_floor` を進める（**`local_names` の最初の消費者**。V-E で用意され消費者ゼロだった） |

判断ロジックは `should_pause_now()` に集約し、ツリーウォーク（`should_pause_at`）と
VM（`vm_should_pause`）が**同じ表を見る**ようにした（#22 の「重複は片方を委譲に畳む」）。

### ⚠ 「宣言済みかどうか」で 2 回つまずいた

VM の flat buffer は**全 slot を `None` で初期化する**ので、値だけでは
「未宣言」と「`None` を代入済み」を区別できない。素直に全 slot を見せると、
ツリーウォークでは `NameError` になる名前が REPL から `None` として引けてしまい off/auto がずれた。

最初は **stepping ループで実行時に追跡**した。これは
**セッション開始前から走っていたフレーム**（`NextAfterCall` で途中から stepping に入ったフレーム）で
破綻する — それまでの代入を実行時には知りようがないので、全ローカルが「未宣言」になった。

→ **停止時に `code[..ip]` を静的走査**する `declared_slots` に変更。
per-op の仕事がゼロになり（通常ループにも stepping ループにも）、途中参加フレームでも正しい。
分岐で実行されなかった宣言も「宣言済み」と見なす過大近似だが、
「このソース位置より前に宣言文がある」と同義で、ツリーウォークの見え方に十分近い。

### ⚠⚠ 副産物: `exec_op` は**元からインライン展開されていなかった**（1.06〜1.32x）

実装後に通常経路が **3〜5% 退行**した。#2a の教訓どおりまず生成物を疑い、
`bench_branch` のバイトコードを両バイナリでダンプすると **md5 一致**
（＝同じ命令を同じだけ実行している）。ならば per-op の実コストが増えたはず、と考えて
**`exec_op` の呼び出し元が `run` の 1 つから `run_stepping` を加えた 2 つになった**ことに思い当たった。

`#[inline(always)]` を付けたところ、退行が消えるどころか **#1 着手前より速くなった**:

| ベンチ | #2a（前） | #1（後） | 倍率 |
|---|---:|---:|---:|
| bench_arith | 1.440s | 1.116s | **1.290x** |
| bench_branch | 1.688s | 1.283s | **1.315x** |
| bench_for | 0.184s | 0.155s | 1.192x |
| bench_control_flow | 2.419s | 2.020s | 1.197x |
| bench_collections | 5.514s | 4.783s | 1.153x |
| bench_method_hot | 3.574s | 3.245s | 1.101x |
| bench_method_body | 2.817s | 2.661s | 1.059x |

＝ **`exec_op` は巨大なので `#[inline]`（ヒント）を LLVM が却下し続けていた**。
呼び出し元が 1 つの間は誰も気付かなかった。バイナリは +59KB（2 箇所への展開と整合）。
2 回連続で同一の数値が出ることを確認済み。

→ **教訓: `#[inline]` は「効いている」ことを意味しない**。ホットループの中身が関数なら、
展開されているかを疑うこと。ここでは 1.3x が眠っていた。

### 効果まとめ

| 観点 | 結果 |
|---|---|
| 正しさ | **既存バグ 1 件を修正**（step-out が VM フレームで効かない） |
| デバッグ速度 | ツリーウォークへのフォールバックが**完全に不要**に（#1-b の 1.97x に加えステップイン先も VM） |
| 通常経路 | **1.06〜1.32x**（#1-x の副産物） |
| #3 への影響 | **前提が #10 のみに減った** |

### 検証
- `cargo build` 警告 0 ／ `cargo test` **705 緑**（+2 = peephole の行テーブルテスト）／
  `cargo clippy --all-targets` **HEAD と byte-identical（62 件・増分 0）**／
  `compare_vm_modes.ps1` **identical 71 / differing 0**／
  **`compare_debug_modes.ps1` identical 5 / differing 0**（バグ再現シナリオ込み）／
  `scan_examples.ps1` **FAIL 0**／`dump_native_ir.ps1` **6 モジュール byte-identical**。

---

## 保留・未着手タスクの調査記録（計画書から移設）

計画書側は「事実＋手法」を 1 行で持ち、判断の根拠となる調査結果はここに置く。

### V-E 本体（op→Span の汎用行テーブル）— 一部対応 2026-07-27／完全版は保留
**【一部対応 2026-07-27／完全版は保留】** 完全版（op→span 行テーブル＋VM ディスパッチループの停止判定＋停止フレームの
buffer ローカルを REPL から名前参照する経路）は大規模のため保留。代わりに**デバッグ中（`DBG_MODE != Inactive`）は
VM を無効化しツリーウォークに委ねる暫定対応**を実施（`dbg_active()` を `vm_eligible` に追加）。これにより
`--vm=auto` でも would-be-VM 関数へステップイン（文単位停止・変数参照）が `--vm=off` と byte-identical に動作する。
完全な VM ネイティブ行テーブルは、対話デバッグの実効速度が問題化した場合にのみ着手する（現状は不要）。

### V-F 最適化 — superinstruction のみ実施 2026-07-27
**【一部対応 2026-07-27】superinstruction を実施**: `local <op> local` → `BinLocalLocal`、`local <op> リテラル`
→ `BinLocalConst`（コンパイラが融合 emit・意味論不変=`apply_bin_fast` 委譲）。算術支配ループで auto ~1.15x。
**残り（保留）**: peephole（Jump 除去等はジャンプ先再マップが要り中規模）・単型算術命令（型注釈依存で要検討）・
**R0-A エスケープ解析は #12 のフレームモデル前提のため #12 とセットで保留**。

### #10-a 最上位ツリーウォークの実測（診断フック `AR_TW_STATS`）— 完了 2026-08-13

**2026-07-27 の保留判断のうち 2 点が実測で覆った。** 当時の (a)「モジュール本体は定義文が支配的」は
**静的には正しいが動的には誤り**だった。全 199 例題の最上位文を数えると定義文は 16%（ただし 89% の
ファイルが 1 つ以上持つ＝「本体一括 Chunk」は事実上必ず bail する）。一方で実行時のディスパッチ数では
定義文は 1 回ずつしか実行されず無視できる。

そこで `AR_TW_STATS=1` を新設し、「`--vm=auto` の実行中にツリーウォークが実際に実行している文」を
`Stmt` バリアント別・**モジュール最上位 / 関数本体内**に分けて数えた（[tw_stats.ps1](tw_stats.ps1)）。
実測（例題 116 件）:

| 区分 | 件数 | 内訳 |
|---|---|---|
| 最上位 | **11,235,617** | CompoundAssign 92.6% / Let 7.3%（＝**最上位ループの本体**） |
| 関数本体内 | **204** | ほぼ枯れている |
| VM コンパイル | 356 | fn 305 成功 / **49 失敗** |

**結論 2 つ**:
1. ツリーウォークに残っている負荷は **実質すべて「モジュール最上位のループ本体」**。関数側は既に VM 化済み。
2. **#10 だけでは #3 の前提を満たさない**。関数の VM コンパイル失敗が 49 件残っており、
   フォールバックを撤去したらそれらが動かなくなる。計画書の「#3 の前提は #10 のみ」は不正確だった。

bail 地点も計装した（`compile_fn` に「失敗したのに bail が 1 件も記録されない＝未帰属」の
catch-all を置き、計上漏れゼロを担保）。

⚠ **診断フックは feature `tw_stats` にした**（env 判定だけにしない）。
`exec()` は文ごとに呼ばれるので、`enabled()` に `OnceLock` の atomic 読みが 1 回入るだけで
**`partial_call_overhead.ar`（5000 万回の文ディスパッチ）が 11% 遅くなった**。
`cfg!(feature=..)` を先に評価して定数 false にすると呼び出しごと消える。

### #10-b 最上位ループの Chunk 化 — 完了 2026-08-13

**手法**: 最上位の `while`/`for` **文**を 1 つずつ Chunk へコンパイルし（`compile_toplevel_stmt`）、
`exec()` の該当アームから実行する。最上位で 2 回以上実行される文はループだけなので、
「1 回実行するためにコンパイルする」損を避けてここに限定した。

- **`Op::StoreGlobal(name_idx, cache_idx)` を新設**（記録どおり、これが唯一の欠けていた基本要素だった）。
  読み側は #21-b の `Resolution::Global` → `LoadGlobal` で既に揃っていたので、**書き側だけ**が穴だった。
- 書き込み先の判定は **リゾルバの `toplevel_visible_globals` へ委譲**（#21-b が `Resolution::Global` を
  付けるのと同一判定。`Stmt::Assign`/`CompoundAssign` は `res` を持たないので実行前に引き直す）。
- ループ本体の `let`/`mut`・`for` ターゲットは**従来どおり slot**（ツリーウォークもスコープを push するので一致）。
- 実行時の適格条件は **`scopes.len() == 1`**（＝モジュール最上位）。これがコンパイル時の
  `toplevel_globals` 判定と実行時の記憶域を結ぶ唯一の根拠。

**結果**:

| 指標 | before | after |
|---|---|---|
| 最上位ツリーウォークのディスパッチ | 11,235,617 | **3,631,623（3.09x 減）** |
| 最上位ループの Chunk 化 | — | 32 成功 / 15 失敗 |
| E2E（release, A/B） | — | field_access **1.28x**・string **1.26x**・bottleneck **1.12x**・name_hash **1.11x**・他は 1.00x |

#### この 1 タスクで踏んだ性能の罠 4 件（いずれも**実測でしか見つからない**）

1. **`StoreGlobal` に索引キャッシュが要る。** ツリーウォークの `Stmt::Assign` は対象を
   `Var::SlotCell` へ昇格して**セルへ直接書く**（`global_slot_cells`）。素直に `assign_var`
   へ委譲しただけの初版は名前引きが毎回残り、**ストア支配のループでツリーウォークに 3.5% 負けた**。
   `try_fill_slot` へ委譲して同じ機構に乗せて解消。
2. **診断フック自体が 11% を食った**（上記 #10-a の feature 化）。
3. **`exec_op` は `#[inline(always)]`。重いアームを足すと巻き添えが出る。**
   `StoreGlobal` の本体をアームに直書きすると、**この op を一切使わない Chunk のホットループが
   4〜6% 遅くなった**（#1-x の「`exec_op` の展開が効いている」の裏返し）。
   逆に**本体を丸ごと外へ出す**と、最上位ループが毎ストア呼び出しを払って別のベンチが 7〜10% 落ちた。
   → **IC の定石どおりヒット経路だけインライン・ミス経路は `#[inline(never)]`** で両立。
4. **`exec()` の `Stmt::While`/`For` アームは関数内ループの実行でも通る。**
   いきなり非インラインの `try_run_toplevel_loop` を呼ぶと数 % 損する。
   フィールド 3 本の比較だけの `toplevel_vm_candidate()`（`#[inline(always)]`）で先に切る。

#### 誤帰属を 3 回した記録（**手順の教訓**）

- `bench_arith`/`bench_for`/`bench_collections` の「退行」は**ノイズだった**。
  `AR_TW_STATS` で見ると **top-level chunk が 0 件**＝ #10-b が触っていない。
  「触っていないことを先に確かめる」のが最短。
- `partial_call_overhead.ar` の 0.875x を**最上位 VM 化のせいだと誤認**した。
  `--vm=off` でも同じ幅で出る（＝ VM 経路は無関係）と判って初めて診断フックに辿り着いた。
  **`--vm` を切り替えられる A/B が無かった**のが原因で、[ab_bench_vm.ps1](ab_bench_vm.ps1) を新設した。
- 最初の `--vm=off` 比較で「auto の方が速い」という矛盾した数字が出たのは、
  **交互実行していなかった**から（A を N 回 → B を N 回だと後者がサーマルドリフトで不利）。
  `ab_bench.ps1` は交互実行している。自前で測るときも必ず交互にすること。
- レイアウト擾乱を疑ったときは、**HEAD 自体に無意味な関数を 1 つ足して測る**とノイズ床が出る
  （このベンチでは 0.993x ＝ 約 1%）。これが無いと「レイアウトのせい」と言い逃れできてしまう。

### #10-b′ グローバル受信者のメソッド呼び出し — **保留**（実装したが巻き戻した）2026-08-13

`object_is_instance` は `slot_type`（型注釈文字列）しか見ないため、**グローバル変数がレシーバの
メソッド呼び出し・属性代入は丸ごと bail** する。これが最上位 Chunk 化の失敗要因の最大項
（実測 15 失敗中 17 bail が `method-receiver`）。型検査の注釈 `InferredType::NamedInstance` を
見る実装を入れると **最上位ツリーウォークは 11.2M → 30,113（373x 減）** まで落ち、
E2E も method_call 1.38x・name_hash 1.30x まで伸びた。

**が、巻き戻した。** 理由は 2 つ:

1. **正しさ**: `NamedInstance(名前)` は **Arrow のクラスとは限らない**。C# スタブ由来のクラスも
   同じ注釈になり、実行時の値は `Value::CsObject`。VM の `CallMethod` が使う
   `call_instance_method_evaled` は `Value::Instance` 前提なので `TypeError` で落ちた
   （[event_cs_handler.ar](examples/interop/event_cs_handler.ar) の off/auto 不一致で検出）。
   **これは #15e の「注釈は最適化ヒントであって意味論の根拠にしてはいけない」の再演**。
2. **根治すると別の退行が出た**。`eval_method_call_evaled` に `CsObject` アームを足して
   （CallArg 版はそちらへ委譲＝ #22 系列の「`*_evaled` とずれた実装を作らない」）
   非 Instance を汎用ディスパッチャへ落とす形にすると、正しさは戻るが
   **ベンチ全体が 0.88〜0.96x へ広く退行**した。`#[cold]` 外出しでも戻らず、
   切り分けコストが効果に見合わないと判断した。

再訪する際の前提: **`eval_method_call_evaled` が FFI レシーバ（CsObject/PyObject/JsProcFn 等）を
CallArg 版と同じだけ扱えるようにする**（C 軸の畳み込み）ことが先。その上で
`object_is_instance` を拡張する。順序を逆にすると必ず off/auto 不一致になる。

### #27 fn の VM コンパイル失敗の解消 — 進行中（49 → 31）2026-08-13

`AR_TW_STATS` の bail 計上を **fn 由来 / 最上位由来に分離**（`vm_bail_fn` / `vm_bail_toplevel`）し、
未帰属を 6 → 2 まで潰してから着手した。分離前は合計 64 の内訳に fn 49 と最上位 15 が混ざっており、
どれが #27 の対象か読めなかった。

| 実施 | 解消 | 手法 |
|---|---|---|
| `pass` | 3 | `compile_stmt` に `Stmt::Pass => {}`（命令ゼロ。文境界の予約は入口で済むので停止位置はずれない） |
| `break_point` | 5 | `Op::BreakPoint(span_idx)`。**`vm_debug_pause` へ委譲**し `Flow::NextAfterCall` を返す |
| `undefined` | 1 | `Expr::Undefined` → `Op::Const(Value::Undefined)` |
| `obj::Trait.attr` の読み書き | 5 | `Op::GetTraitAttr`/`SetTraitAttr`。`trait_access_evaled`/`trait_assign_evaled` を切り出しツリーウォークが委譲 |
| メソッド本体の `Self` | 4 | `Op::LoadSelfClass`。値は `current_class`（`run_vm_method` が設定済み＝ツリーウォークの `Self` 束縛と同じ出どころ） |

**結果**: `fn_FAILED` **49 → 31**、関数本体内のツリーウォーク・ディスパッチ **204 → 161**。

⚠ **`break_point` で 1 度落とし穴を踏んだ**: `exec_breakpoint` を直接呼ぶと REPL から
そのフレームのローカルが見えず `NameError` になる（`dbg_vars.ar` が off/auto 不一致で検出）。
VM フレームのローカルは flat buffer にあるので、**必ず #1 の `vm_debug_pause` を通す**
（`local_names` から一時スコープへ移す処理がそこにある）。

**残り 31 の内訳と見通し**:
- `decl-prepass:FnDef` 8 — 入れ子 `fn`（クロージャ）。§2.4 の既知の穴。**残る最大の単一項目**。
- `method-receiver:Attr` 7 / `method-receiver:Ident` 6 / `assign-target:Attr` 2 /
  `attr-compound-target:Attr` 1 = **16 件が「レシーバを Instance と証明できない」1 つの原因**。
  → #26（#27-a が前提）。ここを直すと半分が一度に片付く。
- `variadic-param` 2 / `decl-prepass:Static` 2 / `for-target-shadow` 1 / 未帰属 2。

### ⚠ コード配置ノイズの正しい測り方（#27 で確立・#10-b の記述を上書きする）

**opcode を足すと、その op を一切実行しなくてもベンチが 5〜12% 動く。**

#27 の A/B は当初こう見えた: `partial_call_overhead` 0.882x・`bench_branch` 0.948x・
`bench_arith` 0.973x・`bench_for` 0.967x。しかし **`--vm=off` でも同じ幅**で出た
（＝ #27 のコードは 1 命令も実行されていない）。そこで **HEAD 側に「#27 と同じ規模の、
決して実行されない」摂動**（Op に 4 variant ＋ `#[inline(never)]` ヘルパ 3 本＋メソッド 3 本）を
入れて測ると:

| ベンチ | #27 | 摂動のみ（**実行されない**） |
|---|---|---|
| partial_call_overhead | 0.882x | **0.884x** |
| bench_branch | 0.948x | **0.944x** |
| bench_arith | 0.973x | **0.956x** |
| bench_for | 0.967x | **0.953x** |
| flat_bench_interp | 0.978x | **0.957x** |

**結論**: #27 のベンチ差はロジックのコストではなくコード配置。

**手順の教訓**:
- **ノイズ床のプローブは「変更と同規模」でなければ意味が無い**。#10-b では小関数 1 本で測って
  0.993x を得て「このベンチは配置に鈍感」と結論したが、**それは規模不足だった**。
  同規模で測ると同じベンチが 0.884x 動く。
- したがって **opcode を足す変更では、E2E ベンチの ±5〜12% を良し悪しの根拠にできない**。
  判断材料は「`--vm=off` でも同じ差が出るか」と「同規模プローブとの比較」の 2 つ。
- 逆に **#10-b で見つけた 4 件の罠（索引キャッシュ・診断フック・`exec_op` の展開・
  関数内ループの巻き添え）は本物**だった。それらは `--vm=off` で消える／プローブで再現しない、
  という形で区別できる。

### #27-a / #26 レシーバの健全な判定 — 完了 2026-08-13

**当初の #27-a の定義（`eval_method_call_evaled` に FFI レシーバを畳む）は、#26 を安全にする目的には
過剰だった**。実測すると `eval_method_call`（CallArg 版）は **16 種のレシーバ・644 行**を扱い、
評価済み版は 2 種（`Instance`/`PyObject`）しか扱っていない。全部を畳むのは #3 には要るが、
**#26 が必要としているのは「レシーバが `Value::Instance` である証明」だけ**である。

そこで**証明する側**を実装した（約 50 行）。

#### 何が問題だったか

`InferredType::NamedInstance(名前)` は **Arrow のクラスと外部言語スタブのクラスを区別しない**。
`import[cs-dll]` 等はパース時に `Stmt::ClassDef` へ変換されるため、型検査の
`known_class_names` には両方が載る。実行時表現は前者が `Value::Instance`、後者が
`Value::CsObject`。**注釈だけを根拠に `Value::Instance` 前提の op へ落とすと落ちる**
（#10-b′ で `event_cs_handler.ar` の off/auto 不一致として実際に踏んだ）。
＝ #15e の「注釈は最適化ヒントであって意味論の根拠ではない」の具体例。

#### 実装

- **`TypeRegistry.arrow_class_names`**（#27-a）: `Stmt::ClassDef` のうち
  **外部言語 import の本体でないもの**だけを集める。ビルダに `foreign_depth` を持たせ、
  `is_arrow_source_lang(lang)`（`ar|tl|ar-auto|tl-auto|arc|tlc`）でない import に入る間だけ加算する。
  ⚠ **未知の lang タグは false（保守的）**。新言語を足したとき黙って Arrow 扱いになるより、
  最適化が効かない方が安全。lang の一覧は `parser/imports/dispatch.rs` の `match lang` が唯一の出所。
- **`AstAnnotations::is_arrow_class`**: 上の集合を VM コンパイラへ渡す窓口。
- **`Compiler::is_arrow_instance_type`**: 判定を **2 段**にした。
  1. `is_user_instance_type` — **形**（ジェネリック・union・組み込み型名を除く）
  2. `annotations.is_arrow_class` — **出自**（外部言語スタブ由来を除く）
- **`Compiler::annot_is_arrow_instance`**（#26）: slot を持たないレシーバ
  （グローバル変数・属性・呼び出し結果）を注釈の `resolved_type` から同じ 2 段で判定する。
  `expr_node_id` は `Ident`/`Attr`/`Call`/`Subscript` の node-id を返す（0＝未採番は None）。

#### 結果

| 指標 | before | after |
|---|---|---|
| 最上位ツリーウォークのディスパッチ | 3,631,623 | **30,123（120x 減）** |
| `fn_FAILED` | 31 | **29** |
| `toplevel_FAILED` | 15 | **11** |
| E2E | — | `bench_method_call` **1.45x**・`bench_name_hash` 1.10x |

⚠ **健全化のコストも実測した**: 2 段目を足した時点で `fn` の成功が 323 → 321 に減った。
これは**従来 unsound にコンパイルしていた 2 件**（非 Arrow クラスをレシーバとする関数）で、
`is_arrow_class` が正しく弾いた。**カバレッジが下がる方向の変化が「正しい」ことがある**。

#### 残った本来の #27-a（#3 に必要）

`method-receiver:Ident` 8 ＋ `method-receiver:Attr` 4 ＝ **12 件は非 Arrow レシーバ**
（`CsObject`/`Signal`/`EventLoop` など）で、これらを VM に載せるには
**`eval_method_call_evaled` を CallArg 版と同じ 16 種まで広げる**必要がある（残 14 アーム・644 行のうち
`Instance`/`PyObject` を除く分）。#3（強制バイトコード）には必須。#26 とは切り離せたので、
着手時は「畳む」作業に専念できる。

### ⚠ ~~opcode を 1 つ足すごとに VM 支配ベンチが ~1〜1.5% 落ちる~~ → **このモデルは誤り**（#28 で反証）

**【当時の記述・反証済み】** 「`exec_op` は `#[inline(always)]` なのでアームを足すとディスパッチ
ループ全体が太る。#27 で 4 op 足すと、`--vm=off` でも `--vm=auto` でも、**決して実行されない
ダミー 4 op でも**同じ幅の差が出た（`bench_branch` 0.93〜0.94x・`partial_call_overhead` 0.88x）。
どの op かではなく何個足したかで決まる」。

**→ #28 で反証された。** 稀な op 7 個を 1 アームに畳んでも**何も回復しなかった**（1 件は悪化）。
アーム数が原因なら減らせば戻るはずである。当時のダミープローブは op だけでなく関数・メソッドも
足しており、「op を足したから」と「同規模の摂動だから」を**区別できていなかった**。

**正しいモデル**: この規模の変更は VM 支配ベンチを**どちら向きにも ±5% 揺らす**（コード配置）。
数 % の E2E 差から因果を読み取らないこと。詳細は #28 の節。

### #10-c 最上位の宣言文を Chunk 化 — 完了 2026-08-13

**計画書の想定と実体がずれていた。** #10-c は「`DeclareGlobal` を足して `let`/`Assign`/`if`/`try`/
式文を覆う」と書いてあり、暗に「最上位の文は 1 回しか実行されないが数が多い」と読める。
実測すると残っていた 30,123 件の内訳は `Let` 14,412 ＋ `LoopYield` 14,018 で、
**93% が 2 ファイルの 4 文**だった:

```
mut pts = for i in range(N) -> list[Particle]:
    let f = float(i) * 0.001
    loop_yield Particle(f, f * 2.0, f * 3.0, 1.0 + f)
```

＝ **宣言文そのものではなく、初期化子のループ式が N 回まわる**のが実体。
「1 回しか実行されない文を Chunk 化しても損」という素朴な判断で切ると**これを取り逃す**。

#### 実装

- **`Op::DeclareGlobal(name_idx, DeclKind)`** — `DeclKind` は `Const`/`Mut`/`LetPlain`/
  `LetFreezeInstance`。**4 つの op に分けず 1 op のオペランドにした**（意味が同じものを
  別 op に分ける理由が無いため。当時の「op 1 個あたり ~1〜1.5%」という根拠は #28 で反証された）。
  実体は `Interpreter::vm_declare_global` 1 箇所で、コピー・フリーズ・再宣言検査を
  `exec_let` / `exec` の `Const`・`Mut` アームと同じ判断で行う。
- **`compile_toplevel_stmt` が `Let`/`Mut`/`Const` を受け付ける**ようになった。
- **`exec()` の該当アームから `try_run_toplevel_stmt` を呼ぶ**（`toplevel_vm_candidate` で先に切る）。

⚠ **slot 採番の落とし穴**: 宣言文を `collect_nested_decls` に渡すと
**その文が宣言する名前に slot を割り当ててしまう**。最上位の宣言はグローバルなので、
slot を振ると `store_target` が slot 側を優先し、`DeclareGlobal` が出ずに値がフレームへ消える。
宣言文だけは**初期化子に対して `collect_expr_decls`** を呼ぶ。

⚠ **`let x = <識別子>` は bail**。`exec_let` は「ソースが `mut` なら copy+freeze」を
ソース変数の可変性で分岐するが、最上位ではコンパイル時に分からない
（`toplevel_globals` は名前の集合しか持たない）。13 件がこれで bail する。

⚠ **評価順が 1 点だけ違う**: ツリーウォークは「再宣言検査 → 初期化子の評価」だが、
VM は「初期化子の評価 → `DeclareGlobal` 内で検査」。ただし**型検査が再宣言を先に弾く**ので
（`redeclare_error.ar` は `StaticTypeError` で実行に到達しない）観測されない。

#### 結果

| 指標 | before | after |
|---|---|---|
| 最上位ツリーウォークのディスパッチ | 30,123 | **2,071（14.5x 減）** |
| 〃（#10 着手前から） | 11,235,617 | **2,071（5,425x 減）** |
| E2E（A/B・release） | — | **退行なし**（0.98〜1.06x・わずかに正） |

E2E の伸びが小さいのは当然で、残っていたのは数万回のディスパッチ＝ミリ秒級だから。
**#10-c の価値は速度ではなく #3 に向けたカバレッジ**にある。

#### 最上位に残る 2,071 件と bail 163 件の内訳

`toplevel_FAILED` が 11 → 163 に増えたのは**試行する文が激増したため**で、退行ではない
（比較すべきは常にディスパッチ数の方）。

| bail 理由 | 件数 | 行き先 |
|---|---|---|
| `method-receiver:*`（Ident 79/Attr 16/Call 2/Cast 1） | **98** | #27-b（非 Arrow レシーバ） |
| `toplevel-let-from-ident` | 13 | 最上位グローバルの可変性を持てば解決 |
| `expr:ImaginaryLit` | 13 | 虚数リテラル。小さい |
| `callee-expr:TemplateInstantiate` | 13 | テンプレート実体化呼び出し |
| `call-arg`（キーワード/可変長） | 8 | |
| `for-tuple-target` | 7 | `for k, v in ...` のタプル分解 |
| 未帰属 / その他 | 11 | |

残るツリーウォークは `Expr`（式文 `print(...)` 等）909 が最大で、これは**まだ試行していない**
（`compile_toplevel_stmt` が受け付けていない）。#3 にはこれと定義文・`import` が要る。

### #27-b メソッド dispatcher の 1 本化 — 完了 2026-08-13

**「644 行を移植する」と見積もっていたが、実際に書き換えが要ったのは 4 パターンだけだった。**
着手前に `eval_method_call` 本体の `args` 参照 48 件を分類したところ:
`expect_no_args` 15／`eval_one_arg` 12／`eval_call_args` 9／既に `_evaled` 版がある委譲 5。
つまり**引数の扱いはこの 4 種に閉じており**、残りは受け取った値を使うだけだった。

#### 実装

- `eval_method_call`（CallArg 版）を **引数を評価して委譲するだけ**にし、16 レシーバの
  ディスパッチを `eval_method_call_full`（評価済み引数）へ移した。
- `object_methods.rs` の `eval_method_call_evaled`（`Instance`/`PyObject` の 2 アームだけを
  持つ独自実装・85 行）を **統一実装への委譲 1 行**に置換。
- **`Value::Instance` アーム 144 行を `call_instance_method_evaled` への委譲に畳んだ**。
  同じ判断（copy／method IC／gen／native／static・class 判定／不変性フィルタ／オーバーロード）が
  二重実装されていた。
- `eval_str_method` / `exec_signal_method` / `exec_event_loop_method` を評価済み引数版へ
  （いずれも冒頭で `eval_call_args` を呼んでいただけ・呼び出し元は各 1 箇所）。
- **コンパイラのレシーバ制限を撤廃**し、`Op::CallMethod`/`CallMethodLocal` に **`node_id` を追加**。

⚠ **`node_id` の追加が要点**。ツリーウォークは `eval_call` の `Expr::Attr` 分岐で
①外部言語の判定 ②ディスパッチ ③`check_ffi_return` の 3 手順を踏む。制限を外して
`Namespace`/`PyObject` レシーバを VM に流すと、**③ が VM 経路だけ素通り**する
（`Op::Call` で #22-a が踏んだ穴と同型）。op で `node_id` を運んで同じ 3 手順を再現した。

⚠ **評価順が 1 点だけ変わる**: 旧 `expect_no_args` は**引数を評価せずに** arity エラーを返していた
（15 箇所）。今は先に評価する。正常系（0 引数）では差が無く、差が出るのは
「no-arg メソッドに副作用つき引数を渡す」＝どのみちエラーになるコードだけ。
`eval_one_arg` は元から全評価後に検査していたので不変。
**確保は増えない**（`Vec::new()` は 0 引数で確保しない／1 引数以上は元から確保していた）。

⚠ `str.format` **だけ**がキーワード引数を使う。値だけに落とす前に処理しないと `k=v` の名前が失われ、
他メソッドの arity 検査の見え方まで変わる（従来は名前つきの値も `vals` に入っていた）。

#### 結果

| 指標 | before | after |
|---|---|---|
| `fn_FAILED` | 29 | **17** |
| `toplevel_FAILED` | 163 | **65** |
| `method-receiver:*` の bail | 110 | **0** |
| 関数本体内のツリーウォーク | 158 | **127** |
| コード | — | 実質 **-230 行**（Instance 144＋旧 evaled 85） |

#### 性能（3 段階で詰めた）

初回は `bench_method_hot` **0.943x**。`--vm=off` では **1.001x** だったので
**ツリーウォークの畳み込みは中立**、VM のディスパッチ側と特定できた。

1. `foreign_call_lang` を Instance 経路より前に呼んでいた → 非 Instance 側へ追い出す（0.943→0.971x）
2. `vm_method_call` という関数レイヤーを 1 枚挟んでいた → `exec_op` の arm で
   `matches!(obj, Value::Instance(_))` を直接見て `call_instance_method_evaled` へ直行
   （0.971→**0.981x**、`bench_method_body` は 1.009x）
3. 非 Instance 経路は `#[inline(never)]`（`exec_op` は `#[inline(always)]`）

残る ~2% はこの種の変更のレイアウト帯（±5%）の内側。**最頻路に判定を 1 つ足すだけで 3% 動く**
のがこの経路の性質で、`vm_method_call` のような「きれいな 1 枚のラッパ」は
ここでは実測で不利だった。

#### 残り（#3 に必要）

- `fn_FAILED` 17: 入れ子 `fn`（クロージャ）8・`static` 2・可変長 2・未帰属 2・その他 3。
- `toplevel_FAILED` 65: 虚数リテラル 13・`let x = <識別子>` 13・テンプレート実体化呼び出し 13・
  キーワード/可変長引数 8・`for` タプル分解 7・未帰属 7・その他 4。
- 最上位のツリーウォーク 2,071 件は**式文 909 が最大で、まだ試行すらしていない**（#10-c2）。

### #27-c 最上位 bail の解消 — 進行中（175 → 134）2026-08-14

#### まず計測の穴を 2 つ塞いだ（数字が読めていなかった）

**(1) `toplevel_FAILED` 511 のうち 336 件は「失敗」ではなかった。**
#10-c2 で受理判定を除外リストにしたとき、定義文は `compile_toplevel_stmt` が入口で `None` を返す。
呼び出し側はそれを一律「コンパイル失敗」として計上していたため、**意図的なスキップが失敗に化けていた**。
`is_toplevel_compile_target` を公開し、`try_run_toplevel_stmt` が**対象外なら計上もキャッシュもしない**
ようにした（`toplevel_FAILED` 511 → 175 ＝ bail 合計と一致）。
⇒ **「対象外」と「失敗」を同じ `None` で表すな**、という設計上の教訓。

**(2) 未帰属 46 → 3。** 未帰属の bail に**文種別を添える**ようにしたら 33 件が `Expr` と分かり、
そこから `Expr::Ident` のフォールバック（slot にもグローバル解決にも載らない識別子）が
無記録で落ちていると特定できた。**識別子名まで記録**する形にしたら原因が一目で分かった
（`mng` 13・`Color` 7・`MyEnum` 3 …）。あわせて block 式／if 式／match 式／ループ式の
脱出制御 bail と temp slot 枯渇も計装した。

#### 実際の修正 3 件（**新しい opcode はゼロ**）

| 対象 | 件数 | 原因と修正 |
|---|---|---|
| `enum` / `new_type` 名 | 10 | `collect_program_globals` が **`EnumDef`/`NewTypeDef` を集めていなかった**。最上位に名前を作るのに `Resolution::Global` が付かず、VM が「slot にもグローバルにも無い識別子」として bail していた |
| `mng <- async->T:` の受信者 | 18 | `collect_bound_names` が **`AsyncAssign` の `target` を束縛扱い**していた。`exec_async_assign` は `get_var(target)` するだけ（未定義なら `NameError`）で**名前を作らない**ので、シャドウ候補に入れるのは誤り |
| 虚数リテラル | 13 | `Expr::ImaginaryLit(f)` → `Op::Const(Value::Complex(0.0, f))`。`eval` と同一 |

⚠ **どちらの誤りも「保守的すぎる」方向のバグ**で、動作は正しいまま最適化だけが効かなくなる。
`AR_TW_STATS` で bail の**名前まで**出さなければ永遠に見つからなかった。

#### 結果

| 指標 | before | after |
|---|---|---|
| `toplevel_FAILED` | 175 | **134** |
| 最上位 Chunk のコンパイル成功 | 1,368 | **1,409** |
| 最上位ツリーウォーク | 643 | **596** |
| E2E（A/B・release） | — | **退行なし**（1.00〜1.07x） |

⚠ リゾルバを変更したので [dump_native_ir.ps1](dump_native_ir.ps1) で **IR byte-identical を確認済み**
（`collect_program_globals`/`collect_bound_names` はネイティブ codegen が消費する解決結果に効くため）。

#### 残り 134 件と、#3 から見た優先度

| bail | 件数 |
|---|---|
| `callee-expr:TemplateInstantiate` | 31 |
| `expr:Slice` | 21 |
| `toplevel-let-from-ident` | 13 |
| `call-arg`（キーワード/可変長） | 10 |
| `stmt:EventSubscribe` / `stmt:Freeze` / `for-tuple-target` | 各 7 |
| `stmt:LetTuple` 6・`stmt:Block` 4・組み込みグローバル名（`EventLoop`/`Async`/`Encoding`…）約 10 | |

⚠ **ただし #3 の観点では件数が優先度ではない**（#10-d と同じ罠）。#3 は TLS/センチネルの削除なので、
効くのは**制御フローを含む文**だけ。最上位に残る非定義文 ~239 件のうち制御フローを持つのは
**`If` 19・`For` 9・`Block` 5・`Match` 3・`Try` 2・`While` 1 の計 39 件**（＋ブロック式内の
`BlockReturn` 19）。**この 39 件を落としている bail から潰すのが #3 への最短路**。

#### 組み込みグローバル名（約 10 件）を潰すときの注意

`EventLoop`/`Async`/`Encoding`/`FileOpenMode`/`StartPoint`/`ByteRecognizingMode` は
**インタプリタが起動時に `scopes[0]` へ登録する**グローバルで、AST には宣言が無いため
リゾルバが知らない。⚠ **「未解決なら `LoadGlobal` に落とす」で済ませてはいけない** —
`len` のような**純粋組み込みは `scopes[0]` に居ない**ので `NameError` になる（#15d の再演）。
潰すなら「起動時に登録される名前の集合」をリゾルバへ渡す形にすること。

### #27-c 続き — bail を「どの文を落としたか」で切る 2026-08-14

#### 計測: bail に**最上位文の種別**を前置した

bail 理由だけでは「何を落としたか」が分からず、#3 に効く**制御フローを含む文**を
選べなかった。`ToplevelCompileGuard` に文種別を持たせ、キーを `<文種別>/<理由>:<詳細>` にした。
⚠ **キーに空白を入れないこと**（集計スクリプトが `key=value` を空白で分割する。一度踏んだ）。

これで #3 に効く bail が一目で分かった（134 件中の**制御フロー文**）:
`For/for-tuple-target` 7・`Block/stmt:Block` 4・`Try/unattributed` 2・その他 5 ＝ **18 件**。
⚠ 併せて分かったこと: ツリーウォークに残る `If` 19 は**最上位の `If` ではなく、bail した親文の
本体に入れ子になった `If`** だった（親が VM 化されれば消える）。**内訳の文種別を
「最上位文の種別」と読み違えない**こと。

#### 実装 2 件

| 対象 | 件数 | 手法 |
|---|---|---|
| `block:` 文 | 4 | `Stmt::Block` をブロック式のコンパイラで処理し値を `Pop`。ツリーウォークの `exec_block_stmt` は **`block_return` を吸収**するので意味論が一致する（脱出制御を含む本体は `block_body_bails` が元から弾く） |
| `for k, v in ...` | 7 | **`Op::UnpackTuple(src_slot, n)` を 1 つだけ新設**。要素をスタックへ push し、既存の `StoreLocal` を**逆順**に並べて受ける（op を 2 つ足さずに済む） |

⚠ **temp slot の解放を忘れない**。`for` の複数ターゲットは受け皿 temp を 1 つ増やすので
`free_temp()` も 2 回要る（1 回のままだと以降の temp 番号がずれる）。

#### 結果

| 指標 | before | after |
|---|---|---|
| `toplevel_FAILED` | 134 | **128** |
| 最上位 Chunk のコンパイル成功 | 1,409 | **1,415** |
| 最上位ツリーウォーク | 596 | **572** |
| 制御フロー文の bail | 18 | **12** |
| E2E（A/B・release） | — | 焦点測定で **0.973〜1.003x**（この規模の変更の揺れ ±5% の内側） |

#### 残り 128 件（うち #3 に効くのは 12 件）

制御フロー文の bail: `For` 4（`call-arg`/`stmt:LetTuple`/未帰属/`attr-compound-target:Subscript` 各 1）・
`Block` 3（`ident-unresolved` 2・`store-target` 1）・`Match` 2・`Try` 2・`While` 1。

件数上位（#3 には効きにくい・一度きりの式文）: `TemplateInstantiate` 31・`Slice` 21・
`toplevel-let-from-ident` 13・`call-arg` 11・`stmt:Freeze` 7・`stmt:EventSubscribe` 7・`stmt:LetTuple` 7。

⚠ `Block/ident-unresolved:*` と `For/stmt:LetTuple` は**今回 `block:`/`for` が通るようになった結果、
より深い bail が露出したもの**。潰すたびに次の層が見えるので、件数の増減だけで判断しないこと。

### #27-c 続き（2）— 制御フロー bail 12 → 10 / 2026-08-14

#### 実装 2 件

**(1) VM コンパイラへ渡すグローバル集合を「シャドウ減算なし」に変えた。**
リゾルバ用の `toplevel_visible_globals`（プログラム全体のシャドウを引いた集合）を渡していたが、
**VM コンパイラにその減算は不要**だった。理由:
- リゾルバは AST ノードに `Resolution::Global` を**一度だけ焼く**。そのノードは `for i in ...` の
  本体でも評価されうるので、全体のシャドウを引く必要がある。
- VM コンパイラは**最上位文を 1 つずつ**コンパイルし、その文で束縛される名前は全部 `slots` に入る。
  最上位に他の囲みスコープは無いので **`slots` に無い名前は必ず `scopes[0]`**。
  減算後の集合を渡すと、別の文の `for i in ...` のせいで `while i < N` の `i` まで落ちる。

`resolver::toplevel_declared_globals` を新設して切り替え、識別子読みのフォールバックでも
（**`slots` を引いた後に**）グローバル読みへ落とすようにした。
⚠ **順序が健全性そのもの**。`slots` を先に引かないと、本当にシャドウしているローカルを
グローバルとして読んでしまう。

⚠ **正直な評価**: この変更単体では bail 件数は動かなかった（128 → 128）。
書き込み側の bail が読み取り側へ移っただけで、実際に減ったのは次の (2) と合わせてから。
一般性と健全性の理由で残している。

**(2) `collect_nested_decls` が `Stmt::Block` を降りていなかった。**
最上位の `block:` 文の中の `let` に slot が割り当てられず、「slot にもグローバルにも無い識別子」
として bail していた。⚠ **`block_body_bails` は元から `Stmt::Block` を降りており、
2 つの walker が不整合**だった。あわせて `Stmt::Raise` の式も拾うようにした。

#### 結果

| 指標 | before | after |
|---|---|---|
| `toplevel_FAILED` | 128 | **126** |
| **制御フロー文の bail** | **12** | **10** |
| 最上位ツリーウォーク | 572 | **563** |
| `Block` のツリーウォーク | 4 | **1** |
| E2E（A/B・release） | — | **1.008〜1.039x**（退行なし・新 op ゼロ） |

リゾルバを触ったので **IR byte-identical を確認済み**。

#### 残り 126 件（うち #3 に効くのは 10 件）

`For` 4（`call-arg`/`stmt:LetTuple`/未帰属/`attr-compound-target:Subscript` 各 1）・
`Try` 2（`try-except-finally` 1・未帰属 1）・`Match` 2（`ident-unresolved`）・
`While` 1・`Block` 1（`ident-unresolved:outer`）。

⚠ 残る `ident-unresolved:outer` は**入れ子関数のキャプチャ変数**とみられ、#27 の
「入れ子 `fn`（クロージャ）」と同じ根（クロージャ非対応）に行き着く。
件数上位（`TemplateInstantiate` 31・`Slice` 21・`toplevel-let-from-ident` 13）は
一度きりの式文で #3 には効かない。

### #27 クロージャ対応 — 外側関数を VM 化（`decl-prepass:FnDef` 8 → 0）2026-08-14

#### 計測: クロージャの問題は 2 つに分かれる

計装を 2 つ足して切り分けた（`vm_ineligible` と `closure_capture`）。

| 区分 | 件数 | 意味 |
|---|---|---|
| `vm_ineligible: closure-capture` | **20** | **クロージャ本体**の呼び出しがコンパイル前に弾かれている（`captured_env` が非空） |
| `decl-prepass:FnDef` | **8** | **外側の関数**が入れ子 `fn` を含むせいで丸ごと bail |
| キャプチャ内訳（生成 27 件） | none 12 / immutable-only 8 / **has-mutable 7** | 可変キャプチャが 26% |

⚠ `vm_ineligible` は **`vm_eligible` が偽だとコンパイルを試みない**ため bail 統計に現れず、
それまで完全に見えていなかった。**「弾かれた理由」も計上しないと穴が見えない**。

#### なぜ可変キャプチャだけ別格か

`capture_env` は可変変数を `Var::Mutable` → **`Var::Cell(Rc<RefCell<Value>>)` へその場で昇格**し、
外側スコープと**同じセルを共有**する。VM のフラット slot は `Value` 直値なので**共有セルを表現できない**。
対応にはフレーム表現の変更（slot と並行するセル表を持ち `LoadCell`/`StoreCell` を足す等）が要る。
＝ §2.4 が「相性が良い」と書いていた部分のうち、**可変キャプチャだけは表現の変更が必須**。

#### 実装（外側関数側のみ・本体側は未対応）

- **`Op::MakeFn(idx)` ＋ `Chunk.fn_defs`** を新設。入れ子 `fn` の関数値を作って slot へ書く。
- **健全性の要**: コンパイラが `nested_fn_captures` で「自由変数 ∩ 外側 slot」を求め、
  **可変が 1 つでもあれば bail**。不変のみなら (名前, slot) を記録し、実行時に**値を複製**して
  `CapturedVar::Immutable` を作る。`capture_env` の不変分岐と一致する。
  ⚠ **`capture_env` と自由変数の定義がずれると閉包変数が黙って消える**。片方を変えたら両方見ること。
- **`exec_fn_def` とのオーバーロード合成を `merge_fn_overload` に集約**（#22 系列の「畳む」）。
- 採番順: リゾルバの `collect_base_decls` が入れ子定義名も base slot に載せるので、
  **decl-prepass でも `Stmt::FnDef` を同じ順で採番**する（他の定義文は従来どおり bail するので整合は保たれる）。
  ⚠ **採番だけ prepass で行い、載せられるかの判定は `compile_stmt`** で行う（自由変数の判定に `slots` の完成が要る）。
- デコレータ・テンプレートは bail（`eval` と `TemplateFnValue` の再現が要るため）。

#### 結果

| 指標 | before | after |
|---|---|---|
| `fn_FAILED` | 17 | **11** |
| `decl-prepass:FnDef` | 8 | **0**（残るのは `nested-fn-mutable-capture` 2） |
| 関数本体内のツリーウォーク | 127 | **111** |
| `FnDef` のツリーウォーク実行 | 16 | **8** |
| VM コンパイル成功（fn） | 338 | **344** |

**残る `vm_ineligible: closure-capture` 20 は手つかず**（クロージャ**本体**の VM 化）。
本体側は「キャプチャ変数を slot へ束縛して実行」で不変分は届くが、
**クロージャ実体ごとに `FnValue` が別物なので Chunk が使い回せない**（`vm_chunks` は `Rc<FnValue>`
アドレス鍵）。ループ内で作られるクロージャでは**コンパイルし直しが毎回**になり得る点を織り込むこと。

#### ⚠ op を 2 つ足した代償（実測）

今回 `UnpackTuple`（#27-c）と `MakeFn` を足した結果、VM 支配ベンチが
`bench_control_flow` **0.944x**・`flat_bench_interp` **0.936x**。`--vm=off` では
`bench_control_flow` が **1.027x** なので、**VM ディスパッチループの肥大**が原因と確定
（この op 自体は当該ベンチで 1 回も実行されない）。
当時は「op 1 個あたり ~1〜1.5%」と解釈したが、**#28 でこのモデルは反証された**（配置ノイズ）。
⇒ **`Op::Rare(RareOp)` への畳み込み**（稀な op を 1 アームに集約し `#[inline(never)]` の
二段目 match で捌く）を実施する価値が出てきた。対象は
`MakeFn`/`UnpackTuple`/`BreakPoint`/`DeclareGlobal`/`GetTraitAttr`/`SetTraitAttr`/`LoadSelfClass` の 7 個。

### #25 `--vm=force` を実際のゲートにする — 完了 2026-08-14

`VmMode::Force` は `main.rs` でパースされるだけで `Auto` と一切区別されておらず、
文書の各所にあった「`--vm=force` で穴を可視化」は**実装されていなかった**。

#### 実装

**フォールバック禁止**にした。Force のとき、以下で `VmForceError` を返して止める:

| 箇所 | 条件 |
|---|---|
| `try_run_toplevel_stmt` | 対象の最上位文が Chunk 化できない |
| `exec_fn_evaled` | 関数本体の Chunk が無い（**`vm_eligible` が偽＝クロージャ等も含む**） |
| `exec_generator` | ジェネレータ本体の Chunk が無い |

⚠ **`vm_eligible` が偽の場合も失敗として扱う**のが要点。そこを見逃すと
「bail 0 なのにツリーウォークが残る」というゲートの穴になる（#27 で見つけた `vm_ineligible` 20 件がまさにそれ）。

⚠ **定義文は対象外**（`is_toplevel_compile_target` が false）。制御フローも TLS も持たず、
設計上インタプリタが実行する（#10-d の判断）。含めると永久に 0 件にならない。

エラーには**文種別と位置**を出す（`Stmt::Expr` 等は Span を持たないので種別は必須）。
理由（bail の種別）は載せない — それは `AR_TW_STATS` の役目。

#### 役割分担（重要）

| 手段 | 役割 |
|---|---|
| `force_gate.ps1`（新設） | **止めて判定するゲート**。0 か 0 でないか |
| `AR_TW_STATS` / `tw_stats.ps1` | **数えるだけの計測**。何がどこに何件あるか（潰す作業はこちら） |

⚠ 件数を数える目的に `force_gate.ps1` を使わないこと（最初の 1 件で止まるので実数は出ない）。

#### 初回計測: **125 例題中 36 件がまだフォールバックする**

内訳の傾向（1 ファイル 1 件しか出ないので参考値）: 最上位文が大半で
`Let`/`Mut`/`While`/`AttrCompoundAssign` 等、関数は `_setup`/`count`/`read_counter`/`shadow_loop_var`。

**これが #3 へ進めるかの唯一の判定基準**になった。`--vm=auto` の挙動は一切変えていない
（`compare_vm_modes` 71 identical・デバッガ 5 identical・例題 FAIL 0）。

⚠ PS5.1 は `if` を**式として**書けない（`-ForegroundColor (if ...)` は PS7 の機能）。色は先に変数へ。

### #28 `Op::Rare` への畳み込み — **却下**（実装して A/B し、前提の反証を得た）2026-08-14

#### やったこと

稀な 7 op（`MakeFn`/`UnpackTuple`/`BreakPoint`/`DeclareGlobal`/`GetTraitAttr`/`SetTraitAttr`/
`LoadSelfClass`）を `Op::Rare(RareOp)` の **1 アーム**に畳み、`#[inline(never)]` な二段目 match で捌いた。
`exec_op`（`#[inline(always)]`）のアーム数を 7 → 1 に減らす狙い。実装は完走し、
検証も全て緑（706 緑・off/auto 71 identical・デバッガ 5 identical・例題 FAIL 0）。

#### 結果: **何も回復しなかった**

| ベンチ | #27 クロージャ着手前からの通算 |
|---|---|
| `bench_control_flow` | 0.944x → **0.936x**（畳み込み後もほぼ同じ） |
| `flat_bench_interp` | 0.936x → **0.962x** |
| `bench_arith` | 0.947x → **0.892x**（**悪化**） |
| `bench_branch` | — → 0.994x |

#### 反証された前提

これまで「**`exec_op` は `#[inline(always)]` なので、アームを足すとディスパッチループ全体が太る
＝ op 1 個あたり ~1〜1.5%**」というモデルで設計判断をしていた（#27/#27-a で「実行されないダミー
4 op を足したら同じ差が出た」ことを根拠にしていた）。

**アーム数が原因なら 6 個削れば回復するはず**だが、回復しなかった。
⇒ **効いているのはアーム数ではなくコード配置**。#27 のダミープローブは op だけでなく関数・メソッドも
足しており、「op を足したから」と「同規模の摂動を加えたから」を**区別できていなかった**。

**正しいモデル**: この規模の変更は VM 支配ベンチを**どちら向きにも ±5% 揺らす**。
数 % の差から「op 追加のコスト」のような因果を読み取ってはいけない。

#### 判断

前提が反証された以上、**指標の改善が無く二段ディスパッチの間接だけが残る**ので巻き戻した。
⚠ ただし **`Op` のサイズ不変条件テスト（`op_size_is_pinned`・現在 20B）は残した**。
これは配置ノイズと無関係に効く本物の指標（最大 variant を超えると命令列全体が太る）。

#### 教訓（この系列の計測手順へ追加）

- **「原因 X」を疑ったら、X を減らす実験もすること**。増やす実験だけでは
  「X が原因」と「同規模の摂動なら何でも起きる」を区別できない。
- 数 % の E2E 差は**設計判断の根拠にならない**。使えるのは
  「`--vm=off` でも同じ差が出るか（＝経路の切り分け）」と「ハードな計数（bail 件数・
  ディスパッチ回数）」の 2 つ。

### #27-c 続き（3）— 最上位 bail 126 → 23 / `force_gate` 36 → 17 / 2026-08-15

#### 判断基準を変えた

#25 で `force_gate.ps1` ができたので、**「#3 に効くのは制御フロー文の bail だけ」という
それまでの絞り込みは無効**になった。ゲートは「1 文でも載らなければその例題は落ちる」なので、
制御フローかどうかに関係なく**全部潰す**必要がある。以降は件数上位から着手した。

#### 載せたもの

| 対象 | 手法 | 減った bail |
|---|---|---|
| スライス式 `a[b:e:s]` | `Op::BuildSlice`。省略要素は `Op::Nil` を積み、`slice_from_values` が `Value::None` を「無し」に畳む | 21 |
| テンプレート実体化 `Tmpl[T](...)` | `Op::CallTemplate` + `Chunk::type_arg_lists` | 31 |
| `let x = <識別子>` | `DeclKind::LetFromIdent(src)`。**コンパイル時に結論を出さず**実行時に `get_var(src)` を引く | 13 |
| 最上位の未解決識別子 | `Op::LoadName`（`Signal`/`dict`/`EventLoop` 等の組み込み型名） | 32 |
| `freeze x` | `Op::FreezeVar` → `exec_freeze` をそのまま呼ぶ | 7 |
| `on`/`once`/`off` | `Op::EventSubscribe`/`EventUnsubscribe` + `*_evaled` 分離 | 9 |
| `let a, b = t` | `Op::LetTuple` + `Chunk::tuple_decls`（束縛先が slot かグローバルかを持つ） | 7 |

`toplevel_FAILED` **126 → 23**、`force_gate` **36 → 17/125**、最上位ツリーウォーク **497 → 429**
（残り 429 のうち 350 は定義文＝設計上の対象外）。

#### 未解決識別子を `LoadName` にできる理由と、その限界

ツリーウォークの `Resolution::Unresolved` は `get_val(name)` そのもので、
`Op::LoadName`（`vm_load_name` = `get_val`）と**エラー文言まで同一**。よって置き換えは無条件に健全…
**ではない**。スコープの隔離を担うのは `frame_floor` で、`exec_fn_evaled` は VM へ分岐する時点では
**まだ `frame_floor` を進めていない**。関数本体で `LoadName` を使うと `get_val` が
**呼び出し元のローカルまで走査してしまう**。

最上位は `toplevel_vm_candidate` が `scopes.len() == 1` を保証するので安全。よって
**最上位モード限定**で置き換えた（`fn_bail` に残る `g_counter`/`Registry` の 2 件は対象外のまま）。

#### テンプレート実体化: 引数を未評価のまま持ち回る

`instantiate_template` は `exec_fn` / `instantiate` / `exec_generator` に `&[CallArg]` を渡していた。
VM はスタック上の評価済み値しか持たないので `*_evaled` へ切り替える必要があるが、
**入口で一括評価すると評価順が変わる**（`check_template_constraints` より前に引数の副作用が出る）。

`TemplateArgs { Ast(&[CallArg]), Evaled(Vec<...>) }` を導入し、**呼び先へ渡す直前**に
`into_evaled` する形にした。これで本体は 1 つのまま、評価の時点が元と一致する。
副作用として `instantiate` / `exec_generator`（`eval_call_args` + `*_evaled` の 2 行ラッパ）が
最後の呼び出し元を失ったので削除した。

#### `let a, b = t` — walker と除外リストは**対で**直さないと壊れる

このタスクで**同じバグを 2 方向から踏んだ**。どちらも検証スクリプトが捕まえた。

1. `collect_nested_decls` が `Stmt::LetTuple` を見ていなかった。→ `for` 本体の
   `let zx, let zy = pair` に slot が振られず**グローバル宣言**に落ち、
   2 周目で `NameError: variable 'zx' is already declared`（`built_in.ar`）。
2. 1 を直したら、今度は**最上位の宣言文**にも slot が振られるようになり、
   `let tx, mut ty = tup1` の値がフレームへ消えて `NameError: 'tx' is not defined`（`collection.ar`）。

正しい形は **2 箇所を対で直すこと**:
- `collect_nested_decls` は **入れ子の** `LetTuple` ターゲットに slot を振る
- `compile_toplevel_stmt` の宣言文除外リスト（#10-c で `Let`/`Mut`/`Const` に入れたもの）に
  `LetTuple` を追加し、**最上位の**ターゲットには振らない

この「slot の有無」がそのまま `Op::LetTuple` の束縛先（slot / `declare_var`）の判別子になる。

⚠ 教訓の再確認: **同じ木を歩く walker が 2 つあると必ずずれる**（既出）。加えて
**宣言文の扱いは walker と除外リストの 2 箇所に散っている**ので、片方だけ直すと
「値が消える」「二重宣言」のどちらかに倒れる。**両方向のテストが要る**。

#### 検査の 1 実装化（#22 の原則の適用）

新しい op を足すたびに、ツリーウォーク側の本体を `*_evaled` へ割り、**検査とエラー文言を
1 箇所に集約**した: `slice_from_values` / `let_tuple_values` / `event_subscribe_evaled` /
`event_unsubscribe_evaled` / `instantiate_template_args` / `let_freeze_instance`。

唯一意図的にずらしたのはスライスの評価順で、旧実装は begin を**検査してから** end を評価していた
（begin が不正なら end/step は評価されない）。今は 3 つとも評価してから検査する。
差が出るのは「不正な境界＋副作用つき境界式」＝どのみち TypeError になるコードだけ。

#### 続けて載せたもの（同日・126 → 23 → **5**）

| 対象 | 手法 | 直った例題 |
|---|---|---|
| キーワード/可変長引数 | `Op::CallKw` + `Chunk::kw_calls`。可変長は `BuildList` で**1 値に畳む**ので、スタック配置は `Op::Call` と同じ | 4 |
| 添字への複合代入 `d[k] += v` | `object`/`index` を**2 回積む**（ツリーウォークが読みと代入で 2 回評価するため） | 2 |
| `obj.attr = v`（レシーバ無制限） | `attr_assign` を `attr_assign_evaled` へ畳んだ（下記） | 1 |
| `for _ in ...` | `_` には `add_decl` が slot を振らないので受け皿 temp を割り当てる | 1 |
| `open` / `close` | `eval_builtin_open_evaled` を分離し `VM_BUILTIN_NAMES` へ追加 | 2 |

`Op` は 20 バイトのままに保った。`Op::Call` は既に最大 variant（5 フィールド）なので、
キーワード名は**副表へ逃がす**しかない（`ffi_call_info` と同じ判断）。

#### コンパイラのレシーバ制限は「2 実装の差」を隠していただけだった

`Stmt::AttrAssign` は `object_is_instance(object)` が真のときしか `Op::SetAttr` を出さず、
型注釈の無いグローバル（`mut p2 = p1.copy()` の `p2`）が bail していた。

調べると、**ツリーウォークの `attr_assign` と VM の `attr_assign_evaled` が別実装**で、
前者にだけ `Value::Class`（`static mut` への代入）のアームがあった。
`object_is_instance` はその差が露見しないようレシーバを絞る役をしていた。

`attr_assign` を `eval(object)` + `attr_assign_evaled` へ畳み、`Value::Class` アームを
`attr_assign_evaled` 側へ移した。**1 実装にした結果、制限そのものが不要になった**。

⚠ 一般化: **コンパイラ側の「この形のときだけ載せる」条件は、意味論上の制約とは限らない。
2 実装の差を隠しているだけのことがある**。条件を緩める前に、まず呼び先が 1 実装かを確かめること。

#### 計測の穴（`unattributed`）と、その修正で作った新しい穴

`For/unattributed:For` の出所は素の `*self.slots.get(name)?` だった（`?` で黙って諦めるので
`record_bail` を通らない）。`slot_of` ヘルパーを作って全置換したところ、`toplevel_FAILED` 5 に対し
**bail が 39 件**という食い違いが出た。

原因は `compile_async_assign` の `filter_map`。ここは
**「本体が参照する名前のうち slot にあるものだけ捕捉する」＝当たらないのが正常**な場所で、
`slot_of` を通したせいで幻の bail が 35 件載っていた。1 箇所だけ素の `get` に戻して bail 5 = FAILED 5 に一致。

⚠ **「対象外」と「失敗」を同じ `None` で表すな**（#27-c で既出）の**再発**。
今回は「失敗を計上する」側を機械的に全置換したことで、逆向きに同じ罠を踏んだ。
`Option` を返すヘルパーを一括置換するときは、**呼び出し側それぞれで `None` が何を意味するか**を見ること。

#### 残り 5 件

`try-except-finally` 1・`Try` 内の `let v = <未定義識別子>`（`decl-no-slot`）1・
メソッドのキーワード引数（`For/call-arg`）1・`block:` 式の中の入れ子 `fn`（`Let/decl-no-slot`）1・
`callee-expr:Block` 1。

`force_gate` は **12/125**。うち **7 件は #27（関数本体）**で、#27-c 由来は 5 件。

⚠ `let v = <識別子>` の入れ子版は、最上位でやった `DeclKind::LetFromIdent` と同じ手
（実行時に `get_var(src)` の可変性を見る）で載るが、**最上位モード限定**にすること
（`LoadName` と同じ `frame_floor` の理由）。

#### 検証

`cargo test` 706 / `compare_vm_modes` 71 identical・0 differing / `scan_examples` FAIL 0 /
`compare_debug_modes` 5 identical / clippy 62（増分 0）。
`Op` サイズは 20 バイトのまま（`op_size_is_pinned` 通過）。

### #10-c2 最上位の残り文を Chunk 化 — 完了 2026-08-14

#### 実装

- `compile_toplevel_stmt` の受理判定を **許可リスト → 定義文の除外リスト**に反転した。
  許可リストだと新しい文種別を黙って取りこぼす（着手時、**式文 909 件が「まだ試行すらしていない」**
  状態で残っていた）。`compile_stmt` が対応していない文はそこで bail するので、
  入口で切るのは「試すだけ無駄と分かっているもの」＝定義文だけでよい。
- `exec()` の最上位 VM 試行を **5 つのアームから入口 1 箇所へ集約**した。
  対象の文種別が増えるたびに同じ 3 行がアームへ散るのを止めるため。
  ⚠ 入口に置く判定は `toplevel_vm_candidate`（フィールド 3 本の比較・`#[inline(always)]`）だけ。
  `exec()` は全文で呼ばれるので重い判定を足すと全体が遅くなる（#10-a で 11% を実測済み）。
  ⚠ **デバッガの `should_pause_at` より後**に置くこと（先に置くと off/auto でステッピングが食い違う）。

#### ⚠ 計測フックのバグを 1 件修正した（これまでの数字が過大だった）

`record_stmt` が **VM 試行より前**にあったため、**VM へ渡した最上位文もツリーウォークとして
数えていた**。「ツリーウォークが実際に実行している文」という指標の意味が崩れていた。

修正の正しさは自己整合で確認できる: 同一実行で
**旧指標 2,030 − 新指標 663 = 1,367** ≒ **実行された最上位 Chunk 数 1,368**。

#### 結果

| 指標 | before | after |
|---|---|---|
| 最上位 Chunk のコンパイル成功 | 499 | **1,368** |
| 最上位ツリーウォーク（**修正後の指標**） | ≈1,572 | **663** |
| E2E（A/B・release） | — | **中立**（焦点測定で 0.982〜0.998x） |

#### 残り 663 件の性質が変わった

| 種別 | 件数 | 行き先 |
|---|---|---|
| **定義文**（FnDef 210・ClassDef 97・Import 28・TraitDef 12・FromImport 8・NewTypeDef 8・ProtocolDef 3・EnumDef 2・GenDef 1） | **369（56%）** | **#10-d**（`exec_module` 本体と定義文のオペコード化） |
| 式文 `Expr` | 98 | #27-c（bail 内訳へ） |
| `Let`/`Mut`/`CompoundAssign`/`If`/… | 196 | #27-c |

**最上位に残るツリーウォークの過半は定義文**になった。#3 に向けて次に効くのは #10-d。

#### 最上位 bail 511 件の新しい内訳（#27-c）

`toplevel_FAILED` が 65 → 511 に増えたのは**試行する文が激増したため**（比較すべきは常に
ディスパッチ数）。内訳: 未帰属 46・テンプレート実体化呼び出し 31・`Slice` 21・虚数リテラル 13・
`let x = <識別子>` 13・キーワード/可変長引数 10・`EventSubscribe` 7・`freeze` 7・
`for` タプル分解 7・`LetTuple` 6・`block:` 文 4 ほか。
⚠ **未帰属 46 件**は計装の穴。#27-c に着手するならまずここを潰すこと（#10-a と同じ手順）。

### #10-d 定義文のオペコード化・import モジュール本体 — **保留**（計測で価値なしと判明）2026-08-14

着手前に 2 点を計測した結果、**タスクの両半分とも #3 に寄与しない**と分かったので保留する。
計測用に `AR_TW_STATS` を「メイン最上位 / import モジュール本体 / 関数本体内」の 3 分類へ拡張した
（`ModuleBodyGuard`。モジュール本体は `exec_module` が `push_scope` してから回すので
`toplevel_vm_candidate` の `scopes.len() == 1` が偽になり、**現状まるごとツリーウォーク**）。

#### 計測 1: 「import モジュール本体」は 20 文しかない

| 区分 | 件数 |
|---|---|
| メイン最上位 | **642** |
| **import モジュール本体** | **20**（ClassDef 19・FnDef 1） |
| 関数本体内 | 127 |

モジュール本体を VM 化するには `StoreGlobal`/`DeclareGlobal` の書き込み先を
`scopes[0]` から**現在のスコープ**へ変える設計変更が要る（モジュールの globals は
push されたスコープに入る）。**20 文のためにそれを入れる価値は無い**。

#### 計測 2: 定義文は TLS/センチネルを使わない ＝ #3 に寄与しない

メイン最上位 642 件の内訳は **定義文が 350 件（55%）**（FnDef 209・ClassDef 78・Import 28・
TraitDef 12・FromImport 8・NewTypeDef 8・ProtocolDef 3・EnumDef 2・GenDef 1）。

しかし **`src/interpreter/exec/definitions.rs` は TLS/センチネルを使うファイル一覧に入っていない**
（`LOOP_DEPTH`/`BLOCK_YIELDS`/`BLOCK_RETURN_EXPECTED_TYPE`/`GENERATOR_YIELDS`/
`RAISE_SENTINEL`/`BREAK_SENTINEL` を含むのは 12 ファイルで、定義文の実行はそこに無い）。

**#3 の実体は「TLS/センチネルの実削除」**なので、判断はこうなる:

- 定義文は**制御フローを持たない**（登録するだけ）。バイトコードに包んでも
  `[Define, ReturnNil]` の 2 命令になるだけで、**逐次実行を逐次実行に置き換える儀式**にしかならない。
- 定義文は **TLS を使わない**ので、ツリーウォークに残っていても TLS の削除を妨げない。
- ⇒ **#3 をブロックしているのは定義文ではなく bail の方**（最上位 511・関数 17）。
  そちらは `for`/`while`/`if`/`try` を含み、まさに TLS を使う。

#### 結論と申し送り

- **#10-d は保留**。再訪の条件は「モジュール本体にホットループを持つ実プログラムが出たとき」
  （そのときは書き込み先スコープの設計から入ること）。
- **#3 の前提から #10-d を外し、#27-c（最上位 bail 511）と #27（関数 bail 17）に置き換える**。
- ただし **#25（`--vm=force` のゲート化）を作るときは定義文の扱いを決める必要がある**。
  「bail 0」を要求すると定義文が永久に引っかかる。**定義文は設計上インタプリタが実行する**
  （`parse_ar` 等と同じ扱い）と明示的に除外するのが素直。

### #10 import モジュール Chunk — 保留 2026-07-27（高コスト・低効果）
> ⚠ **この節は 2026-07-27 時点の判断。(a) と (c) は #10-a の実測で部分的に否定された**（上記参照）。
> (b)「`StoreGlobal` op が無い」は正しく、#10-b でそれを新設して解決した。

(a) モジュール本体は定義文（fn/class/gen/import）が支配的で VM コンパイラが全て bail →「本体一括 Chunk」には
**定義文の VM オペコード化**が必要。(b) top-level 変数はグローバルだが VM に **`StoreGlobal` op が無い**（slot ベース）。
名前ベース（`LoadName`）で回避すると**ツリーウォークと同コスト＝速度向上ゼロ**。(c) ホットコードは関数内で既に
VM 化済み、モジュール top-level は一回きりの初期化＋定義が主で実効メリット小。着手時は「グローバル変数実行モード
＋全定義文オペコード化」の大規模拡張が要る点を織り込むこと。

### #12 R0-A 明示フレームスタック — 保留（ただし残る最大の速度余地）
**【保留。ただし 2026-08-11 時点で「残る最大の速度余地」はここ】**
呼び出し機構（bind／フレーム構築・~630ns/call）が支配項として残っており、#2 の R0-A エスケープ解析もこれが前提。
一方**統一目的には寄与しない**（ランタイムの記憶域の変更であって AST 解決注釈の追加ではなく、
ネイティブ経路はインタプリタのフレームを使わない）。速度を追う判断をしたときに着手する。

### #15d 実行経路で「解決済み」判定を活かす — 未着手（#15c から派生）
#15c ではインタプリタ実行経路の **18 サイトを `res: Resolution::Unresolved` に限定**して
旧挙動を保存した。だが本来は解決済みも受けたほうが正しい箇所がある:
- `eval_builtin_ident_call`（[builtins.rs](src/interpreter/eval/builtins.rs)）は
**名前だけで組み込みへ振り分けシャドウ検査が無い**。`res` を見れば
「ローカル/グローバルに解決済み＝組み込みではない」と静的に判定でき、
VM 側の `is_vm_builtin(name) && !slots.contains_key(name)` と同じ健全性が
ツリーウォークにも入る（現在は名前引きの結果に依存している）。
- トレースバック表示名（`call_name`）は解決済み呼び先を `"<anonymous>"` にしている。
解決済みも名前を出したほうが親切だが、**stdout が変わるので
`compare_vm_modes.ps1` の期待値と例題の出力を洗い直す必要がある**。
いずれも**意味論の変更**なので、着手時は「現状のどれが実際に誤っているか」を
先に例題で再現してから直すこと（#15b で使った計測優先の型）。

### #17 FFI 境界の型表現 — 未着手
「スタブが宣言した型」を根拠に動くため、**スタブが型を持たない/持てない箇所には効かない**。その 2 つを本タスクで扱う。
- **(17-a) C/C++ の `void*` に専用型を用意する**。現状 `void*` は Arrow の `int` へ落ちる
（[imports/mod.rs](src/parser/imports/mod.rs) の `ctype_to_tl_str`）。型タグが「int である」以上を語らないため
**静的にも動的にも守れない**（動的検査を足しても静的型と同じことしか言えない）。
不透明ハンドル専用の型を導入して、任意の整数との相互代入を静的に禁じる。
さらに踏み込むなら出所・生存期間の追跡だが、まずは型の分離まで。
- **(17-b) JS スタブの型付け**。`import[js-proc]` は `.ars` スタブがあれば読むが、現状は型を持たないため
境界検査が実質無効。**基本はすべて `Any`** とし、**`.d.ts` があればそれを使って `.ars` を生成**する。
`Any` は検査不能として素通しされるので、`.d.ts` がある分だけ静的にも動的にも締まる。

### #18 順序比較の食い違い — 完了 2026-08-10
型検査の `ordered_comparable` は 4 演算子すべてで `(int,float)` 混在と `(str,str)` を許可していたのに、
実行時 `apply_binop` は `<`/`>` の混在と int/float 同士しか実装しておらず、
**検査は通るのに実行時 TypeError** になっていた。実行時を検査器の仕様に合わせて 8 アーム追加。
例題 [comparison_matrix.ar](examples/basics/comparison_matrix.ar)。

---

## #27 続き（2026-08-15）— fn bail 11→4・`force_gate` 12→8・**リゾルバの実バグを 1 件検出**

`force_gate` が 12/125 の状態から着手。**最初にやったのは実測**で、これが計画の前提を 2 つ覆した。

### 帰属の実測（着手前の想定と違った）

`force_gate`（止めて判定）と `AR_TW_STATS`（数える）を併用して 7 例題の失敗理由を取った結果:

| 想定（計画書） | 実測 |
|---|---|
| 「`force_gate` 12 件中 **7 件が #27**」 | 実際は **#27 が 4 例題・#27-d が 3 例題**（`spider_solitaire` の `draw_game` は bail ではなく `vm_ineligible: closure-capture`） |
| 「識別子 2」 | **4 bail / 3 例題**（最大要因だった） |

⚠ **`force_gate` のメッセージだけでは #27 と #27-d を区別できない**（どちらも
"cannot compile function 'X' to bytecode"）。`AR_TW_STATS` の `vm_bail_fn` / `vm_ineligible` を
突き合わせないと帰属を間違える。

さらに、**bail を潰しても例題が落ち続ける**組み合わせがある: `functions.ar` / `variable.ar` は
bail（`Static` / 可変キャプチャ）と `vm_ineligible`（クロージャ本体）の**両方**を持つので、
#27 側だけ直しても `force_gate` は減らない。⇒ **「bail を減らす」ではなく「例題が落ちる理由を
全部消す」で優先度を決めること**。

### やったこと

| 項目 | 手法 | 効果 |
|---|---|---|
| `ident-unresolved`（4 bail） | 関数本体の未解決 Ident を **`Op::LoadGlobal`** で載せる | `global_resolution.ar` 解消 |
| `attr-compound-target`（1 bail） | レシーバ制限を撤去し、**局所 slot 以外は 2 回評価する経路**を追加 | `class_trait.ar` 解消・**dead code 約 90 行削除** |
| `for-target-shadow`（1 bail） | **リゾルバのバグ修正**＋ループ本体の間だけ専用 slot へ差し替え | `for_range_typing.ar` 解消・**実バグ 1 件修正** |
| `variadic-param`（2 bail） | 可変長を `local::args` という名前で**末尾 slot**に採番 | `variadic.ar` 解消 |

`fn_FAILED` **11→4**・`in_fn`（ツリーウォーク実行文）**111→70**・`force_gate` **12→8**。

### `LoadName` は関数本体で使えないが `LoadGlobal` なら使える（#27-c の制約の抜け道）

#27-c は最上位の未解決 Ident を `Op::LoadName`（＝`get_val`）で載せたが、関数本体では
`frame_floor` が前進していないため **呼び出し元のローカルが見えてしまう**ので使えなかった。

`Op::LoadGlobal` は `scopes[0]` **だけ**を見る（`vm_global_slot_of` = `scopes[0].slot_of`）ので
この問題が起きない。そして関数本体では **`slots` を引いて外れた名前がローカルでないことが
コンパイル時に確定している**（base slot 採番と `collect_nested_decls` が for ターゲット・入れ子
ブロックを含む全宣言を**先に** `slots` へ入れる）。よってツリーウォークの `get_val`
（`scopes[frame_floor..]` 走査 → `scopes[0]`）と結果が一致する — 前段の走査が必ず外れるため。
`NameError` の文言も両者同一。

⇒ **教訓: 「その op が使えない」の理由を辿ると、別の op なら条件を満たすことがある。**
制約は op 単位ではなく**参照するスコープ単位**で見ること。

### レシーバ制限はまた「最適化の前提」を守っていた（#27-c の逆パターン）

#27-c では `object_is_instance` は「2 実装の差（`attr_assign` vs `attr_assign_evaled`）を
隠していた」ため、1 実装に畳んだら制限ごと外せた。今回の `AttrCompoundAssign` の同名の条件は
**理由が違った**: `GetAttrLocal` でレシーバを 1 回しか評価しない融合を使うため、
「再評価しても副作用が無い＝局所 slot」であることが必要だった。

⇒ 制限を外す手は**融合を使わない一般経路を足す**こと。ツリーウォークは
`eval(value)` → `eval(target)`（object 1 回目）→ 二項演算 → `attr_assign`（object 2 回目）と
**object を 2 回評価する**ので、そのまま 2 回積む（添字複合代入と同じ形）。

⚠ **副次効果**: この条件が `object_is_instance` の最後の消費者だった。#26・#27-a で作った
「形＋出自」2 段判定（`is_user_instance_type` / `is_arrow_instance_type` /
`annot_is_arrow_instance`）が**まるごと dead code になり削除**（約 90 行）。
読み書きが `get_attr_val` / `attr_assign_evaled` へ一本化された結果、どの op も
`Value::Instance` を前提にしなくなったため。
⚠ **`Value::Instance` 前提の op を新設するなら判定を復活させること**（型検査の
`NamedInstance` は外部言語スタブのクラス＝実行時 `Value::CsObject` も同じ注釈になる）。

### **リゾルバの実バグ**（for ターゲットが base 名を覆うと外側の値を読む）

`for-target-shadow` を潰そうとして、その手前で**ツリーウォーク自体が間違っていた**ことが判明した。

```
fn in_fn(let n: int) -> int:
    mut i = 100
    mut s = 0
    for i in range(n):
        s += i        # ← ここが毎回 100 を読んでいた
    return s * 1000 + i
```

| 実装 | 結果 |
|---|---|
| Python 参照実装（`impl_python`） | **6100**（s=6・i=100） |
| Rust（修正前） | **400100**（s=400 ＝ 100 を 4 回加算） |
| Rust 最上位に同じコードを書いた場合 | **6**（正しい） |

原因: `rewrite_stmts` の `Stmt::For` は「ループ変数 target は base に無いので本体の読みは
書き換わらない」という前提でコメントまで書かれていたが、**外側に同名の base 宣言がある場合は
その前提が崩れる**。`i` が base 名なので本体の `i` が `Resolution::Local(base slot)` に
書き換えられ、`eval_local_ref` が `scopes[frame_floor]`（base スコープ）を直接引くため、
内側スコープにあるループ変数を飛ばして外側の値を読んでいた。

修正: `resolve_function` で `base` から**入れ子スコープで束縛されうる名前を差し引く**。
グローバル側（R2-b）は既に同じ差し引きを `collect_toplevel_shadowing` で行っていたので、
**同じ関数を共有**して `collect_shadowing_binders` に改名（同じ判断をする 2 実装を作らない）。
slot 番号は `order` の並びで決まるので**採番後に**除外する（番号はずらさない）。

⚠ **教訓**: 「VM が bail する形」は**ツリーウォークが正しいとは限らない**。
bail はモデルの表現力の問題として片付けられがちだが、今回は
**両実装が同じ入力で違う答えを出す形**＝バグの温床でもあった。
`compare_vm_modes.ps1` は off/auto を比べるので、**両方がツリーウォークに落ちるケースは
検知できない**（`--vm=off` と `impl_python` を比べる検査は存在しない）。

VM 側は、ループ本体のコンパイル中だけ `slots[name]` を temp slot へ差し替え、ループ後に戻す
（`for_target_shadows` は「諦める判定」から「差し替える名前の集合」へ役割変更）。
これで flat-slot のままブロックスコープを表現できる。

### 可変長引数（`local::args`）

`bind_args` は可変長を **`local::args` という名前で末尾に 1 つだけ**束縛する（引数が無ければ
`Value::None`）。VM の一般バインド経路は `bindings[i]` → slot i と**位置で**流し込むので、
コンパイラ側も「非可変長を宣言順 → 最後に `local::args`」で採番すれば一致する。
読み（`Expr::LocalVar`）は `slots["local::args"]` の `LoadLocal` で足りる。
⚠ 型注釈（`let ...: int`）は**要素の型**なので `slot_type` には入れない（型特化が誤る）。
⚠ 並びが `bind_args` と一致することが健全性そのものなので、可変長が末尾でなければ bail する。

### 残り（4 bail）と、それが #27-d と同じ土台であること

| 残り | 件数 | 実体 |
|---|---|---|
| `decl-prepass:Static` | 2 | `static mut x = e` は span をキーに `Rc<RefCell<Value>>` を作り `Var::Cell` で束縛する（`exec_static_var`）。**slot は `Value` 直値なのでセルを置けない** |
| `nested-fn-mutable-capture` | 2 | 可変キャプチャは `capture_env` が外側変数を `Var::Cell` へ昇格して**セルを共有**する。同上 |

⇒ **どちらも「slot と並行するセル表＋`LoadCell`/`StoreCell`」を要求する**。これは計画書が
#27-d（クロージャ本体の VM 化）の前提として挙げているものと**同一の土台**。
⇒ **#27 の残り 4 bail は #27-d と分けて着手する意味がない**（片方だけ作ってももう片方が
同じ表現を必要とする）。また、この 2 件が属する `functions.ar` / `variable.ar` は
クロージャ本体（`vm_ineligible`）も抱えているので、**セル表を入れるまで `force_gate` は減らない**。

---

## #27-c 完了（2026-08-16）— `vm_bail_toplevel` **0 件**・`force_gate` 8→4

最上位文の bail を全て解消した。**残る `force_gate` 4 件はすべて #27-d（クロージャ本体）**で、
最上位側の未対応構文は無くなった。

### 5 例題 → 実測すると原因は 4 種だった

`force_gate` は例題ごとに**最初の 1 件**で止まるので、潰すたびに次の原因が出てくる。
1 例題 = 1 原因ではない（`built_in.ar` は 2 つ、`alias.ar` は 2 つ、`try_except.ar` は 2 つ持っていた）。

| 原因 | 手法 | 例題 |
|---|---|---|
| `callee-builtin:create_flat_int_list` | flat リスト組み込み 3 種を**評価済み引数の 1 実装**（`eval_builtin_flat_evaled`）へ集約し、ツリーウォーク側を委譲に | langtons_ant / _profile |
| `decl-no-slot:<ソース名>` | `let x = <グローバル識別子>` に **`Op::StoreLocalFromIdent`**（ソースの可変性を実行時に見る） | try_except |
| `try-except-finally` | **`try/except` を `try/finally` で包む**（Python と同じ等価変形） | try_except |
| `decl-no-slot:<fn 名>` | `collect_nested_decls` に **`Stmt::FnDef` のアームを追加**（ブロック式内の入れ子 `fn` に slot） | alias |
| `callee-expr:Block` | 一般の呼び先式を **[callee, args…] + `Call`** で素直に積む（表示名は `<anonymous>`） | alias |
| `call-arg`（組み込み） | **`Op::CallBuiltinKw`** ＋ `eval_builtin_evaled_named`（`enumerate(xs, start=1)`） | built_in |
| `call-arg`（メソッド） | **`Op::CallMethodKw`**（`f.read_line(backward=True)`） | built_in |

### `let x = <識別子>` は `Resolution` を見てはいけなかった（**潜在バグ 1 件**）

`exec_let` は**ソース式が `Expr::Ident` でありさえすれば** `get_var(src)` の可変性で 3 分岐する
（mut→deep_copy+freeze／let→そのまま／変数でない→Instance だけ copy+freeze）。
ところが VM コンパイラは `Resolution::Local` と `Resolution::Unresolved` の 2 アームしか持たず、
**`Resolution::Global` の識別子が「非識別子式」の枝**（`StoreLocalFreezeInstance`）へ落ちていた。

⇒ 可変グローバルをソースにした `let`（例: `mut g = [1,2]` … 関数内で `let x = g`）で
**コピーとフリーズが漏れる**。#15e の原則（注釈は最適化ヒントであって意味論の根拠ではない）を
破っていた形で、`for-target-shadow` と同じ「off/auto では検知できない」種類のずれ。
例題が踏んでいなかったので実害は出ていなかったが、分岐条件を
「**ident かどうか**（`Resolution` は見ない）／可変性が slot から分かるか」に組み替えて解消した。

判断そのものは `vm_let_value_from_ident` に 1 本化し、`DeclKind::LetFromIdent`（最上位）と
`Op::StoreLocalFromIdent`（slot）が共有する。**違うのは可変性の引き方だけ**:
前者は `get_var`（最上位は `scopes.len()==1` が保証されている）、後者は `scopes[0]` 限定
（#27 の `LoadGlobal` と同じ理由 — VM フレームでは `get_var` が呼び出し元のローカルを覗く）。

### `try/except/finally` は新実装ではなく**入れ子**で足りた

「finally とハンドラの相互作用が複雑」として bail していたが、
**`try/except` を丸ごと `try/finally` の本体として埋め込む**だけで全経路が揃う:

- 本体が正常終了 → 内側 `PopTry` → 外側 `PopTry` → finally
- ハンドラがマッチ → ハンドラ本体 → 外側 `PopTry` → finally
- どのハンドラにもマッチしない → 内側の `Reraise` が**外側の landing pad** へ落ちる → finally → 再送出
- ハンドラ本体が例外を出した → 同上

成立の根拠は **`run` が例外時に `handlers.pop()` してから landing pad へ跳ぶ**こと
（内側ハンドラは既に外れているので、内側で捕まり直すことがない）。
追加したのはハンドラ本体の脱出チェック（`has_escape(.., true, ..)`）だけで、コードは実質 +10 行。

⇒ **「相互作用が複雑」と書かれた bail は、既存の 2 つを組み合わせられないか先に見ること。**

### 採番 walker の漏れがまた出た（`Stmt::FnDef`）

`collect_nested_decls` に `Stmt::FnDef` のアームが無く、**ブロック式の中の `fn` に slot が
振られていなかった**（`compile_stmt` 側は `slot_of` を引くので `decl-no-slot` で bail）。
関数本体**直下**の `fn` は `compile_fn_inner` の prepass が採番していたので、
「入れ子だけ落ちる」形になっていた。#27-c で 2 回目（前回は `Stmt::Block` と `Stmt::LetTuple`）。

⇒ **同じ木を歩く walker が 2 つある限りこの穴は再発する**。新しい文種別を
`compile_stmt` に足したら、必ず `collect_nested_decls` 側も見ること。

### 組み込み／メソッドのキーワード引数

`CallBuiltin` / `CallMethod` は評価済みの**値だけ**を運ぶので、引数名が要る形が bail していた。
`chunk.kw_calls`（`CallKw` 用の副表）をそのまま流用して `CallBuiltinKw` / `CallMethodKw` を追加。

⚠ **組み込みはキーワードの扱いが名前ごとに違う**（`enumerate` は `start` だけ許容／`zip` は
エラー／`len` は名前を無視して位置引数扱い）。一致を確認した名前だけを
`VM_BUILTIN_KW_NAMES`（現在 `enumerate`・`open`）に挙げ、それ以外は従来どおり bail する。
**「キーワードを一般に通す」実装にすると off/auto がずれる**。

⚠ メソッド側は **`CallMethodLocal`（レシーバ frame 直読み融合）を使うかどうかを
引数のコンパイル前に決める**必要がある（融合はレシーバを push しないので、後から
「やっぱり一般形」に切り替えられない）。`has_named_args` を先に見る形にした。

### 残り

`force_gate` 4 件は `spider_solitaire`(draw_game) / `functions`(inner_greet) /
`variable`(make_id_generator) / `langtons_ant_profile`(draw_cells) — **全て
`vm_ineligible: closure-capture`＝#27-d**。`vm_bail_fn` は #27 の 4 件（`static` 2・
可変キャプチャ 2）のままで、これも #27-d と同じセル表を要求する。
⇒ **#3 に残っているのは #27-d ただ 1 つ**。

---

## #27-d 段階 1・2a（2026-08-16）— `force_gate` 4→2・**残りは「可変キャプチャ」1 種だけ**

### 実測が段階分けを決めた

着手前に 4 例題のキャプチャ内訳を取ったところ、**半分は可変キャプチャを持っていなかった**:

| 例題 | `closure_capture` の内訳 | 必要なもの |
|---|---|---|
| spider_solitaire | immutable-only 15 / none 7 | **セル不要** |
| langtons_ant_profile | immutable-only 2 | **セル不要** |
| functions | has-mutable 5 / immutable-only 1 | セル |
| variable | has-mutable 2 | セル |

⇒ 「クロージャ＝フレーム表現の変更が必須」と一括りにせず、**不変キャプチャだけ先に載せる**ことで
フレーム表現を変えずに 2 例題を消せる、と分かった。計画書は #27-d を丸ごと architectural と
書いていたが、**architectural なのは可変キャプチャの部分だけ**だった。

### 段階 1: 不変キャプチャを slot へ束縛

- `compile_fn` に `captures: &[String]` を追加し、**全ての宣言を採番したあと末尾に**キャプチャ slot を採る。
  末尾なのは `Resolution::Local`（パラメータ→本体直下の宣言の並びで焼かれている）を動かさないため。
- `Chunk.captured_slots: Vec<(String, u16)>` を持ち、呼び出し側（`try_fast_bind` と一般バインドの
  両方）が `fn_val.captured_env` から**名前で引いて**書き込む（`captured_env` は `HashMap` なので
  反復順に依存させない）。
- `vm_eligible` を `captured_env.is_empty()` から **「全て `CapturedVar::Immutable`」** へ緩和。

⚠ **値は `clone` するだけでよい**。`CapturedVar::Immutable` は `capture_env` が
**クロージャ生成時に deep_copy 済み**のスナップショットで、呼び出しごとにコピーし直すのは
ツリーウォークの意味論と違う。

⚠ **キャプチャ名が既存 slot とぶつかったら bail する**（`capture-slot-conflict`）。
`capture_env` は「パラメータでも本体の宣言でもない自由変数」だけを捕まえるので本来ぶつからない。
ぶつかる＝`collect_declared_names`（キャプチャ側）と `slots`（コンパイラ側）の木の歩き方がずれた
ということなので、**黙って上書きすると閉包変数が消える**（#27 で計画書が警告していた形）。

結果: `force_gate` 4→3（langtons_ant_profile が解消）・`vm_ineligible` 20→13。

### 段階 2a: `static mut` は**フレームを変えずに**載る

`static` の記憶域は最初から**フレームではなく `Interpreter::static_cells`**
（宣言位置 (file,line,col) をキーにした `Rc<RefCell<Value>>`）だった。
⇒ span をキーに直接読み書きすれば、セル表を作らずに VM 化できる。

```
StaticInit(span, after) ─ セルが既にあれば after へジャンプ（初期化子を評価しない）
<初期化子>
StaticStore(span)       ─ セルを新規作成して値を入れる
after:
```
読み書きは `LoadStatic(span)` / `StoreStatic(span)`。`exec_static_var` の
「セルが無いときだけ初期化子を評価する」分岐がそのままジャンプになる。

⚠ `static` 名は **slot を持たない**。`slots` を引く経路（`slot_of`・`as_local`・`store_target`・
`Expr::Ident`）すべてで**先に `statics` を見る**こと。`slot_of` に落ちる経路（for ターゲット・
`let a,b = t`・`except as`・入れ子 `fn` の格納先）は共有セルを扱えないので `static-as-slot` で bail する。

⚠⚠ **`StaticInit` の `after` は「コード索引」なので `peephole::code_target_mut` に足すこと**。
最初これを忘れており、**テストも例題も通ってしまっていた**（たまたま該当関数に除去対象の
`Jump` が無かっただけ）。計画書の「コード索引を持つ op は飛び先だけではない」（#2a）が
そのまま当てはまる箇所で、**op を足すときの定型チェック項目**として扱うべきだった。

⚠ リゾルバは `static` を含む関数の解決を諦める（`collect_base_decls` が未対応の宣言文で false）ので
本体は全て `Unresolved`。それでも `Resolution::Local` より**前に** `statics` を見る順序にしてある。

結果: `force_gate` 3→2（spider_solitaire が解消）・`in_fn` 70→**50**。

### 残り: 可変キャプチャ 1 種だけ（設計は確定済み）

`vm_bail_fn` は 4 件すべて `nested-fn-mutable-capture`、`vm_ineligible` は 13 件すべて
`closure-mutable-capture`。**外側フレームとクロージャが `Rc<RefCell<Value>>` を共有する**という
1 点に収束した。残る 2 例題:

```
fn make_counter() -> function[]->int:      fn make_id_generator() -> function[]->int:
    mut count = 0                              static mut next_id = 0
    fn inc() -> int:                           fn next() -> int:
        count += 1                                 next_id += 1
        return count                               return next_id
    return inc                                 return next
```

必要なのは 2 つ:
1. **クロージャ側**（両方が必要）: `captured_env` の `Mutable(cell)` を呼び出しごとに
   フレームのセル表へ束縛し、本体の読み書きを `LoadCell`/`StoreCell` にする。
   ⇒ `Chunk.n_cells` ＋ `captured_cells: Vec<(String,u16)>`、`run`/`exec_op` に
   `cells: &mut Vec<Rc<RefCell<Value>>>` を通す。
2. **外側フレーム側**（`make_counter` だけが必要）: 「入れ子 `fn` に可変キャプチャされる
   ローカル」をコンパイル前に洗い出し、slot ではなくセルに置く。`MakeFn` がそのセルを
   `CapturedVar::Mutable` として渡す。
   ※ `make_id_generator` は外側のセルが `static_cells` にあるので、`MakeFn` が
   **static セルを Mutable キャプチャとして渡す**だけで済む（外側フレームの変更は不要）。

⚠ **`freeze` との相互作用に注意**。`make_var_immutable` は「`Cell` 変数（キャプチャ済み）は
freeze できない」規則を持つ。コンパイル時に「セルに置く」と決めた変数は、ツリーウォークでは
まだ `Var::Mutable` の可能性がある（キャプチャは `fn` 定義の実行時に起きる）ので、
**セル化する名前に `freeze` があれば bail する**こと。

---

## #32 async ブロック本体の VM 化（完了 2026-08-16）— **3.77x**・最上位ツリーウォークが定義文だけになった

### 発端: 「bail していないのにツリーウォークで実行されている文」17 件

`vm_bail_toplevel` が 0 になったあとも `toplevel` に 364 件残っていた。347 は定義文（設計上の
対象外・#10-d）だが、**残り 17 件（`BlockReturn` 15・`Raise` 2）は定義文ではなかった**。
出所は `async_demo` / `async_string_share` / `js_proc_async_test` の
**`mng <- async->T:` の本体**で、`run_task` が `eval_block_expr` でツリーウォーク実行していた（#9 の設計）。

### 実測した 3 つの事実

| 観測 | 実測 |
|---|---|
| async 本体の文はどれくらいツリーウォークか | **100%**。本体に直接書いた 30 万反復のループが `CompoundAssign=300000` として全部計上された |
| 本体に直接書く vs 関数へ出す | **3.53x**（0.811s vs 0.230s）。関数へ出せば worker でも VM 化されるため |
| `--vm=force` は本体を検査しているか | **していない**。100% ツリーウォークの本体が `force` で素通りしていた |

3 つ目の原因は `run_task` が **`Interpreter::new()` を作っていた**こと。`VmMode::default()` は
`Auto` なので、**親スレッドの `--vm` は worker へ一切届いていなかった**
（`--vm=off` も効かず、`--vm=force` も検査できない）。

### 実装

- **`vm::compile_async_body(body, annotations, captures)`** を新設。本体は「値を返すブロック式」
  （`block_return v` が結果）なので `compile_fn` ではなく **`compile_block_expr` の上に `Return`**
  を載せる形にした。捕捉環境は #27-d 段階 1 と**同じ `captured_slots` の仕組み**で末尾 slot へ。
- **`run_task` を VM 経路に**。`vm_mode` を `AsyncTask` 経由で worker へ運び、
  `Off` ならコンパイルしない／`Force` でコンパイル失敗なら `VmForceError` を返す（ゲートの穴を塞ぐ）。
  結果の変換（`RAISE_SENTINEL` の取り出し）は `finish_task` に切り出して両経路で共有。

⚠ **worker は型注釈を持てない**（`AstAnnotations` は `Rc` ベースで `Send` でない）ので
空の注釈でコンパイルする＝**型特化 op は乗らない**。それでも 3.77x 出るのは、
特化以前に「ディスパッチと名前引きが消える」効果が大きいから。
**注釈は最適化ヒントであって意味論の根拠ではない**（#15e）ので結果は変わらない。

⚠ **捕捉名と本体の宣言が衝突したら bail する**（`capture-slot-conflict`）。ツリーウォークでは
捕捉値が push されたスコープに居るので**宣言より前は捕捉値が見える**。slot を宣言側に取られると
その読みが未初期化 slot になり、黙って値が変わる（#27-d 段階 1 と同じ判断）。

⚠ VM 経路でも `push_scope` ＋ `declare_var` は残してある。slot に載らない自由名が
`Op::LoadName` に落ちるときの受け皿で、`toplevel_globals` に捕捉名を入れて最上位モード扱いに
している（worker の `frame_floor` は 0 で、覗かれて困る呼び出し元フレームが無いので安全）。

### 効果と、#3 への意味

- **3.77x**（本体に直接ループを書いた形・`--vm=off` 0.796s → `--vm=auto` 0.211s・交互実行 3 回平均）。
- **最上位ツリーウォーク 364 → 347 件＝全て定義文**になった。`module_body` の 20 件も全て定義文。
- ⇒ **#27-d 段階 2b（`in_fn` の 50 件）が終われば、制御フローを持つツリーウォークはゼロになる**。
  #3 の「TLS 4 本・センチネル 2 種の実削除」が現実に可能になるのはこの状態から。

回帰検知は [async_vm_body.ar](examples/async/async_vm_body.ar)（本体に直接書いた for/while/if ＋
捕捉変数の読み ＋ `block_return` ＋ 本体からの関数呼び出しを 1 本で覆う）。

---

## #27-d 段階 2b 完了（2026-08-16）— **`force_gate` 0/123**・ツリーウォークの制御フローが消えた

可変キャプチャ（`Rc<RefCell<Value>>` の共有）を VM に載せ、**#3 の前提を満たした**。

### 到達点（全部ゼロ）

| 指標 | 前 | 後 |
|---|---|---|
| `force_gate` | 2/123 | **0/123** |
| `vm_bail_fn` | 4 | **0** |
| `vm_bail_toplevel` | 0 | **0** |
| `vm_ineligible` | 13 | **0** |
| `in_fn`（関数本体のツリーウォーク実行文） | 50 | **0** |
| `toplevel`（最上位のツリーウォーク） | 348 | **348（全て定義文＝設計上の対象外）** |

⇒ **制御フローを持つツリーウォークは 1 文も残っていない**。#3 の「TLS 4 本・センチネル 2 種の
実削除」に進める状態になった。

### 設計: slot と**並行する**セル表

`Value` は変えない（§7 の方針）ので、フレームに `Vec<Rc<RefCell<Value>>>` を**並行して**持つ。

- `Chunk.n_cells` / `Chunk.captured_cells`（名前 → セル index）
- `run` が `build_cells` でフレーム入口に作る。**可変キャプチャは `captured_env` のセルを
  そのまま入れる**（`Rc` を clone するだけ＝外側と同じセルを指す）。それ以外は新規セル。
  `n_cells == 0`（大多数）は `Vec::new()` で**確保しない**。
- op は `LoadCell` / `StoreCell` / `StoreCellDeepCopy`（`mut x = e` 用）の 3 つ。
- `exec_op` / `run` / `run_stepping` に `cells` を通す（**ホットパスの引数が 1 本増える**）。

セル変数になるのは 3 つ:
1. このクロージャの**可変キャプチャ**（`mut_captures`。呼び出し側が `captured_env` から分類）
2. **入れ子 `fn` に可変キャプチャされる自分のローカル**（`nested_fn_free_names` ∩ 可変ローカル）
3. `static mut`（記憶域は `Interpreter::static_cells`。セル表は使わず `LoadStatic`/`StoreStatic`）

`MakeFn` は `ChunkFnDef.cell_captures`（外側のセル index）と `static_captures`（span）から
`CapturedVar::Mutable(cell)` を組む ＝ ツリーウォークの `capture_env` が
`Var::Mutable` → `Var::Cell` へ昇格するのと同じ効果。

### ⚠⚠ **`static` の slot 採番ずれ**（段階 2a の潜在バグを本番で踏んだ）

段階 2a で `static` 名に **slot を割り当てなかった**。ところがリゾルバの `collect_base_decls` は
`Stmt::Static` にも `push_base` する。⇒ **以降の base slot が全部 1 つずれ**、
`Resolution::Local(k)` が別の変数（または範囲外）を指す。

`make_id_generator`（`static mut next_id` の次に `fn next`）で
**`LoadLocal` の添字 out-of-bounds パニック**として露見した。2a 単体では
`_ui_state` が「static 以外の base 宣言を持たない」形だったので**症状が出ていなかった**。

修正: `static` にも **slot を 1 つ消費させる**（`slots` には入れない＝穴）。
セル化した名前（2b-B）も同じ理由で**穴を残す**。

⇒ **原則: `slots` に入れるかどうかに関わらず、リゾルバが `push_base` する名前は必ず
1 slot 消費する。「採番はリゾルバと同順・同数」が契約**。破ると別の変数を読む。

`Resolution::Local(slot)` が付いた読みをセルへ振り替えるため、`cell_by_slot`（slot → セル index）を
持って `Expr::Ident` と `as_local` の両方で見ている。

### A/B（ホットパスに引数が 1 本増えた）

| 経路 | 結果 |
|---|---|
| `--vm=auto`（全ベンチ 19 本） | **0.960〜1.046x**（大半 0.98〜1.00x） |
| `--vm=off`（悪化幅の大きい 4 本） | **0.980〜1.017x** |

**`--vm=off` でも同じ幅の差が出る**＝計画書の判定基準では **VM 経路とは無関係**（コード配置・
ノイズ床）。#28 で確定した「この規模の変更は ±5% 揺れる／数 % で良し悪しを判断しない」の範囲。

⚠ A/B のために `git stash push -- src/` を使うときは、**先に `src/` をスクラッチパッドへ
コピーしてハッシュで復元を確認する**こと。今回 `stash pop` 後の比較で全ファイルが「異なる」と
出て焦ったが、**差は改行コード（CRLF↔LF）だけ**だった。バイト比較ではなく
**改行を正規化してから比較する**こと。

---

## #29 ゲートの穴を塞ぐ（完了 2026-08-17）— 未判定 5 例題 → **0**・**128 例題すべて完走**

`force_gate` が 0 件になっても、**GUI/対話/長時間ベンチの 5 例題はタイムアウトで判定されて
いなかった**（`spider_solitaire`・`flat_bench`・`cs_form_app`・`langtons_ant`×2）。
「0 件」が「全例題で確かめた」を意味していない状態だったので、そこを埋めた。

### 見つけた実際の穴: **タイムアウト時に stderr を読まずに捨てていた**

```powershell
if (-not $p.WaitForExit($Timeout * 1000)) {
    try { $p.Kill() } catch {}
    $timedout++
    continue          # ← stderr を見ずに次の例題へ
}
```

`VmForceError` が出力済みでもプロセスが生き残っていれば（別スレッドのエラー・出力バッファ）
**取り逃す**。まずここを直した（kill 後も必ず `ReadToEndAsync` を待って走査する）。

### 判定の健全性の前提（確認して記録した）

`VmForceError` は `make_internal_raised_error` の**許可リストに無い**
（`ZeroDivisionError`/`TypeError`… で始まる文字列だけが Arrow 例外に変換される）。
⇒ **`try/except` で捕まらず、必ずプロセスを終わらせる**。
⇒ **タイムアウト時点で生きている＝その時点まで force エラーは起きていない**。
つまりタイムアウトで失われるのは「その先で初めて実行される経路」だけで、判定結果そのものではない。

### 3 手で 5 件 → 0 件

| 手 | 効果 |
|---|---|
| `-Timeout` を 20→**45 秒** | `flat_bench`（約 24 秒）が完走するようになった |
| **kill の前に窓を閉じる**（`CloseMainWindow`） | DxLib/pygame の例題は `ProcessMessage()` が非 0／QUIT イベントで**正常終了する**。kill と違い**終了処理のコードまで走る** |
| **閉じる操作を繰り返す**（`Refresh` してから再送・既定 25 秒） | `cs_form_app` は**ダイアログを順に出す**ので 1 回では終わらない。閉じ続けると最後まで走る |

⚠ **`MainWindowHandle` はキャッシュされる**ので、繰り返すなら毎回 `Refresh()` してから送ること。

結果: **128 完走（うち 4 件はタイムアウト後に窓を閉じて終了）／サンプル判定 0／`VmForceError` 0**。

### 報告の形も変えた

以前は `(timeout: 5)` という数字だけで、読み手には「5 件スキップ」に見えた。今は
**完走／窓を閉じて終了／サンプル判定（kill）** の 3 つに分け、後ろ 2 つは**名前を列挙**する。
サンプル判定が出た場合でも「黙って捨てる」ことはもう無い。

⚠ スクリプトを Python で書き直すときは **raw 文字列を使う**こと。通常の文字列だと
`'target\release\arrow.exe'` の `\r` が CR に、`\a` がベルになって**パスが壊れる**（実際に踏んだ）。
書いたあとに `\x1b` / `\\archived\\` / `\release\` が残っているか grep で確かめる。
⚠ PS5.1 は `$x = if (...) {...} else {...}` を書けない（PS7 の機能）。既定値を入れてから上書きする。

---

## #3 強制バイトコード（2026-08-17）— **フォールバック撤去は完了・TLS 削除は前提が誤っていた**

### 結論

| #3 の構成要素 | 結果 |
|---|---|
| **デュアルモードのフォールバック撤去** | **完了**。`VmMode` を `Off`/`On` の 2 値に畳み、`On` は載せられなければ `VmForceError` で停止する |
| **TLS 4 本・センチネル 2 種の実削除** | **できない**（下記）。**2 つは VM が現に使っており**、残り 4 つは `--vm=off` のためだけに生きている |

### 計画書の前提が 2 点で古かった

**① `GENERATOR_YIELDS` と `RAISE_SENTINEL` は VM 経路が使っている。**
- `GENERATOR_YIELDS`: `run_vm_generator` が `Some(vec)` を張り、`Op::Yield` の実体 `vm_yield_push` が積む（#8 でジェネレータを VM 化したときからこうなっている）。
- `RAISE_SENTINEL`: `vm_raise` / `vm_reraise` が返す文字列そのもの＝**VM の例外伝播チャネル**（V-C）。

「TLS 4 本／センチネル 2 種」という数え方は **#8 と V-C より前の記述**で、そのまま実行できない。

**② 残り 4 つ（`BLOCK_YIELDS` / `LOOP_DEPTH` / `BLOCK_RETURN_EXPECTED_TYPE` / `BREAK_SENTINEL`）は
通常実行では 1 度も通らない**（計測した）。

`AR_TW_STATS` に `tw_control_flow` を追加し、ツリーウォークの制御フロー入口
（`eval_block_expr` / for・while 式 / for・while 文）に記録を入れて全 128 例題を `--vm=on` で実行:

```
=== tw_control_flow (tree-walk control flow entered) ===
  0 件（どの例題でも通っていない）
```

⇒ これらが生きているのは **`--vm=off` のためだけ**。つまり削除するには `--vm=off` を捨てるしかない。

### なぜ `--vm=off` を捨てなかったか

`--vm=off` は本系列で**唯一の差分検出網**で、計画書自身が検証 4 点セットに入れている:
- [compare_vm_modes.ps1](compare_vm_modes.ps1) の off/on byte-identical 検査
- 「退行を疑ったら `--vm=off` でも同じ差が出るかを見る」という判定基準
  （#10-b・#27・#27-d 段階 2b で実際にこれで誤帰属を防いだ）

⇒ **削除は「網を捨てる」判断とセットでしか成立しない**ので、勝手にやらず **#33** として起票した。

### ⚠⚠ フォールバック撤去は「実行パイプラインを通した文脈」に限る

最初 `VmMode::default()` を `On` にしたら、**単体テストが 1 本落ちて REPL も壊れた**。

原因: `On` は「リゾルバ・型注釈・`toplevel_globals` が揃っている」ことを前提に**載らなければ止まる**。
ところが `Interpreter::new()` を直接使う文脈（**REPL・単体テスト・組み込み**）は解決情報を持たないので、
**正しいコードでも `VmForceError` になる**（`fn on_msg` がグローバルへ代入するだけで落ちた）。

修正: **`Default` は `Off` のまま**にして、`On` にするのは
**`run_program`（＝ファイル実行）だけ**にした。§2.3 が「`eval()`/`exec()` はツール用に残置」と
書いているとおり、REPL は元からツリーウォークの文脈である。

⇒ **教訓: 「強制」を全体既定にしてはいけない。前提（解決情報）が揃う経路にだけ効かせる。**
例題スイートだけ見ていると気づけない（例題は全部 `run_program` を通るため）。

### 互換

CLI は `--vm=off|on` になったが、**`auto` / `force` は `on` の別名として受け付ける**
（`compare_vm_modes.ps1` / `ab_bench_vm.ps1` / `force_gate.ps1` がそのまま動く）。

### 検証

build 警告 0 ／ test **706 緑** ／ compare_vm_modes **72 件 byte-identical・差分 0** ／
scan_examples FAIL 0（**`--vm` 無し＝強制バイトコードで実行**）／ compare_debug_modes 5 件一致 ／
force_gate **0 件・128 例題完走** ／ clippy 62（増分 0）／ REPL 手動確認（`7` を出力）。

---

## 実装メモ（プラン記述からの差分・追記）
- **例外は「静的例外テーブル」ではなく実行時ハンドラスタック**: `run` が `Vec<Handler{handler_ip, stack_len}>` を持ち、
  ディスパッチループが `Err` を捕捉してオペランドを巻き戻し landing pad へ跳ぶ。§5.2 の SETUP_TRY/POP_TRY 通りだが
  Chunk に exception_table は持たない。`RAISE_SENTINEL` は interpreter 境界の規約として残置（VM 内制御はジャンプ）。
- **Chunk 実体**: `Chunk { code, consts, names, attr_caches, spans, local_names, n_locals }`。§5.1 の compiler/ サブ分割
  （stmt/expr/control）は行わず単一 `compiler.rs`。`spans` が行テーブル（Raise/Call の位置）を兼ねる。frame.rs も未分離
  （フレームは共有バッファ `vm_stack` の `base..base+n_locals`）。
- **組み込み呼び出し**: print/range/len を評価済み引数で呼ぶ `CallBuiltin` + `eval_builtin_evaled`（§5.2 の CALL_NATIVE とは別に純粋組み込みを VM 化）。
- **Chunk キャッシュのキー**: `Rc::as_ptr(fn_val)` は**テンプレート実体化の一時 fn_val でアドレス再利用**して古い Chunk を誤用する潜在バグがあった。
  `(Weak<FnValue>, Chunk)` にして `upgrade()` 失敗＝別関数を検出し再コンパイル（リークなし）。
- **V-E メソッドトレースバック**: ツリーウォーク（`eval_method_call`）がメソッド呼び出しの call_span を渡さず degraded なので、
  VM も `call_span=None` に合わせて byte-identical を優先（関数呼び出しは span を渡す）。
- **デバッガ REPL**: §2.3/§3.4-C の「名前引きエスケープハッチ」を `LoadName`/`DeclareName` op ＋ `compile_debug`（`debug_mode`）で実装。
  停止スコープの生変数を名前で参照。メソッド/添字/制御フローはツリーウォークへフォールバック。
- **強制バイトコード未達**: デュアルモード継続中のため、§1.4 のスレッドローカル4本＋センチネル2種はツリーウォークがまだ使用（実削除は D2 時）。

---


---

## #33 の判断材料 — 計測して前提が崩れた（2026-08-17）／ #34・#35・#36 の昇格

#33 は「`--vm=off` を捨てるか」を決めないと成立しないタスクだったので、判断材料を実測した。
結論は **「捨てられない。ただし捨てる方向は正しく、前提タスクを 3 本立てれば到達できる」**。

### 崩れた前提 1: 「4 つの TLS は `--vm=off` のためだけに生きている」→ 誤り

`VmMode::Default` は `Off`（#3 で意図的にそうした）。`set_vm_mode(On)` を呼ぶのは
[main.rs:371](src/main.rs#L371) と [async_mgr.rs:278](src/interpreter/async_mgr.rs#L278) の 2 箇所だけ。
実際の `Interpreter::new()` 呼び出しは **17 箇所**で、内訳は:

| 消費者 | 箇所 | モード |
|---|---|---|
| `run_program` | main.rs:370 | `On`（CLI 指定） |
| async worker | async_mgr.rs:277 | 親から継承（#32） |
| **REPL** | [repl.rs:30](src/repl.rs#L30) | **`Off`** |
| **単体テスト** | tests/mod.rs 6・callables.rs 3・file_io.rs 3・events_external.rs 1・iterator.rs 1 | **`Off`** |

⇒ CLI フラグを消しても REPL と 706 テストがツリーウォーク制御フローを踏み続ける。
**#33 の前提は「`--vm=off` 廃止の判断」ではなく「入口の移行（→ #36）」だった。**

### 崩れた前提 2: 「`force_gate` 0 件＝VM は全部載せられる」→ 誤り

release バイナリ（HEAD・未改変）で確認した **off/on 差**:

| 入力 | `--vm=off` | `--vm=on`（既定） |
|---|---|---|
| `for` 内の `block ->int:` から `break` | `5` | `VmForceError: cannot compile top-level statement 'For'` |
| `for` 内の `if ->int:` から `break` | `5` | `VmForceError` |
| `while ->list[int]:` 内の `if ->int:` から `break` | `[0, 1, 2, 3]` | `VmForceError` |
| 同じ形を**関数本体**へ | `5` | `VmForceError: cannot compile function 'g'` |
| `block ->int: block_return "hello"` | `TypeError: block_return value has type 'str', but 'int' was expected` | **`hello`（素通り）** |

`break` の貫通は `.claude/rules/language-differences.md` に明記された言語機能。
**例題が 1 本も無い**（`break` を含む 34 箇所を確認したが制御フロー式を貫通する形は 0）ため、
`force_gate` にも `compare_vm_modes` にも映っていなかった。⇒ **#34** と **#35** として昇格。

⚠ 一般化: **例題が無い言語機能はゲートに映らない**。`force_gate` は
「128 例題が通るか」の検査であって「言語全体が載るか」の検査ではない。

### 移行コストの実測（#36 の見積もり根拠）

テストヘルパー `run`/`run_get`/`run_exc` に `set_vm_mode(On)` ＋ `set_toplevel_globals(...)` を足して
`cargo test`（実験後に完全復元）:

```
test result: FAILED. 676 passed; 30 failed
```

| 分類 | 件数 | 実体 |
|---|---|---|
| 注釈欠落（実験の副作用。ヘルパーで `check_and_annotate` を呼べば消える） | 19 | `mustbe::*` 13／`unpacking::test_cast_*` 5／`test_redeclaration_in_inner_scope` 1。全て `VmForceError: ... 'Let'` |
| **VM が載せられない**（#34 の実体） | **5** | `break` の制御フロー式貫通 |
| **VM が検査を持たない**（#35 の実体） | **6** | `block_return` の型検査 5／`loop_yield` の位置検査 1 |

⇒ **#36 の実質作業は「入口 4 箇所（REPL ＋ ヘルパー 3 本）で注釈を供給する」だけ**で、
残る 11 件は #34/#35 が閉じれば自動的に緑になる。

### 捨てた場合に失うものの実測

| 失うもの | 代替 |
|---|---|
| [compare_vm_modes.ps1](compare_vm_modes.ps1) の 72 例題 byte-identical 網（**実バグ 4 件**を検出: `JsProcFn` 欠落 #22-a／`<anonymous>` トレースバック #15d-2／`event_cs_handler.ar` の `CsObject` 誤ディスパッチ #10-b′・#27-a） | **#31 が唯一の候補**（下記） |
| [ab_bench_vm.ps1](ab_bench_vm.ps1)（退行が VM 経路由来かの切り分け）と「`--vm=off` でも同じ差が出るか」という判断基準 | 代替なし |

**#31 の実現可能性（`compare_vm_modes` と同一の 72 例題で実測）**:

```
stdout agrees with rust    : 39
stdout disagrees           : 30   （うち _error 例題 8）
impl_python internal crash :  3
```

⇒ **今の impl_python は代替にならない**（33/72 でオラクルとして使えない。`collection.ar` は
rust 125 行に対し py 4 行、`built_in.ar` は 127 行に対し 1 行）。**39 本に絞れば成立する**。
また #31 が埋めるのは「両モードともツリーウォークに落ちる形」という別の穴なので、
**#33 の代替というより補完**。#33 の前に #31 を済ませておくのが望ましい。

### 削除できるコード量の実測（#33 の見返り）

| 対象 | 行数 |
|---|---|
| [eval/control_expr.rs](src/interpreter/eval/control_expr.rs) 全体 | 333 |
| [exec/control_flow.rs](src/interpreter/exec/control_flow.rs) | 182（223 − `make_for_iterator` 41。**VM の `GetIter` が使用中**なので残る） |
| [eval/core.rs](src/interpreter/eval/core.rs) 制御フロー式 5 アーム ＋ `eval_match_expr` | 54 |
| [exec/dispatch.rs](src/interpreter/exec/dispatch.rs) の制御フロー文・信号アーム | 34 |
| [exec/vars.rs](src/interpreter/exec/vars.rs) `exec_loop_yield` | 33 |
| [functions/execution.rs](src/interpreter/functions/execution.rs) `LOOP_DEPTH` 退避×2 ＋ `BREAK_SENTINEL` 検査×2 | 24 |
| [interpreter.rs](src/interpreter.rs) の TLS 3 本 ＋ `BREAK_SENTINEL` 宣言 | 17 |
| [async_mgr.rs](src/interpreter/async_mgr.rs) のツリーウォーク経路 | 8 |
| `ExecResult` の 4 バリアント（`Break` 9／`Continue` 8／`BlockReturn` 12／`BlockYield` 4 箇所） | ~15 |
| **合計** | **≈ 700 行（src 65,012 行の 1.1%）** |

**速度効果はゼロ**（`exec_op` にも VM のホットパスにも触れない）。見返りは構造の単純化のみ。

---

## #34 完了（2026-08-17）— 制御フロー式を貫通する `break`/`continue` の VM コンパイル

`--vm=on`（既定）で `VmForceError` になっていた形を全部載せた。ついでに**ツリーウォークの
`continue` バグ 2 件**と、**エラー報告の off/on 食い違い**（HEAD からの既存ギャップ）を解消した。

### 何が bail していたか（計測）

原因は 1 箇所だけだった。`block_body_bails(stmts, is_loop_expr, loop_depth)` が
「本体内の while/for に囲まれていない `break`/`continue`」を**無条件に非対応**として弾いていた。
`LoopCtx` の doc は「絶対ジャンプなので貫通は自然に成立する」と書いてあり、跳ぶ機構自体は
最初から揃っていた。足りなかったのは**オペランドスタックの平衡**だけ。

### 本当の問題は「跳ぶ時点で積まれている値」だった

ブロック式は値を全部 temp **slot** に置くので、本体の文はブロック式の入口深さで走る。
`let s = 1 + block ->int: … break …` は `1` を積んだまま跳ぶので、跳び先（ループ出口）と
深さが合わない。⇒ **跳ぶ前にその数だけ `Op::Pop` する**。

深さの求め方で 3 案を比較し、最も安い案を採った:

| 案 | 内容 | 判断 |
|---|---|---|
| 全 op のスタック効果表 | `emit()` で深さを追う | **却下**（~120 variant の表を書く＝取り違えると黙って壊れる） |
| 実行時マーク | ループ入口で深さを記録し `break` で truncate（`SetupTry` と同型） | **却下**（`exec_op` の引数が増える。`#[inline(always)]` の signature を触るのは risky） |
| **末尾位置の相対深さ**（採用） | ブロック式は**常に式の末尾**にしか置けないので、親が「自分より左に積んだ数」を渡すだけでよい | **採用**（伝播は `BinOp` 左右と `UnaryOp` の 3 箇所・**新オペコード 0**） |

「どの式位置にブロック式を置けるか」は**構文で確定する**ので実測した:

| 位置 | 可否 |
|---|---|
| `1 + block …` / `block … + 1` / `1 < block …` / `True and block …` / `-block …` / `1 + 2 * block …` | **可** |
| `print(block …)` / `[1, block …]` / `xs[block …]` | **ParseError**（カッコの中には置けない） |

⇒ カッコの中を追う必要が無い。`BinOp` の左右と `UnaryOp` にだけ深さを伝えれば足りる。

### 設計（fail-safe に倒す）

- `Compiler.stmt_base: Option<u16>` … 現在の**文境界**の深さ（最内ループ入口からの相対）。
- `Compiler.pending: Option<u16>` … これからコンパイルする式の開始深さ。
  `compile_expr` が入口で `take()` するので、**直前に設定した親だけ**が伝えられる。
- ループ（文・式の 4 箇所）は本体の `stmt_base` を `Some(0)` に、ブロック式は入口深さに差し替える。
- `Stmt::Break`/`Continue` は `stmt_base` 分の `Op::Pop` を出してから跳ぶ。

⚠ **`None`（不明）なら bail**。伝播を書き漏らした式の形は「壊れる」ではなく「載らない」で止まる。

⇒ 既存コードは `stmt_base = Some(0)` なので **`Pop` は 1 つも増えず、既存 Chunk はバイト単位で不変**。

**伝播漏れは「文の側」にもあった**（実測で発見）。`compile_stmt` が入口で 1 回だけ渡す方式なので、
**値より先に別の式をコンパイルする文**では漏れる。8 つの文形を総当たりして 1 件見つかった:

| 文の形 | 結果 |
|---|---|
| `a = …` / `a += …` / `xs[0] = …` / `xs[0] += …` / `obj.x += …` | ○（値を先にコンパイルする） |
| **`obj.x = …`** | **✗**（`obj` を先に積む ⇒ 右辺は深さ +1）→ `stmt_base + 1` を渡して修正 |
| `print(… block …)` / `let p, q = (…, block …)` | 構文上置けない（ParseError） |

⇒ **fail-safe に倒してあったので症状は `VmForceError`**（誤答ではない）。設計判断が効いた形。

### 2 つ目の walker を消した

判定を `Stmt::Break` のコンパイル時（`loops` と `stmt_base` を見る）へ一本化し、
`block_body_bails` から `break`/`continue` の判定と引数 2 本を削除した（`Stmt::Return` だけを見る）。
⇒ #27-c の教訓「**同じ木を歩く walker が 2 つあるとずれる**」を作らずに済んだ。副産物として
`block:` **文**の中の `break` も貫通するようになった（以前は本体ごと bail していた）。

### 🐛 ツリーウォークの `continue` バグ 2 件（**VM の方が正しかった**）

VM を載せてから off/on/`impl_python` を突き合わせて発覚した。

| 形 | `--vm=off`（旧） | `--vm=on`（新） | `impl_python` |
|---|---|---|---|
| `100 + block ->int: … continue …` | `SyntaxError: 'continue' inside block expression …` | **312** | **312** |
| `1 + if c ->int: continue else: …` | `TypeError: … Add: int and NoneType` | **12** | **12** |

`eval_block_expr` は `continue` を SyntaxError にし、`eval_capture_block_return` は
**アームが無くて `Ok(other)` へ落ち、黙って握り潰して `None` を返して**いた（後者の方が悪質＝
誤った値が出る）。`break` には `BREAK_SENTINEL` があるのに `continue` には無かった。

⇒ **基準は参照実装**（計画書の落とし穴どおり）。`CONTINUE_SENTINEL` を追加し、`break` と同じ
経路（ブロック式 → ループ本体 → 関数境界）で外側ループへ届くようにした。これで
off/on/`impl_python` の 3 実装が一致する。⚠ #33 で消す対象がセンチネル 1 本増えた。

### 🐛 **自分で入れた実バグ**: `try` のハンドラが残る

`break` を通せるようにした直後、`try` との相互作用で踏んだ。**オペランドスタックと同じ問題が
ハンドラスタックにもあった**。

```
fn f() -> int:
    for i in range(5):
        try:
            let _ = block ->int:
                if i == 2:
                    break          # ← PopTry を通らずにループ外へ跳ぶ
            ...
        except ValueError:
            print("WRONG")
    raise ValueError("must escape")  # ← 残ったハンドラがこれを横取りした
```

| | 結果 |
|---|---|
| `--vm=off` / `impl_python` | `OK: caught by the outer try` |
| `--vm=on`（修正前） | **`WRONG: loop handler fired`** ＋ その後 |

原因は **`has_escape` が文しか歩かない**こと。`Stmt::Break` なら弾けるが、
`Expr::Block` の中の `break` は `Stmt::Let` の下に隠れて見えない。
以前は `block_body_bails` が本体ごと弾いていたので露出していなかった。

⇒ `Compiler.try_depth`（最内ループ以降に開いた `SetupTry` の数）を持ち、
跳ぶ前に `Op::PopTry` をその数だけ出す。`finally` は `PopTry` では表せない
（全出口で走る必要がある）ので `finally_guard` を別に持ち、**1 以上なら bail**。
両方ともループ入口で 0 に退避する（外側の try を巻き込まないため）。

⚠ **検知には「跳んだ後に別の例外を投げる」例題が要る**。跳ぶだけの例題（`u_try_break`）は
残ったハンドラに触れないので**素通りした**。例題のケース 12 はこの形にしてある。

**副産物**: `PopTry` を出せるようになったので `has_escape` に `include_break` を足し、
`try/except`（finally なし）では `break`/`continue` を数えないようにした。
⇒ `for: try: … break … except:` という**素の形も VM に載るようになった**（HEAD では bail）。

**残ったギャップ → #37**: `try/finally` を跨ぐ `break` は依然 bail（`--vm=off` と `impl_python` は
正しく finally を走らせる）。HEAD からの既存ギャップで、跳ぶ経路にも finally 本体を出す
（複製 or サブルーチン化）実装が要る。#34 は `finally_guard` で「通さない」ことを明示しただけ。

### 🐛 エラー報告の off/on 食い違い（HEAD からの既存ギャップ）

囲むループの無い `break` は、ツリーウォークが `SyntaxError` を出すのに対し VM は
**コンパイルを諦めていた**ので `--vm=on` が `VmForceError` になっていた（`for` も
ブロック式も無い素の `break` でも同じ＝#34 とは独立の既存バグ）。

⇒ **必ず失敗すると分かっている文は bail しない**。`Op::Fail(name_idx)` を 1 つ足し、
ツリーウォークと**一字一句同じ**メッセージで落とす。発行元は「囲むループの無い
`break`/`continue`」だけなので**既存 Chunk は 1 命令も変わらない**。飛び先索引を持たないので
`peephole::code_target_mut` への登録は不要（`Op` のサイズも `op_size_is_pinned` で据え置き）。

### A/B 実測（新オペコード 1 個ぶんの摂動）

HEAD の `src/` をスクラッチパッドへ退避してビルドした `head.exe` と交互実行（min of 7）。

| ベンチ | `--vm=force`（VM 経路） | `--vm=off`（ツリーウォーク） |
|---|---|---|
| `bench_field_access.ar` | **0.961x** | 1.011x |
| `bench_method_call.ar` | 1.033x（別の回では 0.945x ＝**振れ**） | 1.001x |
| `bench_control_flow.ar` | 0.994x | 1.024x |
| 全体（`ab_bench.ps1`・9 本） | 0.945〜1.009x | — |

⇒ **命令列は 1 命令も変わっていない**（既存コードは `stmt_base = Some(0)` なので `Pop` は増えず、
`Op::Fail` は既存 Chunk に出ない）。`bench_method_call` が 0.945x↔1.033x で振れることが示すとおり
これは**コード配置の揺れ**で、#28 が記録した「op を足す規模の摂動は 1 命令も実行しなくても
0.88〜0.94x 動かす」の範囲内。判断基準どおり `--vm=off` 側には差が出ていない。

⚠ **`ab_bench.ps1` / `ab_bench_vm.ps1` は `ReadToEnd()` を逐次に呼ぶのでデッドロックすることがある**
（1 時間ブロックした。CPU 時間が伸びないので気づける）。`scan_examples.ps1` は非同期読みで回避済み。
今回はパイプを使わない計測（出力を `DEVNULL` へ）に切り替えた。

### 検証

`cargo build` 警告 0 ／ `cargo test` **717 緑**（+11）／
`compare_vm_modes.ps1` **74 identical / 0 differing**（例題 2 本増）／
`scan_examples.ps1` **FAIL 0** ／ `force_gate.ps1` **0 件・130 例題完走** ／
`cargo clippy` **触ったファイルの警告 0**（増分 0）。

新設例題 [control_flow_expr_escape.ar](examples/basics/control_flow_expr_escape.ar)（11 ケース）と
[control_flow_expr_escape_error.ar](examples/basics/control_flow_expr_escape_error.ar) は
**3 実装（off / on / `impl_python`）で byte-identical**。単体テスト 8 件を追加。

⚠ **この例題を削ると検知力を失う**。ここは長らく例題が 0 本で、`force_gate` にも
`compare_vm_modes` にも映らなかった領域。

---

## #35 完了（2026-08-17）— `block_return`/`loop_yield` の実行時検査を VM へ

`--vm=off` にしか無かった 2 つの実行時検査を VM に載せた。⚠ 実装より **「ツリーウォークが
どの注釈を見ているか」を正確に写すこと**が本体で、そこで**新たなバグを 1 件**踏んだ。

### 何が食い違っていた（計測・12 形）

| 形 | `--vm=off` | `--vm=on`（旧） |
|---|---|---|
| `block ->int: block_return "hello"` | `TypeError: block_return value has type 'str', but 'int' was expected` | **`hello`（素通り）** |
| `if True ->str: block_return 42` / `match … ->str:` | 同上（型が逆） | **素通り** |
| `for i in range(3) ->list[int]: loop_yield "s"` | `TypeError: loop_yield value has type 'str', but element type 'int' …` | **`['s','s','s']`** |
| 関数内の `loop_yield`（ループ式の外） | `SyntaxError: 'loop_yield' can only be used …` | **`VmForceError`** |

### 設計

- `BlockCtx` に `return_type: Option<u32>`（名前プール index）を持たせ、5 つのブロック式
  コンパイラが AST の `return_type` を渡す。
- `Stmt::BlockReturn` / `Stmt::LoopYield` は注釈があれば **`Op::CheckBlockReturn`/`CheckLoopYield`**
  を出す（**スタックトップを消費しない**。直後の `StoreLocal`/`ListAppendLocal` が使う）。
- 判定は **`check_block_return_type` / `check_loop_yield_type` の 1 実装へ委譲**。後者は
  `exec_loop_yield` から切り出して共有した（`extract_list_elem_type` ごと `ops/typecheck.rs` へ移動）。
  ⇒ #22 系列の「同じ判断をする 2 実装は片方を委譲にして畳む」。メッセージのずれが原理的に起きない。
- ループ式の外の `loop_yield` は **bail せず `Op::Fail`**（#34 で足した op を再利用）。

新オペコードは 2 個。注釈を持つブロック式にしか出ないので、注釈の無いコードの Chunk は不変。

### ⚠ ツリーウォークが見ている注釈は「最内の**式**」

`BLOCK_RETURN_EXPECTED_TYPE` は **`eval/core.rs` の 5 つの式アームだけ**が push する。
ここから 2 つの非自明な帰結が出て、両方とも実測で確認した:

| 形 | 挙動 | VM 側の対応 |
|---|---|---|
| `block ->int:` の中の **`block:` 文**の `block_return "bad"` | **外側の `->int` で検査**されて TypeError | `Stmt::Block` は外側の `return_type` を**継承**する |
| `for … ->list[int]:` の中の **`if … ->int:`** の `loop_yield "x"` | 最内注釈が `int` ＝ `list[T]` でないので**検査されない** | `block_ctxs.last()` を見る（＝同じ） |

### 🐛 `block:` **文**が `loop_yield` を吸い込んでいた

`Stmt::Block` は `compile_block_expr` を流用しており、**式と同じく蓄積先（`yield_slot`）を
確保していた**。ところがツリーウォークの `exec_block_stmt` は `BLOCK_YIELDS` を push しないので、
文の中の `loop_yield` は**外側の for/while 式へ届く**。

```
let r = for i in range(3) ->list[int]:
    block:
        loop_yield i
```
| | 結果 |
|---|---|
| `--vm=off` / `impl_python` | `[0, 1, 2]` |
| `--vm=on`（修正前） | **`None`**（蓄積が文へ吸い込まれて捨てられた） |

⇒ `compile_block_expr` に `owns_yields` を足し、**式は true・文は false**（透過）にした。
蓄積先が無いときの正常フォールスルー値は `Op::Nil`。
これで「ループ式の外の `block:` 文の `loop_yield` は SyntaxError」も一致する。

### 🔍 副産物: #34/#35 が閉じたことの直接確認と、新たなギャップ #39

テストヘルパーを `VmMode::On` に強制する実験（#33 の判断材料で使ったもの）を再実行:

| | 失敗数 |
|---|---|
| #34/#35 着手前 | **30**（注釈欠落 19 ＋ #34 の 5 ＋ #35 の 6） |
| #35 完了後 | **21**（注釈欠落 19 ＋ グローバル代入 1 ＋ #39 由来 1） |

⇒ **#34/#35 が押さえていた 11 件は全て解消**。残る 19 件は #36 の配線で消える見込み。

残り 2 件から **#39** が出た: `mut g = 0` ＋ `fn bump(): g += 1` が `VmForceError`
（`--vm=off` と `impl_python` は動く）。⚠ **HEAD のバイナリでも同じ**＝既存ギャップで #35 とは独立。

### 検証

`cargo build` 警告 0 ／ `cargo test` **722 緑**（+5）／
`compare_vm_modes.ps1` **76 identical / 0 differing**（例題 2 本増）／
`scan_examples.ps1` **FAIL 0** ／ `force_gate.ps1` **0 件・132 例題完走** ／
`cargo clippy` **触ったファイルの警告 0**（⚠ #34 で入れた `mem::replace(&mut self.stmt_base, Some(0))`
が 4 件の増分になっていたので `Option::replace` へ直した）。

新設例題 [block_return_typecheck.ar](examples/basics/block_return_typecheck.ar)（8 ケース）と
[block_return_typecheck_error.ar](examples/basics/block_return_typecheck_error.ar)。単体テスト 5 件を追加。

---

## #37 完了（2026-08-17）— `finally` を跨ぐ脱出の VM コンパイル

`return`/`break`/`continue`/`block_return` が `try/finally` を跨ぐ形を VM に載せた。
**新オペコードは 0**。#34 で入れた `try_depth`/`finally_guard` を一般化しただけで済んだ。

### 計測: 9 形中 8 形が bail していた

| 形 | `--vm=off` / `impl_python` | `--vm=on`（旧） |
|---|---|---|
| `try: return 1 finally: …` | `fin` → `1` | **`VmForceError`** |
| `except` 併用・入れ子 finally | 同上（内側 finally が先） | **`VmForceError`** |
| `try:` から `break` / `continue` | finally を走らせて抜ける | **`VmForceError`** |
| `try: block_return 7 finally: …` | `fin` → `7` | **`VmForceError`** |
| **`try: loop_yield i finally: …`** | 蓄積して継続 | **`VmForceError`**（← 誤検知） |
| 脱出の無い `try/finally` | 動く | 動く |

### 設計: `try_depth` → `try_stack`

`try_depth: usize` ＋ `finally_guard: usize` を **`try_stack: Vec<Option<Vec<Stmt>>>`**
（開いている `SetupTry` ごとに `finally` 本体・`try/except` は `None`）へ置換した。
`Stmt: Clone` なので本体は `to_vec()` で持てる（コンパイル時だけのコストで finally は小さい）。

`emit_unwind_tries(keep, pop_except)` が**内側から**巻き戻す:

| 脱出 | `keep`（跨がない外側の数） | `pop_except` |
|---|---|---|
| `break` / `continue` | `LoopCtx.try_len` | 真 |
| `block_return` | `BlockCtx.try_len` | 真 |
| `return` | **0**（全部） | **偽**（`run` から即復帰しハンドラごと捨てられる） |

⇒ `return` に偽を渡すので、**finally を持たないコードの Chunk は 1 命令も変わらない**。
バリアを各コンテキストに持たせたので、#34 で入れていた「ループ入口で 0 に退避」は不要になった。

### 消せた bail

- `compile_try_except` の `has_escape` 2 件 … `PopTry` を出せるので**丸ごと不要**。
  ⇒ `for: try: … break … except:` や `try: block_return … except:` も VM に載る。
- `compile_try_finally` の `has_escape` 3 件 … **`fin` 本体の脱出だけ**に縮小（#40 へ）。
- `has_escape` から **`LoopYield` を除去**。⚠ `loop_yield` は蓄積して**そのまま先へ進む**ので
  そもそも脱出ではない。誤検知で `try: loop_yield i` が丸ごと bail していた。

### 残り → #40

**`finally` 本体そのものからの脱出**（`finally: break` / `finally: return`）は `in_finally` で
bail する。finally は正常路・例外路・**各脱出路**に複製されており、**コピーごとにスタックの形が違う**
（例外路は `[exc]` が載り、`return` 路は戻り値が載る）ので、一律の巻き戻しが書けない。

### 検証

`cargo build` 警告 0 ／ `cargo test` **726 緑**（+4）／
`compare_vm_modes.ps1` **77 identical / 0 differing** ／ `scan_examples.ps1` **FAIL 0** ／
`force_gate.ps1` **0 件・133 例題完走** ／ `cargo clippy` 触ったファイルの警告 0。
16 形（基本 9 ＋ 敵対的 7: 入れ子 finally の順序・ループ内 return・例外路・
オペランドが積まれた状態での break・`try/except` を跨ぐ `block_return`・脱出後の `raise`）が
**3 実装（off / on / `impl_python`）で一致**。

新設例題 [try_finally_escape.ar](examples/exceptions/try_finally_escape.ar)（10 ケース）。
⚠ **`_error` 例題は作らない**。#37 は新しいエラーパターンを足しておらず、`finally: break` を
例題にすると **off/on が割れて `compare_vm_modes` が落ちる**（それは #40 の対象）。

---

## #40 完了（2026-08-17）— `finally` 本体そのものからの脱出

`finally:` の**中**の `break`/`continue`/`return`/`block_return` を VM に載せた。
**新オペコード 0**。#37 の `compile_finally_copy` に引数を 1 本足しただけで済んだ。

### 意味論は Python と同じ（`--vm=off` と `impl_python` が 8 形すべて一致）

| 形 | 結果 |
|---|---|
| `try: return 100 finally: break` | **return は破棄**されループを抜ける |
| `try: raise … finally: return 7` | **例外は破棄**され 7 を返す |
| `try: raise … finally: break` | **例外は破棄**されループを抜ける（後続も生存） |
| `try: block_return 1 finally: block_return 2` | 2 |
| 内側 finally が break | **外側 finally は依然走る** |

### 鍵は「複製ごとにスタックの形が違う」こと

`finally` 本体は **正常路・例外路・各脱出路**に複製される。それぞれ土台が違う:

| 複製 | 下に積まれているもの | `extra` |
|---|---|---|
| 正常路 | なし | 0 |
| 例外 landing pad | `[exc]` | **1** |
| `return` 経路 | 戻り値 | **1** |
| `break`/`continue` 経路 | なし（`Pop` は finally の後に出す） | 0 |
| `block_return` 経路 | なし（値は `result_slot` へ退避済み） | 0 |

⇒ `compile_finally_copy(fin, extra)` が **`stmt_base` を `extra` だけ持ち上げる**。
これだけで、複製の中の `break` が跳ぶときに `emit_unwind_to_loop` がその値まで捨てる
＝ **「保留中の動作を破棄する」が自動的に成立する**。跳んだ結果 `Pop`/`Reraise`/`Return` を
飛ばすので、例外・戻り値の破棄も自然に出る。

`block_return` だけは `stmt_base` を捨てない（ブロック式入口の深さへ跳ぶ）ので、
`BlockCtx.entry_depth` を足して**差分だけ `Pop`** する。

### `has_escape` walker を全廃した

#37 で `try/except` の門を、#40 で `try/finally` の門を外した結果、**`has_escape` の呼び出しが
0 件**になったので関数ごと削除（41 行）。⇒ 「同じ木を歩く walker」が 1 つ減った。
判断は `try_stack` / `stmt_base` という**実行時の形に対応した状態**へ完全に一本化された。

### 複製の増殖対策

各 finally が脱出を含むと複製が入れ子に増える。`MAX_FINALLY_NEST = 4` で頭打ちにし、
超えたら bail（ツリーウォークへ）。⚠ 実測では **`try` を 6 段ネストしても発火しない**
（`in_finally` が増えるのは「finally の中の脱出」だけ）。3 段すべての finally が
`break` を持つ病的な形でも正しく `[0, 10, 20]` を出した。

### 検証

`cargo build` 警告 0 ／ `cargo test` **730 緑**（+4）／
`compare_vm_modes.ps1` **78 identical / 0 differing** ／ `scan_examples.ps1` **FAIL 0** ／
`force_gate.ps1` **0 件・134 例題完走** ／ `cargo clippy` 触ったファイルの警告 0。
13 形（基本 8 ＋ 敵対的 5: 例外路の `block_return`・ブロック式内ループからの `return`・
3 段ネスト・`finally` からの `raise`・例外を捨てる `continue`）が **3 実装で一致**。
**過去の全バッテリ 107 形を再実行して不一致 1 件**（#39 の既知ギャップのみ）。

新設例題 [finally_body_escape.ar](examples/exceptions/finally_body_escape.ar)（10 ケース）。
⚠ `_error` 例題は非該当（新しいエラーパターンを足していない）。

---

## #39 完了（2026-08-17）— 関数本体からのグローバル代入

**これで `--vm=off` でしか走らない言語機能は 0 になった**（過去の全バッテリ **127 形**を
`--vm=off` / `--vm=on` で突き合わせて不一致 0）。

### 対象は 2 形だけだった（計測）

8 形を測ったところ、食い違うのは**名前への代入**だけ:

| 形 | 結果 |
|---|---|
| `g = 42` / `g += 1`（関数内） | **`VmForceError`** ← 対象 |
| `g` の読み／`b.v = 5`／`xs.append(3)`／`xs[0] = 9`（関数内） | 元から動く |

属性・添字・メソッド経由の変更は `StoreGlobal` を必要としないので元から載っていた。

### 🐛 委譲先が間違っていた（潜在的な健全性の穴）

`store_target` の `toplevel_globals` 門を外すだけ…**ではなかった**。
`Op::StoreGlobal` のミス経路は `vm_assign_global` → **`assign_var` へ委譲**しており、
`assign_var` は `scopes[frame_floor..]`（ローカル）を**先に**走査する。

⚠ **VM 関数は `scopes` を一切押さない**（フラットな `vm_stack` で動く。しかも
`try_fast_bind` はフレーム構築より**前**に VM を回す）。つまりその走査に映るのは
**呼び出し元のローカル**であり、関数本体からこの op を出せるようにすると
**同名の呼び出し元変数を書き換えうる**。最上位 Chunk では `scopes.len() == 1` なので
偶然一致していただけだった。

⇒ `vm_assign_global` を **`scopes[0]` 限定**の実装に書き換えた（文言は `assign_var` の
グローバル分岐と一字一句同じ）。これは読み側の `Op::LoadGlobal` が採っている根拠と同じで、
「`cells`/`statics`/`slots` を全部外れた名前＝この関数のローカルでもキャプチャでもない」が
コンパイル時に確定していることに乗る。

⚠ **回帰検知は「呼び出し元の同名パラメータ」でしか書けない**。グローバルと同名の
**ローカル宣言**は静的型検査が禁じる（`variable 'x' is already declared in an accessible scope`）
ので、同名にできるのは**パラメータと for ターゲット**だけ。例題のケース 5 がこれ。

### デバッガ REPL だけは bail を残す

`compile_debug`（`debug_mode`）は停止フレームの**生スコープ**へ書く必要があり、
`scopes[0]` 限定の `StoreGlobal` では別の変数を書いてしまう（読み側が `LoadName` に
落ちているのと同じ理由）。ここは従来どおり bail。
⇒ [compare_debug_modes.ps1](compare_debug_modes.ps1) **5 identical / 0 differing** で確認。

### 検証

`cargo build` 警告 0 ／ `cargo test` **734 緑**（+4）／
`compare_vm_modes.ps1` **80 identical / 0 differing** ／
[compare_debug_modes.ps1](compare_debug_modes.ps1) **5 identical / 0 differing** ／
`scan_examples.ps1` **FAIL 0** ／ `force_gate.ps1` **0 件・136 例題完走** ／
`cargo clippy` 触ったファイルの警告 0。
15 形（基本 8 ＋ 敵対的 7: クロージャ・メソッド・パラメータ/for ターゲットのシャドウ・
`static mut` 併用・ブロック式内・未宣言・不変）が 3 実装で一致
（未宣言／不変はエラー**文言**が `impl_python` と違うが off/on は一致）。

新設例題 [global_assign_from_fn.ar](examples/basics/global_assign_from_fn.ar)（9 ケース）と
[global_assign_from_fn_error.ar](examples/basics/global_assign_from_fn_error.ar)。
⚠ `_error` 例題は**未宣言の名前**にしてある（`let` への代入は**静的型検査が先に捕まえる**ので
実行時経路＝`vm_assign_global` の `NameError` を通らない）。

---

## #36 完了（2026-08-17）— REPL・単体テストの VM 経路移行

**これで `--vm=off` でしか走らない入口も 0 になった**（#33 の前提がすべて揃った）。
⚠ 途中で**ゲートの穴 1 件**と**アドレス再利用の実バグ 1 件**を踏んだ。どちらも
「移行そのもの」より重要な発見だった。

### テスト側: 予測 19 件 → 実際は 1 件

ヘルパーを `prepare()` に一本化し `run_program` と同じ 4 点を揃えた
（`resolve_program` / `check_and_annotate` / `set_toplevel_globals` / `set_vm_mode(On)`）。
⚠ **型エラーは無視する**（静的検査が弾く形を意図的に実行するテストがあるため。欲しいのは注釈だけ）。

**注釈を渡すだけで 18 件が解消**（`mustbe` 13・`cast` 5）。残った 1 件
`test_redeclaration_in_inner_scope` は「ブロックスコープの再宣言」で、**本番では静的型検査が
捕まえる**規則だった（`let`/`fn`/`block:`/`for` の 4 形すべてで確認）。VM の最上位 Chunk は
内側 `let` を slot 宣言に落とすので実行時の重複検査を通らない。
⇒ `static_errors()` ヘルパーを足して**静的検査で固定**した。

個別サイト 8 箇所（callables 3・file_io 3・events_external 1・iterator 1）は
`run_interp()` / `run_err_msg()` へ移行。⇒ **734 テスト全件が本番と同じ VM 経路**で走る。
`eval_expr` だけは `interp.eval()` を直接呼ぶので `VmMode` に依存しない（そのまま）。

### REPL: ブロックごとに配線し、旧ツリーウォークと byte-identical

グローバル名は **`extend_toplevel_globals` で積み増す**（前ブロックの宣言も
「`scopes[0]` を指す」と判断できないと後ブロックの代入が載らない）。
注釈は差し替え（node-id はパース単位なので混ぜられない。#15e どおり食い違っても
「特化が乗らない／bail」方向にしか倒れない）。

対話 REPL には検査網が 1 つも無かったので [repl_session.ps1](repl_session.ps1) を新設
（`examples/repl/repl_session.{in,out}` の golden 比較）。**負の対照 3 種**で検知力を確認:

| 外したもの | ゲート |
|---|---|
| **globals の積み増し** | **発火**（`NameError: 'total' is not defined`）＝ load-bearing |
| `resolve_program` | identical（最適化ヒント） |
| 注釈 | identical（#15e の裏取り） |

⚠ **`Process.StandardInput` は PS5.1 だと UTF-8 BOM を先頭に書く**（REPL が
`ParseError: unexpected token` になった）。`cmd /c` のリダイレクトで与える。
起動バナーは ANSI＋非 ASCII でコードページ依存なので比較から除いた。

### 🕳 ゲートの穴: 最上位に宣言が無いプログラムはツリーウォークだった

**負の対照が発火しなかった**ことから判明。`toplevel_vm_candidate()` が
**`!toplevel_globals.is_empty()`** を条件にしていたため:

```
print(1)
for i in range(3):
    print(i)
```
→ `AR_TW_STATS[toplevel] total=6 Expr=5 For=1` ／ **`tw_control_flow: for-stmt=1`**

⇒ **`force_gate` 0 件・`tw_control_flow` 0 は「例題が必ず何かを宣言している」に依存していた**。
#33（ツリーウォーク削除）の前提が崩れる穴。条件を撤去して `vm_compile toplevel=3` になった。
残る条件は `scopes.len() == 1`（「名前は `scopes[0]`」の唯一の根拠）と `vm_mode != Off` だけ。

### 🐛 最上位 Chunk キャッシュのアドレス再利用（REPL が別文の Chunk を実行した）

`Interpreter::vm_toplevel_chunks` は **`Stmt` のアドレス**をキーにする。REPL はブロックごとに
AST を捨てていたので、アロケータが同じアドレスを再利用し **`let xs = […]` が
`let total = 0` の Chunk を実行**した:

```
NameError: variable 'total' is already declared   ← `let xs = [1, 2, 3]` の行
NameError: 'doubled' is not defined
```

⚠ `toplevel_vm_candidate` の条件を外した結果**最上位が必ず VM になった**ことで露出した
（それまではブロック 1 がツリーウォークで、たまたま衝突していなかった）。
⇒ `run_repl` が実行済みブロックの AST を `Vec<Vec<Stmt>>` に**溜め続ける**ようにした
（`Vec` を move してもヒープ上の要素は動かないのでアドレスは保たれる）。
キャッシュ側にも不変条件をコメントで固定した。

⚠ `import` は安全（モジュール本体は `Stmt::Import` に埋め込まれ、プログラム AST と寿命が同じ）。

### 検証

`cargo build` 警告 0 ／ `cargo test` **734 緑**（全件 VM 経路）／
`compare_vm_modes.ps1` **80 identical / 0 differing** ／
[repl_session.ps1](repl_session.ps1) **identical** ／
[compare_debug_modes.ps1](compare_debug_modes.ps1) **5 identical / 0 differing** ／
`scan_examples.ps1` **FAIL 0** ／ `force_gate.ps1` **0 件・136 例題完走** ／
**過去の全バッテリ 133 形で off/on 不一致 0**。

`tw_stats.ps1`（ゲート穴を塞いだ後の全例題計測。⚠ debug ビルドなので 11 例題はタイムアウト）:

| 指標 | 値 |
|---|---|
| **`tw_control_flow`**（TLS/センチネル経路） | **0** |
| `in_fn`（ツリーウォークの関数本体） | **0** |
| `vm_bail_fn` / `vm_bail_toplevel` / `vm_ineligible` | **すべて 0** |
| `toplevel`（ツリーウォークの最上位文） | 371 ＝ **全て定義文**（`FnDef` 229・`ClassDef` 80・`Import` 28・`TraitDef` 12・`FromImport` 8・`NewTypeDef` 8・`ProtocolDef` 3・`EnumDef` 2・`GenDef` 1）。設計上インタプリタが実行する（#10-d） |
| `module_body` | 20 ＝ 全て定義文（`ClassDef` 19・`FnDef` 1） |
| `vm_compile` | 2,133（toplevel 1,733 ／ fn 398 ／ gen 2） |

⇒ **制御フローを持つツリーウォークは 1 文も無い**という #33 の前提が、
ゲート穴（`!toplevel_globals.is_empty()`）を塞いだ**後の**計測でも成り立っている。

A/B（HEAD #34 前 → #34〜#40 の累積・`--vm=force`・min of 7）:
`bench_arith` 0.968x ／ `bench_control_flow` 0.977x ／ `bench_field_access` 1.019x ／
`bench_method_call` 1.008x ／ `bench_for` 0.962x。⚠ 同じ 2 バイナリでも回ごとに
`bench_field_access` が 0.954〜1.029x で振れる（測定 5 回分）ので、**この幅はノイズ**。
`toplevel_vm_candidate` の条件を 1 本外したことで最上位が VM になる範囲は広がったが、
ベンチはいずれも最上位に宣言を持つので命令列は変わっていない。

---

## #31 完了（2026-08-17）— 参照実装（`impl_python`）との差分検査

[compare_python_impl.ps1](compare_python_impl.ps1) を新設。**#33 で失う `compare_vm_modes` の代替網**。

### ⚠ まず判ったこと: `impl_python` は 100 コミット前に同期されている

`impl_python/__main__.py` 冒頭の `# git SHA: 33ef765…` が同期点で、HEAD まで **100 コミット**ある
（`git rev-list --count 33ef765..HEAD`）。⇒ **既知差分が多いのは当然**で、
「差分がある＝Rust のバグ」ではない。この前提を書かずに網を作ると誤検知の山になる。

### 選別の実測（80 例題）

| 分類 | 件数 |
|---|---|
| **AGREE**（stdout 一致） | **44** |
| DIFFER（両方走ったが出力が違う） | 33 |
| CRASH（`impl_python` が Python 例外で落ちる） | 3 |

⚠ #34〜#40 で新設した例題（`control_flow_expr_escape`・`try_finally_escape`・
`finally_body_escape`・`global_assign_from_fn`）は**すべて AGREE 側**だった
＝ 参照実装と意味論が一致していることの裏取りになっている。

DIFFER/CRASH の 36 件は理由別に 5 群: (a) `impl_python` 未実装の言語機能・組み込み 13 件／
(b) FFI・外部ブリッジ 11 件／(c) repr の違い 3 件／(d) **同期以降に Rust 側へ入った修正** 5 件
（`mut→let` コピー #15e・`static mut`・`block_return` の実行時検査 #35）／(e) エラー出力形式 4 件。

### 設計: skiplist（既定で検査）＋ STALE 報告

- **`$knownDiff` に理由つきで明示列挙**し、**載っていない例題は既定で検査対象**。
  ⇒ 新しい例題を足すと**自動的に検査され、合わなければ落ちる**。
  これは #34/#36 の教訓（**例題が無い／検査されない機能はゲートに映らない**）への対処。
- 逆に `$knownDiff` に載っているのに**一致するようになった**項目は **STALE** として報告する
  （黙って残すと網が緩む）。
- **stdout だけ**を比べる（エラー文言は実装ごとに違う。Rust は色付きの表・py は 1 行）。
- Rust 側は **`--vm` を渡さない既定モード**で走らせる。⇒ **#33 で `--vm=off` が消えても
  このスクリプトはそのまま使える**（`compare_vm_modes` はここが理由で成立しなくなる）。
- パイプは `ReadToEndAsync` で**同時に読む**（#38 のデッドロックを踏まないため）。

### 負の対照 2 種（検知力の確認）

| 対照 | 結果 |
|---|---|
| `impl_python` が未対応の機能（`create_flat_int_list`）を使う例題を新設 | **`UNEXPECTED DIFFERENCES` ＋ exit 1** ✓ |
| 一致する例題（`control_flow`）を `$knownDiff` に追加 | **`STALE` として報告** ✓ |

### 検証

`compare_python_impl.ps1`: **checked 44 / identical 44 / unexpected diff 0 / timeout 0**、
既知差分 36・STALE 0。

⚠ **今後 `impl_python` を更新したら `$knownDiff` を削る**（規約: Python 実装更新時は
git SHA も更新する）。同期点が進めば (d) 群は消え、(a) 群も減るはず。

---

## #33 部分完了（2026-08-18）— `--vm` の削除と、削除できなかった理由

**削除前に到達可能性を測ったら、ツリーウォークの制御フローはまだ生きていた。**
⇒ 削除できたのは「真に到達不能だった分」だけ（**src 実質 -284 行**）。残りは #41 待ち。

### 🕳 生きていた経路: 定義文脈の式

クラスのフィールド既定値・`enum` の値は**定義文の一部**なので、VM の Chunk には載らず
インタプリタが `eval()` で評価する。そこに制御フロー式を書くと**中身も本体の文も**
ツリーウォークが実行する:

```
class Summed:
    const total: int = block ->int:
        mut s = 0
        for i in range(4):
            if i == 2:
                continue
            s = s + i
        block_return s
```
→ `AR_TW_STATS`: **`tw_control_flow: for-stmt=1 block-expr=1`** ／
`toplevel: If=4 BlockReturn=1 For=1 Continue=1 Assign=3 Mut=1`

確認できた形は 4 種（フィールド既定値の `block:` / `if` / `for` 式、`enum` 値）。
`static mut n = block …`（関数内）は VM の `StaticInit` が扱うので**該当しない**。

⇒ **`eval/control_expr.rs`・`exec/control_flow.rs`・TLS 5 種・`ExecResult` の 4 バリアントは
削除できない**。#41（定義文脈の式の VM 化）を立てて #33 の前提にした。

### 🔍 計測そのものが過小報告していた

`eval_if_expr_body` と `eval_match_expr` には **`record_tls` フックが無かった**。
つまり `tw_control_flow` は **if/match 式を 1 件も数えていなかった**。
「制御フローを持つツリーウォークは 0」という判断はこの穴の上に乗っていた。⇒ フックを追加。
併せて `FnBodyGuard`（ツリーウォーク関数本体の深さ計測）を削除した（経路ごと消えたので
`in_fn` は**構造的に 0**）。

### 削除できた分

| 対象 | 行数 |
|---|---|
| `exec_fn_evaled` のツリーウォーク関数本体 | 125 |
| `exec_generator_evaled` のツリーウォーク本体 | 84 |
| async worker のツリーウォーク経路 | 12 |
| `VmMode` enum・`vm_mode` フィールド・`set_vm_mode`・`--vm` の解析と plumbing | ~60 |
| `tw_stats::FnBodyGuard` | 17 |

⚠ **`--vm` は「渡されたら警告して無視」にした**。黙って無視すると、古い比較スクリプトが
「同じものを 2 回実行して一致」と報告して**空回りする**（無い検査より悪い）。

### スクリプトの整理（削除と golden 化）

| スクリプト | 措置 |
|---|---|
| `compare_vm_modes.ps1` / `ab_bench_vm.ps1` | **削除**（`--vm` が無いので空回りする） |
| `compare_debug_modes.ps1` | **[debug_session.ps1](debug_session.ps1) へ golden 化**（5 シナリオの期待値比較。負の対照で検知力確認） |
| `force_gate.ps1` / `tw_stats.ps1` / `tw_stats_files.ps1` | `--vm` 引数を除去 |

### 新設例題

[definition_context_expr.ar](examples/classes/definition_context_expr.ar)（5 ケース）。
**この経路の唯一の網**で、`tw_control_flow` を 6 件計上する。`impl_python` と完全一致。
⚠ この形の例題が 1 本も無かったことが、判断が例題依存になっていた原因そのもの。

### 検証

`cargo build` 警告 0 ／ `cargo test` **734 緑** ／
[debug_session.ps1](debug_session.ps1) **5 identical / 0 differing** ／
[repl_session.ps1](repl_session.ps1) **identical** ／
[compare_python_impl.ps1](compare_python_impl.ps1) **45 検査・45 一致**（例題 1 本増）／
`scan_examples.ps1` **FAIL 0** ／ `force_gate.ps1` **0 件・137 例題完走**。

A/B（HEAD #34 前 → 現在・既定モード・min of 7）: `bench_arith` 0.999x ／
`bench_control_flow` 1.004x ／`bench_field_access` 1.035x ／`bench_method_call` 1.048x ／
`bench_for` 1.022x ／`bench_block_expr` 0.935x。⚠ `bench_block_expr` の 0.935x は
**この一連で初めて測った項目**なので比較材料が無い。他の 5 本は 1.0 前後で、
`bench_field_access` は測定 6 回で 0.954〜1.035x に振れている（ノイズ幅）。

---

## この記録の使いどころ

**見積もりが外れた事例**が最も価値がある。同じ判断ミスを繰り返さないために残してある。

| 事例 | 事前の見積もり | 実際 |
|---|---|---|
| #16 c-2（`field_ty` の置換） | 「§4.4 が名指しした本命レバー」 | **実質デッドコード**（到達 7 件・解決 0 件） |
| #16 c-3（自前導出の撤去） | 「効果は小さい（annot_only=7）」 | **1.73x**（属性が typed になると演算まで連鎖した） |
| #16 (b)(i)（属性の静的ディスパッチ） | 「IC のチェックを省く」 | 真因は IC ではなく **`Value` clone（Rc refcount）**。融合で 1.379x |
| #16 (b)(i) メソッド版 | 同じ手が効くはず | **1.042x**（レシーバは所有権が要り clone が消せない） |
| #14 / #11 R2-c | 仕様書どおり実装する前提 | **消費者が存在しない**ことが計測で判明し保留 |
| #15b（Ident への node-id） | 「ローカルの型を注釈に載せる前提基盤」 | **消費者 0 件**（型落ち 40 件は全てハンドル表現が正しい型） |
| #27 の残り（2026-08-15） | 「`force_gate` 12 件中 7 件が #27」 | **#27 は 4 例題・#27-d が 3 例題**（`force_gate` の文言では両者を区別できない） |
| #27 `for-target-shadow` | 「flat-slot で表現できないだけ」 | **ツリーウォークが間違っていた**（Python 実装 6100 に対し Rust 400100）。bail は表現力ではなくバグの目印だった |
| #27-c `try/except/finally` | 「finally とハンドラの相互作用が複雑」＝新実装が要る | **既存の 2 つを入れ子にするだけ**で全経路が揃った（実質 +10 行） |
| #27-c `let x = <ident>` | 「グローバルソースは非対応なだけ」 | **`Resolution::Global` が非識別子式の枝へ落ちていた**＝可変グローバルのコピー漏れ（潜在バグ） |
| #27-d（2026-08-16） | 「クロージャ＝フレーム表現の変更が必須」 | **半分は可変キャプチャを持たない**。不変キャプチャと `static` はフレームを変えずに載り、4→2 まで減った |
| #32（2026-08-16） | 「async 本体のツリーウォークは 17 文だけ＝些細」 | 文の**種類**が問題だった（本体に直接書いたループは全反復ツリーウォーク＝**3.77x** 遅い）。さらに `--vm` が worker へ届いておらず**ゲートが async を検査していなかった** |
| #27-d 段階 2a→2b | 「`static` は slot を持たないので採番から外せばよい」 | リゾルバは `push_base` するので**以降の slot が全部ずれ**、`LoadLocal` が範囲外を読んだ。2a 単体では症状が出ず、2b で初めて露見 |
| #3（2026-08-17） | 「TLS 4 本・センチネル 2 種を消す」 | **2 つは VM が現に使っていた**（`GENERATOR_YIELDS`＝#8・`RAISE_SENTINEL`＝V-C）。残り 4 つも `--vm=off` のために生きている＝**網を捨てる判断とセット** |
| #3 フォールバック撤去 | 「既定を強制にすればよい」 | **REPL と単体テストが壊れた**（解決情報を持たない文脈では正しいコードも `VmForceError`）。効かせるのは `run_program` だけ |
| #33 の前提（2026-08-17） | 「TLS 4 本は `--vm=off` のためだけに生きている＝捨てる判断さえすれば消せる」 | **`Default` が `Off`** なので REPL と 706 テストも同じ経路。さらに **`--vm=on` で実行できない正しいプログラムが実在**（→ #34/#35）。判断ではなく**前提タスク 3 本**が足りていなかった |
| `force_gate` 0 件 | 「VM は言語全体を載せられる」 | **「128 例題で 0」でしかなかった**。例題が 1 本も無い言語機能（制御フロー式貫通 `break`）は**ゲートにも off/on 比較にも映らない**。単体テストだけが 11 件押さえていた |
| #34（2026-08-17） | 「跳び先の計算が要る＝ジャンプ機構の作り直し」 | **機構は最初から揃っていた**（`LoopCtx` は絶対ジャンプ）。足りなかったのは**跳ぶ時点で積まれている値の始末**だけ。しかも「ブロック式は式の末尾にしか置けない」を**構文で実測**したら、深さの伝播は 3 箇所で済み**新オペコード 0**になった |
| #34 の `continue` | 「`break` を通せば `continue` も同じ」 | **ツリーウォークに `continue` の貫通が無かった**（SyntaxError 化＋**黙って握り潰し**）。VM の方が正しく、参照実装と一致していた。`for-target-shadow` と同じ「bail する形はツリーウォークが正しいとは限らない」 |
| #33（2026-08-18） | 「前提（34〜40）を全部潰したのでツリーウォークを削除できる」 | **削除できなかった**。クラスのフィールド既定値・`enum` 値のような**定義文脈の式**が `eval()` で評価され、その中の制御フローがツリーウォークで動いていた。しかも **`if`/`match` 式には計測フックが無く**、`tw_control_flow` 0 はその過小報告の上に乗っていた。⇒ **`force_gate` 0 件・`tw_control_flow` 0 は毎回「例題がその形を書いているか」に依存する**（#27/#34/#36/#33 で 4 回踏んだ） |
| #31（2026-08-17） | 「参照実装と突き合わせれば差分が出る」 | **`impl_python` が 100 コミット前に同期**されていた（`33ef765`）。差分 36 件のうち 5 件は「同期以降に Rust 側へ入った修正」で、**差分がある＝Rust のバグ ではない**。前提を書かずに網を作ると誤検知の山になる |
| #36（2026-08-17） | 「入口に配線を足すだけ。テスト 19 件が直る」 | 直ったのは **18 件で残り 1 件は静的検査の担当**だった。それより重要な副産物が 2 つ: **`force_gate` 0 件が「例題が必ず何かを宣言している」に依存**していた（最上位に宣言の無いプログラムは最上位丸ごとツリーウォーク）／**最上位 Chunk キャッシュが `Stmt` のアドレス**キーで、REPL が AST を捨てて**別文の Chunk を実行**していた。⇒ **負の対照が発火しないときは配線を疑う** |
| #39（2026-08-17） | 「`store_target` の `toplevel_globals` 門を外すだけ」 | **委譲先が間違っていた**。`Op::StoreGlobal` は `assign_var`（`scopes[frame_floor..]` を先に走査）へ委譲しており、**VM 関数は `scopes` を押さない**ので走査に映るのは**呼び出し元のローカル**。最上位では `scopes.len()==1` で偶然一致していただけで、関数本体へ広げた瞬間に健全性が壊れる形だった |
| #40（2026-08-17） | 「複製ごとにスタックの形が違うので一律の巻き戻しが書けない」＝#37 で保留にした | **`stmt_base` を複製の土台の分だけ持ち上げるだけ**で済んだ（引数 1 本）。既存の巻き戻しがそのまま「保留中の動作を破棄する」意味論を出した。⇒ **保留の理由が「難しい」だけのときは、一度は手を動かして確かめる** |
| #37（2026-08-17） | 「跳ぶ経路にも finally を出す＝新しい機構が要る」 | **#34 の `try_depth` を `try_stack` に一般化するだけ**で済んだ（新オペコード 0）。さらに **`loop_yield` は脱出ではなかった**（蓄積して先へ進む）のに `has_escape` が脱出扱いしており、`try: loop_yield i` が**丸ごと誤爆で bail** していた |
| #35（2026-08-17） | 「注釈を `BlockCtx` に持たせて検査 op を出すだけ」 | **どの注釈を見るか**が本体だった。`BLOCK_RETURN_EXPECTED_TYPE` を push するのは**式だけ**なので、`block:` 文は外側の注釈を継承し、かつ **`loop_yield` には透過**でなければならない。後者を見落として `for … ->list[int]: block: loop_yield i` が **`None`** になった |
| #34 の `try` | 「脱出は `has_escape` が既に弾いている」 | **`has_escape` は文しか歩かない**のでブロック式の中の `break` が素通りし、**ハンドラが残って後続の例外を横取り**した。しかも「跳ぶだけ」の例題では**症状が出ない**（跳んだ後に別の例外を投げて初めて分かる） |

**教訓**: 着手前に診断フックで数字を取る。IR / バイトコードを実際にダンプして見る。
