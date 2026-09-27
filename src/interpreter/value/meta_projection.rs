// value/meta_projection.rs — `^` の射影の一覧と実装（設計書 §1.5 / 参考B / D12・タスク 4-1）。
//
// ## なぜ一覧を型にするのか（D12）
//
// 射影は**後から変更・追加・削除が入る前提**の集合。文字列の `match` で散らばらせると
// 「足したのに片方だけ古い」が起きる。⇒ 一覧を [`Projection`] として 1 箇所に定義し、
// 値を作る側は**網羅 match** で受ける。増減すればそこがコンパイルエラーになる。
//
// ⚠⚠ **`_ =>` を書き足してこの仕掛けを無効化しないこと。**
//
// ## 種別が合わないときは黙らない
//
// `^関数.fields` のように種別と射影が噛み合っていないときは**エラーにする**。
// 空のリストを返すと「フィールドが 0 個」と読めてしまい、間違いが通る。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{Accessibility, Expr, FieldKind, Param, Stmt, TemplateParam};
use crate::type_check::MetaKind;

use super::core::Value;
use super::meta::MetaValue;
use super::objects::NamespaceData;

/// `^` に書ける射影の一覧（参考B・D12）。
///
/// ⚠ 追加・削除はここだけを直す。[`project`] が網羅 match なので、合わせ忘れは止まる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    // ── 種別に依らない ──
    /// 対象の名前。
    Name,
    /// 種別の名前（`meta_class` など）。分岐に使う。
    Kind,

    // ── meta_function（参考B #1）──
    /// 仮引数の列。各要素は `name` / `type` / `mutable` / `has_default` / `variadic`。
    Params,
    /// 戻り値の型注釈（無ければ `None`）。
    ReturnType,
    /// `static` 修飾。
    IsStatic,
    /// `class_method` 修飾。
    IsClassMethod,
    /// trait の抽象メソッド宣言か。
    IsAbstract,
    /// アクセス可能性（`public` / `private` / `protected`）。
    Access,
    /// `@` 装飾子の名前の列。⚠ `!装飾子` は展開で消えるのでここには出ない。
    Decorators,

    // ── meta_class（参考B #2 #3 #4 #6）──
    /// フィールドの列（各要素は `meta_member`）。
    Fields,
    /// メソッドの列（各要素は `meta_function`）。
    Methods,
    /// ジェネレータメソッドの列（各要素は `meta_function`）。
    GenMethods,
    /// `const` フィールドの列（クラス変数に相当）。
    StaticFields,
    /// テンプレート型パラメータの列（`name` / `constraints`）。
    TemplateParams,
    /// 基底トレイト名の列。⚠ Arrow は継承を持たずトレイトのみ。
    Bases,
    /// `enum` のバリアント名の列。
    Variants,

    // ── meta_member ──
    /// フィールドの型注釈。
    Type,
    /// `mut` フィールドか。
    Mutable,
    /// 既定値を持つか。⚠ **値そのものはまだ出さない**（展開時に評価する話が要る）。
    HasDefault,

    // ── 位置（参考B #7・タスク 4-7）──
    /// 宣言が書かれた位置（`file` / `line` / `col`）。`compile_error` の文面に使う。
    DeclaredAt,
}

impl Projection {
    /// 射影名から引く。未知なら `None`（呼び出し側が「そんな射影は無い」と言う）。
    pub fn from_name(attr: &str) -> Option<Self> {
        Some(match attr {
            "name" => Self::Name,
            "kind" => Self::Kind,
            "params" => Self::Params,
            "return_type" => Self::ReturnType,
            "is_static" => Self::IsStatic,
            "is_class_method" => Self::IsClassMethod,
            "is_abstract" => Self::IsAbstract,
            "access" => Self::Access,
            "decorators" => Self::Decorators,
            "fields" => Self::Fields,
            "methods" => Self::Methods,
            "gen_methods" => Self::GenMethods,
            "static_fields" => Self::StaticFields,
            "template_params" => Self::TemplateParams,
            "bases" => Self::Bases,
            "variants" => Self::Variants,
            "type" => Self::Type,
            "mutable" => Self::Mutable,
            "has_default" => Self::HasDefault,
            "declared_at" => Self::DeclaredAt,
            _ => return None,
        })
    }
}

