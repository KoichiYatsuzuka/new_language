// type_check/state.rs — 型検査の進行に伴って変化するカーソル状態。
//
// Phase 5A で `TypeChecker` から切り出したサブ構造体のひとつ。
// 依存グラフ上は**葉**であり、`TypeRegistry` / `Diagnostics` を一切参照しない。
// この性質を保つため、ここで診断を報告してはならない（例: `declare` は重複宣言を
// 判定せず、ただ上書きする）。エラーを出すかどうかの判断は呼び出し側の責務。

use std::collections::HashMap;

use super::types::{InferredType, VarInfo};

/// 検査中のスコープスタックと「今どこを検査しているか」を保持する。
pub(super) struct CheckState {
    /// 変数スコープのスタック。インデックス 0 がグローバルスコープ、末尾がローカルスコープ。
    scope_stack: Vec<HashMap<String, VarInfo>>,
    /// 現在型検査中の関数名。`None` はトップレベルまたはクラス本体を示す。
    current_fn_name: Option<String>,
    /// 現在型検査中の関数の**宣言された戻り値型**（0-3）。`return` の照合に使う。
    ///
    /// ⚠ **名前（`current_fn_name`）からレジストリを引いて代用してはいけない。**
    /// オーバーロード・メソッド・入れ子関数では名前だけでシグネチャが一意に定まらない。
    /// ⚠ `enter_fn` で**張り替える**（継承しない）。入れ子 `fn` の `return` が
    /// 外側の関数の戻り値型と照合されると嘘の判定になる。
    current_fn_return: Option<InferredType>,
    /// 現在型検査中のクラス名。`None` はクラス外を示す。
    current_class_name: Option<String>,
    /// **`gen` 本体の直下を検査中か**（bug_fix.md B13）。
    ///
    /// ⚠⚠ `yield` は**自分のフレームにしか現れてはいけない**。これが崩れると
    /// 「中断は `vm::run` を 1 回抜けるだけで済む」というコルーチン化の前提が成り立たない。
    /// ⇒ `fn` に入るとき **false に張り替える**（継承しない）のが要点で、
    ///   `gen` の中の入れ子 `fn` でも `yield` がエラーになる。
    /// ⚠ ブロック式・`if`・ループは**同じフレーム**なので張り替えない。
    in_gen_body: bool,
    /// `for`/`while` 式の入れ子深さ。1 以上のとき `block_return` は
    /// 型エラー `BlockReturnInLoopExpr` になる。
    block_return_forbidden_depth: usize,
    /// **いちばん内側のブロック式の `->T` 注釈**（`block_return` / `loop_yield` の
    /// 照合先・タスク 5.2）。
    ///
    /// ⚠⚠ **1 本のスタックでなければならない。** `examples/.../block_return_typecheck.ar`
    /// （#35）が仕様として固定している「**内側の注釈が効く**」がこれ:
    ///
    /// ```arrow
    /// let f = for i in range(4) ->list[int]:
    ///     let _ = if i == 1 ->int:
    ///         loop_yield "not checked here"   # 内側の注釈は `int` で list[T] ではない
    ///         block_return 0                  # ⇒ loop_yield は検査されない
    ///     else:
    ///         0
    ///     loop_yield i                        # ここは外側の `list[int]` が効く
    /// ```
    ///
    /// ⇒ `block_return` と `loop_yield` で**別のスタックにしてはいけない**
    ///   （別にすると `loop_yield` が内側の `->int` を貫いて外側へ届いてしまい、
    ///   この例題が**偽エラー**になる。実際に一度そう実装して `scan_examples` が落ちた）。
    ///
    /// ⚠ **注釈が付いた式だけ**を積む。注釈なしの `block:` 文・`if` 文は積まないので
    /// 中の `block_return` は外側の注釈と照合される（例題 7・8 がこの形）。
    /// ⚠ **関数本体に入るときは `None` を積む**（継承しない）。入れ子 `fn` の
    /// `block_return` が外側のブロック式の注釈と照合されると嘘の判定になる。
    block_expr_expected: Vec<Option<InferredType>>,
    /// **いま見えているテンプレート型変数**の名前（`fn f[T]` / `class C[T]` の `T`）。
    ///
    /// ⚠⚠ `InferredType::from_ann` は**大文字始まりの未知の識別子をクラス名として扱う**
    /// （`NamedInstance("T")`）。型変数と実在のクラスが区別できないため、これを持たないと
    /// `class Box[T]: mut v: T` の `self.v = 0` や `fn conv[T](…) -> T: return n` が
    /// **偽の型エラー**になる（実測）。
    ///
    /// クラス本体に入るときとテンプレート関数に入るときに積み、抜けるときに戻す
    /// （メソッドの中からは囲みクラスの型変数も見えるので**スタック**にしてある）。
    type_params: Vec<String>,
}

impl CheckState {
    /// グローバルスコープの初期内容を与えて生成する。
    pub(super) fn new(global: HashMap<String, VarInfo>) -> Self {
        Self {
            scope_stack: vec![global],
            current_fn_name: None,
            current_fn_return: None,
            current_class_name: None,
            in_gen_body: false,
            block_return_forbidden_depth: 0,
            block_expr_expected: Vec::new(),
            type_params: Vec::new(),
        }
    }

    // ── スコープ ──────────────────────────────────────────────────────────────

    /// 新しいスコープをスタックに積む。
    pub(super) fn push_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    /// 現在のスコープをスタックから取り除く。グローバルスコープは取り除かない。
    pub(super) fn pop_scope(&mut self) {
        if self.scope_stack.len() > 1 {
            self.scope_stack.pop();
        }
    }

