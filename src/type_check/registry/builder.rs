// type_check/registry/builder.rs — 収集パス。AST を先行スキャンして `TypeRegistry` を組み立てる。
//
// Phase 5A-3b で `TypeChecker` の関数シグネチャ収集（旧 `collect_fn_sigs`）をここへ移設したもの。
// **レジストリへ書き込めるのはこのファイルだけ**であり、`build()` を通した後は
// 読み取り専用の `TypeRegistry` になる（registry/mod.rs の不変条件）。
//
// ここでは診断（エラー・警告）を一切報告しない。収集パスは「宣言を索引化する」
// だけの責務で、`Diagnostics` に依存しない（依存グラフを一方向に保つため）。

use std::collections::{HashMap, HashSet};

use crate::ast::{Accessibility, FieldKind, Param, Stmt};

use super::super::types::{FnSig, InferredType, ProtocolField, ProtocolInfo, ProtocolMethod};
use super::{NameScope, TypeRegistry};

/// 組み込みで登録される例外クラス名。`TypeChecker::new` がグローバルスコープの
/// 束縛を作る際にも使うため公開している。
pub(in crate::type_check) const EXCEPTION_CLASS_NAMES: [&str; 19] = [
    "Exception",
    "ValueError",
    "TypeError",
    "NameError",
    "AttributeError",
    "IndexError",
    "KeyError",
    "ZeroDivisionError",
    "RuntimeError",
    "StopIteration",
    "NotImplementedError",
    "OverflowError",
    "IOError",
    "OSError",
    "AssertionError",
    "ArithmeticError",
    "AccessError",
    // ⚠ `RecursionError` は実行時にだけ発生するが、`except RecursionError:` を
    //   型検査に通すためにここへも登録する。
    //   ⚠⚠ `built_in_types.rs` の一覧と `exceptions.rs` の `CATCHABLE` と**3 つ揃える**こと。
    "RecursionError",
    // `close()` が中断点へ投げ込む（bug_fix.md B13 段階 D）。
    "GeneratorExit",
];

/// 組み込みで登録される new_type（型名 → 元のプリミティブ型名）。
///
/// ⚠ 実行時の登録（`interpreter/built_in_types.rs`）と揃えること。`pointer` は実行時に
///   「`new_type pointer: uint` 相当のラッパークラス」として登録されているのに、ここに無かったので
///   `let p: pointer = id(x)` や `p is pointer` が「知らない型」になっていた（`id` の戻り値の型）。
const BUILTIN_NEW_TYPES: [(&str, &str); 4] =
    [("path", "str"), ("Index", "int"), ("Size", "int"), ("pointer", "uint")];

/// `TypeRegistry` の構築器。`collect` で AST を走査し、`build` で凍結する。
pub(in crate::type_check) struct TypeRegistryBuilder {
    reg: TypeRegistry,
    /// 収集済み import モジュールの `(lang, モジュールパス)`（#16 段階 F）。
    ///
    /// `fn_sigs` は `push` で積むため、同じモジュールを二度収集すると**偽のオーバーロード**が
    /// できてしまう（単一シグネチャ前提の高速パスが崩れる）。同じモジュールが複数箇所から
    /// import される・入れ子 import で再訪する、のどちらも起こるのでここで弾く。
    seen_modules: HashSet<(String, Vec<String>)>,
    /// 収集済みのテンプレートの具体化（`(lang, モジュールパス, 名前)`・タスク 2-16）。
    ///
    /// ⚠ 展開器は具体化を「その枠で最初の `import` 文」の本体に置くので、同じモジュールの
    ///   **2 回目以降の `import` の本体にだけ**ある具体化がある（モジュールが別のモジュールの中で
    ///   先に読み込まれている形）。`seen_modules` で本体ごと読み飛ばすと、それが登録されない。
    seen_instances: HashSet<(String, Vec<String>, String)>,
    /// 外部言語 import の本体を収集中の深さ（#27-a）。
    ///
    /// 0 のときに見た `ClassDef` だけを `arrow_class_names` に載せる。外部言語スタブは
    /// パース時に `Stmt::ClassDef` へ変換されるため、これが無いと C# クラスも
    /// 「Arrow のクラス」に見えてしまう（実際 `event_cs_handler.ar` で
    /// `Value::CsObject` を `Value::Instance` 前提の op に流して落ちた）。
    foreign_depth: u32,
    /// 今集めている文が属する文脈（`TypeRegistry::name_scopes` の添字・0 がメイン・フェーズ10 10-8）。
    /// `import` の別名・`from import` した名前をこの文脈の表に控える。
    cur_scope: usize,
    /// 今集めているモジュールの修飾名（メインは `None`・10-8）。
    module_prefix: Option<String>,
}

