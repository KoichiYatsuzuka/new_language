// vm/run.rs — バイトコード VM のディスパッチループ（Phase V, V-A）。
//
// 値スタックは呼び出し側が持つ単一バッファ（`buf`）を共有し、per-call 確保を避ける。
// フレームローカルは `buf[base .. base+n_locals]`、オペランドスタックはその上（`buf[base+n_locals ..]`）。
//
// 算術・比較は int/float の高速パスを VM ループ内にインライン展開し、それ以外
// （文字列・インスタンス演算子・混在型・ゼロ除算など）は既存の `apply_binop_dyn` へ委譲する。
// 高速パスは `apply_binop` の該当アームと**同一のセマンティクス**（`a + b` 等、オーバーフロー
// 挙動も含め）で書く。属性読み・真偽判定・単項も既存実装へ委譲するので結果はツリーウォークと一致する。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::BinOp;
use crate::interpreter::{Interpreter, Value};
use crate::interpreter::BindCopy;

use super::chunk::Chunk;
use super::op::Op;

mod exec_op;
mod scalar_ops;
mod op_helpers;

use self::exec_op::*;
use self::scalar_ops::*;
use self::op_helpers::*;

/// 1 命令実行後の制御フロー。
enum Flow {
    /// 次の命令へ（ip += 1）。
    Next,
    /// 次の命令へ（ip += 1）。**ただし直前に他のコードを実行した**（呼び出し系 op）。
    ///
    /// `Next` と分けているのは、呼び出しの中で `break_point` が発火して
    /// **デバッグセッションが始まっている可能性がある**のがここだけだから（#1）。
    /// このフレームは既に走り始めているので `run` 入口の判定を通過済みで、
    /// ここで拾わないと停止判定を持たないまま最後まで走ってしまう。
    /// 呼び出しは元々数百 ns かかるので、この再判定のコストは無視できる。
    NextAfterCall,
    /// 絶対 index へジャンプ。
    Jump(usize),
    /// `yield` で**中断**する（bug_fix.md B13）。産出値を持つ。
    ///
    /// ⚠ これが出るのはジェネレータ本体のチャンクだけ。段階 A で
    /// 「`yield` は `gen` 本体の直下だけ」を静的に保証してあるので、
    /// **中断は `run_dispatch` を 1 回抜けるだけ**で済む。
    Suspend(Value),
    /// 関数から値を返す。
    Return(Value),
}

/// フレームのセル表を作る（#27-d 段階 2b）。
///
/// - **可変キャプチャ**（`chunk.captured_cells`）は `captured_env` のセルを**そのまま入れる**
///   （`Rc` を clone するだけ＝外側と同じセルを指す）。ツリーウォークが `Var::Cell` を
///   共有するのと同じ効果。
/// - それ以外の index は**呼び出しごとに新しいセル**（入れ子 `fn` に可変キャプチャされる
///   自分のローカル用。呼び出しごとに別のセルになるのもツリーウォークと同じ）。
///
/// ⚠ `n_cells == 0`（大多数の関数）では `Vec::new()` を返すので**確保は起きない**。
#[inline]
fn build_cells(
    chunk: &Chunk,
    captured_env: Option<&std::collections::HashMap<String, crate::interpreter::CapturedVar>>,
) -> Vec<Rc<RefCell<Value>>> {
    if chunk.n_cells == 0 {
        return Vec::new();
    }
    let mut cells: Vec<Rc<RefCell<Value>>> = (0..chunk.n_cells)
        .map(|_| Rc::new(RefCell::new(Value::None)))
        .collect();
    if let Some(env) = captured_env {
        for (name, idx) in &chunk.captured_cells {
            if let Some(crate::interpreter::CapturedVar::Mutable(cell)) = env.get(name) {
                if let Some(slot) = cells.get_mut(*idx as usize) {
                    *slot = cell.clone(); // ⚠ 中身ではなく **Rc を共有**する
                }
            }
        }
    }
    cells
}

