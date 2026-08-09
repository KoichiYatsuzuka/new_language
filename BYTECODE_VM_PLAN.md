# 実行方式改善計画 — AST 解決層（Phase R）＋ バイトコード VM（Phase V）

Arrow（LLVM IR ターゲットのスクリプト言語, Rust 実装 `src/`）の**非ネイティブ実行経路**を高速化する計画。
現状の「AST ツリーウォーク」を、`AST → 解決層 → バイトコード VM` に置き換える。

> **この文書は単独で実装再開できることを目的とする**（`REFACTORING_HANDOFF.md` / `PHASE5_PLAN.md` と同じ引き継ぎ文書）。
>
> **読み方（トークン節約）**: 「実装状況」→ §0 → §2 → §4/§5/§6 → §10 までが**実装再開に必要**。
> ページ後半の `═══ 参照・根拠 ═══` 以降（§3 アーキテクチャ決定・§1 背景実測・§7 投影・§8 非目標・§9 未決）は
> **決定済みの根拠／履歴**であり、通常は**読まなくてよい**。進捗の一次資料は [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md)。

---

## 実装状況（2026-07-26 時点）

各段の実装詳細・実測は [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md)。off/auto の byte-identical を常時維持。

### 完了 ✅
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
    さらに **段階(c-1)/(c-2)**（2026-08-10・未コミット）でネイティブ codegen へ注釈を配線し実測した結果、
    **§4.4 が置換対象に挙げた `field_ty` は実質デッドコード**・**注釈は codegen の適用範囲を広げられない**ことが判明。
    ボトルネックは**型検査が for ループのターゲットを `Unresolved` で宣言していること**だった。
    詳細と残り・git 状態は **#16 の「段階と実装状況」節**（本文後半）に集約。

### 残り（番号付き）
**Phase V の残り**
1. **V-E 本体** — op→Span の**汎用行テーブル**（VM 関数内のステートメント単位ブレークポイント）。トレースバック・REPL 用途は達成済み。
   **【一部対応 2026-07-27／完全版は保留】** 完全版（op→span 行テーブル＋VM ディスパッチループの停止判定＋停止フレームの
   buffer ローカルを REPL から名前参照する経路）は大規模のため保留。代わりに**デバッグ中（`DBG_MODE != Inactive`）は
   VM を無効化しツリーウォークに委ねる暫定対応**を実施（`dbg_active()` を `vm_eligible` に追加）。これにより
   `--vm=auto` でも would-be-VM 関数へステップイン（文単位停止・変数参照）が `--vm=off` と byte-identical に動作する。
   完全な VM ネイティブ行テーブルは、対話デバッグの実効速度が問題化した場合にのみ着手する（現状は不要）。
2. **V-F 最適化** — peephole・superinstruction・単型算術命令・R0-A エスケープ解析（非エスケープフレームのフラット確保）。
   **【一部対応 2026-07-27】superinstruction を実施**: `local <op> local` → `BinLocalLocal`、`local <op> リテラル`
   → `BinLocalConst`（コンパイラが融合 emit・意味論不変=`apply_bin_fast` 委譲）。算術支配ループで auto ~1.15x。
   **残り（保留）**: peephole（Jump 除去等はジャンプ先再マップが要り中規模）・単型算術命令（型注釈依存で要検討）・
   **R0-A エスケープ解析は #12 のフレームモデル前提のため #12 とセットで保留**。
3. **強制バイトコード（D2）** — 全構文カバー後にフォールバック撤去 → **スレッドローカル4本＋センチネル2種を実削除**（現在はデュアルモードのため保持）。

**VM カバレッジ拡大（独立）**
4. ~~メソッド呼び出し機構の軽量化~~ 【✅ 完了】（高速バインド経路で method_hot 1.61x・method_body 1.64x）。
5. ~~添字 `obj[i]`・コレクションリテラル（list/dict/set/tuple）の VM 化~~ 【✅ 完了】（デバッガ REPL のフォールバックも減った）。
6. ~~その他組み込み（enumerate/zip/str/int 等）の `CallBuiltin` 拡張~~ 【✅ 完了】（純粋6種＝`CallBuiltin`・型コンストラクタ＝`LoadGlobal`+`Call` 委譲。enumerate/zip 支配 1.49x）。
7. ~~テンプレート実体化の Chunk メモ化~~ 【✅ 完了】（`(テンプレート, 型引数)` キーで具体 FnValue をメモ・関数/ジェネレータのみ・auto 5.9x）。
8. ~~ジェネレータ本体の VM 化~~ 【✅ 完了】（`Yield` op・eager 収集維持・`GeneratorFn` 呼び出しギャップ修正・~3.2x）。
9. ~~async の VM 対応~~ 【✅ 完了（関数内）】（`AsyncSubmit` op・frame から capture 再構成・既存 `capture_env` 再利用・D5 維持。モジュール top-level の async は #10 依存）。
10. **import モジュール Chunk**（モジュール本体の一括生成）。**【保留 2026-07-27】** 調査の結果、高コスト・低効果と判明:
    (a) モジュール本体は定義文（fn/class/gen/import）が支配的で VM コンパイラが全て bail →「本体一括 Chunk」には
    **定義文の VM オペコード化**が必要。(b) top-level 変数はグローバルだが VM に **`StoreGlobal` op が無い**（slot ベース）。
    名前ベース（`LoadName`）で回避すると**ツリーウォークと同コスト＝速度向上ゼロ**。(c) ホットコードは関数内で既に
    VM 化済み、モジュール top-level は一回きりの初期化＋定義が主で実効メリット小。着手時は「グローバル変数実行モード
    ＋全定義文オペコード化」の大規模拡張が要る点を織り込むこと。

**Phase R の残り**
11. **R2 グローバル slot の前倒し**（`SlotCache` 実行時キャッシュ → AST 展開時解決, §4.3）。
    **【一部対応 2026-07-27／本体は保留】** VM の `LoadGlobal` に **op レベルの runtime index cache**
    （`Chunk.global_caches` の `SlotCache`・`(slot_epoch, scopes[0] index)` を焼く）を追加し、グローバル変数読み・
    グローバル関数呼び出しの名前ハッシュ引きを索引直読みへ置換した。ただし**実測は ~2-3%**（グローバル名の
    FxHash 引きは元々安価で、呼び出しコストは `bind_args`/フレーム構築が支配的）。**resolve-time R2 本体**
    （固定 index 採番のグローバルシンボル表＋グローバル記憶域の index 配列化＋§6 モジュールモデル接続）は
    中〜大規模かつ限界的メリットが小さいため保留。§6（#14）着手時に併せて実施する。
    → **知見: 変数・関数スロットのアクセス最適化は R1（ローカル=flat buffer）/R3（フィールド IC）/R4（呼び先）/
    #2（superinstruction）/#11（グローバル索引）で概ね頭打ち。残る速度余地は「呼び出し機構」（bind/フレーム構築、
    ~630ns/call）にあり、これは #12 のフレームモデルが対象。**
12. **R0-A 明示フレームスタック**（`Rc<Frame>`・深い再帰のスタックオーバーフロー解消・クロージャ Rc 寿命管理, §3.4-A）。
    **【保留 2026-07-27・統一目的に照らして価値薄と判断】**
    ユーザー確認により、本系列の真の目的は**「バイトコード化とプリコンパイル（ネイティブ codegen）の動作統一」＝
    AST 解決を可能な限り低レベルへ押し込み、両経路が同一の解決済み AST を消費すること**（速度は主目的でない）と判明。
    この基準で評価すると R0-A は**該当しない**:
    - R0-A は**ランタイムのフレーム記憶域**（`Rc<Frame>`）の変更であって **AST 解決注釈の追加ではない**。解決
      （名前→slot）は R1 で済んでおり、R0-A はその slot の実行時格納方法を変えるだけ＝「解決を低レベルへ押し込む」
      には非該当。
    - ネイティブ経路（`partial_compiler/llvm_codegen`）はインタプリタのランタイムフレームを一切使わない（LLVM
      alloca へコンパイル）ため、**R0-A は統一目的に寄与ゼロ**。R0-A の価値（深い再帰の堅牢性）は統一目的と無関係。
    - 現状すでにネイティブは独自解決で typed ABI 出力を生成可能（統一の「機能面」は達成済み・共有していないだけ）。
    → 深い再帰の堅牢性が独立の要件として浮上した場合にのみ、非再帰ループ（トランポリン）とセットで別途着手する。