/// 射影を引く（参考B・D12）。
///
/// ⚠⚠ **`match` は網羅**。射影を足すとここが止まる。`_ =>` を書かないこと。
/// ⚠ 種別と噛み合わない射影は **`Err`**。空リストを返すと「0 個」と読めて間違いが通る。
pub fn project(m: &Rc<MetaValue>, p: Projection) -> Result<Value, String> {
    match p {
        Projection::Name => Ok(Value::str(m.name.as_str())),
        Projection::Kind => Ok(Value::str(kind_name(m.kind))),

        Projection::Params => Ok(list(params_of(m)?.iter().map(param_record).collect())),
        Projection::ReturnType => Ok(match fn_of(m)? {
            (_, Some(rt), ..) => type_value(rt),
            _ => Value::None,
        }),
        Projection::IsStatic => Ok(Value::Bool(fn_of(m)?.2)),
        Projection::IsClassMethod => Ok(Value::Bool(fn_of(m)?.3)),
        Projection::IsAbstract => Ok(Value::Bool(fn_of(m)?.4)),
        Projection::Access => Ok(Value::str(access_name(fn_or_field_access(m)?))),
        Projection::Decorators => Ok(list(
            fn_of(m)?.5.iter().map(|d| Value::str(decorator_name(d))).collect(),
        )),

        Projection::Fields => Ok(list(members_of(m, MemberKind::Field)?)),
        Projection::Methods => Ok(list(members_of(m, MemberKind::Method)?)),
        Projection::GenMethods => Ok(list(members_of(m, MemberKind::GenMethod)?)),
        Projection::StaticFields => Ok(list(members_of(m, MemberKind::StaticField)?)),
        Projection::TemplateParams => {
            Ok(list(template_params_of(m)?.iter().map(template_param_record).collect()))
        }
        Projection::Bases => Ok(list(
            bases_of(m)?.iter().map(|b| Value::str(b.as_str())).collect(),
        )),
        Projection::Variants => Ok(list(
            variants_of(m)?.iter().map(|(n, _)| Value::str(n.as_str())).collect(),
        )),

        Projection::Type => binding_or_field_type(m),
        Projection::Mutable => Ok(Value::Bool(matches!(field_of(m)?.0, FieldKind::Mut))),
        Projection::HasDefault => Ok(Value::Bool(field_of(m)?.2)),

        Projection::DeclaredAt => {
            let r = src_of(m)?;
            let first = &r.tokens[r.start].span;
            Ok(record(
                "Location",
                vec![
                    ("file", Value::str(first.file.to_string())),
                    ("line", Value::Int(first.line as i64)),
                    ("col", Value::Int(first.col as i64)),
                ],
            ))
        }
    }
}

// ── 種別ごとの取り出し（噛み合わなければ Err）──────────────────────────────

/// 射影が種別と噛み合っていないときの文面。
///
/// ⚠ **何を書いたら合うのかまで言う。** 「使えません」だけだと、種別を取り違えたのか
/// 射影名を間違えたのかが分からない。
fn wrong_kind(m: &MetaValue, wanted: &str) -> String {
    format!(
        "{WRONG_KIND_HEAD}{wanted}, but '{}' is a {}",
        m.name,
        kind_name(m.kind)
    )
}

/// 種別違いの文面の頭。呼び出し側が**射影の名前**に差し替える（[`name_the_projection`]）。
const WRONG_KIND_HEAD: &str = "this projection needs ";

/// 種別違いの文面に**どの射影か**を書き込む（タスク 3-9）。
///
/// ⚠ 射影を引く関数（`fn_of` / `body_of` …）は複数の射影から共有されていて、
/// 自分がどの名前で呼ばれたかを知らない。⇒ 名前を知っている入口（属性アクセスと
/// メソッド呼び出し）で書き込む。
/// ⚠ 以前は「this projection needs a class or trait」とだけ出て、メタ関数の中に射影が
/// 幾つもあると**どれが間違いか分からなかった**。
pub fn name_the_projection(e: String, shown: &str) -> String {
    match e.strip_prefix(WRONG_KIND_HEAD) {
        Some(rest) => format!("`{shown}` needs {rest}"),
        None => e,
    }
}

type FnParts<'a> = (
    &'a Vec<Param>,
    &'a Option<String>,
    bool,
    bool,
    bool,
    &'a [Expr],
);

fn fn_of<'a>(m: &'a Rc<MetaValue>) -> Result<FnParts<'a>, String> {
    match &*m.decl {
        Stmt::FnDef {
            params,
            return_type,
            is_static,
            is_class_method,
            is_abstract,
            decorators,
            ..
        } => Ok((params, return_type, *is_static, *is_class_method, *is_abstract, decorators)),
        // ⚠ `gen` は戻り値が「産出する型」。同じ射影で読めるようにしておく。
        //   ⚠ `gen` は `@` 装飾子を持てないので空の列を返す（`decorators` は常に空）。
        Stmt::GenDef { params, yield_type, .. } => {
            Ok((params, yield_type, false, false, false, &[]))
        }
        _ => Err(wrong_kind(m, "a function or generator")),
    }
}

