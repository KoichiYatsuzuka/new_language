// vm/compiler/mod.rs — 解決済み AST → Chunk のコンパイラ（Phase V）。**型と入口の配線だけ**。
//
// 対応範囲は **定義文以外のすべて**。文（制御フロー・例外・宣言・代入・async）と式
// （ブロック式・クロージャ・呼び出し・添字・コレクション・テンプレート）を Chunk に載せる。
// 載らないのは**定義文**（fn/class/gen/trait/protocol/newtype/enum/import）だけで、
// それは設計どおりツリーウォークが実行する（#10-d）。
//
// ⚠ **フォールバックは無い**（#33）。`None` を返すと呼び出し側は `VmForceError` で**停止する**。
// ⇒ `bail()` は「ツリーウォークへ落とす印」ではなく「**まだ載せられていない印**」。
//
// ⚠ 入口は 6 つあり、挙動は [`CompileMode`] から導く（#52）。**新しい入口を足すときは
// バリアントを 1 つ足し、bool を並べない**。
//
// ## サブモジュール（#53 で分割。§5.1 の当初設計にようやく追いついたもの）
//
// | module | 役割 |
// |---|---|
// | [`entry`] | 公開入口 6 つとその `_inner` |
// | [`diag`] | `bail` 診断フック・VM が扱える組み込み名の表 |
// | [`decls`] | slot 採番と AST 走査（⚠ リゾルバと同順・同数） |
// | [`emit`] | 命令発行のプリミティブ・書き込み先の決定・型特化の判定 |
// | [`calls`] | 呼び出し（引数・FFI 情報・書き戻し先・async 投入） |
// | [`control`] | `try`/`finally`/`match` 文と脱出時の巻き戻し |
// | [`stmt`] | `compile_stmt` |
// | [`expr`] | `compile_expr` |
// | [`block_expr`] | ブロック式 5 種 |
//
// ⚠ 分割は **1 行も書き換えない機械的な移動**として行った（#53）。生成バイトコードが
// 全例題で byte-identical であることを確認済み。


use std::collections::{HashMap, HashSet};

use crate::ast::Stmt;
use crate::interpreter::Value;

use super::chunk::Chunk;
use super::op::Op;

mod block_expr;
mod calls;
mod control;
mod decls;
mod diag;
mod emit;
mod entry;
mod expr;
mod stmt;

pub use diag::expr_kind;
/// `vm_builtin_names_are_all_handled`（#22-d）専用の再エクスポート。
/// ⚠ 通常ビルドでは誰も使わないので `cfg(test)` を外すと未使用警告になる
/// （#53 で `cargo fix` に一度消され、テストがコンパイルできなくなった）。
#[cfg(test)]
pub(crate) use diag::VM_BUILTIN_NAMES;
pub use entry::{
    compile_async_body, compile_debug, compile_definition_expr, compile_fn, compile_module_stmt,
    compile_toplevel_stmt, is_toplevel_compile_target,
};

use decls::{
    block_body_bails, collect_expr_decls, collect_nested_decls, for_target_shadows,
    nested_fn_free_names, MAX_FINALLY_NEST,
};
use diag::{bail, bail_expr, has_named_args, is_vm_builtin, VM_BUILTIN_KW_NAMES};


