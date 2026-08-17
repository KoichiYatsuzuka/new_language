// vm/compiler.rs — 解決済み AST → Chunk のコンパイラ（Phase V, V-A）。
//
// 保守的コンパイル: 対応できない構文に出会ったら `None` を返し、呼び出し側は
// ツリーウォークにフォールバックする（デュアルモード, D2）。
//
// V-A の対応範囲（トップレベル関数のリーフ計算に限定）:
// - 文: `return` / `if` / `while` / 式文 / パラメータへの代入・複合代入。
// - 式: リテラル / `Resolution::Local`（パラメータ読み）/ 二項・単項演算 / 属性（フィールド）読み。
// - **非対応（=フォールバック）**: ローカル宣言（let/mut/const の freeze 意味論を避けるため）、
//   関数・メソッド呼び出し、クロージャ、for/match/block、例外、可変長引数、
//   グローバル/組み込み参照、添字、コレクションリテラル 等。

use std::collections::{HashMap, HashSet};

use crate::ast::{
    BinOp, CallArg, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt, TupleTarget, Resolution,
};
use crate::interpreter::Value;

use super::chunk::Chunk;
use super::op::{DeclKind, Op};

/// 診断フック（#10）: コンパイルを諦めた地点と構文種別を計上する。
/// `AR_TW_STATS=1` のときだけ働く（既定は enabled() の分岐 1 つで終わる）。
fn bail(site: &'static str, stmt: Option<&Stmt>) {
    if !crate::interpreter::tw_stats::enabled() {
        return;
    }
    let detail = stmt
        .map(crate::interpreter::tw_stats::stmt_kind_of)
        .unwrap_or("-");
    crate::interpreter::tw_stats::record_bail(site, detail);
}

/// `bail` の式版（`Expr` のバリアント名を採る）。
fn bail_expr(site: &'static str, expr: &Expr) {
    if !crate::interpreter::tw_stats::enabled() {
        return;
    }
    crate::interpreter::tw_stats::record_bail(site, expr_kind(expr));
}

/// `Expr` バリアント名（診断フック用）。
pub fn expr_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Int(..) => "Int",
        Expr::Float(..) => "Float",
        Expr::ImaginaryLit(..) => "ImaginaryLit",
        Expr::Str(..) => "Str",
        Expr::Bool(..) => "Bool",
        Expr::None => "None",
        Expr::Undefined => "Undefined",
        Expr::Ident { .. } => "Ident",
        Expr::LocalVar { .. } => "LocalVar",
        Expr::DebugVar { .. } => "DebugVar",
        Expr::BinOp { .. } => "BinOp",
        Expr::UnaryOp { .. } => "UnaryOp",
        Expr::Call { .. } => "Call",
        Expr::Attr { .. } => "Attr",
        Expr::TraitAccess { .. } => "TraitAccess",
        Expr::Subscript { .. } => "Subscript",
        Expr::Slice { .. } => "Slice",
        Expr::List(..) => "List",
        Expr::Tuple(..) => "Tuple",
        Expr::Dict(..) => "Dict",
        Expr::Set(..) => "Set",
        Expr::IfExpr { .. } => "IfExpr",
        Expr::MatchExpr { .. } => "MatchExpr",
        Expr::ForExpr { .. } => "ForExpr",
        Expr::WhileExpr { .. } => "WhileExpr",
        Expr::Block { .. } => "Block",
        Expr::Cast { .. } => "Cast",
        Expr::IsType { .. } => "IsType",
        Expr::MustBe { .. } => "MustBe",
        Expr::TemplateInstantiate { .. } => "TemplateInstantiate",
    }
}

/// VM の `Call` op で解決できない呼び先名（純粋 builtin・型コンストラクタ）。
/// これらは `eval_builtin_ident_call` で特別扱いされるか、グローバル `Value::Type` として
/// 別セマンティクスで呼ばれるため、コンパイル時に弾いてツリーウォークへフォールバックする。
/// VM 内で評価済み引数から直接呼べる純粋組み込み（`eval_builtin_evaled` が扱う集合）。
/// `for x in range(n)` や `print(...)` を含む関数を VM に載せられるようにする。
/// キーワード/可変長引数を伴う呼び出しは `compile_call_args` が bail するので、ここに
/// 挙げた名前でも純粋な位置引数の呼び出しだけが `CallBuiltin` になる（＝評価済み引数で
/// 意味論が一致する形のみ）。
///
/// 型コンストラクタ（int/str/… は `Value::Type` グローバル）は**ここに含めない**。
/// 通常のグローバル呼び出し（`LoadGlobal`+`Call`）に流し、`call_value_evaled` の
/// `Value::Type` アーム＝`call_type_by_name_evaled` へ委譲する（ツリーウォークと同一経路・
/// ユーザーが同名をグローバル shadow しても `LoadGlobal` が拾うので健全）。
/// VM が `CallBuiltin` を発行する組み込み名。
///
/// ⚠ **この集合は `Interpreter::eval_builtin_evaled` が扱う名前の部分集合でなければならない**
/// （`run.rs` の `CallBuiltin` は `eval_builtin_evaled` が `None` を返すと `NameError` になる）。
/// 2 ファイルに跨る不変条件なので、`vm_builtin_names_are_all_handled` テストで固定してある（#22-d）。
pub(crate) const VM_BUILTIN_NAMES: &[&str] = &[
    "print", "range", "len", "next", "repr", "id", "enumerate", "zip", "getenv", "open", "close",
    // flat リスト（#27-c）。本体は `eval_builtin_flat_evaled` に 1 本化済み。
    "create_flat_int_list", "flat_get_int", "flat_set_int",
];

fn is_vm_builtin(name: &str) -> bool {
    VM_BUILTIN_NAMES.contains(&name)
}

/// **キーワード引数つきでも VM に載せられる**組み込み名（#27-c・`Op::CallBuiltinKw`）。
///
/// キーワードの扱いは組み込みごとに違う（`enumerate` は `start` だけ許容、`zip` はエラー、
/// `len` は名前を無視して位置引数扱い…）ので、`eval_builtin_evaled_named` で
/// **ツリーウォークと一致することを確認した名前だけ**を挙げる。ここに無い名前は従来どおり
/// bail してツリーウォークへ落とす（＝安全側）。
const VM_BUILTIN_KW_NAMES: &[&str] = &["enumerate", "open"];