fn params_of<'a>(m: &'a Rc<MetaValue>) -> Result<&'a Vec<Param>, String> {
    Ok(fn_of(m)?.0)
}

fn fn_or_field_access(m: &Rc<MetaValue>) -> Result<Accessibility, String> {
    match &*m.decl {
        Stmt::FnDef { access, .. } | Stmt::GenDef { access, .. } | Stmt::Field { access, .. } => {
            Ok(access.clone())
        }
        _ => Err(wrong_kind(m, "a function, generator or field")),
    }
}

/// フィールドの `(種別, 型注釈, 既定値の有無)`。
fn field_of(m: &Rc<MetaValue>) -> Result<(FieldKind, &String, bool), String> {
    match &*m.decl {
        Stmt::Field { kind, type_ann, default, .. } => {
            Ok((kind.clone(), type_ann, default.is_some()))
        }
        _ => Err(wrong_kind(m, "a field")),
    }
}

/// フィールドまたは変数束縛の型（タスク 4-1 / 4-4）。
///
/// ⚠ 注釈があれば注釈、無ければ**展開時型推論**の結果（`MetaValue::binding_type`）。
/// ⚠⚠ 推論できなかったら**エラー**。`unknown` のような文字列を型名として返すと、
/// スプライスされたときに壊れた型注釈が黙って通る。
fn binding_or_field_type(m: &Rc<MetaValue>) -> Result<Value, String> {
    match &*m.decl {
        Stmt::Field { type_ann, .. } => Ok(type_value(type_ann)),
        Stmt::Let(_, Some(t), _) | Stmt::Mut(_, Some(t), _) | Stmt::Const(_, Some(t), _) => {
            Ok(type_value(t))
        }
        Stmt::Let(..) | Stmt::Mut(..) | Stmt::Const(..) | Stmt::Static(..) => {
            match &m.binding_type {
                Some(t) => Ok(type_value(t)),
                None => Err(format!(
                    "the type of '{}' could not be inferred at expansion time \
                     (annotate it, e.g. `let {}: int = ...`)",
                    m.name, m.name
                )),
            }
        }
        _ => Err(wrong_kind(m, "a field or a variable binding")),
    }
}

fn body_of<'a>(m: &'a Rc<MetaValue>) -> Result<&'a Vec<Stmt>, String> {
    match &*m.decl {
        Stmt::ClassDef { body, .. } | Stmt::TraitDef { body, .. } => Ok(body),
        _ => Err(wrong_kind(m, "a class or trait")),
    }
}

fn bases_of<'a>(m: &'a Rc<MetaValue>) -> Result<&'a Vec<String>, String> {
    match &*m.decl {
        Stmt::ClassDef { bases, .. } => Ok(bases),
        _ => Err(wrong_kind(m, "a class")),
    }
}

fn variants_of<'a>(m: &'a Rc<MetaValue>) -> Result<&'a Vec<(String, Option<Expr>)>, String> {
    match &*m.decl {
        Stmt::EnumDef { variants, .. } => Ok(variants),
        _ => Err(wrong_kind(m, "an enum")),
    }
}

fn template_params_of<'a>(m: &'a Rc<MetaValue>) -> Result<&'a Vec<TemplateParam>, String> {
    match &*m.decl {
        Stmt::ClassDef { template_params, .. }
        | Stmt::TraitDef { template_params, .. }
        | Stmt::FnDef { template_params, .. }
        | Stmt::GenDef { template_params, .. } => Ok(template_params),
        _ => Err(wrong_kind(m, "a class, trait, function or generator")),
    }
}

/// クラス本体から取り出すメンバーの種類。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MemberKind {
    Field,
    Method,
    GenMethod,
    StaticField,
}

