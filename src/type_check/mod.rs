mod types;
mod errors;
mod diagnostics;
mod state;
mod registry;
mod members;
mod scope;
mod stmt;
mod infer;
mod type_utils;
mod call_check;
mod binop;
mod decorator;
pub mod annotations;

// 型チェッカの公開 API 面。`FnTypeParam` / `TypeErrorKind` / `TypeWarningKind` は
// bin からは未使用だが frontend_tests が使うため、narrowing しないこと。
#[allow(unused_imports)]
pub use types::{FnTypeParam, InferredType, MetaKind};
#[allow(unused_imports)]
pub use annotations::{
    ArgAnnotation, AstAnnotations, BinOperandKind, CallInfo, Directive, TypeId,
};
#[allow(unused_imports)]
pub use errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind};
use types::VarInfo;
use diagnostics::Diagnostics;
use state::CheckState;
use registry::TypeRegistry;

use std::collections::HashMap;
use crate::ast::Stmt;

// ---------------------------------------------------------------------------
// TypeChecker
// ---------------------------------------------------------------------------

/// 静的型検査器。AST を走査してすべての型エラーを収集し報告する。
pub struct TypeChecker {
    /// 検査の進行に伴って変化するカーソル状態（スコープスタック・現在の関数/クラス・
    /// `block_return` 禁止深さ）。操作は scope.rs の委譲メソッド経由で行う。
    state: CheckState,
    /// クラス・trait・protocol・関数の宣言索引。収集パス（registry/builder.rs）で
    /// 組み立て済みで、検査中は**読み取り専用**。
    registry: TypeRegistry,
    /// 収集された静的型エラー・警告。`check()` が返す前にここへ蓄積される。
    /// 追加は `report_error` / `report_warning`（scope.rs）経由で行う。
    diags: Diagnostics,
    /// AST 型解決層の注釈（タスク #16・段階(a)）。検査走査中に `infer`/`check` が node-id 索引で
    /// 型・検査指示を焼く。`check_program` が取り出す（`check` は注釈を捨てる）。
    annotations: annotations::AstAnnotations,
    /// **レジストリが不完全か**（タスク 8.5）。
    ///
    /// ⚠⚠ VS Code 拡張の wasm フロントエンドは **import 先を読み込まない**（fs に触れない）
    /// ので、import 文の body が空になり、その先で定義された型が
    /// レジストリに載らない。この状態で「注釈の型名が実在するか」を検査すると、
    /// **import した型を全部「存在しない」と言ってしまう**
    /// （実測: `compare_wasm_frontend` が INVENTED 6 件を検出した）。
    /// ⇒ ここが `true` のときは [`Self::check_ann_names_exist`] を止める。
    ///
    /// ⚠ **`cfg!(feature = "editor")` で直接分岐しないこと。** 「編集中だから」ではなく
    /// 「レジストリが不完全だから」止める、という理由で書いておかないと、
    /// 将来 import を解決する編集環境が出たときに誤って止め続ける。
    registry_incomplete: bool,
    /// 注釈採取済みの import モジュール `(lang, モジュールパス)`（#16 段階 F）。
    /// 同じモジュールが複数箇所から import される・入れ子 import で再訪する場合の重複走査を防ぐ。
    annotated_modules: std::collections::HashSet<(String, Vec<String>)>,
}

