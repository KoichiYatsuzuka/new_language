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
///
/// ⚠ **型と列はまだ扱えない**（設計書 §1.4 の「型」「列」）。型値の合成は 4-2、
/// 型文字列の検証は 4-3、引数シグネチャの列は 4-1。⇒ ここでは明示エラーにして、
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
                    pieces.push(CodePiece::Token(splice_token(v, &line.span)?));
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
fn splice_token(v: &Value, span: &Span) -> Result<Spanned, String> {
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
                 (str / int / float / bool are supported; splicing types and sequences needs \
                 the type-composition API, task 4-2, whose notation is not designed yet)",
                crate::interpreter::ops::typecheck::runtime_type_name(other)
            ))
        }
    };
    Ok(Spanned { token, span: span.clone() })
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
/// 型値の合成（4-2・**記法は未設計**）とその検証（4-3 の残り半分）。
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
        // ⚠ モジュールの展開の枠にも前口上を置く（枠ごとに大域が分かれている・名前空間の分離）。
        //   旗（[`PLACING_ARMED`]）は置かない —— 旗は 1 つでなければならない。
        self.interp.meta_set_prelude(&[ABORT_CLASS, "compile_error", "gensym", "__enter_placing"]);
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

fn expand_with(ex: &mut Expander, stmts: Vec<Stmt>) -> Result<Vec<Stmt>, String> {
    ex.load_prelude()?;
    // ⚠ 展開時型推論（タスク 4-4）の前提として、展開済みの最上位の文を共有する。
    //   ⚠ メタ関数が 1 つも無いプログラムでは推論が起きえないので**やらない**
    //     （全プログラムで AST を複製するのは割に合わない）。
    // ⚠ 最上位だけでなく入れ子・`import` 先まで見る（タスク 2-11。`ordinary::mentions_meta`）。
    ex.has_metafns = ordinary::mentions_meta(&stmts);
    ex.mono.enabled = monomorph::declares_template(&stmts);
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
            Stmt::Const(ref name, _, ref init) => {
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
            Stmt::Import { lang, module, source_module, alias, body }
                if ex.has_metafns && ordinary::mentions_meta(&body) =>
            {
                let m = modules::expand_module(ex, &lang, &module, body)?;
                let bind = alias.clone().or_else(|| module.last().cloned()).unwrap_or_default();
                modules::bind_import(ex, &bind, &module, &m);
                out.push(Stmt::Import { lang, module, source_module, alias, body: m.body.clone() });
            }
            Stmt::FromImport { lang, module, source_module, names, body }
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
                out.push(Stmt::FromImport { lang, module, source_module, names, body: m.body.clone() });
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
mod tests {
    use super::*;

    /// ソースを展開して、残った文の種別名を並べたものを返す。
    fn expand_kinds(src: &str) -> Result<Vec<&'static str>, String> {
        let out = expand(src)?;
        Ok(out.iter().map(crate::interpreter::tw_stats::stmt_kind_of).collect())
    }

    /// パースして、文の列と**そのパーサが使った node-id カウンタ**を返す。
    ///
    /// ⚠ カウンタを展開器へ渡すのが要点（設計書 §0.3 / タスク 2-1）。
    type Parsed = (
        Vec<Stmt>,
        std::rc::Rc<std::cell::Cell<u32>>,
        std::collections::HashMap<String, crate::parser::TraitInfo>,
    );

    fn parse(src: &str) -> Result<Parsed, String> {
        let tokens = crate::lexer::Lexer::new(src, "").tokenize();
        let mut parser = crate::parser::Parser::new(tokens, None);
        let stmts = parser.parse_program()?;
        Ok((stmts, parser.node_counter(), parser.known_traits()))
    }

    fn expand(src: &str) -> Result<Vec<Stmt>, String> {
        let (stmts, counter, traits) = parse(src)?;
        expand_program(stmts, counter, traits)
    }

    /// メタ関数の定義は**登録されて AST から消える**（設計書 §1.7）。
    /// ⚠ 消えないと、展開後の AST に実行できない文が残る。
    #[test]
    fn metafn_definitions_are_removed_from_the_ast() {
        let kinds = expand_kinds(
            "exprconst !fn place_it() -> None:\n    quote code:\n\nprint(1)\n",
        )
        .expect("expand");
        assert_eq!(kinds, vec!["Expr"], "`print(1)` だけが残る");
    }

    /// 配置メタ関数の呼び出しが、`quote` した `Code` の中身で置き換わる。
    #[test]
    fn a_placing_metafn_call_is_replaced_by_its_code() {
        let out = expand(
            "exprconst !fn place_it() -> None:\n    quote code:\n        let placed = 1\n\nplace_it()\n",
        )
        .expect("expand");
        assert_eq!(out.len(), 1);
        assert!(
            matches!(&out[0], Stmt::Let(name, _, _) if name == "placed"),
            "置かれたのは `let placed = 1`: {:?}",
            crate::interpreter::tw_stats::stmt_kind_of(&out[0])
        );
    }

    /// 空の `code:` を `quote` すると**何も置かれない**（設計書 §1.2 の B-2）。
    #[test]
    fn quoting_an_empty_code_block_places_nothing() {
        let kinds = expand_kinds(
            "exprconst !fn place_nothing() -> None:\n    quote code:\n\nplace_nothing()\nprint(1)\n",
        )
        .expect("expand");
        assert_eq!(kinds, vec!["Expr"], "`print(1)` だけが残る");
    }

    /// スプライス `<! !>` が展開時の値で埋まる（設計書 §1.4）。
    /// ⚠ `str` は**識別子**になる（文字列リテラルではない）。
    #[test]
    fn a_splice_fills_in_an_identifier() {
        let out = expand(
            "exprconst !fn bind(n: str) -> None:\n    quote code:\n        let <! n !> = 1\n\nbind(\"chosen\")\n",
        )
        .expect("expand");
        assert_eq!(out.len(), 1);
        assert!(
            matches!(&out[0], Stmt::Let(name, _, _) if name == "chosen"),
            "スプライスした名前で束縛される"
        );
    }

    /// 置いたコードがさらにメタ関数を呼ぶ形も、同じ前向きの走査で処理される。
    /// ⚠ これは**反復ではない**（§1.7 の「前方のみ参照」は保たれる）。
    #[test]
    fn code_placed_by_a_metafn_is_expanded_too() {
        let out = expand(
            "exprconst !fn inner() -> None:\n    quote code:\n        let deep = 1\n\nexprconst !fn outer() -> None:\n    quote code:\n        inner()\n\nouter()\n",
        )
        .expect("expand");
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], Stmt::Let(name, _, _) if name == "deep"));
    }

    /// ⚠ `quote` に到達せず抜けたら**エラー**（設計書 §1.1）。
    /// 黙って無視すると「メタ関数を書いたのに何も起きない」になる。
    #[test]
    fn a_placing_metafn_that_never_quotes_is_an_error() {
        let err = expand("exprconst !fn nothing() -> None:\n    pass\n\nnothing()\n")
            .expect_err("エラーになること");
        assert!(err.contains("without placing any `Code`"), "実際のエラー: {err}");
    }

    /// ⚠⚠ 展開時の `print` は **stderr** へ出す（設計書 §4.1 / タスク 3-4）。
    /// stdout に混ざると `compare_python_impl` / `compare_outputs` の差分比較が壊れ、
    /// 「意味論を守る唯一の網」が**黙って無効**になる。
    #[test]
    fn the_expander_redirects_print_to_stderr() {
        let mut ex = Expander::new(
            std::rc::Rc::new(std::cell::Cell::new(0)),
            std::collections::HashMap::new(),
        );
        assert!(!ex.interp.is_meta_expanding(), "前口上を読む前は既定のまま");
        ex.load_prelude().expect("prelude");
        assert!(
            ex.interp.is_meta_expanding(),
            "展開器のインタプリタが出力先を切り替えていない"
        );
    }

    /// ⚠⚠ **装飾子ファクトリ**（設計書 §1.6「装飾子にはクロージャも書ける」・タスク 4-8）。
    /// `!repeat(3)` は `repeat(3)` を評価し、返ってきたクロージャを対象に適用する
    /// —— Python の `@deco(args)` と同じ形。クロージャの中の `quote` はそのクロージャを抜ける
    /// （ファクトリは既に返っているので、それ以外の意味は取りようがない）。
    #[test]
    fn a_decorator_factory_receives_its_arguments() {
        let out = expand(concat!(
            "exprconst fn tag_with(label: str):\n",
            "    fn deco(m) -> None:\n",
            "        let n = m.name + \"_\" + label\n",
            "        quote code:\n",
            "            let <! n !> = 1\n",
            "    return deco\n",
            "\n",
            "!tag_with(\"seen\")\n",
            "fn f() -> int:\n",
            "    return 1\n",
        ))
        .expect("expand");
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "f_seen")),
            "ファクトリの引数が届いていない"
        );
    }

    /// ⚠⚠ メタ関数に束縛した `const` は**展開時専用**なので実行時の AST から消える（タスク 4-8）。
    /// 残すと実行時に**もう存在しないメタ関数の名前**を読んで `NameError` になる（実測）。
    /// ⚠ `tagged` は配置メタ関数。別名で呼んでも入口の検査（3-8）を通ることもここで見ている
    /// —— 旗を**名前ではなく実体で**立てているから通る。
    #[test]
    fn a_const_bound_to_a_metafn_works_as_a_decorator_and_vanishes() {
        let out = expand(concat!(
            "exprconst !fn tagged(m) -> None:\n",
            "    quote code:\n",
            "        let tag_seen = 1\n",
            "\n",
            "const my_deco = tagged\n",
            "\n",
            "!my_deco\n",
            "fn f() -> int:\n",
            "    return 1\n",
        ))
        .expect("expand");
        let kinds: Vec<&str> = out.iter().map(crate::interpreter::tw_stats::stmt_kind_of).collect();
        assert_eq!(kinds, vec!["Let"], "`const my_deco` が残っていない・装飾子が効いていること");
    }

    /// ⚠ 呼べないものを装飾子に書いたら**そう言う**。
    #[test]
    fn a_decorator_that_is_not_callable_is_an_error() {
        let err = expand(concat!(
            "exprconst fn not_a_deco() -> int:\n",
            "    return 42\n",
            "\n",
            "!not_a_deco()\n",
            "fn f() -> int:\n",
            "    return 1\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("did not produce something callable"), "実際のエラー: {err}");
    }

    /// ⚠⚠ **配置メタ関数を呼んでよいのは展開器だけ**（設計書 §1.1 の表・タスク 3-8）。
    /// メタ関数の本体から呼ぶと `quote` の結果はただの返り値になり、**黙って捨てられた**
    /// （実測: 置いたつもりの `print("hi")` が消えて exit 0）。純粋メタ関数からも同じ。
    #[test]
    fn a_placing_metafn_called_from_a_metafn_is_an_error() {
        let greet = concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
        );
        let from_placing = format!(
            "{greet}exprconst !fn outer() -> None:\n    greet()\n    quote code:\n\nouter()\n"
        );
        let from_pure = format!(
            "{greet}exprconst fn helper() -> Code:\n    greet()\n    let c = code:\n    return c\n\n\
             exprconst !fn outer() -> None:\n    quote helper()\n\nouter()\n"
        );
        for src in [from_placing, from_pure] {
            let err = expand(&src).expect_err("弾かれること");
            assert!(
                err.contains("placing metafunction 'greet' can only run as a statement"),
                "実際のエラー: {err}"
            );
            assert!(err.contains("(raised by metafunction 'outer')"), "実際のエラー: {err}");
        }
    }

    /// ⚠ 実引数の中（式の位置）でも同じ。展開器が呼ぶのは**文として書かれた呼び出し**だけ。
    #[test]
    fn a_placing_metafn_used_as_an_argument_is_an_error() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "\n",
            "exprconst !fn take(c) -> None:\n",
            "    quote code:\n",
            "\n",
            "take(greet())\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("can only run as a statement")
                && err.contains("(raised while evaluating the arguments of 'take')"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠⚠ 実引数の中の `compile_error` は**文面が出る**（タスク 3-8）。
    /// 以前は実引数の評価だけ例外の整形を通っておらず、番兵文字列 ` __raise__` が漏れた（実測）。
    #[test]
    fn compile_error_in_an_argument_is_reported_with_its_message() {
        let err = expand(concat!(
            "exprconst fn checked(n: int) -> int:\n",
            "    if n < 0:\n",
            "        compile_error(\"n must not be negative\")\n",
            "    return n\n",
            "\n",
            "exprconst !fn bind(value: int) -> None:\n",
            "    quote code:\n",
            "        let x = <! value !>\n",
            "\n",
            "bind(checked(-1))\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("n must not be negative (raised while evaluating the arguments of 'bind')"),
            "実際のエラー: {err}"
        );
        assert!(!err.contains("__raise__"), "番兵が漏れている: {err}");
    }

    /// ⚠⚠ 装飾子式の評価の失敗は**握り潰さない**（タスク 3-8）。
    /// 以前は `.ok()` で捨てていたので「前に宣言されていない」という無関係な診断になった（実測）。
    #[test]
    fn a_failing_decorator_expression_is_described() {
        let err = expand(concat!(
            "exprconst fn tag_with(n: int):\n",
            "    fn deco(m) -> None:\n",
            "        quote code:\n",
            "    return deco\n",
            "\n",
            "!tag_with(1 // 0)\n",
            "fn f() -> int:\n",
            "    return 1\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("while evaluating decorator '!tag_with(...)'") && err.contains("ZeroDivisionError"),
            "実際のエラー: {err}"
        );
        assert!(!err.contains("not a metafunction declared"), "取り違えている: {err}");
    }

    /// ⚠ 実行時の `NameError` への書き添えは**メタ関数の名前のときだけ**（タスク 3-8）。
    #[test]
    fn a_runtime_name_error_on_a_removed_metafn_gets_a_note() {
        let metafns: std::collections::HashMap<String, bool> =
            [("greet".to_string(), true), ("frag".to_string(), false)].into_iter().collect();
        let placing = explain_removed_metafn("NameError: 'greet' is not defined".into(), &metafns);
        assert!(placing.contains("note: 'greet' is a placing metafunction"), "{placing}");
        let pure = explain_removed_metafn("NameError: 'frag' is not defined".into(), &metafns);
        assert!(pure.contains("note: 'frag' is a pure metafunction"), "{pure}");
        let other = explain_removed_metafn("NameError: 'other' is not defined".into(), &metafns);
        assert_eq!(other, "NameError: 'other' is not defined", "関係ない名前には書き添えない");
    }

    /// ⚠⚠ 宣言は**あるが**メタ情報を持たない種類に `^` を当てたら、**そう言う**（タスク 3-9）。
    /// 以前は対象の種類だけを宣言表に入れていたので「前に何も宣言されていない」と誤報した（実測）。
    #[test]
    fn caret_on_a_metafn_says_what_it_is() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "\n",
            "exprconst !fn show(m) -> None:\n",
            "    quote code:\n",
            "\n",
            "show(^greet)\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("`^greet` is not available — 'greet' is a placing metafunction"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠⚠ `^` が**実行時に**評価されたら「宣言されていない」ではない（タスク 3-9）。
    /// `print(^T)` は位置制限（1-3）を通る（呼び先がメタ関数かはパース時に分からない）ので、
    /// 展開器が評価しないまま実行時に来る。以前は「前に何も宣言されていない」と誤報した（実測）。
    #[test]
    fn caret_evaluated_at_run_time_says_so() {
        let interp = Interpreter::new();
        let err = interp.meta_lookup_error("Marker");
        assert!(
            err.contains("can only be evaluated while the compile-time expander runs"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠⚠ 装飾子の失敗は**どの宣言のものか**を位置で言う（タスク 3-9）。
    /// `Expr::Ident` は位置を持たないので、以前は位置の無い診断になっていた（実測）。
    #[test]
    fn a_decorator_failure_points_at_the_decorator() {
        let err = expand(concat!(
            "exprconst fn not_a_deco() -> int:\n",
            "    return 42\n",
            "\n",
            "!not_a_deco()\n",
            "fn f() -> int:\n",
            "    return 1\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("while expanding '!not_a_deco(...)' at") && err.contains("line 4, col 1"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠ メソッド形の射影（`has_field` など）も**どの射影か**を言う（タスク 3-9）。
    #[test]
    fn a_method_projection_that_does_not_fit_names_itself() {
        let err = expand(concat!(
            "fn helper() -> int:\n",
            "    return 1\n",
            "\n",
            "exprconst !fn probe(m) -> None:\n",
            "    let b = m.has_field(\"x\")\n",
            "    quote code:\n",
            "\n",
            "probe(^helper)\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("`.has_field()` needs a class or trait"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠⚠ 置いたコードの構文エラーは `code:` の中の**書いた位置**を指す（タスク 5-1 で発見）。
    /// 2-3 で行ごとに位置を記録していたのに、この文面では使っておらず呼び出し位置しか出なかった。
    #[test]
    fn a_placed_code_parse_error_points_into_the_code_block() {
        let err = expand(concat!(
            "exprconst !fn broken() -> None:\n",
            "    quote code:\n",
            "        let = 1\n",
            "\n",
            "broken()\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("inside the `code:` block") && err.contains("line 3, col 13"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠⚠ **メタ関数でない関数の中では、メタ関数を呼べない**（設計書 §1.1・2026-09-25 確定・タスク 2-11）。
    /// 以前は展開されずに残り、**その関数を呼んだときに**実行時 `NameError` になった。
    #[test]
    fn a_placing_call_inside_a_function_is_rejected() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
            "fn work() -> None:\n",
            "    greet()\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("placing metafunction 'greet' cannot be called inside function 'work'"), "実際のエラー: {err}");
    }

    /// ⚠ メソッドも関数。クラス本体に**文として**書いた呼び出しとは別物。
    #[test]
    fn a_placing_call_inside_a_method_is_rejected() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
            "class C:\n",
            "    fn m(self) -> None:\n",
            "        greet()\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("cannot be called inside function 'm'"), "実際のエラー: {err}");
    }

    /// ⚠ 最上位でも**ブロックの中**は置き換えない。ブロックは実行時に走るが、
    /// メタ関数は展開時に走る（展開時に選びたいならメタ関数の中で `const` で分岐する）。
    #[test]
    fn a_placing_call_inside_a_top_level_block_is_rejected() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
            "const DEBUG = True\n",
            "if DEBUG:\n",
            "    greet()\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("cannot be called inside a block"), "実際のエラー: {err}");
    }

    /// ⚠ 配置メタ関数は値を返さない（§1.1）。
    #[test]
    fn a_placing_metafn_used_as_a_value_is_rejected() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
            "let x = greet()\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("placing metafunction 'greet' cannot be used as a value"), "実際のエラー: {err}");
    }

    /// ⚠ 純粋メタ関数は `Code` を返す。通常コードからは呼べない（§1.1）。
    #[test]
    fn a_pure_metafn_called_from_ordinary_code_is_rejected() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
            "exprconst fn frag() -> Code:\n",
            "    let c = code:\n",
            "    return c\n",
            "\n",
            "frag()\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("pure metafunction 'frag' cannot be called from ordinary code"), "実際のエラー: {err}");
    }

    /// ⚠ 入れ子のメタ関数の定義は許さない（§1.1・2026-09-25 確定）。
    #[test]
    fn a_metafn_defined_inside_a_function_is_rejected() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
            "fn outer() -> None:\n",
            "    exprconst fn inner() -> Code:\n",
            "        let c = code:\n",
            "        return c\n",
            "    print(1)\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("metafunction 'inner' is defined inside function 'outer'"), "実際のエラー: {err}");
    }

    /// ⚠ 関数の中の `!装飾子` も展開されない。以前は実行時まで残って落ちた。
    #[test]
    fn a_decorator_inside_a_function_is_rejected() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
            "exprconst !fn deco(m) -> None:\n",
            "    quote m.code()\n",
            "\n",
            "fn f() -> None:\n",
            "    !deco\n",
            "    let x = 1\n",
            "    print(x)\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("decorator '!deco' is written inside function 'f'"), "実際のエラー: {err}");
    }

    /// ⚠ 通常コードの `^` は展開時に止める（以前は実行時に 3-9 の文面で落ちた）。
    #[test]
    fn caret_in_ordinary_code_is_rejected_while_expanding() {
        let err = expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
            "class Marker:\n",
            "    mut id: int\n",
            "\n",
            "print(^Marker)\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("`^Marker` is written at the top level in ordinary code"), "実際のエラー: {err}");
    }

    /// ⚠⚠ 同じ名前の普通の束縛（ここでは仮引数）があるメタ関数は**調べない**（保守的・2-11）。
    /// 見分けるにはスコープの追跡が要り、取り違えると正しいプログラムを弾くので。
    #[test]
    fn a_parameter_that_shadows_a_metafn_is_not_rejected() {
        expand(concat!(
            "exprconst !fn greet() -> None:\n",
            "    quote code:\n",
            "        print(\"hi\")\n",
            "\n",
            "fn shadow(greet: int) -> int:\n",
            "    return greet + 1\n",
            "\n",
            "greet()\n",
        ))
        .expect("仮引数の `greet` はメタ関数ではない");
    }

    /// `import` 文をディスクを読まずに組み立てて展開する（タスク 2-12 のテスト用）。
    ///
    /// ⚠ モジュールと本体は**同じ node-id カウンタ**でパースする（パーサの `parse_sub_module` と同じ）。
    fn expand_with_module(module_src: &str, imports: &[Stmt], main_src: &str) -> Result<Vec<Stmt>, String> {
        let (body, counter, traits) = parse(module_src)?;
        let tokens = crate::lexer::Lexer::new(main_src, "").tokenize();
        let mut parser = crate::parser::Parser::new(tokens, None);
        parser.set_node_counter(std::rc::Rc::clone(&counter));
        let main = parser.parse_program()?;
        let mut stmts: Vec<Stmt> = Vec::new();
        for imp in imports {
            let mut imp = imp.clone();
            match &mut imp {
                Stmt::Import { body: b, .. } | Stmt::FromImport { body: b, .. } => *b = body.clone(),
                _ => unreachable!(),
            }
            stmts.push(imp);
        }
        stmts.extend(main);
        expand_program(stmts, counter, traits)
    }

    fn import_of(alias: Option<&str>) -> Stmt {
        Stmt::Import {
            lang: "ar-auto".to_string(),
            module: vec!["m".to_string()],
            source_module: None,
            alias: alias.map(str::to_string),
            body: Vec::new(),
        }
    }

    fn from_import_of(names: &[&str]) -> Stmt {
        Stmt::FromImport {
            lang: "ar-auto".to_string(),
            module: vec!["m".to_string()],
            source_module: None,
            names: names.iter().map(|n| (n.to_string(), None)).collect(),
            body: Vec::new(),
        }
    }

    fn module_src() -> &'static str {
        concat!(
            "const LEVEL = 3\n",
            "exprconst fn helper_line() -> Code:\n",
            "    let c = code:\n",
            "        let from_module = <! LEVEL !>\n",
            "    return c\n",
            "exprconst !fn place_it() -> None:\n",
            "    quote helper_line()\n",
            "exprconst !fn tick() -> None:\n",
            "    quote code:\n",
            "        let <! gensym(\"tick\") !> = 1\n",
            "tick()\n",
        )
    }

    /// ⚠⚠ **別のモジュールのメタ関数も使える**（設計書 §1.7・2026-09-25 確定・タスク 2-12）。
    /// `import m` なら `m.place_it()` が配置呼び出しになる。
    #[test]
    fn a_module_metafn_can_be_called_through_the_module_name() {
        let out = expand_with_module(module_src(), &[import_of(None)], "m.place_it()\n").expect("expand");
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "from_module")),
            "`m.place_it()` が置き換えられていない"
        );
    }

    /// ⚠ `from m import name` なら素の名前で呼べる。⚠⚠ メタ関数の名前は**実行時の import から外れる**
    /// （残すと実行時に `ImportError: cannot import name` になった・実測）。
    #[test]
    fn a_from_imported_metafn_is_called_by_its_plain_name() {
        let out = expand_with_module(module_src(), &[from_import_of(&["place_it"])], "place_it()\n")
            .expect("expand");
        assert!(out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "from_module")));
        let Some(Stmt::FromImport { names, .. }) = out.first() else { panic!("FromImport を期待") };
        assert!(names.is_empty(), "メタ関数の名前が実行時の import に残っている: {names:?}");
    }

    /// ⚠⚠ モジュールのメタ関数は**自分のモジュールの名前**を引く（閉包として捕まえる）。
    /// 呼び出し側に同じ名前のメタ関数があっても取り違えない（実行時のモジュール関数と同じ性質）。
    #[test]
    fn a_module_metafn_uses_its_own_module_names() {
        let main = concat!(
            "exprconst fn helper_line() -> Code:\n",
            "    let c = code:\n",
            "        let from_main = 1\n",
            "    return c\n",
            "\n",
            "m.place_it()\n",
        );
        let out = expand_with_module(module_src(), &[import_of(None)], main).expect("expand");
        assert!(out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "from_module")));
        assert!(!out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "from_main")));
    }

    /// ⚠⚠ モジュールは**初めて読み込まれるときに 1 回だけ**展開する（§1.7・2026-09-25 確定）。
    /// 2 回目の import は展開済みの本体を使い回す —— モジュールの中の `gensym` が 1 回しか進まない。
    #[test]
    fn a_module_imported_twice_is_expanded_once() {
        let out = expand_with_module(module_src(), &[import_of(None), import_of(Some("again"))], "")
            .expect("expand");
        let names_in = |s: &Stmt| -> Vec<String> {
            let Stmt::Import { body, .. } = s else { panic!("Import を期待") };
            body.iter()
                .filter_map(|st| match st {
                    Stmt::Let(n, _, _) => Some(n.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(names_in(&out[0]), vec!["tick__1".to_string()]);
        assert_eq!(names_in(&out[1]), vec!["tick__1".to_string()], "2 回目も同じ本体（再展開していない）");
    }

    /// ⚠ モジュールの中の展開エラーは**どのモジュールか**を添える。
    #[test]
    fn an_error_inside_a_module_names_the_module() {
        let bad = "exprconst !fn boom() -> None:\n    compile_error(\"boom\")\n    quote code:\n\nboom()\n";
        let err = expand_with_module(bad, &[import_of(None)], "").expect_err("弾かれること");
        assert!(err.contains("boom") && err.contains("while importing module 'm'"), "実際のエラー: {err}");
    }

    /// ⚠ メタ関数を使うモジュールは**最上位で** import する（関数の中の import の本体は展開されない）。
    #[test]
    fn a_metafn_module_imported_inside_a_function_is_rejected() {
        let (body, counter, traits) = parse(module_src()).expect("parse");
        let mut imp = import_of(None);
        if let Stmt::Import { body: b, .. } = &mut imp {
            *b = body;
        }
        let tokens = crate::lexer::Lexer::new("fn f() -> None:\n    pass\n", "").tokenize();
        let mut parser = crate::parser::Parser::new(tokens, None);
        parser.set_node_counter(std::rc::Rc::clone(&counter));
        let mut main = parser.parse_program().expect("parse");
        if let Stmt::FnDef { body, .. } = &mut main[0] {
            body.insert(0, imp);
        }
        let err = expand_program(main, counter, traits).expect_err("弾かれること");
        assert!(
            err.contains("module 'm' uses metafunctions, so it must be imported at the top level"),
            "実際のエラー: {err}"
        );
    }

    /// 展開後の文の中から、名前が `name` のクラス・関数宣言を探す（単相化のテスト用）。
    fn find_decl<'a>(out: &'a [Stmt], name: &str) -> Option<(usize, &'a Stmt)> {
        out.iter().enumerate().find(|(_, s)| match s {
            Stmt::ClassDef { name: n, .. } | Stmt::FnDef { name: n, .. } | Stmt::GenDef { name: n, .. } => n == name,
            _ => false,
        })
    }

    /// ⚠⚠ **テンプレートの具体化に出会った時点で実体を作る**（インライン単相化・参考P・タスク 2-8）。
    /// 名前は `Box[int]`、型変数は具体型に置換され、**使う文の直前**に置かれる。
    #[test]
    fn a_template_class_is_instantiated_before_its_first_use() {
        let out = expand(concat!(
            "class Box[T]:\n",
            "    mut v: T\n",
            "    mut items: list[T]\n",
            "\n",
            "let b = Box[int](1, [2])\n",
            "let c = Box[int](3, [4])\n",
        ))
        .expect("expand");
        let (at, decl) = find_decl(&out, "Box[int]").expect("`Box[int]` が作られていない");
        let Stmt::ClassDef { body, template_params, .. } = decl else { panic!("ClassDef を期待") };
        assert!(template_params.is_empty(), "具体化は型パラメータを持たない");
        let fields: Vec<(String, String)> = body
            .iter()
            .filter_map(|s| match s {
                Stmt::Field { name, type_ann, .. } => Some((name.clone(), type_ann.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            fields,
            vec![("v".to_string(), "int".to_string()), ("items".to_string(), "list[int]".to_string())],
            "合成型の中の `T` も置換されること"
        );
        let first_use = out.iter().position(|s| matches!(s, Stmt::Let(n, _, _) if n == "b")).unwrap();
        assert!(at < first_use, "使う文より前に置かれること");
        let count = out.iter().filter(|s| matches!(s, Stmt::ClassDef { name, .. } if name == "Box[int]")).count();
        assert_eq!(count, 1, "同じ型引数の具体化は 1 回だけ（メモ化）");
    }

    /// ⚠ テンプレート関数も同じ（`ident[str]("s")` → `fn ident[str](a: str) -> str`）。
    #[test]
    fn a_template_function_is_instantiated() {
        let out = expand(concat!(
            "fn ident[T](let a: T) -> T:\n",
            "    return a\n",
            "\n",
            "print(ident[str](\"s\"))\n",
        ))
        .expect("expand");
        let (_, decl) = find_decl(&out, "ident[str]").expect("`ident[str]` が作られていない");
        let Stmt::FnDef { params, return_type, .. } = decl else { panic!("FnDef を期待") };
        assert_eq!(params[0].type_ann.as_deref(), Some("str"));
        assert_eq!(return_type.as_deref(), Some("str"));
    }

    /// ⚠⚠ **テンプレートの宣言より前**に書いた具体化（関数の本体の中など）も作る（タスク 2-14）。
    /// 宣言に出会った時点で作り、**宣言の直後**に置く。関数の本体が呼ばれるのはもっと後なので、
    /// 実行時の意味は変わらない。以前は作らなかったので、その具体化の検査が丸ごと抜けていた。
    #[test]
    fn an_instantiation_written_before_the_template_is_made_after_it() {
        let out = expand(concat!(
            "fn early() -> int:\n",
            "    return Box[int](1).v\n",
            "\n",
            "class Box[T]:\n",
            "    mut v: T\n",
            "\n",
            "print(early())\n",
        ))
        .expect("expand");
        let (tmpl, _) = find_decl(&out, "Box").expect("テンプレート");
        let (inst, _) = find_decl(&out, "Box[int]").expect("宣言より前の具体化も作られること");
        assert!(tmpl < inst, "テンプレートの宣言の後ろに置かれること");
    }

    /// ⚠⚠ 制約付きのテンプレートは、**制約を満たすと確かめられたときだけ**作る（タスク 2-14）。
    /// 判定は実行時と同じ（型引数がクラスで、基底にその trait がある）。満たさない・判断できない
    /// （組み込み型）ときは作らず、実行時の `TemplateError` に任せる（従来どおり）。
    #[test]
    fn a_constrained_template_is_instantiated_only_when_the_constraint_holds() {
        let out = expand(concat!(
            "trait Named:\n",
            "    fn label(self) -> str:\n",
            "        ...\n",
            "\n",
            "class Cat(Named):\n",
            "    mut n: int\n",
            "    fn label(self) -> str:\n",
            "        return \"cat\"\n",
            "\n",
            "class Holder[T: Named]:\n",
            "    mut v: T\n",
            "\n",
            "let ok = Holder[Cat](Cat(1))\n",
            "let bad = Holder[int](1)\n",
        ))
        .expect("expand");
        assert!(find_decl(&out, "Holder[Cat]").is_some(), "制約を満たす具体化は作る");
        assert!(find_decl(&out, "Holder[int]").is_none(), "満たさない具体化は実行時に任せる");
    }

    /// ⚠⚠ `^Box[int]` で具体化のメタ情報を引ける（タスク 2-8）。フィールドの型は置換後のもの。
    /// ⚠ 呼び出しの付かない `^Box[int]` は以前、添字（`Box` の `int` 番目）として読まれていた。
    #[test]
    fn caret_on_a_template_instantiation_sees_the_concrete_declaration() {
        let out = expand(concat!(
            "class Box[T]:\n",
            "    mut v: T\n",
            "\n",
            "exprconst !fn field_type_of(m) -> None:\n",
            "    let t = m.fields[0].type\n",
            "    quote code:\n",
            "        let copy: <! t !> = 0\n",
            "\n",
            "field_type_of(^Box[int])\n",
        ))
        .expect("expand");
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let(n, Some(t), _) if n == "copy" && t == "int")),
            "`^Box[int].fields[0].type` が `int` になっていない"
        );
    }

    /// ⚠ 置換は**識別子単位**。`Tag` の中の `T` のような部分一致はしない。
    #[test]
    fn subst_type_replaces_whole_identifiers_inside_composite_types() {
        let m: std::collections::HashMap<String, String> =
            [("T".to_string(), "int".to_string()), ("K".to_string(), "str".to_string())].into_iter().collect();
        use crate::template_subst::subst_type;
        assert_eq!(subst_type("T", &m), "int");
        assert_eq!(subst_type("list[T]", &m), "list[int]");
        assert_eq!(subst_type("dict[K, T]", &m), "dict[str, int]");
        assert_eq!(subst_type("Option[Box[T]]", &m), "Option[Box[int]]");
        assert_eq!(subst_type("Tag", &m), "Tag");
        assert_eq!(subst_type("function[T]->T", &m), "function[int]->int");
    }

    /// ⚠⚠ **再帰的な具体化は終わらない**ので、深さの上限で止めて連鎖を見せる（タスク 2-9）。
    /// `f[T]` が `f[Box[T]]` を呼ぶと型引数が伸び続ける。展開時の単相化は書かれた具体化を
    /// すべて先に作るので、実行時の再帰が途中で止まる書き方でも終わらない。
    /// 以前（2-8）は上限まで黙って作り続けて 2 秒以上かかった（実測）。
    #[test]
    fn a_recursive_template_instantiation_is_reported() {
        let err = expand(concat!(
            "class Box[T]:\n",
            "    mut v: T\n",
            "\n",
            "fn depth[T](let x: T, let n: int) -> int:\n",
            "    if n == 0:\n",
            "        return 0\n",
            "    return 1 + depth[Box[T]](Box[T](x), n - 1)\n",
            "\n",
            "print(depth[int](1, 3))\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("template instantiation does not end")
                && err.contains("'depth[int]' → 'depth[Box[int]]' → 'depth[Box[Box[int]]]'"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠ 自分自身を**同じ型引数で**使うテンプレート（型レベルの再帰）は止まる。
    /// メモ化の印を置換の前に付けているので、`Node[int]` の中の `Node[int]` で潜り直さない。
    #[test]
    fn a_template_that_uses_itself_with_the_same_arguments_terminates() {
        let out = expand(concat!(
            "class Node[T]:\n",
            "    mut v: T\n",
            "    mut next: Option[Node[T]]\n",
            "\n",
            "    fn prepend(self, let x: T) -> Node[T]:\n",
            "        return Node[T](x, None)\n",
            "\n",
            "let n = Node[int](1, None)\n",
        ))
        .expect("expand");
        let count = out.iter().filter(|s| matches!(s, Stmt::ClassDef { name, .. } if name == "Node[int]")).count();
        assert_eq!(count, 1);
    }

    /// 展開してから型検査にかけ、エラーの文面（色付けを外したもの）を返す（タスク 2-8 段階 2）。
    fn expand_and_check(src: &str) -> Vec<String> {
        let out = expand(src).expect("expand");
        let (errors, _, _) = crate::type_check::TypeChecker::check_program(&out);
        errors
            .iter()
            .map(|e| {
                let s = e.to_string();
                let mut m = String::new();
                let mut esc = false;
                for c in s.chars() {
                    if esc {
                        esc = c != 'm';
                    } else if c == '\x1b' {
                        esc = true;
                    } else {
                        m.push(c);
                    }
                }
                m
            })
            .collect()
    }

    /// ⚠⚠ **具体化した本体を型検査にかける**（2026-09-26 決定・タスク 2-8 段階 2）。
    /// `Box[str]` の `self.v = 0` は誤り。以前は「`T` とは突き合わせない」として通していた。
    #[test]
    fn a_concrete_type_error_in_an_instantiated_body_is_reported() {
        let errs = expand_and_check(concat!(
            "class Box[T]:\n",
            "    mut v: T\n",
            "    fn reset(mut self) -> None:\n",
            "        self.v = 0\n",
            "let a = Box[int](1)\n",
            "let b = Box[str](\"x\")\n",
        ));
        assert!(errs.iter().any(|m| m.contains("field 'v' of class 'Box[str]' is declared 'str' but got 'int'")), "出ていない: {errs:?}");
        assert!(!errs.iter().any(|m| m.contains("class 'Box[int]'")), "出てはいけない: {errs:?}");
    }

    /// ⚠ 同じ本体でも、型が合う具体化（`Box[int]`）だけなら通る。
    #[test]
    fn an_instantiation_where_the_body_fits_is_accepted() {
        let errs = expand_and_check(concat!(
            "class Box[T]:\n",
            "    mut v: T\n",
            "    fn reset(mut self) -> None:\n",
            "        self.v = 0\n",
            "let a = Box[int](1)\n",
        ));
        assert!(errs.is_empty(), "偽陽性: {errs:?}");
    }

    /// ⚠⚠ **テンプレートクラスのメソッド呼び出しが丸ごと検査されていなかった**（フェーズ10 の発端）。
    /// 受け手のクラスを引く所が `GenericInstance` を見ておらず、検査が飛ばされていた。
    #[test]
    fn method_calls_on_a_template_instance_are_checked() {
        let errs = expand_and_check(concat!(
            "class Box[T]:\n",
            "    mut v: T\n",
            "    fn get(self) -> T:\n",
            "        return self.v\n",
            "    fn set(mut self, let x: T) -> None:\n",
            "        self.v = x\n",
            "    fn tag(mut self, let n: int) -> None:\n",
            "        print(n)\n",
            "mut b = Box[int](1)\n",
            "b.tag(\"wrong\")\n",
            "b.set(\"wrong\")\n",
            "let s: str = b.get()\n",
        ));
        assert!(errs.iter().any(|m| m.contains("argument 0 of 'Box[int].tag' expects 'int' but got 'str'")), "出ていない: {errs:?}");
        assert!(errs.iter().any(|m| m.contains("argument 0 of 'Box[int].set' expects 'int' but got 'str'")), "出ていない: {errs:?}");
        assert!(errs.iter().any(|m| m.contains("'s' is declared 'str' but initialized with 'int'")), "出ていない: {errs:?}");
    }

    /// ⚠⚠ `gen take[T]` の `T` がスコープに入らず誤って弾かれていた（フェーズ10 10-1・8.5 の退行）。
    #[test]
    fn a_template_gen_sees_its_type_parameters() {
        let errs = expand_and_check(concat!(
            "gen take[T](let xs: list[T], let n: int) -> T:\n",
            "    mut i = 0\n",
            "    while i < n:\n",
            "        yield xs[i]\n",
            "        i += 1\n",
            "for x in take[int]([1, 2], 2):\n",
            "    print(x)\n",
        ));
        assert!(errs.is_empty(), "偽陽性: {errs:?}");
    }

    /// ⚠ `mut items: list[T]` の自動 `__init__` が「`list[unknown]` に `list[T]`」と誤って弾かれていた。
    /// 具体化では要素の型違いがちゃんと捕まる。
    #[test]
    fn a_list_field_of_a_type_parameter_is_accepted_and_checked() {
        let errs = expand_and_check(concat!(
            "class Tagged[T]:\n",
            "    mut v: T\n",
            "    mut items: list[T]\n",
            "let good = Tagged[int](1, [2])\n",
            "let bad = Tagged[int](1, [\"x\"])\n",
        ));
        assert!(errs.iter().any(|m| m.contains("expects 'list[int]' but got 'list[str]'")), "出ていない: {errs:?}");
        assert!(!errs.iter().any(|m| m.contains("list[unknown]")), "出てはいけない: {errs:?}");
    }

    /// ⚠ 型変数に関係しない誤りは、テンプレートの本体と各具体化の**全部で**見つかる。
    /// 同じ誤りを具体化の数＋1 回出さないよう、1 件にまとめる（タスク 2-8 段階 2）。
    #[test]
    fn the_same_error_is_not_repeated_for_every_instantiation() {
        let errs = expand_and_check(concat!(
            "class Labeled[T]:\n",
            "    mut v: T\n",
            "    fn label(self) -> str:\n",
            "        return 1\n",
            "let a = Labeled[int](1)\n",
            "let b = Labeled[str](\"x\")\n",
        ));
        let n = errs.iter().filter(|m| m.contains("declared to return 'str' but returns 'int'")).count();
        assert_eq!(n, 1, "重複している: {errs:?}");
    }

    /// ⚠⚠ 適合しているテンプレートの具体化を protocol 型に入れると「クラスのインスタンスではない」と
    /// **誤って弾いていた**（タスク 2-14）。単相化した具体クラスで適合を見る。適合しないものは理由つきで弾く。
    #[test]
    fn a_template_instance_is_checked_against_a_protocol_structurally() {
        let src = |ret: &str| {
            format!(
                "protocol Sized:\n    fn size(self) -> int:\n        ...\n\nclass Box[T]:\n    mut v: T\n    fn size(self) -> {ret}:\n        return 1\n\nlet s: Sized = Box[int](1)\n"
            )
        };
        let ok = expand_and_check(&src("int"));
        assert!(ok.is_empty(), "偽陽性: {ok:?}");
        let bad = expand_and_check(&src("str"));
        assert!(
            bad.iter().any(|m| m.contains("type 'Box[int]' does not satisfy protocol 'Sized'")),
            "出ていない: {bad:?}"
        );
    }

    /// ⚠⚠ テンプレートの本体で `T` の値を具体型の枠へ入れる形（`let y: int = x`・`h(x)`・`return x`・
    /// `self.set("x")`）は、`g[int]` / `Box[str]` しか使っていなければ正しい（タスク 2-15）。
    /// 以前は期待型の側の `T` しか除外しておらず、値の側の `T` で偽のエラーになっていた。
    /// 型引数が合わない具体化は、具体化の本体で捕まる。
    #[test]
    fn a_type_parameter_on_either_side_is_left_to_the_instantiations() {
        let src = |targ: &str, arg: &str| {
            format!(
                "fn h(let n: int) -> int:\n    return n\n\nfn g[T](let x: T) -> int:\n    let y: int = x\n    print(h(x))\n    return x\n\nclass Box[T]:\n    mut v: T\n    fn set(mut self, let x: T) -> None:\n        self.v = x\n    fn fill(mut self) -> None:\n        self.set(\"x\")\n\nprint(g[{targ}]({arg}))\nmut b = Box[{targ}]({arg})\nb.fill()\n"
            )
        };
        let ok = expand_and_check(&src("int", "1").replace("Box[int](1)", "Box[str](\"a\")"));
        assert!(ok.is_empty(), "偽陽性: {ok:?}");
        let bad = expand_and_check(&src("str", "\"a\"").replace("Box[str](\"a\")", "Box[int](1)"));
        assert!(bad.iter().any(|m| m.contains("'y' is declared 'int' but initialized with 'str'")), "{bad:?}");
        assert!(bad.iter().any(|m| m.contains("'g[str]' is declared to return 'int' but returns 'str'")), "{bad:?}");
        assert!(bad.iter().any(|m| m.contains("argument 0 of 'Box[int].set' expects 'int' but got 'str'")), "{bad:?}");
    }

    /// ⚠ 関数型の中の型変数も置換する（タスク 2-15）。`adder[int](1)` が `function[T]->T` のまま
    /// 外へ出て、`f(2)` が「`T` を期待したのに `int`」という偽のエラーになっていた。
    #[test]
    fn a_function_type_returned_from_a_template_function_is_substituted() {
        let errs = expand_and_check(concat!(
            "fn adder[T](let base: T) -> function[T]->T:\n",
            "    fn add(let x: T) -> T:\n",
            "        return base + x\n",
            "    return add\n",
            "let f = adder[int](1)\n",
            "print(f(2))\n",
            "print(f(\"s\"))\n",
        ));
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("argument 0 of 'f' expects 'int' but got 'str'"), "{errs:?}");
    }

    fn template_module_src() -> &'static str {
        concat!(
            "class Box[T]:\n",
            "    mut v: T\n",
            "    fn set(mut self, let x: T) -> None:\n",
            "        self.v = x\n",
            "\n",
            "let inner = Box[str](\"own\")\n",
        )
    }

    fn body_of<'a>(out: &'a [Stmt], i: usize) -> &'a [Stmt] {
        match &out[i] {
            Stmt::Import { body, .. } | Stmt::FromImport { body, .. } => body,
            _ => panic!("import を期待"),
        }
    }

    /// ⚠⚠ `import` したモジュールのテンプレートの具体化（`m.Box[int]` / `from m import Box`）は、
    /// **その `import` 文の本体の末尾**に置く（タスク 2-15）。メインに置くと、クラスの本体が
    /// モジュールの名前ではなくメインの名前を引いてしまう。以前は作らなかったので検査が丸ごと抜けていた。
    #[test]
    fn an_imported_template_is_instantiated_inside_the_module_body() {
        for (imp, main) in [
            (import_of(None), "let b = m.Box[int](1)\n"),
            (from_import_of(&["Box"]), "let b = Box[int](1)\n"),
        ] {
            let out = expand_with_module(template_module_src(), &[imp], main).expect("expand");
            assert!(find_decl(&out, "Box[int]").is_none(), "メインには置かない");
            let body = body_of(&out, 0);
            let (at, _) = find_decl(body, "Box[int]").expect("モジュールの本体に置く");
            assert_eq!(at, body.len() - 1, "本体の末尾に置く");
        }
    }

    /// ⚠ モジュール**自身の中の**具体化も、その本体の中で単相化する（タスク 2-15）。
    /// 使う文より前に置く（メインのプログラムと同じ規則）。
    #[test]
    fn a_module_s_own_instantiation_is_placed_before_its_use() {
        let out = expand_with_module(template_module_src(), &[import_of(None)], "print(1)\n").expect("expand");
        let body = body_of(&out, 0);
        let (at, _) = find_decl(body, "Box[str]").expect("モジュールの中の具体化");
        let use_at = body.iter().position(|s| matches!(s, Stmt::Let(n, _, _) if n == "inner")).unwrap();
        assert!(at < use_at, "使う文より前に置く");
    }

    /// ⚠⚠ `import` 先の具体化の型の誤りも出る（タスク 2-15）。型検査はモジュールの本体の診断を
    /// 捨てるが、具体化の本体で見つかった誤り（型引数を選んだ側の誤り）は残す。
    #[test]
    fn a_type_error_in_an_imported_instantiation_is_reported() {
        let out = expand_with_module(
            concat!(
                "class Box[T]:\n",
                "    mut v: T\n",
                "    fn reset(mut self) -> None:\n",
                "        self.v = 0\n",
                "    fn label(self) -> str:\n",
                "        return 1\n",
            ),
            &[import_of(None)],
            "mut b = m.Box[str](\"x\")\nb.reset()\n",
        )
        .expect("expand");
        let (errors, _, _) = crate::type_check::TypeChecker::check_program(&out);
        let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        assert!(msgs.iter().any(|m| m.contains("Box[str]") && m.contains("declared")), "出ていない: {msgs:?}");
        // テンプレートの本体の誤り（`label`）はモジュール自身の誤りなので import 側には出さない。
        assert!(!msgs.iter().any(|m| m.contains("label")), "出てはいけない: {msgs:?}");
    }

    #[test]
    fn split_instance_name_is_the_inverse_of_instance_name() {
        use crate::template_subst::{instance_name, split_instance_name};
        for args in [vec!["int"], vec!["Pair[int, str]", "int"], vec!["function[int, str]->int"]] {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            let name = instance_name("Box", &args);
            assert_eq!(split_instance_name(&name), Some(("Box", args.clone())));
        }
        assert_eq!(split_instance_name("Box"), None);
    }

    /// ⚠⚠ **具体化どうしで node-id を共有しない**（設計書 §0.3・タスク 2-13）。
    /// 型検査の注釈は node-id で引かれるので、共有すると後に検査した具体化の型が全部に効く
    /// （`add_all[int]` の `a + b` が float の加算 `FBIN` にコンパイルされた・実測）。
    #[test]
    fn instantiations_get_their_own_node_ids() {
        let out = expand(concat!(
            "fn add_all[T](let a: T, let b: T) -> T:\n",
            "    return a + b\n",
            "\n",
            "print(add_all[int](1, 2))\n",
            "print(add_all[float](1.5, 2.5))\n",
        ))
        .expect("expand");
        let add_id = |name: &str| -> u32 {
            let (_, decl) = find_decl(&out, name).expect("宣言が無い");
            let Stmt::FnDef { body, .. } = decl else { panic!("FnDef を期待") };
            let Stmt::Return(Some(Expr::BinOp { node_id, .. })) = &body[0] else { panic!("return a + b を期待") };
            *node_id
        };
        let (t, i, f) = (add_id("add_all"), add_id("add_all[int]"), add_id("add_all[float]"));
        assert!(t != i && t != f && i != f, "node-id が衝突している: {t} {i} {f}");
    }

    /// ⚠⚠ 展開時は**決定的で副作用の無い**ビルトインしか呼べない（D26 / 参考N・タスク 3-7）。
    /// 強制していなかった頃は、メタ関数の中で `open` がそのまま通ってファイルを開けた（実測）。
    /// エディタが展開を走らせるようになると（D5）、打鍵のたびにファイルを触ることになる。
    #[test]
    fn forbidden_builtins_are_rejected_while_expanding() {
        for (call, name) in [
            ("open(\"x.txt\", FileOpenMode.read)", "open"),
            ("id(1)", "id"),
            ("getenv(\"PATH\")", "getenv"),
            ("parse_ar(\"let a = 1\")", "parse_ar"),
        ] {
            let src = format!(
                concat!(
                    "exprconst !fn sneaky() -> None:\n",
                    "    let r = {}\n",
                    "    quote code:\n",
                    "\n",
                    "sneaky()\n",
                ),
                call
            );
            let err = expand(&src).expect_err(name);
            assert!(
                err.contains(&format!("`{name}` cannot be called while expanding")),
                "{name} が通ってしまった: {err}"
            );
        }
    }

    /// ⚠ 受け手の型で止める（メソッド名で並べると後から足したメソッドが素通りする）。
    /// `EventLoop` は大域の単一値なので、生成を止めるだけでは足りない。
    #[test]
    fn the_event_loop_cannot_be_touched_while_expanding() {
        let err = expand(concat!(
            "exprconst !fn ev() -> None:\n",
            "    EventLoop.run()\n",
            "    quote code:\n",
            "\n",
            "ev()\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("the event loop cannot be used while expanding"), "実際のエラー: {err}");
    }

    /// ⚠ 許可されたビルトインは今まで通り使える（厳しすぎないことの対照）。
    #[test]
    fn allowed_builtins_still_work_while_expanding() {
        let out = expand(concat!(
            "exprconst !fn count_up() -> None:\n",
            "    let n = len([1, 2, 3]) + int(\"4\")\n",
            "    let name = \"v\" + str(n)\n",
            "    quote code:\n",
            "        let <! name !> = 1\n",
            "\n",
            "count_up()\n",
        ))
        .expect("expand");
        assert!(matches!(&out[0], Stmt::Let(n, _, _) if n == "v7"));
    }

    /// ⚠⚠ **装飾子で関数を包む**（タスク 2-10 の本来の用途）。元の本体を別名で残し、
    /// 同名のラッパーを置く。`Code` の `+` と 1 行ずつの反復が無いと書けない。
    #[test]
    fn a_decorator_can_wrap_a_function_keeping_its_body() {
        let out = expand(concat!(
            "exprconst !fn logged(m) -> None:\n",
            "    let impl_name = m.name + \"_impl\"\n",
            "    mut first = True\n",
            "    mut rest = code:\n",
            "    for line in m.code():\n",
            "        if first:\n",
            "            first = False\n",
            "        else:\n",
            "            rest = rest + line\n",
            "    let head = code:\n",
            "        fn <! impl_name !>(x: int) -> int:\n",
            "    let wrapper = code:\n",
            "        fn <! m.name !>(x: int) -> int:\n",
            "            return <! impl_name !>(x)\n",
            "    quote head + rest + wrapper\n",
            "\n",
            "!logged\n",
            "fn double(x: int) -> int:\n",
            "    let y = x * 2\n",
            "    return y\n",
        ))
        .expect("expand");
        let fns: Vec<(&str, usize)> = out
            .iter()
            .filter_map(|s| match s {
                Stmt::FnDef { name, body, .. } => Some((name.as_str(), body.len())),
                _ => None,
            })
            .collect();
        assert_eq!(
            fns,
            vec![("double_impl", 2), ("double", 1)],
            "元の本体（2 文）が別名で残り、同名のラッパーが置かれること"
        );
    }

    /// `.indent()` はメソッド形で `+` より強く結合する（§1.2）。`if` の中へ入れられる。
    #[test]
    fn indent_nests_a_fragment_under_another() {
        let out = expand(concat!(
            "exprconst !fn guarded() -> None:\n",
            "    let body = code:\n",
            "        let inside = 1\n",
            "    let guard = code:\n",
            "        if True:\n",
            "    quote guard + body.indent()\n",
            "\n",
            "guarded()\n",
        ))
        .expect("expand");
        let Stmt::If { .. } = &out[0] else { panic!("`if` が置かれていない") };
    }

    /// ⚠⚠ **0 未満になる行が生じる `.dedent()` はエラー**（§1.2）。
    /// 黙って 0 で止めると、入れ子の関係が崩れた断片が「それらしく」置かれる。
    #[test]
    fn dedent_below_the_start_is_an_error() {
        let err = expand(concat!(
            "exprconst !fn bad() -> None:\n",
            "    let c = code:\n",
            "        print(1)\n",
            "    quote c.dedent()\n",
            "\n",
            "bad()\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("left of its start"), "実際のエラー: {err}");
    }

    /// ⚠⚠ `.code()` は**本体ごと**元のコードを返す（設計書 §1.5 / タスク 4-7）。
    /// 本体が黙って落ちる `.code()` は最悪の失敗形（`stub_gen` を流用しなかった理由）。
    #[test]
    fn code_returns_the_whole_declaration_including_its_body() {
        let out = expand(concat!(
            "exprconst !fn duplicate(m) -> None:\n",
            "    quote m.code()\n",
            "\n",
            "!duplicate\n",
            "fn triple(x: int) -> int:\n",
            "    let y = x * 3\n",
            "    return y\n",
        ))
        .expect("expand");
        let Stmt::FnDef { name, body, .. } = &out[0] else { panic!("FnDef を期待") };
        assert_eq!(name, "triple");
        assert_eq!(body.len(), 2, "本体の 2 文が残っていること");
    }

    /// `.declared_at` は宣言が書かれた位置（参考B #7）。
    #[test]
    fn declared_at_points_at_the_declaration() {
        let out = expand(concat!(
            "\n",
            "\n",
            "fn area(w: int) -> int:\n",   // 3 行目
            "    return w\n",
            "\n",
            "exprconst !fn where_is(m) -> None:\n",
            "    let loc = m.declared_at\n",
            "    let n = \"line_\" + str(loc.line)\n",
            "    quote code:\n",
            "        let <! n !> = 1\n",
            "\n",
            "where_is(^area)\n",
        ))
        .expect("expand");
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "line_3")),
            "3 行目を指していない"
        );
    }

    /// ⚠ 合成された宣言（自動 `__init__` など）には元のコードが無い。**そう言う**。
    /// 空の `Code` を返すと「本体が空の関数」と読めてしまう。
    #[test]
    fn code_of_a_generated_declaration_says_so() {
        let err = expand(concat!(
            "class Point:\n",
            "    mut x: int\n",
            "\n",
            "exprconst !fn show(m) -> None:\n",
            "    let init = m.methods[0]\n",
            "    let c = init.code()\n",
            "    quote code:\n",
            "\n",
            "show(^Point)\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("was generated rather than written in source"), "実際のエラー: {err}");
    }

    /// 展開時型推論（設計書 D15 / タスク 4-4）。注釈の無い束縛でも `.type` が答えを持つ。
    /// ⚠ 新しい推論器は書いていない。型検査器を「ここまでの文」にかけて束縛の型を読むだけ。
    #[test]
    fn an_unannotated_binding_has_an_inferred_type() {
        let out = expand(concat!(
            "class Point:\n",
            "    mut x: int\n",
            "\n",
            "let origin = Point(0)\n",
            "\n",
            "exprconst !fn same_type(m) -> None:\n",
            "    let t = m.type\n",
            "    quote code:\n",
            "        let <! t !> = 1\n",
            "\n",
            "same_type(^origin)\n",
        ))
        .expect("expand");
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "Point")),
            "`origin` の型が `Point` と推論されていない"
        );
    }

    /// ⚠ 装飾子の対象になった束縛は**まだ展開済みの文に入っていない**。
    /// ⇒ 推論の前提に足して推論する（足さないと「その名前は無い」になる）。
    #[test]
    fn a_decorated_binding_has_an_inferred_type_too() {
        let out = expand(concat!(
            "exprconst !fn typed_copy(m) -> None:\n",
            "    let t = m.type\n",
            "    quote code:\n",
            "        let copy_of: <! t !> = 0\n",
            "\n",
            "!typed_copy\n",
            "let source = 42\n",
        ))
        .expect("expand");
        let Some(Stmt::Let(_, Some(t), _)) = out.first() else {
            panic!("注釈付きの `let` が置かれていない")
        };
        assert_eq!(t, "int");
    }

    /// ⚠⚠ 推論できなければ**エラー**。`unknown` のような文字列を型名として返すと、
    /// スプライスされたときに壊れた型注釈が黙って通る。
    #[test]
    fn an_uninferable_binding_type_is_an_error() {
        let err = expand(concat!(
            "exprconst !fn show(m) -> None:\n",
            "    let t = m.type\n",
            "    quote code:\n",
            "\n",
            "let mystery = unknown_thing\n",
            "show(^mystery)\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("could not be inferred at expansion time"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠⚠ スプライスした `str` は**識別子として正しくなければならない**（タスク 4-3）。
    /// 確かめないと空白入りの変数が黙って作られる（`"a b"` で実際に完走した）。
    #[test]
    fn a_spliced_str_that_is_not_an_identifier_is_rejected() {
        for bad in ["a b", "list[int]", "if", "1x", ""] {
            let src = format!(
                concat!(
                    "exprconst !fn bind(n: str) -> None:\n",
                    "    quote code:\n",
                    "        let <! n !> = 1\n",
                    "\n",
                    "bind({:?})\n",
                ),
                bad
            );
            let err = expand(&src).expect_err(bad);
            assert!(
                err.contains("is not a single identifier"),
                "{bad:?} が通ってしまった: {err}"
            );
        }
    }

    /// ⚠ 正しい識別子は今まで通り通る（上の検査が厳しすぎないことの対照）。
    #[test]
    fn a_spliced_str_that_is_an_identifier_is_accepted() {
        for good in ["answer", "_private", "Point", "x1"] {
            let src = format!(
                concat!(
                    "exprconst !fn bind(n: str) -> None:\n",
                    "    quote code:\n",
                    "        let <! n !> = 1\n",
                    "\n",
                    "bind({:?})\n",
                ),
                good
            );
            let out = expand(&src).unwrap_or_else(|e| panic!("{good:?} が弾かれた: {e}"));
            assert!(
                matches!(&out[0], Stmt::Let(n, _, _) if n == good),
                "{good:?} で束縛されていない"
            );
        }
    }

    /// ⚠⚠ 置いたコードがさらにメタ関数を呼ぶ形では、失敗した地点だけ見ても
    /// **なぜそこにその呼び出しがあるのか**が追えない（タスク 3-3）。
    /// ⇒ 出所の列（どのメタ関数がどこから置いたか）を添える。
    #[test]
    fn a_nested_expansion_reports_where_it_came_from() {
        let err = expand(concat!(
            "exprconst !fn inner() -> None:\n",
            "    compile_error(\"inner refuses\")\n",
            "\n",
            "exprconst !fn outer() -> None:\n",
            "    quote code:\n",
            "        inner()\n",
            "\n",
            "outer()\n",
        ))
        .expect_err("止まること");
        assert!(err.contains("inner refuses"), "実際のエラー: {err}");
        assert!(
            err.contains("while expanding 'inner'"),
            "内側の出所が無い: {err}"
        );
        assert!(
            err.contains("while expanding 'outer'"),
            "外側の出所が無い —— 「なぜそこに inner() があるのか」が追えない: {err}"
        );
    }

    /// `compile_error("...")` は**利用者が意図して展開を止める**道具（タスク 3-2）。
    /// ⚠ 出るのは**書いた文面だけ**。traceback を被せると肝心の一文が埋もれる。
    #[test]
    fn compile_error_reports_only_the_authors_message() {
        let err = expand(concat!(
            "exprconst !fn need_positive(n: int) -> None:\n",
            "    if n <= 0:\n",
            "        compile_error(\"need_positive requires a positive literal\")\n",
            "    quote code:\n",
            "        print(1)\n",
            "\n",
            "need_positive(0)\n",
        ))
        .expect_err("止まること");
        assert!(
            err.contains("need_positive requires a positive literal"),
            "実際のエラー: {err}"
        );
        assert!(!err.contains("Traceback"), "traceback が被さっている: {err}");
        assert!(!err.contains("MetaAbort"), "内部の例外名が漏れている: {err}");
    }

    /// ⚠ 条件を満たすときは**普通に置く**（`compile_error` が常に止めてしまわないこと）。
    #[test]
    fn compile_error_does_not_fire_when_the_check_passes() {
        let out = expand(concat!(
            "exprconst !fn need_positive(n: int) -> None:\n",
            "    if n <= 0:\n",
            "        compile_error(\"nope\")\n",
            "    quote code:\n",
            "        let accepted = 1\n",
            "\n",
            "need_positive(5)\n",
        ))
        .expect("expand");
        assert!(matches!(&out[0], Stmt::Let(n, _, _) if n == "accepted"));
    }

    /// ⚠⚠ 同じメタ関数を 2 回呼ぶと、置いたコードの局所名がぶつかる（タスク 3-1）。
    /// `gensym` が毎回違う名前を返すことで初めて「置ける道具」になる。
    #[test]
    fn gensym_keeps_placed_local_names_apart() {
        let out = expand(concat!(
            "exprconst !fn twice() -> None:\n",
            "    quote code:\n",
            "        let <! gensym(\"tmp\") !> = 1\n",
            "\n",
            "twice()\n",
            "twice()\n",
        ))
        .expect("expand");
        let names: Vec<&str> = out
            .iter()
            .filter_map(|s| match s {
                Stmt::Let(n, _, _) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names.len(), 2, "2 本置かれること");
        assert_ne!(names[0], names[1], "名前が違うこと: {names:?}");
        assert!(names.iter().all(|n| n.starts_with("tmp__")), "接頭辞が付くこと: {names:?}");
    }

    /// ⚠ `gensym` を使わなければ**ぶつかる**（上のテストが何を守っているかの対照）。
    /// ⚠ ぶつかり方は**静的型検査の二重宣言**なので、展開そのものは通る。
    #[test]
    fn placing_the_same_local_name_twice_collides() {
        let out = expand(concat!(
            "exprconst !fn twice() -> None:\n",
            "    quote code:\n",
            "        let tmp = 1\n",
            "\n",
            "twice()\n",
            "twice()\n",
        ))
        .expect("展開自体は通る");
        let names: Vec<&str> = out
            .iter()
            .filter_map(|s| match s {
                Stmt::Let(n, _, _) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["tmp", "tmp"], "同じ名前が 2 本置かれる（この後 型検査が弾く）");
    }

    /// `const` は**展開時にも値が確定している**ので読める（設計書 §1.7 / タスク 3-6）。
    /// ⚠ これが無いと「展開時に分岐する」用途がほぼ書けない（仮引数しか見られない）。
    #[test]
    fn a_const_is_readable_while_expanding() {
        let out = expand(concat!(
            "const N = 3\n",
            "\n",
            "exprconst !fn pick() -> None:\n",
            "    if N > 1:\n",
            "        quote code:\n",
            "            let big = 1\n",
            "    quote code:\n",
            "        let small = 1\n",
            "\n",
            "pick()\n",
        ))
        .expect("expand");
        let names: Vec<&str> = out
            .iter()
            .filter_map(|s| match s {
                Stmt::Let(n, _, _) | Stmt::Const(n, _, _) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["N", "big"], "`const` を見て分岐し、`const` 自体は AST に残る");
    }

    /// ⚠ 初期化子が実行時の値に依存する `const` は展開時に評価できない。
    /// その `const` が展開時に使えないだけで**プログラムとしては正しい**ので、
    /// 展開を止めず、読まれたときに 3-0 の診断を出す。
    #[test]
    fn a_const_that_needs_run_time_values_is_not_available_while_expanding() {
        let err = expand(concat!(
            "fn at_run_time() -> int:\n",
            "    return 7\n",
            "\n",
            "const LATE = at_run_time()\n",
            "\n",
            "exprconst !fn pick() -> None:\n",
            "    if LATE > 1:\n",
            "        quote code:\n",
            "            let big = 1\n",
            "    quote code:\n",
            "\n",
            "pick()\n",
        ))
        .expect_err("読めないこと");
        assert!(err.contains("is declared at run time"), "実際のエラー: {err}");
    }

    /// `^名前` が**自分より前の宣言**からメタ情報を作ること（設計書 §1.5 / タスク 4-0）。
    #[test]
    fn caret_builds_meta_info_from_an_earlier_declaration() {
        let out = expand(concat!(
            "class Point:\n",
            "    mut x: int\n",
            "\n",
            "exprconst !fn describe(m) -> None:\n",
            "    quote code:\n",
            "        let <! m.name !> = 1\n",
            "\n",
            "describe(^Point)\n",
        ))
        .expect("expand");
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "Point")),
            "`.name` が射影されていない: {:?}",
            out.iter().map(crate::interpreter::tw_stats::stmt_kind_of).collect::<Vec<_>>()
        );
    }

    /// クラスの射影（参考B #2 #4 #5・タスク 4-1）。
    /// ⚠⚠ フィールド名は `decl_names` では取れない（あちらはスコープの名前を答える
    /// モジュールで、クラスのフィールドは意図的に報告しない）。混ぜると `.name` が
    /// **空文字列になる**（実測）。⇒ `MetaValue::name_of` を使う。
    #[test]
    fn class_projections_read_the_declaration() {
        let out = expand(concat!(
            "class Point:\n",
            "    mut x: int\n",
            "    let label: str\n",
            "\n",
            "    fn greet(self) -> str:\n",
            "        return \"hi\"\n",
            "\n",
            "exprconst !fn report(m) -> None:\n",
            "    let first = m.fields[0].name\n",
            "    let ok = m.has_method(\"greet\")\n",
            "    if ok:\n",
            "        quote code:\n",
            "            let <! first !> = 1\n",
            "    quote code:\n",
            "\n",
            "report(^Point)\n",
        ))
        .expect("expand");
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "x")),
            "`.fields[0].name` と `.has_method` が効いていない"
        );
    }

    /// 関数の射影（参考B #1）。⚠ 仮引数の記録は name / type / mutable / has_default / variadic。
    #[test]
    fn function_projections_read_the_signature() {
        let out = expand(concat!(
            "fn greet(mut who: str, times: int = 2) -> str:\n",
            "    return who\n",
            "\n",
            "exprconst !fn report(m) -> None:\n",
            "    let n = m.params[0].name + \"_\" + m.return_type\n",
            "    quote code:\n",
            "        let <! n !> = 1\n",
            "\n",
            "report(^greet)\n",
        ))
        .expect("expand");
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "who_str")),
            "仮引数名と戻り値型が取れていない"
        );
    }

    /// ⚠⚠ 種別と噛み合わない射影は**エラー**（タスク 4-1）。空のリストを返すと
    /// 「フィールドが 0 個」と読めてしまい、取り違えが通ってしまう。
    #[test]
    fn a_projection_that_does_not_fit_the_kind_is_an_error() {
        let err = expand(concat!(
            "fn greet() -> str:\n",
            "    return \"hi\"\n",
            "\n",
            "exprconst !fn wrong(m) -> None:\n",
            "    let f = m.fields\n",
            "    quote code:\n",
            "\n",
            "wrong(^greet)\n",
        ))
        .expect_err("弾かれること");
        // ⚠ **どの射影か**まで言う（タスク 3-9）。以前は「this projection」とだけ出て、
        //   メタ関数の中に射影が幾つもあると、どれが間違いか分からなかった。
        assert!(
            err.contains("`.fields` needs a class or trait, but 'greet' is a meta_function"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠ 展開は前方のみ（§1.7）。**後ろの宣言には `^` を当てられない**。
    /// ⚠ 「まだ宣言されていない」と「メタ情報を取れない種類」を言い分ける。
    #[test]
    fn caret_on_a_later_declaration_says_it_is_not_visible_yet() {
        let err = expand(concat!(
            "exprconst !fn d(m) -> None:\n",
            "    quote code:\n",
            "\n",
            "d(^Later)\n",
            "\n",
            "class Later:\n",
            "    mut a: int\n",
        ))
        .expect_err("見えないこと");
        assert!(
            err.contains("refers to nothing declared before this point"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠ 綴り間違いの射影は**黙って `None` を返さない**。通してしまうと気づかれない。
    #[test]
    fn an_unknown_projection_is_an_error() {
        let err = expand(concat!(
            "class C:\n",
            "    mut a: int\n",
            "\n",
            "exprconst !fn d(m) -> None:\n",
            "    let x = m.typo\n",
            "    quote code:\n",
            "\n",
            "d(^C)\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("has no projection"), "実際のエラー: {err}");
    }

    /// 装飾子の第一引数は**メタ情報**（タスク 4-0）。`parse_ar` の `Namespace` 木ではない。
    #[test]
    fn a_decorator_receives_meta_information() {
        let out = expand(concat!(
            "exprconst !fn rename(m) -> None:\n",
            "    quote code:\n",
            "        let <! m.kind !> = 1\n",
            "\n",
            "!rename\n",
            "class Widget:\n",
            "    mut w: int\n",
        ))
        .expect("expand");
        assert!(
            out.iter().any(|s| matches!(s, Stmt::Let(n, _, _) if n == "meta_class")),
            "装飾子が受け取ったのはメタ情報ではない"
        );
    }

    /// ⚠⚠ 実行時の値を読もうとしたら**そう言う**（設計書 §1.8 / タスク 3-0）。
    /// 素の `NameError` のままだと「定義したのに無いと言われた」と読めてしまう。
    #[test]
    fn reading_a_run_time_value_says_so() {
        let err = expand(concat!(
            "let runtime_v = 5\n",
            "\n",
            "exprconst !fn choose() -> None:\n",
            "    if runtime_v > 3:\n",
            "        quote code:\n",
            "            print(1)\n",
            "    quote code:\n",
            "\n",
            "choose()\n",
        ))
        .expect_err("弾かれること");
        assert!(
            err.contains("is declared at run time"),
            "実際のエラー: {err}"
        );
    }

    /// ⚠ 実行時の宣言**ではない**名前は言い直さない（元の `NameError` をそのまま出す）。
    /// 誤検知で正しい診断を隠さないこと。
    #[test]
    fn an_unknown_name_keeps_its_original_error() {
        let err = expand(concat!(
            "exprconst !fn choose() -> None:\n",
            "    if never_declared > 3:\n",
            "        quote code:\n",
            "            print(1)\n",
            "    quote code:\n",
            "\n",
            "choose()\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("NameError"), "実際のエラー: {err}");
        assert!(!err.contains("is declared at run time"), "実際のエラー: {err}");
    }

    /// ⚠⚠ メタ関数が無限再帰したときの診断が**読める形で外へ出る**こと（タスク 2-6）。
    /// インタプリタは例外を**番兵文字列**で返すので、そのまま出すと
    /// `MetaError: while expanding 'go':  __raise__` という何も分からない診断になる（実測）。
    /// ⚠ **大きめのスタックを持つスレッドで走らせる。** インタプリタの再帰上限は 1000 段で、
    /// debug ビルドの 1 フレームは release より厚い。既定のテストスレッド（2 MiB）だと
    /// **上限が発火する前に Rust 側のスタックが溢れる**（実測: STATUS_STACK_OVERFLOW）。
    /// ⚠ これは 2-6 が持ち込んだ性質ではなく、debug ビルドのインタプリタ全体の性質。
    #[test]
    fn a_recursive_metafn_reports_a_readable_error() {
        let handle = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                expand(concat!(
                    "exprconst fn rec(n: int) -> Code:\n",
                    "    return rec(n + 1)\n",
                    "\n",
                    "exprconst !fn go() -> None:\n",
                    "    quote rec(0)\n",
                    "\n",
                    "go()\n",
                ))
                // ⚠ `Vec<Stmt>` は `Rc` を含むのでスレッドを跨げない。見たいのはエラー文字列だけ。
                .map(|_| ())
            })
            .expect("spawn");
        let err = handle.join().expect("join").expect_err("止まること");
        assert!(err.contains("RecursionError"), "実際のエラー: {err}");
        assert!(!err.contains("__raise__"), "番兵がそのまま漏れている: {err}");
    }

    /// ⚠ 自分自身を置き続けるメタ関数は止まらない。⇒ 歩数上限で切る（本番の診断は 2-6）。
    #[test]
    fn a_self_placing_metafn_is_stopped_by_the_step_budget() {
        let err = expand(
            "exprconst !fn loop_forever() -> None:\n    quote code:\n        loop_forever()\n\nloop_forever()\n",
        )
        .expect_err("止まること");
        assert!(err.contains("did not finish within"), "実際のエラー: {err}");
    }

    /// `!装飾子` が対象の宣言を置き換える（設計書 §1.6）。
    #[test]
    fn a_decorator_replaces_its_target() {
        let out = expand(
            "exprconst !fn swap(target) -> None:\n    quote code:\n        let swapped = 1\n\n!swap\nlet original = 0\n",
        )
        .expect("expand");
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], Stmt::Let(name, _, _) if name == "swapped"));
    }

    /// ⚠ 自分より**前**に宣言されたメタ関数しか見えない（§1.7 の逐次展開）。
    /// ⚠⚠ 後ろにあるものを呼んだら**展開時のエラー**（前方参照のみ・2026-09-25 確定・タスク 2-11）。
    /// 以前は展開されずに残り、実行時の `NameError` になっていた。
    #[test]
    fn a_metafn_declared_later_is_not_visible() {
        let err = expand(
            "later()\nexprconst !fn later() -> None:\n    quote code:\n        let x = 1\n",
        )
        .expect_err("弾かれること");
        assert!(err.contains("is called before it is declared"), "実際のエラー: {err}");
    }

    /// ⚠⚠ 置いたコードの node-id は**本体と衝突してはならない**（設計書 §0.3 / タスク 2-1）。
    /// 衝突すると型検査の注釈が**別のノードのもの**になる（per-module 採番で FFI 境界検査の
    /// 誤検知が実際に再現した）。⇒ 展開器は本体と同じカウンタから採番する。
    #[test]
    fn placed_nodes_do_not_reuse_node_ids() {
        let src = concat!(
            "exprconst !fn place_it() -> None:\n",
            "    quote code:\n",
            "        let placed = other\n",
            "\n",
            "let other = 1\n",
            "let a = other\n",
            "let b = other\n",
            "let c = other\n",
            "place_it()\n",
        );
        let (stmts, counter, traits) = parse(src).expect("parse");
        // 本体をパースし終えた時点の採番位置。置いたコードはこれより**後ろ**から採番される。
        let before = counter.get();
        assert!(before > 0, "本体が node-id を採番していること");
        let out = expand_program(stmts, std::rc::Rc::clone(&counter), traits).expect("expand");

        let mut ids: Vec<u32> = Vec::new();
        collect_node_ids(&out, &mut ids);
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "node-id が重複している: {ids:?}");
        assert!(!ids.contains(&0), "未採番（0）が混ざっている: {ids:?}");
        // ⚠⚠ **これが本命の検査**。カウンタを共有していなければ置いたコードは 1 から
        //   採番され、`before` を超える id は 1 つも出ない（＝ここで落ちる）。
        assert!(
            ids.iter().any(|id| *id > before),
            "置いたコードが本体と同じカウンタから採番されていない (before={before}, ids={ids:?})"
        );
    }

    /// テスト用: 文の列に現れる `Expr::Ident` の node-id を集める。
    fn collect_node_ids(stmts: &[Stmt], out: &mut Vec<u32>) {
        for st in stmts {
            crate::stmt_walk::each_subpart(st, &mut |part| {
                if let crate::stmt_walk::StmtPart::Expr(e) = part {
                    collect_expr_ids(e, out);
                }
            });

        }
    }

    fn collect_expr_ids(e: &Expr, out: &mut Vec<u32>) {
        if let Expr::Ident { node_id, .. } = e {
            out.push(*node_id);
        }
        crate::expr_walk::each_subpart(e, &mut |sub| {
            if let crate::expr_walk::SubPart::Plain(x) | crate::expr_walk::SubPart::Control(x) = sub
            {
                collect_expr_ids(x, out);
            }
        });
    }

    /// ⚠⚠ **スプライスで差し込んだトークンと、組み直した `Indent`/`Newline` は**
    /// **元の位置を持たない**（値やブロック構造には位置が無い）。⇒ 行の位置を代わりに使う
    /// （タスク 2-3）。位置不明のまま出すと「どこで間違えたのか分からない」診断になる。
    ///
    /// ⚠ 地の文のトークンは**もともと自分の `Span` を持っている**ので、そちらを見る検査は
    /// 2-3 の有無に関わらず通ってしまう。⇒ ここでは**差し込んだトークン**を直接見る。
    #[test]
    fn spliced_tokens_carry_the_line_position() {
        let src = concat!(
            "exprconst fn f(n) -> Code:\n",      // 1 行目
            "    let a = code:\n",               // 2 行目
            "        let <! n !> = 1\n",         // 3 行目 ← ここを指してほしい
            "    return a\n",
        );
        let (stmts, _, _) = parse(src).expect("parse");
        let Stmt::MetaFnDef { body, .. } = &stmts[0] else { panic!("MetaFnDef を期待") };
        let Stmt::Let(_, _, Expr::CodeBlock(lines)) = &body[0] else { panic!("CodeBlock を期待") };
        assert_eq!(lines[0].span.line, 3, "行の位置が記録されていること");

        let filled = fill_splices(lines, &[Value::Str("chosen".into())]).expect("fill");
        let spliced = filled[0]
            .pieces
            .iter()
            .filter_map(|p| match p {
                CodePiece::Token(t) => Some(t),
                CodePiece::Splice(_) => None,
            })
            .find(|t| matches!(&t.token, crate::token::Token::Ident(n) if n == "chosen"))
            .expect("差し込んだ識別子があること");
        assert_eq!(spliced.span.line, 3, "差し込んだトークンが行の位置を持つこと");
    }

    /// 置いたコードのノードが元の位置を保つこと（地の文のトークン由来）。
    #[test]
    fn placed_nodes_point_back_at_the_code_block_line() {
        let src = concat!(
            "exprconst !fn place_it() -> None:\n",   // 1 行目
            "    quote code:\n",                     // 2 行目
            "        missing_name()\n",              // 3 行目 ← ここを指してほしい
            "\n",
            "place_it()\n",                          // 5 行目
        );
        let out = expand(src).expect("expand");
        assert_eq!(out.len(), 1);
        let Stmt::Expr(Expr::Call { span, .. }) = &out[0] else { panic!("Call を期待") };
        assert_eq!(span.line, 3, "`code:` の中身が書かれていた行を指すこと");
    }

    /// クラス本体のメンバー装飾子もその場で展開される。
    /// ⚠ ここを歩かないと、クラスのメンバーに付けた装飾子が**黙って無視される**。
    #[test]
    fn decorators_inside_a_class_body_are_expanded() {
        let out = expand(
            "exprconst !fn tag(target) -> None:\n    quote code:\n        mut tagged: int\n\nclass C:\n    !tag\n    mut original: int\n",
        )
        .expect("expand");
        let Stmt::ClassDef { body, .. } = &out[0] else { panic!("ClassDef を期待") };
        let fields: Vec<&str> = body
            .iter()
            .filter_map(|s| match s {
                Stmt::Field { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(fields, vec!["tagged"], "装飾された宣言が置き換わっていること");
    }

    /// ⚠⚠ クラス本体へ置けるのは**メンバー宣言だけ**（タスク 2-5）。
    /// `pass` を素通しすると「装飾子が `pass` を置いてメンバーが**黙って消える**」形になる。
    /// メンバーを消したいなら空の `Code` を `quote` する（§1.2 の B-2）。
    #[test]
    fn placing_pass_in_a_class_body_is_rejected() {
        let err = expand(concat!(
            "exprconst !fn passy(t) -> None:\n",
            "    quote code:\n",
            "        pass\n",
            "\n",
            "class C:\n",
            "    mut a: int\n",
            "    !passy\n",
            "    mut b: int\n",
        ))
        .expect_err("弾かれること");
        assert!(err.contains("cannot be placed in a class or trait body"), "実際のエラー: {err}");
    }

    /// 空の `Code` を置けばメンバーは**意図して消える**（上の `pass` との対）。
    #[test]
    fn quoting_an_empty_code_removes_the_decorated_member() {
        let out = expand(concat!(
            "exprconst !fn drop_it(t) -> None:\n",
            "    let empty = code:\n",
            "    quote empty\n",
            "\n",
            "class C:\n",
            "    mut a: int\n",
            "    !drop_it\n",
            "    mut b: int\n",
        ))
        .expect("expand");
        let Stmt::ClassDef { body, .. } = &out[0] else { panic!("ClassDef を期待") };
        let fields: Vec<&str> = body
            .iter()
            .filter_map(|s| match s {
                Stmt::Field { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(fields, vec!["a"], "装飾されたメンバーだけが消えること");
    }

    /// ⚠⚠ **自動 `__init__` は展開で増えたフィールドを見ていなければならない**（タスク 2-4）。
    /// パース時に生成していた頃は、装飾子が足したフィールドが引数に入らず
    /// 「そのフィールドを渡せないクラス」ができていた（実測）。
    #[test]
    fn the_auto_init_sees_fields_added_by_a_decorator() {
        let out = expand(concat!(
            "exprconst !fn add_counter(target) -> None:\n",
            "    quote code:\n",
            "        mut hits: int\n",
            "\n",
            "class Page:\n",
            "    mut title: str\n",
            "    !add_counter\n",
            "    mut placeholder: int\n",
        ))
        .expect("expand");
        let Stmt::ClassDef { body, .. } = &out[0] else { panic!("ClassDef を期待") };
        let init = body
            .iter()
            .find_map(|s| match s {
                Stmt::FnDef { name, params, .. } if name == "__init__" => Some(params),
                _ => None,
            })
            .expect("自動 `__init__` が生成されていること");
        let names: Vec<&str> = init.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["self", "title", "hits"],
            "装飾子が足した `hits` が引数に入り、置き換えられた `placeholder` は消えていること"
        );
    }
}