/// クラス／トレイト本体のメンバーをメタ情報の列にする。
///
/// ⚠ `const` フィールドは「クラス変数」に当たるので `static_fields` 側へ分ける
/// （参考B #2 の「`FieldKind` は Mut/Let/Const/StaticMut の 4 種」）。
fn members_of(m: &Rc<MetaValue>, want: MemberKind) -> Result<Vec<Value>, String> {
    let body = body_of(m)?;
    let mut out = Vec::new();
    for st in body {
        let matched = match (want, st) {
            (MemberKind::Field, Stmt::Field { kind, .. }) => {
                matches!(kind, FieldKind::Mut | FieldKind::Let)
            }
            (MemberKind::StaticField, Stmt::Field { kind, .. }) => {
                matches!(kind, FieldKind::Const | FieldKind::StaticMut)
            }
            (MemberKind::Method, Stmt::FnDef { .. }) => true,
            (MemberKind::GenMethod, Stmt::GenDef { .. }) => true,
            _ => false,
        };
        if !matched {
            continue;
        }
        let Some(kind) = MetaValue::kind_of(st) else { continue };
        let Some(name) = MetaValue::name_of(st) else { continue };
        out.push(Value::Meta(Rc::new(MetaValue {
            kind,
            name,
            decl: Rc::new(st.clone()),
            // ⚠ クラス本体のメンバーはフィールドとメソッドだけ。フィールドは型注釈が必須なので
            //   推論は要らない。
            binding_type: None,
        })));
    }
    Ok(out)
}

/// 宣言の元のソースの範囲（タスク 4-7）。
///
/// ⚠ 無いときは**理由を言い分ける**。「合成された宣言」と「そもそも範囲を持たない種類」は
/// 利用者にとって別の間違い（前者は置き場所の問題、後者は対象の取り違え）。
fn src_of(m: &Rc<MetaValue>) -> Result<&crate::ast::SrcRange, String> {
    let src = match &*m.decl {
        Stmt::FnDef { src, .. }
        | Stmt::GenDef { src, .. }
        | Stmt::ClassDef { src, .. }
        | Stmt::TraitDef { src, .. }
        | Stmt::EnumDef { src, .. }
        | Stmt::Field { src, .. } => src,
        _ => {
            return Err(format!(
                "'{}' is a variable binding; its source is not recorded \
                 (only classes, traits, enums, functions and fields keep it)",
                m.name
            ))
        }
    };
    src.as_ref().ok_or_else(|| {
        format!(
            "'{}' was generated rather than written in source \
             (for example an automatic `__init__` or a foreign-language stub), \
             so it has no original code",
            m.name
        )
    })
}

/// 元のトークン列を `Code` の行へ組み直す（タスク 4-7）。
///
/// ⚠ 行の段数は**宣言の先頭からの相対**にする（§1.2 の B-1）。`quote` で置き直したとき、
/// 置き先の段数に揃えられるように。
/// ⚠ `Newline` / `Indent` / `Dedent` は行と段数に写して捨てる（`code:` の行と同じ形）。
fn code_lines_of(r: &crate::ast::SrcRange) -> Vec<crate::ast::CodeLine> {
    use crate::ast::{CodeLine, CodePiece};
    use crate::token::Token;
    let mut lines: Vec<CodeLine> = Vec::new();
    let mut depth: i32 = 0;
    let mut pieces: Vec<CodePiece> = Vec::new();
    let mut span: Option<crate::token::Span> = None;
    for t in &r.tokens[r.start..r.end] {
        match &t.token {
            Token::Indent => depth += 1,
            Token::Dedent => depth -= 1,
            Token::Newline | Token::Semicolon | Token::Eof => {
                if !pieces.is_empty() {
                    lines.push(CodeLine {
                        pieces: std::mem::take(&mut pieces),
                        indent: depth,
                        span: span.take().unwrap_or_else(crate::token::Span::unknown),
                    });
                }
            }
            _ => {
                span.get_or_insert_with(|| t.span.clone());
                pieces.push(CodePiece::Token(t.clone()));
            }
        }
    }
    if !pieces.is_empty() {
        lines.push(CodeLine {
            pieces,
            indent: depth,
            span: span.unwrap_or_else(crate::token::Span::unknown),
        });
    }
    lines
}

// ── メソッド形の射影（参考B #4 #5）────────────────────────────────────────

/// `^T.has_field(n)` / `^T.has_method(n)` / `^T.implements(Trait)`（参考B #4 #5）。
///
/// ⚠ **引数を取るので属性ではなくメソッド。** 「`__eq__` が既にあるなら生成しない」
/// のような分岐に使う（§1.8 で展開時の条件分岐を許した意味がここで出る）。
///
/// 未知のメソッド名は `None`。呼び出し側が「そんなメソッドは無い」と言う。
pub fn meta_method(
    m: &Rc<MetaValue>,
    method: &str,
    args: &[Value],
) -> Option<Result<Value, String>> {
    // `.code()`（設計書 §1.5・タスク 4-7）: 元のコード内容を `Code` として返す。
    if method == "code" {
        if !args.is_empty() {
            return Some(Err("TypeError: code() takes no arguments".to_string()));
        }
        return Some(src_of(m).map(|r| Value::Code(Rc::new(code_lines_of(r)))));
    }
    let shown = format!(".{method}()");
    let want: MemberKind = match method {
        "has_field" => MemberKind::Field,
        "has_method" => MemberKind::Method,
        "implements" => {
            return Some(
                one_str_arg(method, args)
                    .and_then(|t| Ok(Value::Bool(bases_of(m)?.iter().any(|b| *b == t))))
                    .map_err(|e| name_the_projection(e, &shown)),
            )
        }
        _ => return None,
    };
    Some(
        one_str_arg(method, args)
            .and_then(|n| {
                let members = members_of(m, want)?;
                Ok(Value::Bool(members.iter().any(|v| match v {
                    Value::Meta(mm) => mm.name == n,
                    _ => false,
                })))
            })
            .map_err(|e| name_the_projection(e, &shown)),
    )
}

