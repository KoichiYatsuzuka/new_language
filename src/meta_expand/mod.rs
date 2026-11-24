// meta_expand/mod.rs — コンパイル時メタ関数の展開器（設計書 §1.7 / タスク 2-0）。
//
// ## 何をするか
//
// パース済みの文の列を**ファイル先頭から逐次に**歩き、
//
//   * `exprconst fn` / `exprconst !fn` の定義を**登録して AST から消す**
//   * 配置メタ関数の呼び出し文を、`quote` した `Code` の中身で**置き換える**
//   * `!装飾子` 付きの宣言を、装飾子が返した `Code` の中身で**置き換える**
//
// を行う。⚠ **繰り返し（不動点）はしない**（§1.7）。⇒ メタ関数・装飾子は
// **自分より前に宣言されたものだけ**を見られる。
//
// ## なぜ「置き換えてから続きを歩く」のか
//
// 置いたコードがさらにメタ関数を呼ぶことはある。これは**反復ではなく、同じ前向きの
// 走査を続けているだけ**なので §1.7 の「前方のみ参照」は壊れない。
// ⚠ ただし自分自身を置き続けるメタ関数は止まらないので、暫定の歩数上限を置いてある
// （ちゃんとした診断は 2-6）。
//
// ## メタ関数本体の実行
//
// **通常のインタプリタ（VM 経由）をそのまま使う。** メタ関数を `Stmt::FnDef` に写して
// 定義し、普通の関数として呼ぶ。
// ⚠ この方式が成立するのは 0-4 / 0-5 で評価コアを切り出してあるから。
// ⚠ `code:` は `Op::MakeCode` / `Op::Const`、`quote` は `Op::Return` に落ちる
//   （`src/vm/compiler/`）。⇒ 展開器のためだけの評価器は要らない。

use std::rc::Rc;

mod modules;
mod monomorph;
// ⚠ エディタ（wasm）用。CLI は展開器の中で単相化するので、ルート crate では使わない。
#[allow(unused_imports)]
pub use monomorph::monomorphize;
mod ordinary;

use crate::ast::{CodeLine, CodePiece, Expr, Stmt};
use crate::interpreter::{Interpreter, Value};
use crate::token::{Span, Spanned, Token};

/// 展開器が最初に読み込む前口上（設計書 タスク 3-1）。
///
/// ## なぜ要るのか
///
/// メタ関数を**2 回以上呼ぶ**と、置いたコードの局所名がぶつかる。
///
/// ```ar
/// exprconst !fn twice() -> None:
///     quote code:
///         let tmp = 1
/// twice()
/// twice()      # ← `tmp` が二重宣言になる
/// ```
///
/// ⇒ 名前を**毎回作る**手段が要る。`gensym("tmp")` は接頭辞に連番を付けた名前を返す。
/// スプライスと組み合わせて `let <! gensym("tmp") !> = 1` と書く。
///
/// ## なぜ Rust の組み込みではなく Arrow で書くのか
///
/// `static mut` という**既にある道具**でそのまま書けるから。組み込みを足すと
/// 「展開中だけ生えている名前」を字句・型検査・補完のどこまで見せるかを決める話になり、
/// 得られるものに対して手数が多い。
///
/// ⚠ **展開用インタプリタにしか入らない。** 実行時に `gensym` を呼んでも未定義。
/// 展開時にしか意味の無い道具なのでそれでよい。
/// ⚠ 利用者が `gensym` という名前のメタ関数を書くと二重宣言で弾かれる（黙って負けない）。
/// ⚠ 返す名前は**普通の識別子**なので、利用者が同じ綴りを自分で書いていればぶつかる。
///    そのときは二重宣言エラーになる（黙って壊れない）。
const PRELUDE: &str = "\
class MetaAbort(Error):
    pass

fn compile_error(message: str) -> None:
    raise MetaAbort(message)

fn gensym(prefix: str) -> str:
    static mut gensym_counter = 0
    gensym_counter = gensym_counter + 1
    return prefix + \"__\" + str(gensym_counter)