struct Compiler {
    code: Vec<Op>,
    consts: Vec<Value>,
    names: Vec<String>,
    attr_caches: Vec<crate::ast::AttrCache>,
    spans: Vec<crate::token::Span>,
    /// 文境界の行テーブル（#1）。`code` と 1:1。詳細は [`Chunk::stmt_spans`](super::chunk::Chunk).
    stmt_spans: Vec<u32>,
    /// 「次に emit する op が文の先頭」を表す予約（#1）。`compile_stmt` が設定し `emit` が消費する。
    /// 文の先頭 op は `compile_stmt` の中で任意の深さから emit されるので、位置ではなく予約で持つ。
    pending_stmt: Option<u32>,
    /// AST 型解決層の注釈（#16 段階(b)/plan A）。node-id で型特化 op の判断に使う。空なら特化しない。
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    /// 名前 → slot（base スコープ: パラメータ + トップレベル let/mut/const、宣言順）。
    /// リゾルバの base slot 採番と同順（パラメータ→宣言）なので `Resolution::Local` と一致する。
    slots: HashMap<String, u16>,
    /// slot → 可変フラグ（`let x = <mut ソース>` の freeze 判定に使う）。
    slot_mut: Vec<bool>,
    /// slot → 型注釈（メソッド呼び出しの「obj は Instance」判定に使う）。
    slot_type: Vec<Option<String>>,
    /// `self` パラメータの slot（メソッド本体をコンパイルするとき Some）。
    /// `self` は型注釈を持たないが常に Instance なので、レシーバ判定で特別扱いする。
    self_slot: Option<u16>,
    /// ネストしたループのコンテキストスタック（break/continue のジャンプ先解決用）。
    loops: Vec<LoopCtx>,
    /// ネストしたブロック式のコンテキストスタック（block_return/loop_yield 用）。
    block_ctxs: Vec<BlockCtx>,
    /// **現在の文境界のオペランドスタック深さ**（最内ループ入口からの相対・#34）。
    ///
    /// `break`/`continue` はブロック式の**途中**から外側ループへ跳べる
    /// （`let s = 1 + block ->int: … break …` は `1` を積んだまま跳ぶ）。
    /// 跳ぶ前にこの数だけ `Op::Pop` してオペランドスタックをループ入口の深さへ戻す。
    ///
    /// ⚠ **`None`（深さ不明）なら `break`/`continue` は bail する。** 既定を `None` 側に
    /// 倒してあるので、伝播を書き忘れた式の形は「壊れる」ではなく「載らない」で止まる。
    stmt_base: Option<u16>,
    /// **これからコンパイルする式が始まる深さ**（`stmt_base` と同じ基準・#34）。
    ///
    /// `compile_expr` が入口で `take()` するので、**直前に設定した親だけ**が値を伝えられる。
    /// 伝播するのは「末尾にブロック式が来られる」式だけでよい（`BinOp` の左右・`UnaryOp`）。
    /// カッコの中（呼び出し引数・リテラル・添字）にブロック式は**構文上置けない**ので不要。
    pending: Option<u16>,
    /// **現在開いている `SetupTry` のスタック**（#34/#37）。各要素は `finally` 本体
    /// （`try/except` は `None`、`try/finally` は `Some(fin)`）。
    ///
    /// ⚠ オペランドスタックと**同じ問題がハンドラスタックにもある**。`break` が try 本体から
    /// 外側ループへ跳ぶと `PopTry` を通らず、**ハンドラが残って後続の例外を横取りする**
    /// （実際に踏んだ: ループを抜けた後の `raise` がループ内の `except` に捕まった）。
    /// さらに `finally` は**全出口で走らねばならない**ので、跳ぶ経路にも本体を複製する。
    ///
    /// ⚠ **式の中の `break` を見る静的判定は存在しない**（`block_body_bails` は `break`/`continue`
    /// を見ない = #34 の「2 つ目の walker を持たない」方針）。ここを数えるのが唯一の防波堤。
    try_stack: Vec<Option<Vec<Stmt>>>,
    /// `finally` 本体（脱出経路への複製を含む）をコンパイル中かどうか（#37）。
    /// **1 以上なら脱出制御は bail する**（finally の中から跳ぶのは未対応）。
    in_finally: usize,
    /// **どの入口からのコンパイルか**（#52）。挙動の分岐はすべてここから導く。
    mode: CompileMode,
    /// 名前付き slot 数（パラメータ + 全ローカル宣言）。temp slot はこの上に積む。
    named_locals: u16,
    /// 現在使用中の temp slot 数（match サブジェクト等のスタック規律の一時領域）。
    temps_in_use: u16,
    /// フレームに必要な総 slot 数（名前付き + temp の最大同時数）。
    n_locals: usize,
    /// 非同期タスクブロック（`AsyncSubmit(idx)` が index で参照, タスク #9）。
    async_blocks: Vec<crate::vm::chunk::AsyncBlock>,
    /// `LoadGlobal` のグローバル索引キャッシュ（#11）。emit ごとに1本割り当てる。
    global_caches: Vec<crate::ast::SlotCache>,
    /// メソッド呼び出しの FFI 境界検査用の表示情報（#27-b）。node_id → (表示名 index, span index)。
    /// 詳細は [`Chunk::ffi_call_info`](super::chunk::Chunk)。
    ffi_call_info: HashMap<u32, (u32, u32)>,
    /// native の `mut` ポインタ引数の書き戻し先（#48）。node_id → `WbCall`。
    /// 詳細は [`Chunk::wb_targets`](super::chunk::Chunk)。
    wb_targets: HashMap<u32, crate::vm::chunk::WbCall>,
    /// 入れ子 `fn` 定義（#27）。`Op::MakeFn` が index で参照する。
    fn_defs: Vec<crate::vm::chunk::ChunkFnDef>,
    /// テンプレート呼び出しの型引数リスト（#27-c）。`Op::CallTemplate` が index で参照する。
    type_arg_lists: Vec<Vec<String>>,
    /// `let a, b = t` の分解情報（#27-c）。`Op::LetTuple` が index で参照する。
    tuple_decls: Vec<crate::vm::chunk::TupleDecl>,
    /// キーワード/可変長引数つき呼び出し（#27-c）。`Op::CallKw` が index で参照する。
    kw_calls: Vec<crate::vm::chunk::KwCall>,
    /// 最上位モード（#10-b）で「`scopes[0]` の同名を確実に指す」と言える名前の集合。
    /// 空 = 関数本体のコンパイル（従来どおり `slots` に無い書き込み先は bail）。
    ///
    /// ⚠ **使う前に必ず `slots` を引くこと**。この集合は「最上位で宣言された名前」であって
    /// 「今この文でシャドウされていない名前」ではない。順序を守れば健全（その文の束縛は
    /// すべて `slots` に入るため）。`resolver::toplevel_declared_globals`（**減算なし**）が入る
    /// — 減算版を渡すと別の文の `for i in ...` のせいで `while i < N` の `i` まで落ちる（#27-c）。
    toplevel_globals: HashSet<String>,
    /// 外側の同名束縛を覆う `for` ループ変数の名前（#27・`for_target_shadows`）。
    /// `Stmt::For` のコンパイル時に、この名前だけ**本体の間だけ**専用 slot へ差し替える。
    shadowed_for_targets: HashSet<String>,
    /// `static mut` の名前 → 宣言位置（#27-d）。
    ///
    /// この名前は **slot を持たない**（記憶域は `Interpreter::static_cells`）ので、
    /// 読み書きは `LoadStatic`/`StoreStatic` に落とす。
    /// ⚠ **`slots` より先に引くこと**（`static` 名に slot は無いので順序で壊れはしないが、
    /// 将来 slot と同名が同居したときに黙って値が消えるのを防ぐ）。
    statics: HashMap<String, crate::token::Span>,
    /// **セル変数**の名前 → セル表の index（#27-d 段階 2b）。
    ///
    /// `Rc<RefCell<Value>>` を外側フレームやクロージャと共有する名前。slot は持たない。
    /// ⚠ **`slots` より先に引くこと**（`statics` と同じ理由）。
    cells: HashMap<String, u16>,
    /// **slot 番号 → セル index**（#27-d 段階 2b）。
    /// `Resolution::Local(slot)` が付いた読みをセルへ振り替えるために要る
    /// （セル化してもリゾルバの採番と合わせるため slot 番号は穴として残してある）。
    cell_by_slot: HashMap<u16, u16>,
    /// セル表の大きさ（`Chunk.n_cells` になる）。
    n_cells: usize,
}

