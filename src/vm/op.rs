// vm/op.rs — バイトコード VM のオペコード定義（Phase V, V-A）。
//
// スタックマシン。命令は `Vec<Op>`（chunk.code）に線形に並び、`vm/run.rs` の
// ディスパッチループが `ip`（命令ポインタ）を進めながら実行する。
// ジャンプ先は絶対 index（code 配列内の位置）で持つ（コンパイル時にバックパッチ）。

use crate::ast::{BinOp, UnaryOp};

/// VM オペコード（V-A の最小セット）。
#[derive(Debug, Clone)]
pub enum Op {
    /// consts[idx] を push する。
    Const(u32),
    /// `None` を push する。
    Nil,
    /// locals[slot] を push する（Resolution::Local / パラメータ / base ローカル読み）。
    LoadLocal(u16),
    /// グローバル変数/関数の読み出し（呼び先の解決・#11 索引化）。未定義なら NameError。フィールド (name_idx, cache_idx)。
    /// `chunk.global_caches[cache_idx]` に `(slot_epoch, scopes[0] index)` を焼き、以後は名前ハッシュ
    /// 引きを飛ばして `scopes[0].slot(idx)` へ直接アクセスする（`freeze` で epoch が進めば自動再解決）。
    LoadGlobal(u32, u32),
    /// pop して**既存のグローバル変数**へ代入する（#10-b: 最上位 Chunk の `x = e` / `x <op>= e`）。
    /// フィールド (name_idx, cache_idx)。
    ///
    /// ツリーウォークの `Stmt::Assign` と**同じ機構**を使う: 初回は `assign_var`（可変性検査・
    /// `Var::Cell`/`SlotCell` の扱い・`NameError`）を通し、`try_fill_slot` が対象を
    /// `Var::SlotCell` へ昇格して `global_slot_cells` の index を焼く。2 回目以降は
    /// そのセルへ直接書き込む（`freeze` が `slot_epoch` を進めれば自動失効）。
    ///
    /// ⚠ **`chunk.global_caches` の index 空間は op ごとに意味が違う**。`LoadGlobal` は
    /// `scopes[0]` の slot 番号を、`StoreGlobal` は `global_slot_cells` の index を焼く。
    /// 1 本のキャッシュ枠は必ず 1 つの op 実体だけが読み書きするので混ざらないが、
    /// **枠を共有・再利用する最適化を足すときはここで壊れる**。
    StoreGlobal(u32, u32),
    /// pop して locals[slot] へそのまま書き込む（const / 代入 / let-from-immutable / リテラル let）。
    StoreLocal(u16),
    /// pop して deep_copy してから locals[slot] へ（`mut` 宣言: exec は常に deep_copy_value）。
    StoreLocalDeepCopy(u16),
    /// pop して deep_copy + freeze してから locals[slot] へ（`let` = mut ソースからの束縛）。
    StoreLocalCopyFreeze(u16),
    /// pop し、Instance のときのみ deep_copy + freeze してから locals[slot] へ
    /// （`let` = 非識別子式からの束縛。exec_let の非 ident 分岐に一致）。
    StoreLocalFreezeInstance(u16),
    /// pop して `let x = <識別子>` のコピー意味論を**実行時に**決めてから locals[slot] へ。
    /// フィールド (slot, ソース名 name_idx)。
    ///
    /// `exec_let` はソース変数の可変性で 3 分岐する（mut→deep_copy+freeze / let→そのまま /
    /// 変数でない→Instance だけ copy+freeze）。ソースが**グローバル**のときその可変性は
    /// コンパイル時に分からないので、`DeclKind::LetFromIdent` と同じ判断を実行時に行う（#27-c）。
    ///
    /// ⚠ **ソースの可変性は `scopes[0]` だけを見る**。この op が出るのは
    /// 「コンパイラが `slots` を引いて外れた＝ローカルではないと確定した」名前に限るので、
    /// グローバルを見るのが正しい（`get_var` だと VM フレームで呼び出し元のローカルが見える）。
    StoreLocalFromIdent(u16, u32),
    /// キーワード引数つきの組み込み呼び出し（#27-c）。フィールドは `chunk.kw_calls` の index。
    ///
    /// `CallBuiltin` は評価済みの値だけを渡すので `enumerate(xs, start=1)` のような形を表現できず、
    /// コンパイラが bail していた。ここでは `kw_calls[i].arg_names` を一緒に運び、
    /// `eval_builtin_evaled_named` がツリーウォークと同じ引数解釈を行う。
    ///
    /// ⚠ **発行できるのは `VM_BUILTIN_KW_NAMES` の組み込みだけ**。
    /// キーワードの解釈はツリーウォークの各アームごとに違う（無視する／エラーにする）ので、
    /// 一致を確認した名前以外は従来どおり bail してツリーウォークへ落とす。
    CallBuiltinKw(u32),
    /// キーワード／可変長引数つきのメソッド呼び出し（#27-c）。フィールドは `chunk.kw_calls` の index。
    ///
    /// `CallMethod` は引数名を持てないので `f.read_line(backward = True)` の形が bail していた。
    /// スタックは `CallMethod` と同じ `[recv, arg0..argN-1]`。引数名を添えて
    /// ツリーウォークと同じ dispatcher（`call_instance_method_evaled` / `vm_method_call_other`）へ渡す。
    /// ⚠ `kw_calls[i].span_idx` は**使わない**（メソッドは call_span=None でツリーウォークに揃える）。
    CallMethodKw(u32),
    /// `static mut x = e` の初期化ガード（#27-d）。フィールド (span_idx, 初期化子の直後の ip)。
    ///
    /// `static` の記憶域は**フレームではなく `Interpreter::static_cells`**（宣言位置＝span をキーに
    /// した `Rc<RefCell<Value>>`）。`exec_static_var` と同じく、**セルが既にあれば初期化子を
    /// 評価しない**ので、あるときは初期化子を飛び越えてジャンプする。
    StaticInit(u32, u32),
    /// `static` セルを**新規作成**して pop した値を入れる（`StaticInit` が素通りしたときだけ到達）。
    StaticStore(u32),
    /// `static` 変数の読み出し（セルの中身を clone して push）。フィールドは span_idx。
    LoadStatic(u32),
    /// `static` 変数への書き込み（pop してセルへ）。フィールドは span_idx。
    StoreStatic(u32),
    /// **セル変数**の読み出し（#27-d 段階 2b）。フィールドはフレームのセル表の index。
    ///
    /// セル変数＝`Rc<RefCell<Value>>` を**外側フレームやクロージャと共有する**ローカル。
    /// slot（`Value` 直値）では共有を表現できないので、slot と並行するセル表に置く。
    /// 該当するのは 2 つ:
    /// - クロージャが**可変キャプチャ**している名前（`CapturedVar::Mutable` の相手）
    /// - 入れ子 `fn` に可変キャプチャされる自分のローカル（生成時に `MakeFn` へ渡す）
    LoadCell(u16),
    /// セル変数への書き込み（pop してセルの中身を置き換える）。共有相手からも見える。
    StoreCell(u16),
    /// `mut x = e` でセル変数を初期化する（pop → deep_copy → セルへ）。
    /// ツリーウォークの `Stmt::Mut` が常に `deep_copy_value` するのに合わせる。
    StoreCellDeepCopy(u16),
    /// スタックトップを1つ捨てる。
    Pop,
    /// 二項演算: pop b, pop a, push apply_binop_dyn(op, a, b)。
    Bin(BinOp),
    /// 超命令（タスク #2）: `local[a] <op> local[b]` を融合。`LoadLocal(a); LoadLocal(b); Bin(op)` と
    /// 同一意味論（apply_bin_fast 委譲）でディスパッチ2回分＋スタック push/pop を削減する。
    BinLocalLocal(u16, u16, BinOp),
    /// 超命令: `local[a] <op> consts[idx]` を融合。`LoadLocal(a); Const(idx); Bin(op)` と同一。
    BinLocalConst(u16, u32, BinOp),
    // ── 型特化二項演算（#16 段階(b)/plan A）──
    // 型検査が両オペランド int/float と確定した箇所で emit。オペランドを **clone せず参照で読み**、
    // op ディスパッチと汎用フォールバックを畳んだ直接算術を行う（意味論は apply_bin_fast と同一）。
    // 対応 op は種別ごとに違い、判定は compiler.rs の `gate_bin_kind` が一元管理する
    // （int は Int/Int アーム全部、float は `//`・`%`・ビット演算を除く）。
    // 想定外の実行時型・ゼロ除算は保守的に汎用へ委譲する。
    //
    // emit 元は `Expr::BinOp` と、同じ演算である `Stmt::CompoundAssign` / `Stmt::AttrCompoundAssign`（#2b）。
    /// `local[a] <op> local[b]`（両 int 確定）。
    IntBinLL(u16, u16, BinOp),
    /// `local[a] <op> consts[idx]`（両 int 確定）。
    IntBinLC(u16, u32, BinOp),
    /// `local[a] <op> local[b]`（両 float 確定）。
    FloatBinLL(u16, u16, BinOp),
    /// `local[a] <op> consts[idx]`（両 float 確定）。
    FloatBinLC(u16, u32, BinOp),
    /// スタック上の2値（両 int 確定）: pop b, pop a, push a <op> b。
    /// `LL`/`LC` と違いオペランドの形を問わないので、属性・添字・呼び出し結果など
    /// **局所変数以外を含む任意の式**に型特化が乗る（#16 段階(b)(iii)）。
    IntBinSS(BinOp),
    /// スタック上の2値（両 float 確定）。`IntBinSS` の float 版。
    FloatBinSS(BinOp),
    /// 単項演算: pop a, push apply_unary_dyn(op, a)。
    Un(UnaryOp),
    /// 属性（フィールド）読み: pop obj, push get_attr_val(obj, names[name_idx], attr_caches[cache_idx])。
    GetAttr(u32, u32),
    /// 属性（フィールド）書き: スタックは `[obj, value]`。value・obj を pop し、
    /// `attr_assign_evaled(obj, names[name_idx], value)` で代入する（値は push しない）。
    /// obj が Instance であることはコンパイル時に保証済み（`self` または instance 型注釈）。
    SetAttr(u32),
    /// スタックトップ2つを入れ替える（複合属性代入で rhs を先に評価しつつ演算順を保つため）。
    Swap,
    /// 型判定: pop v, push Bool(value_is_type(v, names[name_idx]))（match の `is TypeName` パターン）。
    /// 超命令（#16 段階(b)(i)）: `local[slot].method(args)` を融合。
    /// `LoadLocal(slot); ...args...; CallMethod(..)` と同一意味論だが、レシーバをスタックへ
    /// 積まず frame から直接取るので op ディスパッチ 1 回と push/pop 1 組が消える。
    /// 引数は `(slot, name_idx, argc, mut_mask)`。
    ///
    /// **評価順について**: 融合前はレシーバ→引数の順に評価していたが、この op は引数評価後に
    /// frame を読む。VM がコンパイルするコードでは**式の評価中に自フレームの slot が再束縛されることはない**
    /// （再束縛は文＝`StoreLocal` でしか起きず、クロージャによる捕捉は VM 非対応で bail する）ため、
    /// 観測されるレシーバは同一。
    /// (slot, name_idx, argc, mut_mask, node_id)。`node_id` は FFI 境界検査のキー（#27-b）。
    CallMethodLocal(u16, u32, u16, u32, u32),
    /// 超命令（#16 段階(b)(i)）: `local[slot].attr` を融合。`LoadLocal(slot); GetAttr(..)` と同一意味論。
    /// **レシーバをスタックへ clone せず frame から参照で読む**ので `Rc` の refcount 増減が消える。
    /// 引数は `(slot, name_idx, cache_idx)`。
    GetAttrLocal(u16, u32, u32),
    IsType(u32),
    // ── 動的型検査（#16 段階(b)(ii)・`CheckBefore` 指示の消費）──
    // 型検査が「使用前に動的検査が要る」と印を付けたノードに対応する op。
    // これらが無い間は `mustbe`/`=>` を含む関数が**丸ごとツリーウォークへ bail** していた。
    /// `expr mustbe T`: pop v。`names[type_idx]` の外側型に一致すれば push し直し、
    /// 不一致なら `spans[span_idx]` 付きの TypeError を送出（`eval` の MustBe と同一）。
    MustBe(u32, u32),
    /// `expr => T`: pop v, push eval_cast_evaled(v, names[type_idx])。
    Cast(u32),
    /// 純粋・共通な組み込み呼び出し（print/range/len）: argc 個の評価済み引数を pop し、
    /// `eval_builtin_evaled(names[name_idx], args)` で実行して結果を push する。
    CallBuiltin(u32, u16),
    /// イテレータ取得: pop iterable, push make_for_iterator(iterable)（`for` ループの入口）。
    GetIter,
    /// イテレータ前進: `locals[iter_slot]` の `.next()` を呼ぶ。
    /// `EndOfIteration` なら `exit_ip` へジャンプ、要素があれば `locals[target_slot]` へ束縛して継続。
    /// フィールドは (iter_slot, target_slot, exit_ip)。
    ForIter(u16, u16, u32),
    /// 無条件ジャンプ（絶対 index）。
    Jump(u32),
    /// pop した値が偽ならジャンプ（if/while の条件分岐）。
    JumpIfFalse(u32),
    /// スタックトップが偽ならジャンプ（値を残す）、真なら pop して継続（`and` 短絡）。
    JumpIfFalseOrPop(u32),
    /// スタックトップが真ならジャンプ（値を残す）、偽なら pop して継続（`or` 短絡）。
    JumpIfTrueOrPop(u32),
    /// 関数呼び出し: スタックは `[callee, arg0, .., argN-1]`。args を argc 個・callee を pop し、
    /// `mut_mask`（bit i = arg i の is_mutable）付きでディスパッチして結果を push する。
    /// フィールド: (argc, mut_mask, name_idx, span_idx, node_id)。name_idx=呼び出し元名（トレースバック用・
    /// `names`）、span_idx=呼び出し位置（`spans`）、node_id=AST 型解決層の注釈キー（#22-b）。
    ///
    /// `node_id` は **FFI 境界検査（#16）が宣言型を引くために要る**。以前は運んでいなかったため、
    /// 値経由で py/js 関数を呼ぶと VM 経路だけ検査が素通りしていた（#22-a 発見 2）。0 = 未採番。
    Call(u16, u32, u32, u32, u32),
    /// インスタンスメソッド呼び出し: スタックは `[obj, arg0, .., argN-1]`。args を argc 個・obj を pop し、
    /// `names[name_idx]` のメソッドを `mut_mask` 付きでディスパッチして結果を push する。
    /// obj が Instance であることはコンパイル時の型注釈で保証済み。フィールド: (name_idx, argc, mut_mask)。
    /// メソッド呼び出しはツリーウォークが呼び出し位置 span を渡さない（フレームが degraded）ため、
    /// VM も call_span=None で一致させる（byte-identical）。
    /// (name_idx, argc, mut_mask, node_id)。`node_id` は FFI 境界検査のキー（#27-b）。
    /// これが無いと `mod.func()` 等の戻り値検査が **VM 経路だけ素通り**する（#22-a と同型の穴）。
    CallMethod(u32, u16, u32, u32),
    /// スタックトップを関数戻り値として返す。
    Return,
    /// `None` を関数戻り値として返す（本体末尾のフォールオフ）。
    ReturnNil,
    // ── 例外処理（Phase V-C） ──
    /// try 本体の入口: 例外ハンドラ（handler_ip）と現在のオペランドスタック深さを
    /// VM のハンドラスタックに push する。
    SetupTry(u32),
    /// try 本体が正常終了したときにハンドラを1つ pop する。
    PopTry,
    /// `raise expr`: pop した例外値に `spans[idx]` を焼き、current_exception を設定して伝播する。
    Raise(u32),
    /// bare `raise`（再送出）／どの except にもマッチしなかったときの再伝播。
    /// current_exception を用いて伝播する（スタックには触れない）。
    Reraise,
    /// スタックトップを複製する（except 節の型照合で例外値を残すため）。
    Dup,
    /// pop した例外値が `names[name_idx]` 型にマッチするか（`exc_matches`）を Bool で push する。
    ExcMatch(u32),
    /// `names[idx]` を内部エラー文字列として返して停止する（ツリーウォークの `Err(msg)` と同じ経路・#34）。
    ///
    /// 用途は「**実行時に必ず失敗すると分かっている文**」を、コンパイル失敗（`VmForceError`）ではなく
    /// **ツリーウォークと同じメッセージ**で落とすこと。現在の発行元は囲むループの無い
    /// `break`/`continue` だけ。⚠ **飛び先索引を持たない**ので `peephole::code_target_mut` は不要。
    Fail(u32),
    // ── ブロック式（Phase V-C） ──
    /// 空の `Value::List` を push する（loop_yield の蓄積先の初期化）。
    BuildEmptyList,
    /// pop した値を `locals[slot]` のリスト（accumulator）へ追加する（`loop_yield`）。
    ListAppendLocal(u16),
    /// pop したリストが空なら `None`、非空ならそのリストを push する
    /// （for/while/block 式の値: 蓄積が空なら None）。
    ListOrNone,
    // ── デバッガ REPL（V-E。停止スコープの生変数へ名前でアクセス） ──
    /// 現在のスコープから `names[name_idx]` の値を名前引きで push する（`get_val`）。未定義なら NameError。
    LoadName(u32),
    /// pop した値を `let dbg::name` として現在のスコープへ宣言する（不変・`let` 意味論）。
    DeclareName(u32),
    /// 入れ子 `fn` 定義（#27）。`chunk.fn_defs[idx]` から関数値を作り slot へ格納する。
    ///
    /// ⚠ **キャプチャは不変な外側ローカルに限る**（コンパイラが保証）。可変キャプチャは
    /// 外側ローカルとのセル共有が要るが、VM のフラット slot は `Value` 直値なので表現できない。
    MakeFn(u32),
    /// `for k, v in ...` のタプル分解（#27-c）。`locals[slot]` のタプルを検査して
    /// **要素を順に push** する（呼び出し側は `StoreLocal` を逆順に並べて受ける）。
    /// フィールドは (src_slot, 要素数)。
    ///
    /// 検査もエラー文言もツリーウォーク（`exec_for_stmt` の複数ターゲット分岐）と同一:
    /// タプルでなければ `TypeError`、要素数が合わなければ `ValueError`。
    UnpackTuple(u16, u16),
    /// `let a, b = t`（#27-c）。pop した値を `tuple_decls[idx]` に従って分解束縛する。
    ///
    /// 束縛先が slot かグローバル宣言かは `TupleDecl::slots` が持つ。検査・エラー文言・
    /// `let` の freeze / `mut` の deep_copy はツリーウォークと同じ `let_tuple_values`。
    LetTuple(u32),
    /// `freeze x`（#27-c）。フィールドは (name_idx, span_idx)。
    ///
    /// 値はスタックに載せない。`exec_freeze` をそのまま呼ぶ（`__freeze__` の呼び出しや
    /// クロージャセル検査を含む意味論はツリーウォークと同一の 1 実装）。
    FreezeVar(u32, u32),
    /// `src on/once handler`（#27-c）。スタックは `[source, handler]`。
    /// フィールドは (is_once, is_async)。
    EventSubscribe(bool, bool),
    /// `src off handler`（#27-c）。スタックは `[source, handler]`。
    EventUnsubscribe,
    /// キーワード/可変長引数つき呼び出し（#27-c）。スタックは `Op::Call` と同じ
    /// `[callee, arg0..argN-1]` で、引数名だけ `chunk.kw_calls[idx]` が持つ。
    ///
    /// 可変長 `f(... = A, B, C)` は**コンパイラが `BuildList` で 1 値に畳む**ので、
    /// スタック上の引数は常に 1 引数 1 値。`eval_call_args` が作る
    /// `(Some("..."), Value::List, true)` と同じ形になる。
    CallKw(u32),
    /// テンプレート呼び出し `Tmpl[T](args)`（#27-c）。スタックは `[tmpl, arg0..argN-1]`。
    /// フィールドは (type_arg_lists の index, 引数の数, mut マスク)。
    ///
    /// ツリーウォークの `eval_call` の `TemplateInstantiate` 分岐と同じく
    /// `instantiate_template` の本体（`instantiate_template_args`）を通る。
    /// **位置引数のみ**（キーワード引数はコンパイラが bail する。`Call` と同じ制限）。
    CallTemplate(u32, u16, u32),
    /// スライス式 `a[b:e:s]` の `b:e:s` 部分（#27-c）。スタックは `[begin, end, step]`。
    /// 3 つを pop して `Value::Slice` を push する。
    ///
    /// **省略された要素はコンパイラが `Op::Nil` を積む**ので、op にオペランドは要らない
    /// （`slice_from_values` が `Value::None` を「無し」に畳む）。検査もエラー文言も
    /// ツリーウォークと同じ 1 実装（`Interpreter::slice_from_values`）。
    BuildSlice,
    /// pop した値で**グローバルを新規宣言**する（#10-c: 最上位の `let`/`mut`/`const`）。
    /// フィールドは (name_idx, 宣言の種類)。
    ///
    /// コピー・フリーズ・可変性の扱いは `DeclKind` が担う。ツリーウォークの
    /// `exec_let` / `exec` の `Const`/`Mut` アームと**同じ判断を同じ順序で**行う
    /// （実体は `Interpreter::vm_declare_global`）。
    ///
    /// ⚠ **4 つの op に分けず種類はオペランドで持つ**。op を増やすと `Op` のサイズと
    /// ディスパッチの配置に効き、E2E ベンチが**どちら向きにも ±5% 揺れる**（#28 の実測）。
    /// 意味が同じものを別 op に分ける理由は無い。
    DeclareGlobal(u32, DeclKind),
    /// メソッド本体の `Self`（#27）: レシーバのクラスを `Value::Class` として push する。
    ///
    /// ツリーウォークは `exec_fn_evaled` が `Self` をスコープへ宣言するが、VM のフレームは
    /// フラットバッファでスコープを持たない。値の出どころは同じ `current_class`
    /// （`run_vm_method` がレシーバから設定済み）なので意味論は一致する。
    /// コンパイラは `self` パラメータを持つ本体でのみ emit する。
    LoadSelfClass,
    /// `obj::Trait.attr` の読み（#27）: pop obj, push `trait_access_evaled(obj, names[t], names[a])`。
    /// フィールドは (trait_name_idx, attr_idx)。
    GetTraitAttr(u32, u32),
    /// `obj::Trait.attr = v` の書き（#27）: スタックは `[obj, value]`。
    /// value・obj を pop して `trait_assign_evaled` で代入する（値は push しない）。
    /// フィールドは (trait_name_idx, attr_idx)。
    SetTraitAttr(u32, u32),
    /// `break_point`（#27）: `spans[idx]` でデバッガ REPL に入る（`exec_breakpoint` へ委譲）。
    ///
    /// ⚠ 実行後は **`Flow::NextAfterCall` を返す**こと。ここでデバッグセッションが始まるので、
    /// `Flow::Next` だと現フレームが停止判定を持たないまま走り抜ける（#1 で直した既存バグと同じ形）。
    BreakPoint(u32),
    // ── 添字・コレクションリテラル（タスク #5） ──
    /// 添字読み: pop key, pop obj, push `eval_subscript(obj, key)`（`obj[key]`）。
    Subscript,
    /// 添字書き: pop value, pop key, pop obj, `eval_setitem(obj, key, value)`（`obj[key] = value`）。
    SetIndex,
    /// リテラルリスト構築: 末尾 N 要素を pop して `Value::List` を push（`[a, b, ..]`）。
    BuildList(u16),
    /// リテラルタプル構築: 末尾 N 要素を pop して `Value::Tuple`（要素型名収集）を push（`(a, b, ..)`）。
    BuildTuple(u16),
    /// リテラル集合構築: 末尾 N 要素を pop して `Value::Set`（`set_insert` で重複排除）を push（`{a, b, ..}`）。
    BuildSet(u16),
    /// リテラル辞書構築: 末尾 2N 要素（k0,v0,k1,v1,..）を pop して `Value::Dict` を push（`{k: v, ..}`）。
    BuildDict(u16),
    // ── ジェネレータ（タスク #8） ──
    /// `yield expr`: pop した値をジェネレータの yield 収集バッファ（`GENERATOR_YIELDS`）へ追加する
    /// （eager 収集・ツリーウォークの `Stmt::Yield` と同一意味論。値は産出するだけで制御は継続）。
    Yield,
    // ── async（タスク #9） ──
    /// `target <- async->T: body`: pop した AsyncManager に `chunk.async_blocks[idx]` のタスクを投入する。
    /// 捕捉変数は frame から読み、`vm_async_submit`（capture_env 経由）で env を組む（ツリーウォーク一致）。
    AsyncSubmit(u32),
}