fn __enter_placing(name: str) -> None:
    if not __placing_armed:
        compile_error(\"placing metafunction '\" + name + \"' can only run as a statement of the program \
or as a `!` decorator; here it was called from inside a metafunction or as part of an expression, \
where there is nowhere to place its code (to reuse its code, make it a pure metafunction that \
returns `Code`)\")
    __placing_armed = False
";

/// `compile_error` が投げる例外の名前（タスク 3-2）。
///
/// ⚠ 整形済みの報告からこの名前を探して、**利用者の書いた文面だけ**を出す。
/// traceback ごと出すと「展開器の内部が漏れている」ようにしか見えない。
const ABORT_CLASS: &str = "MetaAbort";

/// 「いま展開器が配置メタ関数を呼ぼうとしている」印（設計書 §1.1 の呼び出し可能性・タスク 3-8）。
///
/// ⚠⚠ **配置メタ関数を呼んでよいのは展開器だけ**（§1.1 の表: `exprconst fn` からも
/// `exprconst !fn` からも**不可**）。メタ関数の本体から呼ぶと `quote` の結果は
/// ただの返り値になって**黙って捨てられた**（実測）—— 置いたつもりのコードが消える。
/// ⇒ 展開器は呼ぶ直前にこの旗を立て、配置メタ関数の入口（`__enter_placing`）が旗を
/// **倒しながら**確かめる。旗が立っていない呼び出しは展開器以外からのもの。
/// ⚠ 旗は入口で倒すので、本体の中からさらに配置メタ関数を呼ぶと必ず捕まる。
const PLACING_ARMED: &str = "__placing_armed";

/// 展開時にだけ型の値として束縛する型の構築子（D23・タスク 4-2）。`list[t]` などで型を合成する。
///
/// ⚠ `int` / `str` / `float` / `bool` / `dict` / `set` / `function` は組み込みの大域に最初から
///   型の値としてあるので、ここには入れない（入れると二重に宣言される）。
const TYPE_CONSTRUCTORS: [&str; 6] = ["list", "tuple", "fixed_list", "list_like", "Option", "Result"];

/// 展開器側で起きた失敗の段（診断の言い回しを選ぶ・タスク 3-8）。
///
/// ⚠ **どこで起きたかを言い分ける。** 実引数の評価で `compile_error` したのに
/// 「メタ関数 'bind' が止めた」と言うと、`bind` の本体を探しに行ってしまう。
#[derive(Clone, Copy)]
enum Stage<'a> {
    /// メタ関数の本体を走らせている。
    Body(&'a str),
    /// メタ関数呼び出しの実引数を評価している。
    Args(&'a str),
    /// `!装飾子` の式を評価している（`!repeat(3)` の `repeat(3)`）。
    Decorator(&'a str),
}

impl Stage<'_> {
    /// `compile_error` の文面に添える「どこで止めたか」。
    fn raised_by(self) -> String {
        match self {
            Stage::Body(n) => format!("raised by metafunction '{n}'"),
            Stage::Args(n) => format!("raised while evaluating the arguments of '{n}'"),
            Stage::Decorator(n) => format!("raised while evaluating decorator '!{n}'"),
        }
    }

    /// それ以外の失敗の前置き。
    fn during(self) -> String {
        match self {
            Stage::Body(n) => format!("while expanding '{n}'"),
            Stage::Args(n) => format!("while evaluating the arguments of '{n}'"),
            Stage::Decorator(n) => format!("while evaluating decorator '!{n}'"),
        }
    }
}

/// 1 回の展開で歩ける文の数の上限（設計書 タスク 2-6）。
///
/// ⚠⚠ **自分自身を置き続けるメタ関数は止まらない。** 展開は前向き走査だが、置いたコードは
/// その場から歩き直す（§1.7 の「反復しない」は**前方参照の話**で、置いた分を歩くのは反復ではない）
/// ので、`quote code:` が自分の呼び出しを置けば無限に伸びる。⇒ 歩数で切る。
///
/// ⚠ **再帰深さは別の柵が受け持つ。** メタ関数がメタ関数を呼ぶのは展開器から見れば
/// 1 回の呼び出しの中の出来事で、インタプリタの `RecursionError`（上限 1000）が止める。
/// ⇒ ここで二重に数えない。代わりに、その例外が**読める形で外へ出る**ようにしてある
/// （`describe_failure` の `RAISE_SENTINEL` の扱い）。
///
/// ⚠ 10 万文は「人が書いた展開では絶対に届かないが、暴走は数秒で止まる」あたり。
const STEP_BUDGET: usize = 100_000;

/// 雛形に残っているスプライス（`<! !>`）の数。
///
/// ⚠ `Op::MakeCode` はこの数だけスタックから値を取る。**コンパイラが積む順と
/// ここが数える順は同じ**（行→要素の出現順）でなければならない。
pub fn splice_count(lines: &[CodeLine]) -> usize {
    lines
        .iter()
        .flat_map(|l| l.pieces.iter())
        .filter(|p| matches!(p, CodePiece::Splice(_)))
        .count()
}

/// 雛形のスプライス穴へ、評価済みの値を**出現順に**差し込む（設計書 §1.4）。
///
/// | 値 | 差し込まれるもの |
/// |---|---|
/// | `str` | 識別子 1 つ（変数名・関数名・型名） |
/// | `int` / `float` / `bool` | そのリテラル |
/// | 型の値（`int` / `m.type` / `list[t]`） | その型（字句の並び・型として読めることを確かめる） |
///
/// ⚠ **列はまだ扱えない**（設計書 §1.4 の「列」・引数シグネチャの列は 4-1）。⇒ 明示エラーにして、
/// 「黙って変なトークンが埋まる」ことを避ける。
pub fn fill_splices(lines: &[CodeLine], values: &[Value]) -> Result<Vec<CodeLine>, String> {
    let mut it = values.iter();
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let mut pieces = Vec::with_capacity(line.pieces.len());
        for piece in &line.pieces {
            match piece {
                CodePiece::Token(t) => pieces.push(CodePiece::Token(t.clone())),
                CodePiece::Splice(_) => {
                    let v = it.next().ok_or_else(|| {
                        "internal — fewer spliced values than holes".to_string()
                    })?;
                    // ⚠ 差し込むトークンには元の位置が無い。⇒ **行の位置**を使う（タスク 2-3）。
                    // ⚠ 型の値は字句が複数になる（`list [ int ]`・タスク 4-2）。
                    for t in splice_tokens(v, &line.span)? {
                        pieces.push(CodePiece::Token(t));
                    }
                }
            }
        }
        out.push(CodeLine { pieces, indent: line.indent, span: line.span.clone() });
    }
    Ok(out)
}

/// スプライスされた 1 つの値をトークンへ写す。
///
/// ⚠ 位置は**差し込まれる行の位置**（タスク 2-3）。値の側には位置が無いので、
/// これが「どこに差し込まれたか」を指せる唯一の手がかり。
fn splice_tokens(v: &Value, span: &Span) -> Result<Vec<Spanned>, String> {
    // ⚠⚠ **型の値は型として差し込む**（D23・タスク 4-2 / 4-3 の後半・2026-09-27）。
    //   `^` の `.type` は型の値を返すので、`let x: <! m.type !> = ...` がそのまま書ける。
    //   型として読めることを確かめてから、本物の字句解析器で字句に分ける（型の文字列を
    //   組み立てて差し込む道は 4-3 で塞いである・壊れた型が黙って通らない）。
    if let Value::Type(t) = v {
        let t = canonical_type(t)?;
        return Ok(crate::lexer::Lexer::new(&t, "<splice>")
            .tokenize()
            .into_iter()
            .filter(|tok| !matches!(tok.token, Token::Newline | Token::Eof))
            .map(|tok| Spanned { token: tok.token, span: span.clone() })
            .collect());
    }
    let token = match v {
        // ⚠ `str` は**識別子**になる（文字列リテラルではない）。設計書 §1.4 の一段目。
        //   名前を組み立てて差し込むのがスプライスの主用途なので、ここが既定。
        // ⚠⚠ **識別子として正しいかを確かめる**（タスク 4-3）。確かめないと
        //   `"a b"` から空白入りの変数が**黙って作られ**、`"list[int]"` を型の位置へ
        //   差し込めてしまう（どちらも実測で完走した）。後者は D23 が
        //   「壊れた型文字列が黙って通る経路」と名指しした穴そのもの。
        Value::Str(s) => Token::Ident(checked_identifier(s)?),
        Value::Int(n) => Token::Int(*n),
        Value::Float(f) => Token::Float(*f),
        Value::Bool(b) => {
            if *b {
                Token::True
            } else {
                Token::False
            }
        }
        other => {
            return Err(format!(
                "`<! !>` cannot splice a '{}' \
                 (str / int / float / bool and types are supported)",
                crate::interpreter::ops::typecheck::runtime_type_name(other)
            ))
        }
    };
    Ok(vec![Spanned { token, span: span.clone() }])
}

/// 型の値の文字列が**型として読める**ことを確かめ、正規の綴りを返す（D23・タスク 4-3 の後半）。
///
/// 型の合成（`list[t]`）・差し込み（`<! t !>`）・`^` の射影がここを通る。読めなければ展開時の
/// エラー（壊れた型を黙って置かない）。⚠ 読み方は型注釈と同じパーサ、判定は型検査と同じ `from_ann`
/// （自前の規則を書かない）。綴りを揃えるので、同じ型の値どうしは `==` で等しい。
pub(crate) fn canonical_type(t: &str) -> Result<String, String> {
    let not_a_type = |why: String| format!("TypeError: '{t}' is not a type ({why})");
    let canon = crate::parser::canonical_type_text(t).map_err(not_a_type)?;
    match crate::type_check::InferredType::from_ann(&canon) {
        Some(ty) if !matches!(ty, crate::type_check::InferredType::Unresolved) => Ok(canon),
        _ => Err(not_a_type("the type checker cannot read it".to_string())),
    }
}

// ── `Code` の演算（設計書 §1.2 / タスク 2-10）─────────────────────────────

/// `a + b` — 行を**そのまま**つなぐ（設計書 §1.2）。
///
/// ⚠ 段数は各断片の先頭からの相対なので、**同じ段から始まる断片どうし**なら何も直さずに
/// つなげる。段をずらしたいときは `.indent()` を先にかける（メソッド形なので `+` より強く
/// 結合する —— `a + b.indent()` は `a + (b.indent())`）。
pub fn code_concat(a: &[CodeLine], b: &[CodeLine]) -> Vec<CodeLine> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    out.extend_from_slice(a);
    out.extend_from_slice(b);
    out
}

/// `.indent()` / `.dedent()` — 全行の段数を `delta` だけずらす（設計書 §1.2）。
///
/// ⚠⚠ **0 未満になる行が生じる操作はエラー**（§1.2）。黙って 0 で止めると、
/// 入れ子の関係が崩れた断片が「それらしく」置かれてしまう。
pub fn code_shift(lines: &[CodeLine], delta: i32) -> Result<Vec<CodeLine>, String> {
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        let indent = l.indent + delta;
        if indent < 0 {
            return Err(format!(
                "`.dedent()` would move a line of this `Code` left of its start \
                 (line indent {} - 1 < 0); dedent only what was indented",
                l.indent
            ));
        }
        out.push(CodeLine { pieces: l.pieces.clone(), indent, span: l.span.clone() });
    }
    Ok(out)
}

/// 1 行ずつの反復（設計書 §1.2）。各要素は**1 行だけの `Code`**。
///
/// ⚠ 段数はそのまま運ぶ（相対のまま）。行ごとに `.indent()` してつなぎ直す、のような
/// 使い方で、元の入れ子を保てるように。
pub fn code_lines_as_values(lines: &[CodeLine]) -> Vec<Value> {
    lines
        .iter()
        .map(|l| Value::Code(Rc::new(vec![l.clone()])))
        .collect()
}

/// スプライスした `str` が**そのまま 1 つの識別子として字句解析される**ことを確かめる（タスク 4-3）。
///
/// ⚠⚠ **字句規則を自前で書き直さない。** 本物の字句解析器にかけて「`Ident` がちょうど
/// 1 つだけ出る」ことを見る。こうすれば空白・記号・予約語（`if` は `Ident` ではなく
/// `Token::If` になる）・Unicode の扱いが**言語と食い違わない**。
///
/// ⚠ 型を差し込みたいとき（`list[int]` など）はここで弾かれる。それは正しい ——
/// 文字列を型の位置へ差し込む道は D23 が塞ぐと決めた経路で、正式な手段は
/// 型の値（`m.type` / `list[t]`・タスク 4-2）を差し込むこと。
fn checked_identifier(s: &str) -> Result<String, String> {
    let toks = crate::lexer::Lexer::new(s, "<splice>").tokenize();
    let mut meaningful = toks
        .iter()
        .filter(|t| !matches!(t.token, Token::Newline | Token::Eof));
    let ok = matches!(
        (meaningful.next(), meaningful.next()),
        (Some(Spanned { token: Token::Ident(name), .. }), None) if name == s
    );
    if ok {
        return Ok(s.to_string());
    }
    Err(format!(
        "`<! !>` spliced the str {s:?}, which is not a single identifier. \
         A spliced str becomes a name (a variable, function or type name); \
         composite types such as `list[int]` cannot be built from a str"
    ))
}

/// `Code`（未パースのトークン行）を文の列へ戻す（設計書 §1.2 / タスク 4-5 の①）。
///
/// ⚠⚠ **パースはここ 1 回だけ**。`code:` の中身を書いた時点ではパースしていないので、
/// 未完の断片（本体の無い `if` など）を組み立てられる。組み上がったものを最後に一度
/// パースする、というのが §1.2 の設計。
///
/// ⚠ 行に持っているのは**断片先頭からの相対インデント**なので、`Indent` / `Dedent` を
/// ここで作り直す。字句解析はやり直さない（トークンはもう持っている）。
/// 置き先の文脈。**読み方が変わる**ので区別が要る（タスク 2-0）。
///
/// ⚠⚠ `mut x: int` は最上位では「初期値の無い変数宣言」でエラー、クラス本体では
/// フィールド宣言。⇒ 同じ `Code` でも置き先によってパースし分けなければならない。
/// ⚠ クラス本体に置ける種別の適合検査（`field`/`fn`/`gen` のみ等）は 2-5。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// 最上位・関数本体など、普通の文が書ける場所。
    TopLevel,
    /// クラス／トレイト本体。
    TypeBody,
}