/// 本体の**入れ子 `fn` が自由変数として参照する名前**をすべて集める（#27-d 段階 2b）。
///
/// この中で「自分の**可変**ローカル」に当たるものは、ツリーウォークだと `capture_env` が
/// `Var::Mutable` → `Var::Cell` へ昇格して**外側と `Rc<RefCell<Value>>` を共有**する。
/// VM でも slot ではなくセルに置かないと、クロージャ内の書き込みが外側へ返らない。
///
/// ⚠ **入れ子の入れ子まで拾える**（`collect_referenced_names` が `Stmt::FnDef` の本体へ
/// 降りるので、内側の `fn` の自由変数も中間の `fn` の参照に含まれる）。
fn nested_fn_free_names(body: &[Stmt]) -> HashSet<String> {
    fn walk(stmts: &[Stmt], out: &mut HashSet<String>) {
        for s in stmts {
            if let Stmt::FnDef { params, body, .. } = s {
                let mut own: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
                crate::interpreter::collect_declared_names(body, &mut own);
                let mut referenced: HashSet<String> = HashSet::new();
                crate::interpreter::collect_referenced_names(body, &mut referenced);
                out.extend(referenced.into_iter().filter(|n| !own.contains(n)));
                continue; // 本体へは降りない（その中の `fn` は上の `referenced` に含まれる）
            }
            // 制御フローの中に置かれた `fn` も拾う。
            match s {
                Stmt::If { branches, else_body } => {
                    for (_, b) in branches {
                        walk(b, out);
                    }
                    if let Some(eb) = else_body {
                        walk(eb, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => walk(body, out),
                Stmt::Block(b) => walk(b, out),
                Stmt::Match { arms, .. } => {
                    for a in arms {
                        walk(&a.body, out);
                    }
                }
                Stmt::Try { body, handlers, finally_body } => {
                    walk(body, out);
                    for h in handlers {
                        walk(&h.body, out);
                    }
                    if let Some(fb) = finally_body {
                        walk(fb, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = HashSet::new();
    walk(body, &mut out);
    out
}

/// 引数に**名前付き**（キーワード／可変長）が含まれるか（#27-c）。
/// `compile_call_args` は同じ判定を戻り値で返すが、それでは遅すぎる場面がある:
/// メソッド呼び出しは「レシーバを push するか frame 直読み融合にするか」を
/// **引数をコンパイルする前に**決めなければならない。
fn has_named_args(args: &[CallArg]) -> bool {
    args.iter()
        .any(|a| matches!(a, CallArg::Keyword { .. } | CallArg::Variadic(_)))
}

/// VM が呼び先として扱えず、ツリーウォークへ bail すべき組み込み名。
/// - `eval_builtin_ident_call` 専用の builtin のうち **`eval_builtin_evaled` が扱わない**もの
///   （`parse_ar` は AST を値へ変換するので評価済み引数では表現できない）。
///   `is_vm_builtin` の集合はここより先に判定されるので重複不要。
/// - `Value::Type` グローバルとして**登録されていない**型名（`tuple`/`list`/`type`/`byte`）。
///   これらはツリーウォークでも `NameError`（呼び出し不可）なので、bail して同じ挙動にする。
///   登録済みの型コンストラクタ（int/str/… は `LoadGlobal`+`Call` で解決）はここに含めない。
fn is_builtin_callee(name: &str) -> bool {
    matches!(
        name,
        "parse_ar" | "tuple" | "list" | "type" | "byte"
    )
}

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
    /// ⚠ `has_escape` は**文しか歩かない**ので、ブロック式の中の `break` は見えない。
    /// ここを数えるのが唯一の防波堤。
    try_stack: Vec<Option<Vec<Stmt>>>,
    /// `finally` 本体（脱出経路への複製を含む）をコンパイル中かどうか（#37）。
    /// **1 以上なら脱出制御は bail する**（finally の中から跳ぶのは未対応）。
    in_finally: usize,
    /// **自由な識別子を実行時に名前で引く**（#41）。`eval()` と同じスコープ走査になるので、
    /// **`scopes` の深さを問わず健全**（定義文脈の式は最上位とは限らない — import モジュール
    /// 本体の中のクラス定義など）。`debug_mode` と違い、融合・FFI 情報の記録は落とさない。
    name_lookup: bool,
    /// デバッガ REPL モード: 変数参照は slot ではなく名前引き（`LoadName`）へ落とす。
    /// 停止スコープの生変数へアクセスし、`let dbg::x` を宣言できるようにする（V-E）。
    debug_mode: bool,
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

/// 関数本体を Chunk へコンパイルする。非対応構文があれば `None`。
///
/// - `params`: 仮引数（可変長は `local::args` として末尾 slot に採番）。
/// - `body`: 解決済み関数本体（リゾルバが `res` を付与済み。**入れ子 `fn` の本体は未解決**）。
/// - `captures`: **不変キャプチャの名前**（#27-d）。クロージャ本体をコンパイルするときだけ非空。
///   末尾に slot を採番し、呼び出し側が `chunk.captured_slots` を見て値を書き込む。
pub fn compile_fn(
    params: &[Param],
    body: &[Stmt],
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    captures: &[String],
    mut_captures: &[String],
) -> Option<Chunk> {
    // 診断フック（#10）: 失敗したのに bail 地点が 1 件も記録されなかったら「未帰属」として計上する。
    // 個々の `?` を全て計装する代わりに、取りこぼしを総量で可視化する。
    if !crate::interpreter::tw_stats::enabled() {
        return compile_fn_inner(params, body, annotations, captures, mut_captures);
    }
    let before = crate::interpreter::tw_stats::bail_count();
    let out = compile_fn_inner(params, body, annotations, captures, mut_captures);
    if out.is_none() && crate::interpreter::tw_stats::bail_count() == before {
        bail("unattributed", None);
    }
    out
}

/// モジュール最上位の**単一文**を Chunk へコンパイルする（#10-b）。
///
/// `compile_fn` との違いは 2 点だけ:
/// - **パラメータも base ローカルも無い**。slot は文の内側（ループ本体・for ターゲット等）の
///   宣言にだけ割り当てる（`collect_nested_decls`）。最上位の宣言は slot ではなくグローバル。
/// - **書き込み先にグローバルを許す**（`toplevel_globals`）。読み取りは従来どおり AST の
///   `Resolution::Global`（#21-b）に従うので、ここで渡すのは書き込み判定のためだけ。
///
/// 対象は**定義文以外のすべて**（#10-b/#10-c/#10-c2）。
///
/// 許可リストではなく**定義文の除外リスト**にしてある。新しい文種別が増えたとき、
/// 許可リストだと黙って取りこぼす（#10-c2 の着手時、式文 909 件が
/// 「まだ試行すらしていない」状態で残っていた）。`compile_stmt` が対応していない文は
/// そこで bail するので、除外リストは「試すだけ無駄と分かっているもの」だけでよい。
///
/// 宣言文（`let`/`mut`/`const`）を含む理由は「宣言が多いから」ではない。
/// **初期化子がループ式**のとき（`mut xs = for i in range(N) -> list[T]: ... loop_yield ...`）
/// 本体が N 回まわるからで、実測では最上位のツリーウォークの **93% がこの形**だった。
/// 「1 回しか実行されない文はコンパイル損」と素朴に考えると取り逃す。
/// 最上位文のうち **Chunk 化を試みる対象か**（#10-c2 / #27-c）。
///
/// 定義文（＝インタプリタ状態への登録）は #10-d の担当で、試しても必ず bail する。
/// ⚠ **「対象外」と「コンパイル失敗」は別物**として扱うこと。呼び出し側がこれを見ずに
/// `compile_toplevel_stmt` の `None` を一律「失敗」と数えると、定義文が失敗に化けて
/// 数字が読めなくなる（実際 `toplevel_FAILED` 511 のうち **336 件が定義文**だった）。
pub fn is_toplevel_compile_target(stmt: &Stmt) -> bool {
    !matches!(
        stmt,
        Stmt::FnDef { .. }
            | Stmt::GenDef { .. }
            | Stmt::ClassDef { .. }
            | Stmt::TraitDef { .. }
            | Stmt::ProtocolDef { .. }
            | Stmt::NewTypeDef { .. }
            | Stmt::EnumDef { .. }
            | Stmt::Import { .. }
            | Stmt::FromImport { .. }
            | Stmt::Field { .. }
    )
}

pub fn compile_toplevel_stmt(
    stmt: &Stmt,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    toplevel_globals: &HashSet<String>,
) -> Option<Chunk> {
    if !is_toplevel_compile_target(stmt) {
        return None;
    }
    // 診断フック（#10）: `compile_fn` と同じく取りこぼしを「未帰属」として可視化する。
    if !crate::interpreter::tw_stats::enabled() {
        return compile_toplevel_stmt_inner(stmt, annotations, toplevel_globals);
    }
    // bail の分類先を「最上位」へ切り替える（#27。関数側と残タスクが別物なので混ぜない）。
    let _g = crate::interpreter::tw_stats::ToplevelCompileGuard::new(stmt);
    let before = crate::interpreter::tw_stats::bail_count();
    let out = compile_toplevel_stmt_inner(stmt, annotations, toplevel_globals);
    if out.is_none() && crate::interpreter::tw_stats::bail_count() == before {
        // 未帰属でも**どの文種別か**は分かる。これが無いと 46 件の出所を探せない（#27-c）。
        bail("unattributed", Some(stmt));
    }
    out
}

/// **定義文脈の式**を Chunk へコンパイルする（#41）。
///
/// クラスのフィールド既定値・`enum` の値・デコレータ式は**定義文の一部**なので
/// `compile_toplevel_stmt` の対象外（定義文は除外リストに入っている）。それでも中身は
/// 任意の式で、`block:` / `if` / `for` 式を書ける。ここを VM に載せないと
/// **ツリーウォークの制御フローが生き続ける**（#33 が削除できない理由だった）。
///
/// `compile_toplevel_stmt` との違い:
/// - **文ではなく式**をコンパイルし、値を `Return` で返す。
/// - **自由な識別子は `LoadName`**（`name_lookup`）。定義文の実行位置は最上位とは限らず
///   （import モジュール本体の中など）、`scopes[0]` 限定の `LoadGlobal` では
///   ツリーウォークの `eval()` と答えが変わる。名前引きなら深さを問わず一致する。
/// - **書き込み先は slot だけ**（`store_target` が `name_lookup` で bail する）。
///   式の中で宣言したローカルは `collect_expr_decls` が slot を振るので影響しない。
pub fn compile_definition_expr(
    expr: &Expr,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
) -> Option<Chunk> {
    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut n: u16 = 0;
    collect_expr_decls(expr, &mut slots, &mut slot_mut, &mut slot_type, &mut n)?;

    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }

    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        spans: Vec::new(),
        stmt_spans: Vec::new(),
        pending_stmt: None,
        annotations,
        slots,
        slot_mut,
        slot_type,
        self_slot: None,
        loops: Vec::new(),
        block_ctxs: Vec::new(),
        stmt_base: Some(0),
        pending: Some(0),
        try_stack: Vec::new(),
        in_finally: 0,
        name_lookup: true,
        debug_mode: false,
        named_locals: n,
        temps_in_use: 0,
        n_locals: n as usize,
        async_blocks: Vec::new(),
        global_caches: Vec::new(),
        ffi_call_info: HashMap::new(),
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
    };

    c.compile_expr(expr)?;
    c.emit(Op::Return);
    super::peephole::optimize(&mut c.code, &mut c.stmt_spans);

    Some(Chunk {
        code: c.code,
        consts: c.consts,
        names: c.names,
        attr_caches: c.attr_caches,
        spans: c.spans,
        stmt_spans: c.stmt_spans,
        local_names,
        n_locals: c.n_locals,
        n_params: 0,
        async_blocks: c.async_blocks,
        global_caches: c.global_caches,
        ffi_call_info: c.ffi_call_info,
        fn_defs: c.fn_defs,
        type_arg_lists: c.type_arg_lists,
        tuple_decls: c.tuple_decls,
        kw_calls: c.kw_calls,
        captured_slots: Vec::new(),
        n_cells: 0,
        captured_cells: Vec::new(),
    })
}

fn compile_toplevel_stmt_inner(
    stmt: &Stmt,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    toplevel_globals: &HashSet<String>,
) -> Option<Chunk> {
    let body = std::slice::from_ref(stmt);
    // 関数側と同じ扱い（#27）。最上位でも 1 文の中で `for` 変数が宣言を覆いうる。
    let shadowed_for_targets = for_target_shadows(&[], body);

    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut n: u16 = 0;
    // ⚠ 宣言文は **`collect_nested_decls` に渡してはいけない**（#10-c）。
    // あれは「その文が宣言する名前」に slot を割り当てるが、最上位の宣言はグローバルであって
    // slot ではない。slot を振ると `store_target` が slot 側を優先してしまい、
    // `DeclareGlobal` が出ずに値がフレームへ消える。初期化子の**内側**の宣言だけを採番する。
    let collected = match stmt {
        Stmt::Let(_, _, e) | Stmt::Mut(_, _, e) | Stmt::Const(_, _, e) => {
            collect_expr_decls(e, &mut slots, &mut slot_mut, &mut slot_type, &mut n)
        }
        // `let a, b = t` も宣言文（#27-c）。ターゲットは最上位ではグローバルなので
        // slot を振らない — 振ると `Op::LetTuple` が slot 束縛側を選び、値がフレームへ消える
        // （`collection.ar` の `tx` が実例）。
        Stmt::LetTuple { value, .. } => {
            collect_expr_decls(value, &mut slots, &mut slot_mut, &mut slot_type, &mut n)
        }
        _ => collect_nested_decls(body, &mut slots, &mut slot_mut, &mut slot_type, &mut n),
    };
    if collected.is_none() {
        bail("nested-decls", None);
        return None;
    }

    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }

    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        spans: Vec::new(),
        stmt_spans: Vec::new(),
        pending_stmt: None,
        annotations,
        slots,
        slot_mut,
        slot_type,
        self_slot: None,
        loops: Vec::new(),
        block_ctxs: Vec::new(),
        stmt_base: Some(0),
        pending: None,
        try_stack: Vec::new(),
        in_finally: 0,
        name_lookup: false,
        debug_mode: false,
        named_locals: n,
        temps_in_use: 0,
        n_locals: n as usize,
        async_blocks: Vec::new(),
        global_caches: Vec::new(),
        ffi_call_info: HashMap::new(),
        fn_defs: Vec::new(),
        type_arg_lists: Vec::new(),
        tuple_decls: Vec::new(),
        kw_calls: Vec::new(),
        toplevel_globals: toplevel_globals.clone(),
        shadowed_for_targets,
        // 最上位に `static` は無い（あれば定義文として #10-d の担当）。
        statics: HashMap::new(),
        cells: HashMap::new(),
        cell_by_slot: HashMap::new(),
        n_cells: 0,
    };

    c.compile_stmt(stmt)?;
    // 最上位文は値を返さない（`Return` は型検査が最上位で禁じる）。
    c.emit(Op::ReturnNil);
    super::peephole::optimize(&mut c.code, &mut c.stmt_spans);

    let chunk = Chunk {
        code: c.code,
        consts: c.consts,
        names: c.names,
        attr_caches: c.attr_caches,
        spans: c.spans,
        stmt_spans: c.stmt_spans,
        local_names,
        n_locals: c.n_locals,
        n_params: 0,
        async_blocks: c.async_blocks,
        global_caches: c.global_caches,
        ffi_call_info: c.ffi_call_info,
        fn_defs: c.fn_defs,
        type_arg_lists: c.type_arg_lists,
        tuple_decls: c.tuple_decls,
        kw_calls: c.kw_calls,
        // 最上位文にクロージャキャプチャは無い（#27-d）。
        captured_slots: Vec::new(),
        n_cells: 0,
        captured_cells: Vec::new(),
    };

    if std::env::var("AR_VM_DUMP").is_ok_and(|v| !v.is_empty()) {
        eprintln!("{}", super::disasm::disassemble(&chunk, "<toplevel>"));
    }

    Some(chunk)
}

fn compile_fn_inner(
    params: &[Param],
    body: &[Stmt],
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    captures: &[String],
    mut_captures: &[String],
) -> Option<Chunk> {
    // 外側変数をシャドウする `for` ループ変数は、本体のコンパイル中だけ専用 slot へ差し替える（#27）。
    let shadowed_for_targets = for_target_shadows(params, body);
    // base slot をリゾルバと同順で採番する: パラメータ → トップレベル let/mut/const。
    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut self_slot: Option<u16> = None;
    let mut n: u16 = 0;
    for (i, p) in params.iter().enumerate() {
        // 可変長パラメータ（#27）。`bind_args` は**受け取った値を `local::args` という名前で
        // 末尾に 1 つ**束縛する（引数が無ければ `Value::None`）ので、slot も同じ名前・同じ位置に
        // 採番すれば一般バインド経路（`bindings[i]` → slot i）とそのまま一致する。
        // ⚠ **並びが `bind_args` と一致することが健全性そのもの**。パーサは可変長を末尾に
        // しか許さないが、崩れたときに黙って値がずれないよう明示的に確かめる。
        if p.variadic {
            if i + 1 != params.len() {
                bail("variadic-param-not-last", None);
                return None;
            }
            slots.insert("local::args".to_string(), n);
            slot_mut.push(p.mutable);
            // 型注釈は**要素の型**（`let ...: int`）であって束縛される list の型ではない。
            slot_type.push(None);
            n = n.checked_add(1)?;
            continue;
        }
        if p.name == "self" {
            self_slot = Some(n);
        }
        slots.insert(p.name.clone(), n);
        slot_mut.push(p.mutable);
        slot_type.push(p.type_ann.clone());
        n = n.checked_add(1)?;
    }
    // パラメータ数（`self` 含む）。デバッガが「まだ宣言されていないローカル」を隠すのに使う（#1）。
    let n_params = n as usize;
    // `static mut` の名前 → 宣言位置（#27-d）。slot ではなく `static_cells` を指す名前の集合。
    let mut statics: HashMap<String, crate::token::Span> = HashMap::new();
    // トップレベル宣言を事前採番。LetTuple/Static/入れ子定義など slot をずらす形は非対応。
    for stmt in body {
        match stmt {
            Stmt::Let(name, ty, _) | Stmt::Const(name, ty, _)
                if name != "_" && !slots.contains_key(name) =>
            {
                slots.insert(name.clone(), n);
                slot_mut.push(false);
                slot_type.push(ty.clone());
                n = n.checked_add(1)?;
            }
            Stmt::Mut(name, ty, _) if name != "_" && !slots.contains_key(name) => {
                slots.insert(name.clone(), n);
                slot_mut.push(true);
                slot_type.push(ty.clone());
                n = n.checked_add(1)?;
            }
            // `_` 名・既出名の宣言は base slot を増やさない（no-op）。
            Stmt::Let(..) | Stmt::Const(..) | Stmt::Mut(..) => {}
            // 入れ子 `fn` の名前も base slot を占める（リゾルバの `collect_base_decls` と同順・#27）。
            // ⚠ **採番だけここで行い、載せられるかの判定は `compile_stmt` で行う**
            //    （自由変数の判定に `slots` の完成が要るため）。載せられなければそこで bail する。
            Stmt::FnDef { name, .. } if name != "_" && !slots.contains_key(name) => {
                slots.insert(name.clone(), n);
                slot_mut.push(false);
                slot_type.push(None);
                n = n.checked_add(1)?;
            }
            Stmt::FnDef { .. } => {}
            // `static mut x = e`（#27-d）。記憶域はフレームではなく `Interpreter::static_cells`
            // （宣言位置がキーの共有セル）で、呼び出しをまたいで生き残る。名前 → 宣言位置を
            // 控えておき、本体の読み書きを `LoadStatic`/`StoreStatic` に落とす。
            //
            // ⚠⚠ **slot は「使わないが必ず 1 つ消費する」**。リゾルバの `collect_base_decls` は
            // `Stmt::Static` にも `push_base` するので、ここで飛ばすと**以降の base slot が
            // 全部 1 つずれ**、`Resolution::Local(k)` が別の変数（または範囲外）を指す。
            // 実際に `LoadLocal` の添字 out-of-bounds で落ちた。**採番はリゾルバと同順・同数**が契約。
            Stmt::Static(name, _, span) => {
                statics.insert(name.clone(), span.clone());
                if name != "_" {
                    slot_mut.push(true);
                    slot_type.push(None);
                    n = n.checked_add(1)?; // 穴を空けるだけ（`slots` には入れない）
                }
            }
            // slot を採番する可能性のある未対応の宣言的文があれば、番号ずれを避けて丸ごと諦める。
            Stmt::LetTuple { .. }
            | Stmt::GenDef { .. }
            | Stmt::ClassDef { .. }
            | Stmt::TraitDef { .. }
            | Stmt::ProtocolDef { .. }
            | Stmt::NewTypeDef { .. }
            | Stmt::EnumDef { .. }
            | Stmt::Import { .. }
            | Stmt::FromImport { .. } => {
                bail("decl-prepass", Some(stmt));
                return None;
            }
            _ => {}
        }
    }
    // ネストしたブロック（if/while/match のボディ）内の Let/Const/Mut にも
    // フレーム内固定 slot を割り当てる（R0-B: 関数内の全ローカルが平坦 slot）。
    // トップレベル decl は上で採番済みなのでスキップされる。順序は問わない
    // （compile は slots 引きで参照する）。シャドウイング禁止＝同名は非同時生存なので
    // slot 再利用は健全。リゾルバは nested 名を解決しない（Ident のまま）ので衝突しない。
    if collect_nested_decls(body, &mut slots, &mut slot_mut, &mut slot_type, &mut n).is_none() {
        bail("nested-decls", None);
        return None;
    }

    // クロージャの不変キャプチャに slot を採番する（#27-d）。
    //
    // **必ず最後**に採番する。`Resolution::Local` が付いた本体（＝リゾルバが解決した
    // トップレベル関数）は「パラメータ → 本体直下の宣言」の並びを前提に番号が焼かれているので、
    // 途中に差し込むとずれる。末尾に足すぶんには既存の番号を動かさない。
    // （実際にはクロージャ本体はリゾルバが降りないので全て `Unresolved` だが、
    //   この不変条件を崩さないでおく方が安全。）
    //
    // ⚠ **既に slot がある名前とぶつかったら諦める**。`capture_env` は
    // 「パラメータでも本体の宣言でもない自由変数」だけを捕まえるので本来ぶつからないが、
    // ぶつかるということは `collect_declared_names`（キャプチャ側）と `slots`（コンパイラ側）の
    // 木の歩き方がずれているということ。**黙って上書きすると閉包変数が消える**ので
    // 計測できる形で落とす。
    let mut captured_slots: Vec<(String, u16)> = Vec::with_capacity(captures.len());
    // 採番順を安定させる（`captured_env` は HashMap なので反復順が実行ごとに変わる）。
    let mut capture_names: Vec<&String> = captures.iter().collect();
    capture_names.sort();
    for name in capture_names {
        if slots.contains_key(name) {
            bail("capture-slot-conflict", None);
            return None;
        }
        slots.insert(name.clone(), n);
        slot_mut.push(false); // 不変キャプチャ（`CapturedVar::Immutable`）だけを載せる
        slot_type.push(None);
        captured_slots.push((name.clone(), n));
        n = n.checked_add(1)?;
    }

    // **可変キャプチャ**にセル index を採番する（#27-d 段階 2b）。
    //
    // slot ではなくセルなのは、ツリーウォークが `CapturedVar::Mutable(cell)` として
    // **外側と同じ `Rc<RefCell<Value>>` を共有**するから。slot（`Value` 直値）へ値を
    // コピーすると、クロージャ内の書き込みが外側へ返らない。
    // 実行時は `build_cells` が `captured_env` のセルを**そのまま**この index へ入れる。
    let mut cells: HashMap<String, u16> = HashMap::new();
    let mut captured_cells: Vec<(String, u16)> = Vec::with_capacity(mut_captures.len());
    let mut mut_capture_names: Vec<&String> = mut_captures.iter().collect();
    mut_capture_names.sort(); // 採番順を安定させる（HashMap 由来）
    for name in mut_capture_names {
        // 不変キャプチャと同じ理由で、slot と衝突したら諦める。
        if slots.contains_key(name) || cells.contains_key(name) {
            bail("capture-slot-conflict", None);
            return None;
        }
        let idx = u16::try_from(cells.len()).ok()?;
        cells.insert(name.clone(), idx);
        captured_cells.push((name.clone(), idx));
    }

    // **入れ子 `fn` に可変キャプチャされる自分のローカル**もセルへ移す（#27-d 段階 2b）。
    //
    // ⚠ **slot は解放しない**（穴のまま残す）。`Resolution::Local(k)` はリゾルバの採番で
    // 焼かれているので、詰め直すと別の変数を指す（`static` で実際に踏んだ）。
    // 読み書きは `cells` を先に引くことで slot 側へ行かないようにする。
    let mut cell_by_slot: HashMap<u16, u16> = HashMap::new();
    let mut free_in_nested: Vec<String> = nested_fn_free_names(body).into_iter().collect();
    free_in_nested.sort(); // 採番順を安定させる
    for name in free_in_nested {
        let Some(&slot) = slots.get(&name) else { continue };
        if !slot_mut.get(slot as usize).copied().unwrap_or(false) {
            continue; // 不変ローカルのキャプチャは値の複製で足りる（従来どおり slot）
        }
        let idx = u16::try_from(cells.len()).ok()?;
        slots.remove(&name); // slot 番号は残したまま名前だけ外す
        cells.insert(name.clone(), idx);
        cell_by_slot.insert(slot, idx);
    }
    let n_cells = cells.len();

    // V-E: slot → 変数名 のデバッグ名テーブル（named slot のみ。temp は無名）。
    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }

    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        spans: Vec::new(),
        stmt_spans: Vec::new(),
        pending_stmt: None,
        annotations,
        slots,
        slot_mut,
        slot_type,
        self_slot,
        loops: Vec::new(),
        block_ctxs: Vec::new(),
        stmt_base: Some(0),
        pending: None,
        try_stack: Vec::new(),
        in_finally: 0,
        name_lookup: false,
        debug_mode: false,
        named_locals: n,
        temps_in_use: 0,
        n_locals: n as usize,
        async_blocks: Vec::new(),
        global_caches: Vec::new(),
        ffi_call_info: HashMap::new(),
        fn_defs: Vec::new(),
        type_arg_lists: Vec::new(),
        tuple_decls: Vec::new(),
        kw_calls: Vec::new(),
        toplevel_globals: HashSet::new(),
        shadowed_for_targets,
        statics,
        cells,
        cell_by_slot,
        n_cells,
    };

    for stmt in body {
        c.compile_stmt(stmt)?;
    }
    // 本体末尾までフォールオフしたら None を返す。
    c.emit(Op::ReturnNil);

    // 覗き穴最適化（#2a）。コード生成は「素直に出す」ままにして、構造的に出る無駄
    // （`else` 無し `if` の次命令への `Jump` 等）はここで回収する。意味論は不変。
    super::peephole::optimize(&mut c.code, &mut c.stmt_spans);

    let chunk = Chunk {
        code: c.code,
        consts: c.consts,
        names: c.names,
        attr_caches: c.attr_caches,
        spans: c.spans,
        stmt_spans: c.stmt_spans,
        local_names,
        n_locals: c.n_locals,
        n_params,
        async_blocks: c.async_blocks,
        global_caches: c.global_caches,
        ffi_call_info: c.ffi_call_info,
        fn_defs: c.fn_defs,
        type_arg_lists: c.type_arg_lists,
        tuple_decls: c.tuple_decls,
        kw_calls: c.kw_calls,
        captured_slots,
        n_cells: c.n_cells,
        captured_cells,
    };

    // 開発用フック: `AR_VM_DUMP=1` で生成バイトコードを標準エラーへ逆アセンブルする。
    // どの式に型特化 op が乗ったかを目視で確認するために使う（disasm.rs の唯一の呼び元）。
    if std::env::var("AR_VM_DUMP").is_ok_and(|v| !v.is_empty()) {
        eprintln!("{}", super::disasm::disassemble(&chunk, "<fn>"));
    }

    Some(chunk)
}

/// `mng <- async->T: body` の**本体**を Chunk へコンパイルする（#32）。
///
/// 本体は「値を返すブロック式」（`block_return v` が結果）なので、`compile_fn` ではなく
/// `compile_block_expr` の上に `Return` を載せた形にする。
///
/// `captures` は submit 時に deep-clone された環境の変数名（D5 share-nothing）。
/// #27-d 段階 1 と同じく**末尾に slot を採番**し、実行側が `chunk.captured_slots` を見て
/// 値を書き込む（名前で引くので `env` の並びに依存しない）。
///
/// ⚠ **worker スレッドは型注釈を持てない**（`AstAnnotations` は `Rc` ベースで `Send` でない）。
/// 呼び出し側は空の注釈を渡すので**型特化 op は乗らない**が、
/// 注釈は最適化ヒントであって意味論の根拠ではない（#15e の原則）ので結果は変わらない。
///
/// ⚠ **worker のスコープは `[globals, env]` の 2 段**。ここで slot に載らない自由名は
/// `LoadName`（`get_val`）に落ちる必要があるので、`toplevel_globals` に env の名前を入れて
/// 最上位モード扱いにする（`frame_floor` は 0 のままで、覗かれて困る呼び出し元も居ない）。
pub fn compile_async_body(
    body: &[Stmt],
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    captures: &[String],
) -> Option<Chunk> {
    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut n: u16 = 0;
    // 本体の宣言（入れ子ブロックを含む）を先に採番する。
    if collect_nested_decls(body, &mut slots, &mut slot_mut, &mut slot_type, &mut n).is_none() {
        bail("nested-decls", None);
        return None;
    }
    // 捕捉環境は末尾（#27-d 段階 1 と同じ理由・同じ表）。
    let mut captured_slots: Vec<(String, u16)> = Vec::with_capacity(captures.len());
    let mut capture_names: Vec<&String> = captures.iter().collect();
    capture_names.sort();
    for name in capture_names {
        // ⚠ **本体が同名を宣言していたら諦める**（#27-d 段階 1 と同じ判断）。
        // ツリーウォークでは捕捉値が push されたスコープに居るので**宣言より前は捕捉値が見える**。
        // slot を宣言側に取られると、その読みが未初期化 slot になって黙って値が変わる。
        if slots.contains_key(name) {
            bail("capture-slot-conflict", None);
            return None;
        }
        slots.insert(name.clone(), n);
        slot_mut.push(false);
        slot_type.push(None);
        captured_slots.push((name.clone(), n));
        n = n.checked_add(1)?;
    }

    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }
    let shadowed_for_targets = for_target_shadows(&[], body);

    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        spans: Vec::new(),
        stmt_spans: Vec::new(),
        pending_stmt: None,
        annotations,
        slots,
        slot_mut,
        slot_type,
        self_slot: None,
        loops: Vec::new(),
        block_ctxs: Vec::new(),
        stmt_base: Some(0),
        pending: None,
        try_stack: Vec::new(),
        in_finally: 0,
        name_lookup: false,
        debug_mode: false,
        named_locals: n,
        temps_in_use: 0,
        n_locals: n as usize,
        async_blocks: Vec::new(),
        global_caches: Vec::new(),
        ffi_call_info: HashMap::new(),
        fn_defs: Vec::new(),
        type_arg_lists: Vec::new(),
        tuple_decls: Vec::new(),
        kw_calls: Vec::new(),
        // 空でないと `Op::LoadName` の分岐（最上位扱い）に入らない。名前の中身は
        // 書き込み判定にしか使わないので、捕捉名を入れておけば足りる。
        toplevel_globals: captures.iter().cloned().collect(),
        shadowed_for_targets,
        statics: HashMap::new(),
        // async 本体は可変キャプチャを持たない（submit 時に deep-clone される・D5）。
        cells: HashMap::new(),
        cell_by_slot: HashMap::new(),
        n_cells: 0,
    };

    // async 本体は Chunk の先頭＝オペランドスタックは空（#34）。
    // アノテーションは持たない（`mng <- async->T:` の T は代入先の型で、`block_return` の
    // 実行時検査はツリーウォークでも走らない＝`BLOCK_RETURN_EXPECTED_TYPE` は空・#35）。
    c.compile_block_expr(body, Some(0), None, true)?;
    c.emit(Op::Return);
    super::peephole::optimize(&mut c.code, &mut c.stmt_spans);

    let chunk = Chunk {
        code: c.code,
        consts: c.consts,
        names: c.names,
        attr_caches: c.attr_caches,
        spans: c.spans,
        stmt_spans: c.stmt_spans,
        local_names,
        n_locals: c.n_locals,
        n_params: 0,
        async_blocks: c.async_blocks,
        global_caches: c.global_caches,
        ffi_call_info: c.ffi_call_info,
        fn_defs: c.fn_defs,
        type_arg_lists: c.type_arg_lists,
        tuple_decls: c.tuple_decls,
        kw_calls: c.kw_calls,
        captured_slots,
        n_cells: 0,
        captured_cells: Vec::new(),
    };
    if std::env::var("AR_VM_DUMP").is_ok_and(|v| !v.is_empty()) {
        eprintln!("{}", super::disasm::disassemble(&chunk, "<async>"));
    }
    Some(chunk)
}

/// デバッガ REPL の 1 文をデバッグモード（名前引きアクセス）でコンパイルする。
/// 対応: 式文（値を `Return`）・`let/const dbg::name = 式`（`DeclareName`）。
/// メソッド呼び出し・添字・制御フロー等は `None`（呼び出し側がツリーウォークへフォールバック）。
pub fn compile_debug(stmt: &Stmt) -> Option<Chunk> {
    let mut c = Compiler {
        code: Vec::new(),
        consts: Vec::new(),
        names: Vec::new(),
        attr_caches: Vec::new(),
        spans: Vec::new(),
        stmt_spans: Vec::new(),
        pending_stmt: None,
        // デバッグ経路は注釈を持たない（空＝型特化しない）。
        annotations: std::rc::Rc::new(crate::type_check::AstAnnotations::default()),
        slots: HashMap::new(),
        slot_mut: Vec::new(),
        slot_type: Vec::new(),
        self_slot: None,
        loops: Vec::new(),
        block_ctxs: Vec::new(),
        stmt_base: Some(0),
        pending: None,
        try_stack: Vec::new(),
        in_finally: 0,
        name_lookup: true,
        debug_mode: true,
        named_locals: 0,
        temps_in_use: 0,
        n_locals: 0,
        async_blocks: Vec::new(),
        global_caches: Vec::new(),
        ffi_call_info: HashMap::new(),
        fn_defs: Vec::new(),
        type_arg_lists: Vec::new(),
        tuple_decls: Vec::new(),
        kw_calls: Vec::new(),
        toplevel_globals: HashSet::new(),
        // デバッガ REPL の 1 文。`for` 変数のシャドウは扱わない（名前引きで動く）。
        shadowed_for_targets: HashSet::new(),
        statics: HashMap::new(),
        cells: HashMap::new(),
        cell_by_slot: HashMap::new(),
        n_cells: 0,
    };
    match stmt {
        Stmt::Expr(e) => {
            c.compile_expr(e)?;
            c.emit(Op::Return); // 式の値を返す（呼び出し側が表示）
        }
        Stmt::Let(name, _, e) | Stmt::Const(name, _, e) if name != "_" => {
            c.compile_expr(e)?;
            let ni = c.add_name(name);
            c.emit(Op::DeclareName(ni));
            c.emit(Op::ReturnNil);
        }
        _ => return None,
    }
    Some(Chunk {
        code: c.code,
        consts: c.consts,
        names: c.names,
        attr_caches: c.attr_caches,
        spans: c.spans,
        stmt_spans: c.stmt_spans,
        local_names: Vec::new(),
        n_locals: c.n_locals,
        // デバッグ REPL 用 Chunk は停止対象ではないので 0 でよい。
        n_params: 0,
        async_blocks: Vec::new(),
        global_caches: c.global_caches,
        ffi_call_info: c.ffi_call_info,
        fn_defs: c.fn_defs,
        type_arg_lists: c.type_arg_lists,
        tuple_decls: c.tuple_decls,
        kw_calls: c.kw_calls,
        // デバッガ REPL の 1 文は名前引きで動く（キャプチャ slot は使わない）。
        captured_slots: Vec::new(),
        n_cells: 0,
        captured_cells: Vec::new(),
    })
}

/// param または非 `for` 宣言と名前衝突する `for` ループ変数（式形含む）の集合（#27）。
///
/// Arrow の `for` 変数はブロックスコープで、ループを抜けると外側の同名変数が戻る。
/// flat-slot モデルは名前ごとに 1 slot なので、素直に採番すると外側の値を壊す。
/// ⇒ **ここに挙がった名前だけ、ループ本体のコンパイル中に専用 slot へ差し替える**
/// （`compile_stmt` の `Stmt::For`）。以前はこの集合が空でなければ関数ごと諦めていた。
fn for_target_shadows(params: &[Param], body: &[Stmt]) -> HashSet<String> {
    let mut for_names: HashSet<String> = HashSet::new();
    let mut decl_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
    scan_shadow_stmts(body, &mut for_names, &mut decl_names);
    for_names.intersection(&decl_names).cloned().collect()
}

fn scan_shadow_stmts(
    stmts: &[Stmt],
    for_names: &mut HashSet<String>,
    decl_names: &mut HashSet<String>,
) {
    for s in stmts {
        match s {
            Stmt::Let(n, _, e) | Stmt::Const(n, _, e) | Stmt::Mut(n, _, e) => {
                decl_names.insert(n.clone());
                scan_shadow_expr(e, for_names, decl_names);
            }
            Stmt::LetTuple { targets, value, .. } => {
                for t in targets {
                    match t {
                        TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => {
                            decl_names.insert(n.clone());
                        }
                        TupleTarget::Wildcard => {}
                    }
                }
                scan_shadow_expr(value, for_names, decl_names);
            }
            Stmt::Static(n, e, _) => {
                decl_names.insert(n.clone());
                scan_shadow_expr(e, for_names, decl_names);
            }
            Stmt::For { targets, iter, body } => {
                for t in targets {
                    for_names.insert(t.clone());
                }
                scan_shadow_expr(iter, for_names, decl_names);
                scan_shadow_stmts(body, for_names, decl_names);
            }
            Stmt::Expr(e)
            | Stmt::BlockReturn(e, _)
            | Stmt::LoopYield(e)
            | Stmt::Yield(e)
            | Stmt::Return(Some(e))
            | Stmt::Assign { value: e, .. }
            | Stmt::CompoundAssign { value: e, .. } => scan_shadow_expr(e, for_names, decl_names),
            Stmt::AttrAssign { target, value } | Stmt::AttrCompoundAssign { target, value, .. } => {
                scan_shadow_expr(target, for_names, decl_names);
                scan_shadow_expr(value, for_names, decl_names);
            }
            Stmt::If { branches, else_body } => {
                for (c, b) in branches {
                    scan_shadow_expr(c, for_names, decl_names);
                    scan_shadow_stmts(b, for_names, decl_names);
                }
                if let Some(eb) = else_body {
                    scan_shadow_stmts(eb, for_names, decl_names);
                }
            }
            Stmt::While { cond, body } => {
                scan_shadow_expr(cond, for_names, decl_names);
                scan_shadow_stmts(body, for_names, decl_names);
            }
            Stmt::Match { subject, arms, .. } => {
                scan_shadow_expr(subject, for_names, decl_names);
                for a in arms {
                    scan_shadow_stmts(&a.body, for_names, decl_names);
                }
            }
            Stmt::Block(b) => scan_shadow_stmts(b, for_names, decl_names),
            Stmt::Try { body, handlers, finally_body } => {
                scan_shadow_stmts(body, for_names, decl_names);
                for h in handlers {
                    if let Some(alias) = &h.name {
                        decl_names.insert(alias.clone());
                    }
                    scan_shadow_stmts(&h.body, for_names, decl_names);
                }
                if let Some(fb) = finally_body {
                    scan_shadow_stmts(fb, for_names, decl_names);
                }
            }
            _ => {}
        }
    }
}

fn scan_shadow_expr(
    e: &Expr,
    for_names: &mut HashSet<String>,
    decl_names: &mut HashSet<String>,
) {
    macro_rules! rec {
        ($x:expr) => {
            scan_shadow_expr($x, for_names, decl_names)
        };
    }
    match e {
        Expr::Block { stmts, .. } => scan_shadow_stmts(stmts, for_names, decl_names),
        Expr::IfExpr { branches, else_body, .. } => {
            for (c, b) in branches {
                rec!(c);
                scan_shadow_stmts(b, for_names, decl_names);
            }
            if let Some(eb) = else_body {
                scan_shadow_stmts(eb, for_names, decl_names);
            }
        }
        Expr::MatchExpr { subject, arms, .. } => {
            rec!(subject);
            for a in arms {
                scan_shadow_stmts(&a.body, for_names, decl_names);
            }
        }
        Expr::ForExpr { target, iter, body, .. } => {
            for_names.insert(target.clone());
            rec!(iter);
            scan_shadow_stmts(body, for_names, decl_names);
        }
        Expr::WhileExpr { cond, body, .. } => {
            rec!(cond);
            scan_shadow_stmts(body, for_names, decl_names);
        }
        Expr::BinOp { left, right, .. } => {
            rec!(left);
            rec!(right);
        }
        Expr::UnaryOp { operand, .. } => rec!(operand),
        Expr::Call { func, args, .. } => {
            rec!(func);
            for a in args {
                match a {
                    CallArg::Positional(x) | CallArg::Keyword { value: x, .. } => rec!(x),
                    CallArg::Variadic(xs) => {
                        for x in xs {
                            rec!(x);
                        }
                    }
                }
            }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => rec!(object),
        Expr::Subscript { object, index, .. } => {
            rec!(object);
            rec!(index);
        }
        Expr::Slice { begin, end, step } => {
            for x in [begin, end, step].into_iter().flatten() {
                rec!(x);
            }
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for x in items {
                rec!(x);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                rec!(k);
                rec!(v);
            }
        }
        Expr::TemplateInstantiate { base, .. } => rec!(base),
        Expr::IsType { expr, .. } | Expr::MustBe { expr, .. } => rec!(expr),
        Expr::Cast { object, .. } => rec!(object),
        _ => {}
    }
}

/// slot テーブルへ1つ宣言を追加する（既出名・`_` はスキップ）。
fn add_decl(
    name: &str,
    ty: &Option<String>,
    mutable: bool,
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    if name != "_" && !slots.contains_key(name) {
        slots.insert(name.to_string(), *n);
        slot_mut.push(mutable);
        slot_type.push(ty.clone());
        *n = n.checked_add(1)?;
    }
    Some(())
}

/// ネストしたブロック内の `let`/`const`/`mut` 宣言に平坦 slot を割り当てる（再帰）。
/// コンパイラが本体をコンパイルできる構文（if/while/match/for/try）と、**ブロック式**
/// （式の中の `block:`/if/while/for/match 式の本体宣言）にも踏み込む。
/// 既出名（トップレベル decl・別ブロックの同名）はスキップ（slot 再利用）。
fn collect_nested_decls(
    body: &[Stmt],
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    for stmt in body {
        match stmt {
            Stmt::Let(name, ty, e) | Stmt::Const(name, ty, e) => {
                add_decl(name, ty, false, slots, slot_mut, slot_type, n)?;
                collect_expr_decls(e, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Mut(name, ty, e) => {
                add_decl(name, ty, true, slots, slot_mut, slot_type, n)?;
                collect_expr_decls(e, slots, slot_mut, slot_type, n)?;
            }
            // 入れ子の `let a, b = t`（#27-c）。ツリーウォークはスコープを push するので
            // **反復ごとに宣言し直せる**が、slot を割り当てないとグローバル宣言に落ちて
            // 2 周目で「already declared」になる（`built_in.ar` の `zx` が実例）。
            // ここで slot に載せることで、最上位の `let a, b = t` 文（slot 無し）と
            // 入れ子（slot 有り）をコンパイラが区別できる。
            Stmt::LetTuple { targets, value, .. } => {
                for t in targets {
                    match t {
                        crate::ast::TupleTarget::Wildcard => {}
                        crate::ast::TupleTarget::Let(name)
                        | crate::ast::TupleTarget::Bare(name) => {
                            add_decl(name, &None, false, slots, slot_mut, slot_type, n)?
                        }
                        crate::ast::TupleTarget::Mut(name) => {
                            add_decl(name, &None, true, slots, slot_mut, slot_type, n)?
                        }
                    }
                }
                collect_expr_decls(value, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Expr(e)
            | Stmt::BlockReturn(e, _)
            | Stmt::LoopYield(e)
            | Stmt::Yield(e)
            | Stmt::Return(Some(e)) => collect_expr_decls(e, slots, slot_mut, slot_type, n)?,
            Stmt::Assign { value, .. } | Stmt::CompoundAssign { value, .. } => {
                collect_expr_decls(value, slots, slot_mut, slot_type, n)?
            }
            // 入れ子ブロック（`block:` 式・if/while の本体など）の中の `fn` 定義（#27-c）。
            // 名前は**そのブロックのローカル**なので slot を振る（関数本体直下の `fn` を
            // base slot に採番するのと同じ扱い）。⚠ **本体には踏み込まない**（別フレーム）。
            // ⚠ ここが抜けていたため `alias.ar` の `block->function` が
            // 「slot にもグローバルにも無い識別子」として bail していた
            //   — 採番 walker と `compile_stmt` の walker がずれていた典型例。
            Stmt::FnDef { name, .. } => {
                add_decl(name, &None, false, slots, slot_mut, slot_type, n)?;
            }
            Stmt::AttrAssign { target, value }
            | Stmt::AttrCompoundAssign { target, value, .. } => {
                collect_expr_decls(target, slots, slot_mut, slot_type, n)?;
                collect_expr_decls(value, slots, slot_mut, slot_type, n)?;
            }
            Stmt::If { branches, else_body } => {
                for (c, b) in branches {
                    collect_expr_decls(c, slots, slot_mut, slot_type, n)?;
                    collect_nested_decls(b, slots, slot_mut, slot_type, n)?;
                }
                if let Some(eb) = else_body {
                    collect_nested_decls(eb, slots, slot_mut, slot_type, n)?;
                }
            }
            Stmt::While { cond, body } => {
                collect_expr_decls(cond, slots, slot_mut, slot_type, n)?;
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Match { subject, arms, .. } => {
                collect_expr_decls(subject, slots, slot_mut, slot_type, n)?;
                for arm in arms {
                    collect_nested_decls(&arm.body, slots, slot_mut, slot_type, n)?;
                }
            }
            Stmt::For { targets, iter, body } => {
                // ループ変数は可変（tree-walk は Var::new(item, true)）。型注釈なし。
                for t in targets {
                    add_decl(t, &None, true, slots, slot_mut, slot_type, n)?;
                }
                collect_expr_decls(iter, slots, slot_mut, slot_type, n)?;
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
            }
            // `block: <stmts>` 文の中の宣言も slot を持つ（#27-c）。
            // ⚠ ここが抜けていたため、最上位の `block:` 内の `let` が slot に載らず
            // 「slot にもグローバルにも無い識別子」として bail していた。
            // `block_body_bails` は元から `Stmt::Block` を降りており、**2 つの walker が不整合**だった。
            Stmt::Block(b) => collect_nested_decls(b, slots, slot_mut, slot_type, n)?,
            Stmt::Raise { exc: Some(e), .. } => {
                collect_expr_decls(e, slots, slot_mut, slot_type, n)?
            }
            Stmt::Try { body, handlers, finally_body } => {
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
                for h in handlers {
                    // `except E as e:` の別名は不変束縛（tree-walk は Var::new(exc, false)）。
                    if let Some(alias) = &h.name {
                        add_decl(alias, &None, false, slots, slot_mut, slot_type, n)?;
                    }
                    collect_nested_decls(&h.body, slots, slot_mut, slot_type, n)?;
                }
                if let Some(fb) = finally_body {
                    collect_nested_decls(fb, slots, slot_mut, slot_type, n)?;
                }
            }
            // その他（未対応構文）には踏み込まない。compile_stmt が到達時に bail する。
            _ => {}
        }
    }
    Some(())
}

/// 式の中の**ブロック式**（`block:`/if/while/for/match 式）の本体宣言に slot を割り当てる（再帰）。
/// ブロック式でない部分式も辿り、入れ子のブロック式を漏れなく採番する。
fn collect_expr_decls(
    e: &Expr,
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    macro_rules! rec {
        ($x:expr) => {
            collect_expr_decls($x, slots, slot_mut, slot_type, n)?
        };
    }
    match e {
        // ── ブロック式（本体に宣言を持つ） ──
        Expr::Block { stmts, .. } => collect_nested_decls(stmts, slots, slot_mut, slot_type, n)?,
        Expr::IfExpr { branches, else_body, .. } => {
            for (c, b) in branches {
                rec!(c);
                collect_nested_decls(b, slots, slot_mut, slot_type, n)?;
            }
            if let Some(eb) = else_body {
                collect_nested_decls(eb, slots, slot_mut, slot_type, n)?;
            }
        }
        Expr::MatchExpr { subject, arms, .. } => {
            rec!(subject);
            for arm in arms {
                collect_nested_decls(&arm.body, slots, slot_mut, slot_type, n)?;
            }
        }
        Expr::ForExpr { target, iter, body, .. } => {
            add_decl(target, &None, true, slots, slot_mut, slot_type, n)?;
            rec!(iter);
            collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
        }
        Expr::WhileExpr { cond, body, .. } => {
            rec!(cond);
            collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
        }
        // ── 部分式を辿る（入れ子のブロック式を探す） ──
        Expr::BinOp { left, right, .. } => {
            rec!(left);
            rec!(right);
        }
        Expr::UnaryOp { operand, .. } => rec!(operand),
        Expr::Call { func, args, .. } => {
            rec!(func);
            for a in args {
                match a {
                    CallArg::Positional(x) | CallArg::Keyword { value: x, .. } => rec!(x),
                    CallArg::Variadic(xs) => {
                        for x in xs {
                            rec!(x);
                        }
                    }
                }
            }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => rec!(object),
        Expr::Subscript { object, index, .. } => {
            rec!(object);
            rec!(index);
        }
        Expr::Slice { begin, end, step } => {
            for x in [begin, end, step].into_iter().flatten() {
                rec!(x);
            }
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for x in items {
                rec!(x);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                rec!(k);
                rec!(v);
            }
        }
        Expr::TemplateInstantiate { base, .. } => rec!(base),
        Expr::IsType { expr, .. } | Expr::MustBe { expr, .. } => rec!(expr),
        Expr::Cast { object, .. } => rec!(object),
        // リテラル・Ident 等は宣言を含まない。
        _ => {}
    }
    Some(())
}

/// `stmts` に「囲む try を飛び越える」制御フローがあるかを保守的に判定する。
/// `include_return` が真なら `return` も脱出とみなす（finally は return でも走る必要があるため）。
/// `break`/`continue` は `stmts` 内の while/for（loop_depth>0）に囲まれていなければ脱出。
/// `block_return`/`loop_yield` は常に脱出とみなす。
/// `finally` 本体の複製がネストできる上限（#40）。経路ごとに複製されるので、
/// これを超えるとコード量が指数的に膨らむ。超えたら bail してツリーウォークへ落とす。
const MAX_FINALLY_NEST: usize = 4;

/// ブロック式の本体が VM コンパイル不能な脱出を含むかを判定する。
/// `return` は常に不可（ブロック式内 return は構文エラー）。
/// `block_return`/`loop_yield` は当該ブロック式が扱うので許容。
///
/// ⚠ **`break`/`continue` はここでは見ない**（#34）。跳び先も、跳ぶ前に捨てるオペランド数も
/// `Stmt::Break` のコンパイル時に `loops` / `stmt_base` から確定するので、
/// **同じ木を歩く 2 つ目の walker を持たない**（ずれの温床になる）。
fn block_body_bails(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Return(_) => true,
        Stmt::While { body, .. } | Stmt::For { body, .. } => block_body_bails(body),
        Stmt::If { branches, else_body } => {
            branches.iter().any(|(_, b)| block_body_bails(b))
                || else_body.as_ref().is_some_and(|eb| block_body_bails(eb))
        }
        Stmt::Match { arms, .. } => arms.iter().any(|a| block_body_bails(&a.body)),
        Stmt::Block(b) => block_body_bails(b),
        Stmt::Try { body, handlers, finally_body } => {
            block_body_bails(body)
                || handlers.iter().any(|h| block_body_bails(&h.body))
                || finally_body.as_ref().is_some_and(|fb| block_body_bails(fb))
        }
        _ => false,
    })
}

impl Compiler {
    #[inline]
    fn emit(&mut self, op: Op) -> usize {
        self.code.push(op);
        // 行テーブルを code と 1:1 に保つ（#1）。`compile_stmt` が予約していれば
        // **この op が文の先頭**なので、そこに文の位置を記録する。
        self.stmt_spans
            .push(self.pending_stmt.take().unwrap_or(super::chunk::NOT_STMT));
        self.code.len() - 1
    }

    /// スタック規律の一時 slot を確保する（match サブジェクト等）。名前付き slot の上に積む。
    /// `free_temp` と対で使う。フレーム総 slot 数（`n_locals`）を必要に応じて拡張する。
    fn alloc_temp(&mut self) -> Option<u16> {
        let Some(slot) = self.named_locals.checked_add(self.temps_in_use) else {
            bail("temp-slot-overflow", None);
            return None;
        };
        self.temps_in_use = self.temps_in_use.checked_add(1)?;
        let total = self.named_locals as usize + self.temps_in_use as usize;
        if total > self.n_locals {
            self.n_locals = total;
        }
        Some(slot)
    }

    fn free_temp(&mut self) {
        self.temps_in_use -= 1;
    }

    fn add_const(&mut self, v: Value) -> u32 {
        let idx = self.consts.len() as u32;
        self.consts.push(v);
        idx
    }

    /// 式が「単純なローカル読み」なら slot を返す（超命令の融合判定, #2）。
    /// `LoadLocal` に落ちる形（`Resolution::Local` / slot 表に載る未解決 `Ident`）のみ。
    /// debug_mode では融合しない（未解決 `Ident` は `LoadName` に落ちるため）。
    /// `Resolution::Global` は対象外（`_` に落ちる）。
    fn as_local(&self, e: &Expr) -> Option<u16> {
        if self.debug_mode {
            return None;
        }
        match e {
            // `static mut` / セル変数は slot を持たない（#27-d）。融合の対象外。
            Expr::Ident { name, .. }
                if self.statics.contains_key(name) || self.cells.contains_key(name) =>
            {
                None
            }
            Expr::Ident { res: Resolution::Local(slot), .. } => u16::try_from(*slot)
                .ok()
                .filter(|s| !self.cell_by_slot.contains_key(s)),
            Expr::Ident { name, res: Resolution::Unresolved, .. } => self.slots.get(name).copied(),
            _ => None,
        }
    }

    /// 式が数値/真偽リテラルなら定数値を返す（超命令の融合判定, #2）。
    fn as_const_lit(e: &Expr) -> Option<Value> {
        match e {
            Expr::Int(n) => Some(Value::Int(*n)),
            Expr::Float(f) => Some(Value::Float(*f)),
            Expr::Bool(b) => Some(Value::Bool(*b)),
            _ => None,
        }
    }

    /// 局所変数の型注釈から二項演算の特化種別を導出する（#16 段階 E）。
    ///
    /// 両オペランドが**同一プリミティブの型注釈を持つ局所変数**のときだけ種別を返す。
    /// リテラル（`Expr::Int`/`Expr::Float`）は型が自明なので相方に合わせて認める。
    /// テンプレート実体化後の関数のように「AST には具体型が書かれているが注釈テーブルは
    /// 原型を指している」ケースを拾うのが目的。
    fn local_operand_kind(
        &self,
        left: &Expr,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        let (l, r) = (self.expr_prim(left)?, self.expr_prim(right)?);
        // 両方がリテラルなら特化しても意味が無い（定数同士）。
        if matches!(left, Expr::Int(_) | Expr::Float(_))
            && matches!(right, Expr::Int(_) | Expr::Float(_))
        {
            return None;
        }
        Self::pair_kind(l, r)
    }

    /// 左辺が **slot 番号で与えられる** 場合の特化種別（複合代入 `x <op>= e` 用・#2b）。
    /// `local_operand_kind` の左辺を「式」から「slot」に替えただけで、判断基準は同一。
    fn slot_operand_kind(
        &self,
        slot: u16,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        Self::pair_kind(self.slot_prim(slot)?, self.expr_prim(right)?)
    }

    /// 式のプリミティブ型名。局所変数は型注釈から、数値リテラルは自明な型から。
    fn expr_prim(&self, e: &Expr) -> Option<&'static str> {
        match e {
            Expr::Int(_) => Some("int"),
            Expr::Float(_) => Some("float"),
            _ => self.slot_prim(self.as_local(e)?),
        }
    }

    /// 局所 slot のプリミティブ型名（型注釈が int/float のときのみ）。
    fn slot_prim(&self, slot: u16) -> Option<&'static str> {
        match self.slot_type.get(slot as usize)?.as_deref()? {
            "int" => Some("int"),
            "float" => Some("float"),
            _ => None,
        }
    }

    /// 注釈テーブルが焼いた**結果型**からプリミティブ型名を引く（#2b）。
    /// `slot_type`（AST に書かれた型注釈）では届かないノード（属性読みなど）用。
    fn annot_prim(&self, node_id: u32) -> Option<&'static str> {
        match self.annotations.resolved_type(node_id)? {
            crate::type_check::InferredType::Int => Some("int"),
            crate::type_check::InferredType::Float => Some("float"),
            _ => None,
        }
    }

    /// 両オペランドのプリミティブ型名が一致していれば特化種別に落とす。
    fn pair_kind(l: &str, r: &str) -> Option<crate::type_check::BinOperandKind> {
        use crate::type_check::BinOperandKind as K;
        if l != r {
            return None;
        }
        match l {
            "int" => Some(K::Int),
            "float" => Some(K::Float),
            _ => None,
        }
    }

    /// 注釈が「両オペランド int/float 確定」かつ型特化してよい op なら、その種別を返す（#16 段階(b)）。
    ///
    /// 許可する op は種別ごとに違う。`apply_binop` に対応するアームが存在するものだけを特化し、
    /// それ以外（float の `//`・`%` など）は汎用パスに委ねてエラー処理を一箇所に保つ。
    /// ゼロ除算は特化側が `None` を返して汎用へ落ちるので、op としては許可してよい。
    fn specialized_bin_kind(
        &self,
        op: &BinOp,
        node_id: u32,
        left: &Expr,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        let kind = match self.annotations.binop_kind(node_id) {
            Some(k) => k,
            // 注釈が無いときは**局所変数の型注釈**から導出する（#16 段階 E）。
            //
            // テンプレート実体化では `subst` が param の型注釈を具体型へ置き換える
            // （`fn add[T](a: T, b: T)` → `add[int]` なら `a: int, b: int`）が、
            // node-id は原型からコピーされるため注釈テーブルは**型変数のままの原型**を指す。
            // そこで実体化後の AST に書かれている型注釈を直接見る。
            // 注釈テーブルが届かない箇所（import 先モジュール等）にも同じ理由で効く。
            //
            // 特化 op は実行時型が想定外なら汎用へフォールバックするので、
            // この導出が外れていても**結果は変わらない**（速度の無駄が出るだけ）。
            None => self.local_operand_kind(left, right)?,
        };
        Self::gate_bin_kind(kind, op)
    }

    /// 左辺が **slot 番号で与えられる** 場合の特化種別（複合代入 `x <op>= e` 用・#2b）。
    /// 注釈の引き方・op の許可判定は `specialized_bin_kind` と同一で、
    /// 型導出のフォールバックだけが slot 版になる。
    /// 注釈テーブルだけから二項演算の種別を引く（#10-b のグローバル複合代入用）。
    /// slot 版（`specialized_bin_kind_slot`）の「slot の型注釈から推す」経路はグローバルには
    /// 使えないので、型検査が焼いた `binop_kind` のみを見る。
    fn annot_binop_kind(&self, node_id: u32) -> Option<crate::type_check::BinOperandKind> {
        // op のゲートは呼び出し側が渡す op で行う（`gate_bin_kind`）。ここでは種別だけ返す。
        self.annotations.binop_kind(node_id)
    }

    /// メソッド呼び出しの表示名と位置を副表へ記録する（#27-b・FFI 境界検査のメッセージ用）。
    ///
    /// ツリーウォークの `callee_display_name` と**同じ規則**で作ること
    /// （`obj.attr` 形は `base.attr`、それ以外は `attr`）。ずれると off/auto で
    /// エラーメッセージが食い違う（`ffi_boundary_check_error.ar` が検出した）。
    fn record_ffi_call_info(
        &mut self,
        node_id: u32,
        object: &Expr,
        attr: &str,
        span: &crate::token::Span,
    ) {
        if node_id == 0 {
            return; // 未採番（合成 AST 等）は検査キーが引けない
        }
        let display = match object {
            Expr::Ident { name, .. } => format!("{name}.{attr}"),
            _ => attr.to_string(),
        };
        let ni = self.add_name(&display);
        let si = self.add_span(span);
        self.ffi_call_info.insert(node_id, (ni, si));
    }

    /// 入れ子 `fn` がキャプチャする外側ローカル（名前, slot）を求める（#27）。
    ///
    /// `capture_env` と**同じ自由変数の定義**（参照 − 自前の名前）を使い、現在の `slots`
    /// （＝外側関数のローカル）と交わるものを返す。交わらなければ空 Vec（キャプチャなし）。
    /// ⚠ **`capture_env` と定義がずれると閉包変数が黙って消える**。片方を変えたら両方見ること。
    ///
    /// **可変ローカルを 1 つでも掴むなら `None`**。ツリーウォークはそこで `Var::Cell` へ昇格して
    /// 外側と共有するが、VM のフラット slot（`Value` 直値）では共有セルを表現できない。
    ///
    /// 返す順序は **slot 昇順**（決定的。`captured_env` は `HashMap` なので順序は挙動に影響しないが、
    /// Chunk が実行ごとに変わらないようにするため）。
    #[allow(clippy::type_complexity)]
    fn nested_fn_captures(
        &mut self,
        params: &[Param],
        body: &[Stmt],
    ) -> Option<(Vec<(String, u16)>, Vec<(String, u16)>, Vec<(String, u32)>)> {
        use std::collections::HashSet;
        let mut own: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        crate::interpreter::collect_declared_names(body, &mut own);
        let mut referenced: HashSet<String> = HashSet::new();
        crate::interpreter::collect_referenced_names(body, &mut referenced);

        let mut caps: Vec<(String, u16)> = Vec::new();
        let mut cell_caps: Vec<(String, u16)> = Vec::new();
        let mut static_caps: Vec<(String, u32)> = Vec::new();
        // `add_span` が `&mut self` を要るので、走査順を安定させるためソートしてから回す。
        let mut referenced: Vec<&String> = referenced.iter().collect();
        referenced.sort();
        for n in referenced {
            if own.contains(n) {
                continue;
            }
            // `static mut` のキャプチャ（#27-d 段階 2b）。セルは `Interpreter::static_cells` に
            // あるので、span を運んで実行時に共有する（値のコピーでは書き戻りが消える）。
            if let Some(span) = self.statics.get(n).cloned() {
                let si = self.add_span(&span);
                static_caps.push((n.clone(), si));
                continue;
            }
            // 外側フレームのセル変数のキャプチャ（#27-d 段階 2b）。セル index をそのまま渡す。
            if let Some(&i) = self.cells.get(n) {
                cell_caps.push((n.clone(), i));
                continue;
            }
            let Some(&slot) = self.slots.get(n) else {
                continue; // 外側ローカルでない（グローバル等）＝キャプチャ対象外
            };
            if self.slot_mut.get(slot as usize).copied().unwrap_or(true) {
                // 可変ローカルのキャプチャ。セル化は `mut_captured_by_nested_fn` の
                // 事前解析が担うので、ここへ来るのは解析漏れ（保守的に諦める）。
                return None;
            }
            caps.push((n.clone(), slot));
        }
        caps.sort_by_key(|(_, s)| *s);
        cell_caps.sort_by_key(|(_, i)| *i);
        static_caps.sort_by(|a, b| a.0.cmp(&b.0));
        Some((caps, cell_caps, static_caps))
    }

    /// 最上位宣言の名前なら name プールの index を返す（#10-c）。
    ///
    /// 条件は 2 つ: **最上位モードであること**（`toplevel_globals` が非空）と、
    /// **その名前が slot に無いこと**。後者が要るのは、最上位文の内側（ループ本体・
    /// ブロック式の中）の宣言は slot だから — そちらは従来どおり `StoreLocal*` に落とす。
    fn toplevel_decl_name(&mut self, name: &str) -> Option<u32> {
        if self.toplevel_globals.is_empty() || self.slots.contains_key(name) {
            return None;
        }
        Some(self.add_name(name))
    }

    /// 変数への書き込み先を決める（#10-b）。
    ///
    /// 1. VM の slot にある名前 → `Local`（関数本体でもブロック内宣言でもここに来る）
    /// 2. 最上位モードで可視グローバルと確定できる名前 → `Global`
    /// 3. どちらでもない → `None`（＝ツリーウォークへフォールバック）
    ///
    /// ⚠ 順序が重要。ループ本体の `let` は毎回スコープに入る**ローカル**なので、
    /// 同名グローバルより先に slot を見なければならない。
    /// `slots` 引きの失敗を**必ず計上する**（#27-c）。
    ///
    /// ⚠ 素の `self.slot_of(name)?` は「未帰属」として計測から消える。
    /// 宣言文のコンパイルはここを通してから諦めること
    /// （`For/unattributed:For` の出所がここだった）。
    fn slot_of(&self, name: &str) -> Option<u16> {
        // ⚠ `static mut` / セル変数は slot を持たない（#27-d）。ここへ来る経路（for ターゲット・
        // `let a,b = t`・`except as`・入れ子 `fn` の格納先）は共有セルを扱えないので諦める。
        if self.cells.contains_key(name) {
            if crate::interpreter::tw_stats::enabled() {
                crate::interpreter::tw_stats::record_bail("cell-as-slot", name);
            }
            return None;
        }
        if self.statics.contains_key(name) {
            if crate::interpreter::tw_stats::enabled() {
                crate::interpreter::tw_stats::record_bail("static-as-slot", name);
            }
            return None;
        }
        match self.slots.get(name) {
            Some(&s) => Some(s),
            None => {
                if crate::interpreter::tw_stats::enabled() {
                    crate::interpreter::tw_stats::record_bail("decl-no-slot", name);
                }
                None
            }
        }
    }

    fn store_target(&mut self, name: &str) -> Option<StoreTarget> {
        // セル変数は slot ではなく共有セル（#27-d 段階 2b）。**slot より先に見る**。
        if let Some(&i) = self.cells.get(name) {
            return Some(StoreTarget::Cell(i));
        }
        // `static mut` も slot ではなく共有セル（#27-d）。**slot より先に見る**。
        if let Some(span) = self.statics.get(name).cloned() {
            let si = self.add_span(&span);
            return Some(StoreTarget::Static(si));
        }
        if let Some(&slot) = self.slots.get(name) {
            return Some(StoreTarget::Local(slot));
        }
        // ⚠ デバッガ REPL（`compile_debug`）だけは例外（#39）。停止フレームの**生スコープ**へ
        // 書かねばならず、`scopes[0]` 限定の `StoreGlobal` では別の変数を書いてしまう。
        // 読み側が `LoadName` に落ちているのと同じ理由。ここは従来どおり bail する。
        if self.debug_mode || self.name_lookup {
            // 停止フレーム／外側スコープの**生の変数**へ書く必要があるが、
            // `scopes[0]` 限定の `StoreGlobal` では別の変数を書いてしまう（#39）。
            // ⚠ チャンク内で宣言したローカルは上の `slots` で先に拾われるので影響しない。
            bail("store-target-name-lookup", None);
            return None;
        }
        // ここまで全部外れた名前は**この関数のローカルでもキャプチャでもない**（#39）。
        //
        // 根拠は `Op::LoadGlobal` を関数本体で使うのと同じ（#27）: base slot の採番と
        // `collect_nested_decls` が本体の全宣言を**先に** `slots` へ入れ、可変キャプチャは
        // `capture_env` が作った集合ごと `cells` に入る。つまり `slots`/`cells`/`statics` を
        // 引いて外れた名前は、ツリーウォークの `assign_var` でもローカル走査を必ず素通りして
        // グローバル分岐へ落ちる。⇒ `scopes[0]` へ書く `StoreGlobal` と答えが一致する。
        //
        // ⚠ **最上位で宣言されているか（`toplevel_globals`）は条件にしない**。未宣言の名前は
        // `vm_assign_global` が `NameError: '<name>' is not defined` を返し、これも
        // ツリーウォークと同一文言（以前はここで bail し、関数本体からのグローバル代入が
        // 丸ごと `VmForceError` になっていた）。
        let ni = self.add_name(name);
        // `LoadGlobal` と同じく emit 1 回につきキャッシュ枠を 1 本割り当てる
        // （枠は共有しない。op ごとに焼く index の意味が違うため — `Op::StoreGlobal` 参照）。
        let ci = self.global_caches.len() as u32;
        self.global_caches.push(crate::ast::SlotCache::default());
        Some(StoreTarget::Global(ni, ci))
    }

    fn specialized_bin_kind_slot(
        &self,
        op: &BinOp,
        node_id: u32,
        slot: u16,
        right: &Expr,
    ) -> Option<crate::type_check::BinOperandKind> {
        let kind = match self.annotations.binop_kind(node_id) {
            Some(k) => k,
            None => self.slot_operand_kind(slot, right)?,
        };
        Self::gate_bin_kind(kind, op)
    }

    /// 特化してよい op かを判定する（種別ごとの許可リスト）。
    /// `specialized_bin_kind` / `specialized_bin_kind_slot` の共通判断。
    fn gate_bin_kind(
        kind: crate::type_check::BinOperandKind,
        op: &BinOp,
    ) -> Option<crate::type_check::BinOperandKind> {
        use crate::type_check::BinOperandKind as K;
        let allowed = match kind {
            // int/int は `apply_binop` の Int/Int アームを全て特化できる。
            K::Int => matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::FloorDiv
                    | BinOp::Mod
                    | BinOp::Pow
                    | BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::BitXor
                    | BinOp::LShift
                    | BinOp::RShift
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq
                    | BinOp::Eq
                    | BinOp::NotEq
            ),
            // float は `//`・`%`・ビット演算のアームが無いので算術と比較のみ。
            K::Float => matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Pow
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq
                    | BinOp::Eq
                    | BinOp::NotEq
            ),
        };
        if allowed {
            Some(kind)
        } else {
            None
        }
    }

    /// 二項演算 `left <op> right` を超命令へ融合できれば emit して `true`（#2 ＋ plan A 型特化）。
    /// `local <op> local` → `BinLocalLocal`、`local <op> リテラル` → `BinLocalConst`。
    /// さらに注釈が「両オペランド int/float 確定」かつ対応 op（Add/Sub/Mul・比較）なら**型特化 op**
    /// （`IntBinLL`/`FloatBinLC` 等）を emit（タグ検査・op ディスパッチ・clone を削減）。
    /// 型特化 op は実行時型が想定外なら**汎用へフォールバック**するので、注釈が古くても健全。
    /// 融合できなければ `false`（呼び出し側が通常経路 `LoadLocal…; Bin` を出す）。意味論は不変。
    fn try_emit_bin_fused(&mut self, left: &Expr, right: &Expr, op: &BinOp, node_id: u32) -> bool {
        let Some(a) = self.as_local(left) else {
            return false;
        };
        let kind = self.specialized_bin_kind(op, node_id, left, right);
        self.emit_bin_fused_slot(a, kind, right, op)
    }

    /// 左辺が slot と確定している二項演算を 1 命令へ融合 emit する（`try_emit_bin_fused` の中核）。
    /// 右辺が局所変数なら `*BinLL`、定数リテラルなら `*BinLC`。どちらでもなければ `false` を返し、
    /// 呼び出し側が通常経路（オペランドを積んでから `Bin`/`*BinSS`）を出す。
    ///
    /// **評価順について**: 融合後はスタックへ積まず frame から左辺を読むので、形の上では
    /// 「右辺を用意してから左辺を読む」順になる。ただし融合する右辺は局所変数読みか定数
    /// リテラルのみで**副作用が無い**ため、観測される値は融合前と同一（`CallMethodLocal` と同じ理由）。
    fn emit_bin_fused_slot(
        &mut self,
        a: u16,
        kind: Option<crate::type_check::BinOperandKind>,
        right: &Expr,
        op: &BinOp,
    ) -> bool {
        use crate::type_check::BinOperandKind as K;
        if let Some(b) = self.as_local(right) {
            match kind {
                Some(K::Int) => self.emit(Op::IntBinLL(a, b, op.clone())),
                Some(K::Float) => self.emit(Op::FloatBinLL(a, b, op.clone())),
                None => self.emit(Op::BinLocalLocal(a, b, op.clone())),
            };
            true
        } else if let Some(cv) = Self::as_const_lit(right) {
            let ci = self.add_const(cv);
            match kind {
                Some(K::Int) => self.emit(Op::IntBinLC(a, ci, op.clone())),
                Some(K::Float) => self.emit(Op::FloatBinLC(a, ci, op.clone())),
                None => self.emit(Op::BinLocalConst(a, ci, op.clone())),
            };
            true
        } else {
            false
        }
    }

    /// `LoadGlobal` を index キャッシュ付きで emit する（#11）。name プールとキャッシュ枠を確保。
    fn emit_load_global(&mut self, name: &str) {
        let ni = self.add_name(name);
        let ci = self.global_caches.len() as u32;
        self.global_caches.push(crate::ast::SlotCache::default());
        self.emit(Op::LoadGlobal(ni, ci));
    }

    /// スタック上位 2 値への二項演算を、型特化つきで emit する（#2b）。
    /// `kind` が決まらなければ動的ディスパッチの `Bin`。属性複合代入の 2 経路で共有する。
    fn emit_bin_specialized(&mut self, kind: Option<crate::type_check::BinOperandKind>, op: &BinOp) {
        match kind {
            Some(crate::type_check::BinOperandKind::Int) => self.emit(Op::IntBinSS(op.clone())),
            Some(crate::type_check::BinOperandKind::Float) => self.emit(Op::FloatBinSS(op.clone())),
            None => self.emit(Op::Bin(op.clone())),
        };
    }

    fn add_name(&mut self, name: &str) -> u32 {
        let idx = self.names.len() as u32;
        self.names.push(name.to_string());
        self.attr_caches.push(crate::ast::AttrCache::default());
        idx
    }

    /// AST 型解決層の**検査指示**（`CheckBefore`）を消費する（#16 段階(b)(ii)）。
    ///
    /// `mustbe` / `=>` は型検査が常に `CheckBefore` を付けるので、現状これは実質いつも `true` を返す。
    /// それでも指示を経由するのは、将来チェッカが「この検査は静的に冗長」と証明できるようになったとき、
    /// **この一点を変えるだけで VM とネイティブの双方が検査を落とせる**ようにするため（＝解決の一元化）。
    ///
    /// 指示が `None`（未採番ノード・合成 AST・モジュール横断で注釈が無い）の場合は
    /// **検査が要るのか判らない**ので、その関数の VM 化自体を諦める（`false`）。
    /// 検査を省く方向へは決して倒さない。
    fn check_required(&self, node_id: u32) -> bool {
        matches!(
            self.annotations.directive(node_id),
            crate::type_check::Directive::CheckBefore(_)
        )
    }

    fn add_span(&mut self, span: &crate::token::Span) -> u32 {
        let idx = self.spans.len() as u32;
        self.spans.push(span.clone());
        idx
    }

    /// 次に emit する op を「文の先頭」として予約する（#1・行テーブル）。
    ///
    /// ツリーウォークは `exec()` の冒頭で**すべての文**について `should_pause_at` を呼ぶので、
    /// VM も**すべての文**の先頭を記録しないと停止位置が食い違う。
    /// 位置情報を持たない種類の文（`if`/`while`/`return` 等）は `STMT_NO_SPAN` を記録し、
    /// 表示スパンは `best_span_for` のフォールバック（`dbg_last_span`）に委ねる。
    ///
    /// ⚠ `debug_mode`（デバッガ REPL 用の `compile_debug`）では記録しない。
    /// あちらは停止対象ではなく、REPL 入力を評価するだけの Chunk。
    fn mark_stmt_start(&mut self, stmt: &Stmt) {
        if self.debug_mode {
            return;
        }
        let idx = match crate::interpreter::debugger::stmt_span_of(stmt) {
            Some(span) => self.add_span(&span),
            None => super::chunk::STMT_NO_SPAN,
        };
        self.pending_stmt = Some(idx);
    }

    /// バックパッチ用: 直後に置く命令の index を現在位置として返す。
    #[inline]
    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    fn patch_jump(&mut self, at: usize, target: u32) {
        self.code[at] = match &self.code[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            Op::JumpIfFalseOrPop(_) => Op::JumpIfFalseOrPop(target),
            Op::JumpIfTrueOrPop(_) => Op::JumpIfTrueOrPop(target),
            _ => unreachable!("patch_jump on non-jump op"),
        };
    }

    // ⚠ **レシーバが Arrow インスタンスかを判定する仕組みは撤去した**（#27）。
    // `object_is_instance` / `is_arrow_instance_type` / `annot_is_arrow_instance` /
    // `is_user_instance_type`（#26・#27-a の「形＋出自」2 段判定）は、属性複合代入の
    // レシーバ制限が最後の消費者だった。読み書きが `get_attr_val` / `attr_assign_evaled`
    // へ一本化された今、どの op も `Value::Instance` を前提にしていないので判定自体が不要。
    // ⚠ **`Value::Instance` を前提とする op を新設するなら判定を復活させること**。
    // 型検査の `NamedInstance` は外部言語スタブのクラス（実行時 `Value::CsObject`）も
    // 同じ注釈になるので、`annotations.is_arrow_class`（出自）まで見る必要がある。

    /// 呼び出し引数の `is_mutable`（`eval_call_args` と同じ判定: 変数 ident は変数の可変性、
    /// それ以外の式は保守的に true）。VM は base ローカルしか読まないので slot_mut で判定できる。
    fn arg_is_mutable(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident { res: Resolution::Local(slot), .. } => {
                self.slot_mut.get(*slot as usize).copied().unwrap_or(true)
            }
            Expr::Ident { name, res: Resolution::Unresolved, .. } => self
                .slots
                .get(name)
                .and_then(|&s| self.slot_mut.get(s as usize).copied())
                .unwrap_or(true),
            _ => true,
        }
    }

    /// 位置引数をスタックへ push し、各引数の is_mutable を bit にした mask を返す。
    /// keyword/可変長引数・33個以上は非対応（`None`）。
    /// 呼び出し引数をスタックへ積み、`(mut_mask, 引数名)` を返す（#27-c）。
    ///
    /// 引数名が全て `None`（＝純粋な位置引数）なら 2 番目は `None` を返し、呼び出し側は
    /// `Op::Call` を使う。1 つでも名前付き・可変長があれば `Some(names)` になり、
    /// **`Op::CallKw` を使える呼び出し形でだけ**受け付ける（それ以外は `no_kw` で bail）。
    ///
    /// 可変長 `f(... = A, B, C)` は要素を積んでから `BuildList` で 1 値に畳む。
    /// `eval_call_args` が作る `(Some("..."), Value::List, true)` と同じ形。
    #[allow(clippy::type_complexity)]
    fn compile_call_args(
        &mut self,
        args: &[CallArg],
    ) -> Option<(u32, Option<Vec<Option<String>>>)> {
        if args.len() > 32 {
            bail("too-many-args", None);
            return None;
        }
        let mut mask: u32 = 0;
        let mut names: Vec<Option<String>> = Vec::new();
        let mut any_named = false;
        for (i, arg) in args.iter().enumerate() {
            match arg {
                CallArg::Positional(e) => {
                    if self.arg_is_mutable(e) {
                        mask |= 1 << i;
                    }
                    self.compile_expr(e)?;
                    names.push(None);
                }
                CallArg::Keyword { name, value } => {
                    if self.arg_is_mutable(value) {
                        mask |= 1 << i;
                    }
                    self.compile_expr(value)?;
                    names.push(Some(name.clone()));
                    any_named = true;
                }
                CallArg::Variadic(exprs) => {
                    let n = u16::try_from(exprs.len()).ok()?;
                    for e in exprs {
                        self.compile_expr(e)?;
                    }
                    self.emit(Op::BuildList(n));
                    mask |= 1 << i; // variadic は保守的に mutable 扱い（ツリーウォークと同じ）
                    names.push(Some("...".to_string()));
                    any_named = true;
                }
            }
        }
        Some((mask, any_named.then_some(names)))
    }

    /// `Op::CallKw` を使えない呼び出し形で名前付き引数が来たら bail する（#27-c）。
    fn no_kw(kw: Option<Vec<Option<String>>>) -> Option<()> {
        if kw.is_some() {
            bail("call-arg", None);
            return None;
        }
        Some(())
    }

    /// 通常の呼び出しを発行する（#27-c）。名前付き引数の有無で `Call` / `CallKw` を選ぶ。
    fn emit_call(
        &mut self,
        argc: usize,
        mask: u32,
        name_idx: u32,
        span_idx: u32,
        node_id: u32,
        kw: Option<Vec<Option<String>>>,
    ) -> Option<()> {
        match kw {
            None => self.emit(Op::Call(argc as u16, mask, name_idx, span_idx, node_id)),
            Some(arg_names) => {
                let i = u32::try_from(self.kw_calls.len()).ok()?;
                self.kw_calls.push(crate::vm::chunk::KwCall {
                    argc: u16::try_from(argc).ok()?,
                    mut_mask: mask,
                    name_idx,
                    span_idx,
                    node_id,
                    arg_names,
                });
                self.emit(Op::CallKw(i))
            }
        };
        Some(())
    }

    /// `target <- async->T: body` をコンパイルする（タスク #9）。
    /// 本体が参照する enclosing フレームの slot（`collect_referenced_names ∩ slots`）を捕捉対象に記録し、
    /// マネージャをスタックへロードして `AsyncSubmit(idx)` を発行する。実行時は frame から捕捉値を読み、
    /// グローバルと合わせて `capture_env` で env を組む（ツリーウォークの `exec_async_assign` と同一）。
    /// 捕捉は「本体が参照する slot」に限定（未参照ローカルは env に載せない）＝task 挙動は byte-identical。
    fn compile_async_assign(&mut self, target: &str, stmts: &[Stmt]) -> Option<()> {
        // 本体の参照名を収集し、enclosing frame の slot と交差したものを捕捉する。
        let mut refs: HashSet<String> = HashSet::new();
        crate::interpreter::collect_referenced_names(stmts, &mut refs);
        let mut captures: Vec<(String, u16, bool)> = refs
            .iter()
            .filter_map(|name| {
                // ⚠ ここは `slot_of` を使わない。**当たらないのが正常**（グローバル参照は
                // 捕捉対象外）で、`slot_of` を通すと計測に幻の bail が 35 件載る。
                // 「対象外」と「失敗」を同じ `None` で表さないこと（#27-c で再発）。
                let &slot = self.slots.get(name)?;
                let is_mut = self.slot_mut.get(slot as usize).copied().unwrap_or(false);
                Some((name.clone(), slot, is_mut))
            })
            .collect();
        // 決定的順序（HashSet は非決定）: slot 昇順。env の順序は task 挙動に影響しないが再現性のため固定。
        captures.sort_by_key(|(_, slot, _)| *slot);

        let idx = u32::try_from(self.async_blocks.len()).ok()?;
        self.async_blocks.push(crate::vm::chunk::AsyncBlock {
            body: stmts.to_vec(),
            captures,
        });
        // マネージャ値をスタックへ（ローカル slot 優先、なければグローバル名引き）。
        if let Some(&slot) = self.slots.get(target) {
            self.emit(Op::LoadLocal(slot));
        } else {
            self.emit_load_global(target);
        }
        self.emit(Op::AsyncSubmit(idx));
        Some(())
    }

    /// `break`/`continue` が外側ループへ跳ぶ前に、途中のブロック式が積んだオペランドを捨てる（#34）。
    ///
    /// Arrow の `break` は入れ子の `if`/`match`/`block:` **式**を貫通して外側ループへ届く。
    /// ブロック式は値をすべて temp slot に置くので、跳ぶ時点でオペランドスタックに残るのは
    /// **そのブロック式より外側の式が積んだ分**だけ（`let s = 1 + block ->int: … break …` の `1`）。
    /// その数が `stmt_base`。ループ入口は必ず深さ 0 基準なので、ここまで戻せば跳び先と平衡する。
    ///
    /// `None`（深さ不明）なら bail する。深さを伝えていない式の形は「壊れる」のではなく
    /// 「VM に載らない」で止まるので、伝播の書き漏らしは安全側に倒れる。
    /// ブロック式の `->T` アノテーションを名前プールへ入れて index を返す（#35）。
    /// 注釈が無ければ `None`（＝実行時検査を出さない＝ツリーウォークと同じ）。
    fn add_return_type(&mut self, return_type: &Option<String>) -> Option<u32> {
        return_type.as_ref().map(|t| self.add_name(t))
    }

    fn emit_unwind_to_loop(&mut self) -> Option<()> {
        let loop_try_len = self.loops.last().map_or(0, |l| l.try_len);
        // break/continue の経路では値を積んでいない（pops は finally の後に出す）。
        self.emit_unwind_tries(loop_try_len, true, 0)?;
        let Some(depth) = self.stmt_base else {
            bail("break-unknown-depth", None);
            return None;
        };
        for _ in 0..depth {
            self.emit(Op::Pop);
        }
        Some(())
    }

    /// 脱出が跨ぐ `try` を**内側から**巻き戻す（#34/#37）。
    ///
    /// - `try/except`: `PopTry` だけ（`pop_except` が真のとき）。⚠ `return` は `run` から
    ///   即復帰してハンドラごと捨てられるので不要 ＝ 既存 Chunk を変えないため偽を渡す。
    /// - `try/finally`: `PopTry` ＋ **`finally` 本体をこの経路にも複製**する。
    ///   ネストしていれば内側の finally から順に走る（ツリーウォーク・Python と同じ順）。
    ///
    /// `keep` は「跨がない外側の try の数」。break/continue は最内ループ入口、
    /// block_return は最内ブロック式入口、return は 0（全部）を渡す。
    fn emit_unwind_tries(&mut self, keep: usize, pop_except: bool, extra: u16) -> Option<()> {
        for i in (keep..self.try_stack.len()).rev() {
            let Some(fin) = self.try_stack[i].clone() else {
                if pop_except {
                    self.emit(Op::PopTry);
                }
                continue;
            };
            self.emit(Op::PopTry);
            // ⚠ 複製中は「巻き戻し済みの try」を見せない（同じ finally を二重に出さない）。
            // 外側の try は残るので、複製の中の `break` は**外側の finally を走らせてから**跳ぶ。
            let saved = self.try_stack.split_off(i);
            let ok = self.compile_finally_copy(&fin, extra);
            self.try_stack.extend(saved);
            ok?;
        }
        Some(())
    }

    /// `finally` 本体を**この経路のスタックの上に**複製する（#37/#40）。
    ///
    /// `extra` は「この複製が載っているオペランドの数」。例外経路は `[exc]`、`return` 経路は
    /// 戻り値が 1 つ下に積まれている。⚠ **`stmt_base` をその分だけ持ち上げる**ことで、
    /// 複製の中の `break`/`continue`（#40）が跳ぶときに**その値まで捨ててくれる**
    /// （＝ Python と同じ「保留中の動作を破棄する」意味論になる）。
    ///
    /// 複製は経路ごとに増えるので、入れ子が深いとコードが指数的に膨らむ。
    /// **`MAX_FINALLY_NEST` で頭打ちにして、それ以上は bail** する（ツリーウォークへ落とす）。
    fn compile_finally_copy(&mut self, fin: &[Stmt], extra: u16) -> Option<()> {
        if self.in_finally >= MAX_FINALLY_NEST {
            bail("finally-nest-limit", None);
            return None;
        }
        let saved_base = self.stmt_base;
        if extra > 0 {
            self.stmt_base = self.stmt_base.map(|d| d + extra);
        }
        self.in_finally += 1;
        let mut ok = true;
        for s in fin {
            if self.compile_stmt(s).is_none() {
                ok = false;
                break;
            }
        }
        self.in_finally -= 1;
        self.stmt_base = saved_base;
        if ok {
            Some(())
        } else {
            None
        }
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Option<()> {
        self.mark_stmt_start(stmt);
        // 文の入口はオペランドスタックが平衡（#34）。この文の**最初の**式にだけ深さを伝える。
        // `compile_expr` が `take()` するので 2 つ目以降の式は `None`（＝保守的に bail）になる。
        self.pending = self.stmt_base;
        match stmt {
            Stmt::Expr(e) => {
                self.compile_expr(e)?;
                self.emit(Op::Pop);
            }
            Stmt::Return(Some(e)) => {
                self.compile_expr(e)?;
                // #37: 開いている `finally` を**全部**走らせてから返す（内側から）。
                // ⚠ `try/except` の `PopTry` は不要（`run` から即復帰してハンドラごと捨てられる）。
                //    偽を渡すことで finally を持たない既存 Chunk は 1 命令も変わらない。
                // ⚠ 戻り値が 1 つ積まれた上で finally が走る（#40）。
                self.emit_unwind_tries(0, false, 1)?;
                self.emit(Op::Return);
            }
            Stmt::Return(None) => {
                self.emit_unwind_tries(0, false, 0)?;
                self.emit(Op::ReturnNil);
            }
            // パラメータ（mut）への代入。let への代入は型検査で弾かれるので健全。
            // 最上位モード（#10-b）では slot に無い名前は可視グローバルへの代入になる。
            Stmt::Assign { name, value, .. } => match self.store_target(name)? {
                StoreTarget::Local(slot) => {
                    self.compile_expr(value)?;
                    self.emit(Op::StoreLocal(slot));
                }
                StoreTarget::Global(ni, ci) => {
                    self.compile_expr(value)?;
                    self.emit(Op::StoreGlobal(ni, ci));
                }
                // `static mut` への代入（#27-d）。共有セルへ直接書く。
                StoreTarget::Static(si) => {
                    self.compile_expr(value)?;
                    self.emit(Op::StoreStatic(si));
                }
                // セル変数への代入（#27-d 段階 2b）。共有相手からも見える。
                StoreTarget::Cell(i) => {
                    self.compile_expr(value)?;
                    self.emit(Op::StoreCell(i));
                }
            },
            // `x <op>= e` は `x = x <op> e` と同じ命令列になる（`StoreLocal` は deep_copy しない）ので、
            // `Expr::BinOp` と同じ融合＋型特化を通す（#2b）。通さないと複合代入だけが
            // `LoadLocal; <e>; Bin; StoreLocal` の 4 命令＋汎用ディスパッチに落ちていた（実測 1.9x 遅い）。
            Stmt::CompoundAssign {
                name,
                op,
                value,
                node_id,
                ..
            } => {
                use crate::type_check::BinOperandKind as K;
                match self.store_target(name)? {
                    StoreTarget::Local(slot) => {
                        let kind = self.specialized_bin_kind_slot(op, *node_id, slot, value);
                        if !self.emit_bin_fused_slot(slot, kind, value, op) {
                            // 融合できない右辺（属性・添字・呼び出し結果など）でもスタック版の型特化には乗る。
                            self.emit(Op::LoadLocal(slot));
                            self.compile_expr(value)?;
                            match kind {
                                Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                                Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                                None => self.emit(Op::Bin(op.clone())),
                            };
                        }
                        self.emit(Op::StoreLocal(slot));
                    }
                    // 最上位のグローバルへの複合代入（#10-b）。`x = x <op> e` と同じ命令列。
                    // 融合 op（`BinLocalLocal` 等）は slot 前提なので使えないが、注釈由来の
                    // スタック版型特化（`IntBinSS`/`FloatBinSS`）はそのまま乗る（#2b と同じ扱い）。
                    StoreTarget::Global(ni, ci) => {
                        let kind = self.annot_binop_kind(*node_id);
                        self.emit_load_global(name);
                        self.compile_expr(value)?;
                        match kind {
                            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                            None => self.emit(Op::Bin(op.clone())),
                        };
                        self.emit(Op::StoreGlobal(ni, ci));
                    }
                    // `static mut` への複合代入（#27-d）。グローバル版と同じ形。
                    StoreTarget::Static(si) => {
                        let kind = self.annot_binop_kind(*node_id);
                        self.emit(Op::LoadStatic(si));
                        self.compile_expr(value)?;
                        match kind {
                            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                            None => self.emit(Op::Bin(op.clone())),
                        };
                        self.emit(Op::StoreStatic(si));
                    }
                    // セル変数への複合代入（#27-d 段階 2b）。`static` 版と同じ形。
                    StoreTarget::Cell(i) => {
                        let kind = self.annot_binop_kind(*node_id);
                        self.emit(Op::LoadCell(i));
                        self.compile_expr(value)?;
                        match kind {
                            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                            None => self.emit(Op::Bin(op.clone())),
                        };
                        self.emit(Op::StoreCell(i));
                    }
                }
            }
            Stmt::If { branches, else_body } => {
                // 各分岐: cond, JumpIfFalse(next), body, Jump(end); next: ...
                let mut end_jumps: Vec<usize> = Vec::new();
                for (cond, body) in branches {
                    self.compile_expr(cond)?;
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
                if let Some(body) = else_body {
                    for s in body {
                        self.compile_stmt(s)?;
                    }
                }
                let end = self.here();
                for j in end_jumps {
                    self.patch_jump(j, end);
                }
            }
            Stmt::While { cond, body } => {
                let start = self.here();
                self.compile_expr(cond)?;
                let jf = self.emit(Op::JumpIfFalse(0));
                // ループコンテキストを積む: continue はここ（条件先頭）へ戻る。
                self.loops.push(LoopCtx {
                    continue_target: start,
                    break_jumps: Vec::new(),
                    try_len: self.try_stack.len(),
                });
                // 本体はこのループ入口の深さで走る（#34）。
                let saved_base = self.stmt_base.replace(0);
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.stmt_base = saved_base;
                self.emit(Op::Jump(start));
                let end = self.here();
                self.patch_jump(jf, end);
                // break はループ末尾（end）へバックパッチ。
                let ctx = self.loops.pop().unwrap();
                for j in ctx.break_jumps {
                    self.patch_jump(j, end);
                }
            }
            Stmt::Match { subject, arms, .. } => {
                self.compile_match(subject, arms)?;
            }
            Stmt::For { targets, iter, body } => {
                if targets.is_empty() {
                    bail("for-no-target", None);
                    return None;
                }
                // 受け皿の temp が要るのは 2 通り（#27-c）:
                //  - 複数ターゲット `for k, v in ...` … 要素をいったん受けてから分解する
                //  - 捨てターゲット `for _ in ...`   … `_` は `add_decl` が slot を振らない
                //    （`let _ = e` は値を捨てるため）が、`ForIter` には**書き込み先が要る**
                // ⚠ **外側の同名束縛を覆うループ変数には専用 slot を割り当てる**（#27）。
                //
                // Arrow の `for` 変数はブロックスコープで、ツリーウォークは反復ごとに
                // スコープを push して宣言する（ループを抜けると外側の値が戻る）。
                // 名前ごとに 1 slot の flat モデルでこれを表現するため、**本体の間だけ**
                // `slots` の対応を temp slot へ差し替え、ループ後に元へ戻す。
                // これにより本体内の読みは temp を、ループ後の読みは元の slot を指す。
                //
                // 差し替えは `slot_of`（`target_slot` / `target_slots` の算出）より**前**に
                // 行う必要がある。temp は LIFO なので、解放は iter/sink より後（＝最後）。
                let mut shadow_saved: Vec<(String, u16)> = Vec::new();
                for t in targets {
                    if t != "_" && self.shadowed_for_targets.contains(t) {
                        let fresh = self.alloc_temp()?;
                        if let Some(old) = self.slots.insert(t.clone(), fresh) {
                            shadow_saved.push((t.clone(), old));
                        }
                    }
                }
                let unpack = targets.len() > 1;
                let sink_temp = if unpack || targets[0] == "_" {
                    Some(self.alloc_temp()?)
                } else {
                    None
                };
                let target_slot = match sink_temp {
                    Some(t) => t,
                    None => self.slot_of(&targets[0])?,
                };
                // 分解先 slot は**本体をコンパイルする前に**引いておく（`?` の早期 return で
                // temp の解放が漏れないようにするため）。`_` は捨てるので `None`。
                let mut target_slots: Vec<Option<u16>> = Vec::new();
                if unpack {
                    for t in targets {
                        if t == "_" {
                            target_slots.push(None);
                        } else {
                            target_slots.push(Some(self.slot_of(t)?));
                        }
                    }
                }
                // イテレータを取得して temp slot に格納。
                let iter_temp = self.alloc_temp()?;
                self.compile_expr(iter)?;
                self.emit(Op::GetIter);
                self.emit(Op::StoreLocal(iter_temp));
                // loop_start: ForIter で next。EndOfIteration なら exit へ、要素なら target へ束縛。
                let loop_start = self.here();
                let fi = self.emit(Op::ForIter(iter_temp, target_slot, 0)); // exit は後でパッチ
                // タプル分解: 要素を push して**逆順**に StoreLocal で受ける（pop は末尾から）。
                if unpack {
                    self.emit(Op::UnpackTuple(target_slot, targets.len() as u16));
                    for ts in target_slots.iter().rev() {
                        match ts {
                            Some(slot) => self.emit(Op::StoreLocal(*slot)),
                            None => self.emit(Op::Pop), // `for k, _ in ...` の捨て要素
                        };
                    }
                }
                self.loops.push(LoopCtx {
                    continue_target: loop_start, // continue は次の ForIter へ戻る
                    break_jumps: Vec::new(),
                    try_len: self.try_stack.len(),
                });
                // 本体はこのループ入口の深さで走る（#34）。
                let saved_base = self.stmt_base.replace(0);
                for s in body {
                    self.compile_stmt(s)?;
                }
                self.stmt_base = saved_base;
                self.emit(Op::Jump(loop_start));
                let exit = self.here();
                // ForIter の exit_ip をバックパッチ（patch_jump は Jump 系専用なので手動）。
                self.code[fi] = Op::ForIter(iter_temp, target_slot, exit);
                let ctx = self.loops.pop().unwrap();
                for j in ctx.break_jumps {
                    self.patch_jump(j, exit);
                }
                self.free_temp(); // iter_temp
                if sink_temp.is_some() {
                    self.free_temp(); // タプル分解／`_` 用の受け皿（#27-c）
                }
                // シャドウしていたループ変数の名前を外側の slot へ戻す（#27）。
                for (name, old) in shadow_saved.into_iter().rev() {
                    self.slots.insert(name, old);
                    self.free_temp();
                }
            }
            Stmt::Break => {
                // 最内ループの break_jumps に登録し、末尾へジャンプ（バックパッチ）。
                // ⚠ ブロック式の途中から跳ぶときは、その式が積んだオペランドを先に捨てる（#34）。
                if self.loops.is_empty() {
                    // 囲むループが無い＝実行時に必ず失敗する。**bail せず**ツリーウォークと
                    // 同じメッセージで落とす（bail すると `--vm=on` が `VmForceError` になり、
                    // 正しいエラー報告が off/on で食い違う・#34）。
                    let n = self.add_name("SyntaxError: 'break' outside for/while loop");
                    self.emit(Op::Fail(n));
                    return Some(());
                }
                self.emit_unwind_to_loop()?;
                let j = self.emit(Op::Jump(0));
                self.loops.last_mut()?.break_jumps.push(j);
            }
            Stmt::Continue => {
                let Some(target) = self.loops.last().map(|l| l.continue_target) else {
                    let n = self.add_name("SyntaxError: 'continue' outside for/while loop");
                    self.emit(Op::Fail(n));
                    return Some(());
                };
                self.emit_unwind_to_loop()?;
                self.emit(Op::Jump(target));
            }
            // ── ローカル宣言（exec_let / exec の const・mut と同一セマンティクス） ──
            // 最上位モード（#10-c）では slot ではなくグローバルへ宣言する（`DeclareGlobal`）。
            Stmt::Const(name, _, e) => {
                self.compile_expr(e)?;
                if name == "_" {
                    self.emit(Op::Pop);
                } else if let Some(ni) = self.toplevel_decl_name(name) {
                    self.emit(Op::DeclareGlobal(ni, DeclKind::Const));
                } else {
                    let slot = self.slot_of(name)?;
                    self.emit(Op::StoreLocal(slot)); // const は copy/freeze しない
                }
            }
            Stmt::Mut(name, _, e) => {
                self.compile_expr(e)?;
                if name == "_" {
                    self.emit(Op::Pop);
                } else if let Some(ni) = self.toplevel_decl_name(name) {
                    self.emit(Op::DeclareGlobal(ni, DeclKind::Mut));
                } else if let Some(&i) = self.cells.get(name) {
                    // 入れ子 `fn` に可変キャプチャされるローカル（#27-d 段階 2b）。
                    // deep_copy はセル版でも同じ（`Stmt::Mut` は常に複製する）。
                    self.emit(Op::StoreCellDeepCopy(i));
                } else {
                    let slot = self.slot_of(name)?;
                    self.emit(Op::StoreLocalDeepCopy(slot)); // mut は常に deep_copy
                }
            }
            Stmt::Let(name, _, e) if self.toplevel_decl_name(name).is_some() && name != "_" => {
                // 最上位の `let`（#10-c）。ソースが識別子のときの可変性は**コンパイル時に
                // 分からない**（`toplevel_globals` は名前の集合だけ）ので、予測せず
                // `LetFromIdent` でソース名を渡し、`exec_let` と同じ判断を実行時に行う（#27-c）。
                let kind = match e {
                    Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Str(_) | Expr::None => {
                        DeclKind::LetPlain
                    }
                    Expr::Ident { name: src, .. } => DeclKind::LetFromIdent(self.add_name(src)),
                    _ => DeclKind::LetFreezeInstance,
                };
                let ni = self.toplevel_decl_name(name)?;
                self.compile_expr(e)?;
                self.emit(Op::DeclareGlobal(ni, kind));
            }
            Stmt::Let(name, _, e) => {
                if name == "_" {
                    self.compile_expr(e)?;
                    self.emit(Op::Pop);
                } else {
                    let slot = self.slot_of(name)?;
                    // ソースの種類で store op を選ぶ（exec_let のセマンティクスに一致）。
                    //
                    // ⚠ **`exec_let` は `Resolution` を見ない**。`Expr::Ident` なら何であれ
                    // `get_var(src)` の可変性で分岐する。よってここも**識別子は全て識別子として
                    // 扱う**こと（`Resolution::Global` を非識別子式の枝へ落とすと、可変グローバルを
                    // ソースにした `let` でコピー＆フリーズが漏れる）。
                    let store = match e {
                        // ident ソースのうち**可変性がコンパイル時に分かる**もの（＝slot にある）。
                        // 可変なら copy+freeze、不変ならそのまま。
                        Expr::Ident { res: Resolution::Local(s), .. } => {
                            if self.slot_mut.get(*s as usize).copied().unwrap_or(false) {
                                Op::StoreLocalCopyFreeze(slot)
                            } else {
                                Op::StoreLocal(slot)
                            }
                        }
                        Expr::Ident { name: nm, .. } if self.slots.contains_key(nm) => {
                            let s = self.slots[nm];
                            if self.slot_mut.get(s as usize).copied().unwrap_or(false) {
                                Op::StoreLocalCopyFreeze(slot)
                            } else {
                                Op::StoreLocal(slot)
                            }
                        }
                        // slot に無い ident（グローバル・未定義）は**実行時に**ソースの可変性を見る。
                        Expr::Ident { name: nm, .. } => {
                            let ni = self.add_name(nm);
                            Op::StoreLocalFromIdent(slot, ni)
                        }
                        // リテラル（プリミティブ）は freeze 不要。
                        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Str(_)
                        | Expr::None => Op::StoreLocal(slot),
                        // 非識別子式: Instance のときのみ copy+freeze（exec_let 非 ident 分岐）。
                        _ => Op::StoreLocalFreezeInstance(slot),
                    };
                    self.compile_expr(e)?;
                    self.emit(store);
                }
            }
            // `static mut x = e`（#27-d）。記憶域は `Interpreter::static_cells`（宣言位置がキー）。
            //
            // `exec_static_var` は **セルが既にあれば初期化子を評価しない**ので、
            // 「あればジャンプで飛び越す」形に落とす:
            //   StaticInit(span, after) ─ セルがあれば after へ
            //   <初期化子>
            //   StaticStore(span)       ─ セルを作って値を入れる
            //   after:
            // ⚠ ツリーウォークは毎回 `declare_var(name, Var::new_cell(cell))` するが、VM は
            // 名前の束縛を持たず読み書きを直接セルへ落とすので、宣言そのものは何も出さない。
            Stmt::Static(name, expr, span) => {
                // 採番済みの宣言位置と一致することを前提にする（prepass が入れたもの）。
                let Some(decl_span) = self.statics.get(name).cloned() else {
                    // 入れ子ブロックの中の `static`（prepass が見ていない）は非対応。
                    bail("static-nested", None);
                    return None;
                };
                debug_assert_eq!(
                    (decl_span.line, decl_span.col),
                    (span.line, span.col),
                    "static の宣言位置が prepass と compile でずれている"
                );
                let si = self.add_span(&decl_span);
                let guard = self.emit(Op::StaticInit(si, 0)); // 飛び先は後でパッチ
                self.compile_expr(expr)?;
                self.emit(Op::StaticStore(si));
                let after = self.here();
                self.code[guard] = Op::StaticInit(si, after);
            }
            // 属性代入 `obj.attr = value` / 添字代入 `obj[i] = value`。
            Stmt::AttrAssign { target, value } => match target {
                // `obj.attr = value`。obj を push → value を push → SetAttr。
                //
                // ⚠ レシーバの種類で絞らない（#27-c）。`attr_assign_evaled` が
                // ツリーウォークの `attr_assign` の**唯一の実装**になったので、
                // `Value::Instance` / `Value::Class` / それ以外のエラーまで一致する。
                // 以前は `object_is_instance` で絞っていたが、それは 2 実装の差を
                // 隠すためのもので、型注釈の無いグローバルが bail する原因だった。
                Expr::Attr { object, attr, .. } => {
                    self.compile_expr(object)?;
                    // #34: 右辺の評価中は obj が 1 つ積まれている。伝えないと
                    // `obj.x = 1 + block ->int: … break …` が bail する（実測で見つけた漏れ）。
                    self.pending = self.stmt_base.map(|d| d + 1);
                    self.compile_expr(value)?;
                    let ni = self.add_name(attr);
                    self.emit(Op::SetAttr(ni));
                }
                // `obj[i] = value` — tree-walk は value(rhs) を先に評価するので temp に退避して順序を合わせる。
                Expr::Subscript { object, index, .. } => {
                    let vtmp = self.alloc_temp()?;
                    self.compile_expr(value)?; // value を先に評価
                    self.emit(Op::StoreLocal(vtmp));
                    self.compile_expr(object)?; // obj
                    self.compile_expr(index)?; // key
                    self.emit(Op::LoadLocal(vtmp)); // value
                    self.emit(Op::SetIndex);
                    self.free_temp();
                }
                // `obj::Trait.attr = value`（#27）。`SetAttr` と同じく `[obj, value]` の順で積む。
                Expr::TraitAccess { object, trait_name, attr } => {
                    self.compile_expr(object)?;
                    self.pending = self.stmt_base.map(|d| d + 1); // 同上（#34）
                    self.compile_expr(value)?;
                    let ti = self.add_name(trait_name);
                    let ai = self.add_name(attr);
                    self.emit(Op::SetTraitAttr(ti, ai));
                }
                // 非 instance 属性は非対応。
                other => {
                    bail_expr("assign-target", other);
                    return None;
                }
            },
            // 添字への複合代入 `obj[k] op= value`（#27-c）。
            //
            // ツリーウォークは `rhs = eval(value)` → `lhs = eval(target)` → 二項演算 →
            // `attr_assign(target, result)` の順で、**`object`/`index` を 2 回評価する**
            // （読みで 1 回、代入で 1 回）。副作用まで一致させるため、そのまま 2 回積む。
            Stmt::AttrCompoundAssign { target: target @ Expr::Subscript { .. }, op, value } => {
                let Expr::Subscript { object, index, .. } = target else {
                    unreachable!("matched above")
                };
                let rhs_tmp = self.alloc_temp()?;
                self.compile_expr(value)?; // 1. rhs を先に評価
                self.emit(Op::StoreLocal(rhs_tmp));
                self.compile_expr(object)?; // 2. 現在値の読み
                self.compile_expr(index)?;
                self.emit(Op::Subscript);
                self.emit(Op::LoadLocal(rhs_tmp));
                self.emit(Op::Bin(op.clone())); // 3. 二項演算
                let res_tmp = self.alloc_temp()?;
                self.emit(Op::StoreLocal(res_tmp));
                self.compile_expr(object)?; // 4. 代入（`attr_assign` と同じく再評価）
                self.compile_expr(index)?;
                self.emit(Op::LoadLocal(res_tmp));
                self.emit(Op::SetIndex);
                self.free_temp();
                self.free_temp();
            }
            // 属性複合代入 `obj.attr op= value`。
            //
            // ⚠ **レシーバの種類で絞らない**（#27）。読みは `GetAttr`（ツリーウォークの
            // `eval_attr` と同じ `get_attr_val`）、書きは `SetAttr`（`attr_assign` と**同一の**
            // `attr_assign_evaled`）なので、`Value::Class` の `static mut` まで意味論が一致する。
            // 以前あった `object_is_instance` の条件は「2 実装の差」ではなく、下の
            // **局所 slot 前提の最適化**（レシーバを 1 回しか評価しない融合）を守るためのもの。
            // 局所 slot でないレシーバはツリーウォークどおり 2 回評価する経路へ回す。
            Stmt::AttrCompoundAssign { target, op, value } => {
                let (object, attr) = match target {
                    Expr::Attr { object, attr, .. } => (object, attr),
                    other => {
                        bail_expr("attr-compound-target", other);
                        return None;
                    }
                };
                let ni = self.add_name(attr);
                // 型特化（#2b）: フィールドの型は注釈テーブルが `Expr::Attr` の node_id に焼いている。
                // 右辺は `expr_prim`（リテラル / 型注釈つき局所変数）で見る。
                let kind = match target {
                    Expr::Attr { node_id, .. } => self
                        .annot_prim(*node_id)
                        .zip(self.expr_prim(value))
                        .and_then(|(l, r)| Self::pair_kind(l, r))
                        .and_then(|k| Self::gate_bin_kind(k, op)),
                    _ => None,
                };
                match self.as_local(object) {
                    // レシーバが局所 slot（`self`・ローカル変数）のとき。**再評価が副作用を
                    // 持たない**ので、`SetAttr` のベースを 1 回積むだけで読み書き両方に使える。
                    Some(obj_slot) => {
                        self.compile_expr(object)?; // SetAttr のベース

                        // 評価順（#2a）。ツリーウォークは **value を先に評価してから**現在値を読むので、
                        // 素直に組むと [value, cur] の順にスタックへ乗り `Swap` が要る。
                        // ただし value が**副作用を持たない**（局所変数読み or 定数リテラル）なら、
                        // 先に現在値を読んでも観測結果は同じなので `Swap` を丸ごと落とせる。
                        // レシーバ slot が value の評価中に再束縛されないことは `CallMethodLocal` と
                        // 同じ根拠（再束縛は文＝`StoreLocal` でしか起きず、クロージャ捕捉は VM 非対応
                        // で bail する）。
                        //
                        // 現在値の読み出しは `LoadLocal; GetAttr` の 2 命令を `GetAttrLocal` 1 命令へ
                        // 畳む（レシーバを **clone せず frame から参照で読む**ので `Rc` の refcount
                        // 増減も消える）。`Expr::Attr` の compile と同じ融合。
                        let value_pure =
                            self.as_local(value).is_some() || Self::as_const_lit(value).is_some();
                        if !value_pure {
                            self.compile_expr(value)?; // rhs を先に評価（順序保存）
                        }
                        self.emit(Op::GetAttrLocal(obj_slot, ni, ni));
                        if value_pure {
                            // [obj, cur, value] → Bin → [obj, new]（Swap 不要）
                            self.compile_expr(value)?;
                        } else {
                            // [obj, value, cur] → Swap → [obj, cur, value] → Bin → [obj, new]
                            self.emit(Op::Swap);
                        }
                        self.emit_bin_specialized(kind, op);
                        self.emit(Op::SetAttr(ni));
                    }
                    // 一般レシーバ（グローバル変数・クラス名・属性・呼び出し結果／`debug_mode`）。
                    //
                    // ツリーウォークは `eval(value)` → `eval(target)`（**object 1 回目**）→ 二項演算
                    // → `attr_assign(target, ..)`（**object 2 回目**）の順で、`object` を 2 回評価する。
                    // 副作用まで一致させるため**そのまま 2 回積む**（添字複合代入 `d[k] op= v` と
                    // 同じ扱い・#27-c）。上の融合を使えるのは再評価が無害な局所 slot のときだけ。
                    None => {
                        let rhs_tmp = self.alloc_temp()?;
                        self.compile_expr(value)?; // 1. rhs を先に評価
                        self.emit(Op::StoreLocal(rhs_tmp));
                        self.compile_expr(object)?; // 2. 現在値の読み
                        self.emit(Op::GetAttr(ni, ni));
                        self.emit(Op::LoadLocal(rhs_tmp));
                        self.emit_bin_specialized(kind, op); // 3. 二項演算
                        let res_tmp = self.alloc_temp()?;
                        self.emit(Op::StoreLocal(res_tmp));
                        self.compile_expr(object)?; // 4. 代入（`attr_assign` と同じく再評価）
                        self.emit(Op::LoadLocal(res_tmp));
                        self.emit(Op::SetAttr(ni));
                        self.free_temp();
                        self.free_temp();
                    }
                }
            }
            Stmt::Raise { exc, span } => match exc {
                Some(e) => {
                    self.compile_expr(e)?;
                    let si = self.add_span(span);
                    self.emit(Op::Raise(si));
                }
                None => {
                    self.emit(Op::Reraise); // bare raise（再送出）
                }
            },
            Stmt::Try { body, handlers, finally_body } => {
                self.compile_try(body, handlers, finally_body)?;
            }
            // ブロック式内: block_return は最内ブロック式の result_slot へ格納して出口へ跳ぶ。
            Stmt::BlockReturn(e, _) => {
                let ctx = self.block_ctxs.last()?;
                let result_slot = ctx.result_slot;
                let ann = ctx.return_type;
                let block_try_len = ctx.try_len;
                // #40: `finally` の複製の中から跳ぶときは、複製が載っている分を捨てる。
                let block_pops = match (self.stmt_base, ctx.entry_depth) {
                    (Some(now), Some(base)) => now.saturating_sub(base),
                    _ => 0,
                };
                self.compile_expr(e)?;
                // #35: `->T` があれば実行時検査（ツリーウォークの `check_block_return_type`）。
                if let Some(idx) = ann {
                    self.emit(Op::CheckBlockReturn(idx));
                }
                self.emit(Op::StoreLocal(result_slot));
                // #37: ブロック式入口までの try を巻き戻す（finally を走らせる）。
                // ⚠ 値は既に `result_slot` へ退避済みなので finally が何を積んでも安全。
                self.emit_unwind_tries(block_try_len, true, 0)?;
                for _ in 0..block_pops {
                    self.emit(Op::Pop);
                }
                let j = self.emit(Op::Jump(0));
                self.block_ctxs.last_mut().unwrap().end_jumps.push(j);
            }
            // loop_yield は最内の「yield 先を持つ」ブロック式（block:/for/while 式）の蓄積リストへ追加。
            // if/match 式は透過（yield_slot=None）なので飛ばして外側へ届く。
            Stmt::LoopYield(e) => {
                let Some(yield_slot) = self.block_ctxs.iter().rev().find_map(|c| c.yield_slot)
                else {
                    // for/while 式の外の `loop_yield`（#35）。**bail せず**ツリーウォークと
                    // 同じ文言で落とす（bail すると `--vm=on` だけ `VmForceError` になる）。
                    let n = self.add_name(
                        "SyntaxError: 'loop_yield' can only be used inside a for/while expression (with ->list[T] annotation)",
                    );
                    self.emit(Op::Fail(n));
                    return Some(());
                };
                // ⚠ 要素型は**最内ブロック式**のアノテーションから引く（`yield_slot` を持つ
                // ブロック式とは限らない）。ツリーウォークの `BLOCK_RETURN_EXPECTED_TYPE.last()` と同じ。
                let ann = self.block_ctxs.last().and_then(|c| c.return_type);
                self.compile_expr(e)?;
                if let Some(idx) = ann {
                    self.emit(Op::CheckLoopYield(idx));
                }
                self.emit(Op::ListAppendLocal(yield_slot));
            }
            // ジェネレータ本体の `yield expr`（タスク #8）。値を評価して yield 収集バッファへ産出する。
            // eager 収集なので制御は継続（ツリーウォークの `Stmt::Yield` と同一）。
            Stmt::Yield(e) => {
                self.compile_expr(e)?;
                self.emit(Op::Yield);
            }
            // `target <- async->T: body`（タスク #9）。AsyncManager にタスクを投入する。
            Stmt::AsyncAssign { target, stmts, .. } => {
                self.compile_async_assign(target, stmts)?;
            }
            // 入れ子 `fn` 定義（#27）。**外側フレームを一切参照しない場合に限り**載せる。
            //
            // ⚠ ここが健全性の要。ツリーウォークは `capture_env` で外側スコープを走査して
            // キャプチャを作るが、VM フレームは `scopes` に無いので走査しても見つからない。
            // 「自由変数 ∩ 外側の slot = ∅」を確かめれば、ツリーウォークでも
            // キャプチャは空になる＝両者一致する。**この検査を緩めると閉包変数が黙って消える**。
            //
            // デコレータ・テンプレートは非対応（`eval` と `TemplateFnValue` の再現が要るため）。
            Stmt::FnDef {
                name,
                template_params,
                params,
                body,
                decorators,
                return_type,
                ..
            } => {
                if !template_params.is_empty() {
                    bail("nested-fn-template", None);
                    return None;
                }
                if !decorators.is_empty() {
                    bail("nested-fn-decorator", None);
                    return None;
                }
                let Some((captures, cell_captures, static_captures)) =
                    self.nested_fn_captures(params, body)
                else {
                    // 事前解析（`mut_captured_by_nested_fn`）が拾えなかった可変キャプチャ。
                    bail("nested-fn-mutable-capture", None);
                    return None;
                };
                let slot = self.slot_of(name)?;
                let idx = u32::try_from(self.fn_defs.len()).ok()?;
                self.fn_defs.push(crate::vm::chunk::ChunkFnDef {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    return_type: return_type.clone(),
                    slot,
                    captures,
                    cell_captures,
                    static_captures,
                });
                self.emit(Op::MakeFn(idx));
            }
            // `block: <stmts>` 文（#27-c）。ツリーウォークの `exec_block_stmt` は
            // **`block_return` を吸収**して `Normal` を返し、break/continue/return/raise は外へ通す。
            // ブロック式のコンパイラをそのまま使い、値を捨てれば同じ意味論になる。
            // ⚠ 文なので入口はオペランドスタックが平衡＝深さは `stmt_base` そのもの（#34）。
            // 内部の `break`/`continue` は外側ループへ貫通する（以前は本体ごと bail していた）。
            Stmt::Block(body) => {
                let depth = self.stmt_base;
                // ⚠ `block:` **文**はツリーウォークで `BLOCK_RETURN_EXPECTED_TYPE` へ push しない
                // ので、中の `block_return` は**外側の式**のアノテーションで検査される（#35）。
                let ann = self.block_ctxs.last().and_then(|c| c.return_type);
                // ⚠ `block:` **文**は loop_yield に対して**透過**（#35）。
                self.compile_block_expr(body, depth, ann, false)?;
                self.emit(Op::Pop);
            }
            // `pass` は何も出さない（#27）。ツリーウォークも `ExecResult::Normal` を返すだけ。
            // 文境界の予約（`mark_stmt_start`）は入口で済んでいるので、
            // 命令を 1 つも出さなくてもデバッガの停止位置はずれない。
            Stmt::Pass => {}
            // `break_point`（#27）: デバッガ REPL へ入る。ツリーウォークと同じ `exec_breakpoint`。
            Stmt::BreakPoint { span } => {
                let si = self.add_span(span);
                self.emit(Op::BreakPoint(si));
            }
            // `let a, b = t`（#27-c）。束縛先は slot（制御フロー内の宣言）と
            // グローバル宣言（最上位の宣言文）の 2 通り。`collect_nested_decls` が
            // 入れ子のターゲットにだけ slot を割り当てるので、その有無で判別できる。
            //
            // ⚠ 混ぜると `for` の 2 周目で「already declared」になる（`built_in.ar` の `zx`）。
            Stmt::LetTuple { targets, value, .. } => {
                use crate::ast::TupleTarget;
                let name_of = |t: &TupleTarget| match t {
                    TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => {
                        Some(n.clone())
                    }
                    TupleTarget::Wildcard => None,
                };
                let slots: Vec<Option<u16>> = targets
                    .iter()
                    .map(|t| name_of(t).and_then(|n| self.slots.get(&n).copied()))
                    .collect();
                let any_slot = slots.iter().any(|s| s.is_some());
                if any_slot {
                    // 一部だけ slot に載る形は想定外（`collect_nested_decls` は全ターゲットを
                    // まとめて登録する）。取りこぼしを黙って混ぜないよう弾く。
                    if targets
                        .iter()
                        .zip(&slots)
                        .any(|(t, s)| name_of(t).is_some() && s.is_none())
                    {
                        bail("let-tuple-partial-slots", Some(stmt));
                        return None;
                    }
                } else if self.toplevel_globals.is_empty() {
                    // 最上位モードでないのに slot も無い ＝ 束縛先が決まらない。
                    bail("let-tuple-no-target", Some(stmt));
                    return None;
                }
                self.compile_expr(value)?;
                let i = u32::try_from(self.tuple_decls.len()).ok()?;
                self.tuple_decls.push(crate::vm::chunk::TupleDecl {
                    targets: targets.to_vec(),
                    slots: if any_slot { slots } else { Vec::new() },
                });
                self.emit(Op::LetTuple(i));
            }
            // `freeze x`（#27-c）。値をスタックに載せずに `exec_freeze` を呼ぶだけ。
            Stmt::Freeze(name, span) => {
                let ni = self.add_name(name);
                let si = self.add_span(span);
                self.emit(Op::FreezeVar(ni, si));
            }
            // `src on/once handler` / `src off handler`（#27-c）。
            // 評価順（source → handler）はツリーウォークと同じ。
            Stmt::EventSubscribe { source, handler, is_once, is_async, .. } => {
                self.compile_expr(source)?;
                self.compile_expr(handler)?;
                self.emit(Op::EventSubscribe(*is_once, *is_async));
            }
            Stmt::EventUnsubscribe { source, handler, .. } => {
                self.compile_expr(source)?;
                self.compile_expr(handler)?;
                self.emit(Op::EventUnsubscribe);
            }
            // それ以外（定義・import 等）は非対応。
            _ => {
                bail("stmt", Some(stmt));
                return None;
            }
        }
        Some(())
    }

    /// `try/except` / `try/finally` / `try/except/finally` をコンパイルする。
    ///
    /// 3 点セットは **`try/except` を `try/finally` で包む**形に落とす（#27-c）。
    /// これは Python と同じ等価変形で、finally とハンドラの相互作用を別実装しなくて済む:
    /// - 本体が正常終了 → 内側 `PopTry` → 外側 `PopTry` → finally
    /// - ハンドラがマッチ → ハンドラ本体 → 外側 `PopTry` → finally
    /// - どのハンドラにもマッチしない → 内側の `Reraise` が**外側の landing pad** へ落ちる
    ///   （例外時に `run` がハンドラを pop 済みなので、内側で捕まり直すことはない）→ finally → 再送出
    /// - ハンドラ本体が例外を出した → 同上（外側 landing pad）→ finally → 再送出
    fn compile_try(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        finally_body: &Option<Vec<Stmt>>,
    ) -> Option<()> {
        match finally_body {
            None => self.compile_try_except(body, handlers),
            Some(fin) => self.compile_try_finally(body, handlers, fin),
        }
    }

    /// `try: <body> except ...:` をハンドラスタック（SetupTry/PopTry）＋ landing pad にコンパイルする。
    fn compile_try_except(&mut self, body: &[Stmt], handlers: &[ExceptHandler]) -> Option<()> {
        // try を飛び越える制御フロー（break/continue/block_return/loop_yield）があると
        // SetupTry ハンドラが残るため bail。return は run から即復帰しハンドラは破棄されるので OK。
        // #37: `break`/`continue`/`block_return` は `emit_unwind_tries` が `PopTry` を
        // 出して正しく抜けるので、ここで弾く必要はなくなった（`has_escape` は常に偽を返す）。

        let setup = self.emit(Op::SetupTry(0)); // handler_ip は後でパッチ
        // 本体の間だけハンドラが 1 つ多い（#34）。ここから外側ループへ跳ぶ `break` は
        // `PopTry` を通らないので、跳ぶ側が同じ数だけ戻す必要がある（`finally` は無いので `None`）。
        self.try_stack.push(None);
        let r = (|| {
            for s in body {
                self.compile_stmt(s)?;
            }
            Some(())
        })();
        self.try_stack.pop();
        r?;
        self.emit(Op::PopTry);
        let mut end_jumps = vec![self.emit(Op::Jump(0))]; // 正常終了 → END
        // landing pad: 例外時にここへ来る（スタック = [exc]）。
        let land = self.here();
        self.code[setup] = Op::SetupTry(land);
        for h in handlers {
            match &h.exc_type {
                // bare `except:` — 無条件マッチ。
                None => {
                    self.bind_or_pop_exc(h)?;
                    for s in &h.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    // bare except 以降のハンドラは到達不能（パーサが末尾を保証）。
                }
                // `except E [as e]:` — 型マッチ。
                Some(type_name) => {
                    self.emit(Op::Dup); // [exc, exc]
                    let ni = self.add_name(type_name);
                    self.emit(Op::ExcMatch(ni)); // [exc, bool]
                    let jf = self.emit(Op::JumpIfFalse(0)); // bool を pop・不一致は next へ
                    self.bind_or_pop_exc(h)?; // 一致: [exc] を束縛 or 破棄
                    for s in &h.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
            }
        }
        // どのハンドラにもマッチしなかった: [exc] を捨てて再送出。
        self.emit(Op::Pop);
        self.emit(Op::Reraise);
        let end = self.here();
        for j in end_jumps {
            self.patch_jump(j, end);
        }
        Some(())
    }

    /// `try: <body> [except ...:] finally: <fin>`。正常経路・例外経路の両方で finally を走らせる。
    ///
    /// `handlers` が空でなければ、**内側に `try/except` をそのまま埋め込む**（`compile_try` の doc）。
    fn compile_try_finally(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        fin: &[Stmt],
    ) -> Option<()> {
        // #37/#40: 本体・ハンドラ・`finally` 本体のいずれの脱出も
        // `emit_unwind_tries` / `compile_finally_copy` が扱う（`has_escape` の門は不要になった）。
        let setup = self.emit(Op::SetupTry(0));
        // 本体から跳ぶ脱出は `finally` を走らせてから跳ぶ（#37）。そのために本体を
        // **この finally 付きで**登録する。⚠ `has_escape` は文しか歩かないので、
        // ブロック式の中の `break` はこの登録だけが捕まえる。
        self.try_stack.push(Some(fin.to_vec()));
        let r = (|| {
            if handlers.is_empty() {
                for s in body {
                    self.compile_stmt(s)?;
                }
            } else {
                self.compile_try_except(body, handlers)?;
            }
            Some(())
        })();
        self.try_stack.pop();
        r?;
        self.emit(Op::PopTry);
        // 正常経路の finally（オペランドは積まれていない）。
        self.compile_finally_copy(fin, 0)?;
        let normal_jump = self.emit(Op::Jump(0)); // END
        // 例外 landing pad: スタック = [exc]。finally はスタック中立なので [exc] は底に残る。
        let land = self.here();
        self.code[setup] = Op::SetupTry(land);
        // 例外経路の finally。⚠ `[exc]` が 1 つ積まれた上で走る（#40）。
        // ここから `break`/`return` で跳ぶと `Pop`/`Reraise` を飛ばす＝**例外は破棄される**
        // （ツリーウォーク・Python と同じ）。
        self.compile_finally_copy(fin, 1)?;
        self.emit(Op::Pop); // [exc] を捨てる
        self.emit(Op::Reraise); // 再伝播（current_exception は設定済み）
        let end = self.here();
        self.patch_jump(normal_jump, end);
        Some(())
    }

    /// except ハンドラ landing の [exc] を、別名があれば slot へ束縛、なければ捨てる。
    fn bind_or_pop_exc(&mut self, h: &ExceptHandler) -> Option<()> {
        if let Some(alias) = &h.name {
            let slot = *self.slots.get(alias)?;
            self.emit(Op::StoreLocal(slot)); // exc を束縛（消費）
        } else {
            self.emit(Op::Pop); // exc を破棄
        }
        Some(())
    }

    /// `match` 文を temp slot + ジャンプ列にコンパイルする（`exec_match_stmt` と同一意味論）。
    /// サブジェクトを一度だけ評価して temp に格納し、各アームを順に照合する。
    fn compile_match(&mut self, subject: &Expr, arms: &[MatchArm]) -> Option<()> {
        // サブジェクトを一度評価して temp に退避（各アームの照合で使い回す）。
        let temp = self.alloc_temp()?;
        self.compile_expr(subject)?;
        self.emit(Op::StoreLocal(temp));

        let mut end_jumps: Vec<usize> = Vec::new();
        for arm in arms {
            match &arm.pattern {
                // ワイルドカード `case _:` は無条件マッチ。
                MatchPattern::Case(Expr::Ident { name: n, .. }) if n == "_" => {
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    // 以降のアームは到達不能だが害はない（emit を続けても正しさは保たれる）。
                }
                MatchPattern::Case(pattern_expr) => {
                    self.emit(Op::LoadLocal(temp));
                    self.compile_expr(pattern_expr)?;
                    self.emit(Op::Bin(BinOp::Eq)); // subject == pattern（apply_binop_dyn 委譲）
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
                MatchPattern::IsType(type_name) => {
                    self.emit(Op::LoadLocal(temp));
                    let ni = self.add_name(type_name);
                    self.emit(Op::IsType(ni));
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
            }
        }
        let end = self.here();
        for j in end_jumps {
            self.patch_jump(j, end);
        }
        self.free_temp();
        Some(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Option<()> {
        // #34: 親が「この式が始まる深さ」を伝えていれば受け取る。ここで奪うので、
        // 明示的に伝え直さない限り子の式は `None`（＝ブロック式内 `break` が bail）になる。
        let pending = self.pending.take();
        match expr {
            Expr::Int(n) => {
                let c = self.add_const(Value::Int(*n));
                self.emit(Op::Const(c));
            }
            Expr::Float(f) => {
                let c = self.add_const(Value::Float(*f));
                self.emit(Op::Const(c));
            }
            Expr::Bool(b) => {
                let c = self.add_const(Value::Bool(*b));
                self.emit(Op::Const(c));
            }
            Expr::Str(s) => {
                let c = self.add_const(Value::Str(s.clone()));
                self.emit(Op::Const(c));
            }
            Expr::None => {
                self.emit(Op::Nil);
            }
            // `obj::Trait.attr` の読み（#27）。レシーバの種別はコンパイル時に保証せず、
            // `trait_access_evaled` が実行時に検査する（ツリーウォークと同じ 1 実装）。
            Expr::TraitAccess { object, trait_name, attr } => {
                self.compile_expr(object)?;
                let ti = self.add_name(trait_name);
                let ai = self.add_name(attr);
                self.emit(Op::GetTraitAttr(ti, ai));
            }
            // 虚数リテラル（#27-c）。`eval` の `ImaginaryLit(f) => Value::Complex(0.0, f)` と同じ。
            Expr::ImaginaryLit(f) => {
                let ci = self.add_const(Value::Complex(0.0, *f));
                self.emit(Op::Const(ci));
            }
            // `undefined` リテラル（#27）。`eval` の `Expr::Undefined => Value::Undefined` と同じ。
            Expr::Undefined => {
                let ci = self.add_const(Value::Undefined);
                self.emit(Op::Const(ci));
            }
            // **セル変数**の読み（#27-d 段階 2b）。slot を持たないので slot 系より先に判定する。
            Expr::Ident { name, .. } if self.cells.contains_key(name) => {
                let i = self.cells[name];
                self.emit(Op::LoadCell(i));
            }
            // `static mut` の読み（#27-d）。**slot 系より先に判定する**（slot を持たない名前）。
            // ⚠ `Resolution::Local` より先に置くこと。`static` を含む関数はリゾルバが
            // 解決を諦める（`collect_base_decls` が未対応の宣言文で false を返す）ので
            // 実際には `Local` は付かないが、順序で守っておく。
            Expr::Ident { name, .. } if self.statics.contains_key(name) => {
                let span = self.statics[name].clone();
                let si = self.add_span(&span);
                self.emit(Op::LoadStatic(si));
            }
            Expr::Ident { res: Resolution::Local(slot), .. } => {
                let s = u16::try_from(*slot).ok()?;
                // セル化された base slot（#27-d 段階 2b）。slot は穴なので読んではいけない。
                match self.cell_by_slot.get(&s) {
                    Some(&i) => self.emit(Op::LoadCell(i)),
                    None => self.emit(Op::LoadLocal(s)),
                };
            }
            // 解決済みグローバル参照（R2-b）。リゾルバが「最上位宣言かつ非シャドウ」と
            // 確定した読み取りなので、slots 走査も builtin 判定も要らず直接 LoadGlobal。
            Expr::Ident { name, res: Resolution::Global(_), .. } => {
                self.emit_load_global(name);
            }
            // メソッド本体の `Self` を値として読む（#27）。呼び出し以外の位置でも使える。
            Expr::Ident { name, .. }
                if name == "Self" && self.self_slot.is_some() && !self.slots.contains_key(name) =>
            {
                self.emit(Op::LoadSelfClass);
            }
            // 未解決 Ident は slot にあればローカル読み、無ければグローバル読み。
            // デバッグモードでは停止スコープからの名前引き（LoadName）。
            Expr::Ident { name, .. } => {
                if self.debug_mode {
                    let ni = self.add_name(name);
                    self.emit(Op::LoadName(ni));
                } else {
                    match self.slots.get(name) {
                        Some(&slot) => {
                            self.emit(Op::LoadLocal(slot));
                        }
                        // slot にもグローバル解決にも載らない識別子（組み込み型名 `Signal`/`dict`、
                        // リゾルバがシャドウ懸念で外した名前など）。
                        //
                        // ツリーウォークの `Resolution::Unresolved` は `get_val(name)` そのもので、
                        // `Op::LoadName`（= `vm_load_name` = `get_val`）と**エラー文言まで同一**。
                        // よって最上位ではそのまま置き換えられる（#27-c）。
                        //
                        // ⚠ **関数本体では `LoadName` は使えない**。スコープの隔離は `frame_floor`
                        // が担うが、VM フレームは `exec_fn_evaled` の `frame_floor` 前進より手前で
                        // 分岐するので、`get_val` が**呼び出し元のローカルまで見えてしまう**。最上位は
                        // `toplevel_vm_candidate` が `scopes.len() == 1` を保証するので安全。
                        None if self.name_lookup || !self.toplevel_globals.is_empty() => {
                            let ni = self.add_name(name);
                            self.emit(Op::LoadName(ni));
                        }
                        // 関数本体側は `LoadGlobal`（**`scopes[0]` だけを見る**）で載せる（#27）。
                        //
                        // ここへ来る名前が**この関数のローカルではない**ことはコンパイル時に確定している:
                        // base slot の採番と `collect_nested_decls` が本体の全宣言（for ターゲット・
                        // 入れ子ブロックの宣言を含む）を**先に** `slots` へ入れるので、`slots` を引いて
                        // 外れた名前はどの束縛にも当たらない。よってツリーウォークの `get_val`
                        // （`scopes[frame_floor..]` を走査 → `scopes[0]`）と**結果が一致する**
                        // （前段の走査は必ず外れる）。`LoadName` と違い呼び出し元のローカルを覗かないので
                        // `frame_floor` の問題も起きない。未定義時の `NameError: '<name>' is not defined`
                        // も文言まで同一（`Op::LoadGlobal` のミス経路）。
                        None => {
                            self.emit_load_global(name);
                        }
                    }
                }
            }
            // 可変長引数の読み `local::args`（#27）。ツリーウォークは `get_val("local::args")`
            // だが、VM では `compile_fn_inner` が同名で slot を採番しているので slot 読みで足りる。
            // slot が無い＝可変長パラメータを持たない関数での参照＝ツリーウォークでも
            // `NameError` になる形なので、そちらへ委ねる。
            Expr::LocalVar(name) => {
                let key = format!("local::{name}");
                match self.slots.get(key.as_str()) {
                    Some(&slot) => self.emit(Op::LoadLocal(slot)),
                    None => {
                        bail_expr("localvar-unbound", expr);
                        return None;
                    }
                };
            }
            Expr::UnaryOp { op, operand } => {
                // 被演算子は親と同じ深さで始まる（#34）。`-block ->int: …` が該当。
                self.pending = pending;
                self.compile_expr(operand)?;
                self.emit(Op::Un(op.clone()));
            }
            Expr::BinOp { op, left, right, node_id, .. } => match op {
                // 短絡評価: `a and b` / `a or b` は Python 意味論（値を返す）で書き下す。
                BinOp::And => {
                    self.pending = pending;
                    self.compile_expr(left)?;
                    let j = self.emit(Op::JumpIfFalseOrPop(0));
                    // 右辺の評価中は左辺の値が 1 つ積まれている（`JumpIf*OrPop` は
                    // 短絡したときだけ残す＝右辺へ進む経路でも push 済み・#34）。
                    self.pending = pending.map(|d| d + 1);
                    self.compile_expr(right)?;
                    let end = self.here();
                    self.patch_jump(j, end);
                }
                BinOp::Or => {
                    self.pending = pending;
                    self.compile_expr(left)?;
                    let j = self.emit(Op::JumpIfTrueOrPop(0));
                    self.pending = pending.map(|d| d + 1);
                    self.compile_expr(right)?;
                    let end = self.here();
                    self.patch_jump(j, end);
                }
                _ => {
                    // 超命令融合（#2）＋型特化（plan A）: 単純オペランドなら LoadLocal…+Bin を1命令に。
                    // ⚠ 融合対象は単純オペランドだけなのでブロック式は来ない（深さ伝播は不要）。
                    if !self.try_emit_bin_fused(left, right, op, *node_id) {
                        use crate::type_check::BinOperandKind as K;
                        self.pending = pending;
                        self.compile_expr(left)?;
                        // 右辺の評価中は左辺の値が 1 つ積まれている（#34）。
                        self.pending = pending.map(|d| d + 1);
                        self.compile_expr(right)?;
                        // 融合できない形（属性・添字・呼び出し結果など）でも、注釈が型を確定して
                        // いればスタック版の型特化 op に落とす（#16 段階(b)(iii)）。
                        match self.specialized_bin_kind(op, *node_id, left, right) {
                            Some(K::Int) => self.emit(Op::IntBinSS(op.clone())),
                            Some(K::Float) => self.emit(Op::FloatBinSS(op.clone())),
                            None => self.emit(Op::Bin(op.clone())),
                        };
                    }
                }
            },
            Expr::Attr { object, attr, .. } => {
                let name_idx = self.add_name(attr);
                // 超命令融合（#16 段階(b)(i)）: レシーバが局所変数なら frame から参照読みする
                // 専用 op に落とし、`Value` clone（Rc refcount 増減）と push/pop を省く。
                if let Some(slot) = self.as_local(object) {
                    self.emit(Op::GetAttrLocal(slot, name_idx, name_idx));
                } else {
                    self.compile_expr(object)?;
                    self.emit(Op::GetAttr(name_idx, name_idx));
                }
            }
            // 関数呼び出し `func(args)` / メソッド呼び出し `obj.method(args)`。
            Expr::Call { func, args, span, node_id, .. } => {
                if let Expr::Attr { object, attr, .. } = func.as_ref() {
                    // ── メソッド呼び出し ──
                    // #27-b: **レシーバの型を問わない**。実行時の `vm_method_call` が
                    // ツリーウォークと同じ統一実装（`eval_method_call_full`）へ委ねるので、
                    // list/str/dict/CsObject/Signal… どれでも同じ結果になる。
                    // 以前は `Value::Instance` 専用経路しか無く `object_is_instance` で
                    // 弾いていた（最上位・関数あわせて 110 件が bail していた）。
                    //
                    // ⚠ `node_id` を必ず渡すこと。FFI 戻り値検査のキーで、落とすと
                    // 外部言語メソッドの検査が VM 経路だけ素通りする。
                    // FFI 境界検査のエラーメッセージ用（#27-b）。ツリーウォークは
                    // `callee_display_name(func)`（= `L.get_int`）と呼び出し位置を渡すので、
                    // 同じものをコンパイル時に作って副表へ置く（op は太らせない）。
                    self.record_ffi_call_info(*node_id, object, attr, span);
                    // ⚠ **レシーバを push するかは引数をコンパイルする前に決める**（#27-c）。
                    // 名前付き引数があると `CallMethodLocal`（frame 直読み融合）は使えないので、
                    // 引数の形を先に見て融合の可否を確定させる。
                    let fuse_slot = if has_named_args(args) { None } else { self.as_local(object) };
                    if let Some(slot) = fuse_slot {
                        // 超命令融合（#16 段階(b)(i)）: レシーバが局所変数なら push せず frame 直読み。
                        let (mask, _) = self.compile_call_args(args)?;
                        let ni = self.add_name(attr);
                        self.emit(Op::CallMethodLocal(slot, ni, args.len() as u16, mask, *node_id));
                    } else {
                        self.compile_expr(object)?; // receiver を push
                        let (mask, kw) = self.compile_call_args(args)?;
                        let ni = self.add_name(attr);
                        match kw {
                            None => {
                                self.emit(Op::CallMethod(ni, args.len() as u16, mask, *node_id));
                            }
                            // 名前付き／可変長引数（#27-c）。dispatcher は同じで、
                            // 引数名を `kw_calls` 経由で運ぶだけ。
                            Some(arg_names) => {
                                let i = u32::try_from(self.kw_calls.len()).ok()?;
                                self.kw_calls.push(crate::vm::chunk::KwCall {
                                    argc: u16::try_from(args.len()).ok()?,
                                    mut_mask: mask,
                                    name_idx: ni,
                                    // メソッドは call_span=None で呼ぶので span は使わない。
                                    span_idx: 0,
                                    node_id: *node_id,
                                    arg_names,
                                });
                                self.emit(Op::CallMethodKw(i));
                            }
                        }
                    }
                    return Some(()); // メソッド呼び出しは span 不要
                }
                let site = self.add_span(span); // 関数呼び出しはトレースバック用の呼び出し位置を記録
                if let Expr::Ident { name, res: Resolution::Unresolved, .. } = func.as_ref() {
                    // ── VM 対応組み込み（print/range/len）── 評価済み引数で直接呼ぶ。
                    // ローカル slot に同名（シャドウ）がなければ組み込みとして扱う。
                    if is_vm_builtin(name) && !self.slots.contains_key(name) {
                        // 組み込みは mut_mask 不要。
                        let (_, kw) = self.compile_call_args(args)?;
                        let ni = self.add_name(name);
                        match kw {
                            None => {
                                self.emit(Op::CallBuiltin(ni, args.len() as u16));
                            }
                            // 名前付き引数（#27-c）。解釈を確認済みの組み込みだけ引数名ごと運ぶ。
                            // それ以外は `eval_builtin_evaled` が名前を受け取れないので bail。
                            Some(arg_names) if VM_BUILTIN_KW_NAMES.contains(&name.as_str()) => {
                                let i = u32::try_from(self.kw_calls.len()).ok()?;
                                self.kw_calls.push(crate::vm::chunk::KwCall {
                                    argc: u16::try_from(args.len()).ok()?,
                                    mut_mask: 0, // 組み込みは mut 引数を取らない
                                    name_idx: ni,
                                    span_idx: site,
                                    node_id: *node_id,
                                    arg_names,
                                });
                                self.emit(Op::CallBuiltinKw(i));
                            }
                            Some(_) => {
                                bail("call-arg", None);
                                return None;
                            }
                        }
                    } else if self.debug_mode {
                        // デバッグモード: 呼び先を名前引きで取得（局所・グローバル両対応）。
                        let cn = self.add_name(name);
                        self.emit(Op::LoadName(cn));
                        let (mask, kw) = self.compile_call_args(args)?;
                        self.emit_call(args.len(), mask, cn, site, *node_id, kw)?;
                    } else if let Some(&slot) = self.slots.get(name) {
                        // ローカル/パラメータが関数値を保持している場合は slot 読み。
                        self.emit(Op::LoadLocal(slot));
                        let (mask, kw) = self.compile_call_args(args)?;
                        let ni = self.add_name(name);
                        self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                    } else if name == "Self" && self.self_slot.is_some() {
                        // メソッド本体の `Self(...)`（#27）: レシーバのクラスを積んで通常の
                        // `Call` へ流す（`call_value_evaled` の `Value::Class` アーム＝
                        // ツリーウォークと同一のインスタンス化経路）。
                        self.emit(Op::LoadSelfClass);
                        let (mask, kw) = self.compile_call_args(args)?;
                        let ni = self.add_name(name);
                        self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                    } else if is_builtin_callee(name) || name == "Self" {
                        // その他の純粋 builtin・型コンストラクタ・`Self` は非対応。
                        // 呼び先名まで記録する（どれを `eval_builtin_evaled` へ足せば効くかを測るため・#27）。
                        if crate::interpreter::tw_stats::enabled() {
                            crate::interpreter::tw_stats::record_bail("callee-builtin", name);
                        }
                        return None;
                    } else {
                        // グローバル関数呼び出し（#11: 索引キャッシュ付き LoadGlobal）。
                        let ni = self.add_name(name);
                        let ci = self.global_caches.len() as u32;
                        self.global_caches.push(crate::ast::SlotCache::default());
                        self.emit(Op::LoadGlobal(ni, ci));
                        let (mask, kw) = self.compile_call_args(args)?;
                        self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                    }
                } else if let Expr::Ident { name, res: Resolution::Global(_), .. } = func.as_ref() {
                    // 解決済みグローバル関数呼び出し（R2-b）。
                    // 分類はリゾルバ済みなので builtin/slots の判定は不要。
                    // ただしデバッグモードは停止スコープの名前引きに合わせる。
                    let ni = self.add_name(name);
                    if self.debug_mode {
                        self.emit(Op::LoadName(ni));
                    } else {
                        self.emit_load_global(name);
                    }
                    let (mask, kw) = self.compile_call_args(args)?;
                    self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                } else if let Expr::Ident { name, res: Resolution::Local(slot), .. } = func.as_ref() {
                    // 解決済みローカル関数値の呼び出し。
                    let s = u16::try_from(*slot).ok()?;
                    self.emit(Op::LoadLocal(s));
                    let (mask, kw) = self.compile_call_args(args)?;
                    let ni = self.add_name(name);
                    self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                } else if let Expr::TemplateInstantiate { base, type_args } = func.as_ref() {
                    // テンプレート呼び出し `Tmpl[T](args)`（#27-c）。ツリーウォークの
                    // `eval_call` と同じく「base を評価 → `instantiate_template` 本体」。
                    self.compile_expr(base)?;
                    let (mask, kw) = self.compile_call_args(args)?;
                    Self::no_kw(kw)?; // テンプレートの名前付き引数は未対応（#27-c 残り）
                    let ti = u32::try_from(self.type_arg_lists.len()).ok()?;
                    self.type_arg_lists.push(type_args.clone());
                    self.emit(Op::CallTemplate(ti, args.len() as u16, mask));
                } else {
                    // その他の呼び先式（`block:` 式・添字結果・属性以外の任意式）。
                    //
                    // ツリーウォークの `eval_call` も「呼び先式を評価 → `call_value_evaled`」なので、
                    // 素直に **[callee, args...] を積んで `Call`** すればよい（#27-c）。
                    // ⚠ トレースバック表示名は **`<anonymous>`**（`eval_call` の `call_name` が
                    // 識別子以外に付ける名前と揃える。ここを関数名にすると off/auto で出力が食い違う）。
                    self.compile_expr(func)?;
                    let (mask, kw) = self.compile_call_args(args)?;
                    let ni = self.add_name("<anonymous>");
                    self.emit_call(args.len(), mask, ni, site, *node_id, kw)?;
                }
            }
            // ── 添字・コレクションリテラル（タスク #5） ──
            Expr::Subscript { object, index, .. } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.emit(Op::Subscript);
            }
            Expr::Slice { begin, end, step } => {
                // 省略された要素は `Op::Nil`（= `Value::None`）を積む。`slice_from_values`
                // が「無し」に畳むので、ツリーウォークの `None` と同じ意味になる。
                for part in [begin, end, step] {
                    match part {
                        Some(e) => self.compile_expr(e)?,
                        None => {
                            self.emit(Op::Nil);
                        }
                    }
                }
                self.emit(Op::BuildSlice);
            }
            Expr::List(items) => {
                let n = u16::try_from(items.len()).ok()?;
                for it in items {
                    self.compile_expr(it)?;
                }
                self.emit(Op::BuildList(n));
            }
            Expr::Tuple(items) => {
                let n = u16::try_from(items.len()).ok()?;
                for it in items {
                    self.compile_expr(it)?;
                }
                self.emit(Op::BuildTuple(n));
            }
            Expr::Set(items) => {
                let n = u16::try_from(items.len()).ok()?;
                for it in items {
                    self.compile_expr(it)?;
                }
                self.emit(Op::BuildSet(n));
            }
            Expr::Dict(pairs) => {
                let n = u16::try_from(pairs.len()).ok()?;
                for (k, v) in pairs {
                    self.compile_expr(k)?;
                    self.compile_expr(v)?;
                }
                self.emit(Op::BuildDict(n));
            }
            // ── ブロック式（値を産む制御構文, Phase V-C） ──
            Expr::Block { stmts, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_block_expr(stmts, pending, ann, true)?
            }
            Expr::IfExpr { branches, else_body, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_if_expr(branches, else_body, pending, ann)?
            }
            Expr::MatchExpr { subject, arms, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_match_expr(subject, arms, pending, ann)?
            }
            Expr::ForExpr { target, iter, body, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_for_expr(target, iter, body, ann)?
            }
            Expr::WhileExpr { cond, body, return_type } => {
                let ann = self.add_return_type(return_type);
                self.compile_while_expr(cond, body, ann)?
            }

            // ── 動的型検査（#16 段階(b)(ii)）──
            // 型検査が付けた `CheckBefore` 指示を消費して検査 op を出す。
            // 指示が無い（＝未採番ノード等）場合も**保守的に検査を出す**: 検査を落とす方向へは倒さない。
            Expr::IsType { expr, negated, type_name, .. } => {
                self.compile_expr(expr)?;
                let ni = self.add_name(type_name);
                self.emit(Op::IsType(ni));
                if *negated {
                    // `Op::Un(Not)` は Bool に対し `!b` を返す（eval の negated 分岐と同一）。
                    self.emit(Op::Un(crate::ast::UnaryOp::Not));
                }
            }
            Expr::MustBe { expr, guard_type, span, node_id } => {
                if !self.check_required(*node_id) {
                    bail("check-required-mustbe", None);
                    return None;
                }
                self.compile_expr(expr)?;
                let ni = self.add_name(guard_type);
                let si = self.add_span(span);
                self.emit(Op::MustBe(ni, si));
            }
            Expr::Cast { object, type_name, node_id, .. } => {
                if !self.check_required(*node_id) {
                    bail("check-required-cast", None);
                    return None;
                }
                self.compile_expr(object)?;
                let ni = self.add_name(type_name);
                self.emit(Op::Cast(ni));
            }

            // それ以外は非対応。
            _ => {
                bail_expr("expr", expr);
                return None;
            }
        }
        Some(())
    }

    /// `block: <stmts>` 式。block_return 値、なければ loop_yield 蓄積リスト、どちらもなければ None。
    ///
    /// `entry_depth` は**この式が始まるオペランド深さ**（#34）。本体の文はこの深さで走るので、
    /// 本体内の `break`/`continue` はこの数だけ `Pop` してから外側ループへ跳ぶ。
    ///
    /// `owns_yields` は「このブロックが `loop_yield` の蓄積先を持つか」（#35）。
    /// **`block:` 式は持ち（true）、`block:` 文は持たない（false）**。ツリーウォークの
    /// `exec_block_stmt` は `BLOCK_YIELDS` を push しないので、文の中の `loop_yield` は
    /// **外側の for/while 式へ届く**（届く先が無ければ実行時エラー）。ここを true にすると
    /// 蓄積が文に吸い込まれて捨てられる（`for … ->list[int]: block: loop_yield i` が `None` になった）。
    fn compile_block_expr(
        &mut self,
        stmts: &[Stmt],
        entry_depth: Option<u16>,
        ann: Option<u32>,
        owns_yields: bool,
    ) -> Option<()> {
        if block_body_bails(stmts) {
            bail("block-expr-escape", None);
            return None;
        }
        let saved_base = std::mem::replace(&mut self.stmt_base, entry_depth);
        let r = self.compile_block_expr_inner(stmts, ann, owns_yields);
        self.stmt_base = saved_base;
        r
    }

    fn compile_block_expr_inner(
        &mut self,
        stmts: &[Stmt],
        ann: Option<u32>,
        owns_yields: bool,
    ) -> Option<()> {
        let yield_slot = if owns_yields {
            let s = self.alloc_temp()?;
            self.emit(Op::BuildEmptyList);
            self.emit(Op::StoreLocal(s));
            Some(s)
        } else {
            None
        };
        let result_slot = self.alloc_temp()?;
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot,
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        for s in stmts {
            self.compile_stmt(s)?;
        }
        let ctx = self.block_ctxs.pop().unwrap();
        // 正常フォールスルー: 値 = 蓄積リスト or None（蓄積先を持たなければ常に None）。
        match yield_slot {
            Some(s) => {
                self.emit(Op::LoadLocal(s));
                self.emit(Op::ListOrNone);
            }
            None => {
                self.emit(Op::Nil);
            }
        }
        let after_normal = self.emit(Op::Jump(0)); // → EXPR_END
        // block_return 出口: 値 = result_slot。
        let br_end = self.here();
        for j in ctx.end_jumps {
            self.patch_jump(j, br_end);
        }
        self.emit(Op::LoadLocal(result_slot));
        let expr_end = self.here();
        self.patch_jump(after_normal, expr_end);
        self.free_temp(); // result_slot
        if yield_slot.is_some() {
            self.free_temp(); // yield_slot
        }
        Some(())
    }

    /// `if cond -> T: ... [elif][else]` 式。マッチした分岐の block_return 値、なければ None。
    /// yield に対しては透過（yield_slot=None）。
    fn compile_if_expr(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_body: &Option<Vec<Stmt>>,
        entry_depth: Option<u16>,
        ann: Option<u32>,
    ) -> Option<()> {
        for (_, b) in branches {
            if block_body_bails(b) {
                bail("if-expr-escape", None);
                return None;
            }
        }
        if let Some(eb) = else_body {
            if block_body_bails(eb) {
                bail("ifexpr-else-escape", None);
                return None;
            }
        }
        let saved_base = std::mem::replace(&mut self.stmt_base, entry_depth);
        let r = self.compile_if_expr_inner(branches, else_body, ann);
        self.stmt_base = saved_base;
        r
    }

    fn compile_if_expr_inner(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_body: &Option<Vec<Stmt>>,
        ann: Option<u32>,
    ) -> Option<()> {
        let result_slot = self.alloc_temp()?;
        self.emit(Op::Nil);
        self.emit(Op::StoreLocal(result_slot)); // 既定 None
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: None,
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        let mut branch_ends: Vec<usize> = Vec::new();
        for (cond, body) in branches {
            self.compile_expr(cond)?;
            let jf = self.emit(Op::JumpIfFalse(0));
            for s in body {
                self.compile_stmt(s)?;
            }
            branch_ends.push(self.emit(Op::Jump(0)));
            let next = self.here();
            self.patch_jump(jf, next);
        }
        if let Some(eb) = else_body {
            for s in eb {
                self.compile_stmt(s)?;
            }
        }
        let ctx = self.block_ctxs.pop().unwrap();
        let end = self.here();
        for j in branch_ends {
            self.patch_jump(j, end);
        }
        for j in ctx.end_jumps {
            self.patch_jump(j, end);
        }
        self.emit(Op::LoadLocal(result_slot)); // block_return 値 or None
        self.free_temp();
        Some(())
    }

    /// `match subj -> T: arms` 式。マッチしたアームの block_return 値、なければ None。
    fn compile_match_expr(
        &mut self,
        subject: &Expr,
        arms: &[MatchArm],
        entry_depth: Option<u16>,
        ann: Option<u32>,
    ) -> Option<()> {
        for arm in arms {
            if block_body_bails(&arm.body) {
                bail("matchexpr-escape", None);
                return None;
            }
        }
        let saved_base = std::mem::replace(&mut self.stmt_base, entry_depth);
        let r = self.compile_match_expr_inner(subject, arms, ann);
        self.stmt_base = saved_base;
        r
    }

    fn compile_match_expr_inner(
        &mut self,
        subject: &Expr,
        arms: &[MatchArm],
        ann: Option<u32>,
    ) -> Option<()> {
        let subj_temp = self.alloc_temp()?;
        self.compile_expr(subject)?;
        self.emit(Op::StoreLocal(subj_temp));
        let result_slot = self.alloc_temp()?;
        self.emit(Op::Nil);
        self.emit(Op::StoreLocal(result_slot));
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: None,
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        let mut arm_ends: Vec<usize> = Vec::new();
        for arm in arms {
            match &arm.pattern {
                MatchPattern::Case(Expr::Ident { name: n, .. }) if n == "_" => {
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    arm_ends.push(self.emit(Op::Jump(0)));
                }
                MatchPattern::Case(pat) => {
                    self.emit(Op::LoadLocal(subj_temp));
                    self.compile_expr(pat)?;
                    self.emit(Op::Bin(BinOp::Eq));
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    arm_ends.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
                MatchPattern::IsType(type_name) => {
                    self.emit(Op::LoadLocal(subj_temp));
                    let ni = self.add_name(type_name);
                    self.emit(Op::IsType(ni));
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    arm_ends.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
            }
        }
        let ctx = self.block_ctxs.pop().unwrap();
        let end = self.here();
        for j in arm_ends {
            self.patch_jump(j, end);
        }
        for j in ctx.end_jumps {
            self.patch_jump(j, end);
        }
        self.emit(Op::LoadLocal(result_slot));
        self.free_temp(); // result_slot
        self.free_temp(); // subj_temp
        Some(())
    }

    /// `for target in iter -> T: body` 式。block_return 値、なければ loop_yield 蓄積リスト（空なら None）。
    fn compile_for_expr(
        &mut self,
        target: &str,
        iter: &Expr,
        body: &[Stmt],
        ann: Option<u32>,
    ) -> Option<()> {
        if block_body_bails(body) {
            bail("loopexpr-escape", None);
            return None;
        }
        // 自身が最内ループになるので本体の基準深さは 0（#34）。
        // 本体内の `break` はこの式の NORMAL_END（= 蓄積リストを push する位置）へ跳ぶ。
        let saved_base = self.stmt_base.replace(0);
        let r = self.compile_for_expr_inner(target, iter, body, ann);
        self.stmt_base = saved_base;
        r
    }

    fn compile_for_expr_inner(
        &mut self,
        target: &str,
        iter: &Expr,
        body: &[Stmt],
        ann: Option<u32>,
    ) -> Option<()> {
        let target_slot = *self.slots.get(target)?;
        let yield_slot = self.alloc_temp()?;
        self.emit(Op::BuildEmptyList);
        self.emit(Op::StoreLocal(yield_slot));
        let result_slot = self.alloc_temp()?;
        let iter_temp = self.alloc_temp()?;
        self.compile_expr(iter)?;
        self.emit(Op::GetIter);
        self.emit(Op::StoreLocal(iter_temp));
        let loop_start = self.here();
        let fi = self.emit(Op::ForIter(iter_temp, target_slot, 0)); // exit → NORMAL_END
        self.loops.push(LoopCtx {
            continue_target: loop_start,
            break_jumps: Vec::new(),
            try_len: self.try_stack.len(),
        });
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: Some(yield_slot),
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        for s in body {
            self.compile_stmt(s)?;
        }
        self.emit(Op::Jump(loop_start));
        // NORMAL_END: 反復終了 or break → 蓄積リスト or None。
        let normal_end = self.here();
        self.code[fi] = Op::ForIter(iter_temp, target_slot, normal_end);
        let loop_ctx = self.loops.pop().unwrap();
        for j in loop_ctx.break_jumps {
            self.patch_jump(j, normal_end);
        }
        let block_ctx = self.block_ctxs.pop().unwrap();
        self.emit(Op::LoadLocal(yield_slot));
        self.emit(Op::ListOrNone);
        let after_normal = self.emit(Op::Jump(0)); // → EXPR_END
        // BR_END: block_return → result_slot。
        let br_end = self.here();
        for j in block_ctx.end_jumps {
            self.patch_jump(j, br_end);
        }
        self.emit(Op::LoadLocal(result_slot));
        let expr_end = self.here();
        self.patch_jump(after_normal, expr_end);
        self.free_temp(); // iter_temp
        self.free_temp(); // result_slot
        self.free_temp(); // yield_slot
        Some(())
    }

    /// `while cond -> T: body` 式。block_return 値、なければ loop_yield 蓄積リスト（空なら None）。
    fn compile_while_expr(
        &mut self,
        cond: &Expr,
        body: &[Stmt],
        ann: Option<u32>,
    ) -> Option<()> {
        if block_body_bails(body) {
            bail("loopexpr-escape", None);
            return None;
        }
        // 自身が最内ループになるので本体の基準深さは 0（#34）。
        let saved_base = self.stmt_base.replace(0);
        let r = self.compile_while_expr_inner(cond, body, ann);
        self.stmt_base = saved_base;
        r
    }

    fn compile_while_expr_inner(&mut self, cond: &Expr, body: &[Stmt], ann: Option<u32>) -> Option<()> {
        let yield_slot = self.alloc_temp()?;
        self.emit(Op::BuildEmptyList);
        self.emit(Op::StoreLocal(yield_slot));
        let result_slot = self.alloc_temp()?;
        let loop_start = self.here();
        self.compile_expr(cond)?;
        let jf = self.emit(Op::JumpIfFalse(0)); // false → NORMAL_END
        self.loops.push(LoopCtx {
            continue_target: loop_start,
            break_jumps: Vec::new(),
            try_len: self.try_stack.len(),
        });
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: Some(yield_slot),
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        for s in body {
            self.compile_stmt(s)?;
        }
        self.emit(Op::Jump(loop_start));
        let normal_end = self.here();
        self.patch_jump(jf, normal_end);
        let loop_ctx = self.loops.pop().unwrap();
        for j in loop_ctx.break_jumps {
            self.patch_jump(j, normal_end);
        }
        let block_ctx = self.block_ctxs.pop().unwrap();
        self.emit(Op::LoadLocal(yield_slot));
        self.emit(Op::ListOrNone);
        let after_normal = self.emit(Op::Jump(0));
        let br_end = self.here();
        for j in block_ctx.end_jumps {
            self.patch_jump(j, br_end);
        }
        self.emit(Op::LoadLocal(result_slot));
        let expr_end = self.here();
        self.patch_jump(after_normal, expr_end);
        self.free_temp(); // result_slot
        self.free_temp(); // yield_slot
        Some(())
    }
}