/// アクティブな例外ハンドラ（try 節ごとに VM のハンドラスタックに積む）。
pub(crate) struct Handler {
    /// 例外発生時に飛ぶ landing pad の ip。
    handler_ip: usize,
    /// try 進入時のオペランドスタック深さ（例外時にここまで巻き戻す）。
    stack_len: usize,
}

/// チャンク実行の**再開に必要な状態**（bug_fix.md B13 段階 B）。
///
/// 以前は 3 つとも ディスパッチループ の**ローカル変数**だったので、中断して戻ってくる
/// ことが原理的にできなかった。CPython のフレーム（`f_lasti` + ブロック/例外表 +
/// cell/free）と対応する。
///
/// ⚠ オペランドスタックとローカルは `buf[base..]` にあり、**ここには入らない**。
///   中断する側（ジェネレータ）は共有スタックを使わず**自前のバッファ**を持つ
///   （CPython が `gi_iframe` に記憶域を持つのと同じ理由）。
/// ディスパッチループの抜け方（bug_fix.md B13）。
pub(crate) enum RunOutcome {
    /// 本体が最後まで走った（戻り値）。
    Done(Value),
    /// `yield` で中断した。産出値と、再開に要る状態。
    Suspended(Value, Frame),
}

pub(crate) struct Frame {
    /// 次に実行する命令の index（CPython の `f_lasti` 相当）。
    pub(crate) ip: usize,
    /// 例外ハンドラのスタック。中断中も保持しないと `try` の中で yield できない。
    pub(crate) handlers: Vec<Handler>,
    /// セル表（#27-d 段階 2b）。捕捉は既存の `Rc<RefCell<Value>>` をそのまま共有する。
    pub(crate) cells: Vec<Rc<RefCell<Value>>>,
}

impl Frame {
    /// チャンクの先頭から始めるフレームを作る（ジェネレータ生成用）。
    ///
    /// ⚠ `captured_env` を渡すと**可変キャプチャのセルを共有**する（#27-d 段階 2b と同じ機構）。
    ///   ジェネレータのクロージャ化（B13 段階 E）はこれをそのまま使う。
    pub(crate) fn new(
        chunk: &Chunk,
        captured_env: Option<&std::collections::HashMap<String, crate::interpreter::CapturedVar>>,
    ) -> Self {
        Frame { ip: 0, handlers: Vec::new(), cells: build_cells(chunk, captured_env) }
    }
}

/// チャンクを実行する。**入れ子の深さをここで数える**（bug_fix.md B10 系統）。
///
/// ⚠⚠ 以前は上限が**全く無かった**ので、深さ ~134 の再帰で
/// `thread 'main' has overflowed its stack` となり**プロセスごと落ちていた**
/// （トレースバックなし・catch 不能・exit 127）。
///
/// ⚠ **関数呼び出し側（`exec_fn_evaled`）ではなくここで数える**。
/// ジェネレータの本体は `exec_fn_evaled` を通らないので、関数側だけを守っても
/// **再帰するジェネレータが落ちたまま**だった（実測）。チャンク実行は
/// 関数・メソッド・ジェネレータ・最上位の全てが通る唯一の絞り所。
///
/// ⚠ 増減を**このラッパーだけ**で行うのは、本体（`run_frame`。旧 `run_inner` を統合済み）に早期 `return` が
/// 多く、中で増減させると**必ずどこかで戻し忘れる**ため。
pub fn run(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    captured_env: Option<&std::collections::HashMap<String, crate::interpreter::CapturedVar>>,
) -> Result<Value, String> {
    enter_depth(interp)?;
    // 実行時間分布の計測（`--features prof`）: 抜けるとき呼び出し元の op へ戻す。
    // ⚠ B13 段階 B で `run_inner` を統合（`run_inner` は削除済み）したとき一度落としてしまった。
    //   `--features prof` は既定ビルドで消えるので**コンパイラは何も言わない**——
    //   落とすと `prof_dist.ps1` の計測が黙って壊れる。
    #[cfg(feature = "prof")]
    let _cur = crate::prof::CurGuard::new();
    // ⚠ **ここで `Frame` を作らない**。`Frame` は中断の記録であって実行中の表現では無いので、
    //   新規実行は切り出し前と同じく 3 つのローカルを作って値渡しする。
    //   中断が生まれたときにだけ `Frame` を組み立てる（段階 C）。
    let r = run_dispatch(
        interp, chunk, buf, base,
        0,
        Vec::new(),
        build_cells(chunk, captured_env),
    );
    interp.call_depth -= 1;
    match r {
        Ok(RunOutcome::Done(v)) => Ok(v),
        // ⚠ ここには来ない。段階 A で「`yield` は `gen` 本体の直下だけ」を
        //   静的エラーで保証しているので、非ジェネレータのチャンクに `Yield` は出ない。
        Ok(RunOutcome::Suspended(..)) => {
            Err("RuntimeError: internal: yield outside a generator body".to_string())
        }
        Err(e) => Err(e),
    }
}