/// 置き先の文脈と **node-id カウンタ**を指定して `Code` を文の列へ戻す（タスク 2-1）。
///
/// ⚠⚠ **カウンタを渡さないと採番が 0 から始まり、本体の node-id と衝突する。**
/// node-id はプログラム全体で一意という前提で注釈が引かれる（設計書 §0.3）ので、
/// 衝突すると**別のノードの注釈を読む**。⇒ 展開器は必ず渡す。
/// ⚠ 同じ `Code` を 2 回置いても、そのたびにパースし直すので採番は自然に分かれる
/// （AST を複製して配り回さないので「クローン時の再採番」が要らない設計になっている）。
pub fn code_to_stmts_with(
    lines: &[CodeLine],
    ctx: Context,
    counter: Option<std::rc::Rc<std::cell::Cell<u32>>>,
) -> Result<Vec<Stmt>, String> {
    let mut tokens: Vec<Spanned> = Vec::new();
    let mut depth: i32 = 0;
    // 末尾の `Dedent` / `Eof` に使う位置。⚠ 位置不明のまま出すと、置いたコードの
    // 構文エラーが「どこで間違えたのか分からない」診断になる（タスク 2-3）。
    let mut last_span = Span::unknown();
    for line in lines {
        last_span = line.span.clone();
        while depth < line.indent {
            tokens.push(Spanned { token: Token::Indent, span: line.span.clone() });
            depth += 1;
        }
        while depth > line.indent {
            tokens.push(Spanned { token: Token::Dedent, span: line.span.clone() });
            depth -= 1;
        }
        for piece in &line.pieces {
            match piece {
                CodePiece::Token(t) => tokens.push(t.clone()),
                // `Op::MakeCode` を通っていれば穴は残らない。残っているのは
                // 「スプライス付きの `Code` を評価せずに配置しようとした」ときだけ。
                CodePiece::Splice(_) => {
                    return Err(
                        "MetaError: this `Code` still has an unevaluated `<! !>` splice".to_string()
                    )
                }
            }
        }
        tokens.push(Spanned { token: Token::Newline, span: line.span.clone() });
    }
    while depth > 0 {
        tokens.push(Spanned { token: Token::Dedent, span: last_span.clone() });
        depth -= 1;
    }
    tokens.push(Spanned { token: Token::Eof, span: last_span });

    let mut parser = crate::parser::Parser::new(tokens, None);
    if let Some(c) = counter {
        parser.set_node_counter(c);
    }
    let parsed = match ctx {
        Context::TopLevel => parser.parse_program(),
        Context::TypeBody => parser.parse_class_body_fragment(),
    };
    // ⚠⚠ 止まった位置を添える（タスク 5-1 で発見）。置いたトークンは `code:` に**書かれた位置**を
    //   持っている（2-3）のに、この文面では使っておらず、呼び出し位置しか出ていなかった ——
    //   `code:` が幾つもあると、どの断片が壊れているのか分からない。
    let stmts = match parsed {
        Ok(stmts) => stmts,
        Err(e) => {
            let at = parser.current_span();
            return Err(if at.line == 0 {
                format!("MetaError: the placed `Code` does not parse: {e}")
            } else {
                format!(
                    "MetaError: the placed `Code` does not parse: {e} \
                     (at {at}, inside the `code:` block)"
                )
            });
        }
    };
    if ctx == Context::TypeBody {
        check_type_body_members(&stmts)?;
    }
    Ok(stmts)
}

/// クラス／トレイト本体へ置いた `Code` が**メンバー宣言だけ**でできているか（タスク 2-5）。
///
/// 置けるのは**フィールド宣言・メソッド定義（`fn` / `gen`、`static` / `class_method` 込み）**だけ。
///
/// ⚠⚠ **`pass` を弾くのがここの主目的。** `parse_class_stmt` は手書きの空クラスのために
/// `pass` を受け付けるので、素通しすると「装飾子が `pass` を置いてメンバーが**黙って消える**」
/// という形になる（実測）。メンバーを消したいなら**空の `Code`** を `quote` する
/// （設計書 §1.2 の B-2）。そちらは「意図して何も置かない」と読める。
///
/// ⚠ `!装飾子` は通す。置いた直後に同じ前向き走査が展開する。
fn check_type_body_members(stmts: &[Stmt]) -> Result<(), String> {
    for st in stmts {
        match st {
            Stmt::Field { .. }
            | Stmt::FnDef { .. }
            | Stmt::GenDef { .. }
            | Stmt::MetaDecorated { .. } => {}
            other => {
                return Err(format!(
                    "MetaError: `{}` cannot be placed in a class or trait body \
                     (only field declarations and `fn` / `gen` methods can); \
                     to remove the member instead, `quote` an empty `code:` block",
                    crate::interpreter::tw_stats::stmt_kind_of(other)
                ))
            }
        }
    }
    Ok(())
}

/// 出所の列に積む上限（タスク 3-3）。
///
/// ⚠⚠ **上限が無いと暴走したメタ関数で二次時間になる。** 自分の呼び出しを置き続ける
/// メタ関数は歩数予算（10 万）まで走るが、その 1 歩ごとに列が 1 段伸びる。列は
/// 積むたびに複製するので、上限なしだと 10 万 × 10 万 の複製になる
/// （実測: `cargo test` が 1 秒から **614 秒**になった）。
///
/// ⚠ 溢れたら**外側を残す**。利用者が実際に書いた呼び出しは一番外側にあり、
/// 内側は同じ枠の繰り返しなので、そちらを捨てる方が読める。
const MAX_TRAIL: usize = 16;

/// 「この文はどのメタ関数がどこから置いたか」の 1 段（タスク 3-3）。
#[derive(Clone)]
struct Frame {
    /// 展開したメタ関数の名前。
    name: String,
    /// **呼び出し位置**。置いたコードの中の呼び出しなら、2-3 のおかげで
    /// `code:` のその行を指す。
    span: Span,
}

/// 出所の列を人が読む形にする（タスク 3-3）。
///
/// ⚠⚠ **これが無いと「どのメタ関数の中で起きたか」しか分からない。** 置いたコードが
/// さらにメタ関数を呼ぶ形（`outer` が `inner()` を置く）では、失敗した地点だけ見ても
/// **なぜそこに `inner()` があるのか**が追えない。
fn render_trail(trail: &[Frame]) -> String {
    let mut out = String::new();
    if trail.len() >= MAX_TRAIL {
        out.push_str(&format!(
            "\n  ... (the expansion trail is deeper than {MAX_TRAIL}; only the outermost frames are kept)"
        ));
    }
    // 内側（直近）から外側へ。traceback と同じ並びにする。
    for f in trail.iter().rev() {
        out.push('\n');
        if f.span.line == 0 {
            out.push_str(&format!("  while expanding '{}'", f.name));
        } else {
            out.push_str(&format!("  while expanding '{}' at {}", f.name, f.span));
        }
    }
    out
}

// ── 展開器本体 ───────────────────────────────────────────────────────────────

/// 展開器の状態。メタ関数の登録簿と、それを実行するインタプリタを持つ。
struct Expander {
    /// メタ関数名 → 配置メタ関数（`exprconst !fn`）かどうか。
    ///
    /// ⚠ 実体（呼べる関数値）は `interp` のグローバルスコープに入っている。
    /// ここが持つのは「メタ関数である」ことと種別だけ。
    metafns: std::collections::HashMap<String, bool>,
    /// メタ関数本体を走らせるインタプリタ。
    ///
    /// ⚠ **通常実行のインタプリタとは別物**。展開時に作った変数が実行時へ漏れない。
    interp: Interpreter,
    /// 歩数（`STEP_BUDGET` の判定用）。
    steps: usize,
    /// 直近に展開したメタ関数名（予算超過の診断に使う・タスク 2-6）。
    ///
    /// ⚠ 「予算を使い切った」だけでは**どれが暴走したのか分からない**。
    last_expanded: Option<String>,
    /// **実行時に宣言される**名前（設計書 §1.8 / タスク 3-0）。
    ///
    /// ⚠⚠ **診断のためだけに持っている。** 展開時に確定するのはリテラル・メタ関数の仮引数・
    /// メタ関数の返り値・展開済み `const` だけ（§1.8）なので、それ以外を読もうとすると
    /// 展開器の中では `NameError` になる。だが利用者から見れば**書いた覚えのある名前**なので、
    /// そのままでは「定義したのに無いと言われた」と読めてしまう。⇒ ここに載っていたら
    /// 「それは実行時の値だ」と言い直す。
    ///
    /// ⚠ **これは先行走査だが §1.7 は壊さない。** 展開の可否や結果には一切影響せず、
    /// エラー文面を選ぶだけ。展開そのものは前方のみを見る。
    runtime_names: std::collections::HashSet<String>,
    /// パース時に集めた trait の宣言情報（タスク 2-4）。
    ///
    /// ⚠ 装飾子でメンバーが増えたクラスは、**展開後に** `finalize_class_body` を
    /// 走らせないと自動 `__init__` に増えたフィールドが入らない。
    known_traits: std::collections::HashMap<String, crate::parser::TraitInfo>,
    /// このプログラムが**メタ関数を持つか**（最上位に `exprconst` があるか）。
    ///
    /// ⚠ 偽なら次の 2 つを**やらない**（どちらも全文の複製を伴う）:
    ///   - 展開済みの最上位の文を展開時型推論のために共有する（タスク 4-4）
    ///   - `^` のための宣言表を埋める（タスク 4-0。3-9 で省くようにした）
    ///   メタ関数が無ければ `^` は展開時に評価されず、推論も起きえない。
    has_metafns: bool,
    /// 置いたコードを採番する node-id カウンタ（設計書 §0.3 / タスク 2-1）。
    ///
    /// ⚠⚠ **本体をパースしたパーサと同じものを渡す。** 別のカウンタだと node-id が
    /// 衝突し、型検査の注釈が**別のノードのもの**になる（per-module 採番で実際に
    /// FFI 境界検査の誤検知が再現した — `Parser::node_counter` の doc）。
    node_counter: std::rc::Rc<std::cell::Cell<u32>>,
    /// 配置メタ関数の**実体**（タスク 3-8）。呼ぶ直前に [`PLACING_ARMED`] を立てるかの判定に使う。
    ///
    /// ⚠ **名前ではなく実体で見る。** `const my_deco = tagged` のように別名で呼ばれても、
    /// 装飾子ファクトリが返したクロージャ（配置メタ関数ではない）と取り違えないように。
    placing_fns: Vec<std::rc::Rc<crate::interpreter::value::FnValue>>,
    /// 今の枠で `import` したモジュール（束縛名 → 公開しているメタ関数）（タスク 2-12）。
    ///
    /// ⚠ `m.greet()` を配置呼び出しと見分けるのに使う。`metafns` と同じく**枠ごと**。
    modules: std::collections::HashMap<String, std::collections::HashMap<String, bool>>,
    /// 展開済みのモジュール（鍵は `(lang, モジュールパス)`・タスク 2-12）。
    ///
    /// ⚠ モジュールは**初めて読み込まれるとき**に 1 回だけ展開する（§1.7・2026-09-25 確定）。
    module_cache: std::collections::HashMap<String, std::rc::Rc<modules::ExpandedModule>>,
    /// 展開時の単相化の状態（タスク 2-8）。テンプレートは**枠ごと**、具体化の数は全体で 1 つ。
    mono: monomorph::Mono,
}

