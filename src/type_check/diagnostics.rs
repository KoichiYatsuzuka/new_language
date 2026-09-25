// type_check/diagnostics.rs — 収集された静的型エラー・警告の保持。
//
// Phase 5A で `TypeChecker` から切り出したサブ構造体のひとつ。
// 依存グラフ上は**葉**であり、`TypeRegistry` / `CheckState` を一切参照しない。
// この性質を保つため、ここに検査ロジック（「条件を調べてエラーを積む」処理）を
// 置いてはならない。積むかどうかの判断は呼び出し側（TypeChecker）の責務。

use super::errors::{StaticTypeError, StaticTypeWarning};

/// 型検査中に収集された診断。`TypeChecker::check*` の最終段で取り出される。
#[derive(Default)]
pub(super) struct Diagnostics {
    errors: Vec<StaticTypeError>,
    /// `errors` と同じ長さ。その誤りが**単相化した具体化の本体**で見つかったか（タスク 2-8 段階 2）。
    ///
    /// ⚠ 重複をまとめる対象はこちらが真のものだけ。テンプレートと関係の無い本物の重複
    /// （別々の箇所の同じ文面）まで消さないように。
    from_instance: Vec<bool>,
    warnings: Vec<StaticTypeWarning>,
}

impl Diagnostics {
    /// 静的型エラーを追加する。
    pub(super) fn report_error(&mut self, err: StaticTypeError, from_instance: bool) {
        self.errors.push(err);
        self.from_instance.push(from_instance);
    }

    /// 静的型警告を追加する。
    pub(super) fn report_warning(&mut self, w: StaticTypeWarning) {
        self.warnings.push(w);
    }

    /// 収集結果を `(エラー, 警告)` として取り出す。
    pub(super) fn into_parts(self) -> (Vec<StaticTypeError>, Vec<StaticTypeWarning>) {
        (self.errors, self.warnings)
    }

    /// [`Diagnostics::into_parts`] と同じだが、誤りごとの「具体化の本体で見つかったか」も返す。
    pub(super) fn into_parts_tagged(
        self,
    ) -> (Vec<(StaticTypeError, bool)>, Vec<StaticTypeWarning>) {
        (self.errors.into_iter().zip(self.from_instance).collect(), self.warnings)
    }
}