/// 最上位宣言の種類（#10-c・`Op::DeclareGlobal` のオペランド）。
///
/// ツリーウォーク側の対応:
/// - `Const`            … `exec` の `Stmt::Const` アーム（コピーもフリーズもしない）
/// - `Mut`              … `exec` の `Stmt::Mut` アーム（**常に** `deep_copy_value`）
/// - `LetPlain`         … `exec_let` の「不変ソース／リテラル」分岐（そのまま束縛）
/// - `LetFreezeInstance`… `exec_let` の「非識別子式」分岐（`Instance` のときだけ copy+freeze）
///
/// - `LetFromIdent`     … `exec_let` の「識別子ソース」分岐（#27-c）
///
/// ⚠ `LetFromIdent` だけ**コンパイル時に結論を出さない**。`exec_let` はソース変数の
/// 可変性を実行時の `get_var` で見て分岐するので、VM も同じ実行時判断を行う
/// （ソース名の index を持ち回るだけ）。予測して op を選ぶと再宣言や再束縛で
/// ずれる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    /// `const x = e`
    Const,
    /// `mut x = e`
    Mut,
    /// `let x = e`（ソースが不変と分かっている／リテラル）
    LetPlain,
    /// `let x = e`（非識別子式。`Instance` のときだけ deep_copy + freeze）
    LetFreezeInstance,
    /// `let x = <識別子>`（#27-c）。フィールドは**ソース名**の index。
    /// 実行時に `get_var(src)` を引き、可変なら copy+freeze・不変ならそのまま・
    /// 変数でなければ `LetFreezeInstance` と同じ扱い（＝`exec_let` と同一の分岐）。
    LetFromIdent(u32),
}

#[cfg(test)]
mod size_tests {
    use super::*;

    /// `Op` のサイズは `Vec<Op>`（Chunk の `code`）のキャッシュ密度そのもの。
    ///
    /// 最大 variant は `Call(u16,u32,u32,u32,u32)`。新しい op がこれを超えると
    /// **命令列全体**が太るので、意図せず起きていないかをここで固定する。
    ///
    /// ⚠ この数値が変わったら「速くなったか」を必ず A/B で測ること。ただし
    /// **この規模の変更は E2E ベンチを ±5% 揺らす**（#28 の実測）ので、
    /// 数 % の差を根拠に良し悪しを判断しないこと。
    #[test]
    fn op_size_is_pinned() {
        assert_eq!(
            std::mem::size_of::<Op>(),
            20,
            "Op のサイズが変わった: 命令列全体のキャッシュ密度に効くので意図を確認すること"
        );
    }
}