/// モジュールの本体を、**宣言を `tags.X` の名前にした写し**にする（フェーズ10 10-8）。
///
/// ⚠⚠ 収集パスが読むためだけの写し（実行時と型検査の本体の検査は元の AST を使う）。
///   ⇒ レジストリの登録箇所を 1 つずつ書き換えずに、モジュールの宣言が修飾名で・シグネチャや
///   フィールドの型も修飾名で入る。
/// - 最上位の宣言の名前（`class` / `trait` / `protocol` / `enum` / `new_type` / `fn` / `gen`・
///   具体化の `Box[int]`）を `prefix.名前` にする。
/// - 本体のすべての型注釈の中の名前を、同じ表で置き換える（テンプレートの置換と同じ走査
///   `template_subst::subst_stmts`）。クラスの基底・`new_type` の元の型も。
/// - そのモジュールの中で `from m import x` した名前は `m.x`、`import m as a` した別名は表に控える。
/// ⚠ 入れ子の `import` の本体は書き換えない（別のモジュール・それぞれの文脈で集める）。
/// ⚠ 名前の置き換えは識別子ごと（`t2.Other` のような別名つきの綴りの `Other` も、同名の宣言があれば
///   置き換わってしまう。モジュールが同名の宣言を持ちつつ別名で別モジュールの同名を指す形だけの穴）。
pub(super) fn qualify_module_body(prefix: &str, body: &[Stmt]) -> (Vec<Stmt>, NameScope) {
    let mut scope = NameScope::default();
    for st in body {
        match st {
            Stmt::ClassDef { name, .. }
            | Stmt::TraitDef { name, .. }
            | Stmt::ProtocolDef { name, .. }
            | Stmt::FnDef { name, .. }
            | Stmt::GenDef { name, .. }
            | Stmt::NewTypeDef { name, .. }
                if !name.contains('[') =>
            {
                scope.names.insert(name.clone(), format!("{prefix}.{name}"));
            }
            Stmt::EnumDef { name, .. } => {
                scope.names.insert(name.clone(), format!("{prefix}.{name}"));
            }
            Stmt::FromImport { lang, module, names, .. } if is_arrow_source_lang(lang) => {
                let m = module.join(".");
                for (orig, alias) in names {
                    scope
                        .names
                        .insert(alias.clone().unwrap_or_else(|| orig.clone()), format!("{m}.{orig}"));
                }
            }
            // ⚠ 束縛名と、そこへ入るモジュールは AST の `bind`（`import a.b` は `a` → パッケージ `a`）。
            Stmt::Import { lang, bind: Some(b), .. } if is_arrow_source_lang(lang) => {
                scope.modules.insert(b.name.clone(), b.module.join("."));
            }
            _ => {}
        }
    }
    let map = &scope.names;
    let q = |s: &str| crate::template_subst::subst_type(s, map);
    let out = body
        .iter()
        .map(|st| {
            if matches!(st, Stmt::Import { .. } | Stmt::FromImport { .. }) {
                return st.clone();
            }
            let mut st = crate::template_subst::subst_stmts(std::slice::from_ref(st), map)
                .pop()
                .unwrap_or_else(|| st.clone());
            match &mut st {
                Stmt::ClassDef { name, bases, .. } => {
                    *name = q(name);
                    for b in bases.iter_mut() {
                        *b = q(b);
                    }
                }
                Stmt::TraitDef { name, .. }
                | Stmt::ProtocolDef { name, .. }
                | Stmt::FnDef { name, .. }
                | Stmt::GenDef { name, .. }
                | Stmt::EnumDef { name, .. } => *name = q(name),
                Stmt::NewTypeDef { name, original } => {
                    *name = q(name);
                    *original = q(original);
                }
                _ => {}
            }
            st
        })
        .collect();
    (out, scope)
}

/// `import[lang]` のうち、**モジュール本体が Arrow ソース**であるものか（#27-a）。
///
/// これが true の import で宣言されたクラスは、実行時も Arrow の `Value::Instance` になる。
/// false（`py`/`cs-*`/`js-*`/`cpp-*`/`rs`）のクラスは `Value::PyObject`/`CsObject` 等になるので、
/// `Value::Instance` を前提とする最適化に載せてはいけない。
///
/// ⚠ **未知のタグは false（保守的）**。新しい言語を足したときに黙って
/// 「Arrow のクラス扱い」になって壊れるより、最適化が効かない方が安全。
/// 現行のタグは `parser/imports/dispatch.rs` の `match lang` が唯一の一覧。
fn is_arrow_source_lang(lang: &str) -> bool {
    crate::module_path::is_arrow_source_lang(lang)
}

/// 展開器が置いたテンプレートの具体化（`Box[int]` のような名前の宣言）なら、その名前（タスク 2-16）。
fn instance_decl_name(st: &Stmt) -> Option<&str> {
    match st {
        Stmt::ClassDef { name, template_params, .. }
        | Stmt::FnDef { name, template_params, .. }
        | Stmt::GenDef { name, template_params, .. }
            if template_params.is_empty() && name.ends_with(']') =>
        {
            Some(name)
        }
        _ => None,
    }
}

impl TypeRegistryBuilder {
    /// 組み込みクラス（例外クラス・`path`/`Index`/`Size`・`slice`）を登録した状態で生成する。
    /// `FnSig` に載せる**通常パラメータ**か（可変長・`**kwargs` は除く）。
    ///
    /// ⚠⚠ `params` / `param_mutable` / `required_count` は**すべてこの述語で絞る**こと（B11）。
    /// 別々に書くと並びがずれ、`mut` パラメータの検査が**別の引数を見る**。
    /// 以前は同じクロージャを 4 箇所に書いていた。
    fn is_normal_param(p: &Param) -> bool {
        !p.variadic && !Self::is_py_kwargs_param(p)
    }

