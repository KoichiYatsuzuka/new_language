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
";

/// `compile_error` が投げる例外の名前（タスク 3-2）。
///
/// ⚠ 整形済みの報告からこの名前を探して、**利用者の書いた文面だけ**を出す。
/// traceback ごと出すと「展開器の内部が漏れている」ようにしか見えない。
const ABORT_CLASS: &str = "MetaAbort";

/// 1 回の展開で歩ける文の数の上限（設計書 タスク 2-6）。
///
/// ⚠⚠ **自分自身を置き続けるメタ関数は止まらない。** 展開は前向き走査だが、置いたコードは
/// その場から歩き直す（§1.7 の「反復しない」は**前方参照の話**で、置いた分を歩くのは反復ではない）
/// ので、`quote code:` が自分の呼び出しを置けば無限に伸びる。⇒ 歩数で切る。
///
/// ⚠ **再帰深さは別の柵が受け持つ。** メタ関数がメタ関数を呼ぶのは展開器から見れば
/// 1 回の呼び出しの中の出来事で、インタプリタの `RecursionError`（上限 1000）が止める。
/// ⇒ ここで二重に数えない。代わりに、その例外が**読める形で外へ出る**ようにしてある
/// （`call_metafn` の `RAISE_SENTINEL` の扱い）。
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
    let stmts = parsed.map_err(|e| format!("MetaError: the placed `Code` does not parse: {e}"))?;
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
    /// 置いたコードを採番する node-id カウンタ（設計書 §0.3 / タスク 2-1）。
    ///
    /// ⚠⚠ **本体をパースしたパーサと同じものを渡す。** 別のカウンタだと node-id が
    /// 衝突し、型検査の注釈が**別のノードのもの**になる（per-module 採番で実際に
    /// FFI 境界検査の誤検知が再現した — `Parser::node_counter` の doc）。
    node_counter: std::rc::Rc<std::cell::Cell<u32>>,
}

