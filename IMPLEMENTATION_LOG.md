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

## #41 完了（2026-08-18）— 定義文脈の式の VM 化

#33 が削除できなかった 2 つの消費者のうち**片方**を潰した。もう片方（import モジュール本体）は
#42 として残る。

### 対象は 7 箇所（`definitions.rs` の `self.eval(...)`）

クラスのフィールド既定値 ×2・クラス変数（`static mut`/`const`）・`enum` 値・
デコレータ ×3（関数・メソッド・クラス）。すべて**定義文の一部**なので
`is_toplevel_compile_target` の除外リストに入っており、最上位 VM の対象外だった。

⚠ デコレータも `@block ->T:` と書けるので対象（構文で確認）。
⚠ 関数内の `static mut n = block …` は VM の `StaticInit` が扱うので**対象外**。
⚠ 関数内のクラス定義は元から `VmForceError`（関数本体が載らない）なので考えなくてよい。

### 鍵は `LoadName`（新フラグ `name_lookup`）

定義文の実行位置は**最上位とは限らない**（import モジュール本体の中のクラス定義など）。
`scopes[0]` 限定の `LoadGlobal` を使うと、モジュール本体で宣言された名前に当たらず
**ツリーウォークの `eval()` と答えが変わる**。⇒ 自由な識別子は `Op::LoadName`
（実行時のスコープ走査＝`eval()` と同じ）に落とす。

`debug_mode` を流用しなかった理由: あちらは**融合と FFI 情報の記録も止める**ので、
定義文脈の式で FFI 境界検査が素通りする。`name_lookup` は識別子の読み書きだけを変える。
書き込みは `store_target` が bail する（式の中で宣言したローカルは `slots` が先に拾う）。

### 効果

[definition_context_expr.ar](examples/classes/definition_context_expr.ar) の
`tw_control_flow` **6 → 0**。#33 で見つけた 8 形すべて結果不変。

### 🕳 残るもう 1 つの消費者 → #42

`exec_module` は `push_scope` してから本体を回すので `scopes.len() != 1` になり、
**モジュール本体は丸ごとツリーウォーク**:

```
TwStats[module_body] total=18 If=5 Assign=4 LoopYield=3 FnDef=2 Continue=1 For=1 Mut=1 Let=1
TwStats[tw_control_flow] total=2 loop-expr=1 for-stmt=1
```

⚠ #10-d が「モジュール本体は 20 文」として保留にした判断も**例題依存**だった
（最上位に制御フローを持つモジュールの例題が 1 本も無かった）。**同じパターンは 5 回目**。
⇒ [module_toplevel_flow.ar](examples/interop/module_toplevel_flow.ar) ＋
`test_modules/mod_toplevel_flow.ar` を新設して可視化した。

### 検証

`cargo build` 警告 0 ／ `cargo test` **734 緑** ／
[debug_session.ps1](debug_session.ps1) **5 identical** ／
[repl_session.ps1](repl_session.ps1) **identical** ／
[compare_python_impl.ps1](compare_python_impl.ps1) **46 検査・46 一致**（例題 2 本増）／
`scan_examples.ps1` **FAIL 0** ／ `force_gate.ps1` **0 件・139 例題完走**。

---

## #42 完了（2026-08-18）— import モジュール本体の VM 化

#33 が削除できなかった 2 つの消費者の**もう片方**。これで両方潰れた。

### 足りなかったのは「名前ベースの代入」だけだった

モジュール本体は `exec_module` が `push_scope` してから回すので、名前は `scopes[0]` ではなく
**push 済みスコープ**に入る。既存の仕組みを 1 つずつ確かめたら、大半はそのまま使えた:

| 操作 | 既存の挙動 | 判定 |
|---|---|---|
| 宣言（`Op::DeclareGlobal`） | `declare_var` → **`scopes.last_mut()`** | ✅ そのまま正しい |
| 読み（`Op::LoadName`） | スコープチェーン探索 | ✅ そのまま正しい |
| **代入（`Op::StoreGlobal`）** | **`scopes[0]` 限定**（#39 で厳格化した） | ❌ 届かない |

⇒ **`Op::StoreName`（`assign_var` へ委譲）を 1 個足すだけ**で済んだ。
`assign_var` はツリーウォークの `Stmt::Assign` が使う**同じ関数**なので意味論のずれが起きない。

⚠ #39 で `vm_assign_global` を `scopes[0]` 限定へ厳格化していなければ、ここは
「たまたま動くが呼び出し元のローカルを壊しうる」形になっていた。**厳格化しておいたことで、
足りない部分が「届かない」という形で明示的に現れた**。

### 実装

- `module_mode` フラグ ＋ `compile_module_stmt`（`compile_toplevel_stmt` と共通の内部関数へ
  フラグを通すだけ）／`store_target` が `module_mode` なら `StoreTarget::Name`。
- `try_run_module_stmt`（`scopes.len() == 1` を要求しない版）。
- `exec_module` の**実行ループ 2 箇所**（通常モジュール／ネイティブモジュールのスタブ本体）を接続。
- 定義文は従来どおりインタプリタが実行する（`is_toplevel_compile_target` が `None` を返す・#10-d）。

### 効果

[module_toplevel_flow.ar](examples/interop/module_toplevel_flow.ar):

| | 前 | 後 |
|---|---|---|
| `module_body`（ツリーウォーク文） | **18**（`If=5 Assign=4 LoopYield=3 For=1 Continue=1 …`） | **2**（`FnDef` のみ＝定義文） |
| `tw_control_flow` | `loop-expr=1 for-stmt=1` | **なし** |
| `vm_compile` | — | `module=3` |

### 検証

`cargo build` 警告 0 ／ `cargo test` **734 緑** ／
[debug_session.ps1](debug_session.ps1) **5 identical** ／ [repl_session.ps1](repl_session.ps1) **identical** ／
[compare_python_impl.ps1](compare_python_impl.ps1) **46 検査・46 一致** ／
`scan_examples.ps1` **FAIL 0** ／ `force_gate.ps1` **0 件・139 例題完走**。

### 🔎 #33 の前提の検算（全例題 `tw_stats`）

| 指標 | 値 |
|---|---|
| **`tw_control_flow`** | **0** |
| `in_fn` | **0** |
| `vm_bail_fn` / `vm_bail_toplevel` / `vm_ineligible` | すべて **0** |
| `toplevel`（ツリーウォーク） | 379 ＝ **全て定義文** |
| `module_body` | **22 ＝ 全て定義文**（`ClassDef` 19・`FnDef` 3。**前回は制御フロー込み**） |
| `vm_compile` | 2,158（toplevel 1,752 ／ fn 401 ／ **module 3** ／ gen 2） |

⚠ **今回は測定が盲目ではない**: #41/#42 で見つけた 2 つの消費者はどちらも例題を追加したので、
例題スイートが実際にその形を踏んでいる。これが過去 5 回との違い。

`exec()` の外部呼び出し元も総ざらいした（6 箇所）:
`run_program` ／ `exec_repl_stmt` ／ モジュール本体（定義文のみ）／ 定義文 ／
ツリーウォーク制御フロー自身（削除対象）／ **デバッガ REPL のフォールバック**。
最後のものは **1 行ずつ読む**ので、制御フロー式（`:` ＋改行＋インデントが必須）を
**構造的に受け取れない**（単一行形は `ParseError`。実測で確認）。

⇒ **#33 の残り（制御フロー本体 ≈700 行の削除）は着手可能**。

### A/B（HEAD #34 前 → #42 まで・既定モード・min of 7）

`bench_arith` 1.022x ／`bench_control_flow` 1.006x ／`bench_field_access` 1.038x ／
`bench_method_call` 1.017x ／`bench_for` 0.992x ／**`bench_block_expr` 0.936x**。

⚠ `bench_block_expr` の 0.936x は**測定 2 回で再現した＝ノイズではない**。原因を特定した:
`AR_VM_DUMP` で数えると `CHECK_LOOP_YIELD` が **1 個**あり、それが
`for i in range(n) ->list[int]: loop_yield i * i` の**内側**＝反復ごとに走る。
**#35 が足した実行時型検査の費用**で、`--vm=off` も同じ検査をしていたので**退行ではない**
（VM だけが検査を飛ばしていたのを直した結果）。
⇒ 注釈が `list[int]` で yield 式も `int` と確定しているならコンパイル時に省ける → **#43**。

---

## #43 完了（2026-08-18）— 実行時型検査の高速判定

#42 の A/B で唯一実測された速度課題（`bench_block_expr` 0.936x）を潰した。
**理論上限の 104% を回収**＝検査が実質無料になった。

### まず費用を切り分けた

`loop_yield` 支配のマイクロベンチ（10M 回）で **検査を消したビルド**と比べる:

| | min |
|---|---|
| 検査あり（#43 前） | 0.740 s |
| **検査を消した上限** | 0.633 s |
| ⇒ 検査の費用 | **12.4%** |

### 🔑 「省略」ではなく「速くする」を選んだ

当初の案は「注釈が `list[int]` で yield 式も `int` と確定していれば**検査を省く**」だったが、
これは **#15e の「注釈は最適化ヒントであって意味論の根拠ではない」を破る**（推論が外れたとき
ツリーウォークならエラーになる値を黙って通してしまう）。

⇒ 検査は残したまま**判定を速くする**方針に変えた。費用の内訳を見ると、実際の判定より
**文字列処理**（`extract_list_elem_type` の `strip_prefix`/`strip_suffix` ＋
`c_abi_base_type` ＋ `match ann` の文字列比較連鎖）が**反復ごとに**走っていた。

### 実装

- `TypeTag`（`Any`/`Int`/`UInt`/`Float`/`Str`/`Bool`/`Other`）を **アノテーション文字列から**
  コンパイル時に決めて op に載せる（`CheckLoopYield(u32, TypeTag)` / `CheckBlockReturn(u32, TypeTag)`）。
  ⚠ **型推論の結果は使わない**。見ているのは「ソースに書かれた型注釈そのもの」なので、
  判定内容は `value_matches_type_ann` と同一で、**文字列比較を列挙比較へ置き換えているだけ**。
- 実行時は `tag.matches(v)` 1 回。**外れたときだけ**一般判定へ落とすので
  **エラー文言は 1 実装のまま**（`Other` は常に一般判定）。
- `Any` は自明に真なので **op 自体を出さない**。`list[T]` の形でない注釈も同様
  （`check_loop_yield_type` が `Ok(())` を返すのと同じ）。
- ⚠ `Op` のサイズは **20 バイトのまま**（`op_size_is_pinned` 緑）。

### 効果

| | 値 |
|---|---|
| `loop_yield` マイクロベンチ | 0.740 → **0.629 s**（**1.177x**） |
| 検査を消した上限（0.633 s）との差 | **-0.6%**（＝**上限の 104% を回収**・ノイズ内） |
| `bench_block_expr`（HEAD #34 前との比） | 0.936 → **1.016x** |

⇒ **#34〜#43 の累積で全ベンチが HEAD 同等以上**になった
（`bench_block_expr` 1.016x ／`bench_control_flow` 1.008x ／`bench_arith` 1.004x ／`bench_for` 0.994x）。
break/continue の巻き戻し・実行時型検査・finally 複製・グローバル代入・定義文脈と
モジュール本体の VM 化を**全部足した上で**この水準。

---

## #33 完了（2026-08-18）— ツリーウォーク制御フローの削除【本系列の主目的】

**src 実質 -762 行**。解釈実行はバイトコード VM 一本になった。

### 着手前の規約を守った

計画書に自分で書いた「**着手前に必ず `tw_stats.ps1` を全例題で取り直す**」を実行し、
`tw_control_flow` 0・`vm_bail_*` 0・`in_fn` 0・ツリーウォークは**定義文のみ**（最上位 379・
モジュール本体 22）を確認してから削除に入った。⚠ この規約は #27/#34/#36/#33/#41 で
**5 回**「例題が踏まない形が残っていた」ことから作ったもの。今回は #41/#42 で見つけた
2 つの消費者に**例題を追加済み**なので、測定が盲目ではない。

### 4 層に分けて削除（各層でビルド＋テスト）

| 層 | 対象 |
|---|---|
| A | `eval/control_expr.rs` 全体（**348 行**）・`eval_match_expr`・`eval()` の 5 アーム |
| B | `exec/control_flow.rs`（227→**55 行**。`make_for_iterator` だけ残す＝`Op::GetIter` が使う）・`exec()` の 10 アーム |
| C | **TLS 3 本**（`BLOCK_YIELDS`/`LOOP_DEPTH`/`BLOCK_RETURN_EXPECTED_TYPE`）・**センチネル 2 種**（`BREAK_SENTINEL`/`CONTINUE_SENTINEL`）・`exec_loop_yield`・`record_tls`・`extract_result_guard_call` |
| D | `ExecResult` の 4 バリアント（`Break`/`Continue`/`BlockReturn`/`BlockYield`）・ツリーウォークの `try/except/finally`・`exec_block`/`exec_scoped_block` |

⚠ `GENERATOR_YIELDS`（#8）と `RAISE_SENTINEL`（V-C）は **VM が使うので残す**（当初計画どおり）。

### 🔑 アームは「削除」ではなく「明示的なエラー」に畳んだ

`eval()`/`exec()` の制御フローアームを match から消すと、将来 `Expr`/`Stmt` に
バリアントが増えたときに黙って別の挙動へ落ちうる。⇒ **1 つのエラーアームに畳み**、
到達したら `VmForceError: control-flow … reached the tree-walk` で止める。
コメントに**到達しうる入口を全部列挙**した（最上位／モジュール本体／関数・ジェネレータ・
async 本体／定義文脈の式／デバッガ REPL は 1 行入力なので構文上不可）。

### 検証

`cargo build` 警告 0 ／ `cargo build --features tw_stats` 警告 0 ／ `cargo test` **734 緑** ／
[debug_session.ps1](debug_session.ps1) **5 identical** ／ [repl_session.ps1](repl_session.ps1) **identical** ／
[compare_python_impl.ps1](compare_python_impl.ps1) **46 検査・46 一致** ／
`scan_examples.ps1` **FAIL 0** ／ `force_gate.ps1` **0 件・139 例題完走**。

A/B（HEAD #34 前 → #33 完了・既定モード・min of 7）:
`bench_arith` 1.032x ／`bench_control_flow` 0.984x ／`bench_field_access` 1.032x ／
`bench_method_call` 1.016x ／`bench_for` 0.975x ／`bench_block_expr` 1.023x。
⇒ **#34〜#43 の全変更（機能追加・実行時検査の追加を含む）を積んで概ね同等**。

---

## #38 完了（2026-08-18）— A/B 計測スクリプトのデッドロック解消

[ab_bench.ps1](ab_bench.ps1) の `Measure-Run` が `StandardOutput.ReadToEnd()` →
`StandardError.ReadToEnd()` を**逐次**呼んでいた。子が stderr のパイプ（Windows の匿名パイプは
既定 4KB）を埋めると子は書き込みでブロックし、親は stdout の EOF を待ち続けて相互に固まる。
[scan_examples.ps1](scan_examples.ps1) と同じ `ReadToEndAsync()` による**同時読み**へ揃えた。

### 🔑 先に「壊れていること」を再現してから直した

負の対照が作れたのが今回の収穫。**`AR_VM_DUMP=1` を付けると子が 7KB の stderr を吐く**ので、
これが再現条件そのものになる（`bench_branch.ar` で実測 7014 バイト）。

| 条件 | 修正前 | 修正後 |
|---|---|---|
| 通常実行（stderr ほぼ 0） | 通る | 通る |
| **`AR_VM_DUMP=1`**（stderr 7KB） | **ハング**（45s で `timeout` が kill・exit 124） | 1.004x を出して正常終了 |

⇒ **この不具合は「普段の実行では踏まない」**。A/B を取る場面＝診断フックを付けて中身を見たい
場面でだけ発火するので、「昨日は動いた」は直った証拠にならない。回帰を疑ったら
`AR_VM_DUMP=1` を付けて回すこと。

### ついでに潰した「黙って嘘の値を出す」経路 3 つ

デッドロックと同じく**計測手段が計測者を欺く**種類の問題なので同時に直した。

| 症状 | 修正前の挙動 | 修正後 |
|---|---|---|
| 子が異常終了 | stdout/stderr を捨てて経過秒だけ返す ⇒ **即死した実行が「最速」として min に入る** | `Ok=$false` にして min から除外し、`EXIT=n` ＋ stderr 末尾 1 行を表示 |
| 子が終わらない | `WaitForExit()` に上限が無く**無限待ち** | `-TimeoutSec`（既定 180）で kill し `TIMEOUT(180s)` と表示 |
| `-Scripts` のパスが無い | `Test-Path` で**黙って continue** ⇒ **表が空のまま exit 0** | `NOT FOUND: <path>` と表示 |

⚠ 3 つ目は自分で踏んだ。`powershell -File ./ab_bench.ps1 -Scripts a.ar,b.ar,c.ar` と渡すと
**`-File` は 1 要素の文字列として束縛する**（`-Command` なら配列に割れる）ので、
3 本指定したのに 1 件も測らず成功したように見えた。

⚠ 失敗した実行があった行には `!!` を付け、末尾に `WARN:` 行で再掲する。
**片側が 1 回も成功しなかったら比率を出さない**（`-` を表示）。

### 検証

負の対照（`AR_VM_DUMP=1`）で修正前ハング → 修正後完走を確認。同一バイナリを A/B に渡して
`bench_branch` 1.002x ／`bench_arith` 0.983x ／`bench_for` 1.000x（**ノイズ床 ±5% の内側**）。
異常終了（`mustbe_error.ar`）・タイムアウト（`-TimeoutSec 1`）・不在パスの 3 経路とも
表示を確認し、kill 後に **孤児 `arrow.exe` が残らない**ことも確認した。
⚠ src は 1 行も触っていないので 5 点セットのゲートは対象外（`git status` は本スクリプトのみ）。

⚠ **BOM 必須**（PS5.1 は BOM 無し `.ps1` を ANSI として読む）。編集後に BOM を付け直した。
Write-Host の日本語が化けて見えるのは**捕捉側のコードページ**（cp932）であって
ファイルの問題ではない（`force_gate.ps1` も同じ形で日本語を出している）。

---

## #30 完了（2026-08-18）— クロージャ Chunk の実体跨ぎ再利用

`get_or_compile_chunk` は `Rc::as_ptr(fn_val)` をキーにするので、**クロージャは実体ごとに
本体を再コンパイル**していた。定義サイト（`ChunkFnDef`）に共有の器を持たせて 1 回にした。

### 🔑 まず「ベンチが 1 本も無い」を埋め、コンパイル費用を切り分けてから設計した

計画書の指示どおり計測から入った。**コンパイルは初回呼び出しまで遅延する**ので、
「生成するだけ」と「生成して 1 回呼ぶ」を比べると**コンパイル費用だけが差になる**。

| 経路（20000 実体・min of 3） | 修正前 | 1 実体あたり |
|---|---|---|
| 生成のみ（呼ばない＝コンパイルされない） | 0.021 s | 1.05 µs |
| 生成 ＋ 1 回呼ぶ | 0.073 s | 3.65 µs |
| 対照: 素の関数呼び出し | 0.0055 s | 0.275 µs |

⇒ **再コンパイル ≒ 2.3 µs/実体 ＝ クロージャ呼び出し約 8 回分**。
「生成して 1〜2 回呼ぶ」形（コールバックをループで作る等）では**費用の 6 割がコンパイル**。

`AR_VM_DUMP=1` でチャンク数を数えると **5 実体→8 個・50 実体→53 個**＝実体数に比例していた
（修正後はどちらも **4 個**）。件数で確かめられるので、時間だけを見るより強い。
`AR_TW_STATS=1` でも同じことが出る — [closure_instances.ar](examples/basics/closure_instances.ar) の
`vm_compile.fn` が **62 → 6**。**6 は入れ子 `fn` の定義サイト数ちょうど**
（`make_adder`/`add`/`make_counter`/`inc`/`make_stepper`/`bump`）＝「定義サイトごとに 1 回」の直接の証拠。

### 🔑 「共有してよい」根拠は**既に揃っていた**（新しく作った不変条件は 0）

見積もりでは「実体ごとにキャプチャの並びが変わるから正規化が要る」と踏んでいたが、
実装を読むと **2 つとも先に手当て済み**だった:

1. `compile_fn` は `capture_names.sort()` してから slot を採番する
   （`captured_env` が HashMap なので反復順が実行ごとに変わることへの対策・#27-d）。
2. `bind_captures` / `build_cells` は **名前で引く**（slot 番号や位置に依存しない）。

⇒ **Chunk は元から実体に依存していなかった**。足りなかったのは「定義サイトを指すキー」だけ。
⇒ 実装は `ChunkFnDef::compiled`（`SharedFnChunk`）を足して `Op::MakeFn` が配るだけで済み、
**新オペコード 0・意味論の変更 0**。

### ⚠ 注釈テーブルのガードは `Rc` ごと持つ（生ポインタは #36 の再演になる）

注釈（`Interpreter::annotations`）は入口によって差し替わる（REPL）ので、コンパイル時の
テーブルを覚えて違えば作り直す。当初 `Rc::as_ptr` の `usize` で比較していたが、
**古い表が解放されるとアロケータが同じアドレスを再利用して「同じ表だ」と誤判定する**
（最上位 Chunk キャッシュが `Stmt` のアドレスで踏んだのと同型・#36）。
⇒ `Rc<AstAnnotations>` を**キャッシュが保持**する形に変えた。持っている間は解放されないので
アドレス再利用が原理的に起きない。費用は `Rc::ptr_eq` 1 回（HashMap 引き + `Weak::upgrade` より軽い）。

### 副産物: `vm_chunks` の際限ない成長も止まった

旧経路は `vm_chunks.insert(key, …)` を**実体ごと**に行い、エントリを消さない。
クロージャを 20000 個作れば 20000 エントリ残る（`Weak` は誤ヒット防止であって掃除ではない）。
共有経路はエントリを 1 つも作らないので、この成長ごと消えた。

### 検知力の確認（負の対照を 2 種）

新設した不変条件テスト 3 本が**本当に取り違えを検出するか**を、わざと壊して確かめた。

| 壊し方 | 落ちたテスト |
|---|---|
| 共有 Chunk のとき `bind_captures` を飛ばす（＝実体 1 のキャプチャを持ち回る） | 4 本 FAILED（不変キャプチャ側） |
| `build_cells` の `captured_cells` 反映を飛ばす | 7 本 FAILED（可変キャプチャ側） |

⚠ 1 つ目で**セル側のテストは通ってしまう**（可変キャプチャは `bind_captures` を通らない）。
不変キャプチャ・可変キャプチャ・**混在**の 3 本を別々に置いてあるのはこのため。

### 実測

A/B（HEAD `a3eaf2c` → 本変更・`ab_bench.ps1` の交互実行・min of 5）:

| ベンチ | A/B |
|---|---|
| **`bench_closure`（新設）** | **1.456x** |
| `bench_arith` | 1.006x |
| `bench_for` | 1.030x |
| `bench_method_call` | 1.017x |
| `bench_field_access` | 0.979x |
| `bench_block_expr` | 0.990x |

⇒ クロージャ以外は**ノイズ床 ±5% の内側**。区間別では
生成支配 **2.2〜2.3x**（`spawn` 0.077→0.034 s）、呼び出し支配は 1.15x / 0.96x（ノイズ内）。

### 検証

`cargo build` 警告 0 ／ `cargo test` **737 緑**（+3 = 新設した不変条件テスト） ／
`clippy` **増分 0**（HEAD と同一の 52 件・触ったファイルには 1 件も出ていない） ／
[scan_examples.ps1](scan_examples.ps1) **FAIL 0** ／
[compare_python_impl.ps1](compare_python_impl.ps1) **47 検査・47 一致**
（新例題が**自動で検査対象に入った** = #31 の設計どおり） ／
[repl_session.ps1](repl_session.ps1) **identical** ／
[force_gate.ps1](force_gate.ps1) **0 件・141 例題完走**（新例題 2 本ぶん増） ／
[tw_stats.ps1](tw_stats.ps1) `in_fn` **0**・`tw_control_flow` **0**・`vm_bail_*` **0**・
`vm_ineligible` **0**（#33 完了時の状態を維持＝適格範囲は動かしていない）。

### 🕳 副産物: [debug_session.ps1](debug_session.ps1) は **#33 partial からずっと赤かった**

5 件とも FAILED。**本変更とは無関係**（HEAD のバイナリでも同一に落ちる）だが、原因は
「環境差」ではなく**ゲート自身の不整合**だった。

`6bf039c`（#33 partial）が **1 つのコミットで両方**やっている:
- `debug_session.ps1` に「stdin を **BOM 無し**で書く」修正を入れた
  （PS5.1 の `StandardInput` は BOM を付ける・#36 で REPL が踏んだのと同じ穴）
- golden `.out` を**その修正前の出力のまま**コミットした

⇒ golden には BOM が引き起こした痕跡が焼き付いている: 余分な入力 1 行を消費したことによる
**二重プロンプト `(dbg) (dbg) `** と、stderr の `[dbg] Error: unexpected token: \`?<BOM>\``。
BOM を送らなくなった現在の出力とは**必ず 1 行ずれる**ので、以後どのコミットでも通らない。

⚠ **#33 の完了報告に「debug_session.ps1 5 identical」と書いてあるのは誤り**。
⚠ **`-Update` で黙って上書きしない**。差分の中身（二重プロンプトの解消と偽エラーの消滅）を
確かめた上で録り直すべきもので、それ自体を 1 タスクとして立てる（→ **#44**）。

⇒ ステッピングを**本変更について**検査する代替として、**同じ 5 シナリオを 2 つのバイナリで
走らせて transcript を突き合わせた**（`compare_vm_modes` を失った後の常法・A/B と同じ考え方）:
**5/5 byte-identical**（HEAD `a3eaf2c` vs 本変更）。⇒ #30 はデバッガ経路に影響しない。

### 残した課題（測ったが手を付けていない）

クロージャ生成そのものは **1.05 µs/実体**残っており、その大半は `FnValue` が
**`body: Vec<Stmt>` を毎回ディープクローンする**こと。`Rc<[Stmt]>` 化すれば消せるが、
`body` は広く使われるので影響範囲が大きい。**#30 の目的（Chunk の再利用）とは別軸**なので
分離した（→ 新タスク #45 を提案）。

---

## #44 完了（2026-08-18）— デバッガ golden の録り直し

[debug_session.ps1](debug_session.ps1) が **`6bf039c`（#33 partial）からずっと 5/5 FAILED**
だった件（#30 の作業中に発見）。golden を録り直して **5 identical** に戻した。

### 何が起きていたか

`6bf039c` が **1 つのコミットで両方**やっていた:
- `debug_session.ps1` に「stdin を **BOM 無し**で書く」修正を入れた
  （PS5.1 の `StandardInput` は BOM を付ける — #36 で REPL が踏んだ穴と同じ）
- golden `.out` を**その修正前の出力のまま**コミットした

⇒ golden には BOM が引き起こした痕跡が焼き付いていた。差分は**きっちり 3 種類**:

| 症状 | 旧 golden | 実際の正しい出力 |
|---|---|---|
| **最初のコマンドの結果が消えていた** | `(dbg) (dbg) ` ＝ 二重プロンプト | `(dbg) 3.0` などの実値 |
| **偽のパースエラー** | stderr に `[dbg] Error: unexpected token: \`?<BOM>\`` | 出ない |
| **コマンド列が 1 つずれて末尾が落ちていた** | `step_over_heavy` の停止が **2 回** | **3 回**（正しい） |

### 🔑 「違う」ではなく「**正しい**」ことを 1 値ずつ裏取りしてから録り直した

`-Update` は最後の手段。復活した 4 値はすべてソースから独立に確かめられる:

| シナリオ | `.in` の 1 行目 | 値の根拠 | 復活値 |
|---|---|---|---|
| `dbg_vars` | `p.x` | `Point(3.0, 4.0)` | **3.0** |
| `step_into_fn` | `base` | `let base = 5` | **5** |
| `step_out_to_vm_caller` | `inside` | `caller(5)`→`before=6`→`probe(6)`→`a*3` | **18** |
| `step_over_out` | `first` | `outer(4)`→`inner(4)`→`4*2+1` | **9** |

### ⚠⚠ 壊れた golden は「名前どおりの検査をしていなかった」

`step_over_heavy` は **step-over が重い呼び出しを跨ぐこと**（#1-b の 1.97x）を見るシナリオなのに、
BOM がコマンドを 1 つ食っていたせいで **21→22 の step-over に到達する前に終わっていた**。
⇒ **赤いゲートは「検知力が落ちる」のではなく「別物を検査する」**。放置期間の長さより、
その間ずっと「step-over を検査しているつもりだった」ことの方が損害が大きい。

### 検知力の確認（負の対照 3 種）

録り直した golden が**本当にステッピングの退行を捕まえるか**を、わざと壊して確かめた。

| 壊し方 | 結果 |
|---|---|
| `StepOver` の深さ判定を `==` → `>=`（step-over が呼び先へ降りる） | **FAILED: step_over_heavy** |
| `StepInto` が関数に入った所で止まらない（`true`→`false`） | **FAILED: dbg_vars, step_into_fn, step_over_out** |
| `StepInto` の `StepOver` 遷移を削除 | **検知せず — ただしこれは意味論的に no-op**（次の文で `skip==0` の枝が同じ遷移をする）。ゲートの穴ではない |

⚠ 1 種類目が **`step_over_heavy` だけ**を落とすことに注意 — 壊れた golden が
**唯一検査できていなかったシナリオ**がまさにここ。録り直しで戻った検知力の実体はこれ。

### 再発防止: `-Update` は**書き換える行を必ず出す**ようにした

原因は「録り直し漏れがレビューで見えない」こと。⇒ `-Update` を
**`UPDATING`（差分つき）／`UNCHANGED`（書かない）／`CREATING`** の 3 表示に変え、
黙って上書きできなくした。差分表示は比較側と `Show-Diff` に**1 実装化**（#22 系列と同じ判断・
片方だけ直すとずれる）。

⚠ これで防げるのは「**上書きしたのに気づかない**」だけ。「修正と golden を同じコミットに
入れる」こと自体は防げないので、**ゲートは自分で走らせて緑を確認する**（#30 で追加した落とし穴）。

### 検証

`debug_session.ps1` **5 identical**（`-Filter` なし・release バイナリ）。
transcript は**繰り返し実行でも、コンソールのコードページを変えても byte-identical**
（`[Console]::OutputEncoding` の有無で 2 通り試した — 記録環境に依存しないことの裏取り）。
`-Update` の 3 経路（UNCHANGED / UPDATING / CREATING）を実地で確認。
⚠ `src/` は 1 行も触っていない（`git status` は `debug_session.ps1` と golden 5 本のみ）。

---

## #45 完了（2026-08-18）— `FnValue.body` を `Rc<[Stmt]>` へ（実体ごとの AST 複製を除去）

クロージャ生成の残り費用（#30 で測った 1.05µs/実体）のうち、**本体 AST のディープクローン**を消した。

### 🔑 まず計測して、計画書に書いた自分の前提を否定した

#45 の根拠は「1.05µs の大半が `body: Vec<Stmt>` の複製」だった。**本体の文数だけを変えて測る**と:

| 本体の文数（20000 実体・warmup 後） | 修正前 | 1 実体あたり |
|---|---|---|
| 1 文 | 0.015 s | 0.75 µs |
| 10 文 | 0.055 s | 2.75 µs |
| 40 文 | 0.177 s | 8.85 µs |

⇒ **≈0.19 µs/文の線形項 ＋ 約 0.56 µs の固定項**（残りは `make_adder` 呼び出し自体 ~0.28µs）。
つまり **1 文の本体では body 複製は 1.05µs のうち 0.2µs しかない**＝計画書の前提は誤り。
**効くのは本体が大きいときだけ**。⇒ タスクの価値は「速くなる倍率」ではなく
**「本体サイズへの依存を O(n)→O(1) にすること」**だと定義し直してから着手した。

### 影響範囲は小さかった（消費側 0 件）

型を変えて `cargo build` に列挙させたら **エラーは構築側 10 箇所だけ**。
`Rc<[Stmt]>` は `[Stmt]` へ deref するので、`&fn_val.body` を読む側は**1 箇所も直らなかった**。

### ⚠⚠ 本題はここ: `body.clone()` の**意味が黙って変わる**

`Vec<Stmt>` → `Rc<[Stmt]>` にした瞬間、`body.clone()` は
**「中身の複製」から「参照カウント加算」へ変わる。しかもコンパイルは通る。**
`deep_clone`（スレッド送出経路）でこれが起きると、非アトミックな参照カウントを
複数スレッドが叩く — **#15 で `Value::Str` を直したのと同じ穴が、型変更だけで復活する**。

該当は 5 箇所。うち **3 箇所が `FnValue`**（`Value::Function` / `Value::OverloadedFn` /
`ClassValue::deep_clone` のメソッド）で、`Rc::from(&rc.body[..])` に直した。
残り 2 箇所は `GeneratorFnValue`（`body` は `Vec<Stmt>` のまま）なので `.clone()` で正しい。

⇒ **型では守れない不変条件なのでテストで固定した**（ポインタ同一性を直接見る 3 本）。
負の対照で **1:1 の対応**を確認:

| 壊した経路 | 落ちたテスト |
|---|---|
| `Value::Function` | `..._fn_body_rc` のみ |
| `Value::OverloadedFn` ＋ `ClassValue` | `..._overloaded_fn_body_rc` と `..._method_body_rc` のみ |

### ⚠ async の実地ストレスは**この誤りを検出しない**（測って確かめた）

#15 の手本（[async_string_share.ar](examples/async/async_string_share.ar)）に倣って
[async_closure_share.ar](examples/async/async_closure_share.ar) を書いたが、
**わざと共有させても 5 回とも正常終了**した。理由:

- `let a = f` が clone するのは **外側の `Rc<FnValue>`** で、内側の body `Rc` には触らない。
- `deep_copy_unfrozen` は `Value::Function` を扱わない（`let` 引数コピーでも複製されない）。
- ⇒ worker が内側カウンタに触るのは **タスク終了時の drop 1 回だけ**＝競合窓が狭すぎる。

⇒ **例題は残したが「機能が通ること」の確認に格下げ**し、ヘッダに
「この例題は誤りを検出しない／決定的な検査は単体テスト側」と明記した。
**検知力の無い検査を検知力があるかのように置かない**（#44 で踏んだのと同じ轍）。

### ⚠ ベンチが #45 の対象を測っていなかった（#44 と同じ構図）

`bench_closure` の A/B が **1.030x** にしかならず、原因は
**#30 で書いた自分のベンチの本体が 1 文**だったこと＝**この変更が効く形を書いていない**。
⇒ 本体 13 文の区間 **C-1** を追加した。追加後:

| 区間 | HEAD (#44) | #45 | 倍率 |
|---|---|---|---|
| **C-1 本体 13 文の生成** | 0.0751 s | 0.0230 s | **3.26x** |
| A-1 本体 1 文の生成 | 0.0331 s | 0.0280 s | 1.18x |
| B-1 / B-2 呼び出し支配 | 0.063 / 0.058 | 0.065 / 0.058 | 同等（退行なし） |

本体サイズ別の直接計測では **1 文 1.11x ／ 10 文 4.07x ／ 40 文 13.1x**、
**#45 後はどのサイズでも 0.68 µs で一定**（＝ O(本体) の項が消えた）。

### 検証

`cargo build` 警告 0 ／ `cargo test` **740 緑**（+3 = 不変条件テスト） ／
`clippy` **増分 0**（HEAD と同一の 52 件） ／ [scan_examples.ps1](scan_examples.ps1) **FAIL 0** ／
[compare_python_impl.ps1](compare_python_impl.ps1) **47 検査・47 一致**
（新例題は `AsyncManager` 未実装のため理由つきで `$knownDiff` へ登録＝37 件） ／
[repl_session.ps1](repl_session.ps1) **identical** ／ [debug_session.ps1](debug_session.ps1) **5 identical**
（#44 で直したので**今回から実際に効いている**）。
A/B の checksum が HEAD と一致することも確認した。

---

## #24 却下（2026-08-18）— peephole パターンの追加は**実行時効果ゼロ**と実測

計画書が挙げていた候補（到達不能コード除去・`Const;Pop` 消去）を実測し、**入れない判断**をした。
副産物として、**桁違いに大きい別の穴**（ループの後方 JUMP）を見つけたので #46 として起票する。

### 測り方: 出荷される命令列をそのまま数えた

`AR_VM_DUMP` は **peephole の後**に出る（`optimize` → `disassemble` の順）ので、
全例題 77 本のダンプを集めれば「**現状の peephole を通り抜けて残っている形**」がそのまま得られる。
Rust を 1 行も変えずに測れる。⇒ **2313 チャンク・16838 命令**を静的に集計。

| #24 の候補 | 静的な出現 | 実行時の効果 |
|---|---|---|
| `CONST; POP` | **4 箇所**（0.024%） | ほぼ 0 |
| `RETURN; RETURN_NIL`（到達不能） | 239 箇所（1.42%） | **0%**（到達不能＝定義上実行されない） |
| `JUMP; RETURN_NIL`（到達不能） | 85 箇所（0.51%） | **0%**（同上） |

⇒ **到達不能コード除去は「実行時間」には原理的に効かない**（減るのはコード量だけで、
2313 チャンクに対し 324 命令＝1 チャンクあたり 0.14 命令）。`Const;Pop` は出現自体が 4 件。
⇒ **#24 は却下**。#2a が残した「JUMP 除去だけで総命令の 0.31%」という数字は、
**残りのパターンを足しても改善しない**ことまで含めて確認できた。

### 🔑 副産物: **実行された命令**を数えたら、ループの後方 JUMP が 10〜12% あった

静的な出現数は判断材料として弱い（上位は「最上位文チャンクで 1 回だけ動く形」と
「一度も動かない形」で占められる）。⇒ VM のディスパッチループに**一時的な計測**を入れ、
**実行された op** を数え直した（判断がついたので計測コードは撤去済み）。

| ベンチ | 実行命令の総数 | うち無条件 `Jump` |
|---|---|---|
| `bench_arith` | 150,000,282 | **12.00%** |
| `bench_for` | 12,571,614 | **12.41%** |
| `bench_branch` | 171,000,218 | **10.53%** |
| `bench_method_call` | 16,007,566 | 6.25% |
| `bench_field_access` | 52,123,130 | 3.85% |

決定的なのは比率: `bench_arith` は `Jump` **18,000,000** に対し `JumpIfFalse` **18,000,006** で
**ちょうど 1:1**。⇒ **ループの 1 反復ごとに「条件判定の分岐」と「先頭へ戻る無条件 JUMP」を
両方払っている**。実際の生成コードもそうなっている（`bench_arith` の最内ループは 5 命令）:

```text
    4  IBIN_LL 1 0 Lt        ← 条件
    5  JUMP_IF_FALSE 9
    6  IBIN_LC 1 const[2] Add ← 本体
    7  STORE_LOCAL 1
    8  JUMP 4                 ← 後方 JUMP（これが消せる）
```

⇒ ループ反転（条件を末尾へ回して後方 JUMP を**条件分岐そのもの**にする）で
**5 命令 → 4 命令**にできる。⚠ ただし `JumpIfTrue` op が要る（現状は `JumpIfFalse` と
`*OrPop` 系しかない）ので **#24 の枠（既存 peephole へのパターン追加）を超える**。
⇒ **#46 として起票**した。**そして実装して測った結果、却下した**（下記）。

⚠ **見積もりの教訓**: 静的な出現頻度で最適化を決めてはいけなかった。静的 1 位は
`POP; RETURN_NIL`（1124 件・6.68%）だが、これは最上位文チャンクごとに**1 回**動くだけで、
`bench_arith` の 1.5 億命令の中では見えない。**「何個あるか」ではなく「何回動くか」**。

---

## #46 却下（2026-08-18）— ループ反転は**実装して測ったら 1.003x**（命令数 -20% でも時間は動かない）

#24 の計測で見つけた「実行命令の 10〜12% が後方 `Jump`」を潰しに行った。**実装は完了し
テストも全緑になったが、A/B の結果として捨てた**（#28 と同じ結末・コードは revert 済み）。

### 実装したもの（revert 済み・再挑戦するとき用）

`Op::JumpIfTrue` を新設し、`while` を**末尾条件型**へ組み替えた:

```text
   JUMP check       ← 入口で 1 回だけ
body: <本体>
check: <条件>
   JUMP_IF_TRUE body
end:
```

- 登録が要ったのは **5 箇所**: `op.rs` / `run.rs`（実行）/ `disasm.rs` /
  **`peephole::code_target_mut`**（#27-d の教訓）/ `compiler.rs::patch_jump`。
- `continue` の飛び先が**本体より後ろ**になるので、`LoopCtx.continue_target` を
  `Option<u32>` にして `None`（＝反転 while）は `break` と同じくバックパッチする形にした。
  `for` は `ForIter` 先頭が既知なので `Some(..)` のまま**触っていない**。
- ⚠ **文の先頭マークは入口 `Jump` ではなく条件へ付ける**。入口に付けるとデバッガが
  `while` 行で止まるのが**初回だけ**になる。条件に付ければ反転前と同じく毎反復止まる
  （入口 `Jump` は 1 回しか通らない）。

### 実測 — **3 本のバイナリで切り分けた**

#27 の「**op を足すだけでベンチが 0.88〜0.94x 動く**」を踏まえ、変換の効果を
opcode 追加のノイズから分離するため 3 本用意した:
**A**=HEAD ／ **B**=`JumpIfTrue` を足しただけ（**未使用**）／ **C**=反転まで実装。

| ベンチ | B/A（op 追加のノイズ床） | C/B（反転そのもの・2 回測定） |
|---|---|---|
| `bench_arith` | 0.996x | 1.010x → 1.012x |
| `bench_branch` | 1.001x | 1.007x → 1.013x |
| `bench_for` | 1.004x | 1.014x → 1.039x |
| `bench_method_call` | 1.005x | **0.973x → 1.007x（再現せず＝ノイズ）** |
| `bench_field_access` | 0.974x | 0.974x → 0.982x |

⇒ 今回は op 追加のノイズ床は**ほぼ中立**（#27 のときと違う）。反転の効果は
**while 支配ベンチで +1% 前後**にとどまった。

### 🔑 決定打: **最大効果ケースでも 1.003x**

「実命令が減っているのに時間が動かない」を確かめるため、**効果が最大になる形**
（空ループ `while i < n: i += 1`・1 反復 **5 命令 → 4 命令 ＝ -20%**）を測った。

```text
B: IBIN_LL; JUMP_IF_FALSE; IBIN_LC; STORE_LOCAL; JUMP   (5)
C: IBIN_LC; STORE_LOCAL; IBIN_LL; JUMP_IF_TRUE          (4)
```

⚠ **両バイナリの逆アセンブルを突き合わせて「本当に反転が効いている経路を測っている」ことを
先に確認した**（null 結果こそ「そもそも触っているか」を疑う・#43 の教訓の裏返し）。

結果 **1.003x**。⇒ **無条件 `Jump` はこの VM ではほぼ無料**（よく予測される分岐＋`ip` 代入だけで、
`exec_op` の呼び出しと実際の op の仕事に比べて無視できる）。

### ⚠⚠ 外れた前提: 「**実行命令の割合 ≒ 時間の割合**」

`Jump` は実行命令の **12%** だが、時間では **0.3%** だった。**命令には値段の差がある**。
⇒ 命令数の内訳は「どこを見るべきか」の**当たりを付ける**のには使えるが、
**「何 % 速くなるか」の予測には使えない**。#2a が
「JUMP の 14.3% を除去しても総命令の 0.31%」と書いていたのと同じ穴に、
**別の測り方（実行命令の内訳）で入り直しただけ**だった。

### 判断

**却下してコードを戻した**。理由は 3 つ:
1. 効果が **±5% のノイズ床に埋もれる**（計画書の「数 % で良し悪しを決めない」）。
2. **簡素化にならない** — 新オペコード 1 個＋`continue` のバックパッチ導入＋
   `while` と `for` でループ形状が恒久的に食い違う。計画書の
   「速度効果が小さくても簡素化が見込めるならメリット」の**逆**。
3. 最大効果ケースが 1.003x なので、**別のプログラムで大きく効く見込みも無い**。

⇒ **速度目的の残タスクは無くなった**。次に速度をやるなら、
`Jump` のような「安い命令」ではなく **`exec_op` 1 回あたりのコストそのもの**
（ディスパッチ方式）か、**高い命令**（`Call`・`GetAttr`・`Store*`）を狙うこと。

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
| #43（2026-08-18） | 「注釈で型が確定していれば検査を省ける」 | **省いてはいけなかった**（#15e: 注釈は意味論の根拠ではない。推論が外れた値を黙って通す）。費用の内訳を測ると**判定より文字列処理**が反復ごとに走っていたので、**検査は残して判定だけ速く**した ⇒ 検査を消した上限の **104%** を回収（＝実質無料）。**「省く」より「速くする」の方が安全かつ同等に速いことがある** |
| #42（2026-08-18） | 「モジュール本体を VM に載せるのは大仕事」 | **`Op::StoreName` を 1 個足すだけ**だった。宣言（`declare_var` → `scopes.last_mut()`）と読み（`LoadName`）は元からチェーンを見ていて、届かないのは代入だけ。⚠ **#39 で `vm_assign_global` を `scopes[0]` 限定へ厳格化しておいたおかげ**で、足りない部分が「たまたま動く」ではなく「届かない」という形で明示的に出た |
| #41（2026-08-18） | 「定義文脈を潰せば #33 が通る」 | **もう 1 つ消費者が残っていた** — import モジュール本体は `exec_module` が `push_scope` するので最上位 VM の対象外で、丸ごとツリーウォーク。#10-d が「モジュール本体は 20 文」として保留にした判断も**例題依存**だった（最上位に制御フローを持つモジュールの例題が 0 本）。**同じパターンは 5 回目** |
| #33（2026-08-18） | 「前提（34〜40）を全部潰したのでツリーウォークを削除できる」 | **削除できなかった**。クラスのフィールド既定値・`enum` 値のような**定義文脈の式**が `eval()` で評価され、その中の制御フローがツリーウォークで動いていた。しかも **`if`/`match` 式には計測フックが無く**、`tw_control_flow` 0 はその過小報告の上に乗っていた。⇒ **`force_gate` 0 件・`tw_control_flow` 0 は毎回「例題がその形を書いているか」に依存する**（#27/#34/#36/#33 で 4 回踏んだ） |
| #31（2026-08-17） | 「参照実装と突き合わせれば差分が出る」 | **`impl_python` が 100 コミット前に同期**されていた（`33ef765`）。差分 36 件のうち 5 件は「同期以降に Rust 側へ入った修正」で、**差分がある＝Rust のバグ ではない**。前提を書かずに網を作ると誤検知の山になる |
| #36（2026-08-17） | 「入口に配線を足すだけ。テスト 19 件が直る」 | 直ったのは **18 件で残り 1 件は静的検査の担当**だった。それより重要な副産物が 2 つ: **`force_gate` 0 件が「例題が必ず何かを宣言している」に依存**していた（最上位に宣言の無いプログラムは最上位丸ごとツリーウォーク）／**最上位 Chunk キャッシュが `Stmt` のアドレス**キーで、REPL が AST を捨てて**別文の Chunk を実行**していた。⇒ **負の対照が発火しないときは配線を疑う** |
| #39（2026-08-17） | 「`store_target` の `toplevel_globals` 門を外すだけ」 | **委譲先が間違っていた**。`Op::StoreGlobal` は `assign_var`（`scopes[frame_floor..]` を先に走査）へ委譲しており、**VM 関数は `scopes` を押さない**ので走査に映るのは**呼び出し元のローカル**。最上位では `scopes.len()==1` で偶然一致していただけで、関数本体へ広げた瞬間に健全性が壊れる形だった |
| #40（2026-08-17） | 「複製ごとにスタックの形が違うので一律の巻き戻しが書けない」＝#37 で保留にした | **`stmt_base` を複製の土台の分だけ持ち上げるだけ**で済んだ（引数 1 本）。既存の巻き戻しがそのまま「保留中の動作を破棄する」意味論を出した。⇒ **保留の理由が「難しい」だけのときは、一度は手を動かして確かめる** |
| #37（2026-08-17） | 「跳ぶ経路にも finally を出す＝新しい機構が要る」 | **#34 の `try_depth` を `try_stack` に一般化するだけ**で済んだ（新オペコード 0）。さらに **`loop_yield` は脱出ではなかった**（蓄積して先へ進む）のに `has_escape` が脱出扱いしており、`try: loop_yield i` が**丸ごと誤爆で bail** していた |
| #35（2026-08-17） | 「注釈を `BlockCtx` に持たせて検査 op を出すだけ」 | **どの注釈を見るか**が本体だった。`BLOCK_RETURN_EXPECTED_TYPE` を push するのは**式だけ**なので、`block:` 文は外側の注釈を継承し、かつ **`loop_yield` には透過**でなければならない。後者を見落として `for … ->list[int]: block: loop_yield i` が **`None`** になった |
| #34 の `try` | 「脱出は `has_escape` が既に弾いている」 | **`has_escape` は文しか歩かない**のでブロック式の中の `break` が素通りし、**ハンドラが残って後続の例外を横取り**した。しかも「跳ぶだけ」の例題では**症状が出ない**（跳んだ後に別の例外を投げて初めて分かる） |

| #38（2026-08-18） | 「`ReadToEnd` を `ReadToEndAsync` に替えるだけの 2 行」 | 直す前に**再現条件を作った**（`AR_VM_DUMP=1` で子が 7KB の stderr を吐く）ことで、同じ「計測手段が黙って嘘をつく」経路が**他に 3 つ**見えた（異常終了を min に混ぜる／無限待ち／不在パスを黙殺）。**普段の実行では 4 つとも発火しない**ので、通ったことは直った証拠にならない |

| #45（2026-08-18） | 「クロージャ生成の 1.05µs は大半が `body` のディープクローン」（#30 で自分が書いた） | **1 文の本体では 0.2µs しかない**（0.19µs/文の線形項なので、効くのは本体が大きいとき）。さらに本題は速度ではなく **`body.clone()` の意味が「複製」から「参照カウント加算」へ黙って変わる**こと（コンパイルが通るまま #15 の穴が復活する）。⇒ 価値の定義を「倍率」から「**本体サイズ依存を O(n)→O(1) にする**」へ置き換えた |

| #24（2026-08-18） | 「peephole パターンを足せば効く（効果は要実測）」 | **実行時効果 0% と確定**。到達不能コード除去は**原理的に速度へ効かない**（実行されないものを消すため）／`Const;Pop` は全例題で **4 件**。⇒ 却下。⚠ さらに **静的な出現頻度で決めかけた**のが危なかった — 静的 1 位の `POP;RETURN_NIL`（6.68%）は最上位文ごとに 1 回動くだけ。**実行された op を数え直したら**ループの後方 `Jump` が **10〜12%** あり、そちらが本命だった（→ #46） |

| #46（2026-08-18） | 「実行命令の 10〜12% を占める後方 `Jump` を消せば効く」（#24 の計測で立てた） | **1.003x**（最大効果ケース＝空ループで命令数 -20% でも）。**命令には値段の差がある** — `Jump` は命令の 12% だが時間の 0.3%。⇒ 実行命令の内訳は**当たりを付ける**のには使えるが**速度の予測には使えない**。実装・テスト全緑まで行ってから **#28 と同じく revert**した |

**教訓**: 着手前に診断フックで数字を取る。IR / バイトコードを実際にダンプして見る。

---

## #47 master ↔ byte-code の実行モード別 A/B 実測（2026-08-19）

**目的**: 本系列（Phase R + Phase V + VM 一本化）が、`master` に対して**実際に何倍になったか**を
3 つの実行経路（非コンパイル／コンパイル済み arrow native／C の DLL）ごとに切り分けて測る。
これまでの実測は**全部「変更前後の隣接 A/B」**で、系列全体の端点比較は取っていなかった。

### 計測条件
| 項目 | 内容 |
|---|---|
| A | `master`（`ecf9305`）の `--release` ビルド。`git worktree` を切って別ターゲットでビルド |
| B | `byte-code`（`50bb5c7`）の `--release` ビルド |
| 方式 | [ab_bench_modes.ps1](ab_bench_modes.ps1)（**#47 で新設**）。A,B,A,B… と**交互実行**・各指標 3 反復の **min** |
| 指標 | プロセス経過時間ではなく、スクリプトが出す `METRIC <name> <secs>`（起動・DLL ロード・リスト構築を計測から外す） |
| 健全性 | 各スクリプトが `CHECKSUM` を出し、**A と B で一致しなければ値を出さず警告**（「速い」ではなく「計算していない」を弾く） |
| ⚠ .arc | **測る側のバイナリで `--compile` し直してから**走らせる（.arc の形式がブランチ間で違う。master 91,644B ↔ byte-code 112,124B） |

計測用例題（新設）: [bench_ab_interp.ar](examples/bench/bench_ab_interp.ar) ／
[bench_ab_native.ar](examples/bench/bench_ab_native.ar) + [bench_ab_native_module.ar](examples/bench/bench_ab_native_module.ar) ／
[bench_ab_cdll.ar](examples/interop/bench_ab_cdll.ar)。

### 結果（A/B ＝ master が何倍遅いか）

**モード1: 非コンパイル（解釈実行）— 幾何平均 3.97x（15 指標・2.14〜5.90x）**

| 指標 | master (s) | byte-code (s) | A/B |
|---|---|---|---|
| baseline_empty | 0.1553 | 0.0341 | 4.56x |
| int_arith | 0.3417 | 0.0917 | 3.73x |
| float_arith | 0.3297 | 0.0617 | 5.35x |
| fn_call | 1.6488 | 0.3389 | 4.87x |
| for_range | 0.2827 | 0.0482 | 5.87x |
| branch | 0.4661 | 0.1228 | 3.80x |
| field_access | 0.7450 | 0.2275 | 3.28x |
| method_call | 1.5711 | 0.2661 | 5.90x |
| new_object | 2.4282 | 0.8875 | 2.74x |
| closure_call | 1.0324 | 0.1984 | 5.20x |
| list_index | 0.3209 | 0.1023 | 3.14x |
| str_ops | 0.3032 | 0.1418 | 2.14x |
| dict | 0.3418 | 0.1484 | 2.30x |
| try | 0.1663 | 0.0326 | 5.11x |
| block_expr | 0.4735 | 0.1048 | 4.52x |

**モード2: コンパイル済み arrow native — 純ネイティブ 1.00x／境界・コールバック 1.60x**

| 指標 | 分類 | master (s) | byte-code (s) | A/B |
|---|---|---|---|---|
| hot_int（native 内ループ） | 純ネイティブ | 0.1253 | 0.1273 | 0.98x |
| hot_float（native 内ループ） | 純ネイティブ | 0.0431 | 0.0441 | 0.98x |
| sum_fixed（フラット GEP 反復） | 純ネイティブ | 0.0787 | 0.0751 | 1.05x |
| call_noop（解釈→native ディスパッチ） | 境界 | 0.2526 | 0.1288 | 1.96x |
| call_id1（同・引数 1 本） | 境界 | 0.3479 | 0.2265 | 1.54x |
| sum_list_cb（native→解釈 CB_ITER） | コールバック | 1.8636 | 1.0585 | 1.76x |
| apply_fn_cb（native→解釈 function 引数） | コールバック | 0.2496 | 0.1864 | 1.34x |
| setup_build_list（参考・解釈側の構築） | 解釈 | 0.3649 | 0.2496 | 1.46x |

⇒ **純ネイティブ 3 指標が 1.003x（幾何平均）**なのが本計測の**負の対照**。
codegen は本系列で触っていないので変わらないのが正しく、**計測手順が正しいことの証明**でもある。
native 経路の改善は**全部「解釈側に戻ってくる部分」**（呼び出し境界・コールバック）から来ている。

**モード3: C の DLL（`import[cpp-lib]`）— 幾何平均 2.18x（5 指標）**

| 指標 | master (s) | byte-code (s) | A/B |
|---|---|---|---|
| baseline_loop（fn 内・FFI なし） | 0.0828 | 0.0140 | 5.90x |
| v3_add（fn 内・構造体ポインタ 3 本） | 0.2114 | 0.1263 | 1.67x |
| v3_add_fresh（毎回 V3 を作り直す） | 2.1926 | 0.9438 | 2.32x |
| baseline_toplevel（最上位・FFI なし） | 0.0606 | 0.0375 | 1.62x |
| v3_add_toplevel（最上位・FFI あり） | 0.2005 | 0.1513 | 1.33x |

⚠ **baseline を引いた「FFI 1 回あたり」は 1.15〜1.23x しかない**
（fn 内 428.9→374.2 ns ／ 最上位 466.2→379.4 ns）。
⇒ **C DLL 呼び出しが速くなったのではなく、その周りの解釈実行が速くなっただけ**。
FFI 経路（引数マーシャリング・ハンドル表・書き戻し）は本系列でほぼ手つかず。

⚠ **最上位の baseline は 1.62x しか出ていない**（fn 内は 5.90x）。
最上位 Chunk（#36/#41）は関数本体ほど最適化されていない ＝ **同じコードでも書く場所で 3.6 倍違う**。
速度目的で最上位を触るなら、まずここを測ること。

### クロスチェック（既存ベンチ 14 本・**プロセス全体の経過時間**）

新設ベンチだけを信じないため、[ab_bench.ps1](ab_bench.ps1) で既存ベンチも回した（Reps=2・min）。
**幾何平均 3.66x**（2.64〜5.98x）で、新設ベンチのモード1（3.97x）とほぼ一致した。

| script | A(min) | B(min) | A/B |  | script | A(min) | B(min) | A/B |
|---|---|---|---|---|---|---|---|---|
| bench_arith | 5.129 | 1.335 | 3.84x | | bench_collections | 15.783 | 5.666 | 2.79x |
| bench_for | 0.684 | 0.198 | 3.46x | | bench_control_flow | 14.180 | 2.370 | 5.98x |
| bench_branch | 5.020 | 1.562 | 3.21x | | bench_block_expr | 3.545 | 0.872 | 4.06x |
| bench_method_call | 1.938 | 0.524 | 3.70x | | bench_string | 1.740 | 0.659 | 2.64x |
| bench_method_hot | 17.189 | 3.853 | 4.46x | | bench_name_hash | 1.304 | 0.467 | 2.79x |
| bench_field_access | 6.261 | 1.519 | 4.12x | | flat_bench_interp | 4.018 | 0.860 | 4.67x |
| bench_closure | 1.148 | 0.331 | 3.47x | | bottleneck_bench | 13.419 | 4.018 | 3.34x |

### 副産物 — **byte-code 側の実バグを 1 件検出した**（速度計測とは別件）

[cpp_struct_ptr.ar](examples/interop/cpp_struct_ptr.ar) の `v3_norm`（`double*` out 引数 → `mut float` 変数への
書き戻し）が、**master は `5.0`・byte-code は `0.0` を黙って返す**（最上位・関数内の両方）。

原因（**当初 `assign_var` と `vm_stack` の不一致だと書いたが誤り**。`AR_VM_DUMP` で確かめ直した）:
書き戻しは**引数の AST 式**（`CallArg` が `Expr::Ident` か）を見て初めて登録される
（[native.rs:129-133](src/interpreter/eval/native.rs#L129) typed OutPtr ／
[native.rs:196-201](src/interpreter/eval/native.rs#L196) ハンドル経路 MutPtr）。
VM の `Call` / `CALL_METHOD` は**評価済みの値**をオペランドスタックで渡すので
`call_value_evaled` → `dispatch_native_evaled` に落ちる。この経路は
「**CallArg 情報がなく named-mut 判定ができないため書き戻しを行わない**」と
[native.rs:443-446](src/interpreter/eval/native.rs#L443) / [calls.rs:698-700](src/interpreter/eval/calls.rs#L698)
に**設計として明記**されている。master ではツリーウォークの `eval_call` が
`call_native_function`（式を持つ経路）を通っていたので書き戻しが効いていた。
⇒ **#33 で解釈経路を VM 一本にした結果、式を持つ経路が通常実行から消えた**のが本当の原因。

⚠ 通ったのは「安全側に倒している」と書かれた分岐で、**倒れた先が黙って間違った値**だった。
⚠ 構造体 out 引数（`V3*`）はゼロコピーで同一 `InstanceData` に書くので影響を受けない（`5 7 9` は正しい）。
**壊れるのはプリミティブ out 引数（`double*` 等の typed OutPtr）と、ハンドル経路の MutPtr 書き戻し**。
現行例題でこの形を踏むのは [cpp_struct_ptr.ar](examples/interop/cpp_struct_ptr.ar) だけ。

⚠ **既存ゲートは全部緑のまま素通りした**: `scan_examples` / `force_gate` は **exit code しか見ない**、
`compare_python_impl` は **cpp-lib 例題を対象にしていない**（参照実装が C を呼べない）。
⇒ 計画書の「**検査網は例題が踏む形しか見ない**」の **6 例目**。しかも今回は形ではなく
「**値を見ていない**」という別の穴だった。**FFI の戻り値・書き戻しを検査するゲートが存在しない。**

**残タスクでは解消しない**（#19 / #17-b / #17-a / #14 / #11 R2-c のいずれも実行時ディスパッチに触れない。
#17-a は話題こそ C/C++ 相互運用だが**静的型検査だけ**の変更）。⇒ **新タスク #48 として起票が要る**。
修正の方向: VM は**コンパイル時に引数式を持っている**（`Expr::Ident` → slot / グローバル）ので、
`has_writeback` な native 呼び出しでは書き戻し先の slot を呼び出し点に載せ、
`dispatch_native_evaled` が返した out 値を呼び出し後に `STORE_LOCAL` / `STORE_GLOBAL` する
（＝ツリーウォークが名前でやっていたことを slot でやる）。
⚠ **bail は選択肢にならない**（#33 でフォールバックを撤去したので `VmForceError` で止まる）。

### 見積もりと実測

| 事前の見立て | 実測 |
|---|---|
| 「解釈実行は速くなった」（隣接 A/B の積算） | **3.97x**（幾何平均・15 指標）。クロスチェック 3.66x と一致 |
| 「native 経路は codegen を触っていないので無関係」 | **半分正しい**。純ネイティブは 1.00x だが、**境界とコールバックは 1.60x**（解釈側に戻る部分がある） |
| 「C DLL 呼び出しも速くなった（2.18x）」 | **誤り**。baseline を引くと **FFI 自体は 1.15〜1.23x**。速くなったのは周りの解釈実行 |
| 「最上位も #36/#41 で VM に載ったので同じ速度」 | **1.62x**（fn 内 5.90x）。**載っている ≠ 同じ速さ** |
| 検出したバグの原因は「`assign_var` が `vm_stack` に届かない」 | **誤り**。`AR_VM_DUMP` で見たら最上位の `n1` は `DECLARE_GLOBAL`/`LOAD_GLOBAL`＝`scopes[0]` にあり、`assign_var` なら届く形だった。真因は**書き戻しの登録自体が引数 AST を要求する**こと。⇒ **「届かない」と決める前に、その値がどこに置かれているかをダンプで確かめる** |

---

## #48 native の `mut` ポインタ書き戻しを VM 経路で復旧（2026-08-19）

#47 で検出した実バグの修正。**`import[cpp-lib]` / `import[cpp-dll]` のプリミティブ out 引数
（`double*` 等）と、ハンドル経路の `MutPtr` 引数の書き戻しが、VM 経路では黙って起きなかった。**

### 原因（#47 の記述を実測で確定させた）
書き戻し先は**引数の AST 式**が `Expr::Ident` かどうかで決まる
（[native.rs:129](src/interpreter/eval/native.rs#L129) typed OutPtr ／
[native.rs:196](src/interpreter/eval/native.rs#L196) ハンドル経路 MutPtr）。
VM の `Call` / `CallMethod` は**評価済みの値**をオペランドスタックで渡すため
`call_value_evaled` → `dispatch_native_evaled` に落ち、この経路は
「`CallArg` 情報がなく named-mut 判定ができないため書き戻しを行わない」と**設計として明記**されていた。
⇒ **#33 で解釈経路を VM 一本にしたときに、式を持つ経路が通常実行から消えた**のが真因。

### 手法 — 「判るときに決めて副表へ置く」
引数式は**コンパイル時には有る**ので、そのとき書き戻し先を確定して Chunk の副表に載せる。

| 層 | 変更 |
|---|---|
| [chunk.rs](src/vm/chunk.rs) | `WbCall { mask, targets }` / `WbStore { Local, Cell, Global, Name }` と `Chunk::wb_targets` |
| [compiler.rs](src/vm/compiler.rs) | `wb_store_target`（`store_target` と同順・同記憶域だが **bail せず `None`**）＋ `record_wb_targets`。`compile_call_args` に `wb_node` を追加して全呼び出し形から通す |
| [eval/native.rs](src/interpreter/eval/native.rs) | `dispatch_native_evaled_wb(fn_ref, vals, wb_mask, wb_out)`。**既存の `dispatch_native_evaled` は mask=0 での委譲に畳んだ** |
| [value/native.rs](src/interpreter/value/native.rs) | `NativeFnRef::has_writeback()`（ツリーウォーク側の判定もこれに統一） |
| [vm_toplevel.rs](src/interpreter/vm_toplevel.rs) | `vm_namespace_writeback_fn`（`mod.func` の呼び先を先に同定。外れたら従来経路） |
| [run.rs](src/vm/run.rs) | `native_call_with_wb` / `wb_native_method` / `apply_writeback`（いずれも `#[inline(never)]`） |

**キーは node_id**（`ffi_call_info` と同じ）。code index にすると
[peephole](src/vm/peephole.rs) が命令を詰めた瞬間にずれる。
`static mut` だけは対象外（`static_cells` を span キーで直読みする別経路。実例が無い）。

副産物: **シャドウ変換が要る構造体引数の書き戻しも直った**。VM 経路は
`resolve_typed_ptr_arg` の `named_mut` に常に `None` を渡していたので、レイアウトが
完全一致しない構造体は書き戻されていなかった（`cpp_struct_ptr.ar` はゼロコピーが
効く形なので露見していなかった）。

### ⚠⚠ 一度目の実装で 0.91x の退行を出した（#10-b と同じ失敗）
最初は書き戻しの判定と本体を **`Op::Call` のアームに直接書いた**。`exec_op` は
`#[inline(always)]` なので、`wb_targets.is_empty()`／ハッシュ引き／`has_writeback()`
（`ptr_params` の線形走査）がホットループへ展開され、**native を 1 度も呼ばない Chunk**まで遅くなった:

| 指標 | 1 回目 | 2 回目 | 判定 |
|---|---|---|---|
| `interp_fn_call` | 0.924x | 0.906x | **再現＝実費用** |
| `interp_closure_call` | 0.918x | 0.917x | **再現＝実費用** |
| `interp_for_range` | 0.921x | 1.072x | 揺れ |
| `interp_field_access` | 0.920x | 0.998x | 揺れ |

⚠ **前提が外れていた**: 「副表が空なら素通りするので実質無料」と考えていたが、
`f(mut x)`（例: `leaf(i)` の `i` は `mut` ローカル）は普通に書かれるので
**副表はまず空にならない**。「稀な形だから安い」は**書いてみて測るまで判らない**。

修正: **`exec_op` に残すのは discriminant 1 個だけ**にし
（`Op::Call` は `matches!(callee, Value::NativeFunction(_))`、`CallMethod` は
`matches!(obj, Value::Namespace(_))`）、残りの判定と本体を `#[inline(never)]` へ出した。

| 指標 | 修正後 1 回目 | 修正後 2 回目 |
|---|---|---|
| `interp_fn_call` | 1.002x | 0.980x |
| `interp_closure_call` | 0.964x | 0.976x |
| **interp 幾何平均** | **0.996x** | **0.993x** |

⚠ `interp_str_ops` だけ 0.900x / 0.934x と両方で低い（修正前は 1.015x / 1.004x）。
ただし**ホットループに `Op::Call` が 1 つも無い**ので機序が無く、独立の
[bench_string.ar](examples/bench/bench_string.ar) は **0.987x**。⇒ **コード配置の揺れ**
（#28 の「効くのはアーム数ではなくコード配置」と同じ）と判断した。

**cdll は逆に速くなった**（`Value::NativeFunction` を `call_value_evaled` を経ずに
直接ディスパッチするため）: `cdll_v3_add` **1.131x** ／ `cdll_v3_add_toplevel` **1.157x**（再現あり）。
native モードは負の対照（純ネイティブ `native_sum_fixed`）自身が 1.015x → 0.958x と動いたので、
**その帯の中**（再現する退行なし）。

### 検証
| ゲート | 結果 |
|---|---|
| `cargo build` | 警告 0 |
| `cargo test` | **740 passed** |
| `cargo clippy` | 変更ファイルの指摘 **0 件**（増分 0） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL 0 |
| [force_gate.ps1](force_gate.ps1) | `VmForceError` **0 件** / **147 例題**完走 |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **49/49 identical** |
| [repl_session.ps1](repl_session.ps1) | identical |
| [debug_session.ps1](debug_session.ps1) | **赤（HEAD で既に赤・#49）**。修正前バイナリと出力が **5/5 byte-identical** なので #48 の影響ではない |

回帰検知は [cpp_out_param_writeback.ar](examples/interop/cpp_out_param_writeback.ar)（新設）。
`WbStore` の全アーム（関数内ローカル／最上位グローバル／クロージャの可変捕捉セル／ループ反復）を
1 本ずつ踏み、**期待値と違えば `raise` する**。
⚠ **負の対照を確認済み**: 修正前バイナリで走らせると `exit=1`（`ValueError: local: got 0.0, want 5.0`）。
さらに `compare_python_impl` の `$knownDiff` から `cpp_struct_ptr` が**「stale＝もう一致する」と
報告された**（参照実装との突き合わせによる独立確認）。

### 見積もりと実測
| 事前の見立て | 実測 |
|---|---|
| 「書き戻しは `assign_var` が VM のローカルに届かないのが原因」（#47 で最初にそう書いた） | **誤り**。最上位の変数は `DECLARE_GLOBAL`/`LOAD_GLOBAL` で `scopes[0]` にあり `assign_var` なら届く。真因は**登録自体が引数 AST を要求する**こと。⇒ **「届かない」と決める前に `AR_VM_DUMP` でどこに置かれているか見る** |
| 「副表は稀にしか引かれないので `Op::Call` に判定を置いても無料」 | **0.91x の退行**。`f(mut x)` は普通に書かれるので副表は空にならない。⇒ #10-b の教訓は「重い本体を書かない」だけでなく「**安いつもりの判定も測る**」 |
| 「FFI 経路には触らないので cdll の速度は変わらない」 | **1.13〜1.16x 速くなった**（`call_value_evaled` を経由しなくなったため）。修正のついでに経路が 1 段短くなった |

---

## #49 `debug_session.ps1` の stdin BOM 混入を解消（2026-08-19）

#48 の検証中に「[debug_session.ps1](debug_session.ps1) が HEAD で既に 5/5 赤い」と判明した件。
**ゲートスクリプト側の欠陥**で、インタプリタは無関係だった。

### 症状と切り分け
デバッガ REPL の **1 行目が `?<BOM>` に化けてコマンドが 1 つずれ**、以降の transcript が全部ずれる。
`  [dbg] Error: unexpected token: ` + `?﻿` が stderr に出る（生バイトは `3F EF BB BF`）。

切り分けの順序（**「そもそも自分のせいか」を最初に潰した**）:
1. `.in` / `.ar` / golden に BOM は無い（`xxd` で確認）。
2. **修正前バイナリ（#48 前）で走らせても出力が 5/5 byte-identical** ⇒ #48 とは無関係・HEAD で既に赤い。
3. stdin を `q
1+1
q
` に置き換えても再現 ⇒ `.in` の内容に依らない。
4. `[Console]::InputEncoding` を見ると **cp65001・preamble `EF BB BF`**。

### 原因
.NET Framework の `Process.Start` は `RedirectStandardInput` のとき
**`StandardInput` の `StreamWriter` を `[Console]::InputEncoding` で作り、`AutoFlush = true` を立てる**。
`AutoFlush` の setter は `Flush()` を呼び、`StreamWriter.Flush` は
**まだ書いていなければ preamble を書く**。⇒ **`Start()` が返った時点で子の stdin に BOM が入っている**。

⚠ スクリプトは既に「BOM を避ける」つもりで `$proc.StandardInput.**BaseStream**` へ
`UTF8Encoding($false)` の writer を作って書いていた。**これでは遅い** —
BOM は自分が書く**前**に入っており、`BaseStream` へ書いた内容はその後ろに continue する。

### 手法
子を起こす直前だけ `[Console]::InputEncoding` を **preamble 無し**の `UTF8Encoding($false)` に
差し替え、`finally` で必ず戻す（`Invoke-Debug` の中にスコープ）。
⇒ **golden は 1 行も変えずに 5/5 identical**（`-Update` を一切使っていない）。
＝ **#44 の golden は最初から正しく、壊れていたのは入力の与え方だった**。

### ⚠ 同じ罠を隣のスクリプトが**別の手で**避けていた
[repl_session.ps1](repl_session.ps1) は `cmd /c "exe --repl < file"` の**ネイティブリダイレクト**で
stdin を与えており、マネージド writer 自体が作られないので原理的に踏まない。
しかもコメントに「PS5.1 の Process.StandardInput は UTF-8 BOM を先頭に書いてしまい、
REPL が ParseError: unexpected token になる」と**症状まで書いてあった**。
⇒ **知識はリポジトリ内にあったのに、隣のスクリプトへ渡っていなかった**（相互参照が無かった）。
#49 で両方にクロスリファレンスを入れた。
⚠ `debug_session` が cmd 方式を採れないのは **stdout と stderr を分けて受ける**必要がある
（`--stderr--` 区切り）＋タイムアウト時に `Kill()` したいため。手当てが違ってよい。

### ⚠⚠ このゲートは「黙って赤くなる」癖がある（これで 2 度目）
| 回 | 期間 | 原因 | 見つかり方 |
|---|---|---|---|
| 1 回目 | `6bf039c`〜`7aea0e5` | golden が BOM 修正前のまま（#33 partial が修正と golden を同じコミットに入れた） | #44 で発見 |
| 2 回目 | #44 以降〜#49 | **環境側**（コンソールのコードページが 65001 になり `InputEncoding` に preamble が付いた） | #48 の検証で発見 |

⚠ **2 回目は src も golden も無関係**＝ **コミットを遡っても原因が出ない種類**の赤。
発火はマシン依存なので、**別の環境では緑のまま**通ってしまう。
⇒ 計画書の「ゲートは自分で走らせて緑を確かめる」に、**「緑だった環境と同じとは限らない」**が加わる。

### 検証
- `debug_session.ps1` **5 identical**（2 回連続・golden 無変更）
- `[Console]::InputEncoding` の preamble が実行前後で **3 → 3**（復元されている）
- `repl_session.ps1` identical（巻き添えが無いこと）

### 見積もりと実測
| 事前の見立て | 実測 |
|---|---|
| 「`.in` に BOM が混じっているのだろう」 | **違った**。入力ファイルは全部クリーンで、BOM は `Process.Start` が書いていた |
| 「`.BaseStream` へ BOM 無しで書けば防げる」（スクリプトの既存コメントの前提） | **防げない**。preamble は `Start()` の時点で既に書かれている ⇒ **対策は「後から正しく書く」ではなく「writer に preamble を持たせない」** |
| 「goldens を録り直す必要がある」 | **不要だった**。goldens は正しく、入力が壊れていた。⇒ **赤いゲートを見たら `-Update` の前に「入力が意図どおり届いているか」を疑う** |

---

## #50 非コンパイル実行の**実行時間分布**を実測（2026-08-19）

「VM 化で解釈実行は 3.97x（#47）なのに、`master` と比べた**体感の速度向上が限定的**」という
問いに答えるための計測。**推測せず、時間がどこに行っているかを 2 軸で実測した**。

### 手法 — 計測フックを新設（`--features prof` / [src/prof.rs](src/prof.rs)）

| 軸 | 何を測るか | 方式 |
|---|---|---|
| 軸1 段別 | startup / lex / parse / type_check / resolve / interp_init / exec / teardown | `Instant` 直接計測（段は数回しか通らない） |
| 軸2 op 別 | **exec の中でどの op に何 ms 居たか** | **統計サンプリング**（ディスパッチループが relaxed store で「今の op」を置き、別スレッドが 20µs 間隔で読む） |

⚠ **op ごとに時計を読む方式は採らなかった。** 安い op ほど相対誤差が大きくなり、
「命令には値段の差がある」（#46）という**肝心の量が歪む**。サンプリングなら滞在時間に比例する。
⚠ 既定ビルドでは**コードごと消える**（#10-a の規約）。`AR_PROF=1`（段のみ）/ `AR_PROF=ops`（段＋op）。
実行は [prof_dist.ps1](prof_dist.ps1)（新設）。

### 計測の妥当性検査（先に潰した 3 件）

1. **計測ビルドの下駄** — `bench_ab_interp.ar` を plain と交互実行して **0.95x**（15 指標の個別も
   0.94〜1.03x）。**系統的な遅化なし**＝分布は代表性がある（#27 のノイズ床 ±5〜12% の内側）。
2. **⚠ 1 回目の実行はファイルのコールドリードを踏む** — `startup` が **0.1ms → 10ms** に化けていた
   （最初の集計は全部これに汚染されていた）。⇒ 2 パス走らせて 2 パス目だけ採る。
3. **⚠⚠ 帰属バグを 1 件出した（負の対照で発見）** — `event_handler.ar` の **1 秒の待ちが丸ごと
   `ReturnNil` に化けていた**（VM フレームが返った後も `CUR` が最後の op のまま）。
   ⇒ `run()` の入口に **`CurGuard`（抜けるとき呼び出し元の op へ戻す）** を置いて解消。
   修正後は同例題が `CallMethod`（＝実際にブロックしている呼び出し）99.95% になった。
   ⚠ **「ReturnNil が 72%」という一見それらしい結果**が出ていた。もっともらしい分布ほど裏を取る。
4. **帰属の独立検証** — 5,000,000 回の関数呼び出しだけをするマイクロベンチで、
   解析値（`call − empty` の差分＝79.8%）と サンプリング値（`Call`+`Return`+`LoadGlobal` = 67.8%、
   残りは呼び先本体とループ制御）が整合。**1 呼び出し ≈ 275ns / うち `Call` op 自体が ≈ 213ns**。

### 結果1 — 段別（79 例題・warm・2 パス目）

| 対象 | arrow の実費用 | うち exec | 端点 A/B の上限（exec を 3.97x にしたとき） |
|---|---|---|---|
| 全 79 例題（合計） | 43.9 s | **91.9%** | 3.73x |
| bench 系 21 例題 | 38.3 s | **97.7%** | 3.90x |
| **bench 以外 58 例題** | 5.57 s | **51.9%** | **2.54x** |
| **例題 1 本ごとの中央値** | 3.40 ms | **exec 0.46 ms（14%）** | **1.59x**（p25 1.30x） |

短命スクリプト 1 本の中央値内訳（ms）:
`プロセス生成・イメージロード・終了 1.5` ／ `interp_init 0.32` ／ `type_check 0.175` ／
`parse 0.171` ／ `startup 0.145` ／ `teardown 0.127` ／ `lex 0.083` ／ `resolve 0.019` ／ **`exec 0.46`**。

⇒ **⚠⚠ 「VM を無限に速くしても中央値の例題は 1.6x 弱しか速くならない」**。
高速化したのは **exec だけ**で、それ以外（プロセス費用・parse・type_check・interp_init）は
`master` と同じまま残っている。**体感が伸びない理由はここ**。
⚠ 例外は **import が重いスクリプト**（`event_external_handler.ar` は **parse が 2320ms＝全体の 98%**。
`import` はパース時にモジュールを実行するため）。

⚠ プロセス費用の内訳は別に取った（min・15 回）: `cmd /c exit` **8.63ms**（＝計測側の床）／
`arrow.exe` 引数なし **9.96ms** ⇒ **arrow 自体のプロセス費用は ≈ 1.4ms**、
短命スクリプトの `wall − in_main` の実測中央値 **1.56ms** と整合。

### 結果2 — exec の中の op 別（統計サンプリング）

**純 Arrow 解釈実行 72 例題**（FFI ベンチ・ブロッキング待ちを除く・exec 計 28.2 s）:

| グループ | 割合 | 主な op |
|---|---|---|
| 算術・比較・型判定 | 22.3% | `IntBinLC` `IntBinSS` `IntBinLL` |
| **呼び出し機構 `Call`** | **19.9%** | `Call` |
| **メソッド呼び出し** | **16.3%** | `CallMethodLocal` `CallMethod` |
| ローカル・定数・スタック | 13.2% | `LoadLocal` `StoreLocalFreezeInstance` `StoreLocal` |
| コレクション・反復 | 10.5% | `BuildTuple` `Subscript` `ForIter` |
| 属性 get/set | 6.6% | `GetAttrLocal` `SetAttr` |
| 分岐 | 4.1% | `JumpIfFalse` `Jump` |
| グローバル/名前引き | 3.9% | `LoadGlobal` `StoreGlobal` |
| 組み込み関数 / VM の外 / 復帰 / 例外 | 1.2 / 1.1 / 1.0 / 0.0% | |

⇒ **呼び出し系だけで 37〜38%**（`Call` 19.9 ＋ メソッド 16.3 ＋ 組み込み 1.2）。
**`exec_op` 1 回あたりのコストではなく「高い命令」を狙え**（#46 の指針どおり）という結論を
**実測で裏付けた**形。⚠ `Jump`+`JumpIfFalse` は合わせて **4.1%** しかない。

代表ワークロード別（exec に占める上位 op）:

| 例題 | 支配項 |
|---|---|
| `bench_ab_interp.ar`（#47 の基準） | `Call` **32%** ／ `IntBinLC` 7 ／ `SetAttr` 6 ／ `IntBinLL` 6 |
| `bench_method_hot.ar` | `CallMethodLocal` **70%** ／ `GetAttrLocal` 15 |
| `bottleneck_bench.ar` | `Call` **58%** ／ `LoadGlobal` 13 |
| `bench_field_access.ar` | `Call` **58%** ／ `FloatBinSS` 8 ／ `GetAttrLocal` 8 |
| `bench_collections.ar` | `BuildTuple` 21 ／ `Subscript` 14 ／ `Call` 13 |
| `partial_call_overhead.ar` | `(native_callee)` **75%**（＝FFI 本体。VM の費用ではない） |

⚠ **FFI ベンチを混ぜると絵が変わる**（全 79 例題だと `(native_callee)` が 16.9% で 3 位に来る）。
`Op::Call` の native 分岐に専用バケットを置いて **呼び出し機構と呼び先本体を分離した**。

### 見積もりと実測

| 事前の見立て | 実測 |
|---|---|
| 「exec が 3.97x なら全体もそれに近いはず」 | **違った**。例題 1 本の中央値で **1.59x**（exec は中央値で全体の 14%） |
| 「短命スクリプトは parse が支配的だろう」 | **概ね違った**。parse 0.171ms より `interp_init` 0.322ms の方が大きく、最大項は**プロセス費用 1.5ms** |
| 「`ReturnNil` が 72% という結果」 | **計測バグ**。ブロッキング待ちの誤帰属だった（#50 の `CurGuard` で解消） |
| 「命令は満遍なく効いている」 | **違った**。呼び出し系 3 種で **37〜38%**、分岐は 4% |

---

## #51 doc/属性の迷子と陳腐化コメントの一掃（2026-08-19）

2026-08-19 の**全 src 診断**（66,107 行・非テスト関数 1,136 個を機械スキャン＋目視）で起票した
保守性レーンの 1 本目。**速度目的ではない**が、**実バグを 1 件含んでいた**。

### 起点 — コンパイラが黙っている 3 種類の壊れ方

| 種別 | 件数 | なぜテストで見えないか |
|---|---|---|
| **orphan doc / 属性の迷子** | 3 | doc コメントも属性なので**構文的に正しい**。位置がずれても通る |
| 存在しない識別子への参照 | 37 識別子 / 61 参照 | コメントは検査されない |
| 死んだ enum バリアント | 1 | `#[allow(dead_code)]` が警告を止めていた |

### ⚠⚠ 実バグ — 「外すな」と書いた属性が黙って外れていた

[vm_toplevel.rs](src/interpreter/vm_toplevel.rs) で、#48 が `vm_namespace_writeback_fn` を
**`vm_method_call_other` の doc と `#[inline(never)]` の間に挿入**していた。結果:

```rust
/// ⚠ **`#[inline(never)]` を外さないこと**（`exec_op` は `#[inline(always)]`）。   // ← A の doc
#[inline(never)]                                                                  // ← A の属性
/// `mod.func(...)` の呼び先が … （#48）                                            // ← B の doc
pub(crate) fn vm_namespace_writeback_fn(...)   // ← doc も属性も全部 B に付く
```

⇒ `vm_method_call_other`（`exec_op` から呼ばれる非 Instance レシーバ経路）は
**doc も `#[inline(never)]` も失っていた**。doc 自身が「実測でここを経由させると 3% 落ちた」と
書いていた関数で、**#10-b の「op のアームに重い本体を書かない」を破りうる状態**だった。
逆に 8 行の `vm_namespace_writeback_fn` に不要な `#[inline(never)]` が付いていた。

**同じ形をあと 2 件見つけた**:
- [compiler.rs](src/vm/compiler.rs): `store_target` の doc が、後から挿入された `slot_of` に付いていた
  （`slot_of` は `Option<u16>` を返すのに doc は「`Local`/`Global`/`None` を返す」と書いてある）。
- [compiler.rs](src/vm/compiler.rs): **削除済み `has_escape` の doc**（`include_return` 引数の説明）が
  `const MAX_FINALLY_NEST` の doc に前置されたまま残っていた。

⇒ **教訓: 関数を既存の doc ブロックの「下」へ挿入しない。消すときは doc も一緒に消す。**

### 陳腐化コメント — 特に有害だった 2 件

1. **指示が真逆に矛盾していた**。[interpreter.rs](src/interpreter.rs) は
   「`resolver::toplevel_visible_globals` の結果をそのまま渡すこと（判定を複製しない）」と
   書いていたが、[main.rs](src/main.rs) は「**リゾルバ用の `toplevel_visible_globals`
   （シャドウ減算あり）ではない**。減算するとむしろ解決できる名前を落とす」と書いていた。
   しかも `toplevel_visible_globals` という名前は**もう存在しない**（正は
   `toplevel_declared_globals`／減算版は `toplevel_visible_globals_with`）。
   `interpreter.rs` に従うと **#27-c の実バグが再発する**。
2. **#36 で潰した穴の説明が残っていた**。`toplevel_globals` の doc に
   「**空 = 最上位 VM 化を行わない**」とあったが、#36 でまさにその条件を削除している。
   復活させると「最上位に宣言が 1 つも無いプログラムが丸ごとツリーウォーク」に戻る。

サブシステムのヘッダも全滅だった: [vm/mod.rs](src/vm/mod.rs) と [compiler.rs](src/vm/compiler.rs) は
**削除済みのデュアルモード**（「ツリーウォークにフォールバックする」「`Interpreter::vm_mode`
（Off/Auto/Force）で制御する」「V-A の対応範囲＝トップレベル関数のリーフ計算に限定」）を
説明したままで、**新規参加者が最初に読む 11 行が全部古い**状態だった。

### 死んでいたもの

- `ExecResult::Return` — **構築も match も 0 件**（`Normal` 41 / `Raise` 9）。削除。
  ⚠ 削除したら `modules.rs` の `_ => {}` 2 箇所が unreachable になった
  （＝`#[allow(dead_code)]` が**警告を 2 件も隠していた**）。
- `Chunk::local_names` と `disasm::{disassemble, fmt_op}` の `#[allow(dead_code)]` —
  **どちらも実際には消費されている**（前者は debugger.rs、後者は `AR_VM_DUMP`）。
  付けたままだと**本当に死んだときに警告が出ない**ので外した。

### 再発防止 — [stale_doc_refs.ps1](stale_doc_refs.ps1)（新設）

src のコメント内 `` `識別子` `` がコードに存在するかを検査するゲート。
⚠ **履歴として正しい言及を落とす仕掛けが要る**（「`exec_for_stmt` は #33 で削除した」は正しい記述）。
同じ行に「削除／廃止／撤去／以前／旧／移設／だった／していた」のマーカー語があれば履歴扱いにする。
⇒ **消えたものに言及したいときはマーカー語を書く**、が運用規約になる。
外部成果物（`.ps1` 名・ベンチの METRIC 名・生成 C シンボル）は `$whitelist`。

61 参照 → **0 件**（履歴・外部参照 38 件は除外して報告）。

### 見積もりと実測

| 事前の見立て | 実測 |
|---|---|
| 「コメント修正だけなのでゲートは全部素通りするはず」 | **違った**。`ExecResult::Return` 削除で警告 2 件が新たに出た（隠れていた） |
| 「orphan doc は 1 件（#48 のもの）だけ」 | **違った。3 件**あった。うち 1 件は削除済み関数の doc が定数に付いていた |
| 「陳腐化参照は 30 前後」 | 概ね合っていた（**37 識別子 / 61 参照**） |

### 検証（全ゲート緑・2026-08-19）

| ゲート | 結果 |
|---|---|
| `cargo build` | **警告 0**（⚠ 途中 2 件出た。下記） |
| `cargo test` | **740 passed / 0 failed** |
| `cargo clippy --all-targets` | bin **52**（増分 **0**）／bin test 53（52 duplicates）／bench 12 |
| [scan_examples.ps1](scan_examples.ps1) | **FAIL 0** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **49/49 identical**・unexpected diff 0 |
| [force_gate.ps1](force_gate.ps1) | **147 例題・`VmForceError` 0 件** |
| [debug_session.ps1](debug_session.ps1) | **5 identical / 0 differing** |
| [repl_session.ps1](repl_session.ps1) | **identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1)（新設） | **0 件**（履歴・外部参照 38 件は除外） |

### A/B — `#[inline(never)]` を正しい関数へ戻した影響

A = HEAD（属性が `vm_namespace_writeback_fn` に付いていた状態）、B = #51。
⚠ **コメント以外の差分は 4 つだけ**なので、その 3 ファイルだけを `git stash` して A を作った
（作業ツリーには #50 の未コミット変更が同居していたため、`src/` 全体の stash は避けた）。

| bench | A/B |
|---|---|
| `bench_collections.ar` | 0.986x |
| `bench_string.ar` | 1.001x |
| `bench_ab_interp.ar` | 1.000x |
| `bench_arith.ar` | 1.013x |

⇒ **±1.4% ＝ ノイズ床（#27 の ±5〜12%）の内側**。**退行なし・改善も無し**と読む
（#28 の教訓: 数 % で良し悪しを決めない）。属性を戻したのは**設計どおりの状態に戻すため**であって
速度改善のためではない。

### 踏んだこと（作業中）

- **⚠ `git stash push -- <3 ファイル>` は #50 の未コミット変更を巻き込まない形にした。**
  作業ツリーには他人の未コミット作業（#50 の `prof.rs` / `op_prof.rs` / 各フック）が居たので、
  `src/` 全体の stash は**過去の破棄事故**（`git-checkout-destroyed-uncommitted-value-rs`）と同型。
  先に `src/` をスクラッチパッドへ丸ごとコピーしてから触った。
- **⚠ ベンチのプロセス名は `arrow` ではない**。A/B 用にコピーした exe は `arrow_head` /
  `arrow_new` なので、`Get-Process arrow` は**常に 0 件**を返す。これを「A/B が停止した」と
  誤読して 1 回目の実行を kill してしまった（実際は正常に走っていた・20 本は単に遅い）。
  ⇒ **「動いていない」の判断は、名前を実際に確かめてからにする**。
- **⚠ 1 回目の Edit が doc ブロックの後半しか消せていなかった**。orphan doc は
  「A の doc ＋ A の属性 ＋ B の doc」の 3 連なので、**属性の直上だけを消すと A の doc が残る**。
  残骸は次の読者に同じ誤読をさせるので、**orphan を直したら必ず前後を目視する**。

---

## #52 `CompileMode` 導入 ＋ `Compiler::base` / `finish`（2026-08-20）

保守性レーンの 2 本目。**挙動を 1 ビットも変えずに**、`vm/compiler.rs` の
「38 フィールドの構造体リテラル × 5」「24 フィールドの `Chunk` リテラル × 5」
「4 つのモードフラグが 12 箇所へ散在」を畳む。

### やったこと

| 対象 | 前 | 後 |
|---|---|---|
| モードフラグ | `module_mode` / `name_lookup` / `debug_mode` の 3 bool | `CompileMode` enum 1 本（6 バリアント）＋述語メソッド 3 つ |
| `Compiler` の生成 | 38 フィールドのリテラル × 5 箇所（206 行） | `Compiler::base(mode, annot)` ＋ `..` で差分だけ（**最大 11 フィールド**） |
| `Chunk` の生成 | 24 フィールドのリテラル × 5 箇所（113 行） | `finish(ChunkMeta)` 1 本（差分は 4 フィールド） |
| `peephole::optimize` の呼び出し | 4 箇所に散在 | `finish` の 1 箇所 |

`src/vm/compiler.rs` **4,383 → 4,292 行**（-91）。⚠ この数字は
**`base`/`finish`/`CompileMode`/`ChunkMeta` の新規 ~100 行（doc 込み）を足した後**の純減。
定型の削減量そのものは 319 行。

### 畳まなかったもの — `toplevel_globals`

診断では「`toplevel_globals` が **3 役**（グローバル集合／最上位モードの真偽値／
`AsyncBody` ではフラグを立てるためだけの捕捉名）を兼ねている」を指摘したが、**#52 では畳まなかった**。

理由: `!toplevel_globals.is_empty()` を `matches!(mode, Toplevel | Module | AsyncBody)` に
置き換えると **`AsyncBody` で挙動が変わる**。async 本体の集合は**捕捉名**なので、
`captures` が空なら偽になり `LoadGlobal` へ落ちる。モードから導くと常に真になってしまう。

⇒ **式のまま名前だけ付けた**（`reads_by_name` / `writes_toplevel_globals`）。
「何を見ているか」は読めるようになり、かつ**挙動は同一**。
真に畳むなら「captures が空の async 本体で `LoadGlobal` に落ちるのは正しいのか」を
先に決める必要がある（別タスク）。

### ⚠ `compile_debug` だけ覗き穴最適化を通していない

5 つの `Chunk` リテラルのうち `compile_debug` **だけ** `peephole::optimize` を呼んでいなかった。
`finish()` に一本化すると**デバッガ REPL の 1 文に最適化を新たに通す**ことになる＝挙動の変更。

⇒ `finish`（peephole あり）が `into_chunk`（組み立てのみ）へ**委譲**する形にして、
`compile_debug` だけ `into_chunk` を直接呼ぶ。**`Chunk` を組み立てる場所は 1 箇所**のまま
（#22 の「同じ判断をする 2 実装は片方を委譲に畳む」）。

### 不変条件をテストで固定（`mode_tests`）

畳む前は各リテラルに 3 bool が並んでいたので値は**読めば分かった**。畳んだ後は
`matches!` のアームを 1 つ書き換えると**静かに全入口の挙動が変わる**。
⇒ #51 時点の対応表を写した表駆動テストを置いた（`mode_predicates_match_the_pre_52_flags`）。

畳めた根拠のうち非自明なのは 1 点だけ: **`debug_mode || name_lookup` ≡ `name_lookup`**
（`DebugRepl` は `name_lookup` も真だったため）。これも独立したテストで固定した。

### 検証 — **バイトコードの byte-identical 比較**

「挙動を変えていない」の主張は exit code では弱いので、`AR_VM_DUMP=1` の逆アセンブルを
#51 のバイナリと突き合わせた。

| 結果 | |
|---|---|
| 比較した例題 | **210**（dump を出さない 9 件を除く） |
| byte-identical | **206** |
| 差分 | **4（すべて `examples/async/`）** |

⚠⚠ **その 4 件は #52 とは無関係だった**。**同一バイナリで 2 回走らせても差が出る**
（`async_demo.ar` の `<async>` チャンク数が **7↔8 で揺れる**。`arrow_new` が `7 7 8 8 8 8 7 8`、
`arrow_52` が `7 7 7 8 7 7 7 8`）。worker スレッドが stderr へ書く順序と、
タスクが何回コンパイルされるかがスケジューリング依存であるため。
⇒ **`AR_VM_DUMP` の突き合わせは async 例題には使えない**（#53 で再利用するときの前提）。
⚠ 差分を見た瞬間に「退行だ」と判断せず、**まず同一バイナリで再現するかを見る**こと
（#43 で確立した手順がそのまま効いた）。

### 見積もりと実測

| 事前の見立て | 実測 |
|---|---|
| 「5 つのリテラルは差分 8 フィールド程度」 | 概ね合っていた（最大 11・`fn_inner` が最多） |
| 「`peephole` の呼び出しは 5 箇所」 | **違った。4 箇所**（`compile_debug` は通していない）＝ 一本化は**挙動の変更**になるところだった |
| 「`toplevel_globals` もモードへ畳める」 | **違った**。`AsyncBody` が捕捉名の空・非空に依存しており、畳むと挙動が変わる |
| 「バイトコードは全例題で byte-identical になるはず」 | **206/210**。残り 4 は**検査側の非決定性**（async）で、#52 とは無関係 |

### 検証（全ゲート緑・2026-08-20）

| ゲート | 結果 |
|---|---|
| `cargo build` | **警告 0** |
| `cargo test` | **742 passed / 0 failed**（740 ＋ `mode_tests` 2 本） |
| `cargo clippy --all-targets` | bin **52**（増分 **0**） |
| **`AR_VM_DUMP` 突き合わせ**（#51 バイナリ比較） | **210 中 206 が byte-identical**／残り 4 は async（**同一バイナリでも揺れる**＝無罪） |
| [scan_examples.ps1](scan_examples.ps1) | **FAIL 0** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **49/49 identical** |
| [force_gate.ps1](force_gate.ps1) | **147 例題・`VmForceError` 0 件** |
| [debug_session.ps1](debug_session.ps1) | **5 identical**（`compile_debug` を触ったので重要） |
| [repl_session.ps1](repl_session.ps1) | **identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ A/B ベンチは**取っていない**。コンパイラは Chunk キャッシュの裏にいて実行のホットパスでは
なく、かつ**生成バイトコードが byte-identical** と示せたので、実行時性能は定義上不変。
（`exec_op` のような `#[inline(always)]` の本体には一切触れていない。）

---

## #53 `vm/compiler.rs` のサブ分割（2026-08-20）

保守性レーンの 3 本目。§5.1 に「**実装差分**: compiler/ サブ分割（stmt/expr/control）は未分離」と
**当初設計として書かれたまま実施されていなかった**もの（番号も付いていなかった）。

### 結果

`src/vm/compiler.rs` **4,331 行の単一ファイル → `src/vm/compiler/` 10 モジュール**。

| module | 行 | 役割 |
|---|---|---|
| `stmt.rs` | 818 | `compile_stmt` |
| `entry.rs` | 601 | 公開入口 6 つとその `_inner` |
| `emit.rs` | 533 | 命令発行のプリミティブ・書き込み先の決定・型特化の判定 |
| `decls.rs` | 504 | slot 採番と AST 走査 |
| `expr.rs` | 480 | `compile_expr` |
| `mod.rs` | 436 | 型（`Compiler`/`CompileMode`/`ChunkMeta`/`StoreTarget`/`LoopCtx`/`BlockCtx`）＋ `base`/`finish` |
| `block_expr.rs` | 404 | ブロック式 5 種 |
| `calls.rs` | 324 | 呼び出し（引数・FFI 情報・書き戻し先・async 投入） |
| `control.rs` | 285 | `try`/`finally`/`match` 文と脱出時の巻き戻し |
| `diag.rs` | 127 | `bail` 診断フック・組み込み名の表 |

合計 4,507 行（+176）。増分は**ヘッダ 10 本・`use` ブロック 10 本・`impl Compiler {}` の再構成**で、
**中身は 1 行も書き換えていない**。⇒ 診断 [C-1]「1000 行超のファイル」は VM 側では解消
（残るのは `run.rs` 1,505 行だが、これは `exec_op` が `#[inline(always)]` なので**分割してはいけない**）。

### 方針 — 「切って貼るだけ」に徹した

doc コメント境界込みでアイテムを切り出すスクリプトを書き、`impl Compiler` のメソッドは
行き先ごとに `impl Compiler { .. }` を組み直した。**中身の編集は禁止**にしたので、
差分レビューは「どこへ動いたか」だけを見ればよい。

機械的に必要になった付随修正は 4 種類だけ:

1. **`impl` メソッド 70 個に `pub(super)`** — Rust のメソッド可視性は `impl` が置かれた
   モジュール基準なので、**兄弟モジュールからは private メソッドが見えない**。
2. **自由関数 13 個・定数 2 個に `pub(super)`**（`decls` / `diag`）。
3. **`super::chunk::` → `crate::vm::chunk::`** — モジュールが 1 段深くなったため。
4. **未使用 `use` の除去**（`cargo fix`）。

### 踏んだこと

- **⚠⚠ `cargo fix` がテスト専用の再エクスポートを消した。** `VM_BUILTIN_NAMES` は
  `vm_builtin_names_are_all_handled`（#22-d の 2 ファイル跨ぎ不変条件テスト）**だけ**が使うので、
  通常ビルドでは未使用に見える。消された結果 `cargo test` が**コンパイルできなくなった**
  （`cargo build` は緑のまま）。⇒ `#[cfg(test)] pub(crate) use` で復旧。
  **`cargo fix` の後は必ず `cargo test` までやること**（build だけ見て済ませない）。
- **⚠ clippy が +2 になった**（`unnecessary pub(self)`）。`pub(self)` は private と同義で冗長。
  素の `use` に直して 52（増分 0）へ戻した。
- **⚠⚠ `timeout 20` は Windows の GUI 例題を殺せない**。バイトコード比較のループが
  `cs_form_app.ar` で**無限に止まった**（arrow プロセスは既に居ないのにループが進まない）。
  ⇒ `force_gate.ps1` が特別扱いしているのと**同じ 4 例題**を除外して回した。
  症状は #38 のデッドロックと似ているが**原因は別**（あちらはパイプ、こちらは GUI の終了処理）。
- **⚠ 差分 1 件は「バイトコードの差」ではなかった**。`examples/interop/importation.ar` の
  `import[rs]` が **cargo のビルド状態**に依存して別々の地点で `ParseError` になっていた
  （A は `sha2` で失敗、B はその先の `libm` の cargo build 出力を吐いた）。
  どちらも `== chunk` を 1 つも出していない。**状態が落ち着いた後に両バイナリで取り直したら
  完全に一致**。⇒ #52 で確立した「差分を見たらまず同一バイナリ／同一状態で再現するか」が再び効いた。

### 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| `cargo build` | **警告 0** |
| `cargo test` | **742 passed / 0 failed** |
| `cargo clippy --all-targets` | bin **52**（増分 **0**） |
| **`AR_VM_DUMP` 突き合わせ**（#52 バイナリ比較） | **202 例題すべて byte-identical**（async と GUI 4 件は対象外） |
| [scan_examples.ps1](scan_examples.ps1) | **FAIL 0** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **49/49 identical** |
| [debug_session.ps1](debug_session.ps1) | **5 identical** |
| [repl_session.ps1](repl_session.ps1) | **identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ A/B ベンチは取っていない。**バイトコードが byte-identical** で、かつ `exec_op` を含む
`run.rs` には触れていないため。⚠ ただし「ファイルを移すと LLVM のインライン判断が変わる」実例は
この系列に**現にある**（`vm_toplevel.rs` の冒頭コメント: `exec_fn_evaled` と同居させて 10% 退行）。
`compiler/` はコンパイル時しか走らず Chunk はキャッシュされるので該当しないと判断した。

### 見積もりと実測

| 事前の見立て | 実測 |
|---|---|
| 「純粋な移動なので付随修正はほぼ無い」 | **違った**。可視性 85 箇所＋パス修正が必須（兄弟モジュールは private が見えない） |
| 「`cargo fix` で未使用 import を消せば終わり」 | **違った**。テスト専用の再エクスポートまで消され、`cargo test` が壊れた |
| 「全例題でバイトコードを比較できる」 | **違った**。GUI 4 例題は `timeout` で殺せずループが停止する |

---

## #55 ツリーウォークの**式**評価経路の生死判定（2026-08-20）

#33 以降「ツリーウォークが実行するのは定義文だけ」と言い続けてきたが、**その根拠は文の粒度でしか
測られていなかった**（`record_stmt` は `Stmt` しか数えない）。式経路が生きているかは誰も見ていない。
#54（native typed ABI の 1 本化）の規模がこれで変わるので、先に測った。

### 手法 — `AR_TW_STATS` に `tw_eval` を追加

⚠ **`bump()`（Mutex ＋ `String` 確保）は使えない。** `eval()` は**式ノードごと**に呼ばれるので
1 ノードごとにロックを取ると計測が完走しない。relaxed の `AtomicU64` を叩くだけにした
（async の worker からも数えるので thread_local 不可）。

数えたのは 2 つ:
1. `eval()` の呼び出し総数。
2. **AST の式を引数に取る**ツリーウォーク専用入口 5 つの通過数
   （`eval_call` / `call_native_function` / `dispatch_native_typed_exprs` /
   `eval_method_call` / `eval_builtin_ident_call`）。#48 の実バグはこの二重化が原因だった。

⚠ **配線の負の対照を先に取った**（0 を信じる前に）。`control_flow.ar` で `eval_method_call=1` が
発火することを確認 — これは VM の `Op::GetIter` が `__iter__` を呼ぶ経路で、**ツリーウォークの
式評価ではない**。フックは効いている。

### 結果

| 経路 | `eval()` | `eval_call` | native 2 経路 |
|---|---|---|---|
| 例題 130 本（FFI 含む） | **129 本で 0**／`functions.ar` のみ 21 | **全 0** | **全 0** |
| 対話 REPL | **14** | **3** | 0 |
| デバッガ（`dbg_vars`） | **2** | 0 | 0 |

⚠ **FFI 例題が測定対象に入っていることを確認した**（`cpp_out_param_writeback.ar`＝#48 の負の対照・
`cpp_struct_ptr.ar`・`bench_ab_cdll.ar`・`ffi_boundary_check.ar`・`import_py_json.ar`）。
**それでも `call_native_function` / `dispatch_native_typed_exprs` は 0**。
＝ FFI はすべて VM の評価済み値経路（`dispatch_native_evaled_wb`）を通っている。

`functions.ar` の 21 件はデコレータではない（デコレータは #41 で `eval_definition_expr`＝VM 経由）。
`functions/args.rs` の引数束縛と `templates.rs` の実体化が `self.eval()` を直接呼んでいる。
**いずれも定義文・束縛の内側**で、式文の評価ではない。

### 結論 — #54 は「消す」ではなく「畳む」

AST 引数版は削除できない。**`*_evaled` 版への委譲／共通化に畳む**のが正しい（#22 の作法）。
⚠ **native の 2 経路はどのゲートも通っていない**ので、畳むときは負の対照を別に用意すること。

> ⚠⚠ **訂正（#54 で判明）**: ここで一度「AST 引数版は**通常実行から到達不能**」と結論したが、
> **これは誤り**だった。正しくは「**例題が到達していない**」だけ。
> **デフォルト引数の式は呼び出しのたびにツリーウォークの `eval()` で評価される**ので、
> そこに native 呼び出しを書けば `call_native_function`（初回）と
> `dispatch_native_typed_exprs`（インラインキャッシュ命中）を**通常実行で**通る。
> #54 で [cpp_default_arg_native_call.ar](examples/interop/cpp_default_arg_native_call.ar) を書いて実測した
> （`call_native_function=1` / `dispatch_native_typed_exprs=2`）。
> ⇒ **「全例題で 0」から「到達不能」を結論してはいけない**（#34/#35 の教訓そのものを、
> それを引用している最中に踏んだ）。

### ⚠⚠ 副産物 — 実バグ 1 件（→ #56）

**`parse_ar()` が完全に死んでいる。**

```
$ arrow -src probe.ar          # let ast = parse_ar("let x: int = 1")
VmForceError: cannot compile top-level statement `Let` to bytecode
```

`is_builtin_callee` が `parse_ar` を bail するが、その doc は「**ツリーウォークへ bail すべき**」と
書いている — **#33 でフォールバックは消えた**。bail ＝ `VmForceError` ＝ 停止。
関数の中でも同じ（`cannot compile function 'go' to bytecode`）。

§2.1 は `parse_ar` を **【✅ AST 保持】**とし、`python_converter` / `converter.ar` が依存するとしている。
それが動かない。**#51 のバイナリでも同じ**なので、私の #51〜#53 が原因ではなく **#33 以来**の状態。

同じ根で `tuple`/`list`/`type`/`byte` も壊れている: 本来 `NameError`（呼び出し不可）を出すはずが
`VmForceError` に化ける。#34 が確立した「**必ず失敗する文は bail せず `Op::Fail` で同じ文言を出す**」
がここに適用されていない。

⚠⚠ **なぜどのゲートにも映らなかったか**: **`parse_ar` を使う例題が 1 本も無い**
（`converter.ar` は `std_tools/` にあり、しかも別の ParseError で既に壊れている）。
「⚠ 例題が無い言語機能はゲートに映らない」（#34/#35）の 6 回目。

### 計測の穴も 1 つ塞いだ

`tw_stats::dump()` は `run_program` 経路にしか配線されておらず、**対話 REPL は `AR_TW_STATS` に
一度も映っていなかった**。#55 で `Mode::Repl` にも配線した（これが無ければ「REPL で 14 回」は
測れていない）。

### 検証

`cargo build` 警告 **0**（既定・`--features tw_stats` の両方）／`cargo test` **742 passed**／
clippy bin **52（増分 0）**。

A/B（#53 バイナリ比較・`bench_ab_interp.ar`）は **0.968x → 再実行で 1.021x** と**符号が反転**したので
ノイズと判断（フックは `cfg!` 先行判定で既定ビルドから消えるうえ、`eval()` はそもそも 0 回）。
⚠ 1 回目の 3% を「実費用」と読まずに**同じ 2 バイナリで測り直した**のが効いた（#43 の手順）。

### 見積もりと実測

| 事前の見立て | 実測 |
|---|---|
| 「式経路は死んでいるか、生きているかのどちらか」 | **両方だった**。通常実行では死／REPL・デバッガでは生 |
| 「FFI 例題なら AST 引数版が通るはず」 | **違った**。FFI も全部 VM の評価済み値経路 |
| 「#55 は測るだけの小さいタスク」 | **違った**。`parse_ar` が死んでいるという実バグが出た |

---

## #56 `is_builtin_callee` の bail が #33 で実バグ化していた（2026-08-20）

#55 の計測中に出てきた**実バグ**。`parse_ar()` が **#33 以来まったく動いていなかった**。

```
$ arrow -src probe.ar          # let ast = parse_ar("let x: int = 1")
VmForceError: cannot compile top-level statement `Let` to bytecode
```

### 原因 — 「bail ＝ ツリーウォークへ落とす」という前提が消えていた

`is_builtin_callee` は「VM が呼び先として扱えないので **bail してツリーウォークへ落とす**」ための表。
その doc にもそう書いてあった。**#33 でツリーウォークへのフォールバックは削除された**ので、
今の bail は「落とす」ではなく **`VmForceError` で停止**である。表はそのまま残っていた。

被害は 2 つ:

| 名前 | 起きていたこと |
|---|---|
| `parse_ar` | **完全に停止**。最上位でも関数内でも `VmForceError` |
| `tuple` / `list` / `type` / `byte` | 本来の `NameError` が `VmForceError` に化けていた |

### `parse_ar` を外せた理由 — doc が入力と出力を取り違えていた

bail の根拠は「`parse_ar` は **AST を値へ変換するので評価済み引数では表現できない**」だった。
実装を読むと **`self.eval(args[0].expr())` で文字列を取り出しているだけ**で、AST が要るのは
**出力**（`Value::Namespace` ツリーを作る側）。**入力は文字列 1〜2 個**なので評価済み引数で
完全に表現できる。

⇒ `eval_builtin_evaled` に `parse_ar` を実装し、`VM_BUILTIN_NAMES` へ追加。
AST 版（`eval_builtin_ident_call`）は**引数を評価して委譲するだけ**に畳んだ
（#22 の「同じ判断をする 2 実装は片方を委譲に」／`*_evaled` 版とずれた実装を作らない）。

### `tuple`/`list`/`type`/`byte` は bail を外すだけでよかった

これらは `Value::Type` グローバルとして登録されていないので、**そのまま `LoadGlobal` + `Call` に
流せば実行時に `NameError: 'tuple' is not defined` が出る** — 本来の文言そのもの。
#34 が確立した「**必ず失敗する文は bail せず同じ文言を出す**」がここに適用されていなかった。

⇒ `is_builtin_callee` は**空になったので削除**した。知識（「bail を足す前に bail した先で
何が起きるかを確かめる」）は削除跡のコメントとして `diag.rs` に残した。

### ⚠⚠ なぜ 3 系列も見逃されたか — 例題が 1 本も無かった

`parse_ar` を使う例題が**ゼロ**。`std_tools/convert_to_python/converter.ar` は依存しているが
`examples/` の外にあり、しかも**別の ParseError で既に壊れている**ので誰も走らせていない。
⇒ `force_gate` も `scan_examples` も `compare_python_impl` も、**全部緑のまま**だった。

「⚠ 例題が無い言語機能はゲートに映らない」（#34/#35）の **6 回目**。

⇒ #56 で例題を 3 本新設した:
- [parse_ar.ar](examples/basics/parse_ar.ar) … 単一文・複数文・path 引数・**関数内から**（VM 経路）
- [parse_ar_error.ar](examples/basics/parse_ar_error.ar) … `parse_ar(42)` が **`TypeError`**（`VmForceError` ではない）
- [unregistered_type_call_error.ar](examples/basics/unregistered_type_call_error.ar) … `tuple(1,2)` が **`NameError`**

⚠ 後ろ 2 本は「**エラー文言が壊れていないこと**」を固定するのが役目。
値ではなく**文言**を見る負の対照は、この系列では #48 に続いて 2 例目。

### 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| `cargo build` | **警告 0** |
| `cargo test` | **742 passed**（`vm_builtin_names_are_all_handled`（#22-d）が `parse_ar` の追加も検査） |
| `cargo clippy --all-targets` | bin **52**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | **FAIL 0** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **49/49**（新例題 3 本は `$knownDiff` に理由つきで登録） |
| [force_gate.ps1](force_gate.ps1) | **150 例題・`VmForceError` 0 件** |
| [debug_session.ps1](debug_session.ps1) | **5 identical** |
| [repl_session.ps1](repl_session.ps1) | **identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ 新例題は `compare_python_impl` を**一度落とした**（`parse_ar` が py 側に無い・`tuple` は
Python の組み込み）。これは設計どおりの挙動（「新しい例題を足すと自動的に検査される」）なので、
`$knownDiff` に**理由つきで**登録して解消した。

### 積み残し（別タスク候補）

`Self(...)` をメソッド本体の外で呼ぶと、いまも `VmForceError` になる（本来は
`NameError: 'Self' is not defined`）。**不正なコードなので正しいコードは壊していない**が、
#34 の規則には反している。`is_builtin_callee` と分離して残した。

### 見積もりと実測

| 事前の見立て | 実測 |
|---|---|
| 「`parse_ar` は AST が要るから VM に載せられない」（既存 doc） | **違った**。入力は文字列だけで、AST が要るのは出力側 |
| 「`tuple` 等は bail を `Op::Fail` に置き換える必要がある」 | **違った**。bail を外すだけで正しい `NameError` が出る |
| 「#56 は #54 より軽い」 | 概ね合っていた（src は 3 ファイル・例題 3 本） |

---

## #54 native typed ABI 呼び出しの 1 本化（2026-08-20）

保守性レーンの最後。診断 [F-5] で挙げた「typed ABI の C 呼び出しが 3 箇所に手書きコピー」を畳む。
**#48 の実バグ（VM 経路だけ書き戻しが起きず 0.0 を返す）は、まさにこの二重化が原因**だった。

### ⚠⚠ 着手前に #55 の結論を覆した

#55 で「AST 引数版の native 2 経路は **通常実行から到達不能**」と結論していた。**これは誤り**だった。

正しくは「**例題が到達していない**」だけ。到達条件を追うと:
- `call_native_function` … 呼び先が**裸の識別子**で、その文が**ツリーウォークで実行される**とき
- `dispatch_native_typed_exprs` … **同じ呼び出しノード**の 2 回目以降（インラインキャッシュ命中）

REPL では「ブロック最後の式文」だけが `eval()` に落ちるので `call_native_function` には届くが、
ブロックごとに AST が別なのでキャッシュは当たらない。**しかし通常実行に到達路があった** —
**デフォルト引数の式は呼び出しのたびにツリーウォークの `eval()` で評価される**。

```
fn touch(let _unused: float = norm(v345, m)) -> float: return m
```

これで `call_native_function=1`（初回）／`dispatch_native_typed_exprs=2`（2 回目以降）を実測した。

⇒ **「全例題で 0」から「到達不能」を結論してはいけない。**
#34/#35 の教訓（例題が無い機能はゲートに映らない）を、**それを引用している最中に自分で踏んだ**。

### 負の対照を先に作った

[cpp_default_arg_native_call.ar](examples/interop/cpp_default_arg_native_call.ar) を新設。
デフォルト引数から native を呼び、**期待値と違えば `raise` する**（#47/#48 と同じ形。
`scan_examples`/`force_gate` は exit code しか見ないので、raise しないと値の退行が映らない）。

これで 3 経路すべてに例題が付いた:

| 経路 | 例題 |
|---|---|
| `call_native_function`（初回） | [cpp_default_arg_native_call.ar](examples/interop/cpp_default_arg_native_call.ar) |
| `dispatch_native_typed_exprs`（IC 命中） | 同上（2 回目以降） |
| `dispatch_native_evaled_wb`（VM） | [cpp_out_param_writeback.ar](examples/interop/cpp_out_param_writeback.ar)（#48） |

⚠ **畳む前に、この 2 本が緑であることを確認してから**着手した。

### やったこと

3 箇所に手書きされていた 15 行（transmute → 呼び出し → cleanup → status 判定）と、
戻り値デコードの `match sig.ret` を 2 つの関数へ集約:

- `unsafe fn invoke_typed_abi(typed_ptr, slots, cleanups) -> Result<u64, String>`
- `fn decode_typed_ret(ret_ty, ret) -> Value`

⚠ **`unsafe fn` にした**（呼び出し側に `unsafe` ブロック）。生ポインタを関数ポインタへ
transmute して呼ぶ関数を安全な API として置くのは不正直なので、契約を `# Safety` に明記した。

**3 経路で違ってよいのは「書き戻し先をどこへ返すか」だけ**になった:
- `call_native_function` / `dispatch_native_typed_exprs` … 自分で `assign_var`
- `dispatch_native_evaled_wb` … `wb_out` へ積んで**呼び出し元（VM）が格納**（#48）

typed ABI の `transmute` は **3 箇所 → 1 箇所**。`native.rs` は 669 → 667 行
（削減量そのものは ~57 行で、doc 付きヘルパ ~55 行を足した後の純減）。
⚠ **狙いは行数ではなく「実装が 1 つになること」**。#48 の再演を構造的に防ぐのが目的。

### 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| `cargo build` | **警告 0** |
| `cargo test` | **742 passed** |
| `cargo clippy --all-targets` | bin **52**（増分 **0**） |
| **負の対照（AST 経路 1・2）** | `defarg 1st/2nd/3rd OK 5.0` |
| **負の対照（VM 経路 3）** | `local/global/cell/loop/status/probe` 全 OK（#48 の 6 検査） |
| `cpp_struct_ptr.ar`（シャドウ変換の書き戻し） | 一致 |
| [scan_examples.ps1](scan_examples.ps1) | **FAIL 0** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **50/50**（新例題も検査対象・一致） |
| [force_gate.ps1](force_gate.ps1) | **151 例題・`VmForceError` 0 件** |
| [debug_session.ps1](debug_session.ps1) | **5 identical** |
| [repl_session.ps1](repl_session.ps1) | **identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ A/B は取っていない。`invoke_typed_abi` は `#[inline]` を付けていないが、FFI 呼び出し 1 回の
コスト（DLL 越しの C 呼び出し）に対して関数呼び出し 1 段は無視できる。⚠ ただし #48 は
「判定を `Op::Call` のアームに直接書いて 0.91x」を踏んでいるので、**もし将来 typed ABI の
呼び出しがベンチ支配項になったら測り直すこと**（現状 `bench_ab_cdll.ar` で支配的なのは FFI 本体）。

### 見積もりと実測

| 事前の見立て | 実測 |
|---|---|
| 「#55 で AST 経路は通常実行から到達不能と分かった」 | **違った**。デフォルト引数の式から到達する（#55 の結論を訂正） |
| 「負の対照は REPL かデバッガでしか作れない」 | **違った**。通常の例題で 3 経路すべてを踏める |
| 「1 本化で大幅に行数が減る」 | **違った**。純減は 2 行（doc 付きヘルパを足したため）。**価値は行数ではない** |

---

## #57 `stale_doc_refs` の再発 14 件を 0 に戻した（2026-08-20）

#51 が **61 → 0** にしたゲートが、**その直後の保守性レーン自身**（#52 / #53 / #56）で
**14 件まで戻っていた**。計画書の現状反映のためにゲートを走らせて発覚した。

### 内訳 — 3 種類しかない

| 種別 | 件数 | 中身 | 直し方 |
|---|---|---|---|
| ①改名の取り残し | 4 | #52 で `module_mode` / `debug_mode` を `CompileMode` へ畳んだのに、**コメントが古い名前のまま**（`emit.rs` ×2・`entry.rs`・`stmt.rs`） | 現在の名前（`CompileMode::Module` / `CompileMode::DebugRepl`）へ書き換え |
| ②履歴として正しいがマーカー語が無い | 7 | 「#51 まで 3 bool が別々に立っており」「`is_builtin_callee` と違って」など、**消えたものへの意図的な言及** | 同じ行に `以前` / `削除` を置く |
| ③誤検知 | 3 | `_inner` は「`compile_fn` とその `_inner`」という**接尾辞パターン**で識別子ではない | `$whitelist` へ（既存の `extend_` / `snake_case` と同じ扱い） |

⇒ **①だけが実害**（読んだ人が存在しない名前を探す）。②③はゲートの語彙の問題で、
**#51 が用意した逃げ道（マーカー語・whitelist）を使っていなかっただけ**。

### ⚠ マーカー語は「行」に効く — 足すと他の識別子も道連れに黙る
`$histWords`（`削除`/`廃止`/`撤去`/`以前`/`かつて`/`旧 `/`移設`/`だった`/`していた`）は
**同じ行にあれば その行ごと検査から外す**。つまり②の手当ては
**その行に載っている他の識別子も一緒に見逃す**ようになる。
⇒ マーカーを足した 5 行を目視で確認し、**言及がすべて意図的に消えたものだけ**であることを確かめた
（`module_mode`/`name_lookup`/`debug_mode` の 3 つと `is_builtin_callee` のみ）。
⚠ **「とりあえず『削除』と書いて黙らせる」は検知力を捨てる**ので、行の内容ごと見ること。

### 負の対照（**ゲートがまだ効くか**を確かめた）
直したあとに `totally_bogus_probe_fn` への参照を 1 件わざと入れ、
**1 件で exit 1 になること**を確認してから撤去した（#36 の「負の対照が発火しないときは配線を疑う」）。
⚠ ③で `_inner` を whitelist に入れたので、**黙らせ過ぎていないか**の確認でもある。

### 検証
`cargo build` 警告 0 ／ `cargo test` **742** ／ `cargo clippy` **52 件（増分 0）** ／
[scan_examples.ps1](scan_examples.ps1) FAIL 0 ／ [force_gate.ps1](force_gate.ps1) **0 件・151 例題** ／
[compare_python_impl.ps1](compare_python_impl.ps1) **50/50** ／ [repl_session.ps1](repl_session.ps1) identical ／
[debug_session.ps1](debug_session.ps1) **5 identical** ／ [stale_doc_refs.ps1](stale_doc_refs.ps1) **0 件**。
⚠ コメントだけの変更でもゲートは回した（#51 で **コメントだけの変更が警告を 2 件動かした**ため）。
今回は属性を跨ぐ移動が無いので `cargo build` の警告 0・clippy 52 は変化なし。

### 見積もりと実測
| 事前の見立て | 実測 |
|---|---|
| 「14 件とも改名の取り残しだろう」 | **4 件だけ**。残り 10 件は**ゲートの語彙の問題**（マーカー語・whitelist の使い忘れ）で、実害は無かった |
| 「ゲートを作れば陳腐化は止まる」（#51 の暗黙の前提） | **止まらなかった**。#51 の**直後 4 タスク**で 0 → 14。**改名・削除をしたら走らせる**という運用が要る（#57 で「検証は 5 点セット」へ明記） |
| 「マーカー語を足すのは無害」 | **無害ではない**。マーカーは**行単位**なので、同じ行の他の識別子も検査から外れる。⇒ 足すときは行の内容ごと確認する |

---

## #58〜#68 保守性レーン第 2 弾の起票 — 全 src 機械診断（2026-08-21）

#51〜#57 で保守性レーン第 1 弾が完了した後、**あらためて `src/` 全体を機械計測**して
起票した 11 件。観点は「1 ファイル 1000 行以上／1 関数 300 行以上／多数の関数から参照される型／
深いネスト／ロジックフローの煩雑さ」。**この時点では 1 行も変更していない**（起票のみ）。

### 計測方法（推測ではなく実測・#50 と同じ方針）

| 観点 | 手法 |
|---|---|
| ファイル行数 | `wc -l` を `src/**/*.rs` 全件 |
| 関数長 | ブレース平衡でスパン抽出（行コメント・文字列リテラルを除去してから計数） |
| ネスト深さ | 関数先頭のインデントを基準にした相対深さ（4 スペース = 1 段） |
| 型の参照範囲 | `impl` ブロック内の `fn` 数・フィールド数・`impl` が散っているファイル数 |
| 重複 | 14 行窓の正規化ハッシュ一致（doc コメント・空行を除去） |

⚠ **「大きい」だけでは起票しない**。`exec_op` のように「大きいが平坦で、しかも大きいことに
理由がある」ものを弾くため、**大きい関数は必ずアームごとの内訳まで採った**（下記）。

### ⚠⚠ 副産物 — 実バグ 1 件（→ #68）

診断中に **`enum` を関数本体で宣言すると `VmForceError` になる**ことを発見した。

```
fn outer()->int:
    enum Color:
        RED
        GREEN
    let c = Color.RED
    return 1
print(outer())
```

| 実装 | 結果 |
|---|---|
| `target/release/arrow.exe`（HEAD・a7597a6） | `VmForceError: cannot compile function 'outer' to bytecode` |
| `python -m impl_python`（参照実装） | `1` |

真因は `vm/compiler/stmt.rs` の catch-all `_ => bail("stmt", …)` — `compile_stmt` に
`Stmt::EnumDef` のアームが無い。**`Stmt::ClassDef` / `TraitDef` / `ProtocolDef` / `GenDef` /
`NewTypeDef` も同様に無い**が、差分が確認できたのは `EnumDef` だけだった:

| 形 | Rust HEAD | impl_python | 判定 |
|---|---|---|---|
| `enum` in fn | `VmForceError` | `1` | **差分＝実バグ** |
| `class` in fn | `VmForceError` | `AttributeError: 'P' has no field 'x'` | 両方失敗・別要因 |
| `new_type` in fn | `ParseError` | `ParseError` | 文法として未サポート |

⚠⚠ **これは「VM に載らない正しいプログラムを 0 にした」（#34〜#42）の 3 度目の綻び**。
1 度目＝定義文脈の式（#41）／2 度目＝import モジュール本体（#42）と同じ形で、
**`force_gate` 0 件・151 例題が緑なのは、例題にこの形が 1 本も無いから**。
⇒ #56 の「なぜ 3 系列も見逃されたか＝例題が 1 本も無かった」と**同じ理由で同じことが起きた**。

### 起票した 11 件

**A. ロジックフローが実際に絡んでいる（最優先）**

| # | 対象 | 実測 |
|---|---|---|
| 58 | `exec` の `Stmt::Import` アーム | 関数 394 行のうち **この 1 アームで 210 行**・最大ネスト 11。他 30 アームは全部 5〜21 行で `self.exec_*()` へ委譲している |
| 59 | `collect_declared_names` ⇔ `collect_program_globals` | 14 行の重複が機械検出。**しかも既にドリフト済み**（下記） |
| 60 | `parse_struct_bodies` | 307 行・**最大ネスト 12（src 最悪）**。`decls.rs:scan_scope` も 405 行・深さ 8 で同型 |
| 61 | `run_program` の `ar_config.json` 探索 | パイプライン関数の中に **深さ 9** の探索ループが直書き |

**B. サイズ（大きいが性質が違う）**

| # | 対象 | 実測 |
|---|---|---|
| 62 | `compile_stmt` | **1 関数で 800 行＝ファイル 817 行のほぼ全部**。33 アームだが偏りが大きい（`For` 97／`AttrCompoundAssign` 76／`CompoundAssign` 75／`Let` 55／`AttrAssign` 48） |
| 63 | `eval_method_call_full` | 530 行・深さ 10。**型ごとの切り出しが途中で止まっている**（下記） |
| 64 | `gen_expr_inner` | 558 行・深さ 9。`Expr::Call` アームが 146 行ある一方、同ファイルに `fn gen_call`（156 行）が別に居る＝**呼び出し生成が 2 箇所** |
| 65 | `eval_str_method` | 581 行だが **48 個の `"method" =>` が並ぶ平坦な組み込みメソッド表**（平均 12 行）。優先度低 |

**C. 神クラス**

| 型 | メソッド | フィールド | `impl` の散在 |
|---|---|---|---|
| `Interpreter` | **215** | **31** | **33 ファイル** |
| `Parser` | 95 | 13 | 14 |
| `Compiler` | 65 | **36** | 7 |
| `TypeChecker` | 63 | 5 | 11 |
| `GenCtx` | 39 | **39** | 4 |

| # | 対象 | 実測 |
|---|---|---|
| 66 | `Compiler` の 36 フィールド | **うち 17 は `Chunk` の複製**で、`into_chunk` が 1 対 1 で move するだけの 17 行になっている |
| 67 | `Interpreter` の 31 フィールド | 責務は 7 クラスタに分かれる（スコープ/VM キャッシュ/イベントループ/モジュールロード/クラス情報/デバッガ/コールスタック） |

### #59 の詳細 — walker は既にずれている

| walker | `EnumDef` / `NewTypeDef` |
|---|---|
| `resolver.rs:collect_program_globals` | **有り**（#27-c で追加。無かったため VM が bail したとコメントに明記されている） |
| `exec/mod.rs:collect_declared_names` | **無し** |

`collect_declared_names` の消費者は **3 系統**（`exec/blocks.rs:capture_env` のクロージャキャプチャ、
`vm/compiler/decls.rs`、`vm/compiler/calls.rs`）。同じ文法を歩く walker は現在 **8 本**あり
（`collect_declared_names` / `collect_referenced_names` / `collect_shadowing_binders` /
`collect_program_globals` / `collect_bound_names` / `collect_base_decls` /
`scan_shadow_stmts` / `collect_nested_decls`）、`Stmt` に variant を足しても何も強制しない。
⇒ プランの教訓「**同じ木を歩く walker が 2 つあるとずれる**」（#27-c で 2 回踏んだ）が
**現状のコードで既に成立している**。

### #63 の詳細 — 委譲したものとしていないものが混在

| レシーバ | 状態 |
|---|---|
| `Value::Str` | ✅ `eval_str_method`（別ファイル 601 行）へ 1 行委譲 |
| `Value::Instance` / `FileObject` / `PyObject` | ✅ 委譲（5 行以下） |
| `Value::Set` | ❌ **126 行インライン** |
| `Value::AsyncManager` | ❌ 76 行インライン |
| `Value::Class` | ❌ 71 行インライン |
| `Value::FrozenList` | ❌ 65 行インライン |

`eval_set_method` / `eval_list_method` / `eval_dict_method` は **1 つも存在しない**（grep 済み）。
`classes/` は既に `freeze` / `instantiate` / `lookup` / `object_methods` / `string_methods` に
分かれているので、**受け皿の形は決まっている**。

### ⚠ 対象外と判断したもの（誤って着手しないための記録）

| 箇所 | 行数 | 対象外の理由 |
|---|---|---|
| `vm/run.rs:exec_op` | **836** | **86 アームの平坦なディスパッチ表で、最大アームは `Op::Call` の 42 行**（実測）。`#[inline(always)]` かつ「重い本体はアームに書かない」制約が #10-b で実測済み。`breakpoint_op` 等は既に `#[inline(never)]` で外出しされていて**設計どおり守られている**。⇒ **行数だけを見て割ると #10-b を再演する** |
| `ast.rs` | 1124 | `Expr` 30 / `Stmt` 40 variant の型定義。ロジックがほぼ無い |
| `Value`(352 fn / 64 file)・`Stmt`(200/56)・`Expr`(177/57) | — | 言語処理系の中核データ型。参照が広いのは正常で、狭めると逆に悪化する |
| `parser/exprs.rs` | 940 | 優先順位チェーンで 1 段 1 関数。最大は `parse_primary` の 157 行 |

### 参考 — 現状の clippy 内訳（`cargo clippy --all-targets` 65 件）

上位は `irrefutable let...else` 12／`stripping a prefix manually` 10／`empty line after doc comment` 7／
`very complex type` 5／`doc list item without indentation` 5。
**`#[allow(clippy::too_many_lines)]` は src 全体で 1 箇所だけ**（`string_methods.rs:16`）で、
そこは `///` → 属性 → `///` と **doc コメントに属性が挟まった形**になっている（#51 が潰した orphan doc と同型）。
`#[allow(dead_code)]` は 12 箇所 — #51 の教訓「**属性が警告を食っていないか疑う**」の対象。

### 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「`exec_op` 836 行は最大の分割対象だろう」 | **外れ**。アーム内訳を採ると最大 42 行の平坦な表で、分割は #10-b の再演になる |
| 「`exec` 394 行は全体的に肥大しているのだろう」 | **外れ**。異常なのは `Stmt::Import` の 210 行 **1 アームだけ**で、残り 30 アームは 5〜21 行 |
| 「重複検出はノイズだらけだろう」 | **半分外れ**。14 行窓で 23 組しか出ず、うち 1 組（#59）は**実際にドリフト済み**だった |
| 「診断だけなのでバグは出ないだろう」 | **外れ**。`enum` in fn で実バグ 1 件（→ #68）。#55 に続き **2 回連続で「調査だけのタスク」が実バグを出した** |

---

## #69 `interp_init` の削減（**未着手**・調査済み・速度／#50 の①）

> ⚠ **61（`ar_config.json` 探索の切り出し）と同じ場所を触る。** 61 は保守性、69 は速度。
> **切り出してから速度を入れる**（順序を逆にすると、切り出しの「挙動不変」検査に速度変更が混ざる）。

### 1. この段の役割 — `interp_init` は何をしているか

`run_program` が「インタープリタを実行可能な状態にする」段。中身は 6 つの独立した仕事:

| # | 仕事 | 目的 |
|---|---|---|
| ① | `Interpreter::new()` | 組み込みグローバル（`int`/`str`/`len` 等 24 件）を `scopes[0]` へ登録し、`EventLoop` シングルトンと外部イベントキューを生成する |
| ② | `set_toplevel_globals` | 最上位 Chunk（#10-b/#10-c）が「この名前は `scopes[0]`」と断定するための名前集合を注入（AST を 1 walk） |
| ③ | `set_annotations` | 型解決層の注釈（#16）を `Rc` で注入。VM コンパイラが node-id で引く |
| ④ | `add_source_text` | エラー報告のスタックトレース表示用にソース文字列を登録 |
| ⑤ | **`ar_config.json` の祖先ウォーク** | `import[py]` の検索パスを組み立てる。`source_dir` から**親へ遡って** `ar_config.json` を探し、最初に見つけた所の `python.search_paths` を（相対なら絶対化して）`python_search_dirs` へ push |
| ⑥ | `set_cli_args` | `--key value` を `args` dict としてグローバルへ登録 |

### 2. 動作 — ⑤ が具体的に何をするか

```
walk = source_dir
loop:
    cfg = walk.join("ar_config.json")
    if cfg.exists():                     ← ファイルシステムの metadata 問い合わせ
        read_to_string(cfg)              ← open + read + close
        serde_json::from_str(text)       ← JSON パース
        push 検索パス; break
    walk = walk.parent()                 ← 無ければ**ドライブ root まで**遡る（打ち切り無し）
```

⇒ **見つかるまでの祖先の数だけ `exists()` が走り、見つからない場合は root まで全部走る。**

### 3. 具体的な遅延の要因（**実測**・`--features prof` の `SUB` 行）

同一バイナリ・warm（コールドリードは捨てた）:

| 内訳 | repo 内（3 階層上で発見）× 4 | repo 外（root まで走って不発）× 3 |
|---|---|---|
| **`ar_config_walk`** | **0.176〜0.541 ms（50〜75%）** | **0.234〜0.347 ms（55〜62%）** |
| `interp_new` | 0.137〜0.186 ms（21〜43%） | 0.172〜0.209 ms（35〜42%） |
| `annot+source` | 0.016〜0.022 ms（3〜5%） | 0.007〜0.009 ms（<2%） |
| `toplevel_globals` / `cli_args` | 合計 0.006〜0.013 ms（<2%） | <0.1% |
| **`interp_init` 合計** | 0.351〜0.723 ms | 0.417〜0.555 ms |

**要因1 — ⑤ が syscall の連打になっている。** 支配項は `read_to_string` でも JSON パースでもなく
**`exists()` の回数**（config が見つからない repo 外でも同じだけ掛かっている）。
1 回あたり Windows では `GetFileAttributesW` ＋ フィルタドライバ（Defender 等）を通る。
⇒ **ファイルが深い所にあるほど・config が無いほど高い**（打ち切りが無いため）。

**要因2 — ⑤ は `import[py]` が無いプログラムでも必ず走る。** `python_search_dirs` の唯一の
消費者は Python モジュールの解決なので、**大多数のスクリプトではこの結果を誰も読まない**。

**要因3 — ① が仕事量に対して高い（0.14〜0.21 ms）。** 中身は挿入 24 件と空コレクションだけで、
`add_python_search_dir` は `Vec::push`、`global_ext_queue()` は `OnceLock` の初期化、
`EventLoopData::new()` は空の `VecDeque` — **どれもこの時間を説明できない**。
⇒ **仮説: プロセス最初のまとまったヒープ確保が CRT ヒープ／ページのコミットを引き起こす
first-touch コスト**（`Interpreter::new()` が悪いのではなく、**そこが最初だっただけ**）。

### 4. ⚠ 着手時にまずやること（順序を守る）

1. **要因3 の仮説を先に潰す。** `Interpreter::new()` の直前に同規模のダミー確保を置き、
   `interp_new` が縮んで **`in_main` の合計が変わらなければ first-touch**。
   ⇒ そうなら **①を最適化しても総時間は減らない**（費用が後段へ移るだけ）。**着手範囲から外す。**
2. その上で ⑤ に手を入れる。候補は 4 つ:
   - (a) **遅延化** — `import[py]` が現れたときに初めてウォークする（**要因2 を直接消す・本命**）
   - (b) 打ち切り — 探索段数の上限、またはリポジトリ境界（`.git` 等）で止める
   - (c) `exists()` を廃し `read_to_string` の失敗で判定（祖先 1 段あたり syscall 1 本に）
   - (d) 不発の否定キャッシュ（1 プロセスで複数モジュールを読むとき効く）

### 5. ⚠ 期待値を先に釘付けにしておく（過大評価の防止）

`interp_init` は `in_main` の 20〜50% だが、**`in_main` 自体が process wall の半分以下**
（#50: 非 bench 例題の中央値で wall 3.40ms ／ `in_main` 1.57ms）。
⇒ **完全に消しても端点への効果は 0.3〜0.5 ms/実行**。#50 の「exec 以外が 86%」のうち
**Arrow が触れるのはここと `parse`/`type_check` だけ**で、残り（プロセス生成・イメージロード・終了
≈1.5ms）は OS 側である。**「86% が取れる」ではない。**

### 6. 検証

- 効果は [prof_dist.ps1](prof_dist.ps1) の `-Mode phases` の `interp_init` と **`SUB` 行**で見る。
  ⚠ **process wall では見えない**（spawn 床のゆらぎ ±2ms に埋もれる）。
- 負の対照: **config が無い深い階層**（repo 外）でも改善すること。ここが一番損をしている形。
- 挙動不変: `import[py]` を使う例題（`examples/interop/import_py_*.ar`）が通ること。
  ⚠ (a) を採るなら **`ar_config.json` の `python.search_paths` に依存する例題**が要る。
  無ければ**先に足す**（「検査網は例題が踏む形しか見ない」）。

---

## #70 最上位ループが型特化・融合命令に載らない（**未着手**・調査済み・速度）

> ⚠ **#59（walker 8 本の統合）とは無関係**。番号が近いだけ。

### 1. 役割 — 型特化・融合命令とは何か

`IntBinLL` / `IntBinLC` / `FloatBinLL` / `FloatBinLC` / `BinLocalLocal` / `BinLocalConst` は、
二項演算の **①オペランドのスタック積み ②タグ検査 ③op ディスパッチ ④`Value` の clone** を
**1 命令に畳む**超命令（#2 ＋ #2b の型特化）。`i < n` や `i += 1` のようなループ制御が主な客で、
VM の算術が exec の 22.3%（#50）を占める中の中核。

### 2. 動作 — 融合の入口条件

[src/vm/compiler/emit.rs](src/vm/compiler/emit.rs) `try_emit_bin_fused`:

```rust
let Some(a) = self.as_local(left) else { return false; };   // ← 左辺が slot でなければ即諦める
let kind = self.specialized_bin_kind(op, node_id, left, right);
self.emit_bin_fused_slot(a, kind, right, op)                // 右辺: slot → *BinLL / 定数 → *BinLC
```

つまり **融合の前提は「左辺がフレーム内 slot であること」**。型注釈が int/float 確定なら型特化 op、
決まらなければ `BinLocal*`、どちらでもなければ融合せず通常経路（`LoadX…; Bin`）。

### 3. 具体的な遅延の要因

**最上位（関数の外）で宣言された変数は slot ではなくグローバル**なので `as_local` が `None` を返し、
**融合が一切効かない**。同じ `while i < n:` が置き場所で別のコードになる:

| 置き場所 | 生成される命令 |
|---|---|
| `fn` の中 | `IntBinLL(i, n, Lt)` … **1 命令** |
| 最上位 | `LoadGlobal(i); LoadGlobal(n); IntBinSS(Lt)` … **3 命令**（しかも `LoadGlobal` は IC ヒットでもセル読み＋`Value` clone） |

**実測**（同一プログラム内 A/B・`N = 3,000,000`・best of 3）:

| | 空ループ 1 反復 | 呼び出し 1 回の追加コスト |
|---|---|---|
| `fn` の中 | **30.8 ns** | 279.9 ns |
| 最上位 | **79.5 ns（2.58x）** | 280.7 ns |

⇒ **⚠⚠ 呼び出しコストは置き場所に依らない（280 ns で一致）。**
当初の見立て「最上位は**呼び出し**が高い」は**実測で否定された**。最上位の遅さは
**ループ制御＝変数アクセスが融合非適格**であることが原因。

同プログラムの op サンプリング（4 ループ合計・`AR_PROF=ops`）:
`LoadGlobal` 282.8ms ＋ `StoreGlobal` 98.9ms = **381.7 ms** ／
`LoadLocal` 16.6ms ＋ `StoreLocal` 39.4ms = **56.0 ms**
（fn 側は読みが超命令に畳まれているので `LoadLocal` 自体がほとんど出てこない）。

### 4. 手法の候補

- (a) **帰納変数の slot 昇格** — 最上位の `while`/`for` 文 Chunk 内でローカル化し、文の終わりで
  グローバルへ書き戻す。⚠ 意味論の確認が要る: 途中で例外／`break` が飛んだときの可視性、
  クロージャの捕捉、`static mut`、**デバッガからの観測**（`local_names` / 停止時の名前引き）。
- (b) **グローバル版の融合 op**（`IntBinGL` 等）を足す。
  ⚠ **#27 の教訓: op を足す規模の摂動は 1 命令も実行しなくてもベンチを 0.88〜0.94x 動かす。**
  「変更と同規模のプローブ」との 3 本比較が必須。
- (c) 何もしない（「ホットループは fn の中に書け」と言い続ける）。
  ⚠ #47 の「最上位は fn 内より 3.6 倍遅い」を放置することになる。

### 5. ⚠ 着手前に例題を足すこと

**corpus 全体ではグローバル/名前引きは exec の 3.9% しかない**（#50）。理由は
**例題側の規約**で、bench 群は先頭に「ホットループは必ず `fn` の中に置く」と書いてある。
⇒ **素直に最上位へ書くユーザーのコードは、今の検査網に一切映らない。**
これは「検査網は例題が踏む形しか見ない」（#27/#34/#36/#33/#41 で 5 回踏んだ）の再演なので、
**着手するなら最上位ホットループの例題を先に新設する**。

### 6. ⚠ 参考 — 呼び出し側は投影に到達済み（ここを狙う前に読む）

同日に `bottleneck_bench.ar` の `fn call (no args)` を測ると **0.123〜0.135 µs**
＝ 実装ログ #12 の記録 **0.138 µs** と一致。§7.2 の投影「フラットなら ~0.12µs」は**達成済み**。
1 引数の往復が 280 ns なので、呼び出し系に残る伸びしろは **引数束縛**側にある（§7.2 も別項目）。

⚠⚠ **この確認の途中で「0.405 µs ＝ 3x の退行」という誤報を 1 度出した。**
原因は **`cargo build` 直後の初回実行**を測ったこと（コールドリード）。#50 で自分が
`prof_dist.ps1` に「1 回目は捨てる」と書いた**その罠を、素のベンチで踏んだ**。
⇒ **`bench.ps1` 系も初回実行を捨てる**（`-Reps` の 1 本目を採用しない）。

---

## #68 関数本体の `enum` が `VmForceError`（実バグ）を修正（2026-08-21）

#58〜#68 の起票診断で見つけた実バグ（起票の経緯は 1 つ上の項）。**#33 以降、`enum` を
関数の中で宣言すると `VmForceError` で止まっていた**（参照実装 `impl_python` は通る）。

### 原因は 2 段（片方だけ直すと直らない）

| 段 | 場所 | 症状 |
|---|---|---|
| ① | `compile_fn_inner` の decl-prepass | `Stmt::EnumDef` が**明示的な bail リスト**に入っていた（`bail("decl-prepass")`） |
| ② | `compile_stmt` | `Stmt::EnumDef` のアームが無く catch-all の `bail("stmt")` に落ちる |

⚠ 起票時は②だけを真因と書いたが、**実際に先に発火するのは①**だった（`AR_TW_STATS` で確認）。
①のリストは「slot を採番する可能性のある未対応の宣言的文」で、**リゾルバの
`collect_base_decls` が `EnumDef` に `push_base` している**ため、飛ばすと以降の base slot が
全部 1 つずれる。⇒ **採番を足さずに bail だけ外すと `LoadLocal` が別の変数を読む**。

### 直し方 — 「組み立て」と「記憶域」を分けた

`exec_enum_def` が **クラスの組み立てと `declare_var` を一体で**やっていたので、VM から
呼んでも `Name` が slot ではなく**呼び出し元のスコープ**へ入ってしまい、載せようが無かった。
（VM 関数の実行は `push_scope` しない ＝ `scopes.last_mut()` は呼び出し元のスコープ。）

⇒ `build_enum_classes`（組み立てのみ・`declare_var` しない）へ切り出し、記憶域は
呼び出し元が決める形にした:

| 呼び出し元 | `Name` | `enum_item_Name` |
|---|---|---|
| ツリーウォーク `exec_enum_def`（最上位・モジュール本体） | `declare_var` | `declare_var` |
| VM `Op::EnumDef`（関数本体） | **フレームの slot** | `declare_var` |

`enum_item_Name` を slot にしないのは、**リゾルバが slot を採らない合成名**だから
（`collect_base_decls` は `name` だけ `push_base` する）。実行時に名前で引かれる経路は
`value_is_type` を確かめたが**クラス名の文字列比較**でスコープを見ていない。

### 変更点（src +155 / -9）

- `Op::EnumDef(u32)` を追加（`Op` のサイズは **20 byte のまま**＝`op_size_is_pinned` が緑）
- `Chunk::enum_defs: Vec<ChunkEnumDef>`（`name` / `variants: Rc<[..]>` / `slot`）
- `run.rs`: `enum_def_op` を **`#[inline(never)]`** で外出し（#10-b の教訓。アームは 3 行）
- `compile_stmt`: `slot_of(name)?` → 表へ push → `Op::EnumDef` を emit
  ⇒ **slot を持たない文脈（最上位・モジュール本体）ではこのアームが自分で bail する**ので、
  入口ごとの場合分けが要らない（最上位は `is_toplevel_compile_target` が従来どおり除外）
- `peephole::code_target_mut` への登録は**不要**（`MakeFn` と同じくコード索引ではなく表索引）

### ⚠⚠ 副産物 — #59（walker のドリフト）が実害として発火した

本体直下の `enum` が通るようになった直後、**クロージャが自分の `enum` を宣言する形**が
`capture-slot-conflict` で bail した（`AR_TW_STATS` で確認）。

```
fn outer()->int:
    enum E:
        A = 1
    fn inner()->int:
        enum E:           # ← これで outer ごと VmForceError
            A = 2
        return E.A.value
    return inner() * 10 + E.A.value
```

真因は #59 として起票した**まさにそのドリフト** — `exec/mod.rs:collect_declared_names` に
`EnumDef` が無いので、`capture_env` が `E` を自由変数と誤認して外側を捕まえ、
コンパイラが採番した slot とぶつかった。**`entry.rs` の「ぶつかったら諦める」ガードが
設計どおり機能して**、黙って閉包変数が消えるのではなく計測できる形で落ちていた。
⇒ `collect_declared_names` に `EnumDef` を追加（**この 1 variant だけ**。
`NewTypeDef` は関数内でパースエラーなので検証できず、#59 の統合へ回した）。

### 例題（**この形が 1 本も無かったのが 4 度目の綻びの原因**）

- [enum_in_function.ar](examples/typing/enum_in_function.ar) — 9 ケース:
  本体直下／明示値の自動採番継続／同名グローバルとの分離／複数回呼び出し／
  値式（`1 + 1`）／等値比較／**クロージャが自分の enum を宣言**／**クロージャが親の enum を読む**／
  **if・for・while・try の中**
- [enum_in_function_error.ar](examples/typing/enum_in_function_error.ar) — バリアント値の int 検査。
  ⚠ **#68 以前はこの行に届く前に関数ごと bail していた**＝「必ず失敗する文は bail せず
  同じ文言を出す」（#34）も破れていた。今は最上位に同じ enum を書いたときと**同一文言**。
  `compare_python_impl.ps1` の `$knownDiff` に登録（py は int 検査が無く `x` を通す）。

### 検証（全ゲート緑・**自分で走らせた**）

| ゲート | 結果 |
|---|---|
| `cargo build` | 警告 **0** |
| `cargo test` | **742 passed**（`op_size_is_pinned` 含む） |
| `cargo clippy` | **52 件（増分 0）** |
| [scan_examples.ps1](scan_examples.ps1) | **FAIL 0** |
| [force_gate.ps1](force_gate.ps1) | **0 件 / 153 例題**（151 → +2 は今回の例題） |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51 identical**（50 → +1）・stale 0 |
| [repl_session.ps1](repl_session.ps1) | identical |
| [debug_session.ps1](debug_session.ps1) | **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |
| [tw_stats.ps1](tw_stats.ps1) | `in_fn` **0**・`vm_bail_fn` **0**・`vm_ineligible` **0**・`tw_control_flow` **0**。最上位のツリーウォークは**定義文だけ**（`EnumDef` 4 件＝最上位の enum は従来どおり） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **106/108 byte-identical**。差分は**今回の例題 2 本だけ**（HEAD は bail するので 15 行 / 9 行しか出ない） |

### [compare_bytecode.ps1](compare_bytecode.ps1) を新設した

「挙動不変」を exit code より強く裏付ける **`AR_VM_DUMP=1` の突き合わせ**（#52 で確立）は
毎回手で回していたので固定した（規約: 同じ操作を繰り返すなら .ps1 化）。
残りの保守性タスク（#62 / #63 / #66）が同じ検査を要る。

⚠ **負の対照を先に取ってから使った** — 同一 exe 同士で **108/108 identical**。
これで「差が出ない」のがバグではなく事実だと確かめてから A/B を読んだ。

### ⚠ 詰まった点（次に .ps1 を生成するとき用）

- **ヒアドキュメント経由で Python に渡すとバックスラッシュ 2 個が 1 個に潰れる**。結果、
  Windows パスの区切りが**エスケープとして解釈**され、`\t` が TAB、`\r` が CR、
  `\a` が BEL に化けた。**PS は「行 25 の構文エラー」と誤報**する（実際に壊れていたのは
  行 8 のコメント行）＝ **エラー行番号を信じて探すと見つからない**。
  ⇒ **生成する .ps1 のパス区切りは全部スラッシュ**にした（PowerShell は受け付ける）。
  書き出し後に**バックスラッシュ・TAB・CR・BEL が 1 つも無いことを assert** している。
  ⚠ **この段落自体が 1 度この罠で壊れた**（罠の説明に書いたバックスラッシュが潰れた）。
- `PSParser.Tokenize` は**通るのに `Parser.ParseFile` で落ちる**（字句と構文は別）。
  生成した .ps1 の検査は **`ParseFile` でやる**こと。
- vm-pitfalls §4 の「Python から日本語を print すると cp932 で落ちる」も実際に踏んだ
  （行を確認するだけのワンライナーが `UnicodeEncodeError`）。**進捗表示は ASCII に落とす**。

### 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「真因は `compile_stmt` の catch-all」 | **半分外れ**。先に発火するのは decl-prepass の明示 bail リストで、**採番を足さないと直せない** |
| 「`exec_enum_def` をそのまま VM から呼べばよい」 | **外れ**。`declare_var` と一体だったので `Name` が呼び出し元スコープへ入る。**組み立てと記憶域の分離が必須**だった |
| 「本体直下だけ直せば済む」 | **外れ**。if/for/while/try の中は `collect_nested_decls` も要り、クロージャは #59 のドリフトを踏んだ（**1 バグに 4 箇所**） |
| 「診断で見つけたバグなので小さい」 | **当たり**（src +155 行）。ただし**触った walker は 3 本**（prepass・nested・declared_names） |

---

## #66 `Compiler` の `Chunk` 複製フィールドを畳む（2026-08-21・完了）

保守性レーン第 2 弾。起票時の見立ては「`Compiler` の 36 フィールドのうち **17 が `Chunk` の複製**で、
`into_chunk` が 1 対 1 で move するだけの 17 行になっている」（→ #58〜#68 の診断）。

### 1. 実際に何が重複していたか

`Compiler` と `Chunk` に**同じ名前・同じ型のフィールドが 17 本**並んでいた:

```
code / consts / names / attr_caches / spans / stmt_spans / n_locals / async_blocks /
global_caches / ffi_call_info / wb_targets / fn_defs / enum_defs / type_arg_lists /
tuple_decls / kw_calls / n_cells
```

⇒ **`Chunk` にフィールドを 1 本足すと 4 箇所**を直す必要があった:
`Chunk` の定義／`Compiler` の定義／`Compiler::base()` の初期化／`into_chunk()` の move。
#52 が「構造体リテラル 5 箇所 → 1 箇所」に畳んだのと**同じ形の重複が残っていた**。

⚠ **実際に踏んでいる**: この直前の #68（`enum_defs` の追加）が、まさにこの 4 箇所を全部直している。

### 2. 手法 — ⚠ 起票時の「`ChunkBuilder` 部分構造体へ」は**採らなかった**

起票時の手法欄は「`ChunkBuilder` 部分構造体へ」だったが、それでは
**`into_chunk` の逐語 move は消えず、`ChunkBuilder::build()` の中へ引っ越すだけ**になる
（Rust は構造体 A → 構造体 B のフィールド移送を無償にはできない）。

⇒ **`Compiler` が `Chunk` そのものを組み立てながら持つ**形にした（`Compiler::chunk: Chunk`）。

| | 前 | 後 |
|---|---|---|
| `Compiler` のフィールド | **37**（#68 の `enum_defs` を含む。診断時の 36 はその前の値） | **21** |
| `into_chunk` の本体 | 21 行の逐語 move | **6 行**（`ChunkMeta` の 4 つ ＋ `..self.chunk`） |
| `Chunk` にフィールドを足すとき直す箇所 | **4** | **1**（`Chunk` の定義だけ。`Default` が埋める） |
| `src/vm/compiler/mod.rs` | 439 行 | **395 行**（doc を 20 行増やした上で **-44**） |

`Chunk` に `#[derive(Default)]` を付けた（全フィールドが `Vec`/`HashMap`/`usize` なので
要素型に `Default` を要求しない）。

⚠ **`ChunkMeta` は畳まなかった**（#52 が「入口ごとに違うのはこの 4 つだけ」を可視化するために
導入したもので、消すとその情報が失われる）。`local_names` / `n_params` / `captured_slots` /
`captured_cells` は **コンパイル中には決まらず入口が知っている**値なので、性質としても別物。
⇒ `Chunk` にフィールドを足すとき `ChunkMeta` に足してよいのはこの性質を持つものだけ、と doc に明記した。

### 3. 作業の内訳（**1 行も意味を変えない機械的置換**）

| 対象 | 件数 |
|---|---|
| `self.<field>` → `self.chunk.<field>`（正規表現で一括） | **48 箇所 / 6 ファイル**（emit 19・stmt 10・calls 8・expr 8・control 2・block_expr 1） |
| `mod.rs` の `finish` / `into_chunk` | 手で 2 箇所 |
| `entry.rs` の構造体リテラル 5 箇所（`n_locals` / `n_cells` の初期値） | `chunk: Chunk { n_locals: …, ..Chunk::default() }` へ |
| 不要になった import（`Op` / `Value`） | mod.rs から削除（各サブモジュールは自前で import 済みと確認してから） |

⚠ **`self.` 以外の経路が無いことを先に grep で確かめた**（`compiler.code` のような外部アクセスは 0 件）。
これが無いと一括置換が漏れる。

### 4. 検証（**全ゲート緑**）

| ゲート | 結果 |
|---|---|
| [compare_bytecode.ps1](compare_bytecode.ps1) | **108 / 108 byte-identical**（⚠ **先に同一 exe の負の対照 108/108 を取ってから** A/B を読んだ） |
| `cargo build` | 警告 **0** |
| `cargo test` | **742 passed** |
| `cargo clippy` / `--all-targets` | **52 / 65**（増分 **0**・基準値と一致） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・153 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |
| [ab_bench.ps1](ab_bench.ps1) | 0.972〜1.036x（**退行なし**。1.15〜1.18x に見える 3 本は 0.02s 台＝プロセス起動床のみでノイズ） |

⚠ **バイトコードが byte-identical なので実行時の退行は原理的に起きない**（`vm/run.rs` は 1 行も触っていない）。
A/B は「そう言えることの確認」であって判断材料ではない。

### 5. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「`ChunkBuilder` 部分構造体を作る」 | **手法が不適**。それでは逐語 move が引っ越すだけ。**`Chunk` を直接持つ**のが正解だった |
| 「17 フィールド × 参照多数で大工事だろう」 | **外れ**。`self.<field>` の参照は全部で **48 箇所**しかなく、一括置換で足りた |
| 「`entry.rs` は `..Compiler::base()` なので触らずに済む」 | **外れ**。5 入口が `n_locals` / `n_cells` の**初期値**をリテラルで渡していた（`self.` の grep には出ない） |
| 「`Chunk` に `Default` を足すと要素型にも `Default` が要る」 | **外れ**。全部 `Vec`/`HashMap`/`usize` なので要素型は無関係 |

---

## #58 `exec` の `Stmt::Import` アーム 210 行を切り出す（2026-08-22・完了）

保守性レーン第 2 弾の「A. ロジックフローが実際に絡んでいる」最優先枠。起票時の実測は
「`exec` は 394 行で、**うち `Stmt::Import` の 1 アームが 210 行・最大ネスト 11**。
残り 30 アームは全部 5〜21 行で `self.exec_*()` へ委譲している」。

### 1. 何が異常だったか

`exec` は**ディスパッチ表**である。30 アームは 1〜2 行で専用メソッドへ渡すのに、
`Stmt::Import` だけが **4 言語ぶんの読み込み手順を丸ごと本文に持っていた**
（cpp のキャッシュ／cs-dll のブリッジ DLL 探索／cs-proc のホスト exe 探索／js-proc の
Node ブリッジ起動）。⇒ **「文の種類を振り分ける」以外の関心事が 1 箇所だけ混ざっていた**。

### 2. 手法

`exec/modules.rs` へ `exec_import` ＋ 補助 7 本を新設し、`exec` 側は 1 アーム 8 行の委譲にした。

| | 前 | 後 |
|---|---|---|
| `exec()` | **394 行** | **192 行** |
| `Stmt::Import` アーム | **210 行**・最大ネスト **11** | **8 行**（委譲のみ） |
| 新設側の最大ネスト | — | **5** |
| `exec/dispatch.rs` | 415 行 | **214 行** |
| `exec/modules.rs` | 688 行 | **969 行** |

新設したもの:

| 関数 | 役割 |
|---|---|
| `exec_import` | 言語で分岐して名前空間を作り、共通の束縛をする（**23 行**） |
| `import_bind_name` | `as` が無いときの既定の束縛名（cpp だけファイル stem） |
| `import_cpp_module` | `cpp-dll`/`cpp-lib`。キャッシュキーは**ヘッダのパス** |
| `import_cs_dll` / `find_cs_dll_bridge` | `{Name}_native.dll` を探して `load_bridge` |
| `import_cs_proc` / `find_cs_proc_host` | `{Name}_proc.exe` → `{Name}.exe` を探して `launch_proc` |
| `import_js_proc` | `ar_config.json` → ブリッジ起動 → `list_functions` → `JsProcFn` 登録 |
| `inject_class_var` | 名前空間中の全クラスへ class 変数を 1 本注入した複製を返す |

### 3. ⚠ 畳んだ重複 2 件（**どちらも逐語だったことを確かめてから畳んだ**）

**(a) 早期 return 3 箇所 → 共通の末尾。** cs-dll / cs-proc / js-proc は名前空間を作った直後に
**自分で `declare_var` して `return`** していた。その 3 箇所の束縛名は

```rust
alias.clone().unwrap_or_else(|| module.last().unwrap().clone())
```

で、共通の末尾は

```rust
alias.clone().unwrap_or_else(|| if lang == "cpp-dll" || lang == "cpp-lib" { …stem… }
                                else { module.last().unwrap().clone() })
```

⇒ **この 3 言語では `if` が必ず偽**なので**逐語で同じ式**。早期 return を畳んで
「各経路は名前空間を返すだけ」にできる。**畳めた根拠はこの 1 点だけ**なので doc に書いた。

**(b) クラスへのパス焼き込み 10 行が 2 箇所に逐語で存在。** cs-dll の `__cs_bridge_path__` と
cs-proc の `__cs_proc_path__` は、**キー名と値以外が 1 文字も違わなかった** ⇒ `inject_class_var` へ。

### 4. ⚠ 畳まなかったもの（**理由を確かめてから残した**）

`find_cs_dll_bridge` と `find_cs_proc_host` は**似ているが探索順が違う**:

| | cs-dll | cs-proc |
|---|---|---|
| 候補名 | 1 つ（`{Name}_native.dll`） | 2 つ（`{Name}_proc.exe` → `{Name}.exe`） |
| ループの入れ子 | search_dir を全部見てから CWD 側へ | **候補名ごと**に「search_dir 全部 → CWD 側」 |
| 単一セグメント特例 | なし | `<dir>/{Name}/{exe}` と `{Name}/{exe}` を追加で見る |

⇒ 畳むと **`{Name}_proc.exe` が CWD にあり `{Name}.exe` が search_dir にある**ときに
勝つ方が変わる。**挙動が変わる統合はしない**（プランの「2 実装の差／最適化の前提／
本当の非対応の 3 通りがあり、外し方が違う」）。両者の doc に**畳まない理由**を明記した。

### 5. 副産物 — `exec/dispatch.rs` が import の型を知らなくなった

`std::path::PathBuf` / `ModuleState` / `Value` / `use super::*` が**すべて不要になった**
（ぜんぶこのアームのためだけの import だった）。⇒ ディスパッチ表が本当に
「文の種類を振り分けるだけ」のファイルになったことが、import 行からも読める。

### 6. ⚠⚠ 検査網の穴を 2 つ見つけた（**#58 で最も価値のある部分**）

**(a) `import[cs-proc]` を見ている差分ゲートが 1 つも無かった。**
`cs_proc_app.ar` は**唯一の非 GUI の cs-proc 例題**だが、外部プロセスを起こすことを理由に
[compare_bytecode.ps1](compare_bytecode.ps1) の skip リストに入っており、
他のどのゲートも stdout を突き合わせていなかった。
⇒ **3(a) で畳んだ早期 return のうち cs-proc 分は、既存の網では 1 つも検証できなかった。**
新設した [compare_import_paths.ps1](compare_import_paths.ps1) に**明示のコメント付きで**入れた。

**(b) `ReadToEndAsync` でも孫プロセスがパイプを握ると返らない。**
最初に書いた [compare_import_paths.ps1](compare_import_paths.ps1) は
`compare_bytecode.ps1` と同じ `ReadToEndAsync` 方式にしたが、`js_proc_test.ar` で**ハングした**。
`arrow.exe` は終了して `WaitForExit` も返っているのに `$task.Result` が完了しない
— **node のブリッジが孫として生き残り、パイプの書き込み端を握ったまま**だった。
⇒ `Start-Process -RedirectStandardOutput/-RedirectStandardError` で**ファイルへ落とす**方式へ変更。
⚠ **これは #38 の「逐次 `ReadToEnd` のデッドロック」とは別物**（症状は同じ「CPU 0 のまま生存」）。
skill [vm-pitfalls](.claude/skills/vm-pitfalls/SKILL.md) §4 に**見分け方つきで**追記した。

### 7. 検証（**全ゲート緑**）

| ゲート | 結果 |
|---|---|
| **[compare_import_paths.ps1](compare_import_paths.ps1)**（新設） | **10 / 10 identical**（cs-dll・**cs-proc**・js-proc・cpp-dll/lib。⚠ 先に同一 exe の負の対照 10/10） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **108 / 108 byte-identical** |
| `cargo build` | 警告 **0** |
| `cargo test` | **742 passed** |
| `cargo clippy` / `--all-targets` | **52 / 65**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・153 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |
| [ab_bench.ps1](ab_bench.ps1) | 0.977〜1.029x（**退行なし**。1.15x 超の 3 本は 0.02s 台＝プロセス起動床のみ） |

### 8. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「切り出すだけの機械的な移動」 | **半分外れ**。逐語の重複が 2 件（束縛 3 箇所・パス焼き込み 2 箇所）あり、**畳める根拠を確かめる作業が本体**だった |
| 「cs-dll と cs-proc の探索は同じだろう」 | **外れ**。候補名の数・ループの入れ子・単一セグメント特例がすべて違う。**畳んだら挙動が変わる** |
| 「既存ゲートで挙動不変は言える」 | **外れ**。cs-proc を見ている差分ゲートが **0 個**だった（新設して初めて言えるようになった） |
| 「`compare_bytecode.ps1` と同じ方式で書けばよい」 | **外れ**。js-proc の**孫プロセス**が `ReadToEndAsync` を返さなくする（#38 とは別の原因） |
| 「`exec` は全体的に肥大しているのだろう」（起票時） | **当たっていなかったのが判明済み**（起票時に確認）。異常は 1 アームだけで、切り出したら残り 192 行は全部 5〜21 行の委譲 |

---

## #59 同じ木を歩く walker の統合 — 「宣言される名前」の唯一の定義（2026-08-22・完了）

保守性レーン第 2 弾。起票時の実測は「`collect_declared_names` ⇔ `collect_program_globals` に
**14 行の重複が機械検出され、しかも既にドリフト済み**」。

### 1. ⚠⚠ 起票時の手法（「`collect_program_globals` 側へ委譲」）は**成立しなかった**

読み比べた結果、この 2 本は**委譲できる関係ではない**:

| | `collect_declared_names` | `collect_program_globals` |
|---|---|---|
| 再帰 | if/while/for/try の**本体へ降りる** | **降りない**（最上位の並びだけ） |
| `for` ターゲット | 拾う | 見ない |
| `except ... as` の別名 | 拾う | 見ない |
| `NewTypeDef` | **拾わない** | 拾う |
| `Import` / `FromImport` | **拾わない** | 拾う |

⇒ 一方を他方に委譲すると**両方向に挙動が変わる**。**重複しているのは「再帰」ではなく
「この 1 文が直接束縛する名前は何か」という判断だけ**で、それが 6 本に複製されていた。

### 2. 真因は「重複」ではなく「**強制が無いこと**」

6 本とも `match stmt { …宣言アーム… _ => {} }` の形で、`Stmt` に variant を足しても
**コンパイラは何も言わない**。実際に起きたこと:

- #27-c が `EnumDef` を `collect_program_globals` に**だけ**足した
- #68 で `collect_declared_names` の抜けが**実バグとして発火**した

⇒ **直すべきは「同じ判断を 1 箇所にする」ことではなく「足したら気づく形にする」こと。**

### 3. 手法 — 2 段の強制

新設 [src/decl_names.rs](src/decl_names.rs)（173 行）に唯一の定義を置いた。

| 段 | 仕掛け | 効果 |
|---|---|---|
| ① | `each_declared_name` の `match stmt` を **exhaustive**（`_` を書かない） | **`Stmt` に variant を足すとまずここが壊れる** |
| ② | `DeclOrigin`（束縛の出どころ 14 種）を消費側が**全て exhaustive match** | **束縛すると分類した瞬間、全 walker が壊れる** |

⇒ 「この walker では拾うのか・拾わないのか」を**必ず決めさせられる**。
拾わない場合も**バリアントを列挙して理由を書く**ので、**穴が見える形で残る**。

### 4. ⚠ 統合した 4 本／しなかった 4 本（**理由を確かめてから決めた**）

**統合した（既定が危険な側）**

| walker | 旧 `_ => {}` の意味 | 危険度 |
|---|---|---|
| `exec::collect_declared_names` | 自由変数と誤認 → 外側を捕捉 | **#68 の実バグそのもの** |
| `resolver::collect_bound_names` | シャドウを見落とし `Resolution::Global` を誤付与 | **静かに別の値を読む**（`collection.ar` で実測済み） |
| `resolver::collect_program_globals` | グローバルと認識されず VM が bail | 停止する（声は出る） |
| `vm::compiler::decls::collect_nested_decls` | slot が振られず `VmForceError` | 停止する（声は出る） |

**統合しなかった（理由を `decl_names.rs` の doc に表で記録）**

| walker | 理由 |
|---|---|
| `resolver::collect_base_decls` | 答える問いが違う（「**この文を理解できるか**」）。未知の文は `false` を返して解決を諦める＝**既定が安全側** |
| `vm::compiler::decls::scan_shadow_stmts` | 集めるのは `for` ターゲットとの**衝突候補**で、定義文を意図的に入れない |
| `resolver::collect_shadowing_binders` | 自前の判断を持たず `collect_bound_names` へ委譲するだけ（既に 1 本） |
| `exec::collect_referenced_names` | **参照**を集める walker で、宣言とは逆向きの問い |

### 5. ⚠⚠ `for` ターゲットと `except as` 別名は**わざと載せなかった**（採番順の罠）

`collect_nested_decls` の `Try` アームは **「try 本体 → 別名 → handler 本体」の順**に slot を振る。
`each_declared_name` が別名を報告してしまうと、`declare_stmt_slots` が**本体より先に**振るので
**slot 番号が全部ずれ、`LoadLocal` が別の変数を読む**（プランの「採番はリゾルバと同順・同数」）。
⇒ **順序に意味がある束縛はこの列挙器に載せない**と決め、モジュール doc に理由ごと書いた。
同じ理由で `declare_stmt_slots` は**必ず再帰より前に呼ぶ**（従来の順が「宣言 → 部分式 → 本体」）。

### 6. 調査 — `collect_bound_names` の `EnumDef` 抜けは**実バグではなかった**

統合中に「`collect_bound_names` に `EnumDef`/`NewTypeDef` が無い」ことに気づき、
#68 と同型の実バグを疑って 2 本の例題で確かめた:

```
enum Color:            # 最上位
    RED
    GREEN
fn pick()->int:
    if True:           # ← 入れ子ブロックの enum（base decl ではない）
        enum Color:
            BLUE
        let c = Color.BLUE
        return 42
    return 0
```

| 実装 | 結果 |
|---|---|
| Rust（HEAD） | `42` |
| `python -m impl_python` | `42` |

⇒ **差分なし**。理由は **VM コンパイラが注釈より先に `slots` を引く**から
（`collect_nested_decls` が `EnumDef` に slot を振っているので `Resolution::Global` が上書きされる）。
プランの「**注釈は最適化ヒントであって意味論の根拠ではない**」（#15e）が防波堤として効いていた。
⇒ **#59 では拾わないまま保存**し、`DeclOrigin::Enum | D::NewType => {}` のアームに
**この実測と「直すとバイトコードが動くので独立タスク」**を書き残した。

### 7. ⚠⚠ 負の対照 — 仕掛けが**実際に効くこと**を確かめた（#57 の教訓）

「ゲートを作っただけでは守られない」ので、**わざと壊して検知力を測った**（どちらも実施後に復元）:

| 実験 | 結果 |
|---|---|
| `DeclOrigin` に 1 バリアント追加 | **消費者 4 本ちょうど**が `E0004` で停止（`exec/mod.rs:107` / `resolver.rs:164` / `resolver.rs:202` / `vm/compiler/decls.rs:288`） |
| `Stmt` に 1 variant 追加 | `src/decl_names.rs:95` が **6 件のうち最初**に停止 |

⚠ 副産物: `Stmt` を exhaustive に受けている場所は src 全体で **6 箇所**しかない
（`dispatch` / `templates` / `tw_stats` / `ast_value` / `type_check::stmt::check`、そして #59 の `decl_names`）。
**#59 以前は、そのどれも「この文は名前を束縛するか」を問うていなかった。**

### 8. 検証（**全ゲート緑**）

| ゲート | 結果 |
|---|---|
| [compare_bytecode.ps1](compare_bytecode.ps1) | **108 / 108 byte-identical** ＝ **slot 採番が 1 つも動いていない**（#59 で最重要の検査） |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10 identical** |
| `cargo build` | 警告 **0** |
| `cargo test` | **742 passed** |
| `cargo clippy` / `--all-targets` | **52 / 65**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・153 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |
| [ab_bench.ps1](ab_bench.ps1) | 0.987〜1.052x（**退行なし**。バイトコードが同一なので実行時は原理的に不変） |
| #68 の例題（`enum_in_function.ar`） | 通過（クロージャからの enum 読み・入れ子 enum を含む） |

### 9. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「`collect_declared_names` を `collect_program_globals` へ委譲する」（起票時の手法） | **外れ**。再帰範囲・受理する variant・`for`/`except` の扱いが全部違い、委譲すると**両方向に挙動が変わる** |
| 「8 本を 1 本に畳む」 | **過剰**。畳めるのは「1 文が直接束縛する名前」という**判断だけ**で、降り方は本当に違う。4 本は**理由を確かめて残した** |
| 「`collect_bound_names` の `EnumDef` 抜けも #68 と同じ実バグだろう」 | **外れ**。VM が注釈より先に `slots` を引くので覆い隠されていた（2 例題で実測） |
| 「`for` ターゲットも列挙器に載せれば統合が進む」 | **外れ**。`Try` の採番順（本体→別名）が壊れて **`LoadLocal` が別の変数を読む**。載せてはいけない |
| 「行数が減る」 | **外れ**。src は **+51 行**（`decl_names.rs` 173 行の新設ぶん）。#59 が買ったのは行数ではなく**強制** |

---

## #63 `eval_method_call_full` の委譲漏れを解消（2026-08-23・完了）

保守性レーン第 2 弾。起票時の実測は「530 行・深さ 10。**型ごとの切り出しが途中で止まっている**」。

### 1. 何が中途半端だったか（起票時の表を実測で再確認）

`Value::Str` は別ファイル（`string_methods.rs`）へ **1 行で委譲**、`Instance`/`FileObject`/`PyObject`
も 5 行以下で委譲済み。ところが 4 つのレシーバだけがインラインのまま残っていた:

| レシーバ | 前 | 後 |
|---|---|---|
| `Value::Set` | **126 行**（最大アーム） | **1 行** |
| `Value::AsyncManager` | 76 行 | 3 行 |
| `Value::Class` | 71 行 | 1 行 |
| `Value::FrozenList` | 65 行 | 8 行 |

### 2. 手法 — 「1 レシーバ = 1 ファイル」（既存の `string_methods.rs` に合わせた）

| 新設ファイル | 行数 | 中身 |
|---|---|---|
| [set_methods.rs](src/interpreter/classes/set_methods.rs) | 163 | `eval_set_method` ＋ `set_other_items`（**method_call.rs から移設**・消費者は set 演算 6 種だけ） |
| [class_methods.rs](src/interpreter/classes/class_methods.rs) | 102 | `eval_class_method`（`static` / `class_method` ／ cs-dll・cs-proc の static ディスパッチ） |
| [async_manager_methods.rs](src/interpreter/classes/async_manager_methods.rs) | 102 | `eval_async_manager_method`（`all_done` / `wait_for_finish`） |
| [frozen_list_methods.rs](src/interpreter/classes/frozen_list_methods.rs) | 93 | `eval_frozen_list_method`（`fixed_list` のフラットレイアウト操作） |

| | 前 | 後 |
|---|---|---|
| `eval_method_call_full` | **530 行**・最大ネスト **10** | **205 行**・最大ネスト **6** |
| `classes/method_call.rs` | 748 行 | **415 行** |
| 残る最大アーム | `Set` 126 行 | `CsObject` **33 行**（次点 `Namespace` 29・`List` 28） |

副次的に:
- `expect_no_args_evaled` / `one_arg_evaled` を `pub(super)` へ（`classes/` の中だけに閉じる）。
- **`string_methods.rs:16` の orphan doc を修正**（`///` → `#[allow(too_many_lines)]` → `///` の順で
  属性が doc に挟まっていた。#58〜#68 の診断が「src 唯一の `too_many_lines` がこの形」と
  指摘していたもの。#51 が潰した orphan doc と同型）。

### 3. ⚠⚠ `compare_bytecode.ps1` は #63 の証拠にならない（**新しい検査網を足した**）

**#63 はコンパイラを 1 行も触っていない**ので、バイトコードは**自明に**一致する。
つまり「108/108 byte-identical」は #63 について**何も言っていない**。
一方で `eval_method_call_full` は**実行時に全メソッド呼び出しが通る**場所である。

⇒ [compare_outputs.ps1](compare_outputs.ps1) を新設した（**全例題の stdout / stderr / exit code を
2 バイナリで突き合わせる**）。**解釈側（`eval_*` / `exec_*`）を触ったら、挙動不変はこちらで主張する。**

⚠ これは #62（`compile_stmt`）や #65（`eval_str_method`）でも要る。#65 は #63 と同じく
**解釈側だけの変更**になるので、bytecode では検証できない。

### 4. ⚠⚠ 負の対照で 13 例題が落ちた（**先に取ったから気づけた**）

同一 exe 同士で回した初回、**13 例題が DIFFERS** になった。原因は 2 種:

| 原因 | 例題数 | 対処 |
|---|---|---|
| **ベンチの経過時間**（`METRIC …` / `ns/iter` / `µs/call`） | 10 | `bench` 分類を丸ごと除外（速度の A/B は [ab_bench.ps1](ab_bench.ps1) の担当）。⚠ `interop/bench_ab_cdll.ar` は**ディレクトリ除外では漏れる**ので名指し |
| **オブジェクトアドレス**（`<X object at 0x…>` / `id()` の 10 進値） | 3 | **除外せず正規化**（`Normalize-Volatile`） |

⚠ **アドレスの 3 件を「除外」で済ませてはいけなかった** — その 1 つが `collection.ar` で、
**#63 で切り出した `eval_set_method` を見る主要カバレッジ**である。除外していたら
「網はあるが肝心の所を見ていない」状態になっていた（#58 の cs-proc と同じ形）。

### 5. 例題カバレッジの確認（着手前に数えた）

| 切り出したもの | 踏んでいる例題 |
|---|---|
| `eval_set_method` | `collections/collection.ar` ほか |
| `eval_frozen_list_method` | `collections/fixed_list.ar` / `fixed_list_error.ar` / `swd_class.ar` |
| `eval_class_method`（`static fn` / `class_method fn`） | `classes/class_trait.ar` / `class_trait_error.ar`（**この 2 本だけ**） |
| `eval_class_method`（cs-dll / cs-proc static） | `interop/cs_interop_test.ar` / `cs_proc_app.ar`（[compare_import_paths.ps1](compare_import_paths.ps1)） |
| `eval_async_manager_method` | `async/*.ar` 5 本 ＋ `js_proc_async_test.ar` |

⚠ `static fn` / `class_method fn` は **2 例題しか無い**。カバレッジは足りているが薄い。

### 6. 検証（**全ゲート緑**）

| ゲート | 結果 |
|---|---|
| **[compare_outputs.ps1](compare_outputs.ps1)**（新設） | **91 / 91 identical**（⚠ **負の対照 91/91 を取ってから**読んだ） |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10 identical**（cs-dll / cs-proc の static ディスパッチ） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | 108 / 108（⚠ **#63 では自明に一致する**ので証拠としては弱い） |
| `cargo build` | 警告 **0** |
| `cargo test` | **742 passed** |
| `cargo clippy` / `--all-targets` | **52 / 65**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・153 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |
| [ab_bench.ps1](ab_bench.ps1) | 0.989〜1.048x（**退行なし**） |

### 7. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「4 アームを移すだけの機械的作業」 | **おおむね当たり**。ただし `set_other_items`（set 専用ヘルパ）と可視性 2 件が付いてきた |
| 「`compare_bytecode.ps1` で挙動不変を主張できる」 | **外れ**。コンパイラを触っていないので**自明に一致する**＝証拠にならない。**出力ゲートを新設**した |
| 「新しい出力ゲートは素直に緑になるだろう」 | **外れ**。負の対照で **13 例題**が落ちた（ベンチの経過時間 10・アドレス 3） |
| 「揺れる例題は除外すればよい」 | **半分外れ**。`collection.ar` は **set メソッドの主要カバレッジ**なので、除外ではなく**正規化**が正解だった |
| 「`Value::List` / `Dict` も肥大しているだろう」 | **外れ**。28 行 / 17 行しかない。`eval_list_method` / `eval_dict_method` を作る必要は今のところ無い |

---

## #62 `compile_stmt` 800 行の分割（2026-08-23・完了）

保守性レーン第 2 弾。起票時の実測は「**1 関数で 800 行＝ファイル 817 行のほぼ全部**。33 アームだが
偏りが大きい（`For` 97／`AttrCompoundAssign` 76／`CompoundAssign` 75／`Let` 55／`AttrAssign` 48）」。

### 1. 手法

`compile_stmt` は**文種別の振り分け表**であるべきなのに、9 アームが本文を抱えていた。
それらをメソッドへ出し、代入族は新しいモジュールへ分離した。

| | 前 | 後 |
|---|---|---|
| `compile_stmt` | **817 行** | **371 行** |
| 最大アーム | `For` **97 行** | `LoopYield` **34 行** |
| アーム数 | 34 | **33**（`AttrCompoundAssign` の重複アームを統合） |
| `vm/compiler/stmt.rs` | 834 行 | **643 行** |
| `vm/compiler/stmt_assign.rs` | — | **293 行**（新設） |

**新設 [stmt_assign.rs](src/vm/compiler/stmt_assign.rs)（代入族）**: `compile_assign` /
`compile_compound_assign` / `compile_attr_assign` / `compile_attr_compound_assign`。
この族の不変条件は「**ツリーウォークと同じ評価順・同じ再評価回数**」で、そこだけを 1 ファイルに閉じた。

**`stmt.rs` 内のメソッド**: `compile_for` / `compile_let` / `compile_nested_fn_def` / `compile_let_tuple`。

副次的に:
- **`Stmt::AttrCompoundAssign` のアームが 2 つあった**（添字用のパターンガード付き＋一般）のを
  **入口 1 つ**に統合し、中で `Expr::Subscript` / `Expr::Attr` に分岐する形へ。
  同じ文種別を 2 箇所で受けると片方だけ直す事故が起きる（#59 の walker と同じ形）。
- `StoreTarget` の 4 種（グローバル / モジュール名 / `static mut` / セル）に**逐語で同じ 8 行**が
  並んでいたのを `emit_compound_via_stack` へ畳んだ。**出る命令列は 1 バイトも変わらない**。

### 2. ⚠⚠ **偽の 6.5% 退行**を出し、プローブで棄却した（#62 で最も価値のある部分）

[ab_bench.ps1](ab_bench.ps1) が `flat_bench.ar` で **0.926〜0.939x** を出した。
バイトコードは **108/108 byte-identical**、`vm/run.rs` は 1 行も触っていないのに、である。

**手順（プランの判断基準どおり 2 つとも実施した）**

| 段 | 実験 | 結果 |
|---|---|---|
| ① 再現性 | 同じ 2 バイナリで測り直す | **再現**（0.929／0.939） |
| ① 負の対照 | `-A x.exe -B x.exe`（同一 exe） | **1.004x**（＝計測系は安定） |
| ① 区間別 | `flat_bench` の内訳を見る | `fixed_list` 区間は 0.018s で**完全同一**。差は **native↔解釈のコールバック 25M 回**の区間だけ |
| ① 分散 | 各 4 回・区間値で比較 | A: 23.88〜23.97（**分散 0.4%**）／B: 25.43〜26.10 ⇒ **ノイズではない** |
| ② **同規模プローブ** | **`vm/compiler/mod.rs` の `mod` 宣言の順序を逆にしただけ**のビルド | **0.929x**（#62 と同じ） |

⇒ **#62 の内容とは無関係な、コード配置（CGU 分配）の影響**と確定。プローブは
`bottleneck_bench.ar` でも **0.969x** と #62 の値に**完全一致**した。

**機構**: `Cargo.toml` に `[profile.release]` が無い ＝ **`codegen-units = 16`（既定）・LTO 無し**。
CGU の分配は**モジュール構成に依存する**ので、460 行を新モジュールへ移すだけで
インライン化と配置が crate 全体で変わる。⇒ #28 の教訓「**効くのはアーム数ではなくコード配置**」の再演。

⚠⚠ **`flat_bench.ar` は配置ノイズだけで ±7% 動く**（`bottleneck_bench.ar` は ±3%）。
**このベンチでリファクタリングの良し悪しを判断してはいけない。**
理由は「25M 回の native↔解釈コールバック」という、配置に最も敏感な形だから。

### 3. 検証（**全ゲート緑**）

| ゲート | 結果 |
|---|---|
| **[compare_bytecode.ps1](compare_bytecode.ps1)** | **108 / 108 byte-identical**（⚠ #62 は**コンパイラ**の変更なのでこれが主検査） |
| [compare_outputs.ps1](compare_outputs.ps1) | **91 / 91 identical** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10 identical** |
| `cargo build` | 警告 **0** |
| `cargo test` | **742 passed** |
| `cargo clippy` / `--all-targets` | **52 / 65**（増分 **0**。⚠ 下記参照） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・153 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |
| [ab_bench.ps1](ab_bench.ps1) | **配置ノイズの床の中**（上記 ②） |

⚠ **clippy が一度 52 → 53 に増えた**。原因は `compile_nested_fn_def` の引数を
`&Vec<Stmt>` から `&[Stmt]` にしたことで `Rc::from(&body[..])` が
**redundant slicing** になったこと。`Rc::from(body)` へ直して 52 に戻した。
⇒ **総数ではなく増分を見ること**（1 件でも増えたら原因を特定する）が効いた例。

### 4. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「アームをメソッドへ出すだけの機械的作業」 | **おおむね当たり**。ただし `AttrCompoundAssign` の重複アーム統合と `emit_compound_via_stack` の畳み込みが付いてきた |
| 「バイトコードが同一なら A/B は素通りする」 | **外れ**。`flat_bench` が **0.93x** を出し、**再現もした**。プローブを取るまで棄却できなかった |
| 「再現したなら実差だろう」 | **外れ**。**`mod` 宣言の順序を変えただけで同じ 0.929x が出た** ＝ 配置の影響 |
| 「引数を `&[T]` にするのは無害」 | **半分外れ**。`&body[..]` が冗長になり clippy が 1 件増えた（気づけたのは**増分**を見ていたから） |
| 「`compile_stmt` は 300 行台まで落ちる」 | **当たり**（371 行・最大アーム 34 行） |

---

## #61 `run_program` の `ar_config.json` 探索を切り出す（2026-08-23・完了）

保守性レーン第 2 弾。起票時の実測は「パイプライン関数の中に**深さ 9** の探索ループが直書き」。

### 1. 手法

`load_python_search_paths`（祖先ウォーク）と `read_python_search_paths`（1 ファイルの読み取り）へ分割。
後者は `let ... else` で早期 return に反転した。

| | 前 | 後 |
|---|---|---|
| `run_program` | 195 行・最大ネスト **9** | **173 行**・最大ネスト **5** |

### 2. ⚠ 挿入位置を誤って `run_program` の doc を orphan 化した

最初 `fn run_program(` の直前に置いたが、そこは**`run_program` の doc コメントと本体の間**だった。
結果、`run_program` の doc が新関数に付き、clippy が **52 → 54**（`doc_lazy_continuation` ×2）。
⇒ `run_program` の**後ろ**へ置き直して 52 に戻した。
**#51 が潰した orphan doc と同型を自分で作った**（気づけたのは**増分**を見ていたから）。

### 3. 検証（全ゲート緑）

bytecode **109/109** ／ outputs **92/92** ／ import paths **10/10** ／ `cargo test` **742** ／
clippy **52 / 65**（増分 0）／ scan FAIL 0 ／ force_gate 0 件・154 例題 ／ stale_doc_refs 0 件。

---

## #69 `interp_init` の削減 — `ar_config.json` ウォークの遅延化（2026-08-23・完了）

#50 の①（exec 以外）から起票した速度タスク。

### 1. ⚠ 段階 1: **first-touch 仮説は棄却された**

起票時の仮説「`Interpreter::new()` の 0.14〜0.21ms は、プロセス最初のまとまったヒープ確保が
CRT ヒープ／ページのコミットを引き起こす first-touch コストで、**そこが最初だっただけ**」。

**実験**: `Interpreter::new()` の直前に同規模のダミー確保（`String` 64 本 ＋ `HashMap`）を置く。
⚠ **環境変数（`AR_WARM_PROBE`）で切り替える形にした** — 別ビルドで比べると配置差が混ざるため
（#62 の教訓）。同一バイナリ・各 8 回:

| | `interp_new` | `interp_init` | `in_main` |
|---|---|---|---|
| プローブ OFF | 0.152（min 0.137） | 0.378 | 1.865 |
| プローブ ON | 0.155（min 0.139） | 0.400 | 1.885 |

⇒ **`interp_new` は縮まなかった** ＝ first-touch ではなく**実仕事**。
起票時の「そうなら着手範囲から外す」分岐は**不成立**（が、本命は⑤なのでそちらへ進んだ）。

### 2. ⚠⚠ 段階 2 の前に判明 — **起票時の消費者の見立てが誤り**だった

起票時は「`python_search_dirs` の唯一の消費者は Python モジュールの解決」としていた。実際は:

| 実装 | 消費者 |
|---|---|
| `main.rs` のウォーク → `Interpreter::python_search_dirs` | cs-dll / cs-proc / js-proc の**ブリッジ探索**と **`import[py-int]`** |
| `parser/imports/cs_js_modules.rs::python_search_dirs`（**別実装**） | **`import[py]`**（ファイル `.py` の解決。パース時に走る） |

⇒ **`python.search_paths` を読むコードが 2 つある**。しかも探索範囲が違う
（main.rs は**祖先を root まで全ウォーク**／パーサ側は **source_dir と root_dir の 2 箇所だけ**）で、
パーサ側は serde ではなく**自前の文字列走査**で JSON を読む。→ 統合は別タスクへ（下記の起票提案）。

「大多数のスクリプトはこの結果を誰も読まない」という**結論自体は正しかった**ので、方針（遅延化）は変えていない。

### 3. 手法 — (a) 遅延化

`Interpreter` に `config_base_dir: Option<PathBuf>` と
`config_search_dirs: OnceCell<Vec<PathBuf>>` を持たせ、`run_program` は**起点を覚えるだけ**にした。
実際のウォークは `Interpreter::python_search_dirs()` の**初回呼び出し**で走る。

⚠ 順序（明示登録 → 設定由来）は `python_search_dirs()` が `chain` で保つ。
⚠ 消費者 4 箇所を `&self.python_search_dirs` から `self.python_search_dirs()` へ。
`OnceCell::get_or_init` は `&self` で済むので、`&mut self` への波及は無い。

### 4. 実測（`--features prof`・各 8 回の平均）

| | `ar_config_setup` | `interp_init` |
|---|---|---|
| **前**（repo 内・3 階層上で発見） | **0.19〜0.21 ms** | **0.378 ms** |
| **後**（同上） | **0.000 ms** | **0.172 ms** |
| **後**（repo 外・深い階層・不発 ＝ 一番損をしていた形） | **0.000 ms** | **0.178 ms** |

⇒ `interp_init` が **約 2.2 分の 1**。⚠ **process wall では見えない**（spawn 床のゆらぎに埋もれる）ので
主張は段別の数字に限る。⚠ 起票時の釘付け「完全に消しても端点への効果は 0.3〜0.5 ms/実行」は妥当。

⚠ `prof::Sub::CfgWalk` の表示名を `ar_config_walk` → **`ar_config_setup`** に変え、doc に
**「ここは常に ~0.000 ms でなければならない ＝ 0 でなくなったら eager なウォークが復活している」**と書いた
（遅延化を守る回帰検知になる）。

### 5. ⚠⚠ 副産物 1 — 例題が 1 本も無かった（**5 度目の同型**）

`python.search_paths` は**リポジトリ内の全 `ar_config.json` で `[]`** だった。
⇒ **読み取りが壊れても全ゲートが緑のまま**（#58 の cs-proc・#68 の enum と同じ形）。

新設したもの:
- `examples/interop/py_libs/cfg_probe.py`（**`.ar` と同じディレクトリには置かない**）
- `examples/interop/ar_config.json` に `"python": {"search_paths": ["py_libs"]}`
- `examples/interop/import_py_search_path.ar`（**パーサ側**の実装を踏む）
- `examples/interop/import_py_int_search_path.ar`（**インタープリタ側**＝#69 が遅延化した実装を踏む）

⚠ **負の対照で「本当に設定に依存しているか」を確認した**: `search_paths` を空にすると
前者は `ParseError: cannot find module 'cfg_probe'`、後者は `ImportError: ModuleNotFoundError` で落ちる。
⚠ **2 タグを 1 ファイルに書くと参照実装が落ちる**（`cannot assign to immutable variable`）ので 2 本に分けた。
どちらも `impl_python` は `import[py]`/`import[py-int]` の束縛自体が未対応なので `$knownDiff` へ理由つきで登録。

### 6. ⚠⚠ 副産物 2 — **`--features prof` ビルドが #68 以来壊れていた**

`prof_dist.ps1 -Build` が `E0004: non-exhaustive patterns: &Op::EnumDef(_) not covered`
（`src/vm/op_prof.rs`）で失敗し、**古い非 prof バイナリのまま計測が走って全段 0.0 ms** を出した。

`op_prof.rs` は「ワイルドカードを置いていないので、ずれたまま `--features prof` が通ることはない」
という設計だが、**既定ビルドではファイルごと消える**ので #68 が `Op::EnumDef` を足したとき誰も気づかなかった。
⇒ `op.rs` の宣言順から**再生成**して修正（`EnumDef` は宣言順 65 番で、65 以降の索引が全てずれていた）。
`op_prof.rs` の doc に **「op を足したら `--features prof` と `--features tw_stats` も通すこと」**を明記した。

### 7. 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| [compare_bytecode.ps1](compare_bytecode.ps1) | **109 / 109 byte-identical** |
| [compare_outputs.ps1](compare_outputs.ps1) | **93 / 93 identical** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10 identical**（cs-dll/cs-proc/js-proc ＝ 遅延化の主な消費者） |
| `cargo build` / `--features prof` / `--features tw_stats` | いずれも警告 **0** |
| `cargo test` | **742 passed** |
| `cargo clippy` / `--all-targets` | **52 / 65**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・155 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51**（既知差分 43） |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |
| [ab_bench.ps1](ab_bench.ps1) | 退行なし。⚠ `partial_call_overhead.ar` が一度 **0.887x** を出したが、**Reps 5 で 0.998x**・負の対照 1.002x ＝ **単発の外れ値**（#62 の手順で棄却） |

### 8. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「`Interpreter::new()` は first-touch だろう」 | **外れ**。プローブで `interp_new` は縮まず＝実仕事 |
| 「`python_search_dirs` の消費者は Python モジュール解決だけ」 | **外れ**。実際は cs/js ブリッジ探索と `py-int`。`import[py]` は**別実装**を使う |
| 「読み取りの実装は 1 つ」 | **外れ**。**2 つある**（探索範囲も JSON の読み方も違う） |
| 「例題は既にあるだろう」 | **外れ**。`search_paths` は全 config で `[]` ＝ **踏む例題が 0 本**だった |
| 「計測はすぐ始められる」 | **外れ**。`--features prof` が **#68 以来壊れていた**（既定ビルドでは消えるコードなので誰も気づかない） |
| 「遅延化で `interp_init` は半分くらいになる」 | **当たり**（0.378 → 0.172 ms） |

---

## #71 `import[py]` の関数呼び出しが `VmForceError`（実バグ）を修正（2026-08-23）

#69 で `python.search_paths` の例題を書こうとして発見した実バグ。**`import[py]` で読んだ
Python の関数・メソッドが 1 つも呼べない**（属性の読み出しだけが動く）状態だった。

```
import[py] cfg_probe
print(cfg_probe.MARKER)      # → search_path_ok（動く）
print(cfg_probe.describe())  # → VmForceError: cannot compile function 'describe' to bytecode
```

| 実装 | 結果 |
|---|---|
| `target/release/arrow.exe`（#69 時点） | `VmForceError`（最初の関数呼び出しで停止） |
| CPython（参照） | 正常に 8 行 |

### 1. 真因 — **#33 で前提が消えたのに条件がそのまま残っていた**（5 度目）

[functions/execution.rs](src/interpreter/functions/execution.rs) の `exec_fn_evaled`:

```rust
let vm_eligible = !fn_val.is_python && matches!(self_val, None | Some(Value::Instance(_)));
…
if chunk_opt.is_none() {
    return Err(format!("VmForceError: cannot compile function '{}' to bytecode", fn_val.name));
}
```

`!fn_val.is_python` は **#33 以前は「ツリーウォークで実行する」**という意味だった。
#33 でフォールバックを撤去した後は「**`VmForceError` で死ぬ**」に化けていた。

⚠⚠ **#56（`is_builtin_callee` の bail が `parse_ar` を殺していた）と完全に同型**で、
同じ系列の **5 度目**（#41 定義文脈の式／#42 モジュール本体／#56 `parse_ar`／#68 `enum` in fn／#71 `import[py]`）。
コードのコメント自身が「`is_python` の関数はそもそも VM 非適格で `VmForceError` になるので到達しない」と
書いており、**到達不能ではなく死んでいた**ことを取り違えていた。

### 2. ⚠ 「VM に載らない」は誤解だった — 本体は**普通の Arrow AST**

`import[py]` は `python_converter::convert_python_source` で **Python ソースを Arrow の AST へ変換**する
（`parser/imports/py_modules.rs`）。つまり Python 関数の本体は普通にバイトコードへ載る。
`is_python` が意味するのは**引数束縛の規則だけ**:

| | 非 python | python（`is_python`） |
|---|---|---|
| 束縛 | `bind_args`（厳密） | `bind_args_relaxed` |
| `let` パラメータ ＋ mut 引数 | `copy_value` する | **しない**（Python は値を deep copy しない） |
| `self`（非 mut） | deep copy | **しない** |

### 3. 手法

1. `vm_eligible` から `!fn_val.is_python` を外す（＝本体はコンパイルする）。
2. ⚠⚠ **`try_fast_bind` が python を受けないようにする。**
   高速バインドは `let` パラメータ ＋ mut 引数に `copy_value` を掛けるが、一般経路は
   `is_python` のとき掛けない。受けさせると **2 経路で意味論がずれる**
   （プランの「`*_evaled` 版とずれた実装を作らない」＝実バグ 4 回の形）。
   同じ理由で `self` の deep copy も食い違う。
3. 陳腐化したコメント（「到達しない」）を事実に合わせる。

⇒ **修正は実質 2 行**。ただし ② を忘れると「動くが Python の値渡し規則が壊れる」形になる。

### 4. 検証 — **CPython と 1 行ずつ突き合わせた**

`examples/interop/py_libs/cfg_probe.py` に、関数・デフォルト引数・**破壊的変更**・クラスを足し、
`import_py_search_path.ar` から呼んで CPython の出力と比較:

| 形 | Arrow | CPython |
|---|---|---|
| `describe()` | `cfg_probe from py_libs` | 同 |
| `probe(7)` | `22` | 同 |
| `with_default(5, 10)` | `15` | 同 |
| `mutate([1,2])` → 戻り値 / 呼び出し元 | `3` / `[1, 2, 99]` | 同 |
| `Counter(3).bump(4)` / `.get()` | `7` / `7` | 同 |

⚠ **`mutate` と `Counter` が本質的な検査**。前者は「`let` パラメータでも copy しない」、
後者は「`self` を deep copy しない」を見ている。手順 ② を省くとこの 2 つが CPython と食い違う。

⚠ 静的型検査は **Python のデフォルト引数の省略を許さない**（`'with_default' takes 2 argument(s) but 1 were given`）。
これは #71 とは**別の層**なので触っていない（例題では全部明示的に渡している）。

### 5. 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| [compare_bytecode.ps1](compare_bytecode.ps1) | **109 / 110**。⚠ **唯一の差分が修正そのもの**（`import_py_search_path.ar` が 21→147 行＝Python 関数がコンパイルされるようになった） |
| [compare_outputs.ps1](compare_outputs.ps1) | **92 / 93**。⚠ **唯一の差分が修正そのもの**（旧: `VmForceError` → 新: CPython と一致） |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10 identical** |
| `cargo build` / `--features prof` / `--features tw_stats` | いずれも警告 **0** |
| `cargo test` | **742 passed** |
| `cargo clippy` / `--all-targets` | **52 / 65**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・155 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51**（既知差分 43） |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件**（⚠ 一度 1 件出した — 自分のコメントで**削除済みの** `is_builtin_callee` に言及した。マーカー語で解消） |
| [tw_stats.ps1](tw_stats.ps1) | `in_fn` **0**・`vm_ineligible` **0**・`vm_bail_fn` **0**・`tw_control_flow` **0** |

⚠ **他の 109 例題はバイトコードも出力も 1 バイトも変わっていない** ＝ 巻き添えが無いことの裏づけ。

### 6. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「Python 関数は VM に載らないから別経路が要る」 | **外れ**。本体は `python_converter` が作った**普通の Arrow AST**で、そのまま載る |
| 「`vm_eligible` を直すだけ」 | **半分外れ**。`try_fast_bind` を塞がないと copy 規則が 2 経路でずれる（Python の値渡しが壊れる） |
| 「大がかりな修正になる」 | **外れ**。実質 2 行。**難しかったのは修正ではなく「何が壊れているか」の特定**で、それは例題が無かったせい |
| 「既存のゲートで巻き添えが分かる」 | **当たり**。bytecode 109/110・outputs 92/93 で**差分が修正 1 件だけ**と示せた |

---

## #64 `gen_expr_inner` の `Expr::Call` 二重化（2026-08-23・完了）

保守性レーン第 2 弾。起票時の実測は「`gen_expr_inner` 558 行・深さ 9。`Expr::Call` アームが
146 行ある一方、同ファイルに `fn gen_call`（156 行）が別に居る ＝ **呼び出し生成が 2 箇所**」。
起票時のメモは「**まず理由を確かめる**」だった。

### 1. ⚠⚠ 結論 — **統合してはいけない**（2 実装ではなく「2 つの ABI」だった）

まず `gen_call` の呼び出し元を数えたところ、**`Expr::Call` アームの中の 3 箇所だけ**だった
（独立した 2 実装ではなく、アームが**一部を自分で出して残りを委譲している**形）。
中身を読み比べた結果:

| | `Expr::Call` アーム | `gen_call` |
|---|---|---|
| 戻り値 | `(値, ネイティブ型)` | **i64 ハンドル 1 本** |
| 呼び出し規約 | 戻り値が `Ty::Float` のときだけ **`double` を返す `_impl` / `_fast`** を直接呼ぶ | **ハンドル ABI 一本** |
| アリーナ | **触らない**（save/compact を出さない） | `CB_ARENA_SAVE` → `CB_ARENA_COMPACT` を必ず挟む |
| boxing | 無し（スカラーのまま） | 有り |

⇒ **同じ呼び先に対する 2 つの ABI**。片方へ寄せると
**float の高速経路が消える**（アリーナ操作と boxing が復活する）か、逆に
**アリーナの巻き戻しが漏れる**。プランの分類でいう「**最適化の前提**」であって
「2 実装の差」ではない ⇒ **畳まない**。

### 2. ⚠ 重複していたのは「**判断**」のほうだった

emit は違うのに、**どちらへ行くかを決める判断**が各自に書かれていた:

| 重複していた判断 | 箇所 |
|---|---|
| 「この名前はモジュール内関数か」（`module_fns.contains && !locals.contains_key`） | **4 箇所** |
| 「レシーバのクラスは何か」（`self` → `current_class` ／ 識別子 → `param_classes`） | **2 箇所・逐語で同一** |
| 「非 float の戻り値はハンドル ABI ＋ `CB_TO_INT` で開ける」 | **2 箇所・逐語で同一** |
| 「引数ハンドルを entry ブロックの `alloca` 配列へ積む」 | **3 箇所・逐語で同一**（違うのは生成 IR 側の配列名の接頭辞だけ） |

⇒ ここだけを 4 つのヘルパへ畳んだ（`is_module_fn` / `receiver_class` /
`gen_call_unwrapped` / `spill_handle_array`）。**emit は 1 行も変えていない**。

### 3. 手法と規模

| | 前 | 後 |
|---|---|---|
| `gen_expr_inner` | **558 行**・最大ネスト **9** | **414 行**・最大ネスト **6** |
| `gen_call` | 156 行 | **130 行** |
| `gen_call_expr`（新設） | — | 126 行 |
| `llvm_codegen/expr.rs` | 1,011 行 | 1,044 行（**doc を約 90 行足した上で**コード自体は縮小） |

`Expr::Call` アームは `self.gen_call_expr(func, args)` の **1 行**になった。
役割分担の表は `gen_call_expr` の doc に**そのまま書いてある**（次に読む人が同じ調査をしないため）。

### 4. ⚠⚠ 検証は `dump_native_ir.ps1` の **IR byte-identical**（bytecode は無関係）

#64 は**ネイティブ codegen** の変更なので、`compare_bytecode.ps1` は何も言わない
（#63 で「解釈側だけの変更では bytecode が自明に一致する」と書いたのと**逆向きの同じ話**）。
主検査は [dump_native_ir.ps1](dump_native_ir.ps1) の生成 LLVM IR の突き合わせ。

⚠ **使う前に負の対照を取った** — 同一バイナリで 2 回ダンプして **6/6 byte-identical**
（＝ダンプが決定的であることの確認）。そのうえで:

| 段 | 結果 |
|---|---|
| ① アームをメソッドへ切り出した直後 | **6 / 6 IR byte-identical** |
| ② 判断 4 種をヘルパへ畳んだ後 | **6 / 6 IR byte-identical** |

⚠ 代表 6 モジュール（physics 66KB ／ swd_nested ／ typed_abi_module ／ geometry ／
flat_bench_module ／ partial_call_overhead_module）で**全バイト一致**。

### 5. 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| **[dump_native_ir.ps1](dump_native_ir.ps1)** | **6 / 6 IR byte-identical**（⚠ 負の対照 6/6 を先に取得） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **110 / 110**（⚠ #64 では自明。コンパイラを触っていない） |
| [compare_outputs.ps1](compare_outputs.ps1) | **93 / 93 identical** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10 identical** |
| `cargo build` | 警告 **0** |
| `cargo test` | **742 passed** |
| `cargo clippy` / `--all-targets` | **52 / 65**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・155 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件**（⚠ 一度 3 件出した — 下記） |
| [ab_bench.ps1](ab_bench.ps1) | 退行なし（⚠ 判定までに 4 回測り直した — 下記） |

### 5-b. ⚠⚠ **同じ 2 バイナリが 0.897x と 1.007x を出した**（ベンチの信用）

`partial_call_overhead.ar` が **0.897x → 0.922x** を続けて出した。IR は 6/6 byte-identical で
インタープリタ／VM は 1 行も触っていないのに、である。#62 の手順で潰した:

| 段 | 実験 | 結果 |
|---|---|---|
| ① | 再測定（Reps 5） | **0.922x**（再現した） |
| ① | 負の対照（同一 exe） | 1.001x（計測系は安定に見えた） |
| ② | **同規模プローブ**（`llvm_codegen` の `mod` 宣言順を反転しただけ・`expr.rs` は HEAD 同一） | **1.018x**（**再現しない**） |
| ③ | **3 者を同一セッションで交互に測り直す** | **HEAD vs #64 = 1.007x** ／ HEAD vs probe = 1.017x ／ probe vs #64 = 0.995x |

⇒ **0.897/0.922x は実差ではなくセッション間のドリフト**だった。

⚠⚠ **交絡因子は [dump_native_ir.ps1](dump_native_ir.ps1) だった** — 最初の 2 回は
**IR ダンプ（clang を 6 回起動して `.arc`/`.ars` を書き換え・復元する）の直後**に測っていた。
⇒ **IR ダンプの直後にベンチを取らない。** 取るなら計測対象の全バイナリを
**同一セッションで交互に**測ること（負の対照 A vs A は**時間相関のドリフトを打ち消してしまう**ので
「計測系は安定」の証明にならない）。

⚠ #69 でも同じ `partial_call_overhead.ar` が 0.887x を出して外れ値だった。
**このベンチは数 % の判定に使えない**（`flat_bench.ar` の ±7%・#62 と同類）。

### 5-c. stale_doc_refs

⚠ `stale_doc_refs` が `_margs` / `_targs` / `_cargs` を「実在しない識別子」として挙げた。
これらは **生成 IR 側のレジスタ名の接頭辞**で Rust の識別子ではない。
whitelist へ足さず、**バッククォートを外して地の文にした**（マーカー語は行単位で効くので、
同じ行の他の識別子まで検査から外れるのを避けた）。

### 6. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「呼び出し生成が 2 箇所ある＝統合できる」（起票時） | **外れ**。**2 つの ABI**（float 直返し ↔ ハンドル＋アリーナ）で、畳むと生成コードが変わる |
| 「`gen_call` は別経路から呼ばれているだろう」 | **外れ**。呼び出し元は `Expr::Call` アームの中の **3 箇所だけ** |
| 「重複は無い」 | **半分外れ**。emit は違うが**判断が 4 種・計 11 箇所**重複していた（うち 7 箇所は逐語） |
| 「bytecode ゲートで挙動不変を言える」 | **外れ**。ネイティブ codegen なので bytecode は自明に一致する。**IR ダンプが主検査** |
| 「行数は減る」 | **半分外れ**。`gen_expr_inner` は 558→414 だがファイルは +33 行（**doc に役割分担の表を書いたぶん**） |
| 「A/B は素通りする（IR が同一なので）」 | **外れ**。**同じ 2 バイナリが 0.897x と 1.007x** を出した。交絡は **IR ダンプ直後に測ったこと** |

---

## #60 `parse_struct_bodies` のネスト（2026-08-23・完了）

保守性レーン第 2 弾。起票時の実測は「307 行・**最大ネスト 12（src 最悪）**。
`decls.rs:scan_scope` も 405 行・深さ 8 で同型」。

### 1. ⚠⚠ **起票時の行数は間違っていた**（診断ツールの欠陥）

測り直したところ:

| | 起票時の実測 | 実際 |
|---|---|---|
| `parse_struct_bodies` | 307 行・ネスト 12 | **103 行**・ネスト **11** |
| `decls.rs:scan_scope` | 405 行・深さ 8 | **82 行**・深さ **7** |

**真因**: #58〜#68 の診断は「ブレース平衡でスパン抽出（**行コメント・文字列リテラルを除去**してから計数）」
だったが、**文字リテラルを除去していなかった**。この 2 ファイルは C/C++ ヘッダのパーサで
`'{'` / `'}'` / `';'` を**文字リテラルとして大量に持つ**ため、平衡が崩れて
**関数の終わりがファイル末尾まで伸びていた**（307 ≈ 318-13+2、405 ≈ 416-13+2 と符合する）。

⚠ 同じ罠を**自分でも踏んだ**（#60 の測り直しで最初に書いたスクリプトが同じ理由で例外を投げた）。
⇒ **`'{'` を含みうるソースを機械計測するときは文字リテラルも潰すこと。**

⚠ 影響範囲を確認した: 他の起票（#58 `exec` 394／#63 `eval_method_call_full` 530／
#64 `gen_expr_inner` 558／#62 `compile_stmt` 800）は**着手時の実測と一致**していた。
狂っていたのは **cpp_bridge のパーサ 2 本だけ**（文字リテラルを持つのがここだけだったため）。

⇒ **ネスト 11 が src 最悪という起票の趣旨は正しい**（行数だけが誤り）。

### 2. 手法 — 成功パスを早期 `continue` へ反転

**`parse_struct_bodies`**（`structs.rs`）:

| | 前 | 後 |
|---|---|---|
| `parse_struct_bodies` | **103 行**・最大ネスト **11** | **60 行**・最大ネスト **3** |

- `if starts_with('{')` を**反転**（`{` 以外は「区切りを覚えて 1 バイト進むだけ」で `continue`）
- `if let Some(brace_end) = …` を `let … else { i += 1; continue; }` へ
- セグメントの分類を `StructHead`（`Typedef` / `Class` / `Other`）を返す
  `classify_struct_head` へ切り出し
- 積む処理を `push_typedef_structs`（ネスト 3）／ `push_class_struct`（ネスト 1）へ切り出し
- スコープブロック判定を `is_scope_block` へ

**`scan_scope`**（`decls.rs`。起票メモが「同型」と挙げたもの）:

| | 前 | 後 |
|---|---|---|
| `scan_scope` | **82 行**・最大ネスト **7** | **49 行**・最大ネスト **3** |

`extern "C" { … }` / `namespace X { … }` への降下（5 段の入れ子）を
`try_enter_scope` → `try_enter_extern_c` / `try_enter_namespace` → `descend_scope` へ分解。
いずれも `Option<usize>`（消費バイト数）を返し、`?` で平坦化した。**各ヘルパのネストは 0〜1**。

⚠ **`extern` と `namespace` は同じ位置で両方に一致しえない**ので、片方が `None` を返したときに
もう片方を試さなくてよい（切り出し前と同じ挙動になる根拠）。doc に明記した。

### 3. ⚠⚠ **`trim()` と `trim_start()` の取り違えが実バグになる**（doc に残した）

分類側は **`trim_start()` でなければならない**。`.trim()` すると `"typedef union "` の
**末尾の空白が消えて** `" union "` の部分文字列判定が外れ、**union が struct 扱いになる**
（union はフィールドが重なるので `complete=false` にしなければならない）。
一方スコープ判定側は元の `.trim()` のままにしてある。⇒ 両方を `seg_raw` から個別に取る形にした。

### 4. 検証 — **実ヘッダのパース結果を全ダンプして差分 0**

C++ ヘッダのパーサなので、既存ゲートでは弱い。そこで**一時的な全ダンプテスト**を足し、
実ヘッダ 6 本（DxLib.h ほか）のパース結果を**関数シグネチャ・struct・raw レイアウトまで**
書き出して突き合わせた:

| 段 | 結果 |
|---|---|
| ① `parse_struct_bodies` 書き換え後 | **2,827 行のダンプが byte-identical**（2,445 sig ＋ 65 struct） |
| ② `scan_scope` 書き換え後 | **byte-identical** |

⚠ ダンプテストは**その場限り**なので削除し、代わりに**恒久テストを強化**した。
`test_real_dxlib_header_structs` は #60 以前「**VECTOR という名前が居る**」だけを見ており、
**このリファクタリングを丸ごと素通りさせる**空に近い検査だった。
⇒ 宣言数 **2,445** ／ struct 数 **65** ／ `VECTOR` の raw レイアウト（offsets `[0,4,8]`・16 バイト）を固定した。

### 5. ⚠⚠ 負の対照 — **強化した実ヘッダ検査は union 破壊を検知しなかった**

「`trim_start()` → `trim()`」（＝ §3 で警告した取り違え）をわざと入れて測った:

| 検査 | 結果 |
|---|---|
| 強化した `test_real_dxlib_header_structs`（実ヘッダ・件数固定） | **通ってしまう**（DxLib.h に該当形の union が無い） |
| `test_union_incomplete`（合成テスト） | **FAILED**（検知した） |

⇒ **役割分担が要る**: 実ヘッダ検査は「広い回帰（件数・レイアウト）」を、
合成テストは「意味論（union / 継承 / virtual）」を守る。**どちらか片方では網にならない。**

### 6. 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| **実ヘッダ全ダンプ差分**（一時） | **byte-identical**（2 段とも） |
| `cargo test` | **742 passed**（うち cpp_bridge 14） |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10**（cpp-dll / cpp-lib を含む） |
| [compare_outputs.ps1](compare_outputs.ps1) | **93 / 93 identical** |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **110 / 110** |
| [dump_native_ir.ps1](dump_native_ir.ps1) | **6 / 6 IR byte-identical** |
| `cargo build` | 警告 **0** |
| `cargo clippy` / `--all-targets` | **51 / 64**（⚠ **1 件減**。`strip_prefix` を使ったので "stripping a prefix manually" が 10→9） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・155 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ **clippy の基準値が 52/65 → 51/64 へ下がった**。増分 0 ではなく**減少**なので問題ないが、
以降の基準値はこの値。

### 7. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「307 行・ネスト 12 の関数」（起票時） | **外れ**。**103 行・ネスト 11**。診断が**文字リテラルを潰さず**ブレース平衡を崩していた |
| 「`scan_scope` は 405 行」（起票時） | **外れ**。**82 行**。同じ原因 |
| 「起票の趣旨も外れているのでは」 | **当たっていた**。ネスト 11 は**実際に src 最悪**で、反転の価値はそのまま |
| 「既存テストで挙動不変を言える」 | **外れ**。実ヘッダ検査は「VECTOR が居る」だけの空に近い検査だった ⇒ **全ダンプ差分を自分で作った** |
| 「恒久テストを強化すれば十分」 | **半分外れ**。実ヘッダ検査は **union 破壊を検知しない**（合成テストが検知した）。**両方要る** |
| 「clippy は増分 0 になる」 | **外れ**（良い方向）。`strip_prefix` で 1 件**減った** |

---

## #67 `Interpreter` フィールドの部分クラスタ化（2026-08-23・完了）

保守性レーン第 2 弾。起票時の実測は「`Interpreter` はメソッド **215**・フィールド **31**・
`impl` が **33 ファイル**に散る。責務は 7 クラスタ」。起票時の判断は
「**イベントループ 4 本・デバッガ 2 本のみ畳む／全面分解は保留**」。

### 1. 手法 — 起票どおり**部分適用のみ**

着手時のフィールドは **32 本**（#69 が `config_base_dir` / `config_search_dirs` を足していた）。

| 新設した部分構造体 | 置き場所 | 束ねたフィールド |
|---|---|---|
| `event_loop::EventState` | `event_loop.rs`（**消費者の隣**） | `event_loop_data` / `external_event_queue` / `external_handler_registry` / `next_external_signal_id` |
| `debugger::DebugState` | `debugger.rs`（同上） | `dbg_vars` / `dbg_last_span` |

| | 前 | 後 |
|---|---|---|
| `Interpreter` のフィールド | **32** | **28** |
| 初期化（`Interpreter::new()`） | 6 行 | **2 行** |

⚠ **全面分解はしていない**（起票時の判断をそのまま守った）。`impl` が 33 ファイルに散っているので、
スコープ・VM キャッシュ・コールスタックまで動かすと差分が巨大になる。
⇒ **凝集が明らかで参照が少ないクラスタだけ**を畳んだ。その理由を `EventState` の doc に残した。

### 2. なぜこの 2 クラスタだったか（着手前に数えた）

参照箇所は**合計 13 箇所**しかなかった:

| フィールド | 参照数 |
|---|---|
| `event_loop_data` / `external_event_queue` / `external_handler_registry` / `next_external_signal_id` | 2 / 1 / 2 / 2 |
| `dbg_vars` / `dbg_last_span` | 3 / 3 |

⇒ **書き換え量が小さく、凝集が高い**（外部イベントが来る → `external_handlers` で `SignalData` を
引く → `data` のキューへ積む、が必ずセット）。他のクラスタ（スコープ・VM キャッシュ）は
参照が数百あるので同じ判断にはならない。

### 3. ⚠ 可視性で 1 回引っかかった

`DebugState` を `pub(crate)` にしたら **`type Var` is more private than the item** 警告。
`Var` は `interpreter.rs` で `pub(self)`（= `crate::interpreter` 内）なので、
`DebugState` とそのフィールドも **`pub(super)`**（= 同じ範囲）に揃えた。doc に理由を書いてある。

⚠ もう 1 件: `global_ext_queue()` の呼び出しを `EventState::new()` へ移したので
`Interpreter::new()` のローカル `ext_q` が未使用になった（削除）。

### 4. ⚠⚠ `stale_doc_refs` が**旧名への参照 6 件**を捕まえた

フィールドを改名したので、コメント内の `dbg_last_span`（4 箇所）／`external_event_queue`（1）／
`dbg_vars`（1）が実在しない識別子になった。**6 箇所とも現在の名前へ書き換えた**
（`DebugState::last_span` / `EventState::external_queue` / `DebugState::vars`）。

⚠ 参照元は `debugger.rs` だけでなく **`vm/chunk.rs` と `vm/compiler/emit.rs` と `decl_names.rs`** にもあった
（`best_span_for` のフォールバック規約を VM 側が参照している）。
⇒ **改名の影響は「そのフィールドを使っているファイル」より広い**。
プランの教訓「**改名・削除をしたら [stale_doc_refs.ps1](stale_doc_refs.ps1) を必ず走らせる**」が効いた例。

### 5. 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| **[debug_session.ps1](debug_session.ps1)** | **5 identical**（⚠ デバッガの 2 フィールドを動かしたので**これが主検査**） |
| [compare_outputs.ps1](compare_outputs.ps1) | **93 / 93 identical** |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **110 / 110** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10**（cs-dll / cs-proc の**外部イベント**経路を含む） |
| イベント系例題の直接確認 | `event_external_handler.ar`（外部イベント + 一度きりハンドラ + 遅延）・`async_demo.ar` とも正常 |
| `cargo build` | 警告 **0** |
| `cargo test` | **742 passed** |
| `cargo clippy` / `--all-targets` | **51 / 64**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・155 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) | identical |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件**（⚠ 一度 6 件出した — §4） |
| [ab_bench.ps1](ab_bench.ps1) | 0.979〜1.029x（**退行なし**。`flat_bench` 0.979x は #62 で実測した ±7% の配置ノイズ内） |

### 6. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「フィールド 31 本」（起票時） | **ずれ**。着手時は **32 本**（#69 が 2 本足していた） |
| 「33 ファイルに散る `impl` を触ることになる」 | **外れ**。対象 6 フィールドの参照は**合計 13 箇所・8 ファイル**だけ。触ったのは 7 ファイル |
| 「可視性は素直に通る」 | **外れ**。`Var` が `pub(self)` なので `pub(crate)` にすると警告。`pub(super)` へ揃えた |
| 「改名の影響はデバッガ／イベント周辺だけ」 | **外れ**。コメント内の旧名参照が **`vm/chunk.rs` / `vm/compiler/emit.rs` / `decl_names.rs`** にもあった |
| 「フィールド数が目に見えて減る」 | **半分当たり**。32→28（-4）。**得たのは数より「デバッガ／イベントの持ち物だと分かる」こと** |

---

## #72 `python.search_paths` の読み取り 2 実装（2026-08-23・完了）

#69 で発見して起票したもの。起票時のメモは「⚠ **探索範囲が違う**ので畳むと挙動が変わる。**まず差を測る**」。

### 1. 調査 — 読み手は 2 つではなく **5 つ**あり、探索方針は **4 通り**だった

| 読み手 | セクション | 探索方針 | JSON |
|---|---|---|---|
| `Interpreter::python_search_dirs()`（`load_python_search_paths`） | `python.search_paths` | `source_dir` から**祖先を root まで**・**最初の 1 個で打ち切り** | serde |
| `Parser::python_search_dirs()` | `python.search_paths` | **`source_dir` と `root_dir` の 2 箇所だけ** | **手書き走査** |
| `cpp_bridge::config::load_cpp_config` | `cpp.*` | 祖先 ＋ cwd を**全部レイヤーマージ** | 手書き走査 |
| `exec::find_js_config` | `javascript.*` | search_dirs → cwd の**最初** | serde |
| `parser::imports::parse_cs_lib_paths` | `csharp.lib_paths` | 呼び出し側依存 | 手書き走査 |

⚠ **探索方針にはどれも理由がある**:
- `load_cpp_config` がレイヤーマージなのは、**打ち切りだと中間の部分的な設定がルートの
  `cpp` 設定を丸ごと隠す**実バグを直した結果（コメントに経緯あり）。
- `Parser` の 2 箇所は `root_dir` ＝ **エントリのディレクトリ**（サブパーサにも引き継ぐ）という
  パーサ側の契約。
- `Interpreter` の全走査は #61 で「上位が下位を上書きしない」を保つと明記済み。

⇒ **探索方針は畳まない**（#64 と同じ判断: 畳むのは「判断」であって「方針」ではない）。

### 2. ⚠⚠ 差分を実測した（14 ケース → **5 件相違・うち 3 件は手書き側の誤り**）

一時的な差分テストで両実装に同じ JSON を食わせた:

| ケース | 手書き | serde | 判定 |
|---|---|---|---|
| `{"python":{},"rust":{"search_paths":["x"]}}` | `x` を拾う | 空 | **手書きが誤り**（`python` の外なのに拾う） |
| `{"python":{"search_paths":[1,"a"]}}` | `1` をパス扱い | `a` のみ | **手書きが誤り** |
| `{"python":{"search_paths":["a,b"]}}` | `a` と `b` に割る | `a,b` | **手書きが誤り**（配列区切りと誤認） |
| `{"python":{"search_paths":["","a"]}}` | 空要素を捨てる | `base` 自身を返す | **手書きが妥当** |
| 壊れた JSON | 拾えるだけ拾う | 空 | 議論の余地（手書きが寛容） |

⚠ 手書き走査の根本問題は「**`"python"` を見つけた後ろで `"search_paths"` を探すだけ**」で、
**セクションの内側かどうかを見ていない**こと。

⚠ 副次的な観察: `"C:\\x\\y"` は表示上ずれるが `PathBuf` としては**等しい**
（Windows のパス比較は連続セパレータを潰す）。**文字列で比べると誤検知する**。

### 3. 手法 — **JSON の読み取りだけを畳む**

新設 [src/ar_config.rs](src/ar_config.rs)（**共有**）へ:
- `load_python_search_paths`（祖先ウォーク・`main.rs` から移設）
- `read_python_search_paths` / `read_python_search_paths_from_str`（serde）

そして:
- `Parser::python_search_dirs()` を `crate::ar_config::read_python_search_paths` へ委譲
- 手書きの `parse_python_search_paths` を**削除**
- **空文字列要素は捨てる**方へ揃えた（手書き側が妥当だったので serde 版を寄せた）
  ⇒ 残る差分は**「手書きが誤っていた 3 件」＋「壊れた JSON」だけ**＝ 変更は**すべて修正方向**

⚠ `main.rs` に置いたままだと「パーサが `main` を呼ぶ」形になるので、
**crate 直下の小さな共有モジュール**へ出した（#59 の `decl_names.rs` と同じ形）。

⚠⚠ **モジュール doc に「5 つの読み手 × 4 通りの探索方針」の地図を書いた**。
これが #72 の一番の成果 — 次の人が同じ調査をしなくて済む。

### 4. 恒久テストを新設（負の対照つき）

`ar_config::tests` に 4 本。うち `serde_reader_fixes_hand_rolled_scanner_bugs` は
**削除した手書き実装が取り違えていた 4 形**をそのまま固定してある（また同じ実装を書かないための番人）。

⚠ **負の対照で検知力を確認した**: `python` セクションのスコープ確認をわざと外す
（＝手書きと同じ誤り）と、**この 1 本だけが FAILED** になる。

⚠ もう 1 件、テストが最初 FAILED した — `PathBuf::from("/abs").is_absolute()` は
**Windows では偽**（ドライブが無い）。`cfg!(windows)` で分けた。

### 5. 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| `cargo test` | **746 passed**（+4 = 新設テスト） |
| `import_py_search_path.ar`（パーサ側）／`import_py_int_search_path.ar`（インタープリタ側） | 両方とも従来どおり |
| [compare_outputs.ps1](compare_outputs.ps1) | **93 / 93 identical** |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **110 / 110** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10** |
| `cargo build` | 警告 **0** |
| `cargo clippy` / `--all-targets` | **51 / 64**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・155 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

### 6. ⚠ 残した課題（#72 では触らない）

- **`python` の 2 つは探索方針が食い違ったまま**（祖先全走査 ↔ 2 箇所）。
  中間の祖先にある設定は `import[py-int]` からは見えて `import[py]` からは見えない。
  ⇒ 揃えるなら独立タスクで、**先に「どちらへ揃えるか」を決めること**。
- **`parse_cs_lib_paths`（`csharp.lib_paths`）も同じ手書き走査**で、しかも
  **セクション名で絞ってすらいない**（`"lib_paths"` を全文検索）。同じ 3 つの誤りを持つはず。
  ⇒ `ar_config` に `read_cs_lib_paths` を足して委譲できる（別タスク）。
- `cpp_bridge::config` の手書き JSON パーサ（`extract_json_object` 等）も同系統。
  ただしこちらは**レイヤーマージ**という別の要件を持つので、単純委譲にはならない。

### 7. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「読み取り実装は 2 つ」（起票時） | **外れ**。**5 つの読み手・4 通りの探索方針**があった |
| 「探索範囲が違うので畳めない」（起票時） | **当たり**。しかも**それぞれに理由がある**（cpp のレイヤーマージは実バグの修正結果） |
| 「差は些細だろう」 | **外れ**。14 ケース中 **5 件相違**、うち **3 件は手書き側の実バグ**（セクション外の拾い上げ等） |
| 「委譲すれば挙動が変わる」 | **半分外れ**。空文字列の扱いを手書き側へ寄せたので、**残る変更はすべて修正方向**になった |
| 「テストは素直に通る」 | **外れ**。`/abs` は **Windows では絶対パスでない**（`is_absolute()` が偽）ので 1 本落ちた |

---

## #73 `csharp.lib_paths` も手書き走査（2026-08-23・完了）

#72 で発見して起票したもの。起票時のメモは「⚠ **セクション名で絞ってすらいない**＝
#72 と同じ 3 つの誤りを持つ**はず**」。⇒ 仮説なので**まず測った**。

### 1. 実測 — 11 ケース中 **6 件相違・うち 5 件が手書き側の誤り**（#72 より 1 つ多い）

| ケース | 手書き | serde（`csharp` で絞る） | 判定 |
|---|---|---|---|
| `{"lib_paths":["top"]}`（**セクション無し**） | `top` を拾う | `None` | **手書きが誤り** |
| `{"cpp":{"lib_paths":["cpp1"]},"csharp":{"lib_paths":["cs1"]}}` | **`cpp1` を返す** | `cs1` | **手書きが誤り（最悪）** |
| `{"cpp":{"lib_paths":["cpp1"]}}` | 拾う | `None` | **手書きが誤り** |
| `{"csharp":{"lib_paths":[1,"a"]}}` | `1` をパス扱い | `a` のみ | **手書きが誤り** |
| `{"csharp":{"lib_paths":["a,b"]}}` | `a` と `b` に割る | `a,b` | **手書きが誤り** |
| 壊れた JSON | 拾えるだけ拾う | `None` | 寛容（議論の余地） |

⚠⚠ **`WRONG-SECTION` が最も悪い** — `cpp.lib_paths` と `csharp.lib_paths` が両方あると
**C# の DLL 探索に C++ 用のパスを使う**。`parse_python_search_paths` は少なくとも
`"python"` の**後ろ**を探していたが、`parse_cs_lib_paths` は `"lib_paths"` を**JSON 全文検索**していた。

⚠ 現状は潜在的（`cpp` の設定キーは `lib_patterns` / `system_libs` で `lib_paths` は使っていない）。
**だが 1 キー足されたら発火する。**

### 2. 手法 — #72 と同じく**読み取りだけ**を [ar_config.rs](src/ar_config.rs) へ

- `read_cs_lib_paths_from_str`（serde・`csharp` セクションで絞る）を追加
- 配列 → 絶対パス列の変換を `resolve_path_array` に切り出し、**python 版と共有**
  （空文字列と非文字列を捨てる規則が 1 箇所になった）
- `Parser::load_cs_lib_paths` は委譲。**探索方針（祖先ウォーク）はそのまま**
- 手書きの `parse_cs_lib_paths` を**削除**

⚠ **パスを取る版（`read_cs_lib_paths`）は作らなかった**。呼び出し側は
「**存在するが読めない設定ファイルのときはさらに上へ遡る**」という python 側と違う方針を持っており、
読み込みを自分で行う必要がある。一度作ったが未使用警告が出たので削除した
（`#[allow(dead_code)]` で黙らせない）。その理由を doc に書いてある。

### 3. ⚠⚠ エンドツーエンドの例題は**作らなかった**（理由を記録）

`csharp.lib_paths` を踏む例題はリポジトリに **0 本**（`lib_paths` を書いた設定も 0 個）。
作るには DLL を**既定の候補パスの外**に置く必要があり、さらに実行時の
`{Name}_native.dll` ブリッジも既定外になるため、**リポジトリにバイナリを増やす**ことになる。
⇒ 割に合わないと判断し、代わりに:

- **探索まで含む単体テスト**（`cs_lib_path_search_tests`）— 一時ディレクトリに設定を置き、
  祖先ウォークで届くこと・相対パスが**設定ファイルの場所**基準で解決されることを固定
- **パースの単体テスト**（`ar_config::tests::cs_reader_requires_the_csharp_section`）—
  削除した手書き実装が取り違えていた**6 形すべて**を固定

⚠ **負の対照で検知力を確認**: `csharp` セクションのスコープ確認をわざと外すと、
**この 1 本だけが FAILED** になる。

### 4. 検証（全ゲート緑）

| ゲート | 結果 |
|---|---|
| `cargo test` | **750 passed**（+4 = 新設テスト） |
| [compare_outputs.ps1](compare_outputs.ps1) | **93 / 93 identical** |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **110 / 110** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **10 / 10**（cs-dll / cs-proc を含む） |
| `cargo build` | 警告 **0** |
| `cargo clippy` / `--all-targets` | **51 / 64**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・155 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件**（⚠ 一度 1 件 — 削除した `parse_cs_lib_paths` への言及にマーカー語を付けた） |

### 5. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「#72 と同じ 3 つの誤りを持つはず」（起票時） | **当たり、かつそれ以上**。**5 件**で、うち「別セクションの値を返す」が最悪（#72 には無い形） |
| 「例題を足せる」 | **外れ**。DLL ＋ ネイティブブリッジを既定外に置く必要があり、**バイナリが増える**。単体テスト 2 種で代替し理由を記録 |
| 「python 版と同じ形にできる」 | **半分外れ**。パスを取る版は**呼び出し側の読み込み失敗ポリシーが違う**ので作れない（未使用警告で気づいた） |
| 「`cpp` セクションが `lib_paths` を使っている」 | **外れ**。使っているのは `lib_patterns` / `system_libs`。⇒ `WRONG-SECTION` は**潜在**（1 キー足されたら発火） |

---

## #74 `python.search_paths` の探索方針を祖先ウォークへ統一（2026-08-23・完了）

#72 で確認して起票したもの。起票時のメモは「⚠ **先にどちらへ揃えるかを決める**。
中間の祖先の設定が `import[py]` から見えない」。

⚠⚠ **本系列で初めて「意図的に挙動を変える」タスク**。

### 1. 実証 — 同じディレクトリ・同じ設定で**タグによって結果が違った**

`examples/interop/py_subdir/`（**自分の `ar_config.json` を持たない**）から実行:

| | 結果 |
|---|---|
| `import[py] cfg_probe` | **`ParseError: cannot find module 'cfg_probe'`** |
| `import[py-int] cfg_probe` | `search_path_ok`（**通る**） |

原因はパーサ側の探索方針:

```rust
let config_search = if self.source_dir == self.root_dir {
    vec![self.source_dir.clone()]            // ← 直接実行するとここだけ
} else {
    vec![self.source_dir.clone(), self.root_dir.clone()]
};
```

`py_subdir/x.ar` を直接実行すると `source_dir == root_dir == py_subdir` になるので
**1 つ上の `examples/interop/ar_config.json` に到達できない**。
インタープリタ側は最初から祖先を root まで遡っていたので通っていた。

### 2. どちらへ揃えるか — **ウォーク側**（5 つ中 4 つが既にウォーク）

| 読み手 | 方針 |
|---|---|
| `Interpreter::python_search_dirs()` | 祖先ウォーク |
| `Parser::load_cs_lib_paths()` | 祖先ウォーク |
| `cpp_bridge::config::load_cpp_config` | 祖先ウォーク（＋レイヤーマージ） |
| **`Parser::python_search_dirs()`** | **2 箇所だけ ← 異端** |
| `exec::find_js_config` | search_dirs → cwd |

⇒ 2 箇所ルールが異端。**ウォーク側へ寄せた。**

⚠ `root_dir`（エントリのディレクトリ）は **`source_dir` の祖先とは限らない**
（検索パス経由で読まれるモジュールなど）ので、**空振りしたときのフォールバックとして残した**。
新しい方針は「`source_dir` から祖先へ遡って最初の設定 ／ 無ければ `root_dir` から祖先へ」。

⚠ 「最初に見つかった**設定ファイル**で確定（中身が空でも遡らない）」という既存の意味論を保つため、
`ar_config` に **`find_ancestor_config`（ファイルを探すだけ）** を切り出し、
`load_python_search_paths` もそれを使う形にした（ウォークの実装も 1 本になった）。

### 3. ⚠ `load_cs_lib_paths` は畳まなかった（理由を doc に記録）

`find_ancestor_config` は「最初に**存在する**設定ファイルで打ち切り」だが、
`load_cs_lib_paths` は「**存在するが読めない**ときはさらに上へ遡る」という独自の挙動を持つ（#73 で確認）。
意図的な差として残し、`find_ancestor_config` の doc に書いた。

### 4. 例題を新設（#74 の回帰を止める唯一の網）

`examples/interop/py_subdir/import_py_ancestor_config.ar` —
**同じファイルで両タグを使い、両方が同じ設定を見る**ことを固定する。

⚠⚠ **ゲートへの登録が要った**:
- `compare_outputs.ps1` / `compare_python_impl.ps1` は `examples/<cat>/*.ar` の**非再帰グロブ**なので
  **サブディレクトリの例題を見ない**。
- ⇒ import 解決の検査である [compare_import_paths.ps1](compare_import_paths.ps1) へ登録した。
  ついでに `import_py_search_path.ar` / `import_py_int_search_path.ar` も**未登録だった**ので追加
  （対象 10 → 13 本）。

⚠ **負の対照で確認**: 祖先ウォークを外して `source_dir` だけ見る形に戻すと、
この例題は **ParseError に戻る**。

### 5. 検証（**差分は意図した 1 件だけ**）

| ゲート | 結果 |
|---|---|
| [compare_import_paths.ps1](compare_import_paths.ps1) | **12 / 13**。⚠ **唯一の差分が修正そのもの**（新例題が旧 ParseError → 正常） |
| [compare_outputs.ps1](compare_outputs.ps1) | **93 / 93 identical**（巻き添えなし） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **110 / 110** |
| `cargo build` | 警告 **0** |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **51 / 64**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・156 例題**（+1） |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ **広がる方向の変更なので巻き添えを気にした**が、リポジトリ内の設定は
`ar_config.json`（root・`search_paths: []`）と `examples/interop/ar_config.json`（`["py_libs"]`）だけで、
前者は空なので「祖先で root の設定を見つける」ようになっても**追加されるパスは無い**。
実測でも outputs 93/93・bytecode 110/110 が同一だった。

### 6. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「中間の祖先の設定が見えない」（起票時） | **当たり、かつもっと狭い** — `source_dir == root_dir` なら**1 つ上すら**見えなかった |
| 「どちらへ揃えるか迷う」 | **迷わなかった**。5 つの読み手のうち **4 つが既にウォーク**で 2 箇所ルールが異端 |
| 「`root_dir` は不要になる」 | **外れ**。`source_dir` の祖先とは限らないのでフォールバックとして必要 |
| 「例題を足せばゲートが拾う」 | **外れ**。`compare_outputs` / `compare_python_impl` は**非再帰**でサブディレクトリを見ない ⇒ `compare_import_paths` へ明示登録が要った |
| 「巻き添えが出るかもしれない」 | **出なかった**（root の設定が空だったため）。outputs / bytecode とも同一 |

---

## #70 最上位ループの型特化・融合（2026-08-24・完了）

#50 の実行時間分布から起票した速度タスク。起票時の実測は
「fn 内比 **2.6x**・原因は `try_emit_bin_fused` の `as_local` 前提と確定」。

### 1. ⚠ まず例題を新設した（起票時の指示）

`examples/bench/bench_toplevel_loop.ar` — **同一プログラム内**で「fn 内ループ」と
「最上位ループ」を測り、比を出す。

⚠⚠ **他の bench は全部「ホットループは必ず fn の中に置く」と冒頭に書いてあり**
（#10-b で最上位も VM 化された後もその文言が残っていた）、
**素直に最上位へ書くユーザーのコードを測るものが 1 本も無かった**。
⇒ その古い記述も `bench_arith.ar` / `bench_branch.ar` で訂正した。

⚠ **同一プログラム内 A/B** にしたのは、別プロセス比較だとプロセス生成の床（±2ms）と
コード配置ノイズ（#62 で ±7% を実測）に埋もれるため。

### 2. 実測 — 命令列を dump して原因を確定

```
# while i < N:  /  i += 1   （最上位）
0  LOAD_GLOBAL i      3  JUMP_IF_FALSE      6  IBIN_SS Add
1  LOAD_GLOBAL N      4  LOAD_GLOBAL i      7  STORE_GLOBAL i
2  IBIN_SS Lt         5  CONST 1            8  JUMP          ← 9 命令/反復
```
fn 内は `IntBinLL` / `IntBinLC; StoreLocal` で **5 命令/反復**。

### 3. ⚠⚠ 見立ては**半分外れていた** — 効いたのは命令数ではなく **`Value` の clone**

融合 op（`IntBinGG` / `IntBinGC`）を足して **9 → 5 命令**（fn と同数）にしたが:

| | ratio（最上位 / fn） |
|---|---|
| 元 | 2.295 ／ 2.499 |
| **融合のみ**（命令数は fn と同数） | 1.955 ／ 2.338 ← **ほとんど縮まない** |
| **融合 ＋ clone 回避** | **1.188 ／ 1.342** |

命令数を揃えても 2x が残った ⇒ 原因はグローバル読みの**中身**だった。
`Var::get_value()` は必ず `Value` を **clone** するが、slot 版 `IntBinLL` は
`&buf[..]` を**参照のまま match** して clone しない。
⇒ 「索引キャッシュが当たっていて両方 int なら clone せず `i64` を取り出す」速い経路を足した
（`vm_global_int_by_slot` / `cached_global_int`）。

⚠ **これが本命だった**。融合だけなら 1.05〜1.16x、clone 回避まで入れて **1.80〜1.86x**。
プランの「推測せず先に計測する」が効いた例で、**op を足す前に clone を疑うべきだった**。

### 4. 手法

| 追加 | 中身 |
|---|---|
| `Chunk::global_refs: Vec<(u32, SlotCache)>` | **グローバル参照表**。op が u32 1 本で「どのグローバルか」を指す |
| `Op::IntBinGG(u32, u32, BinOp)` | グローバル `<op>` グローバル |
| `Op::IntBinGC(u32, u32, BinOp)` | グローバル `<op>` 定数 |
| `Interpreter::vm_global_int_by_slot` | **clone しない** int 読み |
| `run::cached_global_int` | キャッシュ命中時限定の clone なし読み |

⚠⚠ **op に `(name_idx, cache_idx)` を直接持たせなかった**のは、`Op` が **20 バイト**を超えると
**全命令が太る**から（`op_size_is_pinned`）。副表へ逃がす判断は `ffi_call_info` / `wb_targets` と同じ。
**変更後も `Op` は 20 バイトのまま**。

⚠ **int 特化が確定しているときだけ**融合する。Float / 型不明のグローバル版 op は足していない
（op を増やすほど命令列のキャッシュ密度を削るので、**実測で効くと分かった形だけ**に絞った）。

⚠ 融合の可否判定（`global_read_name`）は **`compile_expr` の `Expr::Ident` アームと同じ順序**
（cells → statics → `Resolution::Local` → `Resolution::Global`）でなければならない。
間違えると**別の記憶域を読む**。`add_global_ref` / `add_const` は副表に積む副作用があるので、
**判定を全部済ませてから積む**（途中で諦めると使われない枠が残る）。

### 5. ⚠ 3 者比較で判定した（#27 の要求）

op を足す規模の変更は「1 命令も実行しなくてもベンチが動く」ので、
**op は足すが融合を一切発行しない プローブ**を作って 3 本で比べた:

| bench | ① HEAD vs プローブ<br>（op 追加の摂動だけ） | ② HEAD vs #70 | ③ **プローブ vs #70**<br>（**純粋な融合の効果**） |
|---|---|---|---|
| `bench_toplevel_loop` | 1.111x | 1.392x | **1.212x** |
| `bottleneck_bench` | 1.021x | 1.108x | **1.090x** |
| `bench_string` | 1.034x | 1.089x | **1.047x** |
| `bench_name_hash` | 1.059x | 1.103x | **1.044x** |
| その他 18 本 | 0.977〜1.003x | 0.970〜1.029x | — |

⇒ **op 追加の摂動は 1〜2%**（①）、**融合の実効は 1.04〜1.21x**（③）。
⚠ 新設ベンチの信用は**内部 METRIC のほう**（②の 1.392x はプロセス wall なのでばらつく）。
内部計測では ratio **2.295 → 1.188** ／ **2.499 → 1.342**、絶対値で
**73.6 → 39.6 ns/iter（1.86x）** ／ **113.3 → 62.9（1.80x）**。

### 6. ⚠ バイトコードは**意図的に変わる**（挙動不変は出力で見る）

| ゲート | 結果 |
|---|---|
| **[compare_outputs.ps1](compare_outputs.ps1)** | **93 / 93 identical** ← **挙動不変の主検査** |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **93 / 111**（18 例題で差分）。⚠ **全部 B のほうが短い**（例: `control_flow.ar` 1443→1397 行）。目視で `LOAD_GLOBAL; CONST; IBIN_SS` → `IBIN_GC` と飛び先の調整だけと確認 |
| [compare_import_paths.ps1](compare_import_paths.ps1) | 12 / 13（差分は **#74** のもの。基準バイナリが #72〜#74 より前） |
| `cargo build` / `--features prof` / `--features tw_stats` | いずれも警告 **0** |
| `cargo test` | **750 passed**（`op_size_is_pinned` **20 バイトのまま**） |
| `cargo clippy` / `--all-targets` | **51 / 64**（増分 **0**） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・157 例題**（+1） |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ **`op_prof.rs` が `--features prof` で落ちた**（#69 で直した仕掛けが正しく発火）。
既定ビルドでは消えるファイルなので、**op を足したら feature ビルドも通す**を実際に守って再生成した。

### 7. ⚠ 採らなかった案とその理由

- **(a) 帰納変数の slot 昇格**（起票時の第 1 候補）… **採らなかった**。
  ループ本体から呼ばれた関数が**同じグローバルへ代入できる**（#39 で入った機能）ので、
  slot へ写して最後に書き戻すと**更新が消える**。呼び出しを含むループは実際のコードでは普通なので、
  「本体に呼び出しが無いときだけ」に絞ると**ベンチしか通らない**。
  例外・`break` の可視性やデバッガからの観測も同じ問題を持つ。
- **Float / 型不明のグローバル融合** … 足していない（実測で効くと分かった形に絞る）。

### 8. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「原因は `as_local` 前提＝命令数」（起票時） | **半分外れ**。命令数を fn と同数にしても **2x が残った**。本命は `Value` の clone |
| 「(a) slot 昇格が第 1 候補」（起票時） | **採れない**。呼び出し先がグローバルへ代入できるので更新が消える |
| 「op を足すと `Op` が太るかも」 | **回避できた**。`(name, cache)` を副表へ逃がして **20 バイト維持** |
| 「op 追加の摂動が大きいかも」（#27） | **1〜2%**（プローブで実測）。融合の実効 1.04〜1.21x が上回る |
| 「効くのは新設ベンチだけ」 | **外れ**。既存の `bottleneck_bench` +9%・`bench_string` +5%・`bench_name_hash` +4%、**18 例題でバイトコードが短くなった** |

---

## #65 `eval_str_method` 48 アーム（2026-08-24・完了。ただし**定義を見直した**）

保守性レーン第 2 弾の最後。起票時の実測は「581 行だが **48 個の `"method" =>` が並ぶ
平坦な組み込みメソッド表**（平均 12 行）。**優先度低**（平坦な表・得るのは行数だけ）」。

### 1. ⚠⚠ 起票時の手法（カテゴリ別サブ関数へ）は**採らなかった**

測り直した結果、起票時の診断どおり**完全に平坦な表**だった:

| | `eval_str_method` | `exec_op`（**対象外**と判断済み） |
|---|---|---|
| 行数 | 581 | 836 |
| アーム数 | **48** | 86 |
| 最大アーム | **32 行** | 42 行 |
| 中央値 / 平均 | **9 / 10 行** | — |

⇒ **`exec_op` を対象外にしたのと同じ形**（「大きいが平坦で、大きいことに理由がある」）。
カテゴリ別に割っても**得るのは行数だけ**で、メソッド名から実装への直行性が落ちる。
⇒ **表は平坦なまま残す**（プランの「タスクの定義が目的に対して過剰なら定義を見直す」）。

### 2. ⚠ 代わりに**重複**を測ったら、そこから**実バグが 2 件**出た

6 行窓の正規化ハッシュ一致で重複を検出したところ **20 組**。中身を見ると:

| 重複 | 判断 |
|---|---|
| `ljust` / `rjust` / `center` の**幅と埋め文字の解析が逐語で 3 回** | **畳んだ**（`pad_args!`） |
| `split` / `rsplit` が 9 割同じ | **畳まなかった**（下記・理由を doc に記録） |

**畳んだ側から実バグが 2 件**出た（3 回書いた結果、3 つが別実装になっていた）:

| # | バグ | 旧 | CPython |
|---|---|---|---|
| ① | 埋め文字つき `ljust` が**元の文字列内の空白まで置換**していた | `"a b".ljust(6,"*")` → `a*b***` | `a b***` |
| ② | `rjust` / `center` が**バイト長**で幅を判定 ⇒ マルチバイトが**まったく詰められない** | `"あい".rjust(5,"*")` → `あい` | `***あい` |

①の原因は `ljust` だけが「空白で幅詰め → 空白を埋め文字へ `replace`」という別実装だったこと。
②の原因は `s.len()`（バイト）を使っていたこと（`ljust` は `format!("{:<width$}")` で文字数だったので**そこだけ正しかった**）。
⇒ 3 つとも `pad_args!` ＋ `chars().count()` ＋ `repeat(pad)` に揃えた。

### 3. ⚠⚠ **clippy が 3 年ぶんの受け入れ済み警告の中でバグを指していた**

修正後 clippy が **51 → 50** に減った。消えたのは
**`replacing text with itself`**（`replacen(&fill, &fill, width)`）＝ **まさにバグの行**。

⇒ **「増分 0」を守るだけでは足りない。受け入れ済みの baseline にも実バグへの指差しが埋もれている。**
本系列は #58 以降ずっと「clippy 51/64・増分 0」を緑としてきたが、その 51 の中にこれがあった。

### 4. `split` / `rsplit` は**畳まなかった**（理由を doc に記録）

9 割同じだが、違う 3 点が**どれも CPython に合わせるために必要**だった（突き合わせて確認）:
① `splitn` ↔ `rsplitn`（＋ `reverse()`）／② 空白区切り＋maxsplit で `split` だけ `retain(非空)`／
③ 区切り指定＋maxsplit で `rsplit` だけ `reverse()`。

⇒ 「reverse するか」1 フラグでは表せず、**畳むと非対称の理由が読めなくなる**。
代わりに**なぜ非対称かと検証済みのケースを doc に書いた**（#64 で「畳めるのは判断であって方針ではない」と
学んだのと同じ形）。

### 5. 例題 — 既存の例題は**バグを踏まない形**だった

`examples/basics/built_in.ar` は既に `ljust`/`rjust`/`center` を呼んでいたが、
**`"hi"`（空白なし ASCII）** だったので **2 件とも発火しなかった**（「例題が踏む形しか見ない」の再演）。
⇒ 同じ場所に `"a b"` と `"あい"` のケースを足した。

⚠ **参照実装では検査できない** — `impl_python` は `ljust` 自体を実装していない
（`AttributeError: 'str' object has no attribute 'ljust'`）。`built_in` は元から `$knownDiff`。
⇒ **意味論は CPython と直接突き合わせて**確認し、回帰は [compare_outputs.ps1](compare_outputs.ps1) が止める。

### 6. 検証（**差分は修正した 2 バグだけ**）

| ゲート | 結果 |
|---|---|
| **[compare_outputs.ps1](compare_outputs.ps1)** | **92 / 93**。⚠ **唯一の差分が修正そのもの**（`a*b***`→`a b***`、`あい`→`***あい`/`--あい--`） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **111 / 111**（解釈側だけの変更なので当然） |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **13 / 13** |
| CPython との直接比較 | ljust / rjust / center / split / rsplit の 12 ケース**全一致** |
| `cargo build` | 警告 **0** |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **50 / 63**（⚠ **1 件減** — バグ行を指していた警告が消えた） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・157 例題** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **51/51**（⚠ `built_in` は元から既知差分＝**この修正は見ていない**） |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ **clippy の基準値が 51/64 → 50/63 に下がった**。以降の基準はこの値。

### 7. 規模

| | 前 | 後 |
|---|---|---|
| `eval_str_method` | 581 行 | **588 行**（doc を足したぶん **+7**） |
| アーム数 / 最大アーム | 48 / 32 | **48 / 32**（変えていない） |

⇒ **行数は増えた**。#65 が買ったのは行数ではなく**実バグ 2 件の修正と、なぜ平坦なままかの説明**。

### 8. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「カテゴリ別サブ関数へ」（起票時の手法） | **採らなかった**。`exec_op` を対象外にしたのと同じ形（平坦な表） |
| 「得るのは行数だけ」（起票時） | **外れ**。行数は**増えた**が、**実バグが 2 件**出た |
| 「重複はノイズだろう」 | **外れ**。20 組のうち 2 種が本物で、**片方（3 回書かれた埋め文字解析）がバグの原因そのもの** |
| 「`split`/`rsplit` は畳める」 | **外れ**。3 つの非対称が**どれも CPython 準拠に必要**。畳むと理由が読めなくなる |
| 「clippy の増分 0 を守れば十分」 | **外れ**。**受け入れ済みの 51 件の中にバグを指す警告が埋もれていた** |
| 「既存の例題が守ってくれる」 | **外れ**。`"hi"` を使っていたので**2 件とも発火しなかった** |

---

## #75〜#82 保守性レーン第 3 弾の起票 — 全 src 機械診断（2026-08-24）

#58〜#74（第 2 弾）が全件完了した後、あらためて `src/` 全体を機械計測して起票した 8 件。
観点は依頼どおり **「ロジックの複雑さ」の 3 軸＝制御ネストの深さ／分岐の数／変数の参照の多さ**。
母集団は **204 ファイル・68,564 行・関数 2,105 個**。**この時点では 1 行も変更していない**（起票のみ）。

### 計測方法（推測ではなく実測・#50/#58 と同じ方針）

| 観点 | 手法 |
|---|---|
| 関数長 | ブレース平衡でスパン抽出（行コメント・ブロックコメント・文字列・**文字リテラル**・raw 文字列を除去してから計数） |
| **制御ネスト深さ** | **制御構文が開いたブレースだけ**を数えた相対深さ（`if`/`else`/`match`/`for`/`while`/`loop`/アーム/クロージャ） |
| 分岐数 | `if` + `while` + `for` + `loop` + `&&` + `\|\|`（**`match` アーム数は別集計**＝平坦な表を弾くため） |
| 変数の参照 | 関数内の `let` 束縛数／構造体フィールドごとの `self.<field>` 参照箇所数・参照ファイル数 |
| 重複 | 14 行窓の正規化ハッシュ一致（空行・`{`/`}` だけの行・行コメント・属性を除去） |

### ⚠⚠ 起票前に指標の欠陥を 1 つ潰した（#60 の再演を回避）

**最初の版は構造体リテラルのブレースを制御ネストとして数えていた。** この指標だと
`type_check/call_check.rs:check_call_args` が **深さ 9・src 最悪級**に見えるが、実体は
`StaticTypeError { kind: TypeErrorKind::X { … } }` の入れ子で、**制御フローの深さは 6**。
そのまま起票していたら「深さ 9 の関数を割る」という**存在しない仕事**を作るところだった。

⚠ **#60 のときの欠陥（文字リテラルを潰さずブレース平衡を崩した）とは別物**。
⇒ **診断ツールは毎回、既知値との負の対照を取ってから使うこと**。今回取った対照:

| 対象 | 実装ログの記載値 | 今回の測定 |
|---|---|---|
| `parse_struct_bodies`（#60 後） | 60 行・ネスト 3 | **60 / 3** ✅ |
| `scan_scope`（#60 後） | 49 行・ネスト 3 | **49 / 3** ✅ |
| `exec`（#58 後） | 192 行 | **192** ✅ |
| `exec_op` / `eval_str_method` | 895 / 588 行 | **895 / 588** ✅ |

⚠ **重複検出では文字列リテラルを潰してはいけない**（関数長の計測とは逆）。潰すと
`("alpha", "α"),` のようなデータ表が全部同じ行に化け、`lexer/math.rs` だけで
**96 窓の偽陽性**が出た。⇒ 重複検出は「行コメントと属性だけを落とす」正規化にした。

### ⚠⚠ 事故 — 診断スクリプトが `src/lexer/math.rs` を上書きした

補助スクリプト（`shape.py`）が本体（`complexity.py`）を import した際、**モジュール直下に
置いていた `json.dump(…, open(sys.argv[1], 'w'))` が import 時に走り**、
`sys.argv[1]`（＝ 検査対象として渡した `src/lexer/math.rs`）を JSON で上書きした。

`git status` がクリーンだったため `git checkout -- src/lexer/math.rs` で復元し、
**HEAD と byte-identical**（改行コード差のみ）を確認。さらに**復元後に全計測をやり直して
汚染前の結果と完全一致**することを確認した（＝ 起票の根拠になった数字は有効）。

⇒ **教訓: 走査スクリプトの実行部は必ず `if __name__ == '__main__'` で囲う。**
import 副作用で**書き込み先が「検査対象のパス」に化ける**。破壊的操作の前に `git status`
を見る習慣（過去の教訓）が、今回は事後の復元可否を即断できた理由になった。

### 全体分布 — **大半はすでに十分浅い**

| 指標 | 中央値 | p90 | p99 | 最大 |
|---|---|---|---|---|
| 関数行数 | 11 | 53 | 218 | 895 |
| **制御ネスト深さ** | **0** | **2** | **5** | **8** |
| 分岐数（アーム除く） | 0 | 6 | 24 | 48 |

制御ネスト **≥6 は 2,105 個中 19 個（0.9%）**、≥5 で 47 個、≥4 で 98 個。
**#51〜#74 の保守性レーンは効いている。** 残る複雑さは散らばっておらず、**4 クラスタに集中**する。

### ⚠⚠ 副産物 — 実バグ 1 件（→ #75）。**「調査だけのタスク」が実バグを出したのは 3 回連続**

`interpreter/exec/mod.rs` の `collect_refs_expr` が `_ => {}` で終わっており、
**`Expr` 全 28 variant のうち 8 個を落としている**（`Set` / `Block` / `IfExpr` / `ForExpr` /
`WhileExpr` / `MatchExpr` / `Cast` / `MustBe`）。消費者は 3 系統ともクロージャの自由変数解決
（`exec/blocks.rs:capture_env`・`vm/compiler/calls.rs`・`vm/compiler/decls.rs`）。
⇒ **内側の関数が外側の変数を「式形式の制御構文の中だけ」で参照すると捕捉されない。**

| 参照を書いた場所 | Rust HEAD（35bd912） | `python -m impl_python`（参照実装） | 判定 |
|---|---|---|---|
| `if` **文**（**対照**） | `10` | `10` | 一致（＝原因の切り分けができた） |
| `if` **式**（`->int` + `block_return`） | `NameError: 'base' is not defined` | `10` | **差分＝実バグ** |
| `match` 式 | `NameError` | `10` | **実バグ** |
| `block:` 式 | `NameError` | `10` | **実バグ** |
| `for` 式（`loop_yield`） | `NameError` | `10` | **実バグ** |
| `while` 式（`loop_yield`） | `NameError` | `10` | **実バグ** |
| set リテラル `{base, 99}` | `NameError` | `2` | **実バグ** |
| `Cast`（`as`） | `ParseError` | `ParseError` | 両方失敗・構文が別（差分なし） |

⚠⚠ **対照（`if` **文**）が通ることが、原因が `collect_refs_expr` であることの証明**。
⚠⚠ **またしても「例題が 1 本も無かった」**（#56 / #68 / #71 と同じ理由）。全ゲート緑のまま生きていた。
⚠ #59 は **`Stmt` の「束縛する名前」だけ**を exhaustive 強制にした。**`Expr` は対象外**で、
`Expr` に variant を足しても何も止まらない。#75 はその穴が発火したもの（→ 恒久対策は #81）。

### 起票した 8 件

**A群. 実バグの温床（同じ判断の複数実装）**

| # | 対象 | 実測 |
|---|---|---|
| 75 | `collect_refs_expr` の `_ => {}` | 上記の実バグ。`Expr` 28 variant 中 **8 個欠落・5 形が壊れている** |
| 76 | typed ABI 引数マーシャリングの **4 コピー** | `call_native_function` 247行/ネスト7・`dispatch_native_evaled_wb` 208行/7・`dispatch_native_typed_exprs` 100行/6・`ar_call_fn` 218行/8 ＝ **計 773 行**。前 2 者の `diff` は **467 行中 151 行だけ**（≒ 150 行が共通）。**src の制御ネスト上位 5 位のうち 4 つがこのクラスタ** |
| 77 | `exec_raise` ⇔ `vm_raise` | 「例外インスタンスへ file/line/col/code_context を書き込み `StackFrame` を組む」約 40 行が 2 実装。**コード内コメント自身が「同一意味論」と明記**している |

⚠⚠ **76 の根拠は #54 のコメント自身**: 「**3 経路で違ってよいのは『書き戻し先をどこへ返すか』だけ**」。
#54 は**呼び出し**（`call_typed_abi`）だけを 1 本化し、**マーシャリング（slots 構築・`Ptr`/`OutPtr`
解決・cleanup・書き戻し）は 4 コピーのまま**。#48 の実バグはまさにこの二重化が原因だった。
⚠ 76 はホットパス（#48 で cdll 1.13〜1.16x を取った場所）なので [ab_bench_modes.ps1](ab_bench_modes.ps1) の
C DLL 経路と [compare_import_paths.ps1](compare_import_paths.ps1) が要る。
⚠ 77 は `exec_event_subscribe` → `event_subscribe_evaled` ← `vm/run.rs` という**既に正しい形**が
同じファイルにある（`*_evaled` へ委譲）。畳む先の形は決まっている。

**B群. 逐語重複（低リスク・機械的）**

| # | 対象 | 実測 |
|---|---|---|
| 78 | `lexer/math.rs:render_math_str` | **src 最深（制御ネスト 8）**・167 行。`'^'` アーム 69 行と `'_'` アーム 69 行の `diff` が **4 行だけ**（マーカー文字と `to_superscript_char`/`to_subscript_char`）。138 → 約 72 行 |
| 79 | `parser/imports/ar_modules.rs` | `load_tl_module` / `load_tl_source_module` / `load_tlc_module` の末尾 **22 行が 3 コピー逐語**（子パーサ生成・キャッシュ／循環検出／`node_counter` 継承・マージ）。**`#16・C1` の注意書きごと 3 重化**している |
| 80 | `ClassValue` の 22 フィールド構築 | **11 箇所**でリテラル構築（`built_in_types.rs` 4・`exec/definitions.rs` 5・`modules.rs` 1・`templates.rs` 1）＝ フィールド言及 **約 240 箇所** |

⚠⚠ **80 で `..Default::default()` を使ってはいけない。** 「フィールドを足したら全 11 箇所が
コンパイルエラーになる」という強制が消え、**#59 が作った仕掛けを逆行させる**。
取るべき形は「**共通コンストラクタ 1 本＋そこだけが exhaustive なリテラル**」。
⚠ 78 は畳んでも**深さは減らない**（深さ 8 は `while → match → if → while → if → while → if → for → if`
に由来する）。深さを下げるには「`\symbol` か 1 文字を読む」スキャナを別に切り出す必要がある。

**C群. ドリフト強制の穴（#59 の続き・`Expr` 側）**

| # | 対象 | 実測 |
|---|---|---|
| 81 | `Expr` 再帰骨格の重複 | `vm/compiler/decls.rs` の `scan_shadow_expr`(86行) と `collect_expr_decls`(102行) は **35 行の再帰骨格が逐語一致**。`resolver.rs:collect_bound_in_expr`・`exec/mod.rs:collect_refs_expr` と合わせ **4 本が `_ => {}` 終端** |
| 82 | `generate_llvm_module` の適格関数収集 | 251行/ネスト6。`FnDef`/`GenDef` × 最上位/`ClassDef` 内 の **ほぼ同一 4 ブロック 48 行**。かつ **5 本目の `Stmt` walker** |

⚠⚠ **81 は「walker を 1 本に統合」ではない。** #59 は「**本体・部分式へどう降りるかは
walker ごとに本当に違う**」と理由付きで残置を決めている（`decl_names.rs` の doc）。
取るべき形は「**部分式を exhaustive に列挙する visitor を 1 本置き、降り方の方針は walker 側に残す**」。
⚠ 81 は #75 と同じ穴を塞ぐので **75 → 81 の順**（75 は最小修正、81 は再発防止）。

### ⚠ 対象外と判断したもの（誤って着手しないための記録）

| 箇所 | 行数 | 制御ネスト | 対象外の理由 |
|---|---|---|---|
| `vm/run.rs:exec_op` | 895 | **5** | **行数 1 位だが深さは 20 位圏外**＝平坦。#65 の「対象外」判断を**別の指標で追認**した。割ると #10-b の再演 |
| `classes/string_methods.rs:eval_str_method` | 588 | **5** | 同上（#65 で再確認済み） |
| `ops/operators.rs:apply_binop` | 325 | 4 | 110 アームの表 |
| `ast_value.rs:stmt_to_value` | 321 | **3** | **分岐 0**。純粋な写像 |
| `Interpreter` の全面分解 | — | — | **#67 が却下済み**（`impl` が 37 ファイルに散る）。第 3 弾の再測定は 29 フィールド／233 メソッドで、参照は上位 4 つ（`scopes` 58・`annotations` 41・`module_cache` 38・`current_class` 23）で **6 割超**、残り 17 は **≤5 箇所**＝ #67 の基準（凝集が明らかなクラスタ）に合う塊はもう無い |
| `ArCallbacks`（38 フィールド） | — | — | `extern "C"` 関数ポインタの **vtable**。フィールド数がそのまま ABI |
| 制御ネスト 6 の残り 13 個 | 40〜152 | 6 | `check_intersection_members`・`convert_class`・`lex_fstring`・`check_call_args`・`parse_call`・`instantiate_template_args`・`load_cpp_module` ほか。**線形なフェーズ構造＋正当な二重ループ**で、深さはエラー構築か入れ子反復に由来。切り出しの検査コストが利得を上回る |

### 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「第 2 弾で潰したので、もう大きな塊は残っていないだろう」 | **半分当たり**。分布は健全（中央値 0・p90 2）だが、**上位 5 位のうち 4 つが同一クラスタ**（#76）で残っていた |
| 「深さ 9 の関数が 3 本ある」（最初の指標） | **外れ**。構造体リテラルを数えていた。制御フローの最大は **8**（`render_math_str` の 1 本だけ） |
| 「診断だけなのでバグは出ないだろう」 | **外れ**。**3 回連続**（#55 → #58〜#68 → 今回）。今回は**正しいプログラムが 5 形で動かない** |
| 「重複検出は第 2 弾で採り切ったはず」 | **外れ**。ファイル間 11 組・同一ファイル内 25 組。うち 4 組（#76/#77/#78/#79）が本物 |
| 「`exec_op` の対象外判断は行数ベースの主観だろう」 | **外れ**。制御ネスト指標で**独立に追認**された（895 行で深さ 5） |

---

## #75 クロージャの捕捉が「式形式の制御構文」を見ていなかった（実バグ）を修正（2026-08-24・完了）

第 3 弾の診断（1 つ上の項）で見つけた実バグ。**正しいプログラムが 6 形で動かなかった。**

### 1. 症状 — 参照を「どの構文の中に書いたか」で結果が変わっていた

内側の関数が外側の変数を**式形式の制御構文の中だけ**で参照すると `NameError` になる。
同じ参照を**文形式**（`if` 文）で書けば通る ⇒ **原因は評価側ではなく「自由変数を集める側」**、
という切り分けがここで付いた（詳細な表は 1 つ上の起票の項）。

| 形 | 修正前 | 参照実装 `python -m impl_python` | 修正後 |
|---|---|---|---|
| `if` **文**（対照） | `10` | `10` | `10` |
| `if` 式 / `match` 式 / `block:` 式 / `for` 式 / `while` 式 / **`match` 文** / set リテラル | **`NameError`** | `10`（set は `2`） | **一致** |

### 2. 真因 — `_ => {}` で 2 本の walker が variant を黙って落としていた

[exec/mod.rs](src/interpreter/exec/mod.rs) の `collect_referenced_names_stmt` と `collect_refs_expr`。
消費者は 3 系統ともクロージャの自由変数解決:
`exec/blocks.rs:capture_env`（ツリーウォークの捕捉）／`vm/compiler/calls.rs`（VM の捕捉 slot）／
`vm/compiler/decls.rs:nested_fn_free_names`。

| walker | 落ちていた variant |
|---|---|
| `collect_refs_expr` | **`Expr` 30 中 8**: `Set` / `Block` / `IfExpr` / `ForExpr` / `WhileExpr` / `MatchExpr` / `Cast` / `MustBe` |
| `collect_referenced_names_stmt` | **`Stmt` 40 中 6**: **`Match`** / `AsyncAssign` / `EventSubscribe` / `EventUnsubscribe` / `Field` / `EnumDef`(既定値) / `DebugLet` |

⚠⚠ **`Stmt::Match`（`match` **文**）の抜けは診断の起票時には気づいていなかった。**
`Expr` 側を直す過程で `Stmt` 側も variant 単位で数え直して見つけた（実バグとして再現も確認済み）。
⇒ **「片側を exhaustive にするなら、対になっている walker も同時に数え直す」**。

### 3. 直し方 — `_ => {}` を消して exhaustive に（#59 と同じ方針）

降りるバリアントを足したうえで、**降りないバリアントも理由付きで列挙**した。
`_ => {}` を書かないので、`Expr`/`Stmt` に variant を足すと**ここがコンパイルエラーになる**。

⚠ 落とし穴として明記したもの:
- **`ForExpr` の `target` は参照として拾わない** — 入れ子スコープの束縛で、`Stmt::For` の既存の
  扱いと揃える（`decl_names` の「順序に意味がある束縛は載せない」側）。
- **`DebugVar` / `LocalVar`（`dbg::name` / `local::name`）は捕捉しない** — 専用の名前空間への
  参照であって外側フレームのローカルではない。捕捉対象にすると別物を掴む。
- **`Import` / `FromImport` の `body` には降りない** — 別モジュールの本体。

### 4. 例題（#36/#41/#42 の教訓 ＝ 例題が無いから 6 形とも生き残った）

[examples/basics/closure_capture_expr_forms.ar](examples/basics/closure_capture_expr_forms.ar) を新設。
壊れていた 6 形すべて＋**対照の `if` 文**＋**可変キャプチャ（セル昇格）を式の中だけで参照する形**の 9 ケース。

⚠⚠ **負の対照で検出力を確認した**: 修正前バイナリでこの例題を実行すると
`NameError: 'flag' is not defined`（`c_if_expr` で停止）。**ゲートは作っただけでは守られない**（#57）。

### 5. 検証

| ゲート | 結果 |
|---|---|
| `cargo build` / `--release` | 警告 **0** |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **増分 0**（⚠ 下記） |
| [scan_examples.ps1](scan_examples.ps1) | FAIL **0** |
| [force_gate.ps1](force_gate.ps1) | **0 件・158 例題**（例題 +1） |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **52/52**（例題 +1 が自動で入り一致） |
| [compare_outputs.ps1](compare_outputs.ps1) | **93 / 94 identical**（⚠ **差分は新例題 1 本だけ ＝ 意図した変更のみ**。既存 93 例題は全部同一）。⚠⚠ **負の対照（同一 exe 同士）で 94/94** を先に取ってから読んだ |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **111 / 112 identical**（⚠ 差分は新例題 1 本だけ。**既存例題のバイトコードは 1 つも変わっていない**）。負の対照 **112/112** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件**（⚠ 下記） |
| [tw_stats.ps1](tw_stats.ps1) | `in_fn` **0**・`vm_ineligible` **0**・bail **0**・`tw_control_flow` **0** |

⚠⚠ **clippy の「基準 50 / 63」は数え方が違っていた。** 同じコマンド・同じ数え方で
**修正前の HEAD を測り直したら 51 / 66** で、修正後も **51 / 66** ＝ **増分 0**。
差はサマリ行（`` warning: `arrow` (lib) generated N warnings ``）を数えるかどうか。
⇒ プランの「**基準値はコマンドごと書く・総数ではなく増分 0 を見る**」がそのまま効いた事例。
**総数だけ見ていたら「+1/+3 の退行」と誤読していた。**

⚠ **[stale_doc_refs.ps1](stale_doc_refs.ps1) が今回の doc コメントを 1 件捕捉した**
（`` `impl_python` `` — src に無い外部成果物）。バッククォートを外して解消。
⇒ #57 の「改名・削除をしたら必ず走らせる」は**新規コメントを書いたときにも効く**。

⚠ **この変更は `compare_bytecode` も意味を持つ**（「解釈側だけを触った変更は自明に一致する」の例外）。
`collect_referenced_names` は **VM コンパイラの入力**でもある（`vm/compiler/calls.rs` の捕捉 slot・
`decls.rs:nested_fn_free_names`）ので、捕捉集合が変われば**バイトコードが変わりうる**。
⇒ 既存 111 例題が byte-identical だったことが「**既存プログラムのコンパイル結果は動いていない**」の根拠。

### 6. 規模

| | 前 | 後 |
|---|---|---|
| `collect_referenced_names_stmt` | 68 行・`_ => {}` あり | **107 行**・exhaustive |
| `collect_refs_expr` | 50 行・`_ => {}` あり | **105 行**・exhaustive |

⇒ **行数は増えた**（+94）。買ったのは行数ではなく「**variant を足すとコンパイルが止まる**」性質。

### 7. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「`Expr` 側 8 variant を足せば終わり」 | **外れ**。対になる `Stmt` 側にも 6 variant の穴があり、**`match` 文が実バグとして再現した** |
| 「捕捉が増えると既存の挙動が動くかもしれない」 | **当たらなかった**。既存例題は 1 つも踏んでいない（＝ だから 6 形とも生き残っていた） |
| 「clippy が +1/+3 したので退行だ」 | **外れ**。**数え方の違い**。修正前を同じコマンドで測り直して増分 0 と確定 |
| 「診断で見つけたバグなので小さい」 | **当たり**（src +94 行・1 ファイル）。ただし**触った walker の消費者は 3 系統**（ツリーウォーク捕捉・VM 捕捉 slot・`nested_fn_free_names`） |

---

## #76 typed ABI 引数マーシャリングの 4 コピーを 1 本化（2026-08-24・完了）

第 3 弾の診断で「**src の制御ネスト上位 5 位のうち 4 つが同一クラスタ**」と出た箇所。
#54 は**呼び出し本体**（`invoke_typed_abi`）だけを畳み、**引数マーシャリングは 4 コピーのまま**だった。

### 1. 畳める根拠 — 差は「入力」であって「ロジック」ではない

畳む前に `resolve_typed_ptr_arg` を読んで、`named_mut` が **3 状態**であることを確認した:

| `named_mut` | 意味 | 挙動 |
|---|---|---|
| `Some(true)` | 名前付き `mut` 変数 | 書き戻す |
| `Some(false)` | 名前付き `let` 変数 | **`Err`**（書き込みポインタへ `let` を渡した） |
| `None` | 判定できない経路 | エラーにせず**書き戻さない**（安全側） |

⚠⚠ **`Some(false)` と `None` は同義ではない。** ここを潰すと `let` の誤用が黙って通る。
4 経路が違うのは「**何を知っているか**（＝ この入力を何にできるか）」と「**書き戻し先**」だけ:

| 経路 | 作れる `named_mut` | 書き戻し先 |
|---|---|---|
| `call_native_function`（ツリーウォーク） | 3 状態すべて（引数式を見る） | 自分で `assign_var` |
| `dispatch_native_typed_exprs`（IC 命中） | 3 状態すべて | 自分で `assign_var` |
| `dispatch_native_evaled_wb`（VM） | `Some(true)` / `None`（マスク） | **呼び出し元へ返す**（VM のローカルは触れない・#48） |
| `ar_call_fn`（ネイティブ→C コールバック） | `None` のみ（アリーナハンドル止まり） | **書き戻しなし** |

⇒ `TypedArgs::marshal(vals, params, named_mut)` を [value/native.rs](src/interpreter/value/native.rs) に置き、
**`named_mut` の作り方と `out_wb` の消費だけを呼び出し側に残した**。

### 2. ⚠⚠ 値で返す API にしなかった理由（安全性）

`OutPtr` のスロットには `out_locals` の**要素のアドレス**が入る。`TypedArgs` を値で返すと
move で **C が解放済みスタックへ書く**。⇒ 呼び出し側のローカルに置き `&mut` で `marshal` を呼ぶ形にし、
doc に「**move 禁止**」を明記した。

### 3. ⚠ 副産物 — `dispatch_native_typed_exprs` の引数評価を 3 経路と揃えた

畳む前のこの経路は **eval とマーシャリングが交互**で、型不一致で打ち切ると
**残りの引数式が一度も評価されなかった**（他の 2 経路は `eval_call_args` で全部評価している）。
畳むにあたり「全部評価してからマーシャリング」に揃えた。⚠ `Vec` の確保は**元からあった**
（`evaled` に毎回 push していた）ので追加コストは無い。
併せて OutPtr へ名前付き `let` を渡す誤りの拒否も、ハンドル経路（MutPtr 事前チェック）と
**同じ規則で明示的に書いた**。根拠は「`AbiTy::OutPtr` は必ず `PtrParam::MutPtr`」
（どちらも `CType::Ptr { mutable: true }` 由来）と確認したこと。

### 4. ⚠ `ar_call_fn` の純スカラー枝だけは共有マーシャラを通さない（残した例外）

ハンドル → `Value` のクローンを挟まずアリーナから直接デコードする最適化で、
**STATE アクセスが ~8 回 → 1〜2 回**になる（既存のコメントに実測が残っている）。
型の許容表は共有側と同一であることを確認したうえで残し、**残した理由をコードに書いた**。
一方、構造体ポインタを含む枝と**呼び出し本体（`invoke_typed_abi`）は合流させた**
（cleanup の実行順と status の扱いが 4 経路で同一になる）。

### 5. ⚠⚠ 実測で退行を 1 件出して潰した — **`Vec` をホットパスに置いた**

最初の実装は `named_mut` を `Vec<Option<bool>>` で作っていた（`.collect()`）。
**FFI 呼び出しごとに 1 回ヒープ確保**が増え、cdll ベンチが落ちた:

| メトリクス | `Vec` 版 | 固定長配列版 | 負の対照（同一 exe） |
|---|---|---|---|
| `cdll_v3_add` | **0.900x** | 0.980x / 0.966x | 1.014x |
| `cdll_v3_add_toplevel` | **0.901x** | 1.007x / 0.966x | 0.989x |
| `cdll_v3_add_fresh` | 1.004x | 1.002x / 1.006x | 0.989x |

⇒ `std::array::from_fn` の `[Option<bool>; 16]` に変更（typed 経路の引数上限が 16 なので足りる）。
**doc に「ヒープ確保しないこと」と実測値を書いた。**

⚠⚠ **判定は「負の対照」で行った。** 修正後も 0.966〜1.007x と振れるが、
**同一バイナリ同士の A/B が 0.966〜1.031x** なので**ノイズ帯の内側**。
プランの「数 % で良し悪しを決めない」「同じ 2 バイナリで測り直しても同じ差が出るか」がそのまま効いた。
⚠ 逆に **`Vec` 版の 0.900x は 2 メトリクスで一貫していてノイズ帯の外**だった ＝ 実差の見分けはついた。
（`interp` モードは全指標 1.02〜1.05x で、退行なし。）

### 6. 規模と複雑さ

| 関数 | 前 | 後 |
|---|---|---|
| `call_native_function` | 247 行・ネスト **7** | **189 行・ネスト 3** |
| `dispatch_native_typed_exprs` | 100 行・ネスト **6** | **62 行・ネスト 1** |
| `dispatch_native_evaled_wb` | 208 行・ネスト **7** | **152 行・ネスト 3** |
| `ar_call_fn` | 218 行・ネスト **8** | **196 行・ネスト 8**（下記） |
| 合計 | **773 行** | **599 行** |
| 新設（共有） | — | `marshal` 40 行/ネスト 4・`invoke_typed_abi` 25 行・`decode_typed_ret` 11 行 |

src 全体: **制御ネスト 7 の関数が 4 本 → 0 本**、深さ ≥6 が **19 → 16 個**。
差分は 3 ファイルで **+311 / -340**（doc コメントを厚く書いたので正味の削減は行数より大きい）。

⚠ **`ar_call_fn` のネスト 8 は残った**（`render_math_str` と並んで src 最深）。
残った深さは**マーシャリングではなく「typed 高速パスに入るためのガードの入れ子」**
（`if let NativeFunction` → `if let Some(sig)` → `if typed_ptr != 0` → `if !has_ptr` →
`STATE.with` → `for` → `match` → `match`）。
⇒ **#76 の定義（4 コピーの重複）は達成したのでここで止めた。** ガードの平坦化
（typed 高速パスを `try_typed_fast_call(...) -> Option<i64>` へ切り出す）は
**ホットな FFI コールバック経路の構造を変える別件**で、独立に A/B が要る。**未起票**。

### 7. 検証

| ゲート | 結果 |
|---|---|
| `cargo build` / `--release` / `--features prof` / `--features tw_stats` | すべて警告 **0** |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **50 / 65**（⚠ **1 件減**。消えたのは `the loop variable i is used to index slots` ＝ 畳んだマーシャリングのループそのもの。実害を指す警告ではないことを確認した） |
| [compare_outputs.ps1](compare_outputs.ps1) | **94 / 94 identical** |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **112 / 112 identical** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **13 / 13 identical**（⚠ FFI を触ったのでこれが主検査） |
| [ab_bench_modes.ps1](ab_bench_modes.ps1) | cdll・interp とも**ノイズ帯内**（上表・負の対照つき） |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **52/52** |
| [scan_examples.ps1](scan_examples.ps1) / [force_gate.ps1](force_gate.ps1) | FAIL **0** / **0 件・158 例題** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ 併せて C++ FFI 例題 6 本を**直接 A/B**（md5 一致）— 特に `cpp_out_param_writeback.ar`
（#48 の実バグを固定している例題。local / global / cell / loop / status / probe の 6 形）。

### 8. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「4 経路の差は書き戻し先だけ」（起票時） | **半分外れ**。差は**2 つ**あった —「書き戻し先」と「`named_mut` を何にできるか」。後者を潰さなかったのが正解で、潰すと `let` の誤用が黙って通る |
| 「保守性の変更なので速度は動かない」 | **外れ**。`Vec` 1 個で **0.90x**。⚠ **ホットパスに `.collect()` を置かない** |
| 「`ar_call_fn` も同じだけ浅くなる」 | **外れ**。マーシャリングは畳めたが、深さの実体は**ガードの入れ子**だった（8 のまま） |
| 「差分ゲートは compare_outputs で足りる」 | **外れではないが不足**。FFI は例題が少ないので [compare_import_paths.ps1](compare_import_paths.ps1) と C++ 例題の直接 A/B が主検査 |

---

## #83 `ar_call_fn` のガードの入れ子を平坦化（2026-08-24・完了）

#76 で**重複**は畳んだが**深さ**が残った箇所（制御ネスト **8** ＝ `render_math_str` と並んで src 最深）。
残っていた深さの実体は**マーシャリングではなく「typed 高速パスに入るためのガード」**:
`if let NativeFunction` → `if let Some(sig)` → `if typed_ptr != 0` → `if !has_ptr` →
`STATE.with` → `for` → `match h` → `match arena.get`。

### 1. 直し方 — 早期 return へ反転し、深い所を 2 段に切り出す

| 新設 | 役割 | 行数 / ネスト |
|---|---|---|
| `try_typed_fast_call` | typed 高速パスの試行。`Some(h)`＝決着 / `None`＝ハンドル経路へ | **72 / 2** |
| `marshal_scalar_handles` | 全引数が純スカラーのときのマーシャリング（1 回の STATE ボローで全引数） | **18 / 2** |
| `scalar_handle_slot` | ハンドル 1 個 → u64 スロット（アリーナ直読み・`Value` クローンなし） | **18 / 1** |

`ar_call_fn` 側は `if let Some(h) = try_typed_fast_call(...) { return h; }` の 3 行になった。

| 関数 | 前 | 後 |
|---|---|---|
| `ar_call_fn` | 196 行・ネスト **8** | **96 行・ネスト 5** |

src 全体で **制御ネスト ≥7 が 2 本 → 1 本**（残りは `render_math_str` ＝ #78 の対象）、≥6 が 16 → 15。

### 2. ⚠⚠ `#[inline(always)]` は「速度のため」ではなく「**生成コードを変えないため**」

切り出しただけ（`#[inline]`）の版で **cdll ベンチが落ちた**:

| メトリクス | `#[inline]` 版（2 回） | `#[inline(always)]` 版（2 回） |
|---|---|---|
| `cdll_v3_add` | **0.933x / 0.963x** | 0.979x / 0.999x |
| `cdll_v3_add_toplevel` | **0.940x / 0.947x** | **1.036x / 1.037x** |

⚠⚠ **これは経路の速度ではない。** `bench_ab_cdll.ar` は**非コンパイル実行**で C DLL を呼ぶ
（`eval/native.rs` 側）ので、**`ar_call_fn` を 1 度も通らない**。
それでも 0.94x が出て、**属性を 1 つ変えるだけで 1.04x に振れた**
＝ **コード配置に支配されている**（#28「効くのはアーム数ではなくコード配置」・
#62「`mod` 宣言の順序を逆にしただけで 0.929x」の 3 度目）。

⇒ 採用したのは `#[inline(always)]`。理由は数値ではなく**意味論**:
**呼び出し元は `ar_call_fn` の 1 箇所だけで、#83 以前は文字どおりインライン展開されていたコード**
なので、これが「生成コードを保ったまま**構造だけ**平坦化する」形になる。
⚠ #10-b（`exec_op` のアームに重い本体を書くな）とは状況が違う — **呼び出し元が 1 つなので
インライン化してもコードは複製されない**。doc に「外さないこと」と実測を書いた。

### 3. ⚠ 「ノイズ帯」は固定値ではない — 測るたびに取り直す

同一バイナリ同士の A/B（負の対照）は**その時々で幅が違った**:

| 負の対照 | `cdll_v3_add` | `cdll_v3_add_toplevel` |
|---|---|---|
| #76 完了時 | 1.014x | 0.989x（帯 ±3.5%） |
| #83 途中（n83f 同士 / n76b 同士） | 1.002x / 1.004x | 1.010x / 1.006x（**±1%**） |
| #83 最終（probe 同士） | 0.978x | 1.031x（±3%） |

⇒ **「#76 で測った ±3.5% を使い回す」ができない。** `#[inline]` 版を退行と判定できたのは、
**同じ時間帯に取った ±1% の対照**と並べたから。⇒ **A/B と負の対照は同じセッションで取る**。

### 4. 検証

| ゲート | 結果 |
|---|---|
| `cargo build` / `--release` / `--features prof` / `--features tw_stats` | すべて警告 **0** |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **50 / 65 ＝ 増分 0**（⚠ 下記） |
| [compare_outputs.ps1](compare_outputs.ps1) | **94 / 94 identical** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **13 / 13 identical** |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **112 / 112 identical** |
| [ab_bench_modes.ps1](ab_bench_modes.ps1) | cdll・native とも**同一バイナリの振れ幅の内側**（上表） |
| [compare_python_impl.ps1](compare_python_impl.ps1) / [scan_examples.ps1](scan_examples.ps1) / [force_gate.ps1](force_gate.ps1) | 52/52 / FAIL 0 / **0 件・158 例題** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) / [stale_doc_refs.ps1](stale_doc_refs.ps1) | identical / 5 identical / **0 件** |

⚠ **`ar_call_fn` を実際に踏むのはコンパイル済みネイティブ → インタープリタの経路**なので、
[ab_bench_modes.ps1](ab_bench_modes.ps1) の **native モード**（`native_sum_list_cb` /
`native_apply_fn_cb`）が主検査。`CHECKSUM` の突き合わせも通っている（食い違えば値が出ない）。
併せて**コンパイル済みモジュールを踏む例題 2 本**（`typed_abi.ar` / `swd_nested_runner.ar`）を
`--compile` からやり直して A/B（**出力一致**）。⚠ `importation.ar` は
**環境要因で元から失敗する**（`import[rs] 'sha2'` のクレート未配置）ので検査に使えない。

⚠ **clippy が一度 +1 した**（`needless_range_loop`）。`0..n` の添字ループを
`params.iter().enumerate().take(n)` に直して増分 0 に戻した。⚠ #76 で**消えた**のも同じ
lint（`used to index slots`）で、**マーシャリングを書くとこの lint が出入りする**。
⚠ [stale_doc_refs.ps1](stale_doc_refs.ps1) も 1 件捕捉（`` `enter`/`exit_native_call` `` と
**識別子を割って書いた**ため `enter` が存在しない扱いに）。`` `enter_native_call` `` へ直した。

### 5. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「切り出せば深さは下がる。速度は関係ない」 | **半分外れ**。深さは 8→5 に下がったが、**関係ないはずの cdll が 0.94x に動いた**（配置） |
| 「cdll が落ちたのだから FFI 経路の退行だ」 | **外れ**。cdll は `ar_call_fn` を通らない。**属性 1 つで 1.04x に振れて確定** |
| 「#76 で測ったノイズ帯を使い回せる」 | **外れ**。帯は ±1% 〜 ±3.5% で変動する。**対照は同じセッションで取る** |
| 「`ar_call_fn` はネスト 0〜1 まで下がる」 | **外れ**。残り 5（ハンドル経路の DLL シンボル解決 `if`→`match`→`match`）。p99 = 5 なので許容 |

---

## #77 `exec_raise` ⇔ `vm_raise` の 2 実装を 1 本化（2026-08-24・完了）

第 3 弾 A 群の最後。**コード内コメント自身が「`exec_raise` と同一意味論」と明記していながら
同じ 40 行が 2 箇所に書かれていた**箇所（`vm_raise` の doc）。
プランの「⚠ **`*_evaled` 版とずれた実装を作らない** — 実バグ 4 回」がそのまま当てはまる形。

### 1. 畳んだもの — 差は「組んだ結果をどう返すか」だけ

共有部分は「`Value` ＋ `Span` → `RaisedError`」:
① 例外インスタンスへ `file` / `line` / `col` / `code_context`（および `Error::` 修飾名）を直接書き込み、
② raise 地点の `StackFrame` を 1 つ持つ `RaisedError` を組む。

| 経路 | 組んだ結果の扱い（**唯一の差**） |
|---|---|
| `exec_raise`（ツリーウォーク） | `Ok(ExecResult::Raise(err))` で返す |
| `vm_raise`（VM） | `current_exception` へ積んで `RAISE_SENTINEL` を返す |

⇒ `Interpreter::build_raised_error(&self, exc_val, span) -> RaisedError` に集約。

| 関数 | 前 | 後 |
|---|---|---|
| `exec_raise` | 58 行 | **15 行**（bare 再送出の分岐 ＋ 1 行の委譲） |
| `vm_raise` | 40 行 | **4 行** |
| `build_raised_error`（新設・共有） | — | 40 行・制御ネスト **1** |

差分は **+35 / -51**（doc を厚くしたうえで正味 -16 行）。

### 2. ⚠ 副産物 — `get_context_lines` の二重呼び出しを解消

畳む前は**同じ引数で 2 回**呼んでいた（インスタンス書き込み用とフレーム用）。
`get_context_lines(&self, ...)` は `source_map` を読むだけの純粋関数なので 1 回に統合した。
⚠ 速度目的ではない（`raise` は例外時にしか通らない）。**2 回呼ぶ理由が無いのに 2 回呼ぶ形が
残っていたのは、2 実装だったから**という位置づけで記録する。

### 3. ⚠⚠ この組み立てを見る例題が **1 本も無かった**（→ 新設）

`e.file` / `e.line` / `e.col` / `e.code_context` を読むアクティブな例題は **0 本**だった
（`examples/archived/exceptions.ar` のみ・archived はゲート対象外）。
⇒ **`build_raised_error` のインスタンス書き込み側は、壊しても全ゲートが緑のまま**だった。
#56 / #68 / #71 / #75 と**同じ形**なので
[examples/exceptions/raise_span_fields.ar](examples/exceptions/raise_span_fields.ar) を新設した。

見るもの: 関数内 `raise`（VM 経路）と最上位 `raise` の両方で
`file`（末尾一致のみ — **絶対パスは print しない**）・`line`・`col`・`code_context` の内容、
および「位置が違えば `line` も違う」（＝ 焼き込みが呼び出しごとに起きている）。

⚠⚠ **負の対照で検出力を確認した**: `build_raised_error` の `context` を `String::new()` に
差し替えると `ctx-has-raise: True` → `False` に反転する。

⚠ **`impl_python` はこの焼き込みを実装していない**（`line`/`col` が 0・`code_context` が空）ので
[compare_python_impl.ps1](compare_python_impl.ps1) の `$knownDiff` に**理由つきで登録**した。

### 4. ⚠ 観察 — `exec_raise` は通常実行では到達しない（**起票はしない**）

[tw_stats.ps1](tw_stats.ps1) で全例題を集計したところ、ツリーウォークが実行する文は
`FnDef` / `ClassDef` / `Import` / `FromImport` / `TraitDef` / `NewTypeDef` / `EnumDef` /
`ProtocolDef` / `GenDef` の**定義文だけ**で、**`Raise` は 1 件も現れない**
（`in_fn` は 0・`module_body` も `ClassDef`/`FnDef` のみ）。
⇒ `dispatch.rs` の `Stmt::Raise => self.exec_raise(..)` は**通常実行から到達不能**の可能性が高い。

⚠ **#56（`parse_ar` が死んでいた）とは向きが違う**ので、消す判断はここではしない:
あちらは「動くべきものが bail していた」、こちらは「VM が担当しているので単に通らない」。
到達不能を**証明**するには REPL・デバッガ REPL・モジュール本体・定義文脈の式まで
洗い出す必要があり、それは #56 が示したとおり**例題だけでは足りない**。
⇒ **本タスクでは畳むだけに留めた**（畳んだので、仮に到達しても VM と同じ結果になる）。

### 5. 検証

| ゲート | 結果 |
|---|---|
| `cargo build` / `--release` / `--features prof` / `--features tw_stats` | すべて警告 **0** |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **50 / 65 ＝ 増分 0** |
| [compare_outputs.ps1](compare_outputs.ps1) | **95 / 95 identical**（例題 +1） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **113 / 113 identical** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **52/52**・既知差分 **44**・stale **0** |
| [scan_examples.ps1](scan_examples.ps1) / [force_gate.ps1](force_gate.ps1) | FAIL **0** / **0 件・159 例題** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) / [stale_doc_refs.ps1](stale_doc_refs.ps1) | identical / **5 identical** / **0 件** |

⚠ **例外系例題 6 本を直接 A/B** した（`examples/exceptions/*.ar`）。
⚠⚠ **最初 `traceback_frame_names` が「差分」に見えた** — 実体は
`<ZeroDivisionError object at 0x…>` の**ヒープアドレス**で、
[compare_outputs.ps1](compare_outputs.ps1) が正規化している揮発値だった。
⇒ **手で A/B するときも `0x…` を正規化してから比較する**（さもないと存在しない退行を報告する）。

⚠ 速度 A/B は取っていない。`raise` は例外時にしか通らず、`compare_bytecode` が
**113/113 byte-identical**（＝ コンパイル結果は 1 バイトも動いていない）なので、
ホットパスへの影響が無いことはそれで足りる。

### 6. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「40 行の逐語重複を畳むだけ」 | **当たり**。ただし副産物で `get_context_lines` の二重呼び出しも消えた |
| 「例外は例題が多いので網は足りている」 | **外れ**。**`file`/`line`/`col`/`code_context` を読む例題は 0 本**だった（`message` しか読んでいない） |
| 「手で md5 を突き合わせれば A/B になる」 | **外れ**。**アドレスを正規化しないと偽の差分が出る**（1 本で実際に出た） |
| 「`exec_raise` は普通に使われている」 | **外れ**。tw_stats では `Raise` が 1 件も出ない（ただし到達不能の**証明**にはならない） |

---

## #78 `render_math_str` の `^` / `_` 2 アームを 1 実装へ（2026-08-25・完了）

第 3 弾 B 群の 1 件目。**src に残っていた最後の制御ネスト 8**（起票時は `ar_call_fn` と 2 本、
#83 でそちらが 5 に下がって以降はここだけ）。

### 1. 何が重複していたか

`'^'` アーム 69 行と `'_'` アーム 69 行。**識別子を正規化して diff を取ると 4 行しか違わない**
（マーカー文字 `'^'`/`'_'` と写像 `to_superscript_char`/`to_subscript_char`）。
⇒ この 2 つを引数（`marker: char` / `type ScriptMap = fn(char) -> Option<&'static str>`）にして畳んだ。

さらに深さの実体は「`while` → `match` → `if` → `while` → `if` → `while` → `if` → `for` → `if`」
だったので、**波括弧グループと単一要素を別関数へ切り出した**（畳むだけでは深さは下がらない）。

| | 前 | 後 |
|---|---|---|
| `render_math_str` | **167 行・制御ネスト 8** | **28 行・ネスト 3** |
| `render_script`（新設） | — | 9 行・ネスト 0 |
| `render_script_group`（新設） | — | 34 行・ネスト 4 |
| `render_script_single`（新設） | — | 26 行・ネスト 2 |
| `push_script_char`（新設） | — | 9 行・ネスト 1 |
| `read_command_name`（新設） | — | 7 行・ネスト 0 |
| `math.rs` 全体 | 359 行 | **328 行**（-31） |

🎉 **src から制御ネスト ≥7 の関数が消えた**（第 3 弾の起票時 2 本 → 0 本）。≥6 は 15 → 14。

### 2. ⚠⚠ 畳めなかった 2 点（＝ グループ形と単独形は本当に違う）

「`^`/`_` の差はマーカーと写像だけ」は正しいが、**同じアーム内の「波括弧あり／なし」は 2 点で違う**:

| | 未知コマンド（`sym` が空） | 既知コマンドの未対応文字 |
|---|---|---|
| `^{\qqq}` グループ形 | `\qqq` をそのまま | `out.push(c)`（**マーカー無し**） |
| `^\qqq` 単独形 | **`^` を先に付けて** `^\qqq` | `map(c).unwrap_or(sym)`（**文字ではなく記号全体**） |

⚠ 後者は**多文字シンボルが来ると挙動が割れる**。ただし `math_command_to_str` の
**71 個はすべて 1 文字**と全件確認したので、現状は観測できない（`sym == c` になるため）。
⇒ **畳むついでに直さない**（挙動不変が本タスクの前提）。doc に潜在的な非対称として明記した。

### 3. 検証 — **先に golden を取ってから書き換えた**

書き換える前に **23 分岐を網羅するスクリプト**を HEAD のバイナリで実行して golden を取り、
書き換え後に突き合わせた（`^`/`_` × 波括弧/単独 × 通常文字/コマンド × 既知/未知 ＋
未終端・末尾 `^`・入れ子・`\sum_{i=0}^{n}`）。⇒ **23 分岐すべて byte-identical**。

⚠⚠ **この網は既存例題には無かった**。`built_in.ar` の 6 行が唯一の `m"..."` 使用箇所で、
**単独形のコマンド（`^\pi` / `^\qqq`）・未知コマンド・未終端はどのゲートも見ていなかった**。
⇒ [examples/basics/math_string.ar](examples/basics/math_string.ar) を新設して golden を恒久化した。
⚠ 期待値は「**現時点の挙動**」であって望ましい表示ではない旨をヘッダに明記
（未終端 `^{ab` → `ᵃᵇ` など素直でない出力があるが、#78 は 1 バイトも変えていない）。
⚠⚠ **負の対照で検出力を確認**: `push_script_char` のマーカーを `'!'` に変えると
`x^Z` → `x!Z` に反転する。
⚠ `impl_python` は **`m"..."` 自体が未実装**（ParseError）なので
[compare_python_impl.ps1](compare_python_impl.ps1) の `$knownDiff` へ理由つきで登録した。

| ゲート | 結果 |
|---|---|
| `cargo build` / `--release` / `--features prof` / `--features tw_stats` | すべて警告 **0** |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **50 / 65 ＝ 増分 0** |
| [compare_outputs.ps1](compare_outputs.ps1) | **96 / 96 identical**（例題 +1） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **114 / 114 identical** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **52/52**・既知差分 **45**・stale **0** |
| [scan_examples.ps1](scan_examples.ps1) / [force_gate.ps1](force_gate.ps1) | FAIL **0** / **0 件・160 例題** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) / [stale_doc_refs.ps1](stale_doc_refs.ps1) | identical / 5 identical / **0 件** |

⚠ 速度 A/B は取っていない。`render_math_str` は**字句解析で `m"..."` に出会ったときだけ**動く
（`lexer/scan.rs` の 2 箇所）ので実行時のホットパスではなく、`compare_bytecode` が
**114/114 byte-identical** ＝ コンパイル結果も動いていない。

### 4. ⚠ 踏んだ落とし穴 2 件（どちらもツール側）

- **⚠⚠ この環境の heredoc は「クォート付き」でもバックスラッシュを 1 段食う。**
  `cat > x.py <<'EOF'` に `'\\'`（Rust の文字リテラル）を書いたら**ファイルには `'\'` が入り**、
  `unterminated character literal` で落ちた（複雑さ診断のときにも同じ罠を踏んでいる）。
  ⇒ **バックスラッシュを含むコードを heredoc で生成しない**。`chr(92)` を組み立てるか、
  書き込んだ後に置換で直す。
- **⚠ `src/lexer/math.rs` は CRLF。** LF 前提の文字列置換が**黙って 0 件マッチ**になり、
  負の対照が「適用されたつもり」で通ってしまった（`assert count==1` を入れていたので気づけた）。
  ⇒ **置換スクリプトには必ず件数の assert を入れる**。改行コードは LF/CRLF の両方を試す。
- **⚠ `scan_examples.ps1` を比較ゲートと同時に走らせたら `partial_call_overhead.ar` が
  TIMEOUT した**（単独で再実行するとクリーン）。⇒ **タイムアウトを持つゲートは並走させない**。

### 5. 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「畳めば深さも下がる」（#76 の起票時に「下がらない」と書いた） | **起票時の読みが当たり**。畳むだけでは 8 のまま。**グループ形／単独形を別関数へ出して初めて 3** |
| 「`^` と `_` の差はマーカーと写像だけ」 | **当たり**（diff 4 行）。ただし**同じアーム内の波括弧あり／なしに 2 点の非対称**があった |
| 「非対称は片方に揃えられる」 | **やらなかった**。`unwrap_or(sym)` は多文字シンボルで割れるが**現状 71 個すべて 1 文字**。挙動不変を優先し、潜在として記録 |
| 「例題は built_in.ar があるので足りる」 | **外れ**。単独形コマンド・未知コマンド・未終端は**どのゲートも見ていなかった** |

---

## #79 `ar_modules.rs` の逐語重複を 1 本化（2026-08-25・完了）

3 つのローダ（`load_tl_module` / `load_tl_source_module` / `load_tlc_module`）の重複。
起票時は「子パーサ生成 22 行 × 3」だけを見ていたが、**実際は 4 種類が重複していた**。

| 重複していたもの | コピー数 | 畳んだ先 |
|---|---|---|
| 検索ディレクトリ（`source_dir` → `root_dir`、同じなら重複させない）5 行 | **3** | `module_search_dirs` |
| キャッシュ命中 ＋ 循環 import 検査 10 行 | **3** | `module_cache_probe` |
| 子パーサ生成・親からの引き継ぎ・キャッシュのマージ 22 行 | **3** | `parse_sub_module` |
| `node_counter` の注意書き（`#16・C1`）コメント | **3** | 同上（doc へ 1 本化） |

⚠ **`node_counter` の注意書きが 3 重化していた**のが典型的な症状 — 「直す人が 3 箇所とも
直したか誰にも分からない」形で、`decl_names`（#59）が潰したのと同じ構造。

### 畳まなかったもの（理由つき）

**候補パスの列挙と「見つからない」エラー文**は 3 つで違うので**畳まない**:

| ローダ | 候補 | エラー文 |
|---|---|---|
| `load_tl_module`（auto） | `.arc` → `.ar` → `__init__.ar` | `cannot find module` |
| `load_tl_source_module`（`import[ar]`） | `.ar` → `__init__.ar` | `cannot find source module` |
| `load_tlc_module`（`import[arc]`） | `.arc` のみ | `cannot find compiled module`（`--compile` の案内つき） |

⚠ **探索順に意味がある**（`.arc` が `.ar` より先＝ #58 の「探索順が違う 2 本は畳まない」と同じ判断）。

### ⚠ 保存した既存の挙動

`parse_program` が失敗すると `abs_path` は `loading` に**残る**（3 実装とも元からそう）。
畳むときに直したくなるが、変えると「一度失敗したモジュールを再 import すると循環扱いになる」が
変わるので**そのまま保存**し、doc に明記した。

規模: **264 行**（-14）。`+93 / -107`。

---

## #80 `ClassValue` の 11 箇所リテラルを `synthetic` へ（2026-08-25・完了）

19 フィールドの `ClassValue` を**リテラルで組む箇所が 11**（`built_in_types.rs` 4 ／
`exec/definitions.rs` 5 ／ `exec/modules.rs` 1 ／ `templates.rs` 1）。
どれも「空の既定値 8〜17 個 ＋ 違うところ数個」で、フィールド言及は約 200 箇所あった。

⇒ [`ClassValue::synthetic(name, class_id)`](src/interpreter/value/callables.rs) を追加し、
各サイトは `ClassValue { field_index, .., ..ClassValue::synthetic(name, id) }` の形へ。

### ⚠⚠ `Default` は実装しない（起票時の方針をそのまま守った）

`..Default::default()` を許すと「フィールドを足したら考える」強制が消える。
`synthetic` を**exhaustive なリテラル**にしてあるので、`ClassValue` にフィールドを足すと
**まずここがコンパイルエラーになる**（#59 と同じ仕掛け）。

⚠ **doc の「src で唯一の exhaustive リテラル」は誤りだったので直した。**
実際は **2 つ**あり、答える問いが違う:
`synthetic` = 「合成クラスの既定値は何か」／`deep_clone` = 「そのフィールドはどう深いコピーを作るか」。
**どちらもフィールド追加で止まる**ので、2 つあるのが正しい。

### ⚠⚠ 最大の危険は「行数」ではなく **`class_id` の採番順**

`class_id: alloc_class_id()` をリテラルの中に書いていた 9 サイトは、`..synthetic(name, id)` へ
移すと **`id` の評価が構造体更新記法の位置＝列挙したフィールドより後ろ**になる。
採番がずれると **`class_id` はインスタンスヘッダとネイティブ dispatch の GEP に焼かれる**ので、
`compare_outputs` では見えない形で壊れうる。

⇒ **`class_id` を引数で受ける設計**にして呼び出し側に残し（`synthetic` の中で採番しない）、
検査は [dump_native_ir.ps1](dump_native_ir.ps1) の **生成 LLVM IR が byte-identical** で行った
（代表 6 モジュール・`partial_call_overhead_module` 23,333 バイトを含む）。
⇒ **IR 完全一致 ＝ 採番も不変**と確認できた。⚠ プランの「codegen を触ったら IR が主検査」が、
**codegen を触っていなくても効いた**例。

### ⚠ 手写しをしない

11 サイト × 19 フィールドを手で書き換えると転記ミスが必ず出るので、**機械変換**した:
リテラルをブレース平衡で切り出し、**既定値と完全一致する 1 行フィールドだけ**を落として
`..ClassValue::synthetic(..)` を足す（多行フィールドには触らない）。
⚠ 途中 2 回、変換スクリプト自身の欠陥を assert が捕まえた:
① フィールド短縮記法（`methods,`）を未対応 ②`TemplateClassValue {` を
**`ClassValue {` として誤マッチ**（単語境界が無かった）。⇒ **`assert count == n` は必ず書く**。

規模: 4 ファイルで **-138 行**（`built_in_types.rs` -64・`definitions.rs` -56・
`modules.rs` -12・`templates.rs` -6）、`callables.rs` +39（`synthetic` 本体と doc）。

### 検証（#79・#80 まとめて）

| ゲート | 結果 |
|---|---|
| `cargo build` / `--release` / `--features prof` / `--features tw_stats` | すべて警告 **0** |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **50 / 65 ＝ 増分 0** |
| [dump_native_ir.ps1](dump_native_ir.ps1) | **生成 LLVM IR byte-identical**（#80 の主検査） |
| [compare_outputs.ps1](compare_outputs.ps1) | **96 / 96 identical** |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **114 / 114 identical** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **13 / 13 identical**（#79 の主検査） |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **52/52**・stale **0** |
| [scan_examples.ps1](scan_examples.ps1) / [force_gate.ps1](force_gate.ps1) | FAIL **0** / **0 件・160 例題** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) / [stale_doc_refs.ps1](stale_doc_refs.ps1) | identical / 5 identical / **0 件** |

⚠ 速度 A/B は取っていない。#79 は import 時（1 モジュール 1 回）、#80 はクラス定義時にしか
通らず、`compare_bytecode` **114/114** と **IR byte-identical** で生成物が動いていないことを確認済み。
⚠ 例題は増やしていない（**どちらも既存例題が濃く踏む経路** — import は 13 例題、
クラス生成は全例題）。#77/#78 と違い「網が無い」状況ではなかった。

### 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「#79 は子パーサの 22 行 × 3 だけ」 | **外れ**。**4 種類**が重複していた（検索ディレクトリ・キャッシュ検査・子パーサ・注意書き） |
| 「#80 は行数を減らすだけの退屈な作業」 | **外れ**。**`class_id` の採番順**という壊れ方が隠れていて、`compare_outputs` では見えない。IR 比較が要った |
| 「11 サイトなら手で書き換えられる」 | **やらなかった**。機械変換にして正解 — スクリプトの欠陥を assert が 2 回捕まえた |
| 「`Default` を derive すれば早い」 | **却下**（起票時の方針どおり）。強制が消える。⚠ ただし **exhaustive リテラルは 2 つある**（`deep_clone` も）ので doc を直した |

---

## #81 `Expr` 再帰骨格の 4 本を [`expr_walk`](src/expr_walk.rs) へ 1 本化（2026-08-25・完了）

**#75 の実バグ（式形式の制御構文の中だけで外側変数を参照するクロージャが `NameError`）の恒久対策。**
#75 は `collect_refs_expr` を exhaustive にしただけで、**同じ形の walker が他に 3 本**残っていた。

| walker | 状態（#81 前） |
|---|---|
| `exec::collect_refs_expr` | #75 で exhaustive 化済み（ただし構造は手書き） |
| `vm::compiler::decls::scan_shadow_expr` | `_ => {}` |
| `vm::compiler::decls::collect_expr_decls` | `_ => {}` |
| `interpreter::resolver::collect_bound_in_expr` | `_ => {}`（**`Slice` と `TemplateInstantiate` を見ていなかった**） |

### 1. 仕掛け — #59（`decl_names`）と同じ 2 段

新設 [expr_walk.rs](src/expr_walk.rs) の `each_subpart(expr, &mut f)` が
「**この式の直下に何がぶら下がっているか**」を答える。

1. `match expr` が **exhaustive**（`_` を書かない）⇒ `Expr` に variant を足すと**まずここが止まる**。
2. `SubPart` を消費側が**すべて exhaustive match** で受ける ⇒ 部分の種類を足すと**全 walker が止まり**、
   「この walker ではどう扱うか」を必ず決めさせられる。

```
SubPart::Plain        純粋な部分式（項・引数・要素・添字…）
SubPart::Control      式形式制御構文の式（if/while の条件・match の subject・for の iter）
SubPart::Body         式形式制御構文の**文の本体**  ← 降り方は walker ごとに違う
SubPart::ForTarget    `for` の target             ← 束縛。拾うかは walker が決める
SubPart::MatchPattern `case <expr>` のパターン     ← 参照 walker だけが降りる
```

### 2. ⚠⚠ 仕掛けが**実際に働いて**取りこぼしを 1 件止めた

最初 `SubPart` に `MatchPattern` を置いておらず、**#75 が入れた「`case` パターンも参照として拾う」が
消えるところだった**（`each_subpart` がパターンを返さないため）。
気づいて `MatchPattern` を足した瞬間、**消費者 4 本すべてがコンパイルエラーで停止**し、
各 walker に「降りるのか降りないのか」を書かせた:

| walker | `MatchPattern` の扱い | 理由 |
|---|---|---|
| `collect_refs_expr` | **降りる** | パターンは参照（#75） |
| `collect_bound_in_expr` | 降りない | `case` は値比較で束縛を作らない |
| `scan_shadow_expr` | 降りない | 宣言を含まない |
| `collect_expr_decls` | **降りない** | 降りるとパターン内のブロック式が slot を取り**採番がずれる** |

⚠⚠ **これは「畳んだら壊れた」の実例**。exhaustive でなければ黙って通っていた。
⚠ ただし**コンパイラが教えてくれたのは 2 段目だけ**（`SubPart` を足したとき）。
1 段目（そもそもパターンを列挙し忘れる）は**人間が読んで気づいた** — 仕掛けは万能ではない。

### 3. ⚠ 意図的に広げた網 — `Slice` / `TemplateInstantiate`

`collect_bound_in_expr` は `_ => {}` に落ちて**この 2 つを見ていなかった**。
`each_subpart` は exhaustive なので #81 で**降りるようになった**（束縛は取りこぼすと危険側なので
広がるのは正しい方向）。挙動が動かないことは [compare_bytecode.ps1](compare_bytecode.ps1)
**114/114 byte-identical** で確認した（＝ 現状の例題はこの形を踏んでいない）。

### 4. 例題

`match` **パターンの中だけ**で外側変数を参照する形は、**#75 の例題も踏んでいなかった**
（`case 1:` のリテラルパターンだけだった）。
[examples/basics/closure_capture_expr_forms.ar](examples/basics/closure_capture_expr_forms.ar)
に `c_match_pattern` を追加。
⚠⚠ **負の対照で検出力を確認**: `P::MatchPattern(x) => collect_refs_expr(x, out)` を
`P::MatchPattern(_) => {}` に変えると `NameError: 'key' is not defined` に反転する。
⚠ Rust / `impl_python` / HEAD の 3 者とも同じ出力（`100` / `0`）。

### 5. 規模

| ファイル | 差分 |
|---|---|
| `vm/compiler/decls.rs` | **-160**（2 walker が 86+102 行 → 各 12 行前後） |
| `interpreter/exec/mod.rs` | -85 |
| `interpreter/resolver.rs` | -63 |
| `expr_walk.rs`（新設） | +178 |
| 実質 | **約 -130 行**（doc を厚く書いたうえで） |

---

## #82 `generate_llvm_module` の適格関数収集を 1 本化（2026-08-25・完了）

**最上位 × クラス本体** の 2 通り × **`fn` / `gen`** の 2 通り＝ **4 ブロック 48 行**が
ほぼ同一だった。⇒ `eligible_fn(stmt, class_name) -> Option<EligibleFn>` に集約し、
ローカル `struct EligibleFn` をモジュールスコープへ引き上げた。

**4 つで違うのはシンボルをクラス修飾するかだけ**。`fn` と `gen` の差は 3 点しかない:
① 適格判定（`body_eligible` / `body_eligible_gen`）② `is_abstract` は `fn` にしか無い
③ `gen` は戻り値注釈を見ない。⇒ すべて 1 つの `match` の中に収まった。

### ⚠ 残した穴を**明記**した（畳んでも消えない）

これは **`Stmt` を歩く 5 本目の walker** で、`_ => {}` で終わっている
（#59 の `decl_names` は「束縛する名前」だけを強制化しており、ここは対象外）。
⇒ **関数を含みうる `Stmt` variant を足したらここも見る**必要があり、
**何も強制されない**。畳んだうえでコメントに既知の穴として書いた。

### ⚠⚠ 検査は IR byte-identical

収集の**順序が emit 順＝生成 IR** を決めるので、[dump_native_ir.ps1](dump_native_ir.ps1) が主検査。
⇒ 代表 6 モジュールの **LLVM IR が byte-identical**（#81 と合わせて確認）。

規模: `llvm_codegen/mod.rs` は `+133/-` の書き換えで、収集ループが **48 行 → 18 行**。

---

## #81・#82 の検証（まとめて）

| ゲート | 結果 |
|---|---|
| `cargo build` / `--release` / `--features prof` / `--features tw_stats` | すべて警告 **0** |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **50 / 65 ＝ 増分 0** |
| [dump_native_ir.ps1](dump_native_ir.ps1) | **生成 LLVM IR byte-identical**（#82 の主検査） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **114 / 114 identical**（#81 の主検査 — slot 採番が動いていない証拠） |
| [compare_outputs.ps1](compare_outputs.ps1) | **96 / 96 identical** |
| [compare_import_paths.ps1](compare_import_paths.ps1) | **13 / 13 identical** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **52/52**・stale **0** |
| [scan_examples.ps1](scan_examples.ps1) / [force_gate.ps1](force_gate.ps1) | FAIL **0** / **0 件・160 例題** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) / [stale_doc_refs.ps1](stale_doc_refs.ps1) | identical / 5 identical / **0 件** |

⚠ 速度 A/B は取っていない。どちらもコンパイル前段（解決・codegen 収集）で 1 回しか通らず、
**バイトコードと IR がどちらも byte-identical** ＝ 生成物が動いていない。

## #84〜#89 新文法の追加に対する冗長性の評価と起票（2026-08-25）

依頼は「**インクリメント `++` やメタプログラミング用 `expr` のような新しい文法を足したとき、
柔軟に対応できるか**」の評価。⚠ **#75〜#83（第 3 弾）の完了直後に測った**ので、
**#75（クロージャ捕捉）と #81（`expr_walk`）が入った後の数字**である。
**この時点では 1 行も変更していない**（起票のみ）。

### 計測方法 — 負の対照（#59 / #81 と同じ手法）

推測せず、**`Expr` / `Stmt` / `DeclOrigin` にプローブ variant を 1 個ずつ足して `cargo build` を
通し、止まった箇所を数えた**。実施後すべて復元し、`git status` クリーン・`cargo build` 警告 0 を確認。
併せて**非テスト `src` の `match` を全数走査**し、`Expr::` / `Stmt::` のアームを持つものを
`_ =>` の有無で分類した。

⚠ **走査器は先に負の対照を取った**（#60 / 第 3 弾の教訓）。`DeclOrigin` +1 で
**消費者がちょうど 4 本**停止することを確認し、実装ログ #59 の記載値と一致することを見てから使った。

### 結果 ① — 「コンパイラが止める数」と「実在する walker 数」の乖離

| 追加するもの | **止まる数** | 実在 walker | うち `_ =>` |
|---|---|---|---|
| `Stmt` variant | **7** | 45 | 35 |
| `Expr` variant | **6** | 27 | 19 |
| `DeclOrigin`（#59 の 2 段目） | **4** | 4 | 0 |

止まる箇所の実測内訳:

- `Stmt`: `check_stmt` ／ `exec`(dispatch) ／ `subst_stmt` ／ `stmt_to_value` ／ `stmt_kind` ／
  `each_declared_name` ／ **`collect_referenced_names_stmt`（#75 で増えた）**
- `Expr`: `infer` ／ `eval` ／ `subst_expr` ／ `expr_to_value` ／ `expr_kind` ／
  **`each_subpart`（#81 で増えた）**

⇒ **第 3 弾は効いている**。`Expr` 5→6・`Stmt` 6→7 に増え、`Expr` walker の `_ =>` は **23→19**、
`Expr` walker 総数も **30→27**（4 本が `expr_walk` へ委譲され消えた）。

⚠⚠ **それでも最大の AST 消費者 2 本は止まらない** — `compile_expr`（33 アーム）と
`compile_stmt`（33 アーム）はどちらも `_ => bail`。#33 以降これは「ツリーウォークへ落とす」ではなく
「**実行時に `VmForceError` で死ぬ**」であり、**その形の例題があるときだけ**発覚する。
**#56 / #68 / #71 / #75 の実バグ 4 件はすべてこの経路**で、共通原因は「**その形の例題が 1 本も無かった**」。

### 結果 ② — 失敗の型で 3 分類した（`_ =>` を一律に危険と見ない）

**A. 落ちる（安全側だが例題頼み）** — `compile_expr` / `compile_stmt` の bail。

**B. 静かに劣化するだけ（実害なし・設計として正しい）**

- `llvm_codegen` の `expr_eligible` / `stmt_eligible` / `walk_expr` / `walk_stmts`・`stub_gen`
  → **ネイティブ対象外になるだけ**
- `resolver::rewrite_expr` / `rewrite_stmt` → **最適化されないだけ**
- ⚠ **小さい `_ =>` は保守的ホワイトリスト**で、既定が安全側 —
  `as_const_lit` / `expr_prim` / `as_local` は `_ => None`、`arg_is_mutable` は `_ => true`
- `node_id` の採番漏れ → **`0 = 未採番`** を `type_check/annotations.rs` 4 箇所と
  `vm/compiler/calls.rs` 2 箇所が防御済み（＝合成 AST は「遅いが動く」）

**C. 静かに壊れる（本当の穴）** — すべて「**どの名前が束縛／参照されるか・どこへ降りるか**」を
答える walker。⚠⚠ **`Expr` 側は #75 / #81 で塞がった**（4 本が `expr_walk` へ委譲）。
**残っているのは `Stmt` 側と VM 側**:

1. **`Stmt` 本体への降り方が 6 本バラバラ**（→ **#84**）—
   `collect_nested_decls`(12 アーム) ／ `scan_shadow_stmts`(11) ／ `collect_bound_names`(14) ／
   `collect_declared_names`(4) ／ `nested_fn_free_names::walk`(5) ／ `collect_base_decls`(5)。
   `decl_names` は「**どの名前**」を、`expr_walk` は「**式の直下に何**」を強制化したが、
   「**文の下にどの `Vec<Stmt>` がぶら下がるか**」は**どちらも意図的に範囲外**にしている
   （#59 / #81 の doc に明記）。⇒ 3 つ目の欠けている半分。
2. **`peephole::code_target_mut` が `_ => None`**（→ **#87**）。`Op` は **89 variant**。
   プラン自身が「**忘れてもテストは通ってしまう**」と名指ししている唯一の穴。
3. **VM の slot 採番に不変条件の検査が 0 件**（→ **#86**）。
   ⚠⚠ ツリーウォーク側には `eval_local_ref` ほか **`debug_assert` が 4 件**あるが、
   **採番を実際に決めている `vm/compiler/decls.rs` は 0 件**。#33 / #55 でツリーウォークが
   定義文しか実行しなくなったので、**この安全網は実質もう働いていない**。
4. `impl_python/`（**16,410 行**の第 2 実装）— 動的型なので**強制ゼロ**。
   ゲートは [compare_python_impl.ps1](compare_python_impl.ps1) だけ。⇒ **起票はしない**
   （第 2 実装を持つこと自体が設計判断で、畳む先が無い）。

### 結果 ③ — 依頼の 2 例に対する答え

#### `++`（インクリメント）: **文位置なら AST 追加ゼロで通る**

`lexer/keyword.rs` と `parser/stmts/assignment.rs` で `CompoundAssign` /
`AttrCompoundAssign` へ**脱糖するだけ**。パーサ内脱糖の前例あり（f-string → `str()` 呼び出し・
`parser/exprs.rs`）。`AttrAssign { target: Expr }` が属性と添字を一律に受けるので
**lvalue の分岐も増えない**。⇒ **上記 C の穴を 1 つも踏まない**。
⚠ `a[f()]++` が `object` / `index` を 2 回評価するのは**既存の `+=` と同じ**
（`vm/compiler/stmt_assign.rs` に「ツリーウォークと副作用まで一致させるため」と明記済み）
＝ 新しい問題ではない。

⚠⚠ **式位置（`arr[i++]`）は別物**。現在 `Expr` に**変数へ書き込む variant は 0 個**で、
副作用は `Call` の先にしか無い。`compile_attr_compound_assign` の
「value が副作用を持たないなら `Swap` を丸ごと落とす」融合など、**複数の最適化が
「式は概ね純粋」を前提**にしている。**判定は上記 B の保守的ホワイトリストなので既定は安全側**
＝ 今すぐ壊れはしないが、式位置を入れるなら**この既定に依存している箇所を先に列挙する**必要がある。
⇒ **起票はしない**（言語設計の決定が先）。**式位置 `++` を採用すると決めたときの前提**として記録する。

#### メタプログラミング用 `expr`: **詰まるのは AST 側ではなく `Value↔AST` 境界とパイプライン入口**

追い風（既にある）:

- **AST は既に第一級の値** — `ast_value.rs`（735 行・Stmt 40 / Expr 29 が **exhaustive**）。
  variant を足せば**必ずコンパイルが止まる**（結果 ① の実測どおり）
- **「実行時に AST を作って実行する」は `templates.rs` が実証済み**
  （`subst_stmt` / `subst_expr` ＋ #7 の `(テンプレート, 型引数)` キー Chunk メモ化）
- **合成 AST は `node_id: 0` / `Resolution::Unresolved` でも正しく動く**（上記 B）＝「遅いが動く」が既定
- **パイプラインが `&mut Vec<Stmt>` の独立パスに分かれている**（`resolve_program(&mut stmts)`）
  ⇒ **展開パスは parse と type_check の間にそのまま挟める**

向かい風:

- **`Value → AST` の逆変換が無い** — `ast_value.rs` は**一方向のみ**
  （`value_to_stmt` / `value_to_expr` は grep で **0 件**）。今できるのは
  「AST を読む」「文字列を組んで `parse_ar`」まで。
  ⚠⚠ **`Value` は動的なので match で守れない** ＝ 逆変換を書くと **C の穴が一気に増える**
- **AST→実行の入口が無い** — `compile_debug` は `Stmt::Expr` / `Stmt::Let` の
  **2 形だけ**で `_ => return None`
- **パイプライン配線が 3 入口に手写し**。⚠ しかも**関数まで違う** —
  `run_program` は `TypeChecker::check_program`、`--compile` と REPL は
  `TypeChecker::check_and_annotate`（＋テストヘルパー 1 本）

### 起票（#84〜#89）

⚠ **採番は別スレッドの #75〜#83 の続き**。この評価は当初 4 件を提案したが、そのうち
**「`Expr` 版 `decl_names`」は #81 が、「クロージャ捕捉の穴」は #75 が先に実装済み**だったため
**取り下げ、HEAD で測り直して立て直した**。⇒ **評価の数字は必ず対象の HEAD で取り直すこと**。

| # | タスク | 手法 | 前提（依存） | 状態 |
|---|---|---|---|---|
| 84 | `Stmt` 本体への降り方 6 本 | `expr_walk` の `Stmt` 版（**列挙だけ** exhaustive 化・判断は walker に残す） | — | 未着手 |
| 85 | 未カバー構文の機械カウント | exhaustive な `stmt_kind` / `expr_kind` を `compile_*` 入口で bump し例題横断で集計 | — | 未着手 |
| 86 | VM の slot 採番の不変条件検査 | デバッグ名テーブル（§2.3）と `LoadLocal` / `StoreLocal` の slot を突き合わせ | — | 未着手 |
| 87 | `peephole::code_target_mut` の `_ => None` | 全 89 variant を列挙して**op を足すと止まる**形へ | — | 未着手 |
| 88 | パイプライン配線 3 入口の手写し | まず 3 入口の**差を測る**（畳まない判断もありうる） | — | 未着手 |
| 89 | メタプロの前提（`Value→AST` ＋ AST→実行の入口） | 逆変換 ＋ `compile_debug` の一般化 | **← 84・85 ＋ 消費者の出現** | 別レーン・優先度低 |

各件の補足:

- **#84** — ⚠⚠ **#59 と #81 はどちらも「降り方は walker ごとに本当に違う」として意図的に外した**。
  したがって畳むのは「**どの `Vec<Stmt>` がぶら下がるか**」の列挙だけで、
  **降りる／降りないの判断は walker に残す**（`expr_walk::SubPart::Body` と同じ形）。
  ⚠⚠ **列挙の順序＝採番の順序**（`Try` は「本体 → 別名 → handler 本体」）。並べ替えると
  **`LoadLocal` が別の変数を読む**。⚠ `collect_base_decls` は**既定が安全側**（未知の文で
  解決を諦める）なので、**対象に含めるかは測ってから決める**（#72 の「読み手ごとに理由がある」）。
- **#85** — **最も安く、最も効く**。プラン冒頭が「**未カバーの構文を例題側から数える手段がまだ無い**
  （`Stmt` の全 variant × 文脈のマトリクスが無い）」と明記しており、**#56 / #68 / #71 / #75 の
  実バグ 4 件すべての共通原因**（「その形の例題が 1 本も無かった」）に直接効く。
  ⚠ **材料は既に揃っている** — exhaustive な `tw_stats::stmt_kind`（40 アーム）と
  `vm::compiler::expr_kind`（30 アーム）、集計機構 `bump` / `record_bail`、
  [tw_stats.ps1](tw_stats.ps1)。⚠ **`--features tw_stats` 側に閉じる**（#69 の教訓で
  `--features prof` / `--features tw_stats` の両方を通すこと）。
- **#86** — ⚠ **速度に触るので `#[cfg(debug_assertions)]` に閉じる**
  （#10-b: `exec_op` は `#[inline(always)]`・診断フックは 11% 退行の実績あり）。
  ⚠ #84 と同時にやると**検出力の負の対照が取りやすい**（採番を 1 つずらして止まるかを見る）。
- **#88** — ⚠⚠ **畳むのが正解とは限らない**。#72 は「読み手 5 つ・探索方針 4 通りで
  **どれにも理由がある**」で**方針を畳まなかった**。⇒ **先に 3 入口の差を測ってから**決める。
- **#89** — 判定基準は「**消費者が居るか**」（#11 R2-c / #14 / #15b と同じ）。
  **今は消費者が居ない**ので優先度低。⚠ 着手するなら **#84 / #85 が先**
  （逆変換は match で守れないので、`Stmt` 側の強制と未カバー計測が無いまま入れると
  C の穴が制御不能になる）。

## #84 `Stmt` 再帰骨格の 7 本を [`stmt_walk`](src/stmt_walk.rs) へ 1 本化（2026-08-25・完了）

#59（[decl_names.rs](src/decl_names.rs)＝「`Stmt` はどの名前を束縛するか」）と
#81（[expr_walk.rs](src/expr_walk.rs)＝「`Expr` の直下に何があるか」）が強制化した残りの半分、
**「文の下にどの本体・部分式がぶら下がるか」**を 1 箇所へ。
⚠ **どちらの doc も明示的に「範囲外」と書いていた**ので、3 つ目の欠けている半分だった。

### ⚠⚠ 畳む前に表を突き合わせたら、実バグが 3 件出た

vm-pitfalls §3 の「**畳む前に元の N 実装がそれぞれ何を見ていたかを表にして突き合わせる**」
（#81 の教訓）を実際にやった。**8 walker × 40 variant の表を機械生成**したところ、
制御フロー本体へ「6 本は降りるが 1 本だけ降りない」というセルが 3 つ見つかり、
**3 件とも実バグ**だった（いずれも `impl_python` は正しく実行できる正しいプログラム）。

| # | ずれ | 症状 | 再現 |
|---|---|---|---|
| ① | `exec::collect_declared_names` **だけ**が `Stmt::Match` のアーム本体へ降りない | アーム内の宣言が「自前の名前」から漏れ、**自由変数として捕捉**され、採番側（`collect_nested_decls` は降りる）の slot と衝突 ⇒ `capture-slot-conflict` で **`VmForceError`** | `impl_python` は 109 |
| ② | 同 walker が**部分式へも降りない** | ①と同じ衝突が `let q = block ->T: …` 経由でも起きる（**別経路**） | `impl_python` は 109 |
| ③ | `vm::compiler::decls::collect_nested_decls` **だけ**が `Stmt::Static` の初期化式へ降りない | `static mut s = block ->T: …` の本体宣言が slot を取れず **`decl-no-slot`** で `VmForceError` | `impl_python` は 7 |

⚠⚠ **#68 と同じ壊れ方が 2 度（①②）**。#68 は `decl_names`（名前）側のずれ、今回は**降り方**側のずれで、
**#59 が「降り方は walker ごとに違う」として意図的に残した差**がそのまま実バグになっていた。
⚠⚠ **3 件とも「例題が 1 本も無かった」** — #56 / #68 / #71 / #75 と**同じ再演で 5 度目**。

### 突き合わせた表（機械生成・`O` ＝ その variant のアームを持つ）

母集団は**本体または部分式を持つ 14 variant**（`Vec<Stmt>` / `MatchArm` / `ExceptHandler` を持つ文）。
ずれていた 3 セルを **★** で示す。

| variant | nested_decls | shadow_stmts | body_bails | fn_free | bound_names | declared_names | referenced |
|---|---|---|---|---|---|---|---|
| `If` / `While` / `For` / `Block` / `Try` | O | O | O | O | O | O | O |
| `Match` | O | O | O | O | O | **★ .** | O |
| `Static`（初期化式） | **★ .** | O | . | . | O | . | O |
| 部分式一般 | O | O | . | . | O | **★ .** | O |
| `FnDef` / `GenDef` | . | . | . | O(fn のみ) | O | . | O |
| `ClassDef` / `TraitDef` | . | . | . | . | O | . | O |
| `ProtocolDef` | . | . | . | . | **.**（意図） | . | O |
| `Import` / `FromImport` | . | . | . | . | . | . | **.**（意図） |
| `AsyncAssign` | . | . | . | . | O | . | O |

⚠ **`.` がすべてバグではない**。#81 の 3 分類で仕分けた:
①**本当の取りこぼし**（★ の 3 件 ＋ `collect_bound_names` の `AttrAssign`/`AttrCompoundAssign`/`Raise`）
②**意図的な差**（`ProtocolDef` はシグネチャ宣言だけ／`Import` は別モジュール／
パターンへ降りると**採番がずれる**／async 本体は別チャンク）
③**現状観測できない**（`nested_fn_free_names` が `GenDef` を見ない ⇒ 下記）

### 手法 — #59 / #81 と同じ 2 段の強制

1. `each_subpart` の `match stmt` は **exhaustive**（`Stmt` に variant を足すと止まる）
2. `StmtPart` を消費側は**すべて exhaustive match**（種類を足すと**全 walker が止まる**）

`StmtPart` は **11 種**。⚠ **本体の種類を細かく分けてある**のは walker ごとに本当に判断が違うため:

- `Control`（同じフレーム）／`FnBody { params, body }`／`GenBody`／`TypeBody`／`ProtocolBody`／
  `ModuleBody`／`AsyncBody` — **7 本が全部違う組み合わせで降りる**
- `Expr`／`MatchPattern` — パターンへ降りるのは**参照を集める walker だけ**（#81 と同じ判断）
- `ForTarget`／`ExceptAlias` — **束縛**で**採番順に意味がある**（#59 が `decl_names` に載せなかったもの）
- `TargetName` — `x = …` の左辺・`freeze x`・`mng <- async` の `mng`。**束縛ではなく既存名**

⚠ `FnBody` だけ `params` を持つ。**これが無いと `nested_fn_free_names` に 2 つ目の
`match stmt` が残ってしまう**（自由変数解析は「仮引数 ＋ 本体の宣言」を自前の名前とするため）。
⚠ `GenBody` に `params` は無い（要るのは `fn` の解析だけ）。

### ⚠⚠ 2 段目が実地で発火した（負の対照ではなく本番で）

`TargetName` を足した瞬間に**消費者 6 本がコンパイルエラーで停止**した（当時の変換済み本数）。
⚠ #81 の `MatchPattern` に続く **2 例目**。⚠ ただし**止めてくれるのは 2 段目だけ**で、
「`TargetName` を足すべきだ」と気づいたのは**人間が `dead_code` 警告を読んだから**
（`MatchPattern` / `ProtocolBody` の payload を誰も読んでいなかった ⇒ 7 本目の walker を
変換する判断につながった）。

### 変換した 7 本／しなかった 1 本

| walker | 変換 | 備考 |
|---|---|---|
| `vm::compiler::decls::collect_nested_decls` | ✅ | **採番順に意味がある**（`For`＝ターゲット→iter→本体、`Try`＝本体→別名→handler→finally）。③を修正 |
| `vm::compiler::decls::scan_shadow_stmts` | ✅ | ついでに**①も `decl_names` へ委譲**（手書きの名前収集を削除。集合は完全一致を確認） |
| `vm::compiler::decls::block_body_bails` | ✅ | `Stmt::Return` の判定だけ手前に残す |
| `vm::compiler::decls::nested_fn_free_names::walk` | ✅ | `FnBody { params, body }` で**丸ごと委譲**（外側の `if let` を削除） |
| `interpreter::resolver::collect_bound_names` | ✅ | `AttrAssign`/`AttrCompoundAssign`/`Raise` などへ**新たに降りる**ようになった |
| `interpreter::exec::collect_declared_names` | ✅ | ①②を修正。式側は新設 `collect_declared_in_expr` が [`expr_walk`](src/expr_walk.rs) を歩く |
| `interpreter::exec::collect_referenced_names_stmt` | ✅ | **#75 で既に exhaustive** だったが、`MatchPattern`/`ProtocolBody`/`TargetName` を読む唯一の walker なので変換した |
| `interpreter::resolver::collect_base_decls` | ❌ | ⚠ **そもそも降りない**（関数本体の直下しか見ない）。問いは「**この文を理解できるか**」で、未知の文は `false` を返して解決を諦める＝**既定が安全側**（#59 が `decl_names` で下したのと同じ判定） |

⚠ **`partial_compiler/llvm_codegen` の `Stmt` walker 5 本は対象外**
（`walk_stmts`・`stmt_eligible`・`gen_stmt`・`stmt_writes_param`・`stmt_has_loop_yield`）。
`_ =>` の既定が「**ネイティブ非対象**」＝**安全側の劣化**で、#84 の起票時評価の分類 B。

### ⚠ 直さなかったもの（起票候補）

**入れ子 `gen` が外側の可変ローカルを捕捉すると `VmForceError`**（`impl_python` は 6）。
⚠ 原因は #84 ではない — `decl-prepass:GenDef` で**入れ子 `gen` は捕捉の有無に関係なく
VM 非対応**（実測で確認）。`nested_fn_free_names` が `GenDef` を見ないのはそのため
（③ の分類＝現状観測できない）。⇒ `gen` の VM 対応と同時にやる別タスク。

### 検証（**すべて自分で走らせて確認**）

| ゲート | 結果 |
|---|---|
| `cargo build` / `--features prof` / `--features tw_stats` | **警告 0**（3 つとも） |
| `cargo test` | **750 passed** |
| `cargo clippy` / `--all-targets` | **50 / 65** ＝ ⚠ **同じコマンドで HEAD を測り直して 50 / 65**（増分 0） |
| [compare_bytecode.ps1](compare_bytecode.ps1) | **114 / 114 byte-identical**（⚠ 先に**同一 exe の負の対照**で 114/114 を確認してから読んだ） |
| [compare_outputs.ps1](compare_outputs.ps1) | **96 / 96 identical** |
| [compare_python_impl.ps1](compare_python_impl.ps1) | **53 / 53 identical**・unexpected diff 0 |
| [scan_examples.ps1](scan_examples.ps1) / [force_gate.ps1](force_gate.ps1) | **FAIL 0** / **0 件・161 例題** |
| [tw_stats.ps1](tw_stats.ps1)（⚠ **VM の適格範囲に触ったので必須**） | `in_fn` **0**・`tw_control_flow` **0**・`vm_bail_fn` **0**・`vm_ineligible` **0** |
| [repl_session.ps1](repl_session.ps1) / [debug_session.ps1](debug_session.ps1) | identical / **5 identical** |
| [stale_doc_refs.ps1](stale_doc_refs.ps1) | **0 件** |

⚠ **`force_gate` は 1 度やり直した** — 走行中に `src/` へプローブを当ててしまったため
（vm-pitfalls §4「タイムアウトを持つゲートを並走させない」の再演）。
**止めて、`src/` がバックアップと一致することを確認してから再実行**した。

⚠⚠ **bytecode が 114/114 一致した ＝ 3 件の実バグに例題が 1 本も無かったことの裏取り**。
網を広げても既存例題では 1 バイトも動かない（#81 と同じ結論）。

⚠ **clippy の基準値は当てにしない** — プランには「50 / 63」、#75 の実装ログには「51 / 66」と
**3 通りの数字**があった。⇒ **同じセッションで HEAD を stash して測り直す**のが唯一の方法。

### 負の対照（検出力の確認）

| 対照 | 結果 |
|---|---|
| `StmtPart` に variant を +1 | **消費者 7 本ちょうど**が停止（＝変換した 7 本と一致） |
| `Stmt` に variant を +1 | **7 箇所**が停止（`each_subpart` を含む。構成は変わったが数は同じ — ⚠ **利得は件数ではなく「1 箇所が 7 walker を代表する」こと**） |
| 新例題を**修正前バイナリ**で実行 | **exit=1**（`VmForceError`） |
| ①②③ を**個別に**修正前バイナリで実行 | **3 件とも個別に exit=1**（⚠ 1 例題 = 1 原因ではないので分けて確認した） |

### 成果物

- [src/stmt_walk.rs](src/stmt_walk.rs) 新設（274 行）
- `Stmt` を歩く walker **45 → 39 本**・うち `_ => {}` **35 → 29 本**（どちらも -6）
- 例題 [examples/basics/nested_decl_scopes.ar](examples/basics/nested_decl_scopes.ar) 新設
  （①②③ ＋ **対照 3 種**: 同じ宣言を文形式で書いた場合／`for` ターゲット／`except as` 別名。
  ⚠ 後 2 つは**採番順を崩すと値が壊れる**ので順序の検査を兼ねる）
- ⚠ `ModuleBody` の payload だけ `#[allow(dead_code)]`（**7 本すべてが降りないと決めている**ため）。
  **`allow` の範囲は enum の 1 フィールドのみ**＝ #51 の「属性が別の警告を食う」は起こりえない

## 見積もりと実測

| 事前の想定 | 実際 |
|---|---|
| 「#81 は 3 本を委譲にするだけ」 | **半分外れ**。**4 本目**（#75 で直した `collect_refs_expr`）も寄せた。さらに **`MatchPattern` の取りこぼし**が出て、**仕掛けが停止させた** |
| 「exhaustive にすれば取りこぼしは無くなる」 | **外れ**。止めてくれるのは **2 段目**（種類を足したとき）だけ。**そもそも列挙し忘れる**のは人間が読むしかない |
| 「`_ => {}` を消すと挙動が動くはず」 | **動かなかった**（bytecode 114/114）。`Slice`/`TemplateInstantiate` の中に束縛を書く例題が無いため。**網が広がったこと自体は正しい方向** |
| 「#82 は 4 ブロックを 1 つにするだけ」 | **当たり**。ただし **5 本目の walker という穴は畳んでも消えない**ので、明記に留めた |