    pub(in crate::type_check) fn with_builtins() -> Self {
        let mut known_class_names: HashSet<String> = HashSet::new();
        let mut new_type_originals: HashMap<String, String> = HashMap::new();
        for (cls_name, prim_type) in BUILTIN_NEW_TYPES {
            known_class_names.insert(cls_name.to_string());
            new_type_originals.insert(cls_name.to_string(), prim_type.to_string());
        }
        known_class_names.insert("slice".to_string());
        // ⚠ `open()` が返す `FileObject` と、`FileOpenMode` などの組み込みの列挙は
        //   `builtins.ars` の宣言から登録する（`collect_builtin_types`）。

        let mut class_bases: HashMap<String, Vec<String>> = HashMap::new();
        // ⚠⚠ **組み込み例外のフィールド型も登録する**（タスク 2.8）。名前だけ登録していたため
        //    `except ValueError as e:` の `e.message` が `Unresolved` になり、
        //    `Unresolved` が `type_matches_exact` の万能受容体であることから
        //      let s: int = e.message    # str を int へ入れて通っていた
        //    のように束縛経由の義務が無効化されていた。
        // ⚠ 内容は実行時の `Error` trait の登録（`interpreter.rs` の `trait_field_order`）と
        //    **揃えること**。片方だけ変えると静的と実行時がずれる。
        let mut exc_fields: HashMap<String, (FieldKind, InferredType)> = HashMap::new();
        for (fname, fty) in [
            ("message", InferredType::Str),
            ("code_context", InferredType::Str),
            ("file", InferredType::Str),
            ("line", InferredType::Int),
            ("col", InferredType::Int),
        ] {
            // ⚠ 実行時は `let`（不変）で登録されている（`trait_field_order` の可変フラグが false）。
            exc_fields.insert(fname.to_string(), (FieldKind::Let, fty));
        }
        let mut class_field_details: HashMap<String, HashMap<String, (FieldKind, InferredType)>> =
            HashMap::new();
        for class_name in EXCEPTION_CLASS_NAMES {
            known_class_names.insert(class_name.to_string());
            class_bases.insert(class_name.to_string(), vec!["Error".to_string()]);
            class_field_details.insert(class_name.to_string(), exc_fields.clone());
        }
        // ⚠⚠ **基底の `Error` 自身にも登録する**（タスク 7.5）。組み込み例外だけに入れていたので、
        //    利用者が `class MyErr(Error)` と書いたときに `e.message` が引けなかった
        //    （`collect_class_field_details` は基底を辿るが、`Error` に何も無かった）。
        known_class_names.insert("Error".to_string());
        class_field_details.insert("Error".to_string(), exc_fields.clone());

        Self {
            reg: TypeRegistry {
                fn_sigs: HashMap::new(),
                class_method_sigs: HashMap::new(),
                trait_method_sigs: HashMap::new(),
                trait_field_details: HashMap::new(),
                known_class_names,
                instance_classes: HashMap::new(),
                instance_names: HashSet::new(),
                classes_with_unexpanded_decorators: HashSet::new(),
                arrow_class_names: HashSet::new(),
                new_type_originals,
                class_bases,
                class_base_args: HashMap::new(),
                class_fields: HashMap::new(),
                class_field_details,
                enum_members: HashMap::new(),
                class_member_access: HashMap::new(),
                class_static_methods: HashMap::new(),
                known_protocols: HashMap::new(),
                template_params: HashMap::new(),
                name_scopes: vec![NameScope::default()],
                module_scope_index: HashMap::new(),
                current_scope: std::cell::Cell::new(0),
            },
            seen_modules: HashSet::new(),
            seen_instances: HashSet::new(),
            foreign_depth: 0,
            cur_scope: 0,
            module_prefix: None,
        }
    }

    /// 収集を終えてレジストリを凍結する。
    pub(in crate::type_check) fn build(self) -> TypeRegistry {
        self.reg
    }

    /// `NamedInstance(name)` がプロトコル名であれば `Protocol(name)` に変換する。
    /// 収集パス中は `known_protocols` が育っている途中なので、ここで参照するのは
    /// 「その時点までに登録済みのプロトコル」であることに注意（移設前の挙動と同一）。
    fn resolve_protocol_type(&self, ty: InferredType) -> InferredType {
        if let InferredType::NamedInstance(ref name) = ty {
            if self.reg.known_protocols.contains_key(name.as_str()) {
                return InferredType::Protocol(name.clone());
            }
        }
        ty
    }

    /// 文のスライスを先行スキャンして関数・クラス・trait のシグネチャ情報を収集する。
    /// 組み込みの型の宣言（`builtins.ars` の `enum` / `class`・`type_check::builtins::type_decls`）を集める。
    ///
    /// ⚠ `class` は **Arrow のクラスではない**（`FileObject` の実行時の値は `Value::FileObject`）ので、
    ///   外部言語のスタブと同じく `foreign_depth` を上げて集める（`arrow_class_names` に載せない）。
    ///   載せると VM が `Value::Instance` と見なして型特化した命令を出し、実行時に落ちる。
    /// ⚠ 利用者のプログラムより**先に**集める（同じ名前の利用者の宣言が後から上書きする）。
    pub(in crate::type_check) fn collect_builtin_types(&mut self, decls: &[Stmt]) {
        self.foreign_depth += 1;
        self.collect(decls);
        self.foreign_depth -= 1;
    }

