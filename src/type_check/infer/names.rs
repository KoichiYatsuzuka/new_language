// type_check/infer/names.rs — 名前の検査: 未定義の名前・裸のクラスメンバー・呼べない外部関数。

use super::*;

impl TypeChecker {
    /// 今の Arrow から正しく呼べない C/C++ の関数を使っていたら誤りにする（ffi_value_exchange_plan.md 1-2）。
    ///
    /// `spelled` は束縛の綴り（fp.f / f）、`binding` はその先頭の名前（fp / f。束縛の深さを見て、同じ綴りの別の束縛を
    /// 取り違えない）。呼び出しに限らず、値として持ち出すのも誤り
    /// （どの使い方でも、戻り値が壊れる・落ちる・書き込みが失われる関数なので）。
    pub(super) fn check_ffi_unsupported_use(&mut self, spelled: &str, binding: &str, span: Option<Span>) {
        let Some((depth, shown, reason, what)) = self.ffi_unsupported.get(spelled) else { return };
        if self.state.lookup_depth(binding) != Some(*depth) {
            return;
        }
        let (function, reason, what) = (shown.clone(), reason.clone(), what.to_string());
        self.report_error(StaticTypeError {
            // 印の付いたモジュールの関数は C/C++ と Rust のスタブ（クラスのメソッドは `TypeRegistry::ffi_unsupported_method` が引く）
            kind: TypeErrorKind::FfiUnsupported { what, function, reason },
            span,
        });
    }

    /// 今のスコープに無い名前を読み書きしたとき、それが**ここから見えない**なら誤りにする（10-12・name_scope_plan.md の 1-4）。
    ///
    /// 規則は `type_check::names` の doc。次のどれかなら通す: 実行時の組み込みの名前・クラス / trait / protocol /
    ///   テンプレートの名前・今見えている型変数・`Self`・`_`、それと**後で実行される本体の中から読む、このモジュールの
    ///   最上位の束縛**（前方参照）。import 先を読めていない環境（エディタ）では見ない。
    /// ⚠ 関数の名前を特別扱いしない。最上位の関数は宣言した時点からスコープにあり（`check_fn_def`）、
    ///   その前に読めるのは後で実行される本体の中だけ（実行時も関数を巻き上げない）。
    /// 戻り値は**見えるか**（通したら `true`・誤りにしたら `false`。判定しない環境では `true`）。
    pub(in crate::type_check) fn check_name_defined(&mut self, name: &str) -> bool {
        if self.registry_incomplete {
            return true;
        }
        if self.already_reported_undefined(name) {
            return false;
        }
        if let Some((class_name, spelled)) = self.bare_class_member(name) {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::BareClassMember { name: name.to_string(), class_name, spelled },
                span: None,
            });
            return false;
        }
        let forward_ref = self.state.in_deferred_body()
            && self.module_toplevel.last().is_some_and(|t| t.contains(name));
        if forward_ref
            || crate::type_check::names::is_runtime_builtin_name(name)
            || name == "Self"
            // `case _:` の `_`（どれにも当たる腕）。束縛ではないので宣言の集合には入らない。
            || name == "_"
            || self.state.is_type_param(name)
            || self.registry.is_known_class(name)
            || self.registry.is_known_trait(name)
            || self.registry.is_protocol(name)
            || self.registry.template_params(name).is_some()
        {
            return true;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::UndefinedName { name: name.to_string() },
            span: None,
        });
        false
    }

    /// 同じ文で、同じ名前の「見えない名前」の誤りをもう出したか（name_scope_plan.md の 1-4・R-2）。
    ///
    /// 以前は誤りの位置が文の位置だったので、`n = n + 1` の左辺と右辺、`mh.enable(1)` を受け手と呼び出しの
    /// 2 経路で推論する形が、同じ誤りを 2 回出していた。
    /// ⚠ 誤りは名前の位置を指す（diagnostic_position_plan.md の 2-2）ので、誤りの位置ではなく
    ///   **報告したときの文の位置**（`Diagnostics::errors_with_stmt`）で比べる（2-3）。同じ文を 2 回検査しても
    ///   1 件のまま。⚠ 横に別の表を持たないのは、型だけが欲しい推論が誤りを捨てる（`Diagnostics::rollback`）と
    ///   表だけが残り、本物の誤りを「もう出した」と取り違えるため。
    pub(super) fn already_reported_undefined(&self, name: &str) -> bool {
        let Some(pos) = self.stmt_pos.as_ref() else { return false };
        self.diags.errors_with_stmt().any(|(e, at)| {
            let same_name = match &e.kind {
                TypeErrorKind::UndefinedName { name: n } | TypeErrorKind::BareClassMember { name: n, .. } => n == name,
                _ => false,
            };
            same_name && at.is_some_and(|s| s.file == pos.file && s.line == pos.line && s.col == pos.col)
        })
    }

    /// 見えない名前 `name` が、検査中のクラスの**メンバー**なら（クラス名, 書くべき形）（name_scope_plan.md の 1-3）。
    ///
    /// メンバーはクラスのスコープに宣言しない（`CheckState::in_class_body`）ので、メソッドの中の裸のフィールド名は
    /// ここに来る。⚠ 1-3 の当時は、プログラムのどこかで束縛される名前を全部通していた（`__init__` の同名の
    /// 引数などで通ってしまう）ので、それより先に見ていた。今も「何が違うか」を添えるために先に見る。
    /// ⚠ モジュールの最上位の束縛と同じ名前なら対象外（メソッドから同じ名前の最上位の変数を読む・前方参照は正しい）。
    /// ⚠ メンバーの顔ぶれが未確定のクラス（未展開の装飾子）も対象外。
    fn bare_class_member(&self, name: &str) -> Option<(String, String)> {
        let class = self.state.current_class()?;
        if self.module_toplevel.last().is_some_and(|t| t.contains(name)) || self.registry.members_unresolved(class) {
            return None;
        }
        let spelled = if let Some((kind, _)) =
            self.registry.class_field_details(class).and_then(|f| f.get(name))
        {
            match kind {
                FieldKind::Const | FieldKind::StaticMut => format!("{class}.{name}"),
                FieldKind::Mut | FieldKind::Let => format!("self.{name}"),
            }
        } else if self.registry.class_methods(class).is_some_and(|m| m.contains_key(name)) {
            format!("self.{name}")
        } else {
            return None;
        };
        Some((class.to_string(), spelled))
    }
}
