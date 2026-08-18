// vm/chunk.rs — コンパイル済み関数の実行表現（Phase V, V-A）。

use crate::ast::AttrCache;
use crate::ast::Stmt;
use crate::interpreter::Value;
use crate::token::Span;

use super::op::Op;

/// `target <- async->T: body` の VM 表現（タスク #9）。`AsyncSubmit(idx)` op が参照する。
/// `body` は非同期タスクの AST（別スレッドでツリーウォーク実行される）。`captures` は本体が参照する
/// enclosing フレームの `(変数名, slot, is_mutable)`（実行時に frame から値を読んで env を組む）。
/// 定義サイトごとに共有する、コンパイル済みクロージャ本体（#30）。
///
/// `Op::MakeFn` はクロージャ**実体**ごとに新しい `FnValue` を作るので、`FnValue` の
/// アドレスをキーにする `Interpreter::vm_chunks` では**実体ごとに再コンパイル**していた
/// （実測 **2.3µs/実体** ＝ クロージャ呼び出し約 8 回分）。本体は `ChunkFnDef`（＝定義サイト）
/// ごとに 1 つで足りるので、ここに持たせて全実体で共有する。
///
/// **実体に依らないことの根拠**（`compile_fn` の入力が定義サイトで決まること）:
/// - `params` / `body` は `ChunkFnDef` の値そのもの。
/// - キャプチャ名の集合は `captures` / `cell_captures` / `static_captures` で固定。
/// - slot 採番は `capture_names.sort()` 済みで **`captured_env`（HashMap）の反復順に依存しない**。
/// - 束縛は `bind_captures` が**名前で引く**ので、slot 番号が実体間で共有されても正しい。
///
/// ⚠ 注釈テーブル（`Interpreter::annotations`）だけは入口によって差し替わりうる（REPL）ので、
/// コンパイル時のものを一緒に覚えて**違えば作り直す**（`Rc::ptr_eq` 1 回）。
/// ⚠ **生ポインタではなく `Rc` を保持する**。生ポインタだと古い表が解放されたあと
/// アロケータが同じアドレスを再利用し、**別の表で当たったと誤判定する**
/// （最上位 Chunk キャッシュが `Stmt` のアドレスで踏んだのと同じ形・#36）。
/// `Rc` を持っている間はその表が解放されないので、この誤判定が原理的に起きない。
/// ⚠ **スレッドへ送る `deep_clone` では共有しない**（`Rc` の参照カウントは非アトミック・#15）。
#[derive(Clone, Default)]
pub struct SharedFnChunk(std::rc::Rc<std::cell::RefCell<Option<CachedFnChunk>>>);

struct CachedFnChunk {
    /// コンパイル時の注釈テーブル。**アドレス再利用を防ぐため `Rc` ごと保持する**。
    annot: std::rc::Rc<crate::type_check::AstAnnotations>,
    /// `None` = このクロージャは VM に載らないと判明済み（再挑戦しない）。
    chunk: Option<std::rc::Rc<Chunk>>,
}

impl SharedFnChunk {
    /// キャッシュを引く。`None` = 未コンパイル、または注釈が差し替わっていて使えない。
    /// `Some(inner)` の `inner` が `compile_fn` の結果（`None` = 非対応）。
    pub fn lookup(
        &self,
        annot: &std::rc::Rc<crate::type_check::AstAnnotations>,
    ) -> Option<Option<std::rc::Rc<Chunk>>> {
        match &*self.0.borrow() {
            Some(c) if std::rc::Rc::ptr_eq(&c.annot, annot) => Some(c.chunk.clone()),
            _ => None,
        }
    }

    /// コンパイル結果を覚える（非対応＝`None` も覚えて再挑戦を防ぐ）。
    pub fn store(
        &self,
        annot: std::rc::Rc<crate::type_check::AstAnnotations>,
        chunk: Option<std::rc::Rc<Chunk>>,
    ) {
        *self.0.borrow_mut() = Some(CachedFnChunk { annot, chunk });
    }
}

// `Chunk` は `Debug` を実装しないので手書きする（`ChunkFnDef` の derive を保つため）。
impl std::fmt::Debug for SharedFnChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = match &*self.0.borrow() {
            None => "uncompiled",
            Some(c) if c.chunk.is_some() => "compiled",
            Some(_) => "ineligible",
        };
        write!(f, "SharedFnChunk({state})")
    }
}

