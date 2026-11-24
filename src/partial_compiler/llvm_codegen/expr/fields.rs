// llvm_codegen/expr/fields.rs — 受け手のクラスの決定とフィールドの読み書き。

use super::*;

/// メソッド呼び出しの受け手のクラス（[`GenCtx::receiver_class`] の答え・3-1）。
pub(in crate::partial_compiler::llvm_codegen) struct Receiver {
    pub(in crate::partial_compiler::llvm_codegen) class: String,
    /// 型検査の注釈で決めた（実行時に class_id を確かめてから直接呼ぶ・3-2・D-2）
    pub(super) guarded: bool,
}

/// 型検査が型を記録する式の node-id（受け手の注釈を引くため・3-1）。0 は未採番（合成 AST）。
fn expr_node_id(e: &Expr) -> Option<u32> {
    match e {
        Expr::Ident { node_id, .. }
        | Expr::Attr { node_id, .. }
        | Expr::Call { node_id, .. }
        | Expr::Subscript { node_id, .. }
        | Expr::Cast { node_id, .. } => Some(*node_id),
        _ => None,
    }
}

impl<'a> GenCtx<'a> {
    /// メソッド呼び出しのレシーバ式から**クラス**を引く（`obj.m()` の `obj`）。
    ///
    /// - `self` … 現在コンパイル中のクラス（ガード不要）
    /// - 型注釈付きの仮引数（`param_classes`）… その型（ガード不要: 引数は型検査が呼び出しで確かめる。`Any` は渡せない）
    /// - それ以外（フィールド `self.pos`・ループ変数・注釈付きのローカル・`int` / `float` のフィールドを持たないクラスの
    ///   仮引数など）… 型検査の注釈（`resolved_type` が Arrow のクラスの `NamedInstance`）。**実行時のガード付き**
    ///   （3-1・3-2・D-2: 注釈はヒントなので、外れたら名前による呼び出しへ回る）。注釈が無い／`node_id` 0 は不明
    ///
    /// ⚠ #64 以前は `gen_call_expr` と `gen_call` に**逐語で同じ `match`** が置かれていた。
    /// ⚠ 3-1 以前は `self` と `int` / `float` のフィールドを持つクラスの仮引数しか引けず、ほかは全部名前で呼んでいた。
    pub(in crate::partial_compiler::llvm_codegen) fn receiver_class(&self, object: &Expr) -> Option<Receiver> {
        match object {
            e if ident_name(e) == Some("self") => {
                self.current_class.clone().map(|class| Receiver { class, guarded: false })
            }
            e if ident_name(e).is_some_and(|n| self.param_classes.contains_key(n)) => {
                let class = self.param_classes[ident_name(e).unwrap()].clone();
                Some(Receiver { class, guarded: false })
            }
            _ => {
                use crate::type_check::InferredType;
                let nid = expr_node_id(object).filter(|&n| n != 0)?;
                match self.annotations.resolved_type(nid)? {
                    InferredType::NamedInstance(n) if self.annotations.is_arrow_class(n) => {
                        Some(Receiver { class: n.clone(), guarded: true })
                    }
                    _ => None,
                }
            }
        }
    }

    /// `obj.attr` の読み出し（受け手は評価済みのハンドル `h`）。数値のフィールドで、受け手のクラスがこのモジュールの
    /// raw レイアウトのクラスと分かるなら番地から直接読む（copy_alloc_speed_plan.md 8-3）。ほかは IC 付きのコールバック。
    /// ⚠ 受け手のクラスは注釈・仮引数の型から決めたヒントで、実行時に合っているかは番地を返すコールバックが確かめる。
    pub(in crate::partial_compiler::llvm_codegen) fn emit_field_read(&mut self, object: &Expr, h: &str, attr: &str, known_ty: Option<Ty>) -> (String, Ty) {
        if let Some(ty @ (Ty::Int | Ty::Float)) = known_ty {
            if let Some(recv) = self.receiver_class(object) {
                if let Some(r) = self.emit_direct_read(&recv.class, BlockSrc::Handle(h), attr, ty) {
                    return r;
                }
            }
        }
        self.emit_attr_read(h, attr, known_ty)
    }

    /// `a.b.x` の読み出し（受け手 `object` が属性 `a.b` で、まだ評価していない）: `a.b` のクラスがこのモジュールの raw レイアウトの
    /// クラスで `x` が数値のフィールドなら、`a.b` をアリーナに積まず、1 回のコールバックで番地を取って読む
    /// （copy_alloc_speed_plan.md 8-7）。当てはまらなければ何も出さずに `None`（呼ぶ側が受け手を評価して今までの道を使う）。
    /// ⚠ 判定はコードを出す前に済ませる（親を評価した後で引き返すと、親を 2 回評価することになる）。
    pub(super) fn emit_nested_field_read(&mut self, object: &Expr, attr: &str, known_ty: Option<Ty>) -> Option<(String, Ty)> {
        let ty = known_ty.filter(|t| matches!(t, Ty::Int | Ty::Float))?;
        let Expr::Attr { object: parent, attr: pattr, .. } = object else { return None };
        let class = self.receiver_class(object)?.class;
        self.direct_read_plan(&class, attr, ty)?;
        let (pv, pt) = self.gen_expr(parent);
        let ph = self.to_handle(&pv, pt);
        self.emit_direct_read(&class, BlockSrc::Field { parent_h: &ph, attr: pattr }, attr, ty)
    }

    /// `obj.attr = v` の書き込み（受け手は評価済みのハンドル `oh`・値は評価済み）。`int` / `float` の値を、このモジュールの
    /// raw レイアウトのクラスの `mut` の数値のフィールドへ書くなら番地へ直接書く（copy_alloc_speed_plan.md 8-4）。
    /// ほかは今までのコールバック（[`Self::emit_attr_assign_typed`]）。
    pub(in crate::partial_compiler::llvm_codegen) fn emit_field_assign(&mut self, object: &Expr, oh: &str, attr: &str, v: &str, vt: Ty) {
        if matches!(vt, Ty::Int | Ty::Float) {
            if let Some(recv) = self.receiver_class(object) {
                if self.emit_direct_write(&recv.class, BlockSrc::Handle(oh), attr, v, vt) {
                    return;
                }
            }
        }
        self.emit_attr_assign_typed(oh, attr, v, vt);
    }
}
