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
    /// 現在型検査中のクラス名。`None` はクラス外を示す。
    current_class_name: Option<String>,
    /// `for`/`while` 式の入れ子深さ。1 以上のとき `block_return` は
    /// 型エラー `BlockReturnInLoopExpr` になる。
    block_return_forbidden_depth: usize,
}

impl CheckState {
    /// グローバルスコープの初期内容を与えて生成する。
    pub(super) fn new(global: HashMap<String, VarInfo>) -> Self {
        Self {
            scope_stack: vec![global],
            current_fn_name: None,
            current_class_name: None,
            block_return_forbidden_depth: 0,
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
    pub(super) fn enter_fn(&mut self, name: String) -> Option<String> {
        self.current_fn_name.replace(name)
    }

    /// `enter_fn` が返した値を渡して関数本体の検査を抜ける。
    pub(super) fn exit_fn(&mut self, prev: Option<String>) {
        self.current_fn_name = prev;
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

    /// `for`/`while` 式の本体に入る。
    pub(super) fn enter_loop_expr(&mut self) {
        self.block_return_forbidden_depth += 1;
    }

    /// `for`/`while` 式の本体を抜ける。
    pub(super) fn exit_loop_expr(&mut self) {
        self.block_return_forbidden_depth -= 1;
    }
}