13. **R4 ネイティブ codegen 側の消費**（§4.4・`llvm_codegen` の自前解決を Phase R 結果へ置換）。
    **← 統一目的（バイトコード↔プリコンパイル）の本命レバー。** ネイティブ codegen は現状 Phase R 注釈
    （`LocalRef`/`AttrCache`/`SlotCache`）を**一切消費せず** `locals`/`param_classes`/`field_ty`（[context.rs](src/partial_compiler/llvm_codegen/context.rs)）
    で独自再導出している（確認済み・grep 0 件）。ここを Phase R の解決済み AST 消費に置換すれば「両経路が同一解決を
    共有」が実現する。#12 ではなくこれが「AST 解決を低レベルへ押し込む」目的に直結する。

**その他（§6・§7.4）**
14. **§6 モジュール動的リンク**（ディスクリプタシンボル＋ABI ハッシュ照合）。**【保留 2026-07-27】** ネイティブ
    `.arc` の動的リンク方式であり、「AST 解決の低レベル化・両経路統一」という当面の目的とは別軸（リンク時 ABI 照合）。
    統一レバー #13 とは独立のため、モジュールモデル（#10/R2/#11 本体）着手時にまとめて扱う。
15. **§7.4-1 `Value::Str(String)` → `Rc<str>`** ／ **§7.4-3 文字列インターン**。