impl Expander {
    fn new(
        node_counter: std::rc::Rc<std::cell::Cell<u32>>,
        known_traits: std::collections::HashMap<String, crate::parser::TraitInfo>,
    ) -> Self {
        let mono = monomorph::Mono::new(std::rc::Rc::clone(&node_counter));
        Self {
            metafns: std::collections::HashMap::new(),
            interp: Interpreter::new(),
            steps: 0,
            last_expanded: None,
            runtime_names: std::collections::HashSet::new(),
            has_metafns: false,
            known_traits,
            node_counter,
            placing_fns: Vec::new(),
            modules: std::collections::HashMap::new(),
            module_cache: std::collections::HashMap::new(),
            mono,
        }
    }

    /// `compile_error("...")` で止めたときの文面を取り出す（タスク 3-2）。
    ///
    /// ⚠ **利用者が書いた文面だけを出す。** メタ関数の作者が「こう書いてはいけない」と
    /// 伝えるための道具なので、展開器の traceback を被せると肝心の一文が埋もれる。
    fn compile_error_message(report: &str) -> Option<String> {
        let head = format!("{ABORT_CLASS}: ");
        let i = report.rfind(&head)? + head.len();
        let rest = &report[i..];
        let end = rest.find('\n').unwrap_or(rest.len());
        Some(rest[..end].trim_end().to_string())
    }

    /// 展開時の `NameError` が**実行時の宣言**を指していたら、そう言い直す（設計書 §1.8）。
    ///
    /// ⚠ 例外の実体から名前を取り出す API が無いので、整形済みの報告から拾っている。
    /// 拾えなければ `None` を返して元の報告をそのまま出すだけなので、**外すと黙るのではなく
    /// 元に戻る**（誤検知で正しい診断を隠すことはない）。
    fn runtime_name_hint(&self, report: &str) -> Option<String> {
        const HEAD: &str = "NameError: '";
        const TAIL: &str = "' is not defined";
        let i = report.find(HEAD)? + HEAD.len();
        let j = report[i..].find(TAIL)? + i;
        let name = &report[i..j];
        if !self.runtime_names.contains(name) {
            return None;
        }
        Some(format!(
            "'{name}' is declared at run time, so it is not available while expanding. \
             A metafunction can only use values that are settled at expansion time: \
             literals, its own parameters, results of other metafunctions, and expanded `const`s"
        ))
    }

    /// 置き先の文脈に合わせて `Code` を文へ戻す（採番は本体と共有）。
    fn place(&self, code: &[CodeLine], ctx: Context) -> Result<Vec<Stmt>, String> {
        code_to_stmts_with(code, ctx, Some(std::rc::Rc::clone(&self.node_counter)))
    }

    /// 前口上（`gensym` など）を展開用インタプリタへ読み込む（タスク 3-1）。
    ///
    /// ⚠ ここが失敗したら**展開そのものを止める**。前口上が入っていないのに展開を続けると、
    /// `gensym` を使ったメタ関数が「未定義の名前」という無関係な診断で落ちる。
    fn load_prelude(&mut self) -> Result<(), String> {
        // ⚠ 展開時の `print` は stderr へ（設計書 §4.1）。stdout に混ざると
        //   `compare_python_impl` / `compare_outputs` の差分比較が壊れる。
        self.interp.set_meta_expanding(true);
        // 展開中の VM は命令数を数える（D5）。前口上の実行にも上限を張る。
        self.interp.meta_reset_budget();
        // ⚠ 旗は Rust 側で宣言する。前口上に `mut` の最上位宣言を書いて `exec` へ渡すと、
        //   最上位の文は VM 経由で走る決まりなので `VmForceError` になる（3-6 で実測）。
        self.interp
            .vm_declare_global(
                PLACING_ARMED,
                crate::vm::op::DeclKind::Mut,
                &[],
                Value::Bool(false),
            )
            .map_err(|e| format!("MetaError: internal — {e}"))?;
        let tokens = crate::lexer::Lexer::new(PRELUDE, "<meta-prelude>").tokenize();
        let stmts = crate::parser::Parser::new(tokens, None)
            .parse_program()
            .map_err(|e| format!("MetaError: internal — the expander prelude does not parse: {e}"))?;
        for st in &stmts {
            self.interp
                .exec(st)
                .map_err(|e| format!("MetaError: internal — the expander prelude failed: {e}"))?;
        }
        // ⚠⚠ **型の構築子を型の値として束縛する**（D23・タスク 4-2）。`list[t]` / `Option[t]` /
        //   `tuple[a, b]` が型の値を返す（`Interpreter::compose_type`）。`int` / `str` / `dict` / `set`
        //   などは組み込みの大域に既に型の値としてある。ここで足すのは実行時には値として無いものだけ。
        // ⚠ **展開用インタプリタにだけ**置く。実行時の大域に `list` を足すと、利用者が変数名に
        //   使った `list` が「既に宣言されている」になる。
        for ctor in TYPE_CONSTRUCTORS {
            self.interp
                .vm_declare_global(
                    ctor,
                    crate::vm::op::DeclKind::Const,
                    &[],
                    Value::Type(ctor.to_string()),
                )
                .map_err(|e| format!("MetaError: internal — {e}"))?;
        }
        // ⚠ モジュールの展開の枠にも前口上を置く（枠ごとに大域が分かれている・名前空間の分離）。
        //   旗（[`PLACING_ARMED`]）は置かない —— 旗は 1 つでなければならない。
        let mut prelude: Vec<&str> = vec![ABORT_CLASS, "compile_error", "gensym", "__enter_placing"];
        prelude.extend(TYPE_CONSTRUCTORS);
        self.interp.meta_set_prelude(&prelude);
        Ok(())
    }

    /// メタ関数を「普通の関数」としてインタプリタに登録する。
    ///
    /// ⚠ `Stmt::MetaFnDef` のまま `exec` へ渡すと「展開されていない」エラーになるので、
    /// ここで [`Stmt::FnDef`] へ写す。本体はそのまま（`code:` / `quote` は VM が扱う）。
    fn define_metafn(&mut self, def: &Stmt) -> Result<(), String> {
        let Stmt::MetaFnDef { name, params, return_type, body, is_placing } = def else {
            return Err("MetaError: internal — define_metafn on a non-metafunction".to_string());
        };
        self.metafns.insert(name.clone(), *is_placing);
        // ⚠⚠ 配置メタ関数の入口で「展開器から呼ばれたか」を確かめる（§1.1・タスク 3-8）。
        //   本体の先頭に `__enter_placing("名前")` を差し込む。展開器以外から呼ばれると
        //   `compile_error` で止まる（[`PLACING_ARMED`] の doc）。
        //   ⚠ 採番は本体と同じカウンタから（§0.3）。
        let mut body = body.clone();
        if *is_placing {
            let src = format!("__enter_placing(\"{name}\")\n");
            let tokens = crate::lexer::Lexer::new(&src, "<meta-prologue>").tokenize();
            let mut parser = crate::parser::Parser::new(tokens, None);
            parser.set_node_counter(std::rc::Rc::clone(&self.node_counter));
            let mut prologue = parser
                .parse_program()
                .map_err(|e| format!("MetaError: internal — the placing prologue does not parse: {e}"))?;
            prologue.append(&mut body);
            body = prologue;
        }
        // ⚠ メタ関数の写しは展開専用の内部表現。元のソース範囲は持たない。
        let as_fn = Stmt::FnDef { src: None,
            name: name.clone(),
            template_params: Vec::new(),
            params: params.clone(),
            body,
            decorators: Vec::new(),
            return_type: return_type.clone(),
            is_static: false,
            is_class_method: false,
            is_abstract: false,
            access: crate::ast::Accessibility::Public,
        };
        self.interp.exec(&as_fn)?;
        if *is_placing {
            if let Some(Value::Function(f)) = self.interp.vm_load_name(name) {
                self.placing_fns.push(f);
            }
        }
        Ok(())
    }