/// 入れ子 `fn` 定義 1 件ぶんのデータ（#27・`Op::MakeFn`）。
#[derive(Debug, Clone)]
pub struct ChunkFnDef {
    pub name: String,
    pub params: Vec<crate::ast::Param>,
    /// 入れ子 `fn` の本体。**`Rc` で持つ**（#45）。`Op::MakeFn` は実体ごとに
    /// この `Rc` を clone するだけなので、クロージャ生成から**AST の複製が消える**。
    /// ⚠ **スレッドへ送る `deep_clone` では共有しない**（参照カウントが非アトミック・#15）。
    pub body: std::rc::Rc<[Stmt]>,
    pub return_type: Option<String>,
    /// 生成した関数値を書き込む slot（リゾルバの base slot と同じ番号）。
    pub slot: u16,
    /// キャプチャする外側ローカル（名前, slot）。**不変な変数だけ**（#27）。
    /// 生成時点の値を複製して `CapturedVar::Immutable` にする。
    pub captures: Vec<(String, u16)>,
    /// 可変キャプチャ（名前, **外側フレームのセル index**）（#27-d 段階 2b）。
    /// `CapturedVar::Mutable(cell)` としてセルを**共有**する（ツリーウォークの
    /// `capture_env` が `Var::Mutable` → `Var::Cell` へ昇格するのと同じ効果）。
    pub cell_captures: Vec<(String, u16)>,
    /// `static mut` のキャプチャ（名前, 宣言位置の span index）（#27-d 段階 2b）。
    /// セルは `Interpreter::static_cells` にあるので、実行時に span をキーに引いて共有する。
    pub static_captures: Vec<(String, u32)>,
    /// この定義サイトから作られた**全クロージャ実体で共有**するコンパイル済み本体（#30）。
    pub compiled: SharedFnChunk,
}

/// `let a, b = t` 1 件ぶんの分解情報（#27-c）。
pub struct TupleDecl {
    pub targets: Vec<crate::ast::TupleTarget>,
    /// 各ターゲットの格納先 slot（`Wildcard` と最上位宣言は `None`）。
    ///
    /// **空 Vec なら最上位**（`declare_var` でグローバルを宣言する）。空でなければ
    /// `targets` と同じ長さで、フラットフレームの slot へ直接書く。
    /// 入れ子の宣言は `collect_nested_decls` が slot を割り当てるので、この 2 つで
    /// 「最上位の宣言文」と「制御フロー内の宣言」を区別できる。
    pub slots: Vec<Option<u16>>,
}

/// キーワード/可変長引数つき呼び出し 1 件ぶんの情報（#27-c）。`Op::CallKw(idx)` が参照する。
///
/// `Op::Call` は (argc, mut_mask, name, span, node_id) で既に 20 バイト＝最大 variant なので、
/// 引数名を足すと**全命令が太る**。稀な形なので副表へ逃がしてある（`ffi_call_info` と同じ判断）。
pub struct KwCall {
    pub argc: u16,
    pub mut_mask: u32,
    /// 呼び先の表示名（トレースバック用）。
    pub name_idx: u32,
    pub span_idx: u32,
    pub node_id: u32,
    /// 各引数の名前。`None` = 位置引数、`Some("...")` = 可変長（値はリストに畳んである）。
    /// 長さは `argc` と一致する。
    pub arg_names: Vec<Option<String>>,
}

pub struct AsyncBlock {
    pub body: Vec<Stmt>,
    pub captures: Vec<(String, u16, bool)>,
}

/// 文境界の行テーブル（`Chunk::stmt_spans`）の番兵。
///
/// - `NOT_STMT`: その ip は**文の先頭ではない**（大多数の op）。
/// - `STMT_NO_SPAN`: 文の先頭だが、その文は位置情報を持たない
///   （`stmt_location` が `None` を返す種類 — `if`/`while`/`return` 等）。
///   ツリーウォークの `best_span_for` はこの場合 `dbg_last_span` へフォールバックするので、
///   VM も同じ扱いにする（そうしないと transcript が食い違う）。
pub const NOT_STMT: u32 = u32::MAX;
/// 文の先頭だが位置情報なし（`best_span_for` のフォールバックに委ねる）。
pub const STMT_NO_SPAN: u32 = u32::MAX - 1;

