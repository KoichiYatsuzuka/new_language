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
pub use types::{FnTypeParam, InferredType};
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
    fn has_unloaded_import(stmts: &[Stmt]) -> bool {
        stmts.iter().any(|s| match s {
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
        let (errors, warnings) = tc.diags.into_parts();
        (errors, warnings, annotations)
    }
}