/// **コンパイルの入口ごとのモード**（#52）。
///
/// #52 以前は `module_mode` / `name_lookup` / `debug_mode` の 3 bool が別々に立っていた。
/// 入口を足すときに「どれを立てるか」を推測するしかなかった（対応表がどこにも無かった）。
/// 組み合わせをこの enum 1 本に閉じ込め、**判定はすべてメソッド経由**にする。
///
/// ⚠ **新しい入口を足すときはここにバリアントを足す**。`base()` の呼び出し側で
/// bool を並べるのではなく、下のメソッドが答えを持つ形を保つこと。
///
/// ⚠ **`toplevel_globals` はモードではなくデータ**（#52 では畳まなかった）。
/// 「最上位相当か」の判定に `!toplevel_globals.is_empty()` を使っている箇所があり、
/// `AsyncBody` ではその集合が**捕捉名**なので `captures` が空だと偽になる。
/// モードから導くと**挙動が変わる**ので、そこは `reads_by_name` / `writes_toplevel_globals`
/// として式のまま名前を付けてある。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CompileMode {
    /// 関数本体（`compile_fn`）。読みは `LoadGlobal`（`scopes[0]` 限定）。
    Function,
    /// モジュール最上位の 1 文（`compile_toplevel_stmt`・#10-b）。書きは `StoreGlobal`。
    Toplevel,
    /// import モジュール本体の 1 文（`compile_module_stmt`・#42）。書きは `StoreName`。
    Module,
    /// 定義文脈の式（`compile_definition_expr`・#41）。自由な識別子は `LoadName`。
    DefinitionExpr,
    /// async ブロック本体（`compile_async_body`・タスク #9）。
    AsyncBody,
    /// デバッガ REPL の 1 文（`compile_debug`・V-E）。宣言は `DeclareName`。
    DebugRepl,
}