/// 1 関数分のコンパイル結果。`Rc<Chunk>` で関数ごとにキャッシュされる。
pub struct Chunk {
    /// 命令列。
    pub code: Vec<Op>,
    /// 定数プール（リテラル値）。
    pub consts: Vec<Value>,
    /// 属性名プール（`GetAttr` が参照）。
    pub names: Vec<String>,
    /// 属性アクセスのインラインキャッシュ（R3 と同じ機構を VM 内でも使う）。
    pub attr_caches: Vec<AttrCache>,
    /// 位置情報プール（`Raise`/`Call`/`CallMethod` が参照。例外・トレースバックの file/line/col）。
    pub spans: Vec<Span>,
    /// **文境界の行テーブル**（#1）。`code` と同じ長さで、`stmt_spans[ip]` は
    /// `NOT_STMT` / `STMT_NO_SPAN` / `spans` への index のいずれか。
    ///
    /// デバッガの文単位ブレークが「この ip は文の先頭か・その位置はどこか」を O(1) で引くために使う。
    /// **通常の実行ループは一切参照しない**（デバッグセッション中の専用ループだけが読む）。
    ///
    /// ⚠ 命令を削除・移動する最適化（[peephole](super::peephole)）は**この表も同時に詰め直す**こと。
    /// ずれると停止位置が別の文にずれる。
    pub stmt_spans: Vec<u32>,
    /// slot → 変数名 のデバッグ名テーブル（V-E。デバッガ REPL 用メタデータ。実行では未使用なので
    /// 現状は保持のみ — デバッガ VM 統合で消費予定）。
    #[allow(dead_code)]
    pub local_names: Vec<String>,
    /// フレームのローカル slot 総数（パラメータ + base ローカル）。
    pub n_locals: usize,
    /// 先頭から何 slot がパラメータか（`self` を含む）。
    /// デバッガが「停止時点で**まだ宣言されていない**ローカル」を隠すのに使う（#1）:
    /// パラメータは入口で束縛済みなので最初から可視、それ以外は Store されるまで不可視。
    pub n_params: usize,
    /// 非同期タスクブロック（`AsyncSubmit(idx)` が参照, タスク #9）。async を含まない関数では空。
    pub async_blocks: Vec<AsyncBlock>,
    /// `LoadGlobal` のグローバル索引キャッシュ（#11）。`(slot_epoch, scopes[0] index)` を焼く。
    pub global_caches: Vec<crate::ast::SlotCache>,
    /// メソッド呼び出しの FFI 境界検査用の表示情報（#27-b）。node_id → (表示名 index, span index)。
    ///
    /// **エラーメッセージのためだけ**に要る（`check_ffi_return` の `callee_name` / `call_span`）。
    /// op のオペランドに足すと `Op` のサイズが `Call` を超えて全命令が太るので、
    /// **参照されるのが外部言語レシーバのときだけ**という性質を使って副表に逃がしてある。
    /// これが無いと `L.get_int` が `get_int`、位置が `<unknown>` になり off/auto が食い違う。
    pub ffi_call_info: std::collections::HashMap<u32, (u32, u32)>,
    /// 入れ子 `fn` 定義（#27）。`Op::MakeFn(idx)` が index で参照する。
    ///
    /// ⚠ **キャプチャを持たない入れ子 `fn` に限る**。コンパイラが
    /// 「自由変数 ∩ 外側の slot = ∅」を確かめてから積むので、生成する `FnValue` の
    /// `captured_env` は**構成上空**になり、ツリーウォークの `capture_env`（外側スコープを
    /// 走査して何も見つからない）と一致する。
    pub fn_defs: Vec<ChunkFnDef>,
    /// テンプレート実体化の型引数リスト（#27-c）。`Op::CallTemplate(idx, ..)` が index で参照する。
    ///
    /// `names` に入れて (開始, 個数) で持つ手もあるが、`instantiate_template_evaled` が
    /// `&[String]` を要求するので**そのまま渡せる形**にしてある。
    pub type_arg_lists: Vec<Vec<String>>,
    /// `let a, b = t` の分解（#27-c）。`Op::LetTuple(idx)` が参照する。
    pub tuple_decls: Vec<TupleDecl>,
    /// キーワード/可変長引数つき呼び出し（#27-c）。`Op::CallKw(idx)` が参照する。
    pub kw_calls: Vec<KwCall>,
    /// フレームの**セル表**の大きさ（#27-d 段階 2b）。0 なら確保しない（大多数の関数）。
    ///
    /// セルは `Rc<RefCell<Value>>` で、slot（`Value` 直値）では表現できない
    /// 「外側フレームやクロージャとの共有」を担う。`LoadCell`/`StoreCell` の index 空間。
    pub n_cells: usize,
    /// **可変キャプチャ**の束縛先（#27-d 段階 2b）。`(変数名, セル index)`。
    ///
    /// 呼び出し側が `fn_val.captured_env` の `CapturedVar::Mutable(cell)` を**そのまま**
    /// この index へ入れる（clone するのは `Rc` だけ＝**外側と同じセルを指す**）。
    /// ここに載らないセル index は呼び出しごとに新規作成される（自分のローカル用）。
    pub captured_cells: Vec<(String, u16)>,
    /// クロージャの**不変キャプチャ**の束縛先（#27-d）。`(変数名, slot)`。
    ///
    /// 呼び出し側（`exec_fn_evaled`）がパラメータを束縛したあと、`fn_val.captured_env` から
    /// 同名の値を読んでこの slot へ書き込む。**名前で引く**ので `captured_env`（HashMap）の
    /// 反復順に依存しない。
    ///
    /// ⚠ **可変キャプチャ（`CapturedVar::Mutable`）はここに載せられない**。
    /// ツリーウォークは外側と `Rc<RefCell<Value>>` を共有するので、値を slot へコピーすると
    /// 書き戻りが消える。可変キャプチャを含むクロージャは `vm_eligible` が偽になる。
    pub captured_slots: Vec<(String, u16)>,
}
