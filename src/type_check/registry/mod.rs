// type_check/registry/mod.rs — 宣言（クラス/trait/protocol/関数）の索引。
//
// Phase 5A で `TypeChecker` から切り出したサブ構造体のひとつ。
// 依存グラフ上は**葉**であり、`CheckState` / `Diagnostics` を一切参照しない。
//
// **不変条件: このレジストリは収集パスで一度組み立てたら、以降は読み取り専用。**
// 構築は `builder::TypeRegistryBuilder` だけが行い、`build()` を通ってここに来た後は
// フィールドを書き換える手段が存在しない（`&self` のゲッターしか公開しない）。
// この不変条件のおかげで、検査中に「まだ登録されていないクラス」を参照して
// 結果が実行順に依存する、といった事故が型レベルで起きない。
//
// ここに検査ロジック（エラーを出す判断）を置いてはならない。索引を引くだけ。

pub(super) mod builder;

use std::collections::{HashMap, HashSet};

use crate::ast::{Accessibility, FieldKind};

use super::types::{FnSig, InferredType, ProtocolInfo};

/// クラス・trait・protocol・関数の宣言情報。収集パス後は不変。
pub(super) struct TypeRegistry {
    /// トップレベルおよびネストした関数のシグネチャ。キー: 関数名 → オーバーロード候補。
    fn_sigs: HashMap<String, Vec<FnSig>>,
    /// クラスメソッドのシグネチャ。キー: クラス名 → (メソッド名 → 候補)。
    class_method_sigs: HashMap<String, HashMap<String, Vec<FnSig>>>,
    /// トレイトメソッドのシグネチャ（Intersection 適合チェックで使用）。
    trait_method_sigs: HashMap<String, HashMap<String, Vec<FnSig>>>,
    /// トレイトフィールドの詳細（Intersection 適合チェックで使用）。
    trait_field_details: HashMap<String, HashMap<String, (FieldKind, InferredType)>>,
    /// パース済みクラス・new_type 名の集合。`NamedInstance` の解決に使用する。
    known_class_names: HashSet<String>,
    /// `new_type Name: Original` の元の型名。キー: 新しい型名 → 元の型名。
    new_type_originals: HashMap<String, String>,
    /// クラスの基底クラス・トレイト名。継承チェック・protected アクセス検査に使用。
    class_bases: HashMap<String, Vec<String>>,
    /// クラスフィールドの可変フラグ。キー: クラス名 → (フィールド名 → 可変か)。
    class_fields: HashMap<String, HashMap<String, bool>>,
    /// クラスフィールドの詳細（種別・型）。Protocol 適合チェックで使用する。
    class_field_details: HashMap<String, HashMap<String, (FieldKind, InferredType)>>,
    /// クラスメンバーのアクセス可能性。`Public` 以外のみ格納。
    class_member_access: HashMap<String, HashMap<String, Accessibility>>,
    /// `static fn` で定義されたスタティックメソッド名。
    class_static_methods: HashMap<String, HashSet<String>>,
    /// プロトコル定義。プロトコル名 → `ProtocolInfo`。
    known_protocols: HashMap<String, ProtocolInfo>,
}

impl TypeRegistry {
    // ── 関数 ──────────────────────────────────────────────────────────────────

    /// 関数名のオーバーロード候補。
    pub(super) fn fn_sigs(&self, name: &str) -> Option<&Vec<FnSig>> {
        self.fn_sigs.get(name)
    }

    // ── クラス ────────────────────────────────────────────────────────────────

    /// クラス・enum・new_type として登録済みの名前か。
    pub(super) fn is_known_class(&self, name: &str) -> bool {
        self.known_class_names.contains(name)
    }

    /// クラスのメソッド表（メソッド名 → オーバーロード候補）。
    pub(super) fn class_methods(&self, class: &str) -> Option<&HashMap<String, Vec<FnSig>>> {
        self.class_method_sigs.get(class)
    }

    /// クラスの基底クラス・トレイト名。
    pub(super) fn class_bases(&self, class: &str) -> Option<&[String]> {
        self.class_bases.get(class).map(|v| v.as_slice())
    }

    /// クラスのフィールド詳細（種別・型）。
    pub(super) fn class_field_details(
        &self,
        class: &str,
    ) -> Option<&HashMap<String, (FieldKind, InferredType)>> {
        self.class_field_details.get(class)
    }

    /// `class` が `field` という名前のフィールドを持つか。
    pub(super) fn has_field(&self, class: &str, field: &str) -> bool {
        self.class_fields
            .get(class)
            .is_some_and(|f| f.contains_key(field))
    }

    /// `class.field` が `mut` 宣言か。フィールドが存在しなければ `None`。
    pub(super) fn field_is_mutable(&self, class: &str, field: &str) -> Option<bool> {
        self.class_fields.get(class)?.get(field).copied()
    }

    /// `class.member` のアクセス可能性。未登録のメンバーは `Public` 扱い。
    pub(super) fn member_access(&self, class: &str, member: &str) -> Accessibility {
        self.class_member_access
            .get(class)
            .and_then(|m| m.get(member))
            .cloned()
            .unwrap_or(Accessibility::Public)
    }

    /// `class.method` が `static fn` として定義されているか。
    pub(super) fn is_static_method(&self, class: &str, method: &str) -> bool {
        self.class_static_methods
            .get(class)
            .is_some_and(|s| s.contains(method))
    }

    // ── trait ─────────────────────────────────────────────────────────────────

    /// トレイトのメソッド表。
    pub(super) fn trait_methods(&self, name: &str) -> Option<&HashMap<String, Vec<FnSig>>> {
        self.trait_method_sigs.get(name)
    }

    /// トレイトのフィールド詳細。
    pub(super) fn trait_field_details(
        &self,
        name: &str,
    ) -> Option<&HashMap<String, (FieldKind, InferredType)>> {
        self.trait_field_details.get(name)
    }

    // ── protocol / new_type ───────────────────────────────────────────────────

    /// プロトコルとして登録済みの名前か。
    pub(super) fn is_protocol(&self, name: &str) -> bool {
        self.known_protocols.contains_key(name)
    }

    /// プロトコル定義。
    pub(super) fn protocol(&self, name: &str) -> Option<&ProtocolInfo> {
        self.known_protocols.get(name)
    }

    /// `new_type Name: Original` の元の型名。
    pub(super) fn new_type_original(&self, name: &str) -> Option<&str> {
        self.new_type_originals.get(name).map(|s| s.as_str())
    }
}