impl CompileMode {
    /// import モジュール本体か。代入を `StoreName`（チェーン探索）にする（#42）。
    #[inline]
    pub(super) fn is_module_body(self) -> bool {
        matches!(self, Self::Module)
    }

    /// 自由な識別子を**実行時に名前で引く**か（`LoadName`）。
    ///
    /// `eval()` と同じスコープ走査になるので `scopes` の深さを問わず健全（#41）。
    /// ⚠ #51 まで `debug_mode || name_lookup` と書かれていたが、`DebugRepl` は
    /// 常に `name_lookup` も真だったので**この 1 つに畳める**（挙動は同一）。
    #[inline]
    pub(super) fn uses_name_lookup(self) -> bool {
        matches!(self, Self::DefinitionExpr | Self::DebugRepl)
    }

    /// デバッガ REPL か。変数参照を slot ではなく名前引きへ落とし、`let dbg::x` を宣言できる（V-E）。
    /// ⚠ `uses_name_lookup` と違い、**融合・FFI 情報の記録も落とす**のはこちらだけ。
    #[inline]
    pub(super) fn is_debug_repl(self) -> bool {
        matches!(self, Self::DebugRepl)
    }
}

/// `Chunk` のうち**モードごとに違う**メタ情報（#52・`Compiler::finish` の引数）。
/// これ以外の 15 フィールドは `Compiler` からそのまま移すので、ここに並ぶのが差分のすべて。
#[derive(Default)]
struct ChunkMeta {
    /// slot → 変数名（デバッガ表示用）。`compile_debug` は slot を持たないので空。
    local_names: Vec<String>,
    /// 先頭から何 slot がパラメータか。関数本体以外は 0。
    n_params: usize,
    /// 不変キャプチャの (名前, slot)（#27-d）。クロージャ／async 本体だけ非空。
    captured_slots: Vec<(String, u16)>,
    /// 可変キャプチャの (名前, セル index)（#27-d 段階 2b）。クロージャ本体だけ非空。
    captured_cells: Vec<(String, u16)>,
}

/// 変数への書き込み先（#10-b）。`store_target` が決める。
enum StoreTarget {
    /// VM フレームの slot（`StoreLocal`）。
    Local(u16),
    /// `scopes[0]` のグローバル（`StoreGlobal`）。値は (name プール index, キャッシュ枠 index)。
    Global(u32, u32),
    /// `static mut` の共有セル（`StoreStatic`）。値は宣言位置の span index（#27-d）。
    Static(u32),
    /// セル変数（`StoreCell`）。値はフレームのセル表の index（#27-d 段階 2b）。
    Cell(u16),
    /// 名前で引いて代入する（`StoreName`・#42）。値は name プール index。
    /// **import モジュール本体**専用（`scopes[0]` 限定の `StoreGlobal` では届かない）。
    Name(u32),
}