    /// 現在スコープに変数を宣言する。同名の変数があれば上書きする。
    pub(super) fn declare(&mut self, name: String, ty: InferredType, mutable: bool) {
        self.scope_stack
            .last_mut()
            .unwrap()
            .insert(name, VarInfo { ty, mutable });
    }

    /// スコープスタックを内側から外側へ走査して変数情報を返す。見つからない場合は `None`。
    pub(super) fn lookup(&self, name: &str) -> Option<&VarInfo> {
        self.scope_stack.iter().rev().find_map(|s| s.get(name))
    }

    // ── 検査位置カーソル ──────────────────────────────────────────────────────

    /// 現在型検査中の関数名。
    pub(super) fn current_fn(&self) -> Option<&str> {
        self.current_fn_name.as_deref()
    }

    /// 現在型検査中のクラス名。
    pub(super) fn current_class(&self) -> Option<&str> {
        self.current_class_name.as_deref()
    }

    /// 関数本体の検査に入る。戻り値は退出時に `exit_fn` へ渡すこと。
    /// `gen` / `fn` 本体へ入るときに張る。戻り値を [`Self::exit_gen_body`] へ渡す。
    pub(super) fn enter_gen_body(&mut self, in_gen: bool) -> bool {
        std::mem::replace(&mut self.in_gen_body, in_gen)
    }

    /// [`Self::enter_gen_body`] が返した値を渡して元に戻す。
    pub(super) fn exit_gen_body(&mut self, prev: bool) {
        self.in_gen_body = prev;
    }

    /// `gen` 本体の直下か。
    pub(super) fn in_gen_body(&self) -> bool {
        self.in_gen_body
    }

    #[allow(clippy::type_complexity)]
    pub(super) fn enter_fn(
        &mut self,
        name: String,
        return_type: Option<InferredType>,
    ) -> (Option<String>, Option<InferredType>) {
        (
            self.current_fn_name.replace(name),
            std::mem::replace(&mut self.current_fn_return, return_type),
        )
    }

    /// `enter_fn` が返した値を渡して関数本体の検査を抜ける。
    pub(super) fn exit_fn(&mut self, prev: (Option<String>, Option<InferredType>)) {
        self.current_fn_name = prev.0;
        self.current_fn_return = prev.1;
    }

    /// テンプレート型変数を積む。戻り値を `pop_type_params` へ渡すこと。
    pub(super) fn push_type_params(&mut self, names: impl Iterator<Item = String>) -> usize {
        let saved = self.type_params.len();
        self.type_params.extend(names);
        saved
    }

    /// `push_type_params` が返した長さまで戻す。
    pub(super) fn pop_type_params(&mut self, saved: usize) {
        self.type_params.truncate(saved);
    }

    /// `name` がいま見えているテンプレート型変数か。
    pub(super) fn is_type_param(&self, name: &str) -> bool {
        self.type_params.iter().any(|t| t == name)
    }

    /// 現在型検査中の関数の宣言された戻り値型（0-3）。
    pub(super) fn current_fn_return(&self) -> Option<&InferredType> {
        self.current_fn_return.as_ref()
    }

    /// クラス本体の検査に入る。戻り値は退出時に `exit_class` へ渡すこと。
    pub(super) fn enter_class(&mut self, name: String) -> Option<String> {
        self.current_class_name.replace(name)
    }

    /// `enter_class` が返した値を渡してクラス本体の検査を抜ける。
    pub(super) fn exit_class(&mut self, prev: Option<String>) {
        self.current_class_name = prev;
    }

    // ── block_return の可否 ───────────────────────────────────────────────────
    //
    // 2つの操作パターンしかない:
    //   バリアント A（障壁）: 関数本体・block/if/match 式に入ると深さは 0 にリセットされ、
    //                         抜けると復元される（内側の block_return は外側のループに属さない）
    //   バリアント B（ループ）: for/while 式に入ると +1、抜けると -1
    // 生の深さ値を外へ出さないことで、+1 と復元の取り違えを防ぐ。

    /// `block_return` が現在禁止されているか（`for`/`while` 式の直下にいるか）。
    pub(super) fn block_return_forbidden(&self) -> bool {
        self.block_return_forbidden_depth > 0
    }

    /// 深さの障壁に入る（深さを 0 にする）。戻り値は `exit_barrier` へ渡すこと。
    pub(super) fn enter_barrier(&mut self) -> usize {
        std::mem::replace(&mut self.block_return_forbidden_depth, 0)
    }

    /// `enter_barrier` が返した値を渡して障壁を抜ける。
    pub(super) fn exit_barrier(&mut self, saved: usize) {
        self.block_return_forbidden_depth = saved;
    }

    // ── ブロック式の結果型（タスク 5.2）──────────────────────────────────────

    /// ブロック式の照合先を積む（注釈なしは `None`＝照合しない）。
    pub(super) fn push_block_expr_expected(&mut self, ty: Option<InferredType>) {
        self.block_expr_expected.push(ty);
    }

    /// ブロック式の照合先を降ろす。
    pub(super) fn pop_block_expr_expected(&mut self) {
        self.block_expr_expected.pop();
    }

    /// いちばん内側のブロック式の `->T`。囲みが無ければ `None`。
    pub(super) fn block_expr_expected(&self) -> Option<&InferredType> {
        self.block_expr_expected.last().and_then(|t| t.as_ref())
    }

    /// `for`/`while` 式の本体に入る。
    pub(super) fn enter_loop_expr(&mut self) {
        self.block_return_forbidden_depth += 1;
    }

    /// `for`/`while` 式の本体を抜ける。
    pub(super) fn exit_loop_expr(&mut self) {
        self.block_return_forbidden_depth -= 1;
    }
}