impl Expander {
    fn new(
        node_counter: std::rc::Rc<std::cell::Cell<u32>>,
        known_traits: std::collections::HashMap<String, crate::parser::TraitInfo>,
    ) -> Self {
        Self {
            metafns: std::collections::HashMap::new(),
            interp: Interpreter::new(),
            steps: 0,
            last_expanded: None,
            runtime_names: std::collections::HashSet::new(),
            known_traits,
            node_counter,
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
        let tokens = crate::lexer::Lexer::new(PRELUDE, "<meta-prelude>").tokenize();
        let stmts = crate::parser::Parser::new(tokens, None)
            .parse_program()
            .map_err(|e| format!("MetaError: internal — the expander prelude does not parse: {e}"))?;
        for st in &stmts {
            self.interp
                .exec(st)
                .map_err(|e| format!("MetaError: internal — the expander prelude failed: {e}"))?;
        }
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
        let as_fn = Stmt::FnDef {
            name: name.clone(),
            template_params: Vec::new(),
            params: params.clone(),
            body: body.clone(),
            decorators: Vec::new(),
            return_type: return_type.clone(),
            is_static: false,
            is_class_method: false,
            is_abstract: false,
            access: crate::ast::Accessibility::Public,
        };
        self.interp.exec(&as_fn).map(|_| ())
    }

    /// メタ関数を呼んで `Code` を得る。
    fn call_metafn(&mut self, name: &str, args: Vec<Value>) -> Result<Rc<Vec<CodeLine>>, String> {
        let callee = self
            .interp
            .vm_load_name(name)
            .ok_or_else(|| format!("MetaError: metafunction '{name}' is not defined"))?;
        let evaled: Vec<(Option<String>, Value, bool)> =
            args.into_iter().map(|v| (None, v, false)).collect();
        self.last_expanded = Some(name.to_string());
        let out = match self.interp.call_value_evaled(callee, evaled, name, None, 0) {
            Ok(v) => v,
            // ⚠⚠ インタプリタは例外を**番兵文字列**で返し、実体は自分の中に置く。
            //    そのまま出すと `MetaError: while expanding 'go':  __raise__` という
            //    何も分からない診断になる（実測）。⇒ 本体の入口（`src/main.rs`）と
            //    同じように取り出して整形する。`RecursionError` の traceback もこれで出る。
            Err(e) if e == crate::interpreter::RAISE_SENTINEL => {
                let detail = self
                    .interp
                    .take_current_exception()
                    .map(|r| Interpreter::format_error_report(&r))
                    .unwrap_or_else(|| "(no details available)".to_string());
                // ⚠ `compile_error` は**利用者が意図して止めた**印。最優先で、文面だけ出す。
                if let Some(msg) = Self::compile_error_message(&detail) {
                    return Err(format!("MetaError: {msg} (raised by metafunction '{name}')"));
                }
                if let Some(hint) = self.runtime_name_hint(&detail) {
                    return Err(format!("MetaError: while expanding '{name}': {hint}"));
                }
                return Err(format!(
                    "MetaError: while expanding '{name}':{}{detail}",
                    '\n'
                ));
            }
            Err(e) => return Err(format!("MetaError: while expanding '{name}': {e}")),
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
    ex.load_prelude()?;
    // ⚠ 診断専用の先行走査（設計書 §1.8 / タスク 3-0）。展開の可否や結果には影響しない。
    for st in &stmts {
        // ⚠ `const` は展開時に読めるはず（設計書 §1.7）なので**実行時の名前ではない**。
        if matches!(st, Stmt::Const(..)) {
            continue;
        }
        crate::decl_names::each_declared_name(st, &mut |n, _, _| {
            ex.runtime_names.insert(n.to_string());
        });
    }
    expand_stmts(&mut ex, stmts, Context::TopLevel, std::rc::Rc::new(Vec::new()))
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
        register_decl(ex, &stmt);

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
                match ex.interp.eval(init) {
                    Ok(v) => {
                        let _ = ex.interp.vm_declare_global(
                            name,
                            crate::vm::op::DeclKind::Const,
                            &[],
                            v,
                        );
                    }
                    Err(_) => {
                        ex.runtime_names.insert(name.clone());
                    }
                }
                out.push(stmt);
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

                let vals = eval_args(ex, args).map_err(|e| e + &render_trail(&frames))?;
                let code = ex.call_metafn(&name, vals).map_err(|e| e + &render_trail(&frames))?;
                let placed = ex.place(&code, ctx).map_err(|e| e + &render_trail(&frames))?;
                // ⚠ **置いたものを先に歩く**。置いたコードがさらにメタ関数を呼ぶことはある。
                for s in placed.into_iter().rev() {
                    pending.push_front((s, std::rc::Rc::clone(&frames)));
                }
            }

            // `!装飾子` 付きの宣言 → 装飾子が返した中身で置き換える。
            Stmt::MetaDecorated { decorators, target } => {
                // ⚠ 装飾子には位置が無い（`Expr::Ident` が `Span` を持たない）。
                //   ⇒ 出所の列には名前だけを積む。位置まで欲しければ AST 側に足すことになる。
                let placed = apply_decorators(ex, decorators, *target, ctx, &here)?;
                for s in placed.into_iter().rev() {
                    pending.push_front((s, std::rc::Rc::clone(&here)));
                }
            }

            // クラス／トレイト本体もその場で展開する（メンバーの装飾子がここで消える）。
            Stmt::ClassDef {
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
                out.push(Stmt::ClassDef {
                    name,
                    template_params,
                    bases,
                    base_args,
                    body,
                    decorators,
                });
            }
            Stmt::TraitDef { name, template_params, body } => {
                let body =
                    expand_stmts(ex, body, Context::TypeBody, std::rc::Rc::clone(&here))?;
                out.push(Stmt::TraitDef { name, template_params, body });
            }

            // それ以外はそのまま。
            // ⚠ **関数本体の中は今は歩いていない**（§1.7 はファイル先頭からの逐次展開で、
            //   本体の中の展開は別途決める必要がある）。⇒ 未対応として記録済み。
            other => out.push(other),
        }
    }
    Ok(out)
}