/// ループ1つ分の break/continue ジャンプ先。`continue` は `continue_target` へ、
/// `break` はループ末尾（コンパイル完了時にバックパッチ）へジャンプする。
/// Arrow の「break/continue が入れ子の if/match/block を貫通して外側ループへ届く」規則は、
/// これらが単なる絶対ジャンプなので自然に成立する（スタックは文境界で平衡）。
struct LoopCtx {
    /// `continue` のジャンプ先（while の条件先頭）。
    continue_target: u32,
    /// `break` 命令の位置（ループ末尾へバックパッチする）。
    break_jumps: Vec<usize>,
    /// ループ入口時点の `try_stack` の深さ（#37）。`break`/`continue` はここまで巻き戻す
    /// ＝**ループの内側で開いた `try` だけ**を閉じ、その `finally` だけを走らせる。
    try_len: usize,
}

/// ブロック式1つ分のコンテキスト（block:/if/while/for/match 式）。
/// `block_return` は `result_slot` に値を格納して `end_jumps`（block_return 出口へバックパッチ）へ跳ぶ。
/// `loop_yield` は `yield_slot`（Some のブロック式＝block:/for/while）のリストへ追加する。
/// if/match 式は yield に対して透過（`yield_slot=None`）＝外側の for/while/block へ届く。
struct BlockCtx {
    /// `block_return` の値の格納先 slot。
    result_slot: u16,
    /// `block_return` の `Jump` 命令位置（block_return 出口へバックパッチ）。
    end_jumps: Vec<usize>,
    /// `loop_yield` の蓄積リスト slot（block:/for/while 式は Some、if/match 式は None＝透過）。
    yield_slot: Option<u16>,
    /// ブロック式入口時点の `try_stack` の深さ（#37）。`block_return` はここまで巻き戻す。
    try_len: usize,
    /// ブロック式入口時点のオペランド深さ（#40）。`finally` の複製の中の `block_return` は
    /// **複製が載っている分**（例外経路の `[exc]` 等）を捨ててから跳ぶ必要がある。
    entry_depth: Option<u16>,
    /// `->T` アノテーションの名前プール index（#35）。`block_return`/`loop_yield` の実行時検査に使う。
    ///
    /// ⚠ ツリーウォークは `BLOCK_RETURN_EXPECTED_TYPE.last()`（**式のスタックの最内**）を見る。
    /// `block:` **文**は TLS へ push しないので、**外側の式のアノテーションを引き継ぐ**
    /// （`Stmt::Block` のコンパイル時に継承する）。ここを独立させると off/on が割れる。
    return_type: Option<u32>,
}

impl Compiler {
    /// **全モード共通の初期状態**（#52）。呼び出し側は `..Compiler::base(mode, annot)` で
    /// 差分フィールドだけを書く。
    ///
    /// ⚠ **フィールドを足すときに直すのはここ 1 箇所**。#51 まで 38 フィールドの構造体
    /// リテラルが 5 箇所にあり、1 本足すたびに 5 箇所を直していた
    /// （差分が 5 倍に膨らみ、レビューで異常が埋もれる — #33 の golden 録り直し漏れと同じ形）。
    pub(super) fn base(mode: CompileMode, annotations: std::rc::Rc<crate::type_check::AstAnnotations>) -> Self {
        Compiler {
            code: Vec::new(),
            consts: Vec::new(),
            names: Vec::new(),
            attr_caches: Vec::new(),
            spans: Vec::new(),
            stmt_spans: Vec::new(),
            pending_stmt: None,
            annotations,
            slots: HashMap::new(),
            slot_mut: Vec::new(),
            slot_type: Vec::new(),
            self_slot: None,
            loops: Vec::new(),
            block_ctxs: Vec::new(),
            // 文の入口はオペランドスタックが平衡（#34）。
            stmt_base: Some(0),
            // ⚠ 既定は `None`（＝深さ不明なら `break`/`continue` は bail する安全側）。
            // `Some(0)` を要るのは「Chunk の先頭がそのまま式」の入口だけ（`DefinitionExpr`）。
            pending: None,
            try_stack: Vec::new(),
            in_finally: 0,
            mode,
            named_locals: 0,
            temps_in_use: 0,
            n_locals: 0,
            async_blocks: Vec::new(),
            global_caches: Vec::new(),
            ffi_call_info: HashMap::new(),
            wb_targets: HashMap::new(),
            fn_defs: Vec::new(),
            type_arg_lists: Vec::new(),
            tuple_decls: Vec::new(),
            kw_calls: Vec::new(),
            toplevel_globals: HashSet::new(),
            shadowed_for_targets: HashSet::new(),
            statics: HashMap::new(),
            cells: HashMap::new(),
            cell_by_slot: HashMap::new(),
            n_cells: 0,
        }
    }