    /// 展開用インタプリタで起きた失敗を、利用者に読める診断にする（タスク 3-0 / 3-2 / 3-8）。
    ///
    /// ⚠⚠ インタプリタは例外を**番兵文字列**で返し、実体は自分の中に置く。
    /// そのまま出すと `MetaError: while expanding 'go':  __raise__` という
    /// 何も分からない診断になる（実測）。⇒ 本体の入口（`src/main.rs`）と
    /// 同じように取り出して整形する。`RecursionError` の traceback もこれで出る。
    /// ⚠ 本体だけでなく**実引数と装飾子式**もここを通す（タスク 3-8）。以前は実引数の中の
    /// `compile_error` が番兵のまま漏れ、装飾子式の失敗は握り潰されていた（どちらも実測）。
    fn describe_failure(&mut self, e: String, stage: Stage<'_>) -> String {
        if e == crate::interpreter::RAISE_SENTINEL {
            let detail = self
                .interp
                .take_current_exception()
                .map(|r| Interpreter::format_error_report(&r))
                .unwrap_or_else(|| "(no details available)".to_string());
            // ⚠ `compile_error` は**利用者が意図して止めた**印。最優先で、文面だけ出す。
            if let Some(msg) = Self::compile_error_message(&detail) {
                return format!("MetaError: {msg} ({})", stage.raised_by());
            }
            if let Some(hint) = self.runtime_name_hint(&detail) {
                return format!("MetaError: {}: {hint}", stage.during());
            }
            return format!("MetaError: {}:{}{detail}", stage.during(), '\n');
        }
        if let Some(hint) = self.runtime_name_hint(&e) {
            return format!("MetaError: {}: {hint}", stage.during());
        }
        // ⚠ 下の層が `MetaError:` を付けていたら重ねない（二重の前置きは 3-3 で踏んだ）。
        let body = e.strip_prefix("MetaError: ").unwrap_or(&e);
        format!("MetaError: {}: {body}", stage.during())
    }

    /// 呼べる値を呼んで `Code` を得る（タスク 4-8）。`name` は診断用の呼び名。
    ///
    /// ⚠ 名前で引けない装飾子（`!repeat(3)` のファクトリが返したクロージャ・
    /// `const` に束縛したメタ関数）も**ここを通る**。診断の整形を 1 か所にするため。
    fn call_for_code(
        &mut self,
        callee: Value,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Rc<Vec<CodeLine>>, String> {
        let evaled: Vec<(Option<String>, Value, bool)> =
            args.into_iter().map(|v| (None, v, false)).collect();
        self.last_expanded = Some(name.to_string());
        // ⚠ 配置メタ関数を呼ぶときだけ旗を立てる（[`PLACING_ARMED`] の doc）。
        //   ⚠⚠ **呼んだ後は必ず下ろす。** 入口の手前で失敗すると（引数の数違いなど）旗が
        //   立ったまま残り、次に式の中から呼ばれた配置メタ関数が素通りしてしまう。
        let armed = matches!(&callee, Value::Function(f)
            if self.placing_fns.iter().any(|p| std::rc::Rc::ptr_eq(p, f)));
        if armed {
            self.interp
                .meta_assign_main_global(PLACING_ARMED, Value::Bool(true))
                .map_err(|e| format!("MetaError: internal — {e}"))?;
        }
        let result = self.interp.call_value_evaled(callee, evaled, name, None, 0);
        if armed {
            self.interp
                .meta_assign_main_global(PLACING_ARMED, Value::Bool(false))
                .map_err(|e| format!("MetaError: internal — {e}"))?;
        }
        let out = match result {
            Ok(v) => v,
            Err(e) => return Err(self.describe_failure(e, Stage::Body(name))),
        };
        match out {
            Value::Code(lines) => Ok(lines),
            // ⚠ `quote` に到達せず抜けた形（設計書 §1.1）。診断の本番は 3-5。
            Value::None => Err(format!(
                "MetaError: metafunction '{name}' returned without placing any `Code` \
                 (use `quote` — an empty `code:` block places nothing)"
            )),
            other => Err(format!(
                "MetaError: metafunction '{name}' must produce `Code`, got '{}'",
                self.interp.type_name(&other)
            )),
        }
    }
}

/// 実行時の `NameError` が**展開で消えたメタ関数**を指していたら、理由を書き添える（タスク 3-8）。
///
/// ⚠⚠ メタ関数は展開後の AST から消える。⇒ 通常コードから呼ぶ（§1.1 の表で「不可」）と、
/// 実行時に `NameError: 'greet' is not defined` とだけ出て「定義したのに無いと言われた」と
/// 読めてしまう（実測）。展開器が置き換えるのは**最上位とクラス本体の文位置の呼び出しだけ**
/// なので、関数本体の中から呼んだ場合もここへ来る。
///
/// ⚠ **静的には弾かない。** 同じ名前の局所変数がメタ関数を隠していることがあり、
/// その判定には型検査と同じスコープの追跡が要る。実行時に「本当に引けなかった」ときだけ
/// 書き添えるので誤検知は起きない。
pub fn explain_removed_metafn(
    msg: String,
    metafns: &std::collections::HashMap<String, bool>,
) -> String {
    for (name, placing) in metafns {
        if !msg.contains(&format!("NameError: '{name}' is not defined")) {
            continue;
        }
        let why = if *placing {
            format!(
                "note: '{name}' is a placing metafunction (`exprconst !fn`). The expander replaces a \
                 call to it only when the call is a statement at the top level or directly in a class \
                 body; a call inside a function body or a block, or one used as a value, is not \
                 expanded, and metafunctions no longer exist at run time"
            )
        } else {
            format!(
                "note: '{name}' is a pure metafunction (`exprconst fn`). It exists only while the \
                 compile-time expander runs, so ordinary code cannot call it; call it from another \
                 metafunction and `quote` the `Code` it returns"
            )
        };
        return format!("{msg}\n{why}");
    }
    msg
}

/// [`expand_program`] と同じだが、**消したメタ関数の一覧**も返す（タスク 3-8）。
///
/// ⚠ 実行時の `NameError` に理由を書き添えるため（[`explain_removed_metafn`]）。
/// ⚠ エディタ用 wasm は展開しない（5-0）ので、そちらのクレートでは使われない。
#[allow(dead_code)]
pub(crate) fn expand_program_with_metafns(
    stmts: Vec<Stmt>,
    node_counter: std::rc::Rc<std::cell::Cell<u32>>,
    known_traits: std::collections::HashMap<String, crate::parser::TraitInfo>,
) -> Result<(Vec<Stmt>, std::collections::HashMap<String, bool>), String> {
    let mut ex = Expander::new(node_counter, known_traits);
    let out = expand_with(&mut ex, stmts)?;
    Ok((out, std::mem::take(&mut ex.metafns)))
}

/// 文の列を展開する（設計書 §1.7）。入力を壊さず、新しい列を返す。
///
/// ⚠⚠ **`resolve_and_annotate` より前に走らせること。** 展開で生えた宣言を
/// リゾルバ・型検査・VM が見られるようにするため。配線は 2-2。
pub fn expand_program(
    stmts: Vec<Stmt>,
    node_counter: std::rc::Rc<std::cell::Cell<u32>>,
    known_traits: std::collections::HashMap<String, crate::parser::TraitInfo>,
) -> Result<Vec<Stmt>, String> {
    let mut ex = Expander::new(node_counter, known_traits);
    expand_with(&mut ex, stmts)
}

/// プログラムがメタ関数の構文（`exprconst` / `code:` / `!装飾子` / `^` など）を含むか
/// （入れ子・`import` 先も見る・タスク 2-11 の `ordinary::mentions_meta`）。
///
/// ⚠ エディタは含まないプログラムでは展開器を起こさない（D5・タスク 5-0）。含まないなら
///   展開の結果は単相化（[`monomorphize`]）と同じで、展開用インタプリタを作る分だけ無駄になる。
#[allow(dead_code)] // CLI は常に展開器を通す（エディタ専用の入口）
pub fn mentions_meta(stmts: &[Stmt]) -> bool {
    ordinary::mentions_meta(stmts)
}

/// **ブロックを 1 つずつ**展開する（REPL・D36・タスク 2-16）。
///
/// 展開器の状態（メタ関数・テンプレート・作った具体化・展開用インタプリタ）をブロックをまたいで
/// 持ち続ける。⚠⚠ 実行時の具体化を廃止した（D36）ので、REPL もブロックを受け取ったときに
/// 展開して具体化を置き、実行時は具体化済みの AST を読む（前のブロックで宣言したテンプレートも使える）。
pub struct Session {
    ex: Expander,
    started: bool,
}

impl Session {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            ex: Expander::new(
                std::rc::Rc::new(std::cell::Cell::new(0)),
                std::collections::HashMap::new(),
            ),
            started: false,
        }
    }

    /// 1 ブロックを展開する。`node_counter` / `known_traits` はそのブロックをパースしたパーサのもの。
    ///
    /// ⚠ 置いたコードの node-id はブロックのパーサと同じカウンタから振る（§0.3）。
    #[allow(dead_code)] // エディタ用 wasm は REPL を持たない
    pub fn expand_block(
        &mut self,
        stmts: Vec<Stmt>,
        node_counter: std::rc::Rc<std::cell::Cell<u32>>,
        known_traits: std::collections::HashMap<String, crate::parser::TraitInfo>,
    ) -> Result<Vec<Stmt>, String> {
        let ex = &mut self.ex;
        ex.node_counter = std::rc::Rc::clone(&node_counter);
        ex.mono.set_counter(node_counter);
        ex.known_traits.extend(known_traits);
        ex.steps = 0;
        if !self.started {
            ex.load_prelude()?;
            self.started = true;
        }
        // ⚠ 前のブロックでメタ関数・テンプレートに出会っていれば、この先も有効のまま。
        ex.has_metafns = ex.has_metafns || ordinary::mentions_meta(&stmts);
        ex.mono.enabled = ex.mono.enabled || monomorph::declares_template(&stmts);
        expand_body(ex, stmts)
    }
}