    pub(in crate::type_check) fn collect(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            // ⚠⚠ 単相化で作ったクラス（`Box[int]`）は**普通のクラスとして**登録し、
            //   テンプレートの具体化の型（`GenericInstance`）からそれを引けるようにする
            //   （タスク 2-8 段階 2）。型は読み直した表示形をキーにする（書き方の揺れを吸収）。
            if let Stmt::ClassDef { name, .. } = stmt {
                if name.contains('[') {
                    if let Some(t) = crate::type_check::InferredType::from_ann(name) {
                        self.reg.instance_classes.insert(t.to_string(), name.clone());
                    }
                }
            }
            if let Stmt::ClassDef { name, .. } | Stmt::FnDef { name, .. } | Stmt::GenDef { name, .. } =
                stmt
            {
                if name.contains('[') {
                    self.reg.instance_names.insert(name.clone());
                }
            }
            match stmt {
                Stmt::FnDef {
                    name,
                    params,
                    return_type,
                    body,
                    template_params,
                    ..
                } => {
                    let ret = return_type
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .map(|t| self.resolve_protocol_type(t));
                    let sig = self.sig_of(params, ret);
                    self.reg.fn_sigs.entry(name.clone()).or_default().push(sig);
                    // 実体化呼び出しの引数検査に要る（型変数名と並び）。
                    if !template_params.is_empty() {
                        self.reg.template_params.insert(
                            name.clone(),
                            template_params.iter().map(|p| p.name.clone()).collect(),
                        );
                    }
                    self.collect(body);
                }
                // ⚠⚠ **`gen` も関数と同じく登録する**（フェーズ10 10-7）。以前は登録が無く、
                //    `gen` の名前は `Unresolved` だったので、呼び出しの結果の型が分からず
                //    `for s in each(xs):` の `s` に型が付かなかった。`let z: str = s` のような
                //    誤りが静的にも実行時にも止まらなかった（実測）。引数も検査されていなかった。
                //    ⚠ 結果の型は**産出型ではなくジェネレータ**（`generator[T]` = `IteratorOf(T)`）。
                Stmt::GenDef {
                    name, params, yield_type, body, template_params, ..
                } => {
                    let ret = yield_type
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .map(|t| InferredType::IteratorOf(Box::new(self.resolve_protocol_type(t))));
                    let sig = self.sig_of(params, ret);
                    self.reg.fn_sigs.entry(name.clone()).or_default().push(sig);
                    if !template_params.is_empty() {
                        self.reg.template_params.insert(
                            name.clone(),
                            template_params.iter().map(|p| p.name.clone()).collect(),
                        );
                    }
                    self.collect(body);
                }
                Stmt::ClassDef {
                    name, bases, base_args, body, template_params, decorators, ..
                } => {
                    // ⚠ 未展開の `!装飾子` が残っていたら、このクラスのメンバーは未確定。
                    //   ⇒ メンバー存在検査を黙らせる（タスク 2-4）。
                    // ⚠ 外部言語（Python）のクラスにデコレータが付いていても同じ（フェーズ10 10-16）。
                    //   デコレータは実行時にクラスを差し替えられる（`class Wrapped(cls)` を返してメンバーを足す）。
                    if body.iter().any(|s| matches!(s, Stmt::MetaDecorated { .. }))
                        || (self.foreign_depth > 0 && !decorators.is_empty())
                    {
                        self.reg
                            .classes_with_unexpanded_decorators
                            .insert(name.clone());
                    }
                    self.reg.known_class_names.insert(name.clone());
                    // 外部言語スタブ由来でなければ「Arrow のクラス」（#27-a）。
                    if self.foreign_depth == 0 {
                        self.reg.arrow_class_names.insert(name.clone());
                        // ⚠ モジュールのクラスは素の名前も載せる（10-8）。モジュールの本体を検査して付けた
                        //   注釈は素の名前（`Tag`）の型を持つことがあり、VM はこの集合で「Arrow のインスタンスか」を
                        //   見る。以前（素の名前で登録していた頃）と同じ判断になる。
                        if let Some(bare) = self
                            .module_prefix
                            .as_deref()
                            .and_then(|p| name.strip_prefix(p))
                            .and_then(|r| r.strip_prefix('.'))
                        {
                            self.reg.arrow_class_names.insert(bare.to_string());
                        }
                    }
                    self.reg.class_bases.insert(name.clone(), bases.clone());
                    // ⚠ `bases` と `base_args` は**同じ並び**（`ast.rs` の不変条件）。
                    //   zip なので長さがずれても壊れず、足りない分が落ちるだけ。
                    self.reg.class_base_args.insert(
                        name.clone(),
                        bases
                            .iter()
                            .cloned()
                            .zip(base_args.iter().cloned())
                            .collect(),
                    );
                    if !template_params.is_empty() {
                        self.reg.template_params.insert(
                            name.clone(),
                            template_params.iter().map(|p| p.name.clone()).collect(),
                        );
                    }
                    self.collect_class_methods(name, body);
                    self.collect_class_members(name, body);
                    // Only recurse into method bodies for nested closures;
                    // class methods themselves must NOT be added to fn_sigs.
                    for s in body.iter() {
                        match s {
                            Stmt::FnDef { body: method_body, .. } => self.collect(method_body),
                            // `gen` メソッドの中の入れ子の関数・`gen` も同じ（10-7）。
                            Stmt::GenDef { body: method_body, .. } => self.collect(method_body),
                            _ => {}
                        }
                    }
                }
                Stmt::EnumDef { src: _, name, variants } => {
                    self.reg.known_class_names.insert(name.clone());
                    // ⚠⚠ **メンバーと `.value` の型を登録する**（タスク 2.3）。
                    //    以前は名前を `known_class_names` に入れるだけだったので
                    //    `Color.Red` も `Color.Red.value` も `Unresolved` になり、
                    //    `Unresolved` は `type_matches_exact` の万能受容体なので
                    //      let s: str = Color.Red.value   # 1 を表示していた
                    //      let s: int = Color.Red         # enum オブジェクトを表示していた
                    //      let v: A = B.Y                 # 別 enum のメンバーが入っていた
                    //    が黙って通っていた。`.value` は言語規則として `int`
                    //    （実行時 `build_enum_classes` が「must be int」で強制している）。
                    // `Color` 型の値（インスタンス）のフィールドは `value` だけ。
                    let mut enum_fields = HashMap::new();
                    enum_fields.insert("value".to_string(), (FieldKind::Const, InferredType::Int));
                    self.reg.class_field_details.insert(name.clone(), enum_fields);
                    // ⚠⚠ **メンバーは値なので `value` は書き換えられない**（`field_is_mutable` が引く表）。
                    //    以前はこの表に無く、`Color.BLUE.value = 5` が型検査を通って**共有のメンバー
                    //    そのものを書き換えていた**（実行時もフィールドが可変だった・`build_enum_classes`）。
                    self.reg.class_fields.insert(name.clone(), HashMap::from([("value".to_string(), false)]));
                    // バリアント名 → そのバリアントの型。**型は enum 型そのもの**で、`Color.Red` は
                    // `NamedInstance("Color")` になる（タスク 2-2・`implementation_plans/enum_member_type_plan.md`）。
                    // ⚠⚠ 以前は別の型 `enum_item_Color` で、`Color` 型の値が存在しなかった
                    //    （`c == Color.Red`（`c: Color`）が「決して真にならない」と弾かれていた）。
                    //    実行時もメンバーのクラス名は enum 名（`build_enum_classes`）。
                    // ⚠⚠ **フィールドの表とは別の表に入れる**（タスク 2-1）。メンバーは型の値
                    //    （`Color.Red`）からだけ引け、インスタンス（`m.Red`）からは引けない。
                    let members = variants
                        .iter()
                        .map(|(vname, _)| (vname.clone(), InferredType::NamedInstance(name.clone())))
                        .collect();
                    self.reg.enum_members.insert(name.clone(), members);
                }
                Stmt::TraitDef { name, body, template_params, .. } => {
                    // trait 自身の型変数も登録する。適合検査で `-> T` のような
                    // 「実体化前の要求」を具体型と突き合わせないために要る。
                    if !template_params.is_empty() {
                        self.reg.template_params.insert(
                            name.clone(),
                            template_params.iter().map(|p| p.name.clone()).collect(),
                        );
                    }
                    self.collect_trait(name, body);
                }
                Stmt::ProtocolDef { name, body } => {
                    self.collect_protocol(name, body);
                }
                Stmt::Match { arms, .. } => {
                    for arm in arms {
                        self.collect(&arm.body);
                    }
                }
                Stmt::If {
                    branches,
                    else_body,
                    span: _,
                } => {
                    for (_, body) in branches {
                        self.collect(body);
                    }
                    if let Some(body) = else_body {
                        self.collect(body);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Block(body) => {
                    self.collect(body);
                }
                // import 先モジュールの定義も収集する（#16 段階 F）。
                //
                // これが無いと import したクラスが `known_class_names` に載らず、
                // **メインプログラム側でも** `v.x`（`v: Vec2` が import 由来）の型が引けない。
                // 実測では import クラスを使う算術が 1 件も型特化されていなかった。
                // 同一モジュールの二重収集は `fn_sigs` の偽オーバーロードを生むので弾く。
                Stmt::Import { lang, module, body, .. }
                | Stmt::FromImport { lang, module, body, .. } => {
                    let modpath = module.join(".");
                    let arrow = is_arrow_source_lang(lang);
                    // 今の文脈に別名・取り込んだ名前を控える（`TypeRegistry::name_scopes` の doc・10-8）。
                    if arrow {
                        let scope = &mut self.reg.name_scopes[self.cur_scope];
                        match stmt {
                            // ⚠ 束縛名と、そこへ入るモジュールは AST の `bind`（CPython 準拠）。
                            //   `import a.b` は `a` → パッケージ `a`（`a.b.Tag` は `a` を引き直して `a.b.Tag`）。
                            Stmt::Import { bind, .. } => {
                                if let Some(b) = bind {
                                    scope.modules.insert(b.name.clone(), b.module.join("."));
                                }
                            }
                            Stmt::FromImport { names, .. } => {
                                for (orig, alias) in names {
                                    scope.names.insert(
                                        alias.clone().unwrap_or_else(|| orig.clone()),
                                        format!("{modpath}.{orig}"),
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                    // 具体化（展開器が置いた `Box[int]`）は、集めたものを控える（`seen_instances` の doc）。
                    let instances: Vec<Stmt> = body
                        .iter()
                        .filter(|st| {
                            instance_decl_name(st).is_some_and(|n| {
                                self.seen_instances.insert((lang.clone(), module.clone(), n.to_string()))
                            })
                        })
                        .cloned()
                        .collect();
                    if self.seen_modules.insert((lang.clone(), module.clone())) {
                        if arrow {
                            // モジュールの宣言は `tags.X` の名前で登録する（`qualify_module_body`・10-8）。
                            let (qbody, scope) = qualify_module_body(&modpath, body);
                            let idx = self.reg.name_scopes.len();
                            self.reg.name_scopes.push(scope);
                            self.reg.module_scope_index.insert(modpath.clone(), idx);
                            self.collect_in_module(&modpath, idx, &qbody);
                        } else {
                            // 外部言語のスタブ本体に入る間は `arrow_class_names` へ載せない（#27-a）。
                            self.foreign_depth += 1;
                            self.collect(body);
                            self.foreign_depth -= 1;
                        }
                    } else if !instances.is_empty() {
                        // ⚠ 2 回目以降の `import`: この本体にだけある具体化を集める（タスク 2-16）。
                        match self.reg.module_scope_index.get(&modpath).copied() {
                            Some(idx) if arrow => {
                                let map = self.reg.name_scopes[idx].names.clone();
                                let q = |s: &str| crate::template_subst::subst_type(s, &map);
                                let qinst: Vec<Stmt> = crate::template_subst::subst_stmts(&instances, &map)
                                    .into_iter()
                                    .map(|mut st| {
                                        if let Stmt::ClassDef { name, .. }
                                        | Stmt::FnDef { name, .. }
                                        | Stmt::GenDef { name, .. } = &mut st
                                        {
                                            *name = q(name);
                                        }
                                        st
                                    })
                                    .collect();
                                self.collect_in_module(&modpath, idx, &qinst);
                            }
                            _ => self.collect(&instances),
                        }
                    }
                }
                _ => {}
            }
        }
        for stmt in stmts {
            if let Stmt::NewTypeDef { name, original } = stmt {
                self.reg.known_class_names.insert(name.clone());
                self.reg
                    .new_type_originals
                    .insert(name.clone(), original.clone());
                if let Some(orig_sigs) = self.reg.class_method_sigs.get(original).cloned() {
                    self.reg.class_method_sigs.insert(name.clone(), orig_sigs);
                }
                // ⚠⚠ **`value` フィールドを登録する**（タスク 7.5）。実行時の
                //    `make_primitive_wrapper_class`（`interpreter/built_in_types.rs`）は
                //    `__init__` で `self.value = value` を代入する**宣言済みフィールド**を
                //    合成しているのに、静的側は `class_method_sigs` しか引き継いでいなかった。
                //    ⇒ `Celsius(1.0).value` が `Unresolved` になり、型検査が素通りしていた。
                // ⚠ 基底の連鎖を根まで辿る（`Kg: Meters: float` なら `float`）。
                //   根がクラスのときは包まず**継承**なので登録しない（タスク 5.6 と同じ判断）。
                let mut root = original.clone();
                let mut seen = std::collections::HashSet::new();
                while let Some(next) = self.reg.new_type_originals.get(&root).cloned() {
                    if !seen.insert(next.clone()) {
                        break;
                    }
                    root = next;
                }
                if let Some(ty) = InferredType::from_ann(&root) {
                    if !matches!(
                        ty,
                        InferredType::NamedInstance(_)
                            | InferredType::GenericInstance { .. }
                            | InferredType::Unresolved
                    ) {
                        let mut f = HashMap::new();
                        f.insert("value".to_string(), (FieldKind::Let, ty));
                        self.reg.class_field_details.insert(name.clone(), f);
                    }
                }
            }
        }
    }

    /// モジュールの本体（修飾名に書き換えた写し）を、そのモジュールの文脈で集める（10-8）。
    fn collect_in_module(&mut self, modpath: &str, idx: usize, body: &[Stmt]) {
        let prev_scope = std::mem::replace(&mut self.cur_scope, idx);
        let prev_prefix = self.module_prefix.replace(modpath.to_string());
        self.collect(body);
        self.cur_scope = prev_scope;
        self.module_prefix = prev_prefix;
    }

    /// ★ Python の `**kwargs` 番兵パラメータかどうか。
    ///
    /// `import[py]` の変換器は `**kwargs` を Arrow の識別子にできない名前のパラメータとして出す。
    /// これは「任意のキーワード引数を受ける」印であって**位置引数のスロットではない**ので、
    /// シグネチャの引数列からも必要数からも外す。
    fn is_py_kwargs_param(p: &Param) -> bool {
        p.name == crate::ast::PY_KWARGS_PARAM
    }

    /// ★ 引数の個数が**開いている**シグネチャか（`*args` か `**kwargs` を持つ）。
    ///
    /// `FnSig` は個数固定の引数列しか表せないので、開いている場合は
    /// `variadic_type` を立てて**上限なし**として扱わせる（`call_check.rs` の個数照合）。
    /// これをしないと `f(1, 2, 3)` が `takes 1 argument(s) but 3 were given` という
    /// **嘘のエラー**になる。
    fn has_open_arity(params: &[Param]) -> bool {
        params.iter().any(|p| p.variadic || Self::is_py_kwargs_param(p))
    }

    /// 最上位（と入れ子）の `fn` / `gen` の仮引数からシグネチャを作る。`ret` は**呼び出しの結果の型**
    /// （`gen` なら `generator[T]`）。
    fn sig_of(&self, params: &[Param], ret: Option<InferredType>) -> FnSig {
        let variadic_param = params.iter().find(|p| p.variadic);
        let open_arity = Self::has_open_arity(params);
        FnSig {
            // ⚠ `params` と**同じ絞り込み**で並べること（B11）。
            param_mutable: params
                .iter()
                .filter(|p| Self::is_normal_param(p))
                .map(|p| p.mutable)
                .collect(),
            params: params
                .iter()
                .filter(|p| Self::is_normal_param(p))
                .map(|p| {
                    let ty = p
                        .type_ann
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .map(|t| self.resolve_protocol_type(t));
                    (p.name.clone(), ty)
                })
                .collect(),
            required_count: params
                .iter()
                .filter(|p| Self::is_normal_param(p) && p.default.is_none())
                .count(),
            return_type: ret,
            // ⚠ `**kwargs` しか無い場合も**上限なし**にしたいので、
            //   型が取れなくても `Any` を入れて `Some` にする。
            variadic_type: variadic_param
                .and_then(|p| p.type_ann.as_deref().and_then(InferredType::from_ann))
                .or(if open_arity { Some(InferredType::Any) } else { None }),
        }
    }

    /// クラス本体のメソッドシグネチャを収集して `class_method_sigs` に登録する。
    fn collect_class_methods(&mut self, name: &str, body: &[Stmt]) {
        let mut cls_methods: HashMap<String, Vec<FnSig>> = HashMap::new();
        for s in body.iter() {
            if let Stmt::FnDef {
                name: mname,
                template_params,
                params,
                return_type,
                ..
            } = s
            {
                let storage_name = if mname == "__cast__" && !template_params.is_empty() {
                    format!("__cast__[{}]", template_params[0].name)
                } else {
                    mname.clone()
                };
                let variadic_param = params.iter().find(|p| p.variadic);
                let open_arity = Self::has_open_arity(params);
                let sig = FnSig {
                    // ⚠ `params` と**同じ絞り込み**で並べること（B11）。
                    param_mutable: params
                        .iter()
                        .filter(|p| Self::is_normal_param(p))
                        .map(|p| p.mutable)
                        .collect(),
                    params: params
                        .iter()
                        .filter(|p| Self::is_normal_param(p))
                        .map(|p| {
                            (
                                p.name.clone(),
                                p.type_ann.as_deref().and_then(InferredType::from_ann),
                            )
                        })
                        .collect(),
                    required_count: params
                        .iter()
                        .filter(|p| {
                            Self::is_normal_param(p) && p.default.is_none()
                        })
                        .count(),
                    return_type: return_type
                        .as_deref()
                        .and_then(InferredType::from_ann),
                    variadic_type: variadic_param
                        .and_then(|p| p.type_ann.as_deref().and_then(InferredType::from_ann))
                        .or(if open_arity { Some(InferredType::Any) } else { None }),
                };
                cls_methods.entry(storage_name).or_default().push(sig);
            }
            // ⚠⚠ **`gen` メソッドがここに登録されていなかった**（既存のドリフト）。
            //    `Stmt::GenDef` はクラス本体に置けるのに、この収集が `Stmt::FnDef` しか
            //    見ていなかったため、`b.each()` が `'Bag' has no member 'each'` になる。
            //    ⚠ **純 Arrow の `gen` メソッドでも再現**する（`import[py]` 固有ではない）。
            //    ⚠ 呼び出しの戻り値は**産出型ではなくジェネレータ**。産出型が書いてあれば
            //      `generator[T]`（`IteratorOf`・フェーズ10 10-7）、無ければ素の `generator`。
            //      以前は常に素の `generator` で、`for x in b.each():` の `x` に型が付かなかった。
            if let Stmt::GenDef {
                name: mname,
                params,
                yield_type,
                ..
            } = s
            {
                let variadic_param = params.iter().find(|p| p.variadic);
                let open_arity = Self::has_open_arity(params);
                let sig = FnSig {
                    // ⚠ `params` と**同じ絞り込み**で並べること（B11）。
                    param_mutable: params
                        .iter()
                        .filter(|p| Self::is_normal_param(p))
                        .map(|p| p.mutable)
                        .collect(),
                    params: params
                        .iter()
                        .filter(|p| Self::is_normal_param(p))
                        .map(|p| {
                            (
                                p.name.clone(),
                                p.type_ann.as_deref().and_then(InferredType::from_ann),
                            )
                        })
                        .collect(),
                    required_count: params
                        .iter()
                        .filter(|p| Self::is_normal_param(p) && p.default.is_none())
                        .count(),
                    return_type: Some(
                        yield_type
                            .as_deref()
                            .and_then(InferredType::from_ann)
                            .map(|t| InferredType::IteratorOf(Box::new(t)))
                            .unwrap_or_else(|| InferredType::NamedInstance("generator".to_string())),
                    ),
                    variadic_type: variadic_param
                        .and_then(|p| p.type_ann.as_deref().and_then(InferredType::from_ann))
                        .or(if open_arity { Some(InferredType::Any) } else { None }),
                };
                cls_methods.entry(mname.clone()).or_default().push(sig);
            }
        }
        self.reg.class_method_sigs.insert(name.to_string(), cls_methods);
    }

    /// クラス本体のフィールド・アクセス指定・スタティックメソッドを収集する。
    fn collect_class_members(&mut self, name: &str, body: &[Stmt]) {
        let mut fields: HashMap<String, bool> = HashMap::new();
        let mut field_details: HashMap<String, (FieldKind, InferredType)> = HashMap::new();
        let mut member_access: HashMap<String, Accessibility> = HashMap::new();
        let mut static_methods: HashSet<String> = HashSet::new();
        for s in body.iter() {
            match s {
                Stmt::Field {
                    name: fname,
                    kind,
                    type_ann,
                    access,
                    ..
                } => {
                    // ⚠⚠ **`StaticMut` も可変**（タスク 1.2）。`static mut name: T` の仕様は
                    //    「全インスタンスで共有される**可変**セル。インスタンス経由・
                    //    クラス名経由どちらでもアクセス・**代入可能**」（`ast.rs` の
                    //    `FieldKind::StaticMut` の doc）。
                    //    ここで `Mut` だけを可変と数えていたため、インスタンス経由の代入
                    //    `c.n = 5` が `cannot assign to immutable field 'n'` で塞がれていた。
                    //    ⚠ **実行時は実装済み**（`attrs.rs` が `inst_class.static_vars` を
                    //    更新する）＝静的検査だけが塞いでいる層の食い違いだった。
                    //    露見しなかったのは、例題が `Registry.entry_count`（クラス名経由）
                    //    でしか代入しておらず、インスタンス経由を書いた例題が 1 本も
                    //    無かったため。
                    let mutable = matches!(kind, FieldKind::Mut | FieldKind::StaticMut);
                    fields.insert(fname.clone(), mutable);
                    let fty = InferredType::from_ann(type_ann)
                        .unwrap_or(InferredType::Unresolved);
                    field_details.insert(fname.clone(), (kind.clone(), fty));
                    if *access != Accessibility::Public {
                        member_access.insert(fname.clone(), access.clone());
                    }
                }
                Stmt::FnDef {
                    name: mname,
                    is_static,
                    access,
                    ..
                } => {
                    if *access != Accessibility::Public {
                        member_access.insert(mname.clone(), access.clone());
                    }
                    if *is_static {
                        static_methods.insert(mname.clone());
                    }
                }
                // ⚠ `gen` メソッドのアクセス指定。`Stmt::GenDef` に `is_static` は
                //   無い（Arrow の `gen` に `static` 形が無い）ので access だけ拾う。
                Stmt::GenDef {
                    name: mname,
                    access,
                    ..
                } => {
                    if *access != Accessibility::Public {
                        member_access.insert(mname.clone(), access.clone());
                    }
                }
                _ => {}
            }
        }
        self.reg.class_fields.insert(name.to_string(), fields);
        self.reg.class_field_details.insert(name.to_string(), field_details);
        self.reg.class_member_access.insert(name.to_string(), member_access);
        if !static_methods.is_empty() {
            self.reg
                .class_static_methods
                .insert(name.to_string(), static_methods);
        }
    }

    /// trait 本体のメソッドシグネチャ・フィールドを収集する（Intersection 適合チェック用）。
    fn collect_trait(&mut self, name: &str, body: &[Stmt]) {
        let mut tmethods: HashMap<String, Vec<FnSig>> = HashMap::new();
        let mut tfields: HashMap<String, (FieldKind, InferredType)> = HashMap::new();
        for s in body.iter() {
            match s {
                Stmt::FnDef { name: mname, params, return_type, body: method_body, .. } => {
                    let variadic_param = params.iter().find(|p| p.variadic);
                    let sig = FnSig {
                        // ⚠ `params` と**同じ絞り込み**で並べること（B11）。
                        param_mutable: params.iter().filter(|p| !p.variadic).map(|p| p.mutable).collect(),
                        params: params
                            .iter()
                            .filter(|p| !p.variadic)
                            .map(|p| (p.name.clone(), p.type_ann.as_deref().and_then(InferredType::from_ann)))
                            .collect(),
                        required_count: params.iter().filter(|p| !p.variadic && p.default.is_none()).count(),
                        return_type: return_type.as_deref().and_then(InferredType::from_ann),
                        variadic_type: variadic_param.and_then(|p| p.type_ann.as_deref().and_then(InferredType::from_ann)),
                    };
                    tmethods.entry(mname.clone()).or_default().push(sig);
                    self.collect(method_body);
                }
                Stmt::Field { name: fname, kind, type_ann, .. } => {
                    let ty = InferredType::from_ann(type_ann).unwrap_or(InferredType::Unresolved);
                    tfields.insert(fname.clone(), (kind.clone(), ty));
                }
                _ => {}
            }
        }
        self.reg.trait_method_sigs.insert(name.to_string(), tmethods);
        self.reg.trait_field_details.insert(name.to_string(), tfields);
    }

    /// protocol 本体のフィールド・メソッド要件を収集して `known_protocols` に登録する。
    fn collect_protocol(&mut self, name: &str, body: &[Stmt]) {
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        for s in body.iter() {
            match s {
                Stmt::Field { name: fname, kind, type_ann, .. } => {
                    let ty = InferredType::from_ann(type_ann)
                        .unwrap_or(InferredType::Unresolved);
                    fields.push(ProtocolField {
                        name: fname.clone(),
                        kind: kind.clone(),
                        ty,
                    });
                }
                Stmt::FnDef { name: mname, params, return_type, .. } => {
                    let ret = return_type
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .unwrap_or(InferredType::Unresolved);
                    let method_params: Vec<(String, bool, InferredType)> = params
                        .iter()
                        .filter(|p| p.name != "self")
                        .map(|p| {
                            let ty = p
                                .type_ann
                                .as_deref()
                                .and_then(InferredType::from_ann)
                                .unwrap_or(InferredType::Unresolved);
                            (p.name.clone(), p.mutable, ty)
                        })
                        .collect();
                    methods.push(ProtocolMethod {
                        name: mname.clone(),
                        params: method_params,
                        return_type: ret,
                    });
                }
                _ => {}
            }
        }
        self.reg
            .known_protocols
            .insert(name.to_string(), ProtocolInfo { fields, methods });
    }
}