**統一基盤（新規・2026-08-02 昇格 / 2026-08-03 具体化）**
16. **AST 型解決層 — コンパイル/ツリーウォーク/バイトコードの挙動統一**（#13 を包含・plan A の土台）。**【新タスク・本命】**
    型検査器が既に全式ぶん計算している型（`infer(&Expr)->InferredType`・[infer.rs:10](src/type_check/infer.rs#L10)）を
    **node-id 別テーブルへ焼き込み**、可能な限り低レベルまで解決する（具象型・メソッド/フィールドのバイトオフセット・
    呼び出しシグネチャ・検査要否）。解決点は決め打ち（直接オフセット/直接ディスパッチ・検査なし）、未解決点のみ検査指示を付す。

    #### 目的（速度ではない・承知の上）
    (1) コンパイル時とツリーウォーク時の**挙動統一**。(2) バイトコード化・各最適化で生じがちな「型が確定しているか/どの経路か」
    等の**例外的な条件分岐を AST 段階で解消**（各経路が単純化）。速度貢献は現段階で限定的（VM は R3/メソッド IC が緩衝。#3 実測:
    overloaded 演算は「探索」~2-3% のみ解決可能・残りは呼び出し機構＋確保で不可避。**ネイティブは IC が無いぶん効果が桁違い**）。

    #### 注釈モデル（採用: node-id ＋ 2直交側テーブル ＋ 型インターン表）
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

    #### 検査要否の決定（点2/3・調査で確定）
    - **要検査 → `CheckBefore`**: `Expr::MustBe`（不一致で raise・[ast.rs:538](src/ast.rs#L538)）／`Expr::Cast`（`__cast__`/コンストラクタ
      ディスパッチ・[ast.rs:513](src/ast.rs#L513)）／他言語 FFI 境界／**非コンパイル Arrow ライブラリ境界**（コンパイル済みは typed ABI 保証で無検査）／
      型が `Any`/`Protocol`/`Union`/`Unresolved` の消費点。
    - **無検査（点3）は自動的に満たされる**: 型ガードは [check.rs:376](src/type_check/stmt/check.rs#L376) が**絞り込んだ型で変数を分岐スコープに
      再宣言**して実現。したがって**分岐内の各出現で `infer` は絞り込み済み具象型を返す**＝そこへ焼けば自然に無タグ。**別途 narrowing 抽出は不要**。

    #### 消費者（三経路が同一注釈を消費）
    - **ツリーウォーク**（`eval`）: 注釈があれば直接オフセット/直接ディスパッチ、`CheckBefore` があれば動的検査。
    - **バイトコード**（plan A）: 解決点は特化 op、`CheckBefore` は検査 op。**`Value` は boxed 維持**（内省保持・unbox=解釈B は非採用）。
    - **ネイティブ codegen**（#13）: `context.rs` の独自再導出を**この注釈の消費に置換**。typed 機械語を生成し**不要な型情報は消去**。
      **境界の動的検査は CALL 注釈の引数検査指示＋型インターン表から呼び出し前にインライン生成**
      （＝点4「ライブラリ内テーブルで呼び出し前に検査」を**この機構に畳み込み・別機構は不要＝削除**）。

    #### 入口コスト（調査済み・#1）
    型は既に `infer` で計算済みだが `&Expr` を取り**破棄**、`check` はエラーのみ返す（書き戻し 0 件）。第一歩は
    **「型検査の走査中に node-id テーブルへ型＋検査指示を書き込む」**（narrowing はスコープ再宣言で `infer` に既に反映済み＝
    検査走査中に書くのが最適）。型推論の再実装は不要＝中規模。

    #### 段階と実装状況（2026-08-05 更新・**コンテクストなし再開用ハンドオフ**）
    段階: **(a) 注釈永続化層** → **(b) ツリーウォーク/VM が消費（plan A）** → **(c) ネイティブ codegen が消費（#13）**。
    関係: R1/R3/R4 の解決注釈を**型情報まで拡張・一本化**。#13 を包含し、plan A と #11 resolve-time R2 はこの層の**消費側**。

    ##### ✅ 段階(a) 完了（注釈生成＋ランタイム配線）
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

    ##### ✅ 段階(b) 第1増分 完了（plan A: 型特化二項演算）
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

    ##### ✅ 段階(c-1)/(c-2) 完了（ネイティブ codegen への注釈配線＋実測）— 2026-08-10
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

    ##### 🔍 c-2 の結論（**プランの前提が実測で覆った・要判断**）
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

    ##### ⬜ 残り（次スレッドの着手候補）
    - **段階(b) 続き**: (i) 属性/メソッドの静的ディスパッチ化（静的クラス確定時に R3/メソッド IC のチェックを省く）、
      (ii) `CheckBefore` 指示の消費（境界での明示的動的検査 op 挿入・現状は生成のみで未消費）、(iii) 比較以外・混在 int/float の特化。
    - **【新規・段階(c) の前提】for ループターゲットの要素型推論**（上記 c-2 結論 4）。これ無しに c-3 を進めても効果ゼロ。
    - **段階(c-3)**: 自前再導出の撤去＋`CallInfo` の引数検査指示を境界インライン検査へ。**上記の前提を満たしてから**。
    - **テンプレート対応**: 現状 templates は注釈が型変数のまま＝実体化時 subst が要る（非テンプレ関数のみ充填済み）。
    - **モジュール横断**: node-id は per-module 採番。現配線は**メインプログラムの注釈のみ**注入（import モジュール関数は各自の
      node-id 空間＝未注入＝安全にフォールバック）。段階(b)/(c) でモジュール横断の注釈管理が要るならここで対応。

    ##### ⚠️ 別途の再検討事項（本タスク範囲外）
    - **`Ident` 表現の再設計**: `Ident` は葉の名前参照だが**タプル変種 `Ident(String)`・97 サイト**で node-id 化は極めて侵襲的。
      現状は「Ident に注釈せず型は消費側（BinOp/Call/Attr）で捕捉」で回避。将来「Ident を構造体変種化して node-id/解決情報を
      持たせる」等の AST 再設計は独立に評価する。

    ##### 🔧 検証スクリプト（本タスク用に追加）
    - [dump_native_ir.ps1](dump_native_ir.ps1) — 代表 6 モジュールを `--compile` して生成 LLVM IR を保存
      （`AR_DUMP_LL` フック・[module_compiler.rs](src/partial_compiler/module_compiler.rs)）。codegen 変更の前後で
      ハッシュ比較し **IR byte-identical** を確認する。`.arc`/`.ars` は退避・復元するので作業ツリーは汚れない。
    - [annot_diff.ps1](annot_diff.ps1) — `AR_ANNOT_DIFF=1` で「自前導出 vs 注釈」の一致内訳と、
      `Ty::Handle` へ落ちた式のうち注釈が具象型を持つ件数を出力する。
    - [compare_vm_modes.ps1](compare_vm_modes.ps1) — 例題を `--vm=off` / `--vm=auto` で走らせ **stdout byte-identical** を検証。
      `examples/bench` は経過時間を出力するため既定で除外（`-IncludeBench` で含める）。1 例題ごとに `-TimeoutSec`。

    ##### 🔧 現在の git 状態（**重要**）
    - 段階(a)/(b) の実装コードは**コミット済み**（ブランチ `byte-code`・コミット **`#13-1`〜`#13-5`**＝
      `4034f62`/`f35a0d2`/`04be005`/`086ec6c`/`9261855`）。`cargo test` **686 緑**・**警告0**・off/auto byte-identical を確認済み。
    - **段階(c-1)/(c-2) は未コミット**（2026-08-10・ユーザー許可待ち）。`cargo test` **686 緑**・**警告 0**・
      代表 6 モジュールの **IR byte-identical**・off/auto **33 例題すべて byte-identical**（differing 0）を確認済み。

### 実装メモ（プラン記述からの差分・追記）
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

## 0. この文書の使い方 / 現在の決定状態

### 0.1 全体像（2フェーズ）

```
                        ┌─ 解釈経路: 注釈付き AST を フレーム/slot でツリーウォーク（Phase R 中間状態）
source → lexer → parser ┤                              ↓ その上に
        → type_check     │                        バイトコード VM を載せる（Phase V, 強制バイトコード）
        → [Phase R: 解決層]┤
                        └─ ネイティブ経路: 注釈付き AST → LLVM（既存 codegen を簡素化・拡張）
```

- **Phase R**（解決層）は `src/interpreter/`（解釈）と `src/partial_compiler/`（ネイティブ）の**両方**が消費する共有基盤。
- **Phase V**（バイトコード VM）は解釈経路の**最終実行形**。解釈実行は最終的に**強制バイトコード**（ツリーウォークは通常実行から除去、ただし `eval()`/`exec()` は §2.1/§2.3 のツール用に残置）。

### 0.2 決定済み事項（確定・再litigateしない）

| # | 決定 | 根拠 |
|---|---|---|
| D1 | **共有 IR は AST 解決（X 案）**。バイトコード→機械語（Y 案）は**不採用** | §3.1 |
| D2 | **解釈実行は強制バイトコード**（最終状態）。移行中のみデュアルモード | §3.2 |
| D3 | **スタックマシン**（レジスタマシンではない） | §3.3 |
| D4 | **変数解決モデル R0**: A=参照カウントフレームのスタック / B=関数内固定 slot / C=動的名はほぼ静的解決 | §3.4 |
| D5 | **async は share-nothing 維持**。deepcopy 削減は COW/不変 Arc 共有/move、共有可変は明示プリミティブ | §3.5 |
| D6 | **モジュール動的リンク = ディスクリプタシンボル + モジュール単位 ABI ハッシュ照合** | §3.6 / §6 |
| D7 | フェーズ0（計測）完了・ゲート通過。支配項＝呼び出し機構・引数束縛・ローカル名前引き | §1.5 |
| D8 | Phase 5（TypeChecker 分割）完了・main マージ済み。着手障害なし | — |

### 0.3 実装順序

1. **Phase R**（§4）— R0 ランタイムモデル導入 → R1〜R4 解決ステップ。各ステップ独立コミット可、`cargo test` 緑維持。
2. `bench.ps1` 再測定（§4.5 ゲート）— 呼び出し機構がなお支配的かを確認（Phase V の設計裏付け）。
3. **Phase V**（§5）— V-A〜V-F。バイトコード VM を R0 のフレーム/slot 上に載せる。
4. **モジュールリンク**（§6）— ネイティブ経路（`.arc`）の安全動的リンク。Phase V と並行可。

### 0.4 未決事項（§9 に集約）

着手時に確認すべき小さな設計判断のみ。方針転換級の未決はない。

---

## 2. 設計制約 — naïve なバイトコード化を阻む既存機能（**着手前に必読**）

「AST を捨ててバイトコードにする」を**不可能にする**要素。把握していないと途中で詰まる。
（✅=対応済 / 🔶=部分 / ❌=未着手 を各項目に付記）

### 2.1 AST は実行時の第一級の値 — `ast_value.rs` 【✅ AST 保持】
[ast_value.rs](src/interpreter/ast_value.rs) は `parse_ar()` 組み込みの実装で、**AST ノードを `Value::Namespace` ツリーに変換**して
Arrow へ渡す（`__type__` 等）。`python_converter` / converter.ar が依存。
→ **AST は破棄不可。バイトコードは「置き換え」でなく「追加の実行表現」**。Chunk は AST から生成、AST は保持し続ける。

### 2.2 テンプレートは実行時 AST 置換 — `templates.rs` 【✅ #7 で `(テンプレート,型引数)` キーの実体化メモ化・関数/ジェネレータ。クラスは副作用のため対象外】
[templates.rs](src/interpreter/templates.rs) の `subst_stmt`/`subst_expr` が、呼び出し時に型変数を具体型へ置換した
**新 AST を clone-walk で生成**してから実行。
→ **テンプレートは実体化時オンデマンドコンパイル**。`(テンプレート名, 型引数リスト)` をキーに Chunk キャッシュ。解決も実体化ごと。

### 2.3 デバッガが文単位 Span に依存 — `debugger.rs` 【✅ 行テーブル(トレースバック)・デバッグ名テーブル・REPL バイトコード実行】
`exec()` 冒頭で `should_pause_at(stmt)` を Span で判定（[dispatch.rs:19-23](src/interpreter/exec/dispatch.rs#L19)）。
デバッガ REPL は**生スコープに対して式を評価**し `let dbg::name = …` で一時変数宣言。
→ **必須成果物**: ① バイトコードオフセット→Span の**行テーブル**、② slot→変数名の**デバッグ名テーブル**。
   **後付け不可。Chunk の初期設計に入れる**。（V-E で ①②とも実装・REPL は `LoadName`/`compile_debug` で名前引きバイトコード実行。
   VM 関数内のステートメント単位ブレークは残 #1。）

### 2.4 クロージャは定義時に名前でキャプチャ — `capture_env` 【❌ クロージャは現状 bail】
[blocks.rs:39](src/interpreter/exec/blocks.rs#L39) が本体の自由変数を走査、外側から拾って `HashMap<String, CapturedVar>` を作る。
可変変数は `Var::Mutable` → `Var::Cell(Rc<RefCell<Value>>)` に**その場で昇格**して共有。
→ **バイトコード化と相性が良い**。自由変数解析をコンパイル時に前倒しすれば R0-A のフレーム参照になる。`Rc<RefCell>` 表現は流用。

### 2.5 名前が実行時に計算される代入 【✅ 通常は静的解決・デバッガは名前引き】
- `local::{name}`（可変長引数）— [eval/core.rs:34](src/interpreter/eval/core.rs#L34)。ただし `name` は AST 内リテラル＝静的解決可（§3.4-C）
- `Self` / `kwargs` — メソッド/Python 呼び出しで動的に `declare_var`。ただし名前は既知＝slot/型index に解決可
- `import` バインド名 — パース時に一意確定＝静的解決可

→ 大半は静的解決可能（§3.4-C）。**真に動的なのは実行後パースされるコード（デバッガ/対話 REPL）のみ**。そこだけ動的名エスケープハッチ（`LoadName`/`DeclareName`）。

### 2.6 `freeze` が実行時に可変性を変える 【✅ 保守的に扱う】
`make_var_immutable`（[scope.rs:83](src/interpreter/scope.rs#L83)）が `Mutable`→`Immutable` に降格。
→ **「不変だから定数畳み込み」を仮定してはいけない**。freeze 対象になりうる変数は保守的に扱う。

### 2.7 ジェネレータが eager（先行収集）【✅ #8 で本体を VM 化（`Yield` op・eager 収集維持）。lazy 化は §8 非目標のまま】
`exec_generator`（[functions/execution.rs:292](src/interpreter/functions/execution.rs#L292)）は**本体を最後まで実行し全 yield 値を Vec 収集**
してから `Value::Generator` を返す。遅延評価ではない。
→ **Phase V では eager のまま移植**。lazy 化（frame 中断・再開＝新機能）は §8 非目標。

### 2.8 その他（影響小・意味論変更なし）【🔶 FFI/overload は実行時委譲・import VM は残 #10】
- **FFI 経路**（DLL / cpp_bridge / cs-dll / cs-proc / js-proc / PyO3）は全て `eval_call` の先。`CALL_NATIVE` 系に集約されるだけ。
- **オーバーロード解決**は評価済み引数への実行時ディスパッチ（`dispatch_overload`）。当面そのまま実行時。
- **import は実行時にモジュールを実行**して名前空間を作る。モジュール単位 Chunk、初回 import 時コンパイル。

### 2.9 オフセットアクセスの記憶域は既に存在する（重要な追い風）【✅ R3 IC が field_value offset を利用】
インスタンス先頭ポインタからのオフセットアクセスは、**コンパイルと無関係に解釈実行でも既に全インスタンスの実行時表現**:
- `InstanceData { raw: Box<[u64]>, class, boxed_fields }`（[value/instance.rs:129](src/interpreter/value/instance.rs#L129)）。
  slot 0=`[class_id][flags]`、適格クラス（全プリミティブ・trait 継承なし・≤24 フィールド）は C-ABI レイアウトで raw に格納。
- `field_value(idx)` が `byte_offset` で**直接オフセット読み**（[value/instance.rs:213](src/interpreter/value/instance.rs#L213)）。
- ただし **名前→idx の解決だけが毎回辞書引き**（`get_attr_val` → `field_index.get(attr)`, [eval/attrs.rs:70](src/interpreter/eval/attrs.rs#L70)）。
  `Expr::Attr` は解決キャッシュを持たない（[ast.rs:334](src/ast.rs#L334)）。

→ **記憶域は整地済み。Phase R の R3 は「名前→idx の辞書引き」だけを潰せばよい**（詳細 §4.3）。
   設計思想は c-abi-interop（[.claude/skills/c-abi-interop](.claude/skills/c-abi-interop/SKILL.md)）が**ネイティブ境界**に既に採る
   「AST 作成時に解決済みなら直接オフセット、できねば辞書」を、**解釈経路そのものに広げる**もの。

---

## 4. Phase R — AST 解決層 ＋ フレーム/slot ランタイム（**共有基盤・本命**）　【✅ R1/R3/R4 完了・R2/R0-A/codegen 消費は残 #11/#12/#13】

「AST 展開時に、静的に決まる参照を slot / オフセット / 解決済みターゲットへ落とし、決まらない所は `Dynamic` 印」。
解釈経路・ネイティブ codegen の**両方**がこの解決結果を消費する。

### 4.1 成果物の表現
AST を破壊的に書き換えず、**ノードに解決結果を持たせる**（既存 `SlotCache`（[ast.rs:73](src/ast.rs#L73)）と同じ
「ノード埋め込み `Cell` / 付随フィールド」方式。§9-1 で最終確認）。決まらなければ `Dynamic` を保持し実行時に従来経路。

### 4.2 R0 ランタイムモデルの導入（解釈器ストレージ改修）— **Phase R の主作業**
Phase R で解釈経路の速度が上がるには、**解釈器の変数ストレージを `scopes: Vec<HashMap>` から §3.4 の
フレーム/slot モデルへ改修する必要がある**（注釈を付けるだけでは不十分）。この改修が Phase R の実質的な主作業。
- フレームスタック（`Vec<Rc<Frame>>` 相当）を `Interpreter` に導入。`Frame` はサイズ既知のローカル領域。
- `get_var`/`declare_var`/`assign_var`（[scope.rs](src/interpreter/scope.rs)）を slot 索引アクセスに置換。
- ネイティブ codegen はストレージ改修不要（自前 alloca 保持）。**解決注釈だけを消費**。
- この R0 ストレージは **Phase V のバイトコード VM がそのまま再利用**する（フレーム/slot は共通）。
- 現状: `frame_floor` によるスコープ隔離＋ `Scope` の slot 配列化までは実装（明示 `Rc<Frame>` スタックは残 #12）。

### 4.3 解決ステップ R1〜R4（各々独立コミット可）

| ステップ | 内容 | 消す辞書引き |
|---|---|---|
| **R1. ローカル/引数の slot 化** 【✅】 | 関数本体の変数を宣言順に slot 番号付け（B: フレーム内固定 slot）。`Expr::Ident` に `Resolved::Local{frame_level, slot}` を付与。決まらなければ `Dynamic`（§2.5） | scope HashMap 引き（~0.09µs/access） |
| **R2. グローバルの slot 化** 【保留 #11・§6 と連動】 | 既存 `SlotCache` を「実行時遅延」から「AST 展開時解決」へ前倒し。各 .ar ファイルは固有のグローバル配列を持ち index アクセス（§6 のモジュールモデルと接続） | epoch 検証つき実行時キャッシュ |
| **R3. フィールドのオフセット化** 【✅】 | 呼び出し点でオブジェクトの具象クラスが型チェッカから判れば `Expr::Attr` に `(class_id, idx)` を焼く（記憶域は §2.9 の通り既存）。判らなければ **多相 IC**: `InstanceData.class_id`（[value/core.rs:19](src/interpreter/value/core.rs#L19) `alloc_class_id`）で「前回と同じ class_id ならオフセット再利用、違えば `field_index` 引き直してキャッシュ更新」 | `field_index.get(attr)`（[eval/attrs.rs:70](src/interpreter/eval/attrs.rs#L70)） |
| **R4. 呼び先の解決** 【✅】 | Arrow 関数呼び出しを名前引きから解決済みターゲット（グローバル関数 index / 関数ポインタ）へ。`Expr::Call.cache`（[ast.rs:356](src/ast.rs#L356)）を Arrow 関数にも拡張。関数オブジェクトを変数に代入した場合は slot 内 Value の CALL ディスパッチ（名前引きなし） | 呼び先名前引き |

- **protocol 引数・template**: 原型 AST は `templates.rs` が保持（§2.2）。解決可能な呼び出し点は固定オフセットに焼き（monomorphize 相当）、
  真に多相な protocol 引数は R3 の多相 IC に倒す（＝「できねば辞書アクセス」の実体）。

### 4.4 ネイティブ codegen 側の消費 【❌ #13】
`llvm_codegen` の自前再導出（`locals`/`param_classes`/`field_ty`）を Phase R の解決結果に置換し、**codegen を簡素化 + 適用範囲拡大**
（今まで解決できず native 非適格だったケースが解決情報で救える）。

### 4.5 検証 + Phase V ゲート
- **検証**: `cargo test` 672 緑 + `run_examples.ps1` 回帰 + `bench.ps1` 再測定（**R1/R3 で名前引きコストが消えるはず**）
  + `--compile examples/interop/test_modules/physics.ar` が従来と数値一致。
- **中断可能性**: R1〜R4 は各々独立コミット・単体で価値あり。R1（ローカル slot）だけでも支配項の一角に効く。
- **ゲート**: Phase R 完了時に `bench.ps1` を再測定。呼び出し機構(0.53µs)・ノードディスパッチがなお支配的なら Phase V の
  効果（§7 の投影）が裏付けられる。強制バイトコード（D2）が最終目標なので Phase V は実施前提だが、この測定で設計を確認する。

---

## 5. Phase V — バイトコード VM（解釈経路の最終実行形）　【✅ V-A〜V-E・V-F/強制バイトコードは残 #2/#3】

入力は Phase R で解決済みの AST。バイトコード生成は解決ロジックを持たず軽い（起動バジェットは §7.3）。

### 5.1 モジュール構成（`src/vm/`）
> **実装差分**: compiler/ サブ分割（stmt/expr/control）・frame.rs は未分離。単一 `compiler.rs` ＋ 共有バッファ方式（実装メモ参照）。

`src/partial_compiler/` の構成に倣う:
```
src/vm/
  mod.rs          公開 API: compile_fn / compile_debug / run
  op.rs           Op 列挙型（オペコード定義）
  chunk.rs        Chunk { code, consts, names, attr_caches, spans(行テーブル), local_names(デバッグ名テーブル), n_locals }
  compiler.rs     Compiler 本体・関数単位のコンパイル（Phase R の解決注釈を読む）／ compile_debug（デバッガ）
  run.rs          ディスパッチループ本体（exec_op + ハンドラスタック）
  disasm.rs       逆アセンブラ（開発に必須・後回し厳禁）
```
- **`Value` は変更しない**（§7 の Value 表現改善は別テーマ）。値スタックは `Vec<Value>`（共有バッファ `vm_stack`）。
- **行テーブル・デバッグ名テーブルを Chunk の初期設計に入れる**（§2.3, 後付け不可）。

### 5.2 オペコード素案
> **実装差分**: 例外はハンドラスタック（SETUP_TRY/POP_TRY を `run` の Vec が持つ）で exception_table は不使用。
> 純粋組み込みは `CALL_BUILTIN`（print/range/len）。デバッガは `LOAD_NAME`/`DECLARE_NAME`。

```
定数/変数   CONST(idx) NIL TRUE FALSE
            LOAD_LOCAL(slot) STORE_LOCAL(slot)
            LOAD_UPVAL(frame_level, slot) STORE_UPVAL(frame_level, slot)   ← R0-A のフレーム参照
            LOAD_GLOBAL(idx) STORE_GLOBAL(idx)                             ← .ar ファイル固有グローバル配列
            LOAD_DYN(name_idx)          ← §2.5 の動的名前用エスケープハッチ（デバッガ/REPL のみ）
演算        ADD SUB MUL DIV ... CMP_EQ CMP_LT ...   （Int/Float 高速パス + 汎用フォールバック） NEG NOT
ジャンプ    JUMP(off) JUMP_IF_FALSE(off) JUMP_IF_TRUE(off) POP
呼び出し    CALL(argc) CALL_METHOD(ic_slot, argc) CALL_NATIVE(...) RETURN
コレクション BUILD_LIST(n) BUILD_DICT(n) BUILD_TUPLE(n) BUILD_SET(n)
            GET_INDEX SET_INDEX GET_ATTR(class_id, idx | ic) SET_ATTR(class_id, idx | ic)   ← R3
反復        GET_ITER FOR_ITER(off)
例外        SETUP_TRY(handler_off) POP_TRY RAISE RERAISE
式ブロック  BLOCK_RETURN(depth)  LOOP_YIELD
クロージャ  MAKE_CLOSURE(fn_idx, captured_frames)   ← R0-A: 掴むフレームを Rc で保持
```

### 5.3 制御フローのジャンプ化（TLS/センチネル除去がこのフェーズの成果物）
`block_return`/`loop_yield`/`break`/`continue` は**「どのブロックまで抜けるか」をコンパイル時に決定できる**ので、
`ExecResult` 伝播と `LOOP_DEPTH`/`BLOCK_YIELDS` スレッドローカル（§1.4）が**丸ごと消える**。
Arrow 特有の「`break` が入れ子の if/match/block を貫通して外側ループへ届く」規則も、コンパイル時のジャンプ先計算で自然に表現でき、
**実行時センチネル（`RAISE_SENTINEL`/`BREAK_SENTINEL`）が不要**になる。例外はフレームアンワインド + 例外テーブルで表現。
（VM 経路では達成済み。実削除はデュアルモード撤去＝強制バイトコード D2 時 #3。）

### 5.4 段階（V-A 〜 V-F, 各段で 672 緑 + `--vm=force` で穴可視化 + ベンチ再測定）
- **V-A** 【✅】: VM 骨格（op/chunk/frame/run/disasm）+ 算術・ローカル slot・制御フロー・呼び出し・クロージャ（R0-A フレーム）。※クロージャは残。
- **V-B** 【✅】: クラス・メソッド・属性（R3 の解決/多相 IC を `GET_ATTR`/`CALL_METHOD` へ）。
- **V-C** 【✅】: 例外テーブル・match・ブロック式 → **スレッドローカル4本 + センチネル2種を削除**（§1.4 / §5.3）。※VM 経路で不使用化。実削除は D2。
- **V-D** 【✅】: for ループ・組み込み（print/range/len）・Chunk キャッシュ健全化。※テンプレート #7 / ジェネレータ本体 #8 / async #9 / import #10 は残。
- **V-E** 【✅ 実利部分】: デバッガ統合（行テーブル＝トレースバック・デバッグ名テーブル・REPL バイトコード実行）。※汎用行テーブル #1 は残。
- **V-F** 【❌ #2】: 最適化（peephole・superinstruction・単型算術命令）+ **R0-A エスケープ解析**（非エスケープフレームのフラット確保, §3.4-A）。
- **完了時** 【❌ #3】: デュアルモードのフォールバックを撤去し**強制バイトコード**へ（D2）。

---

## 6. モジュール動的リンク仕様（D6 詳細）　【❌ 未実装 #14】

ネイティブ経路（`.arc`）の安全な動的リンク。現状は `try_load_native_module` が **関数ごとに `GetProcAddress`**
（`{fn_name}_tl` シンボル）を引く。これを**モジュールにつき1回**へ改める。`.arc` フォーマットは
`partial_compiler/module_compiler.rs`（`write_tlc_native`, v1）を拡張。

### 6.1 各モジュール DLL がエクスポートするもの
- **単一ディスクリプタシンボル**（例 `__ar_module_descriptor`）— `GetProcAddress` はこれ1回だけ。ディスクリプタは:
  1. **エクスポート表**: グローバル変数/関数/型それぞれの `index → { name, kind, 型 or シグネチャ, 実体ポインタ or グローバル slot }`
  2. **モジュール ABI ハッシュ**: エクスポート表の（名前 + index + シグネチャ + 型レイアウト）の内容ハッシュ（u64/u128）
- 型も同様に**型エクスポート表**（型 index → { 名前, レイアウト/フィールド, kind }）+ 型 ABI ハッシュ。

### 6.2 各 import 側（`.arc`）ヘッダが保持するもの
- 自身のエクスポート表 + 自身の ABI ハッシュ。
- **呼び出す外部モジュールごと**に: モジュール識別子 + **コンパイル時に見た相手の ABI ハッシュ** + 焼いた
  「名前 → 期待 index」対応（照合・再解決用）。
- 呼び出す外部の型についても同様のリスト。

### 6.3 ロード時のリンク手順
```
for each import 辺 (自分 → B):
    desc_B = GetProcAddress(B.dll, "__ar_module_descriptor")   # モジュールにつき1回
    if desc_B.abi_hash == 自ヘッダに焼いた B の abi_hash:        # ハッシュ1個の比較 = O(1)
        安全確定。焼いた index をそのまま使用（B のグローバル/関数/型配列へ index アクセス）
    else:
        フォールバック:
          - 名前で再解決（B のエクスポート表の name→index）+ シグネチャ/型レイアウト照合
          - 一致すれば relink（新 index に差し替え）、不一致なら明示エラー（何が変わったか diff 報告）
```
- **関数オブジェクトを変数へ代入**した場合は index 直解決の限りではないが、**関数自体を関数ポインタで管理（変数として）**
  すれば実質的に名前参照は消える（呼び出しは slot/Value の CALL ディスパッチ）。

### 6.4 コスト概算（一度きりのロード時）
支配項は Arrow の表照合ではなく **OS の `GetProcAddress`**:

| 規模 | 素朴（関数ごと GetProcAddress + 文字列比較） | 本方式（ディスクリプタ1回 + ABI ハッシュ照合） |
|---|---|---|
| 小（数モジュール, ~50 sym） | ~0.1ms | **~10µs** |
| 中（~500 sym, 数十モジュール） | ~1ms | **~50µs** |
| 大（~5000 sym） | ~10ms | **<1ms** |

→ 安全検査つきでもリンクは実用上ほぼ無視できる。バイトコード生成コスト（§7.3）と同オーダー。

---

## 10. 検証コマンド / 規約

```
cargo test                          # 672 passed を維持（各ステップ/各段ごと）
cargo build                         # 警告0 を維持
cargo clippy --all-targets          # exit 0
./run_examples.ps1                  # 例題スイートの回帰確認
./bench.ps1                         # Phase R の各ステップ / Phase V の各段で再測定（フェーズ0基準 = bench_baseline.md）
cargo run -- --compile examples/interop/test_modules/physics.ar  # Phase R: native 経路の数値一致確認
cargo run -- --vm=force <file.ar>   # Phase V 移行中: 未対応構文を可視化（フォールバック禁止）
./generate-codebase-map.ps1         # src/vm/ 等の新設後に必須
```

規約（`.claude/rules/regulations.md`）:
- 新文法の追加はないため example / `_error` example 追加は非該当。
- VS Code 拡張・Python 実装に変更が及ばないため VSIX 再生成・git SHA 同期は非該当。
- 同じスクリプトを繰り返し実行する場合は .ps1 化（`bench.ps1` は作成済み）。

### 参照資料
- [PHASE_R1_RESULTS.md](PHASE_R1_RESULTS.md) — Phase R / Phase V-A〜V-E の実装詳細と実測（進捗の一次資料）。
- [bench_baseline.md](bench_baseline.md) — フェーズ0の全実測値と支配項の切り分け（各ステップの比較基準）。
- [.claude/skills/c-abi-interop](.claude/skills/c-abi-interop/SKILL.md) — オフセットアクセス記憶域（`InstanceData` raw ブロック）の設計仕様。
- `codebase-map` スキル — `src/` のディレクトリ別役割 + ファイル別行数。

<br>

═══════════════════════════════════════════════════════════════════════════════
## 以降は「参照・根拠」（決定済み・履歴）— 実装再開には**読まなくてよい**
§3 アーキテクチャ決定 / §1 背景実測 / §7 速度投影 / §8 非目標 / §9 未決事項。
番号は初版のまま（本文中の相互参照 §3.4 等を保つため物理位置のみ末尾へ移動）。
═══════════════════════════════════════════════════════════════════════════════

## 3. アーキテクチャ決定（確定事項と根拠）

### 3.1 【D1】共有 IR は AST 解決（X 案）。バイトコード→機械語（Y 案）は不採用

**問題**: 解釈経路とネイティブ経路の**両方**が共有する解決結果をどこに置くか。
- **X**: AST に解決表現を注釈（slot/オフセット/解決済みターゲット）。両経路が同じ注釈付き AST を消費。
- **Y**: バイトコードを共有 IR にし、ネイティブ codegen もバイトコードを消費して機械語へ。

**決定的事実**: ネイティブ codegen（`llvm_codegen`）は**既に AST を直接消費し、解決を自前で行っている**:

| codegen が既にやっている解決 | コード |
|---|---|
| ローカル名 → alloca レジスタ | `GenCtx.locals: HashMap<String,(reg,Ty)>` [context.rs:22](src/partial_compiler/llvm_codegen/context.rs#L22) |
| 引数名 → クラス | `param_classes` [context.rs:33](src/partial_compiler/llvm_codegen/context.rs#L33) |
| フィールド → 型付きオフセット(GEP) | `field_ty()` [context.rs:48](src/partial_compiler/llvm_codegen/context.rs#L48) |
| 二項演算を再帰でレジスタへ | `Expr::BinOp{left,right}` 依存（**木構造前提**） [expr.rs](src/partial_compiler/llvm_codegen/expr.rs) |

**X が軽い理由**:
1. codegen は既に AST 消費・解決済み。X は**表現を変えず**事前解決注釈を読むだけ（むしろ簡素化）。Y は入力を総取り替えし動くコードを捨てる。
2. Y はスタックバイトコード→機械語で**デスタック**（push/pop から SSA 復元）が要る。木構造が持つ情報を捨てて拾い直す方向で本質的に重い。
3. X の解決パスは、いまバラバラな4機構（`SlotCache`/`field_index`/`NativeCallCache`/codegen の `locals`）を「AST 展開時に一度」統合したもの。

→ **X 採用。ネイティブ経路は Phase R の解決付き AST を入力に保つ**（Y=バイトコード→機械語は §8 非目標）。

### 3.2 【D2】解釈実行は強制バイトコード（最終状態）。移行中のみデュアルモード

- **最終状態**: 通常のプログラム実行は**常にバイトコード VM**を通る。関数単位のツリーウォークフォールバックは**除去**。
- **`eval()`/`exec()` は残置**（§2.1 `parse_ar` / §2.3 デバッガ REPL / 対話 REPL のツール経路が使う）。「ツリーウォーク廃止」は
  **通常実行経路からの除去**の意味であり、`eval()` 関数自体を消すことではない。
- **移行中（Phase V 実装期間）はデュアルモード**が生命線: 未対応構文はツリーウォークにフォールバックし、`cargo test` を常時緑に保つ。
  ```
  関数を初めて呼ぶとき:
    compile(fn) が Ok(chunk)          → VM 実行、chunk キャッシュ
    compile(fn) が Err(Unsupported)   → 従来 exec_fn_evaled にフォールバック（移行中のみ）
  ```
  `--vm=off` / `--vm=auto`（既定）/ `--vm=force`（未対応で失敗させ穴を可視化）の CLI フラグ。
  V-A〜V-F 完了時に全構文対応が済んだらフォールバックを撤去し強制バイトコードに切替。

### 3.3 【D3】スタックマシン

現在の `eval()` が既にスタック的評価構造なので、移植が「式ごとに push/pop へ書き下す」機械作業になる。レジスタ VM の
上積み（命令数減）は IC や §7 より優先度が低い。将来 superinstruction で一部回収。

### 3.4 【D4】変数解決モデル R0（A / B / C）

Phase R/V の**ランタイム表現の土台**。解釈のフレーム/slot ストレージと、バイトコード VM の両方がこのモデルを使う。

**A. 参照カウント付きフレームのスタック** — フラット1本スタックではなく、**フレーム(活性化レコード)のスタック**。
- 関数に入るたび**フレーム(サイズ既知のローカル領域)を生成**しフレームスタックに push。フレームは `Rc` 管理。
  関数を抜けるとフレームスタックから pop、参照カウント 0 で破棄。
- 関数は「現スコープから見えるフレーム群 + グローバル」への参照を持ち、**(フレーム階層, index)** でローカル/クロージャ変数を解決
  （ハッシュなし・非局所は Rc を1段たどる）。
- **クロージャ**: フレームスタックから pop されても、クロージャが掴むフレームは参照カウントが 0 にならず**生き続ける**。
  → フラット案の「open/closed upvalue 開閉処理」「slot に直値/セル混在」が**不要**になり、クロージャが Rc 寿命管理で自動成立。
  **コード単純化がこのモデルの主目的**。
- **トレードオフ（実測ベース）**: 呼び出しごとにフレームをヒープ確保するため、呼び出しパスの伸びはフラット案 ~4x に対し
  **~2x（0.53µs→~0.25–0.35µs）**。ローカルアクセスの ~30x は不変。
- **後段最適化（V-F）**: `capture_env` の自由変数解析で「クロージャに捕捉されない=エスケープしないフレーム」を判定し、
  **非エスケープフレームだけフラット確保**に落として呼び出し速度を回収。まず一様ヒープフレームで正しさ・単純さを取る。
- 単スレッド `Rc` で可（async は D5 の share-nothing で per-thread フレーム）。

**B. 関数内は固定 slot** — 関数内の if/while/for/block は**実行時に push/pop せず**、全ローカルにフレーム内固定 slot を割り当てる
（フレームサイズ = 同時生存ローカルの最大数、コンパイル時算出）。ブロックスコープはコンパイル時にのみ強制。
→ **関数内の実行時スコープ操作が消える**。Arrow は可視名の再宣言を禁止
（[dispatch.rs:31](src/interpreter/exec/dispatch.rs#L31) 付近の "already declared" 検査）＝**シャドウイングなし**なので slot 割り当てが素直。
ブロックを跨ぐ同名の slot 再利用/寿命解析のみ要実装。

**C. 動的名はほぼ静的解決可** — `self`/可変長 `local::`/`kwargs`/import バインド名/`Self`/`static` は
**識別子が AST 内リテラルのため slot/型index へ静的解決可能**。「通常実行の変数アクセスから実行時ハッシュは消える」。
**残余は実行後にパースされるコードのみ**:
- **デバッガ REPL**: 生フレームへの任意式 → **slot→名前 デバッグメタデータ**で解決（ホットパス外, §2.3）。
- **対話 REPL**: 各行を逐次コンパイル → **リゾルバがインクリメンタルに global slot を採番**（実行時ハッシュ不要）。
- テンプレートは実体化ごとに解決（名前は既知＝静的解決の範疇, §2.2）。
- 補足: 属性/メソッドの**多相**ディスパッチは slot ではなく `class_id` インラインキャッシュ（§4.3 R3）。動的名とは別機構。

### 3.5 【D5】async は share-nothing 維持 【✅ #9 で関数内 async を VM 化（capture を frame から再構成し `capture_env` 再利用）。share-nothing 不変】
現行 async（`mng <- async->T: body`）は**投入時 deep-clone = 共有可変状態なし**。これを維持。
- 過剰 deepcopy 削減: **読み取りのみキャプチャは不変 Arc 共有** / **書込み要は COW** / **投入後に main が使わない変数は move**。
- 「async が main の mut 変数を編集」したい場合は**明示的 Mutex 系プリミティブ**（CLAUDE.md 記載の将来機能・別テーマ）。
- **フレーム暗黙共有による mut 編集は不採用**（`Arc<Mutex>` 化 → ロック → データ競合 → R0-A の `Rc` 前提が壊れる）。

### 3.6 【D6】モジュール動的リンク = ディスクリプタシンボル + ABI ハッシュ
ネイティブ経路（`.arc`）の安全な動的リンク方式。**詳細仕様は §6**。要点:
- 各モジュール DLL は**単一のディスクリプタシンボル**をエクスポート（`GetProcAddress` はモジュールにつき1回）。
  ディスクリプタはエクスポート表（グローバル/関数/型の index↔名前↔シグネチャ↔ポインタ）+ **モジュール ABI ハッシュ**を指す。
- import 側は「コンパイル時に見た相手モジュールの ABI ハッシュ + 焼いた index」を自ヘッダに保持。
- ロード時: import 辺ごとにディスクリプタを1回引き、**ABI ハッシュ1個を比較**（O(モジュール数)）。一致=安全確定、
  不一致=名前で再解決するか明示エラー。
- **支配項は OS の `GetProcAddress`（~1–5µs/sym）**なので「関数ごとに引かない」が肝。概算リンク時間: 中規模 ~50µs / 大規模 <1ms。

---

## 1. 背景 — 現在の実行モデルと実測ベースライン（実コード確認済み）

### 1.1 実行パイプライン

```
source → lexer → parser → type_check → Interpreter::exec(&Stmt) / eval(&Expr) を再帰呼び出し
```

- `exec()` = [exec/dispatch.rs:16](src/interpreter/exec/dispatch.rs#L16) の巨大 match。`Stmt` バリアントごとに専用メソッドへ委譲。
- `eval()` = [eval/core.rs:16](src/interpreter/eval/core.rs#L16) の match。同様。
- 関数呼び出しは **Rust の再帰**。`exec_fn_evaled`（[functions/execution.rs:31](src/interpreter/functions/execution.rs#L31)）が
  スコープ退避 → 新スコープ構築 → `exec_block` → 復元。

### 1.2 変数アクセスのコスト構造

```rust
type ScopeMap = HashMap<String, Var, FxBuildHasher>;   // interpreter.rs:187
scopes: Vec<ScopeMap>                                   // 0=グローバル, 末尾=最内
```

- `get_var` は**末尾→先頭へ線形にスコープを遡り、各段で文字列ハッシュ**（[scope.rs:30](src/interpreter/scope.rs#L30)）。
- `get_val` は `Var::get_value()` = **`v.clone()`**（[interpreter.rs:219](src/interpreter.rs#L219)）。
  → **変数を1回読むたびに `Value` が1つクローンされる**。`Value::Str(String)` なら毎回ヒープ確保。

### 1.3 既存の場当たり最適化（Phase R/V で不要になる = 置換・撤去対象）

| 既存最適化 | 場所 | 目的 | 置換後 |
|---|---|---|---|
| `FxHasher` | interpreter.rs:150 | 変数名ハッシュ高速化 | **撤去**（名前引きが消える） |
| `SlotCache`（epoch 付き） | [ast.rs:73](src/ast.rs#L73) | グローバル代入のスコープ検索回避 | **撤去**（グローバル索引化, R2） |
| `global_slot_cells` / `slot_epoch` | interpreter.rs:268 | 同上 + `freeze` 時の一括無効化 | **撤去** |
| `Expr::Call { cache }` | [ast.rs:356](src/ast.rs#L356) | 呼び先解決キャッシュ | R4 の解決 + 多相 IC へ発展 |

> `SlotCache` を AST に埋め込んでいる時点で、既に「AST を可変な実行表現として使う」方向にある。Phase R/V はその延長線上。

### 1.4 制御フローの3系統（Phase V で構造的に消える）

1. `ExecResult` enum の戻り値伝播 — `Normal`/`Break`/`Continue`/`Return`/`BlockReturn`/`BlockYield`/`Raise`
2. **スレッドローカル 4本**（[interpreter.rs:117-136](src/interpreter.rs#L117)）
   - `LOOP_DEPTH`（break/continue 妥当性）/ `GENERATOR_YIELDS`（yield 収集）/
     `BLOCK_YIELDS`（loop_yield 収集）/ `BLOCK_RETURN_EXPECTED_TYPE`（block_return 実行時型検査; 式ごと push/pop）
3. **文字列センチネル** — `RAISE_SENTINEL` / `BREAK_SENTINEL` を `Result<Value,String>` の `Err` に載せ、
   実体は `self.current_exception` に置く（`eval()` が `RaisedError` を返せない回避策）

→ Phase V ではジャンプ命令 + 例外テーブルに解消。**速度以前に設計上の大整理**。（VM 経路では不使用化済み・実削除は D2。）

### 1.5 フェーズ0 実測（支配項の切り分け・ゲート）【完了 2026-07-21, 詳細 [bench_baseline.md](bench_baseline.md)】

`examples/bench/bottleneck_bench.ar`（要因分離, N=100万）+ `bench_field_access.ar`（E2E）, release, 各3回・安定。

| 要因 | コスト | 誰が潰すか |
|---|---|---|
| **関数呼び出し機構**（scope HashMap 構築 + ExecResult 伝播） | **0.53 µs/call** | Phase V（フレーム）+ R0 |
| **引数束縛**（1個: eval + HashMap insert + clone） | **~0.7 µs/arg** | Phase V + R |
| **ローカル変数アクセス**（HashMap 引き・**SlotCache 無し**） | **~0.09 µs/access** | Phase R（slot 索引） |
| AST ノードディスパッチ（enum match + `Box<Expr>` 追跡） | baseline 0.13µs に内包 | Phase V（線形走査） |
| Value deep_clone（複合値） | ~0.13 µs | §7 のみ（バイトコードでは不変） |
| フィールド読み | ~0.15 µs | Phase R（offset/IC） |

- **決定的な発見**: グローバルは `SlotCache` で索引化済みだが、**関数内ローカルは毎回 HashMap 引き**。slot 化の伸びしろ最大。
- **§7（`Value::Str(Rc<str>)` 等）は数値コードでは支配項でない**（deep_clone ~0.13µs）。文字列多用時のみ効く二次テーマ。
  ※ 本ベンチは int/float 中心で String クローンを踏んでいない点に留意。

---

## 7. 速度・コスト投影

### 7.1 定性的効果
| 項目 | 効果 |
|---|---|
| 変数アクセス | ハッシュ + スコープ遡り → **配列インデックス1回** |
| 制御フロー | ExecResult 伝播 + TLS borrow → **ジャンプ1命令** |
| 深い再帰 | Rust スタック依存 → **明示フレームスタック**（スタックオーバーフローが消える） |
| コードの整理 | センチネル文字列2種・スレッドローカル4本が**構造的に不要になる** |
| 将来性 | lazy generator、末尾呼び出し、プロファイル駆動最適化への**足場** |

### 7.2 定量投影（フェーズ0実測からの積み上げ・実測ではない）
効くのは**ディスパッチと名前解決**のみ。**Value 操作・FFI・実演算には効かない**。

| 成分 | ツリーウォーク(実測) | バイトコード+R(投影) | 倍率 | 担当 |
|---|---|---|---|---|
| ローカル変数読み | 0.09 µs (HashMap) | ~0.003 µs (`stack[base+slot]`) | ~30x | Phase R |
| フィールド読み | 0.15 µs (HashMap+offset) | ~0.03 µs (offset) | ~5x | Phase R |
| ノードディスパッチ | baseline 0.13µs に内包 | 線形走査 | ~2–3x | Phase V |
| 関数呼び出し機構 | 0.53 µs | ~0.25 µs（R0-A ヒープフレーム; フラットなら ~0.12） | ~2x（フラット ~4x） | Phase V + R0 |
| 引数束縛 | ~0.7 µs/arg | ~0.15 µs/arg | ~4–5x | Phase V+R |
| 制御フロー | ExecResult+TLS | ジャンプ | ~2–5x | Phase V |
| **Value clone/コピー** | 0.13 µs | 0.13 µs | **1x** | §7.4 のみ |
| **FFI/ネイティブ** | 固定 | 固定 | **1x** | `--compile` |

- **実測ベンチ適用**: 空 while 0.13→~0.04µs(~3x) / 引数なし呼び出し 0.53→~0.20–0.25µs(R0-A) / E2E `kinetic()` 3.13→~1.0–1.2µs(~3x)。
- **ワークロード別**: 呼び出し/ローカル/制御フロー支配 = **3〜5x**（R0-A のフレーム確保ぶん上限やや低下）/ 典型混在 = **2〜3x** /
  clone・コレクション・文字列支配 = **1.3〜2x** / FFI 支配 = **~1x**。
- **Phase R と V の切り分け**: Phase R 単独（注釈付きツリーウォーク + R0 ストレージ）で **~1.3–2x**、Phase V 上乗せで **さらに ~1.5–2.5x**。
- **上限を抑える要因**: `Value` は約80バイト（`JsProcFn` が String×3=72B 内包）。スタック push/move/clone で毎回 ~80B コピー。
  §7.4 の一部（大 variant の Box 化）を並行すると呼び出し/引数束縛の高速化上限が上がる。（§7.4-2 は実施済み・72→32B。）

### 7.3 起動コストのバジェット（強制バイトコード＝全プログラムが生成を通る）
`scratchpad/gen_*.ar`（規模違いのパース支配プログラム）で実測代理:
- **パース+型検査 = ~2.9 µs/行**（250/1000/4000 関数で線形・一定）。
- **バイトコード生成 = 0.3〜1.0 µs/行**（AST 1walk + emit。パース/型検査より単純ゆえその一部が上界）。

| 規模 | 一括生成コスト | 判断 |
|---|---|---|
| 100 行 | 0.03〜0.1 ms | プロセス起動 ~8.5ms に埋もれる＝無視可 |
| 700 行（spider 全体） | 0.2〜0.7 ms | 体感不能 |
| 5,000 行 | 1.5〜5 ms | 許容 |
| 24,000 行 | 7〜24 ms | この規模はパース+型検査で既に ~70ms |

**設計指針**: 強制でも**関数単位の遅延生成（初回呼び出し時コンパイル + キャッシュ）**を採る。デッドコードの生成費用を払わず起動が
規模フラット。ホット関数は初回 ~数µs のみ（呼び出し1回 0.53µs＝生成 ~3µs は ~6回で回収）。モジュール本体は一括生成でよい。

### 7.4 併走を検討すべき別テーマ（本計画と独立）
1. **`Value` クローンコスト削減** — `Value::Str(String)` → `Value::Str(Rc<str>)`。変数を読むたび String ヒープ確保（§1.2）。【❌ #15】
2. **`Value` サイズ削減** — `JsProcFn`（String×3=72B）等の大 variant を `Box` 化して `size_of::<Value>()` を縮小。スタック操作が軽くなる。【✅ 72→32B】
3. **文字列インターン** — 属性名・メソッド名を `Rc<str>` + ポインタ比較。【❌ #15】

---

## 8. 非目標（今回やらないこと）

- **ジェネレータの lazy 化**（意味論変更・新機能。別提案として切り出す, §2.7）
- **`Value` 表現の変更**（§7.4 として独立管理。Phase R/V は `Value` を変更しない）
- **`eval()` / `exec()` の削除**（`parse_ar` / デバッガ REPL / 対話 REPL が使い続ける。D2 の「ツリーウォーク廃止」は通常実行経路からの除去の意）
- **バイトコード → 機械語（§3.1 Y 案）**。ネイティブ経路は Phase R の解決付き AST を入力に保つ
- **async の share-mutable 化**（D5）。既定は share-nothing 維持、共有可変は明示プリミティブ、フレーム暗黙共有は不採用
- **`impl_python/` の追従**（Rust 側のみ。規約の git SHA 同期は非該当）
- **JIT**（バイトコード VM 安定後の話）

---

## 9. 残る未決事項（着手時に確認・小さな判断のみ）

1. **解決注釈の持ち方**: AST 埋め込み（`Cell`/付随フィールド, `SlotCache` 流儀）か 別テーブル（node-id キー）か。
   前者はノード局所で速いが AST 型が肥大、後者は AST を汚さないが間接引き。→ **前者推奨**（既存流儀）。
2. **R3 の型チェッカ連携**: 呼び出し点でオブジェクトの具象クラスを渡せる API が `type_check/` にあるか。着手時に推論結果の受け渡し方を確認。
3. **R0-A フレームの内部表現**: `Rc<RefCell<Vec<Value>>>` か `Rc<Frame>`（`Frame` に inline 配列 + 借用管理）か。RefCell borrow の
   パニック表面とコストを見て決定。まず素直な `Rc<RefCell<...>>` で正しさ優先、V-F で最適化。
4. **ブロック跨ぎ同名の slot 再利用**（B）: ブロックを抜けた後の同名変数の slot 寿命解析の正確な規則。Arrow のブロックスコープ意味論を
   リゾルバ構築時に確認（可視名再宣言禁止＝シャドウなしは追い風）。※現状は「既出名はスキップ＝slot 再利用」で実装済み。
5. **循環 import のリンク**（§6）: A⇄B の相互参照は「全シンボル宣言（index 採番）→本体解決」の2フェーズで解く。
