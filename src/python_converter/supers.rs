// python_converter/supers.rs — `super()` の脱糖に使う「変換中クラスの第 1 基底」の保持。
//
// ★ Python の `super().m(args)` は Arrow に相当構文が無い。クラス継承そのものは
// インタープリタ側（`exec_class_def` の `import[py]` 限定分岐）で成立させたので、
// ここでは **`super().m(args)` → `<第1基底>.m(self, args)`** に**変換時に**書き換える。
// 受け側（クラス経由のアンバウンド呼び出し）は `classes/class_methods.rs` が
// `FnValue::is_python` 限定で許可している。
//
// ⚠ 基底名を `convert_expr` まで引数で引き回すとシグネチャ変更が全域に及ぶため、
//   変換中だけ有効なスレッドローカルのスタックで持つ。変換器は 1 スレッドで動く。
//   （入れ子クラスでも push/pop が対応するようスタックにしてある。）

use std::cell::RefCell;

thread_local! {
    /// 変換中クラスの第 1 基底名（基底なしのクラスでは `None` を積む）。
    static SUPER_BASE: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
}

/// `convert_class` の間だけ第 1 基底を積む RAII ガード。
pub(crate) struct SuperBaseGuard;

impl SuperBaseGuard {
    pub(crate) fn push(base: Option<String>) -> Self {
        SUPER_BASE.with(|s| s.borrow_mut().push(base));
        SuperBaseGuard
    }
}

impl Drop for SuperBaseGuard {
    fn drop(&mut self) {
        SUPER_BASE.with(|s| {
            s.borrow_mut().pop();
        });
    }
}

/// 現在変換中のクラスの第 1 基底名を返す。クラスの外、または基底なしなら `None`。
pub(crate) fn current_super_base() -> Option<String> {
    SUPER_BASE.with(|s| s.borrow().last().cloned().flatten())
}