/// **中断したフレームから再開**する（B13 段階 B で用意、段階 C から使う）。
///
/// ⚠ 新規実行（[`run`]）と**同じディスパッチループ**を使う。違いは初期値だけ。
/// ⚠ この段階では呼ばれない（中断が存在しないのでフレームも生まれない）。
/// 中断中のフレームの**その位置へ例外を投げ込んで**再開する（bug_fix.md B13 段階 D）。
///
/// `close()` が `GeneratorExit` を投げ込むのに使う。囲む `try` があれば `finally` が走る。
/// ⚠ ハンドラが無ければ本体はそこで終わる（`Ok(None)` 相当）。それが正常な `close`。
pub(crate) fn resume_frame_throwing(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    mut frame: Frame,
    err: &str,
) -> Result<RunOutcome, String> {
    match unwind_to_handler(interp, buf, &mut frame.handlers, err) {
        Some(landing) => {
            frame.ip = landing;
            resume_frame(interp, chunk, buf, base, frame)
        }
        // 囲む `try` が無い＝走らせる `finally` も無い。本体はここで終わり。
        None => Ok(RunOutcome::Done(Value::None)),
    }
}

pub(crate) fn resume_frame(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    frame: Frame,
) -> Result<RunOutcome, String> {
    // ⚠⚠ **ここでも深さを数える**（bug_fix.md B10）。
    //    再帰するジェネレータは入れ子の分だけここを通るので、`run` だけ守っても
    //    **スタックが満ちてプロセスごと落ちる**。B13 の実装中に実際に再発させ、
    //    `recursion_limit.ar` §5 が捕まえた。
    enter_depth(interp)?;
    let r = run_dispatch(interp, chunk, buf, base, frame.ip, frame.handlers, frame.cells);
    interp.call_depth -= 1;
    r
}