fn expand_with(ex: &mut Expander, stmts: Vec<Stmt>) -> Result<Vec<Stmt>, String> {
    ex.load_prelude()?;
    // ⚠ 展開時型推論（タスク 4-4）の前提として、展開済みの最上位の文を共有する。
    //   ⚠ メタ関数が 1 つも無いプログラムでは推論が起きえないので**やらない**
    //     （全プログラムで AST を複製するのは割に合わない）。
    // ⚠ 最上位だけでなく入れ子・`import` 先まで見る（タスク 2-11。`ordinary::mentions_meta`）。
    ex.has_metafns = ordinary::mentions_meta(&stmts);
    ex.mono.enabled = monomorph::declares_template(&stmts);
    expand_body(ex, stmts)
}

/// 前口上と旗の準備が済んだ展開器で、最上位の文の列を展開する（プログラム全体・REPL のブロック共通）。
fn expand_body(ex: &mut Expander, stmts: Vec<Stmt>) -> Result<Vec<Stmt>, String> {
    prescan_runtime_names(ex, &stmts);
    let out = expand_stmts(ex, stmts, Context::TopLevel, std::rc::Rc::new(Vec::new()))?;
    // ⚠⚠ 展開後の通常コードに残ったメタ関数への言及を弾く（§1.1・タスク 2-11）。
    //   関数の中の呼び出し・値としての使用・入れ子の定義・通常コードの `^`。
    if ex.has_metafns {
        ordinary::check(&out, &ordinary::MetaNames { plain: &ex.metafns, modules: &ex.modules })?;
    }
    Ok(out)
}

/// 診断専用の先行走査（設計書 §1.8 / タスク 3-0）。展開の可否や結果には影響しない。
///
/// ⚠ モジュールの枠でも同じものを作る（タスク 2-12）。枠ごとに入れ替える。
fn prescan_runtime_names(ex: &mut Expander, stmts: &[Stmt]) {
    for st in stmts {
        // ⚠ `const` は展開時に読めるはず（設計書 §1.7）なので**実行時の名前ではない**。
        if matches!(st, Stmt::Const(..)) {
            continue;
        }
        crate::decl_names::each_declared_name(st, &mut |n, _, _| {
            ex.runtime_names.insert(n.to_string());
        });
    }
}