/// 展開時に見える宣言として登録する（タスク 4-0）。
///
/// ⚠ メタ情報の対象になる種類だけを入れる（`MetaValue::kind_of` が決める）。
/// ⚠ `!装飾子` に包まれた宣言は**中身**を登録する（包みには名前が無い）。
fn register_decl(ex: &mut Expander, stmt: &Stmt) {
    let inner = match stmt {
        Stmt::MetaDecorated { target, .. } => &**target,
        other => other,
    };
    if crate::interpreter::value::MetaValue::kind_of(inner).is_none() {
        return;
    }
    let decl = std::rc::Rc::new(inner.clone());
    crate::decl_names::each_declared_name(inner, &mut |name, _, _| {
        ex.interp.add_meta_decl(name.to_string(), std::rc::Rc::clone(&decl));
    });
}

/// 呼び出し式が「登録済みの**配置**メタ関数」ならその名前を返す。
fn placing_call_name(ex: &Expander, func: &Expr) -> Option<String> {
    let Expr::Ident { name, .. } = func else { return None };
    match ex.metafns.get(name) {
        Some(true) => Some(name.clone()),
        _ => None,
    }
}

/// 宣言そのものからメタ情報の値を作る（タスク 4-0）。
///
/// ⚠ 装飾子の第一引数に使う。`^名前` と違って**表を引かない**（対象はもう手元にある）。
fn meta_value_for(decl: &Stmt) -> Option<Value> {
    let kind = crate::interpreter::value::MetaValue::kind_of(decl)?;
    // ⚠ `decl_names` ではなく `name_of`。フィールドは `decl_names` が報告しない
    //   （スコープの名前ではないため）ので、混ぜると名前が空になる。
    let name = crate::interpreter::value::MetaValue::name_of(decl)?;
    Some(Value::Meta(std::rc::Rc::new(
        crate::interpreter::value::MetaValue { kind, name, decl: std::rc::Rc::new(decl.clone()) },
    )))
}

/// 実引数を展開時に評価する。
///
/// ⚠ 展開時に確定しないもの（実行時の変数など）はここで失敗する。
/// 「何が確定するか」の診断は 3-0（§1.8）。
fn eval_args(ex: &mut Expander, args: &[crate::ast::CallArg]) -> Result<Vec<Value>, String> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        match a {
            crate::ast::CallArg::Positional(e) => out.push(ex.interp.eval(e)?),
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
    target: Stmt,
    ctx: Context,
    trail: &std::rc::Rc<Vec<Frame>>,
) -> Result<Vec<Stmt>, String> {
    let mut current = vec![target];
    for deco in decorators.into_iter().rev() {
        let Expr::Ident { ref name, .. } = deco else {
            return Err(
                "MetaError: only a plain decorator name is supported so far \
                 (arguments and closures come later)"
                    .to_string(),
            )
        };
        if !ex.metafns.contains_key(name) {
            return Err(format!(
                "MetaError: decorator '!{name}' is not a metafunction declared before this point"
            ));
        }
        if current.len() != 1 {
            return Err(format!(
                "MetaError: decorator '!{name}' cannot be applied — the inner decorator produced \
                 {} statements, and a decorator takes exactly one declaration",
                current.len()
            ));
        }
        // ⚠ 装飾子には**メタ情報**を渡す（タスク 4-0）。`parse_ar` の `Namespace` 木ではない。
        //   名前が無い宣言（`pass` など）は対象外なので、そこは弾かれている。
        let arg = meta_value_for(&current[0]).ok_or_else(|| {
            format!(
                "MetaError: decorator '!{name}' cannot be applied to this declaration \
                 (meta information exists for classes, traits, enums, functions, fields \
                 and variable bindings)"
            )
        })?;
        let mut frames = (**trail).clone();
        if frames.len() < MAX_TRAIL {
            frames.push(Frame { name: name.clone(), span: Span::unknown() });
        }
        let code = ex
            .call_metafn(name, vec![arg])
            .map_err(|e| e + &render_trail(&frames))?;
        current = ex.place(&code, ctx).map_err(|e| e + &render_trail(&frames))?;
    }
    Ok(current)
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
        assert!(
            err.contains("this projection needs a class or trait"),
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
    /// 後ろにあるものを呼んでも展開されず、そのまま残る（実行時に NameError になる）。
    #[test]
    fn a_metafn_declared_later_is_not_visible() {
        let kinds = expand_kinds(
            "later()\nexprconst !fn later() -> None:\n    quote code:\n        let x = 1\n",
        )
        .expect("expand");
        assert_eq!(kinds, vec!["Expr"], "呼び出し文が展開されずに残る");
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
