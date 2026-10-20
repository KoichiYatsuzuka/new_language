// python_converter/supers.rs — `super()` の脱糖に使う「変換中クラスの名前と第 1 基底」の保持。
//
// ★ Python の `super().m(args)` は Arrow に相当構文が無い。クラス継承そのものは
// インタープリタ側（`exec_class_def` の `import[py]` 限定分岐）で成立させたので、
// ここでは **`super().m(args)` → `<第1基底>.m(self, args)`** に**変換時に**書き換える。
// 受け側（クラス経由のアンバウンド呼び出し）は `classes/class_methods.rs` が
// `FnValue::is_python` 限定で許可している。
// 2 引数の `super(C, self).m(args)` は `C` が変換中のクラスのときだけ同じに書き換える（タスク 6-2）。
//
// ⚠ 基底名を `convert_expr` まで引数で引き回すとシグネチャ変更が全域に及ぶため、
//   変換中だけ有効なスレッドローカルのスタックで持つ。変換器は 1 スレッドで動く。
//   （入れ子クラスでも push/pop が対応するようスタックにしてある。）

use std::cell::RefCell;

thread_local! {
    /// 変換中クラスの（素の名前, 第 1 基底名）。基底なしのクラスでは基底に `None` を積む。
    static SUPER_BASE: RefCell<Vec<(String, Option<String>)>> = const { RefCell::new(Vec::new()) };
}

/// `convert_class` の間だけクラスの名前と第 1 基底を積む RAII ガード。
pub(crate) struct SuperBaseGuard;

impl SuperBaseGuard {
    pub(crate) fn push(class_name: String, base: Option<String>) -> Self {
        SUPER_BASE.with(|s| s.borrow_mut().push((class_name, base)));
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
    SUPER_BASE.with(|s| s.borrow().last().and_then(|(_, b)| b.clone()))
}

/// 現在変換中のクラスの素の名前（入れ子でも `Inner`）。クラスの外なら `None`（`super(C, self)` の `C`・タスク 6-2）。
pub(crate) fn current_super_class() -> Option<String> {
    SUPER_BASE.with(|s| s.borrow().last().map(|(c, _)| c.clone()))
}

/// クラスの変換中か（基底が無いクラスも含む）。基底が無いクラスの `super()` は `object` を指す（タスク 6-10）。
pub(crate) fn inside_class() -> bool {
    SUPER_BASE.with(|s| !s.borrow().is_empty())
}