/// 1 つの文の列を前から順に展開する。
fn expand_stmts(
    ex: &mut Expander,
    stmts: Vec<Stmt>,
    ctx: Context,
    trail: std::rc::Rc<Vec<Frame>>,
) -> Result<Vec<Stmt>, String> {
    // ⚠ 置換で文が増えるので、**残りを持ち回る**形にする（`for` では書けない）。
    // ⚠ 文ごとに**出所**（どのメタ関数が置いたか）を連れて回る（タスク 3-3）。
    //   `Rc` なので、同じ出所を持つ文が何本増えても複製は起きない。
    let mut pending: std::collections::VecDeque<(Stmt, std::rc::Rc<Vec<Frame>>)> =
        stmts.into_iter().map(|s| (s, std::rc::Rc::clone(&trail))).collect();
    let mut out: Vec<Stmt> = Vec::new();

    while let Some((stmt, here)) = pending.pop_front() {
        ex.steps += 1;
        // ⚠ メタ関数が実行してよい命令数は**文ごと**に張り直す（D5・`META_OPS_BUDGET`）。
        ex.interp.meta_reset_budget();
        if ex.steps > STEP_BUDGET {
            let culprit = match &ex.last_expanded {
                Some(n) => format!("the last metafunction expanded was '{n}'"),
                None => "no metafunction was expanded, so the program itself is enormous".to_string(),
            };
            return Err(format!(
                "MetaError: expansion did not finish within {STEP_BUDGET} steps — \
                 a metafunction is probably placing a call to itself ({culprit})"
            ));
        }

        // ⚠⚠ **分岐の前に登録する。** `^` はここから引く（タスク 4-0）。
        //   前から順に入れるので、メタ関数は自分より前の宣言しか見られない（§1.7）。
        //   ⚠ **自分自身は含めない**ようにここで入れる —— 登録してから展開すると、
        //     その宣言を装飾している最中の装飾子が「展開前の自分」を見てしまう。
        // ⚠⚠ **メタ関数の無いプログラムでは登録しない。** `^` を評価する場面が無いのに、
        //   以前は全プログラムで最上位の宣言を**全部複製して**いた（4-0 で入った無駄・3-9）。
        let out_len_before = out.len();

        // ⚠⚠ **テンプレートの具体化はこの文より前に置く**（インライン単相化・タスク 2-8）。
        //   `^Box[int]` や `Box[int](..)` を含む配置呼び出しは、展開の途中で具体化を参照する
        //   ので、文を処理する**前**に作っておく。置いたコードの中の具体化も、その文が
        //   ここへ来たときに拾われる（反復は要らない）。
        if ctx == Context::TopLevel {
            // ⚠ 具体化が終わらないとき（再帰的な具体化）は展開時エラー（タスク 2-9）。
            let before_inst = out.len();
            monomorph::instantiate_sites(&mut ex.mono, &stmt, &mut out)
                .map_err(|e| e + &render_trail(&here))?;
            // 置いた具体化も `^Box[int]` で引けるようにする。
            if ex.has_metafns {
                for i in before_inst..out.len() {
                    register_decl(ex, &out[i]);
                }
            }
        }

        if ex.has_metafns {
            register_decl(ex, &stmt);
        }

        match stmt {
            // 定義は登録して AST から消す。
            Stmt::MetaFnDef { .. } => ex.define_metafn(&stmt)?,

            // `const` は**展開時にも値が確定している**（設計書 §1.7）。
            // ⇒ 展開器のインタプリタにも束縛して、メタ関数から読めるようにする。
            // ⚠ **AST からは消さない。** 実行時にも要る。
            // ⚠ 初期化子が実行時の値に依存していると評価できない（`const x = f()` など）。
            //   そのときは**黙って諦める**——その `const` が展開時に使えないだけで、
            //   プログラムとしては正しい。⇒ 読まれたときに 3-0 の診断が出るよう名前を控える。
            Stmt::Const(ref name, _, ref init, _) => {
                // ⚠ `exec` には渡せない。最上位の文は VM 経由で走る決まりなので、
                //   `Stmt::Const` を `exec` へ渡すと `VmForceError` になる（実測）。
                //   ⇒ 初期化子だけ評価して、VM と同じ経路で束縛する。
                // ⚠⚠ **メタ関数（やそれが返したクロージャ）に束縛した `const` は展開時専用**
                //   （タスク 4-8）。メタ関数は展開後の AST から消えるので、`const` を残すと
                //   実行時に**もう存在しない名前を読んで `NameError`** になる（実測）。
                //   ⇒ メタ関数の定義と同じく、実行時の AST から消す。
                //   ⚠ 展開時に関数値になるのはメタ関数由来だけ（D1: 普通の `fn` は展開用
                //     インタプリタに定義されない）なので、値の型で判定してよい。
                //   ⚠ `Code` に束縛した `const` は**消さない**。消すと実行時の失敗が `const` の
                //     名前の `NameError` になり、原因（純粋メタ関数を通常コードで呼んだ）が
                //     見えなくなる。残せば**メタ関数の名前で**落ち、3-8 の書き添えが付く。
                //     ⚠ 型検査（1-5）がこれを弾くのは展開しないエディタだけ。CLI では型検査より
                //     前にメタ関数が消えている（4-8 の時点の「型検査に弾かせる」は誤り・3-8 で訂正）。
                let mut expansion_only = false;
                match ex.interp.eval(init) {
                    Ok(v) => {
                        expansion_only = matches!(v, Value::Function(_) | Value::OverloadedFn(_));
                        // ⚠ **今の枠に**束縛する（タスク 2-12）。モジュールの枠の中では、呼び出し側に
                        //   同じ名前の `const` があっても隠して束縛する（各モジュールのメタ関数は
                        //   自分のモジュールの値を読む）。
                        ex.interp.meta_bind(name, v);
                    }
                    Err(e) => {
                        // ⚠ 例外で失敗したときは実体がインタプリタに残っている。捨てておかないと、
                        //   後の無関係な失敗の報告にこの例外が混ざる。
                        if e == crate::interpreter::RAISE_SENTINEL {
                            let _ = ex.interp.take_current_exception();
                        }
                        ex.runtime_names.insert(name.clone());
                    }
                }
                if !expansion_only {
                    out.push(stmt);
                }
            }

            // 配置メタ関数の呼び出し文 → `quote` した中身で置き換える。
            Stmt::Expr(Expr::Call { ref func, ref args, ref span, .. })
                if placing_call_name(ex, func).is_some() =>
            {
                let name = placing_call_name(ex, func).expect("checked by the guard");
                // ⚠ 出所を積んでから呼ぶ。失敗の報告にこの列を添える（タスク 3-3）。
                let frames = if here.len() >= MAX_TRAIL {
                    // ⚠ 上限に達したら**伸ばさない**（そのまま使い回す）。複製が二次になるのを防ぐ。
                    std::rc::Rc::clone(&here)
                } else {
                    let mut frames = (*here).clone();
                    frames.push(Frame { name: name.clone(), span: span.clone() });
                    std::rc::Rc::new(frames)
                };

                let vals = eval_args(ex, args, &name).map_err(|e| e + &render_trail(&frames))?;
                // ⚠ 呼び先は**式として評価する**。`greet` なら名前を引き、`m.greet`（タスク 2-12）
                //   なら名前空間から引く。同じ経路で両方が引ける。
                let callee = match ex.interp.eval(func) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(ex.describe_failure(e, Stage::Body(&name)) + &render_trail(&frames))
                    }
                };
                let code = ex
                    .call_for_code(callee, &name, vals)
                    .map_err(|e| e + &render_trail(&frames))?;
                let placed = ex.place(&code, ctx).map_err(|e| e + &render_trail(&frames))?;
                // ⚠ **置いたものを先に歩く**。置いたコードがさらにメタ関数を呼ぶことはある。
                for s in placed.into_iter().rev() {
                    pending.push_front((s, std::rc::Rc::clone(&frames)));
                }
            }

            // `!装飾子` 付きの宣言 → 装飾子が返した中身で置き換える。
            Stmt::MetaDecorated { decorators, spans, target } => {
                // ⚠ 位置は `!` のもの（`Expr::Ident` は `Span` を持たないので AST 側に控えてある・3-9）。
                let placed = apply_decorators(ex, decorators, spans, *target, ctx, &here)?;
                for s in placed.into_iter().rev() {
                    pending.push_front((s, std::rc::Rc::clone(&here)));
                }
            }

            // `import` した `.ar` モジュール（タスク 2-12）。初めてなら展開し、結果を束縛する。
            // ⚠ メタ関数の構文を含まないモジュール（Python の翻訳など）は触らない。
            Stmt::Import { lang, module, source_module, alias, body, origin, bind }
                if ex.has_metafns && ordinary::mentions_meta(&body) =>
            {
                let m = modules::expand_module(ex, &lang, &module, body)?;
                // ⚠ 束縛名は AST の `bind`（CPython 準拠・`import a.b` は `a`）。束縛されるのが
                //   このモジュールそのもののときだけ、メタ関数の名前空間を束縛する
                //   （パッケージ `a` の束縛は、そのパッケージを読み込んだ文が担う）。
                if let Some(b) = bind.as_ref().filter(|b| b.module == module) {
                    modules::bind_import(ex, &b.name, &module, &m);
                }
                out.push(Stmt::Import { lang, module, source_module, alias, body: m.body.clone(), origin, bind });
            }
            Stmt::FromImport { lang, module, source_module, names, body, origin }
                if ex.has_metafns && ordinary::mentions_meta(&body) =>
            {
                let m = modules::expand_module(ex, &lang, &module, body)?;
                modules::bind_from_import(ex, &names, &m);
                // ⚠⚠ **メタ関数の名前は実行時の import から外す**（`const` の別名と同じ・4-8）。
                //   メタ関数は展開後のモジュールに無いので、残すと実行時に
                //   `ImportError: cannot import name 'greet'` になる（実測）。
                //   ⚠ 全部外れて空になっても文は残す —— モジュールの本体を走らせる入口として要る。
                let names: Vec<(String, Option<String>)> =
                    names.into_iter().filter(|(orig, _)| !m.metafns.contains_key(orig)).collect();
                out.push(Stmt::FromImport { lang, module, source_module, names, body: m.body.clone(), origin });
            }

            // クラス／トレイト本体もその場で展開する（メンバーの装飾子がここで消える）。
            Stmt::ClassDef { src,
                name,
                template_params,
                bases,
                base_args,
                body,
                decorators,
            } => {
                // ⚠⚠ 装飾子があったクラスは**パース時に仕上げを保留してある**
                //    （タスク 2-4。`parse_class_def` を参照）。展開でメンバーが確定した
                //    ここで自動 `__init__` 生成と trait デフォルト実装の注入を行う。
                //    保留していなければ二重に走らせない（顔ぶれは変わっていない）。
                let deferred = body.iter().any(|s| matches!(s, Stmt::MetaDecorated { .. }));
                let mut body = expand_stmts(ex, body, Context::TypeBody, std::rc::Rc::clone(&here))?;
                if deferred {
                    let bases_with_args: Vec<(String, Vec<String>)> = bases
                        .iter()
                        .cloned()
                        .zip(base_args.iter().cloned())
                        .collect();
                    crate::parser::classes::finalize_class_body(
                        &ex.known_traits,
                        &name,
                        &bases_with_args,
                        &mut body,
                    )
                    .map_err(|e| format!("MetaError: while finishing class '{name}': {e}"))?;
                }
                out.push(Stmt::ClassDef { src: src.clone(),
                    name,
                    template_params,
                    bases,
                    base_args,
                    body,
                    decorators,
                });
            }
            Stmt::TraitDef { src, name, template_params, body } => {
                let body =
                    expand_stmts(ex, body, Context::TypeBody, std::rc::Rc::clone(&here))?;
                out.push(Stmt::TraitDef { src: src.clone(), name, template_params, body });
            }

            // それ以外はそのまま。
            // ⚠⚠ **関数本体・ブロックの中へは降りない**（§1.1・2026-09-25 確定）。
            //   メタ関数でない関数の中ではメタ関数を呼べない。残っていたら展開の最後に
            //   `ordinary::check` が弾く（タスク 2-11）。
            other => out.push(other),
        }

        // テンプレートとクラスの宣言を覚える。テンプレートの宣言より前に書かれていた具体化は
        // ここで作り、宣言の後ろに置く（タスク 2-14）。
        if ctx == Context::TopLevel {
            let before_pending = out.len();
            monomorph::note_decls(&mut ex.mono, &mut out, out_len_before)
                .map_err(|e| e + &render_trail(&here))?;
            if ex.has_metafns {
                for i in before_pending..out.len() {
                    register_decl(ex, &out[i]);
                }
            }
        }

        // ⚠ 展開時型推論の前提（タスク 4-4）。**最上位で出した文だけ**を足す。
        //   クラス本体の中の文はそのクラスの文に含まれて届くので二重に足さない。
        if ex.has_metafns && ctx == Context::TopLevel && trail.is_empty() {
            let handle = ex.interp.meta_prefix_handle();
            let mut prefix = handle.borrow_mut();
            for st in &out[out_len_before..] {
                prefix.push(st.clone());
            }
        }
    }
    Ok(out)
}

/// 展開時に見える宣言として登録する（タスク 4-0）。
///
/// ⚠⚠ **メタ情報の対象にならない種類も入れる**（タスク 3-9）。`^` が引けなかったときに
/// 「まだ宣言されていない」と「宣言はあるがメタ情報を持たない種類」を言い分けるため
/// （`Interpreter::meta_lookup_error`）。以前は対象の種類だけを入れていたので、
/// `^メタ関数名` が**「前に何も宣言されていない」と誤報された**（実測）。
/// 値を作るかどうかは `MetaValue::kind_of` が引くときに決める。
/// ⚠ `!装飾子` に包まれた宣言は**中身**を登録する（包みには名前が無い）。
fn register_decl(ex: &mut Expander, stmt: &Stmt) {
    use crate::interpreter::value::MetaDecl;
    let inner = match stmt {
        Stmt::MetaDecorated { target, .. } => &**target,
        other => other,
    };
    // ⚠ 名前を作らない文（式文・制御構文…）は何もしない。**複製の前に**確かめる。
    let mut names: Vec<String> = Vec::new();
    crate::decl_names::each_declared_name(inner, &mut |name, _, _| names.push(name.to_string()));
    if names.is_empty() {
        return;
    }
    let entry = if crate::interpreter::value::MetaValue::kind_of(inner).is_some() {
        MetaDecl::Decl(std::rc::Rc::new(inner.clone()))
    } else {
        // ⚠⚠ メタ情報を持たない種類は**複製しない**（`import` の本体はモジュール丸ごと）。
        MetaDecl::Opaque(non_meta_kind(inner))
    };
    for name in names {
        ex.interp.add_meta_decl(name, entry.clone());
    }
}

