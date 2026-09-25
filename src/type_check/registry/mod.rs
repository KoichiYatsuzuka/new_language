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
    /// **テンプレートの具体化 → 展開器が単相化したクラス名**（タスク 2-8 段階 2）。
    ///
    /// キーは型の表示形（`GenericInstance` の `to_string()`・例 `"Box[int]"`）。値は AST 上の
    /// クラス名。ふつうは同じ文字列だが、型引数の書き方の揺れ（`dict[str,int]` と
    /// `dict[str, int]`）を吸収するために、キーは型として読み直した表示形にしてある。
    instance_classes: HashMap<String, String>,
    /// 単相化で作った宣言の名前（クラス・関数・`gen`。`Box[int]` / `ident[str]`）。
    /// 同じ誤りの重複報告をまとめるのに使う（`TypeChecker::merge_instance_errors`）。
    instance_names: HashSet<String>,
    /// 未展開の `!装飾子` を本体に残しているクラス（タスク 2-4）。
    ///
    /// ⚠ 装飾子が何を足すか分からないので、このクラスの**メンバーの顔ぶれは未確定**。
    /// 通常ビルドでは展開器が型検査の前に走る（2-2）ので常に空。
    classes_with_unexpanded_decorators: HashSet<String>,
    /// `new_type Name: Original` の元の型名。キー: 新しい型名 → 元の型名。
    new_type_originals: HashMap<String, String>,
    /// クラスの基底クラス・トレイト名。継承チェック・protected アクセス検査に使用。
    class_bases: HashMap<String, Vec<String>>,
    /// **基底 trait ごとの具体型引数**（タスク 9.9）。キー: クラス名 → 基底名 → 型引数。
    ///
    /// ⚠⚠ これが無いと `trait Holder[T]` を継承したクラスのフィールド型が
    /// **`T` のまま**になる（置換表が作れない）。以前はパーサが同じ情報を持ちながら
    /// AST へ載せずに捨てていた。
    class_base_args: HashMap<String, HashMap<String, Vec<String>>>,
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
    /// **Arrow ソースで `class` 宣言された**クラス名（#27-a）。
    ///
    /// `known_class_names` との違いが要点: あちらは**外部言語のスタブ由来クラスも含む**
    /// （`import[cs-dll]` 等はパース時に `Stmt::ClassDef` へ変換されるため）。
    /// スタブ由来クラスの実行時表現は `Value::CsObject` などで **`Value::Instance` ではない**ので、
    /// 「このインスタンスは Arrow のクラスだ」と断定してよいのはこちらの集合だけ。
    ///
    /// 用途: VM コンパイラがメソッド呼び出し・属性代入のレシーバを
    /// `Value::Instance` 前提の op へ落としてよいかの判定（#26/#27-a）。
    arrow_class_names: HashSet<String>,
    /// テンプレート宣言の**型変数名**（宣言順）。キー: クラス名 / 関数名。
    ///
    /// ⚠ これが無いと `Box[int]("s")` のような**実体化呼び出しの引数型を検査できない**。
    /// 呼び出し点で `T` → `int` の置換表を作るのに、型変数の**名前と並び**が要る。
    /// 以前は型変数名がレジストリに無く、`Expr::TemplateInstantiate` の呼び出しは
    /// `func_name` が `None` になって検査経路に入らないまま素通りしていた。
    template_params: HashMap<String, Vec<String>>,
}

impl TypeRegistry {
    // ── 関数 ──────────────────────────────────────────────────────────────────

    /// 関数名のオーバーロード候補。
    pub(super) fn fn_sigs(&self, name: &str) -> Option<&Vec<FnSig>> {
        self.fn_sigs.get(name)
    }

    // ── クラス ────────────────────────────────────────────────────────────────

    /// Arrow ソース由来クラス名の集合（注釈テーブルへ渡す・#27-a）。
    pub(super) fn arrow_class_names(&self) -> &HashSet<String> {
        &self.arrow_class_names
    }

    /// テンプレート宣言の型変数名（宣言順）。非テンプレートは `None`。
    pub(super) fn template_params(&self, name: &str) -> Option<&[String]> {
        self.template_params.get(name).map(|v| v.as_slice())
    }

    /// テンプレートの具体化（`Box[int]` の表示形）に対応する**単相化したクラス名**（タスク 2-8 段階 2）。
    ///
    /// ⚠⚠ これがあれば、そのクラスは**普通のクラスとして**検査できる（メソッド引数・戻り値・
    /// メンバー・アクセス制御がすべて普通の経路を通る）。無いとき（宣言より前の具体化・
    /// 制約付きテンプレート・展開しないエディタ）は従来どおりテンプレートと置換表で扱う。
    pub(super) fn instance_class(&self, display: &str) -> Option<&String> {
        self.instance_classes.get(display)
    }

    /// 単相化で作った宣言の名前（タスク 2-8 段階 2）。
    pub(super) fn instance_names(&self) -> &HashSet<String> {
        &self.instance_names
    }

    /// クラス・enum・new_type として登録済みの名前か。
    pub(super) fn is_known_class(&self, name: &str) -> bool {
        self.known_class_names.contains(name)
    }

    /// クラスのメソッド表（メソッド名 → オーバーロード候補）。
    pub(super) fn class_methods(&self, class: &str) -> Option<&HashMap<String, Vec<FnSig>>> {
        self.class_method_sigs.get(class)
    }

    /// クラスの基底クラス・トレイト名。
    /// `class` が基底 `base`（trait）へ渡した**具体型引数**（タスク 9.9）。
    ///
    /// ⚠ 型引数を書かなかった／trait がテンプレートでない場合は空スライス。
    /// 「引数が無い」と「そもそも基底でない」を区別したいときは `class_bases` を見ること。
    pub(super) fn class_base_args(&self, class: &str, base: &str) -> &[String] {
        self.class_base_args
            .get(class)
            .and_then(|m| m.get(base))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub(super) fn class_bases(&self, class: &str) -> Option<&[String]> {
        self.class_bases.get(class).map(|v| v.as_slice())
    }

    /// クラスのフィールド詳細（種別・型）。
    /// **メンバーの顔ぶれが未確定なクラス**か（タスク 2-4）。
    ///
    /// ⚠⚠ クラス本体に未展開の `!装飾子` が残っていると、装飾子が何を足すか分からない。
    /// ⇒ そのクラスに対する**メンバー存在検査は答えを持たない**。
    /// 通常ビルドでは展開器が型検査の前に走る（2-2）のでここは常に空になるが、
    /// エディタ用 wasm は**わざと展開しない**（設計書 5-0）ので実際に効く。
    /// ⚠ 「編集中だから」ではなく「**このクラスのメンバーが未確定だから**」で止める。
    pub(crate) fn members_unresolved(&self, class_name: &str) -> bool {
        self.classes_with_unexpanded_decorators.contains(class_name)
    }

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

    /// トレイトとして登録済みの名前か（タスク 3.4 の妥当性検査で使う）。
    ///
    /// ⚠ メンバーが 0 個の trait もあるので、**フィールド表とメソッド表のどちらかに
    /// エントリがあれば trait** と判定する。
    pub(super) fn is_known_trait(&self, name: &str) -> bool {
        self.trait_field_details.contains_key(name) || self.trait_method_sigs.contains_key(name)
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