impl TypeChecker {
    /// 組み込み型・例外クラスを登録し、`stmts` の収集パスを済ませた [`TypeChecker`] を生成する。
    ///
    /// 収集パス（`TypeRegistryBuilder::collect`）はここで完了し、以降 `registry` は不変。
    fn new(stmts: &[Stmt]) -> Self {
        let mut global: HashMap<String, VarInfo> = HashMap::new();
        let builtins: &[(&str, InferredType)] = &[
            ("int", InferredType::Int),
            ("float", InferredType::Float),
            ("str", InferredType::Str),
            ("bool", InferredType::Bool),
            ("Any", InferredType::Any),
            (
                "function",
                InferredType::Function {
                    params: None,
                    return_type: Box::new(InferredType::Any),
                },
            ),
        ];
        for (name, inner) in builtins {
            global.insert(
                name.to_string(),
                VarInfo {
                    ty: InferredType::TypeValOf(Box::new(inner.clone())),
                    mutable: false,
                },
            );
        }
        // 組み込み**関数**の戻り値型（#16 段階 D）。
        //
        // これが無いと `range` は未知の識別子扱いで `range(n)` が `Unresolved` になり、
        // **`for i in range(n)` のループ変数が型無し**になる。最頻出のループ形なのに
        // 本体の演算が一切型特化されず、そこから `Unresolved` が式全体へ伝播していた。
        // シグネチャは `params: None`（引数の検査はしない）で戻り値型だけ与える。
        //
        // ⚠⚠ **`len` を外していた理由は誤りだった**（タスク 7.4 で実測して訂正）。
        //    以前ここには「`let len = ...` は今まで通っていた書き方なので新たなエラーを
        //    増やす」「例題が `len` を変数名に使っている（実測 3 箇所）」と書いてあったが:
        //      - `let len = 5` は**すでに実行時 `NameError: variable 'len' is already declared`**
        //      - 「例題 3 箇所」の実体は `print("len:", len(nums))` のような**文字列 `"len:"`**
        //        だった（テキスト grep の誤読）。変数名に使っている例題は **0 件**
        //    ⇒ `len` を登録しても新しく壊れるものは無い。
        //
        // ⚠ ここへ登録した名前はグローバルスコープを占める（`int`/`str` と同じ扱い）。
        //   占有してよいのは「実行時も既に占有している」名前だけ。
        //
        // ## 引数の型（実測して表を書いた）
        //
        // | 関数 | 引数 | 戻り値 |
        // |---|---|---|
        // | `range` | `int` を 1〜3 個 | `list[int]` |
        // | `len` | 1 個（`list`/`str`/`dict`/`set`/`tuple`/`fixed_list`/`__len__` を持つクラス） | `int` |
        //
        // ⚠ `len` の引数は「大きさを持つ型」で、`InferredType` に対応する型が無い。
        //   ⇒ `Any` にして個数だけ検査し、**明らかに大きさを持たない型**は
        //     `check_len_argument`（`call_check.rs`）で弾く。
        let int_p = |name: &str, has_default: bool| types::FnTypeParam {
            name: name.to_string(),
            mutable: false,
            ty: InferredType::Int,
            has_default,
        };
        let builtin_fns: Vec<(&str, Option<Vec<types::FnTypeParam>>, InferredType)> = vec![
            (
                "range",
                Some(vec![
                    int_p("start", false),
                    int_p("stop", true),
                    int_p("step", true),
                ]),
                InferredType::ListOf(Box::new(InferredType::Int)),
            ),
            (
                "len",
                Some(vec![types::FnTypeParam {
                    name: "obj".to_string(),
                    mutable: false,
                    ty: InferredType::Any,
                    has_default: false,
                }]),
                InferredType::Int,
            ),
        ];
        for (name, params, ret) in builtin_fns {
            global.insert(
                name.to_string(),
                VarInfo {
                    ty: InferredType::Function {
                        params,
                        return_type: Box::new(ret),
                    },
                    mutable: false,
                },
            );
        }
        for name in ["begin", "last"] {
            global.insert(
                name.to_string(),
                VarInfo {
                    ty: InferredType::NamedInstance("Index".to_string()),
                    mutable: false,
                },
            );
        }
        global.insert(
            "Error".to_string(),
            VarInfo {
                ty: InferredType::TypeValOf(Box::new(InferredType::NamedInstance(
                    "Error".to_string(),
                ))),
                mutable: false,
            },
        );
        // 例外クラスの登録はレジストリ側（with_builtins）と対になっている。
        // ここではグローバルスコープの束縛のみを作る。
        for class_name in registry::builder::EXCEPTION_CLASS_NAMES {
            global.insert(
                class_name.to_string(),
                VarInfo {
                    ty: InferredType::TypeValOf(Box::new(InferredType::NamedInstance(
                        class_name.to_string(),
                    ))),
                    mutable: false,
                },
            );
        }

        let mut builder = registry::builder::TypeRegistryBuilder::with_builtins();
        builder.collect(stmts);

        Self {
            state: CheckState::new(global),
            registry: builder.build(),
            diags: Diagnostics::default(),
            annotations: annotations::AstAnnotations::default(),
            registry_incomplete: Self::has_unloaded_import(stmts),
            annotated_modules: std::collections::HashSet::new(),
        }
    }