/// 命令ディスパッチの本体。**新規実行と再開の両方がここを通る**。
///
/// ⚠ 引数を全て**値渡し**にしてあるのは、切り出し前の `run_inner`（削除済み）と**同じコード形状**
/// に保つため（既存の `run_stepping` も同じ形）。`&mut Frame` 越しに毎命令使うと
/// Vec の ptr/len をレジスタに保てなくなる。
///
/// 速度の A/B（別 worktree を cargo に直接ビルドさせて実測。負の対照 ±0.6%）:
///
/// | ベンチ | 切り出し前 → 後 |
/// |---|---|
/// | `bench_for` / `bench_arith` | 変化なし |
/// | `bench_method_call` | **約 4% 速い** |
/// | `bottleneck_bench` | **約 3.5% 遅い** |
///
/// ⚠ 両方向で一貫した符号なのでノイズではないが、**向きが逆**なので
/// 系統的なコストではなくコード配置の差と見ている。B13 全体（段階 A〜E）では
/// 全ベンチが負の対照帯内で、**正反両方向で打ち消し合った**。
///
/// ⚠⚠ **測り方の罠**: 最初の測定は「新しい方だけ exe をコピーする」形で取っていて、
/// Windows Defender が新規ファイルをスキャンする分が B 側にだけ乗り、
/// **一貫して「遅い」という偽の結果**になった（最終的には複製自体がブロックされた）。
/// ⇒ **A/B は両方を cargo に直接ビルドさせる**こと（`git worktree` + 別 target）。
/// ⚠ `Frame` は**中断したときの記録**であって、実行中の表現ではない。
/// ⚠ 中断するとき（段階 C）はここで `Frame` を**組み立ててから**抜けること。
/// 例外を最内ハンドラへ巻き戻し、landing pad の ip を返す（`None` = 伝播）。
///
/// ⚠ 通常のディスパッチと `close()`（中断点への例外注入・段階 D）で**同じ処理を使う**。
///   分けて書くと片方だけ直したときに黙ってずれる。
fn unwind_to_handler(
    interp: &mut Interpreter,
    buf: &mut Vec<Value>,
    handlers: &mut Vec<Handler>,
    e: &str,
) -> Option<usize> {
    // 最内ハンドラがあればオペランドを巻き戻して例外値を積み landing pad へ。
    // 変換できない（active exception なし等）・ハンドラなしなら伝播。
    let h = handlers.pop()?;
    buf.truncate(h.stack_len);
    let exc_val = interp.vm_take_raised(e)?;
    buf.push(exc_val);
    Some(h.handler_ip)
}

/// 入れ子の深さを検査して +1 する（`run` / `resume_frame` の共通部・bug_fix.md B10）。
///
/// ⚠ 減算は呼び出し側（`interp.call_depth -= 1`）。`Drop` ガードにすると `&mut Interpreter` を
///   握り続けることになり、その間 VM を回せない。
/// ⚠⚠ **上限に当たったときの文言はここ 1 箇所**。2 箇所に書くとずれる。
fn enter_depth(interp: &mut Interpreter) -> Result<(), String> {
    if interp.call_depth >= crate::interpreter::MAX_CALL_DEPTH {
        // ⚠ ここではまだ増やしていないので戻す必要も無い。
        return Err(format!(
            "RecursionError: maximum recursion depth exceeded (limit {})",
            crate::interpreter::MAX_CALL_DEPTH
        ));
    }
    interp.call_depth += 1;
    Ok(())
}

/// ディスパッチループの振り分け。
///
/// ⚠⚠ **振り分けは大きなループ関数の外で行う**（`#[inline(always)]`）。ループ関数は `exec_op` を
///   丸ごと展開した大きなフレームを持つので、ループ関数の中から別のループ関数を呼ぶと、再帰の
///   1 段ごとに大きなフレームが 2 つ積まれる。展開中のループ（`run_budgeted`）をそうしたら、
///   デバッグビルドでメタ関数の再帰が深さの上限に届く前にスタックがあふれた（実測）。
#[inline(always)]
fn run_dispatch(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    ip: usize,
    handlers: Vec<Handler>,
    cells: Vec<Rc<RefCell<Value>>>,
) -> Result<RunOutcome, String> {
    // 展開中（メタ関数の本体）は命令数を数えるループへ（D5・`meta_ops_left` の doc）。
    if interp.is_meta_expanding() {
        run_budgeted(interp, chunk, buf, base, ip, handlers, cells)
    } else {
        run_normal(interp, chunk, buf, base, ip, handlers, cells)
    }
}

