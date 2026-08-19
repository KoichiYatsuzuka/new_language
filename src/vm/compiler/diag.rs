// vm/compiler/diag.rs — 診断フック（`bail`）と、VM が呼び先として扱える組み込み名の表。
//
// ⚠ `bail()` は「ツリーウォークへ落とす印」ではなく「**まだ載せられていない印**」（#33 で
// フォールバックは消えた）。件数は `AR_TW_STATS=1`（要 `--features tw_stats`）で数える。


use crate::ast::{
    CallArg, Expr, Stmt,
};


/// 診断フック（#10）: コンパイルを諦めた地点と構文種別を計上する。
/// `AR_TW_STATS=1` のときだけ働く（既定は enabled() の分岐 1 つで終わる）。
pub(super) fn bail(site: &'static str, stmt: Option<&Stmt>) {
    if !crate::interpreter::tw_stats::enabled() {
        return;
    }
    let detail = stmt
        .map(crate::interpreter::tw_stats::stmt_kind_of)
        .unwrap_or("-");
    crate::interpreter::tw_stats::record_bail(site, detail);
}

/// `bail` の式版（`Expr` のバリアント名を採る）。
pub(super) fn bail_expr(site: &'static str, expr: &Expr) {
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
    // `parse_ar`（#56）。入力は文字列だけなので評価済み引数で表現できる。
    // ⚠ #33〜#55 の間、`is_builtin_callee` が bail していたせいで **`VmForceError` で死んでいた**。
    "parse_ar",
];

pub(super) fn is_vm_builtin(name: &str) -> bool {
    VM_BUILTIN_NAMES.contains(&name)
}

/// **キーワード引数つきでも VM に載せられる**組み込み名（#27-c・`Op::CallBuiltinKw`）。
///
/// キーワードの扱いは組み込みごとに違う（`enumerate` は `start` だけ許容、`zip` はエラー、
/// `len` は名前を無視して位置引数扱い…）ので、`eval_builtin_evaled_named` で
/// **ツリーウォークと一致することを確認した名前だけ**を挙げる。ここに無い名前は従来どおり
/// bail してツリーウォークへ落とす（＝安全側）。
pub(super) const VM_BUILTIN_KW_NAMES: &[&str] = &["enumerate", "open"];

/// 引数に**名前付き**（キーワード／可変長）が含まれるか（#27-c）。
/// `compile_call_args` は同じ判定を戻り値で返すが、それでは遅すぎる場面がある:
/// メソッド呼び出しは「レシーバを push するか frame 直読み融合にするか」を
/// **引数をコンパイルする前に**決めなければならない。
pub(super) fn has_named_args(args: &[CallArg]) -> bool {
    args.iter()
        .any(|a| matches!(a, CallArg::Keyword { .. } | CallArg::Variadic(_)))
}

// ⚠⚠ **`is_builtin_callee` は #56 で削除した。**
//
// 「VM が呼び先として扱えないので bail してツリーウォークへ落とす」ための表だったが、
// **#33 でツリーウォークへのフォールバックは消えている**ので bail ＝ `VmForceError` で停止。
// その取り違えのせいで:
// - `parse_ar` … 「AST を値へ変換するので評価済み引数では表現できない」は**出力と入力の
//   取り違え**（入力は文字列 2 つだけ）。**この組み込みは #33 以来まったく動かなかった**。
//   → `VM_BUILTIN_NAMES` へ移して VM に載せた。
// - `tuple` / `list` / `type` / `byte` … 本来の `NameError` が `VmForceError` に化けていた。
//   → そのまま `LoadGlobal` + `Call` に流せば実行時に正しい `NameError` が出る（#34 と同じ考え方）。
//
// ⇒ **新しく bail を足すときは「bail した先で何が起きるか」を必ず確かめること。**
