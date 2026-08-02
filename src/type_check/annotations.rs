// type_check/annotations.rs — AST 型解決層の注釈（タスク #16, 段階(a)）。
//
// 型検査器が `infer` で計算する型を、node-id 索引の 2 直交テーブル＋型インターン表へ**永続化**する。
// これを VM（plan A: 検査省略）・ツリーウォーク・ネイティブ codegen（#13）が共通に消費して、
// 「型が確定しているか？どの経路か？」等の実行時条件分岐を AST 段階で解消する（挙動統一）。
//
// - node-id: パーサが annotatable な Expr へ per-module で採番する（`Expr::*.node_id`）。0 = 未採番。
// - 型は `InferredType` をインライン展開せず**型インターン表への index（`TypeId`）**で持つ
//   （AST 軽量・比較高速・ネイティブの型記述子テーブル生成と直結）。

use super::types::InferredType;
use std::collections::HashMap;

/// 型インターン表への index（#16）。`AstAnnotations::intern` の位置を指す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

/// 実行時型検査の指示（#16・検査指示テーブルの値）。
// 段階(a): 現状は生成＋テストで消費。ランタイム/codegen での消費は段階(b)/(c)。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum Directive {
    /// 検査不要（静的に型が確定している）。テーブルに載せない場合の既定でもある。
    None,
    /// 使用/呼び出しの前に対象型で**動的検査**する（`mustbe` / cast / FFI・非コンパイル境界 /
    /// `Any`・`Protocol` 消費点）。ネイティブでは呼び出し前にインライン検査を生成する。
    CheckBefore(TypeId),
}

/// AST 型解決層の注釈（#16）。node-id 索引の 2 直交テーブル ＋ 型インターン表。
/// 型検査の走査中に充填し、検査後は**読み取り専用**として各バックエンドが消費する。
#[derive(Debug, Default)]
pub struct AstAnnotations {
    /// 型インターン表: index → `InferredType`（重複排除・per-module）。
    intern: Vec<InferredType>,
    /// 解決型テーブル: node_id → `TypeId`。値の型が `Any`/`Protocol`/`Unresolved` なら「動的」を意味する。
    resolved: HashMap<u32, TypeId>,
    /// 検査指示テーブル: node_id → `Directive`。載っていない node は `Directive::None` 相当。
    directives: HashMap<u32, Directive>,
}

// 段階(a): reader（type_of/resolved/directive/intern_len）はテストで消費。段階(b)/(c) で
// ランタイム/codegen が消費する。intern/set_* は型検査（infer_mustbe）で既に使用中。
#[allow(dead_code)]
impl AstAnnotations {
    /// 型を型インターン表へ登録して `TypeId` を返す（重複は既存 index を再利用）。
    pub fn intern(&mut self, ty: InferredType) -> TypeId {
        if let Some(i) = self.intern.iter().position(|t| *t == ty) {
            return TypeId(i as u32);
        }
        let id = self.intern.len() as u32;
        self.intern.push(ty);
        TypeId(id)
    }

    /// `TypeId` から `InferredType` を引く。
    pub fn type_of(&self, id: TypeId) -> Option<&InferredType> {
        self.intern.get(id.0 as usize)
    }

    /// node の解決型を記録する（node_id==0＝未採番は無視）。
    pub fn set_resolved(&mut self, node_id: u32, ty: InferredType) {
        if node_id == 0 {
            return;
        }
        let tid = self.intern(ty);
        self.resolved.insert(node_id, tid);
    }

    /// node の検査指示を記録する（node_id==0 は無視）。
    pub fn set_directive(&mut self, node_id: u32, dir: Directive) {
        if node_id == 0 {
            return;
        }
        self.directives.insert(node_id, dir);
    }

    /// node の解決型（`TypeId`）を引く。
    pub fn resolved(&self, node_id: u32) -> Option<TypeId> {
        self.resolved.get(&node_id).copied()
    }

    /// node の検査指示を引く（未登録は `Directive::None`）。
    pub fn directive(&self, node_id: u32) -> Directive {
        self.directives.get(&node_id).cloned().unwrap_or(Directive::None)
    }

    /// 型インターン表の長さ（テスト・デバッグ用）。
    pub fn intern_len(&self) -> usize {
        self.intern.len()
    }
}