fn run_normal(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    mut ip: usize,
    mut handlers: Vec<Handler>,
    cells: Vec<Rc<RefCell<Value>>>,
) -> Result<RunOutcome, String> {

    // デバッグセッション中はステップ判定つきのループへ（#1）。
    // ⚠ **通常経路には何も足さない**のがこの分岐の目的。文境界ごとの停止判定を
    // このループに入れると、#12/#2b/#2a で削ったコストを毎命令ぶん戻すことになる。
    if crate::interpreter::debugger::dbg_active() {
        return run_stepping(interp, chunk, buf, base, ip, handlers, cells);
    }
    loop {
        // 実行時間分布の計測（`--features prof`）。既定ビルドでは消える。
        #[cfg(feature = "prof")]
        crate::prof::note_op(&chunk.code[ip]);
        // ⚠ `handlers` と `cells` は**別フィールド**なので同時に借りられる。
        match exec_op(interp, chunk, buf, base, ip, &mut handlers, &cells) {
            Ok(Flow::Next) => ip += 1,
            Ok(Flow::Jump(t)) => ip = t,
            Ok(Flow::Return(v)) => return Ok(RunOutcome::Done(v)),
            // ⚠ 再開は `yield` の**次**の命令から。`ip` のままだと同じ yield を無限に繰り返す。
            Ok(Flow::Suspend(v)) => {
                return Ok(RunOutcome::Suspended(v, Frame { ip: ip + 1, handlers, cells }))
            }
            // 呼び出しから戻った直後にデバッグセッションが始まっていることがある
            // （ネストした `break_point`）。**この再判定が無いと、既に走っている VM フレームが
            // 停止判定を持たないまま最後まで走り抜ける**（step-out が効かない実バグだった）。
            // 検査は「呼び出しから戻った直後」だけなので、通常のホット命令には影響しない。
            Ok(Flow::NextAfterCall) => {
                ip += 1;
                if crate::interpreter::debugger::dbg_active() {
                    return run_stepping(interp, chunk, buf, base, ip, handlers, cells);
                }
            }
            Err(e) => match unwind_to_handler(interp, buf, &mut handlers, &e) {
                Some(landing) => ip = landing,
                None => {
                    note_error_site(interp, chunk, ip, &e);
                    return Err(e);
                }
            },
        }
    }
}

/// 処理されずにフレームを抜ける誤りについて、**失敗した文の位置**を控える（フェーズ10 10-17）。
///
/// ⚠ 以前は関数の中で起きた誤りの traceback の最も内側のフレームが `File "", in f` で、どの行で
///   落ちたかが出なかった（呼び出し元の位置しか分からなかった）。`stmt_spans`（文境界の行テーブル・#1）を
///   `ip` から遡って文の先頭を探し、`Interpreter::error_site` に置く。関数の呼び出し（`run_vm_method`）が
///   最も内側のフレームを作るときに取り去る。
/// ⚠ 誤りの経路でしか走らない（通常の実行には何も足さない）。`raise` は自分で位置を持つので見送る。
#[cold]
fn note_error_site(interp: &mut Interpreter, chunk: &Chunk, ip: usize, err: &str) {
    if err == crate::interpreter::RAISE_SENTINEL {
        return;
    }
    let mut i = ip.min(chunk.stmt_spans.len().saturating_sub(1));
    loop {
        match chunk.stmt_spans.get(i) {
            Some(&v) if v < crate::vm::chunk::STMT_NO_SPAN => {
                interp.error_site = chunk.spans.get(v as usize).cloned();
                return;
            }
            Some(&crate::vm::chunk::STMT_NO_SPAN) | None => {
                interp.error_site = None;
                return;
            }
            _ => {}
        }
        if i == 0 {
            interp.error_site = None;
            return;
        }
        i -= 1;
    }
}