    /// **本体を読み込めていない import があるか**（タスク 8.5）。
    ///
    /// VS Code 拡張の wasm フロントエンドは fs に触れないので import 先を読まず、
    /// `Stmt::Import` / `Stmt::FromImport` の `body` が**空**になる。そのとき
    /// import 先で定義された型はレジストリに載らないので、
    /// 「レジストリに無い＝存在しない」と断定できない。
    ///
    /// ⚠ **空 body は「読めなかった」の印**であって「中身が無い」ではない。
    /// 実体のあるモジュールが本当に空になることは（定義文が 1 つも無いファイルを
    /// import しない限り）無く、その場合に検査を止めても取りこぼしが増えるだけで
    /// 誤検出は出ない ⇒ 保守的側へ倒す。
    ///
    /// ⚠⚠ **`editor` では body が非空でもレジストリは完成しない。** スタブ
    /// （ホスト由来・同梱 py）は実モジュールの**部分集合**でしかなく、しかも
    /// 古くなりうる（DLL / ヘッダを更新してもスタブは自動では追随しない）。
    /// ここで「読めた」と見なすと [`Self::check_ann_names_exist`] が動き出し、
    /// **CLI が出さないエラーをエディタだけが出す**。それは
    /// `compare_wasm_frontend.ps1` の不変条件
    /// 「wasm は少なく報告してよいが、多く報告してはならない」を破る。
    /// ⇒ editor では **import が 1 つでもあれば不完全**と答える。
    ///   import が無いファイルは従来どおり `false` なので、検査範囲は狭まらない。
    fn has_unloaded_import(stmts: &[Stmt]) -> bool {
        stmts.iter().any(|s| match s {
            #[cfg(feature = "editor")]
            Stmt::Import { .. } | Stmt::FromImport { .. } => true,
            #[cfg(not(feature = "editor"))]
            Stmt::Import { body, .. } | Stmt::FromImport { body, .. } => body.is_empty(),
            // ⚠ import は最上位にしか書けないが、`if` の中などへ移ったときに
            //   静かに見落とさないよう、定義の本体だけは覗いておく。
            Stmt::FnDef { body, .. }
            | Stmt::ClassDef { body, .. }
            | Stmt::TraitDef { body, .. } => Self::has_unloaded_import(body),
            _ => false,
        })
    }

    /// 文のスライスを静的型検査して、収集されたすべての [`StaticTypeError`] を返す。
    // バイナリの実行経路は `check_program` を使うため（#88 で入口を 1 本化した）、
    // 現在この最小版はテストからのみ呼ばれる。
    #[allow(dead_code)]
    pub fn check(stmts: &[Stmt]) -> Vec<StaticTypeError> {
        let mut tc = Self::new(stmts);
        tc.check_stmts(stmts);
        tc.diags.into_parts().0
    }


    /// 展開時型推論（設計書 D15 / タスク 4-4）: `prefix` を型検査し終えた時点で
    /// 最上位の名前 `name` に付いている型。
    ///
    /// ⚠⚠ **新しい推論器を書かない。** 型検査器は既に右辺から推論できる（D15 の実測）ので、
    /// 展開時には「ここまでの文」をそのまま検査にかけ、束縛に付いた型を読むだけにする。
    /// 推論規則が 2 本になると、展開時と実行前で型が食い違う。
    ///
    /// ⚠ 逐次展開（§1.7）なので `prefix` は**その束縛より前の文だけ**でよい。不動点は要らない。
    /// ⚠ 推論できなかった（`Unresolved`）ときは `None` を返す。**`unknown` という文字列を
    /// 型として返さない** —— 型名として差し込まれると壊れた型注釈が黙って通る。
    pub fn binding_type_after(prefix: &[Stmt], name: &str) -> Option<InferredType> {
        let mut tc = Self::new(prefix);
        tc.check_stmts(prefix);
        let ty = tc.lookup(name)?.ty.clone();
        if ty == InferredType::Unresolved {
            return None;
        }
        Some(ty)
    }