/// `^` を当てられない宣言の呼び名（タスク 3-9）。
///
/// ⚠ **何を取り違えたのかまで言う。** 「使えません」だけだと、名前を間違えたのか
/// 対象の種類を間違えたのかが分からない。
/// ⚠ 末尾の `_` は呼び名を持たない残りを受ける（メタ情報を持つ種類はここへ来ない）。
///   ここは walker ではなく**文面を選ぶだけ**なので、網羅を強制する意味が無い。
fn non_meta_kind(decl: &Stmt) -> &'static str {
    match decl {
        Stmt::MetaFnDef { is_placing: true, .. } => "a placing metafunction",
        Stmt::MetaFnDef { is_placing: false, .. } => "a pure metafunction",
        Stmt::ProtocolDef { .. } => "a protocol",
        Stmt::NewTypeDef { .. } => "a `new_type`",
        Stmt::Import { .. } | Stmt::FromImport { .. } => "an imported name",
        Stmt::LetTuple { .. } => "a name bound by tuple unpacking",
        _ => "a declaration of a kind that has no meta information",
    }
}

/// 呼び出し式が「登録済みの**配置**メタ関数」ならその名前を返す。
///
/// ⚠ `m.greet()`（`import` したモジュールのメタ関数・タスク 2-12）も配置呼び出し。
///   返す名前は `"m.greet"`（診断用の呼び名）。
fn placing_call_name(ex: &Expander, func: &Expr) -> Option<String> {
    match func {
        Expr::Ident { name, .. } => match ex.metafns.get(name) {
            Some(true) => Some(name.clone()),
            _ => None,
        },
        Expr::Attr { object, attr, .. } => match &**object {
            Expr::Ident { name: m, .. } => match ex.modules.get(m).and_then(|f| f.get(attr)) {
                Some(true) => Some(format!("{m}.{attr}")),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// 宣言そのものからメタ情報の値を作る（タスク 4-0）。
///
/// ⚠ 装飾子の第一引数に使う。`^名前` と違って**表を引かない**（対象はもう手元にある）。
fn meta_value_for(ex: &Expander, decl: &Stmt) -> Option<Value> {
    // ⚠ `decl_names` ではなく `name_of`。フィールドは `decl_names` が報告しない
    //   （スコープの名前ではないため）ので、混ぜると名前が空になる。
    let name = crate::interpreter::value::MetaValue::name_of(decl)?;
    // ⚠ 装飾される宣言は**まだ展開済みの文に入っていない**（これから置き換える）。
    //   ⇒ 推論の前提に足して推論させる（タスク 4-4）。
    ex.interp.make_meta_value(&name, std::rc::Rc::new(decl.clone()), true)
}

/// 実引数を展開時に評価する。
///
/// ⚠ 展開時に確定しないもの（実行時の変数など）はここで失敗する。
/// 「何が確定するか」の診断は 3-0（§1.8）。
fn eval_args(
    ex: &mut Expander,
    args: &[crate::ast::CallArg],
    callee: &str,
) -> Result<Vec<Value>, String> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        match a {
            crate::ast::CallArg::Positional(e) => match ex.interp.eval(e) {
                Ok(v) => out.push(v),
                Err(err) => return Err(ex.describe_failure(err, Stage::Args(callee))),
            },
            _ => {
                return Err(
                    "MetaError: metafunction calls take positional arguments only (for now)"
                        .to_string(),
                )
            }
        }
    }
    Ok(out)
}

/// `!装飾子` を適用して、置き換える文の列を返す（設計書 §1.6）。
///
/// ⚠ **内側から**適用する（§1.6）。宣言に一番近い装飾子＝リストの**末尾**が先。
/// ⚠ 対象は**第一引数**として渡る。渡す形は `ast_value` の AST 値（4-0 で `meta_*` に寄せる）。
fn apply_decorators(
    ex: &mut Expander,
    decorators: Vec<Expr>,
    spans: Vec<Span>,
    target: Stmt,
    ctx: Context,
    trail: &std::rc::Rc<Vec<Frame>>,
) -> Result<Vec<Stmt>, String> {
    let mut current = vec![target];
    // ⚠ 位置が欠けていても装飾子は落とさない（`zip` だと短い方で黙って切れる）。
    let spans = spans.into_iter().map(Some).chain(std::iter::repeat(None));
    for (deco, span) in decorators.into_iter().zip(spans).collect::<Vec<_>>().into_iter().rev() {
        // ⚠⚠ 装飾子は**式**（設計書 §1.6「装飾子にはクロージャも書ける」・タスク 4-8）。
        //   - `!name`          … 名前で登録されたメタ関数
        //   - `!alias`         … `const` に束縛したメタ関数（3-6 で展開時に評価される）
        //   - `!repeat(3)`     … **装飾子ファクトリ**。`repeat(3)` を評価して返ってきた
        //                        クロージャを対象に適用する（Python の `@deco(args)` と同じ形）
        //   ⚠ ファクトリが返したクロージャの中の `quote` は**そのクロージャを抜ける**。
        //     ファクトリは既に返っているので、それ以外の意味は取りようがない。
        //     §1.3 の未決（外側のメタ関数の実行中に呼ばれた補助クロージャの `quote`）とは別の場合。
        let name = decorator_label(&deco);
        // ⚠⚠ この装飾子の**出所**を先に積む（タスク 3-9）。以降の失敗はすべてこれを添える。
        //   以前は位置を持っておらず、装飾子の失敗が**どの宣言のものか分からなかった**（実測）。
        let mut frames = (**trail).clone();
        if frames.len() < MAX_TRAIL {
            frames.push(Frame { name: format!("!{name}"), span: span.unwrap_or_else(Span::unknown) });
        }
        let callee = match &deco {
            Expr::Ident { name: n, .. } if ex.metafns.contains_key(n) => {
                ex.interp.vm_load_name(n)
            }
            // ⚠ 名前が引けないのは「前に宣言されていない」（§1.7）。下で言う。
            Expr::Ident { .. } => ex.interp.eval(&deco).ok(),
            // ⚠⚠ 式（`!repeat(3)` など）の評価の失敗は**握り潰さない**（タスク 3-8）。
            //   以前は `.ok()` で捨てていたので、ファクトリの実引数の `1 // 0` が
            //   「前に宣言されていない」という無関係な診断になった（実測）。
            _ => match ex.interp.eval(&deco) {
                Ok(v) => Some(v),
                Err(e) => {
                    return Err(
                        ex.describe_failure(e, Stage::Decorator(&name)) + &render_trail(&frames)
                    )
                }
            },
        };
        let Some(callee) = callee else {
            return Err(format!(
                "MetaError: decorator '!{name}' is not a metafunction declared before this point"
            ) + &render_trail(&frames));
        };
        if !is_callable(&callee) {
            return Err(format!(
                "MetaError: decorator '!{name}' did not produce something callable \
                 (got '{}'); a decorator must be a metafunction, or an expression that \
                 returns one",
                ex.interp.type_name(&callee)
            ) + &render_trail(&frames));
        }
        if current.len() != 1 {
            return Err(format!(
                "MetaError: decorator '!{name}' cannot be applied — the inner decorator produced \
                 {} statements, and a decorator takes exactly one declaration",
                current.len()
            ) + &render_trail(&frames));
        }
        // ⚠ 装飾子には**メタ情報**を渡す（タスク 4-0）。`parse_ar` の `Namespace` 木ではない。
        //   名前が無い宣言（`pass` など）は対象外なので、そこは弾かれている。
        let arg = meta_value_for(ex, &current[0]).ok_or_else(|| {
            format!(
                "MetaError: decorator '!{name}' cannot be applied to this declaration \
                 (meta information exists for classes, traits, enums, functions, fields \
                 and variable bindings)"
            ) + &render_trail(&frames)
        })?;
        let code = ex
            .call_for_code(callee, &name, vec![arg])
            .map_err(|e| e + &render_trail(&frames))?;
        current = ex.place(&code, ctx).map_err(|e| e + &render_trail(&frames))?;
    }
    Ok(current)
}

/// 装飾子式の呼び名（診断用・タスク 4-8）。`!repeat(3)` なら `repeat(...)`。
fn decorator_label(deco: &Expr) -> String {
    match deco {
        Expr::Ident { name, .. } => name.clone(),
        Expr::Call { func, .. } => format!("{}(...)", decorator_label(func)),
        Expr::Attr { object, attr, .. } => format!("{}.{attr}", decorator_label(object)),
        _ => "<expression>".to_string(),
    }
}

/// 呼べる値か（装飾子として適用できるか）。
///
/// ⚠ メタ関数も、ファクトリが返したクロージャも、展開用インタプリタの中では
/// ただの関数値（`Value::Function`）。
fn is_callable(v: &Value) -> bool {
    matches!(v, Value::Function(_) | Value::OverloadedFn(_))
}

#[cfg(test)]
mod tests;