/// **命令数を数える**ディスパッチループ（展開中だけ・D5・タスク 5-0）。
///
/// 通常の `run` と同じ `exec_op` を回し、命令のたびに残りの命令数を減らす。使い切ったら
/// 終わらないメタ関数とみなして止める（エディタは打つたびに展開するので、止めないと解析が固まる）。
/// ⚠ 呼び出し先のフレームもそれぞれこのループで回る（入口の判定は `run_dispatch` ごと）。
fn run_budgeted(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    mut ip: usize,
    mut handlers: Vec<Handler>,
    cells: Vec<Rc<RefCell<Value>>>,
) -> Result<RunOutcome, String> {
    loop {
        if interp.meta_ops_left == 0 {
            return Err(format!(
                "MetaError: a metafunction did not finish within {} steps — it probably loops forever \
                 (the compile-time expander stops it so that the build and the editor do not hang)",
                crate::interpreter::META_OPS_BUDGET
            ));
        }
        interp.meta_ops_left -= 1;
        match exec_op(interp, chunk, buf, base, ip, &mut handlers, &cells) {
            Ok(Flow::Next) | Ok(Flow::NextAfterCall) => ip += 1,
            Ok(Flow::Jump(t)) => ip = t,
            Ok(Flow::Return(v)) => return Ok(RunOutcome::Done(v)),
            Ok(Flow::Suspend(v)) => {
                return Ok(RunOutcome::Suspended(v, Frame { ip: ip + 1, handlers, cells }))
            }
            Err(e) => match unwind_to_handler(interp, buf, &mut handlers, &e) {
                Some(landing) => ip = landing,
                None => {
                    note_error_site(interp, chunk, ip, &e);
                    return Err(e);
                }
            },
        }
    }
}

/// **文単位のステップ判定つき**ディスパッチループ（#1）。
///
/// 通常の `run` と同じ `exec_op` を回し、各命令の実行前に
/// 「その ip が文の先頭なら停止判定」を挟むだけ。位置は**ツリーウォークの `exec()` 冒頭と同じ**
/// （文を実行する前）なので、`--vm=off` と同じ順序で止まる。
///
/// このループへ入るのは 2 経路:
/// - `run` の入口でデバッグセッション中だったとき
/// - 呼び出しから戻ったらセッションが始まっていたとき（`Flow::NextAfterCall`）
///
/// ⚠ 通常経路（`run` のループ）には停止判定を**一切入れない**。入れると全命令に
/// 分岐が乗り、#12/#2b/#2a で削ったコストが戻る。
fn run_stepping(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    // ⚠ B13 段階 B: 通常ループから**所有権ごと**引き継ぐ（切り出し前と同じ形）。
    mut ip: usize,
    mut handlers: Vec<Handler>,
    cells: Vec<Rc<RefCell<Value>>>,
) -> Result<RunOutcome, String> {
    loop {
        // 文の先頭か？（行テーブルは code と 1:1 なので O(1)）
        if let Some(&span_idx) = chunk.stmt_spans.get(ip) {
            if span_idx != super::chunk::NOT_STMT {
                if let Some(span) = interp.vm_should_pause(chunk, span_idx) {
                    let declared = declared_slots(chunk, ip);
                    interp.vm_debug_pause(chunk, buf, base, &span, &declared)?;
                }
            }
        }
        match exec_op(interp, chunk, buf, base, ip, &mut handlers, &cells) {
            Ok(Flow::Next) | Ok(Flow::NextAfterCall) => ip += 1,
            Ok(Flow::Jump(t)) => ip = t,
            Ok(Flow::Return(v)) => return Ok(RunOutcome::Done(v)),
            Ok(Flow::Suspend(v)) => {
                return Ok(RunOutcome::Suspended(v, Frame { ip: ip + 1, handlers, cells }))
            }
            Err(e) => match handlers.pop() {
                Some(h) => {
                    buf.truncate(h.stack_len);
                    match interp.vm_take_raised(&e) {
                        Some(exc_val) => {
                            buf.push(exc_val);
                            ip = h.handler_ip;
                        }
                        None => {
                            note_error_site(interp, chunk, ip, &e);
                            return Err(e);
                        }
                    }
                }
                None => {
                    note_error_site(interp, chunk, ip, &e);
                    return Err(e);
                }
            },
        }
    }
}