    /// エラー・警告・**AST 型解決層の注釈**をまとめて返す（**型検査の唯一の入口**）。
    /// `check` と同じ検査に**警告と注釈生成**を加えたもの（同一走査・追加コストは注釈の充填のみ）。
    ///
    /// ⚠ #88 まで警告を落としただけの**逐語コピー**が併存し、
    /// 入口によってどちらを呼ぶかが分かれていた。⇒ **こちらへ 1 本化**し、
    /// 警告を捨てるかどうかは呼び出し側が決める（配線は
    /// [`resolver::resolve_and_annotate`](crate::interpreter::resolver::resolve_and_annotate)）。
    pub fn check_program(
        stmts: &[Stmt],
    ) -> (
        Vec<StaticTypeError>,
        Vec<StaticTypeWarning>,
        annotations::AstAnnotations,
    ) {
        let mut tc = Self::new(stmts);
        tc.check_stmts(stmts);
        // Arrow ソース由来のクラス名を注釈へ移す（#27-a）。VM コンパイラが
        // 「このレシーバは `Value::Instance` だ」と断定してよいかの唯一の根拠。
        tc.annotations
            .set_arrow_classes(tc.registry.arrow_class_names().clone());
        let annotations = std::mem::take(&mut tc.annotations);
        let instance_names = tc.registry.instance_names().clone();
        let (errors, warnings) = tc.diags.into_parts_tagged();
        let errors = Self::merge_instance_errors(errors, &instance_names);
        (errors, warnings, annotations)
    }

    /// **同じ誤りの重複報告をまとめる**（タスク 2-8 段階 2）。
    ///
    /// テンプレートは本体（型変数のまま）と、単相化した具体化（`Box[int]` / `Box[str]` …）の
    /// 両方が検査される。型変数に関係しない誤り（`fn label(self) -> str: return 1`）は
    /// その**全部で**報告され、1 つの誤りが具体化の数＋1 回出てしまう。
    /// ⇒ 具体化の名前をテンプレートの名前に戻したとき**位置と文面が同じ**になる報告は、
    ///   最初の 1 件（ふつうはテンプレートの本体の分）だけ残す。
    /// ⚠ 具体化ごとに合否が違う誤り（`Box[str]` だけが誤り）は文面が具体化名を含むので残る。
    /// ⚠⚠ まとめる対象は**具体化の本体で見つかった誤りだけ**。テンプレートと関係の無い本物の
    /// 重複（別々の箇所の同じ文面・位置不明）まで消すと、箇所の数が分からなくなる（実測で踏んだ）。
    fn merge_instance_errors(
        errors: Vec<(StaticTypeError, bool)>,
        instance_names: &std::collections::HashSet<String>,
    ) -> Vec<StaticTypeError> {
        if instance_names.is_empty() {
            return errors.into_iter().map(|(e, _)| e).collect();
        }
        // ⚠ 長い名前から置き換える（`Box[Box[int]]` の中の `Box[int]` を先に崩さない）。
        let mut names: Vec<&String> = instance_names.iter().collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        let normalize = |msg: &str| -> String {
            // ⚠ 文面は名前を ANSI で色付けしている（引用符の直後に制御文字が入る）ので外してから比べる。
            let mut m = String::with_capacity(msg.len());
            let mut in_escape = false;
            for c in msg.chars() {
                if in_escape {
                    in_escape = c != 'm';
                } else if c == '\x1b' {
                    in_escape = true;
                } else {
                    m.push(c);
                }
            }
            for n in &names {
                // メソッドの名乗り（`Box[int].half`）はテンプレート側の `half` にそろえる。
                m = m.replace(&format!("{n}."), "");
                m = m.replace(n.as_str(), crate::template_subst::display_name(n));
            }
            m
        };
        let mut seen = std::collections::HashSet::new();
        errors
            .into_iter()
            .filter(|(e, from_instance)| {
                let fresh = seen.insert(normalize(&e.to_string()));
                fresh || !from_instance
            })
            .map(|(e, _)| e)
            .collect()
    }
}