    /// 覗き穴最適化を掛けてから `Chunk` に固める（#52）。**通常の入口はすべてこちら**。
    ///
    /// 覗き穴最適化（#2a）は、コード生成を「素直に出す」ままに保ちつつ構造的に出る無駄
    /// （`else` 無し `if` の次命令への `Jump` 等）を回収する。意味論は不変。
    ///
    /// ⚠ **`peephole::optimize` の呼び出しはこの 1 箇所**にしてある。コード索引を持つ op を
    /// 足したときに `peephole::code_target_mut` への登録漏れ（#27-d で `StaticInit` の飛び先を
    /// 忘れ、**テストも例題も通ってしまった**）を探す場所を 1 つに保つため。
    pub(super) fn finish(mut self, meta: ChunkMeta) -> Chunk {
        super::peephole::optimize(&mut self.code, &mut self.stmt_spans);
        self.into_chunk(meta)
    }

    /// `Chunk` を組み立てるだけ（**覗き穴最適化を掛けない**）。
    ///
    /// ⚠ 掛けないのは `compile_debug` だけ — #52 以前からそうで、**この差を消すのは
    /// 挙動の変更**（デバッガ REPL の 1 文に最適化を新たに通すこと）になるので保存した。
    /// 通したいなら独立したタスクとして A/B と golden 比較つきで判断すること。
    pub(super) fn into_chunk(self, meta: ChunkMeta) -> Chunk {
        Chunk {
            code: self.code,
            consts: self.consts,
            names: self.names,
            attr_caches: self.attr_caches,
            spans: self.spans,
            stmt_spans: self.stmt_spans,
            local_names: meta.local_names,
            n_locals: self.n_locals,
            n_params: meta.n_params,
            async_blocks: self.async_blocks,
            global_caches: self.global_caches,
            ffi_call_info: self.ffi_call_info,
            wb_targets: self.wb_targets,
            fn_defs: self.fn_defs,
            type_arg_lists: self.type_arg_lists,
            tuple_decls: self.tuple_decls,
            kw_calls: self.kw_calls,
            captured_slots: meta.captured_slots,
            n_cells: self.n_cells,
            captured_cells: meta.captured_cells,
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::CompileMode::*;
    use super::CompileMode;

    /// **#52 で 3 つの bool を `CompileMode` へ畳んだときの対応表**を固定する。
    ///
    /// ⚠ #52 以前は各入口の構造体リテラルに `module_mode` / `name_lookup` / `debug_mode` が
    /// 並んでいたので、値は**読めば分かった**。畳んだ後は `matches!` のアームを 1 つ書き換えると
    /// **静かに全入口の挙動が変わる**。ここが唯一の防波堤。
    ///
    /// 期待値は #51 時点の実装から写したもの（左から module_mode / name_lookup / debug_mode）。
    #[test]
    pub(super) fn mode_predicates_match_the_pre_52_flags() {
        let expected: &[(CompileMode, bool, bool, bool)] = &[
            //                     is_module_body / uses_name_lookup / is_debug_repl
            (Function, false, false, false),
            (Toplevel, false, false, false),
            (Module, true, false, false),
            (DefinitionExpr, false, true, false),
            (AsyncBody, false, false, false),
            (DebugRepl, false, true, true),
        ];
        for &(m, module_body, name_lookup, debug_repl) in expected {
            assert_eq!(m.is_module_body(), module_body, "is_module_body for {m:?}");
            assert_eq!(m.uses_name_lookup(), name_lookup, "uses_name_lookup for {m:?}");
            assert_eq!(m.is_debug_repl(), debug_repl, "is_debug_repl for {m:?}");
        }
    }

    /// `uses_name_lookup` は畳む前の **`debug_mode || name_lookup`** と一致すること。
    /// ⚠ 畳めた根拠は「`DebugRepl` は `name_lookup` も真だった」の 1 点だけなので、
    /// `DebugRepl` を名前引きでなくするなら**この畳み込みも解く**必要がある。
    #[test]
    pub(super) fn debug_repl_implies_name_lookup() {
        assert!(DebugRepl.uses_name_lookup());
    }
}
