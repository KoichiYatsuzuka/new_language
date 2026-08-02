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
    /// 型・検査指示を焼く。`check_and_annotate` で取り出す（既存 `check`/`check_with_warnings` は不変）。
    annotations: annotations::AstAnnotations,
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
        }
    }

    /// 文のスライスを静的型検査して、収集されたすべての [`StaticTypeError`] を返す。
    pub fn check(stmts: &[Stmt]) -> Vec<StaticTypeError> {
        let mut tc = Self::new(stmts);
        tc.check_stmts(stmts);
        tc.diags.into_parts().0
    }


    /// 静的型検査に加えて **AST 型解決層の注釈**（タスク #16・段階(a)）を生成して返す。
    /// 検査走査中に `infer`/`check` が node-id 索引で型・検査指示を焼いた結果。
    /// 既存 `check`/`check_with_warnings` は注釈を捨てるため挙動不変（この経路のみ注釈を取り出す）。
    #[allow(dead_code)]
    pub fn check_and_annotate(
        stmts: &[Stmt],
    ) -> (Vec<StaticTypeError>, annotations::AstAnnotations) {
        let mut tc = Self::new(stmts);
        tc.check_stmts(stmts);
        let annotations = std::mem::take(&mut tc.annotations);
        (tc.diags.into_parts().0, annotations)
    }

    /// エラー・警告・**AST 型解決層の注釈**をまとめて返す（ランタイム配線用・main.rs が使う）。
    /// `check_with_warnings` と同じ検査に注釈生成を加えたもの（同一走査・追加コストは注釈の充填のみ）。
    pub fn check_program(
        stmts: &[Stmt],
    ) -> (
        Vec<StaticTypeError>,
        Vec<StaticTypeWarning>,
        annotations::AstAnnotations,
    ) {
        let mut tc = Self::new(stmts);
        tc.check_stmts(stmts);
        let annotations = std::mem::take(&mut tc.annotations);
        let (errors, warnings) = tc.diags.into_parts();
        (errors, warnings, annotations)
    }
}