/// 引数がちょうど 1 つの `str` であることを確かめる。
fn one_str_arg(method: &str, args: &[Value]) -> Result<String, String> {
    match args {
        [Value::Str(s)] => Ok(s.to_string()),
        _ => Err(format!(
            "TypeError: {method}() takes exactly one str argument"
        )),
    }
}

// ── 値の組み立て ─────────────────────────────────────────────────────────

fn list(items: Vec<Value>) -> Value {
    Value::List(Rc::new(std::cell::RefCell::new(items)))
}

/// 型注釈を**型の値**にする（D23・タスク 4-2・2026-09-27）。
///
/// ⚠⚠ 以前は型を**文字列**で返していた（`m.type` が `"int"`）。型は型の値で扱う ——
///   比較（`m.type == int`）・合成（`list[m.type]`）・差し込み（`<! m.type !>`）がそのまま書け、
///   文字列の連結で壊れた型が作られることも無い。名前が要るときは `.name`（`"int"`）。
fn type_value(t: &str) -> Value {
    // ⚠ 綴りを揃える（`canonical_type`）。展開時型推論の結果（型検査の表示）は `dict[str, int]` の
    //   ように型注釈と綴りが違うことがある。読めない綴りはそのまま（差し込むときに弾かれる）。
    Value::Type(crate::meta_expand::canonical_type(t).unwrap_or_else(|_| t.to_string()))
}

fn record(type_name: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut members = HashMap::new();
    for (k, v) in fields {
        members.insert(k.to_string(), v);
    }
    Value::Namespace(Rc::new(NamespaceData { name: type_name.to_string(), members, live: None }))
}

/// 仮引数 1 つの記録（参考B #1）。
///
/// ⚠ **既定値そのものは出さない**（`has_default` だけ）。式のまま渡しても使えず、
/// 展開時に評価するなら「いつ・どの環境で」を決める必要がある。⇒ 別途。
fn param_record(p: &Param) -> Value {
    record(
        "Param",
        vec![
            ("name", Value::str(p.name.as_str())),
            (
                "type",
                match &p.type_ann {
                    Some(t) => type_value(t),
                    None => Value::None,
                },
            ),
            ("mutable", Value::Bool(p.mutable)),
            ("has_default", Value::Bool(p.default.is_some())),
            ("variadic", Value::Bool(p.variadic)),
        ],
    )
}

/// テンプレート型パラメータ 1 つの記録（参考B #3）。
fn template_param_record(tp: &TemplateParam) -> Value {
    record(
        "TemplateParam",
        vec![
            ("name", Value::str(tp.name.as_str())),
            (
                "constraints",
                list(tp.constraints.iter().map(|c| Value::str(c.as_str())).collect()),
            ),
        ],
    )
}

/// `@` 装飾子の名前。
///
/// ⚠ `@deco` と `@deco(...)` の両方を名前として読む。それ以外の式は名前を持たないので
/// そのまま出す（**黙って落とさない** —— 装飾子が消えたように見えるのを避ける）。
fn decorator_name(d: &Expr) -> String {
    match d {
        Expr::Ident { name, .. } => name.clone(),
        Expr::Call { func, .. } => decorator_name(func),
        other => format!("<{}>", crate::vm::compiler::expr_kind(other)),
    }
}

/// 種別の表示名。⚠ **網羅 match**。種別を足したらここが止まる。
pub fn kind_name(kind: MetaKind) -> &'static str {
    match kind {
        MetaKind::Instance => "meta_instance",
        MetaKind::Function => "meta_function",
        MetaKind::Class => "meta_class",
        MetaKind::Member => "meta_member",
    }
}

/// アクセス可能性の表示名。⚠ **網羅 match**。
fn access_name(a: Accessibility) -> &'static str {
    match a {
        Accessibility::Public => "public",
        Accessibility::Private => "private",
        Accessibility::Protected => "protected",
    }
}
