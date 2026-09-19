/**
 * wasm_providers.ts — VS Code の言語機能 7 種を、Arrow の**本物のフロントエンド**で実装する。
 *
 * 解析は `frontend.ts` 経由で wasm（`crates/arrow-frontend`）に投げる。その中身は
 * `cargo run` が使うのと同一の lexer / parser / type_checker なので、ここには
 * 「Arrow の文法とはどういうものか」という判断が**一切無い**。このファイルがやるのは
 * 解析結果を VS Code のオブジェクトへ翻訳することだけ。
 *
 * 旧実装（analysis.ts / type_infer.ts）は行単位の正規表現で同じことを近似していたため、
 * Rust 側に構文が増えるたびに手で追随する必要があり、実際 16 個のキーワードが取り残されて
 * いた（`protocol` は宣言として認識すらされていなかった）。その構造的な原因を消すのが目的。
 */

import * as vscode from 'vscode';
import * as fs from 'fs';
import { analyze, isFrontendReady, type AnalysisResult, type WasmDiagnostic } from './frontend';

// ===== 解析結果の型（frontend が返す JSON の形） =====

interface Pos { line: number; col: number }

/** 宣言 1 件。`at` は名前トークンの位置（0 始まり）。 */
export interface Symbol {
    name: string;
    kind: string;
    at: Pos;
    mutability: string | null;
    typeAnn: string | null;
    /** 注釈が無い宣言の推論型（初期化式の node-id で型検査器から引いたもの）。 */
    inferred: string | null;
    signature: string | null;
    doc: string | null;
    access: string | null;
    container: string | null;
    bases: string[];
    scope: number;
    bodyScope: number | null;
}

interface Scope { parent: number; startLine: number; endLine: number }
interface ExprType { at: Pos; type: string }
/**
 * 型注釈位置に現れた型名 1 件（`let x: int` の `int`）。
 *
 * これはパーサ（`parse_type_expr`）だけが知っている事実で、**名前からは復元できない**。
 * `int` / `str` / `float` / `bool` / `uint` / `set` / `slice` / `path` / `type` の 9 個は
 * 型名であると同時に `builtins.ars` の組み込み関数でもあるので、名前引きだけに頼ると
 * 型注釈中の `int` が「キャスト関数」として着色・hover される。
 */
interface TypeRef { at: Pos; name: string }

/**
 * トークン 1 個の範囲と粗い種別。位置は 0 始まり・列は UTF-16（VS Code と同じ）。
 *
 * これが入ったことで、このファイルから**自前の字句解析が消えた**。以前は行を
 * 識別子の正規表現で走査し、文字列とコメントを `maskLine()` で潰していたが、
 * それは本物の lexer とズレる（複数行 `"""…"""`・`m"…"` / `$…$`・`r"` のプレフィックス）。
 *
 * ⚠ `tokens` は **`ok: false` のときも現在のテキストのもの**が来る。`Lexer::tokenize()` は
 *   失敗しないため。だから構文エラー中でも色と語の特定だけは正確に保てる。
 */
interface Token {
    kind: 'ident' | 'keyword' | 'str' | 'num' | 'comment' | 'op';
    line: number;
    col: number;
    endLine: number;
    endCol: number;
}
interface Member {
    name: string;
    kind: string;
    type: string | null;
    mutability?: string;
    access: string;
    params?: { name: string; label: string; type: string | null; optional: boolean; variadic: boolean }[];
    doc?: string | null;
}
interface MemberTable { members: Member[]; bases: string[] }

interface Analysis extends AnalysisResult {
    symbols: Symbol[];
    scopes: Scope[];
    exprTypes: ExprType[];
    typeRefs: TypeRef[];
    tokens: Token[];
    /**
     * 構文エラーで**パーサが止まった位置**。`ok` が true のときは null。
     *
     * これが来る前は、拡張がエラーメッセージ本文に正規表現を当てて行・列を
     * 読み直していた。`parse_program` が `Result<_, String>` を返す ＝ 位置を
     * 人間向けの文章に埋めて捨てるため、それ以外に手が無かった。
     */
    parseErrorAt: Pos | null;
    members: Record<string, MemberTable>;
}

/**
 * `line:col` → その位置に書かれた型名。`analysis.typeRefs` の逆引き。
 *
 * 解析結果はバージョン単位でキャッシュされるので、索引も同じ寿命で持てる。
 */
const typeRefIndexCache = new WeakMap<Analysis, Map<string, string>>();

function typeRefsOf(analysis: Analysis): Map<string, string> {
    let index = typeRefIndexCache.get(analysis);
    if (!index) {
        index = new Map();
        // 旧い wasm（`typeRefs` を出さない版）と組み合わさっても落ちないようにする。
        for (const ref of analysis.typeRefs ?? []) {
            index.set(`${ref.at.line}:${ref.at.col}`, ref.name);
        }
        typeRefIndexCache.set(analysis, index);
    }
    return index;
}

// ===== キャッシュ =====

interface CacheEntry {
    /** このバージョンの解析結果。構文エラーなら `null`。 */
    fresh: Analysis | null;
    /** 直近で構文が通ったときの結果。入力途中の穴埋めに使う。 */
    lastGood: Analysis | null;
    /**
     * プロバイダへ実際に渡すもの。構文エラー中は「`lastGood` の宣言表 ＋
     * **現在のテキストの** `tokens`」を合成したもの。
     *
     * 合成をここに 1 つ持つのは、providers が呼ばれるたびに作り直すと
     * `typeRefIndexCache`（Analysis をキーにする WeakMap）が毎回ミスするため。
     */
    view: Analysis | null;
    version: number;
}

const cache = new Map<string, CacheEntry>();

// ===== 組み込み関数のプレリュード =====

/**
 * `builtins.ars`（`print` / `len` / … のスタブ）を解析した結果。
 *
 * 組み込みも**同じフロントエンドで**解析する。TypeScript 側に組み込みの表を持たせると、
 * Rust 側に組み込みが増えたときに手で追随することになり、いま直している問題が
 * 小さい形で戻ってくる。`builtins.ars` は valid な Arrow なのでそのまま食わせられる。
 */
let prelude: Symbol[] = [];
let preludeMembers: Record<string, MemberTable> = {};

/** 拡張の activate から 1 度だけ呼ぶ。失敗しても致命的ではない（組み込みが出ないだけ）。 */
export function loadPrelude(builtinsPath: string): boolean {
    try {
        const result = analyze(fs.readFileSync(builtinsPath, 'utf8')) as Analysis | null;
        if (!result?.ok) return false;
        // トップレベル（スコープ 0）の宣言だけを組み込みとして扱う。
        prelude = result.symbols.filter(s => s.scope === 0);
        preludeMembers = result.members;
        return true;
    } catch {
        return false;
    }
}

/**
 * ドキュメントを解析する（同一バージョンならキャッシュを返す）。
 *
 * 構文エラーのときは **`lastGood` を返す**。エディタのバッファは入力中ほぼ常に
 * 構文不正なので、そこで情報を捨てると「打っている最中だけ hover も補完も死ぬ」
 * という、直そうとしている問題より悪い状態になる。
 */
function getAnalysis(document: vscode.TextDocument): Analysis | null {
    const key = document.uri.toString();
    const entry = cache.get(key);
    if (entry && entry.version === document.version) {
        return entry.view;
    }

    const result = analyze(document.getText()) as Analysis | null;
    const lastGood = entry?.lastGood ?? null;

    if (!result) {
        // wasm 自体が使えない。旧実装へのフォールバックはしない（二重実装を残さないため）。
        cache.set(key, { fresh: null, lastGood, view: lastGood, version: document.version });
        return lastGood;
    }
    if (!result.ok) {
        // 構文エラー。宣言・スコープ・型は `lastGood`（**古いテキスト**）で埋めるしかないが、
        // `tokens` だけは今のテキストのものが来ている（`Lexer::tokenize()` は失敗しない）。
        // 差し替えないと、打っている最中ずっと**別の場所**に色が付く。
        const view = lastGood ? { ...lastGood, tokens: result.tokens ?? [] } : null;
        cache.set(key, { fresh: null, lastGood, view, version: document.version });
        return view;
    }
    // `tokens` が無い wasm（このキーを出す前のビルド）と組み合わさっても落ちないようにする。
    // 色が出なくなるだけで、拡張全体は動く。
    if (!result.tokens) result.tokens = [];
    cache.set(key, { fresh: result, lastGood: result, view: result, version: document.version });
    return result;
}

/** ドキュメントが閉じられたらキャッシュを捨てる。 */
export function forgetDocument(document: vscode.TextDocument): void {
    cache.delete(document.uri.toString());
}

/**
 * 解析キャッシュを全部捨てる。
 *
 * ⚠ **スタブ表を入れ替えたら必ず呼ぶこと。** キャッシュの鍵は `document.version` だけなので、
 *    テキストが変わらない限り古い解析結果を返し続ける。スタブだけ更新しても
 *    「型が出ない・古い型が出る」ままになる（`stubs.ts` 冒頭 doc）。
 */
export function clearAnalysisCache(): void {
    cache.clear();
}

/** この版の解析が構文エラーだったか（診断の出し分けに使う）。 */
function freshParseFailed(document: vscode.TextDocument): boolean {
    const e = cache.get(document.uri.toString());
    return !!e && e.version === document.version && e.fresh === null;
}

// ===== スコープ・シンボル探索 =====

/** `line` を含むスコープ id を、内側から外側へ並べて返す。 */
function scopeChainAt(analysis: Analysis, line: number): number[] {
    // 行を含む最も内側のスコープを選ぶ（同じ行を含むものが複数あれば後ろ＝より内側）。
    let innermost = 0;
    analysis.scopes.forEach((s, id) => {
        const end = s.endLine < 0 ? Number.MAX_SAFE_INTEGER : s.endLine;
        if (line >= s.startLine && line <= end) innermost = id;
    });

    const chain: number[] = [];
    let cur = innermost;
    const guard = analysis.scopes.length + 1;   // 壊れた親リンクで無限ループしない保険
    for (let i = 0; i < guard && cur >= 0; i++) {
        chain.push(cur);
        const parent: number = analysis.scopes[cur]?.parent ?? -1;
        cur = parent;
    }
    return chain;
}

/**
 * `line` の位置から見える宣言を返す（内側のスコープが外側を隠す）。
 *
 * これがスコープ無視の名前列挙（旧実装はファイル内の全シンボルを 128 件返していた）を
 * 直している中核。可視性の判断はパーサが作ったスコープ木がそのまま根拠になる。
 */
function visibleSymbols(analysis: Analysis, line: number): Symbol[] {
    const chain = scopeChainAt(analysis, line);
    const seen = new Set<string>();
    const out: Symbol[] = [];
    for (const scopeId of chain) {
        for (const s of analysis.symbols) {
            if (s.scope !== scopeId) continue;
            if (seen.has(s.name)) continue;
            seen.add(s.name);
            out.push(s);
        }
    }
    // 組み込みは最も外側。ユーザーが同名を宣言していればそちらが勝つ。
    for (const s of prelude) {
        if (seen.has(s.name)) continue;
        seen.add(s.name);
        out.push(s);
    }
    return out;
}

// ===== トークン列（自前の字句解析を置き換える土台） =====

/**
 * `analysis.tokens` の中で `pos` を覆うトークンの添字。無ければ -1。
 *
 * トークンは位置順に並んでいるので二分探索でよい。1 行あたり数十トークンでも
 * ファイル全体では数千になるため、hover のたびに線形走査はしない。
 */
function tokenIndexAt(analysis: Analysis, pos: vscode.Position): number {
    const toks = analysis.tokens;
    let lo = 0, hi = toks.length - 1, found = -1;
    while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        const t = toks[mid];
        const startsAfter = t.line > pos.line || (t.line === pos.line && t.col > pos.character);
        const endsBefore = t.endLine < pos.line
            || (t.endLine === pos.line && t.endCol <= pos.character);
        if (startsAfter) hi = mid - 1;
        else if (endsBefore) lo = mid + 1;
        else { found = mid; break; }
    }
    return found;
}

/** `pos` を覆うトークン。無ければ undefined。 */
function tokenAt(analysis: Analysis, pos: vscode.Position): Token | undefined {
    const i = tokenIndexAt(analysis, pos);
    return i < 0 ? undefined : analysis.tokens[i];
}

/**
 * `pos` の**直前**（同じ位置で終わるものを含む）にあるトークンの添字。
 *
 * `x.` の `.` や、呼び出しの `(` を後ろ向きに探すときの起点。行をまたいで
 * 正しく動くのが要点で、これができないと複数行にまたがる呼び出しを扱えない。
 */
function tokenIndexBefore(analysis: Analysis, pos: vscode.Position): number {
    const toks = analysis.tokens;
    let lo = 0, hi = toks.length - 1, best = -1;
    while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        const t = toks[mid];
        // t の終端が pos 以下なら候補。
        const endsAtOrBefore = t.endLine < pos.line
            || (t.endLine === pos.line && t.endCol <= pos.character);
        if (endsAtOrBefore) { best = mid; lo = mid + 1; } else hi = mid - 1;
    }
    return best;
}

/** トークンの範囲を VS Code の Range にする。 */
function tokenRange(t: Token): vscode.Range {
    return new vscode.Range(t.line, t.col, t.endLine, t.endCol);
}

/**
 * 位置 `pos` にある識別子とその範囲。識別子トークンの上でなければ null。
 *
 * 旧実装は `getWordRangeAtPosition()` に識別子の正規表現を渡していたので、コメントや
 * 文字列の中の語も拾っていた（`# int を返す` の `int` に hover が出た）。いまはトークンが
 * `comment` / `str` かどうかを lexer が答えているので、その手の誤爆が構造的に起きない。
 */
function wordAt(document: vscode.TextDocument, analysis: Analysis, pos: vscode.Position):
    { word: string; range: vscode.Range } | null {
    const t = tokenAt(analysis, pos);
    if (!t || t.kind !== 'ident') return null;
    const range = tokenRange(t);
    return { word: document.getText(range), range };
}

/** ちょうどその位置で宣言されているシンボル（名前トークンの上にカーソルがある場合）。 */
function declarationAt(analysis: Analysis, pos: vscode.Position, word: string): Symbol | undefined {
    return analysis.symbols.find(
        s => s.at.line === pos.line && s.name === word &&
             pos.character >= s.at.col && pos.character <= s.at.col + s.name.length,
    );
}

/** その名前の宣言のうち、`line` から見えるもの（無ければファイル内の最初の同名宣言）。 */
function lookup(analysis: Analysis, name: string, line: number): Symbol | undefined {
    return visibleSymbols(analysis, line).find(s => s.name === name)
        ?? analysis.symbols.find(s => s.name === name);
}

/** カーソル位置の式の推論型（型検査器が実際に計算したもの）。 */
function exprTypeAt(analysis: Analysis, pos: vscode.Position): string | undefined {
    // node の位置は式の**末尾**トークンを指す。識別子ではその識別子自身なので、
    // 同じ行で、カーソルを含む名前の末尾に一致するものを探す。
    let best: string | undefined;
    for (const e of analysis.exprTypes) {
        if (e.at.line !== pos.line) continue;
        if (Math.abs(e.at.col - pos.character) <= 64) {
            if (e.at.col <= pos.character) best = e.type;
        }
    }
    return best;
}

// ===== 表示用の整形 =====

/** hover に出す 1 行目。`public mut x: int` / `fn f(let a: int) -> str` の形。 */
function renderSignature(sym: Symbol, inferred?: string): string {
    if (sym.signature && (sym.kind === 'function' || sym.kind === 'generator')) {
        return `${sym.access ? sym.access + ' ' : ''}${sym.signature}`;
    }
    switch (sym.kind) {
        case 'class':    return `class ${sym.name}`;
        case 'trait':    return `trait ${sym.name}`;
        case 'protocol': return `protocol ${sym.name}`;
        case 'enum':     return `enum ${sym.name}`;
        case 'new_type': return `new_type ${sym.name}: ${sym.typeAnn ?? '?'}`;
        case 'alias':    return `alias ${sym.name}`;
        case 'module':   return sym.signature ?? `module ${sym.name}`;
        case 'enum_member': return `${sym.container}.${sym.name}`;
        default: {
            const ty = sym.typeAnn ?? inferred;
            const prefix = [sym.access, sym.mutability].filter(Boolean).join(' ');
            const head = prefix ? `${prefix} ${sym.name}` : sym.name;
            return ty ? `${head}: ${ty}` : head;
        }
    }
}

function symbolKindOf(kind: string): vscode.SymbolKind {
    switch (kind) {
        case 'class':       return vscode.SymbolKind.Class;
        case 'trait':
        case 'protocol':    return vscode.SymbolKind.Interface;
        case 'enum':        return vscode.SymbolKind.Enum;
        case 'enum_member': return vscode.SymbolKind.EnumMember;
        case 'function':
        case 'generator':   return vscode.SymbolKind.Function;
        case 'field':       return vscode.SymbolKind.Field;
        case 'param':       return vscode.SymbolKind.Variable;
        case 'module':      return vscode.SymbolKind.Module;
        case 'new_type':
        case 'alias':       return vscode.SymbolKind.TypeParameter;
        default:            return vscode.SymbolKind.Variable;
    }
}

function completionKindOf(kind: string): vscode.CompletionItemKind {
    switch (kind) {
        case 'class':         return vscode.CompletionItemKind.Class;
        case 'trait':
        case 'protocol':      return vscode.CompletionItemKind.Interface;
        case 'enum':          return vscode.CompletionItemKind.Enum;
        case 'enum_member':   return vscode.CompletionItemKind.EnumMember;
        case 'function':
        case 'generator':     return vscode.CompletionItemKind.Function;
        case 'method':
        case 'static_method': return vscode.CompletionItemKind.Method;
        case 'field':         return vscode.CompletionItemKind.Field;
        case 'param':         return vscode.CompletionItemKind.Variable;
        case 'module':        return vscode.CompletionItemKind.Module;
        default:              return vscode.CompletionItemKind.Variable;
    }
}

// ===== 1. Hover =====

export function provideHover(
    document: vscode.TextDocument,
    position: vscode.Position,
): vscode.Hover | undefined {
    const analysis = getAnalysis(document);
    if (!analysis) return undefined;
    const w = wordAt(document, analysis, position);
    if (!w) return undefined;

    const sym = declarationAt(analysis, position, w.word) ?? lookup(analysis, w.word, position.line);

    // 型位置に書かれた名前で、引けた宣言が型でない（＝`int` のような型名/関数の兼用名）なら、
    // 関数のシグネチャを見せてはいけない。ユーザー定義のクラス・トレイト等はこの分岐を
    // 通さず下の通常経路へ流し、継承元や docstring も含めてそのまま見せる。
    if (typeRefsOf(analysis).has(`${position.line}:${w.range.start.character}`)
        && (!sym || !TYPE_TOKEN_OF[sym.kind])) {
        const md = new vscode.MarkdownString();
        md.appendCodeblock(`type ${w.word}`, 'arrow');
        return new vscode.Hover(md, w.range);
    }

    // 宣言に紐づく推論型を優先し、無ければカーソル位置の式の型で補う。
    const inferred = sym?.inferred ?? exprTypeAt(analysis, position);

    if (!sym) {
        // 宣言が引けなくても、型検査器が式の型を出していれば見せる価値がある。
        if (!inferred) return undefined;
        const md = new vscode.MarkdownString();
        md.appendCodeblock(`${w.word}: ${inferred}`, 'arrow');
        return new vscode.Hover(md, w.range);
    }

    const md = new vscode.MarkdownString();
    md.appendCodeblock(renderSignature(sym, inferred), 'arrow');
    if (sym.bases.length > 0 && (sym.kind === 'class' || sym.kind === 'trait')) {
        md.appendMarkdown(`\n\nimplements: ${sym.bases.map(b => `\`${b}\``).join(', ')}`);
    }
    if (sym.container) {
        md.appendMarkdown(`\n\nmember of \`${sym.container}\``);
    }
    if (sym.doc) {
        md.appendMarkdown('\n\n---\n\n' + sym.doc);
    }
    return new vscode.Hover(md, w.range);
}

// ===== 2. Inlay hints =====

export function provideInlayHints(
    document: vscode.TextDocument,
    range: vscode.Range,
): vscode.InlayHint[] {
    const analysis = getAnalysis(document);
    if (!analysis) return [];

    const hints: vscode.InlayHint[] = [];
    for (const sym of analysis.symbols) {
        // 注釈が既に書いてある宣言にヒントは要らない。
        if (sym.typeAnn) continue;
        if (sym.kind !== 'variable' && sym.kind !== 'param') continue;
        if (sym.at.line < range.start.line || sym.at.line > range.end.line) continue;

        // 推論型は型検査器の答えをそのまま使う（`inferred` は初期化式の node-id 経由）。
        const inferred = sym.inferred;
        if (!inferred) continue;

        const at = new vscode.Position(sym.at.line, sym.at.col + sym.name.length);
        const hint = new vscode.InlayHint(at, `: ${inferred}`, vscode.InlayHintKind.Type);
        hint.paddingLeft = false;
        hints.push(hint);
    }
    return hints;
}

// ===== 3. Semantic tokens =====

export const SEMANTIC_TOKENS_LEGEND = new vscode.SemanticTokensLegend(
    ['class', 'interface', 'enum', 'enumMember', 'function', 'method',
     'property', 'parameter', 'variable', 'namespace', 'type'],
    ['declaration'],
);

const TOKEN_TYPE_OF: Record<string, string> = {
    class: 'class',
    trait: 'interface',
    protocol: 'interface',
    enum: 'enum',
    enum_member: 'enumMember',
    function: 'function',
    generator: 'function',
    field: 'property',
    param: 'parameter',
    variable: 'variable',
    module: 'namespace',
    new_type: 'type',
    alias: 'type',
};

/**
 * **型位置に現れた名前**が、同名の宣言を持つときのトークン種別。
 *
 * `TOKEN_TYPE_OF` と分けてあるのは、型位置では「関数」「変数」といった種別を
 * 採ってはいけないから。ここに載っていない種別（`function` など）だった場合は
 * 汎用の `type` に落とす — それが `int` を関数色にしないための分岐そのもの。
 */
const TYPE_TOKEN_OF: Record<string, string> = {
    class: 'class',
    trait: 'interface',
    protocol: 'interface',
    enum: 'enum',
    new_type: 'type',
    alias: 'type',
};

export function provideDocumentSemanticTokens(
    document: vscode.TextDocument,
): vscode.SemanticTokens {
    const builder = new vscode.SemanticTokensBuilder(SEMANTIC_TOKENS_LEGEND);
    const analysis = getAnalysis(document);
    if (!analysis) return builder.build();

    // 宣言そのものには declaration 修飾を付ける。
    const declAt = new Map<string, Symbol>();
    for (const s of analysis.symbols) declAt.set(`${s.at.line}:${s.at.col}`, s);

    const typeRefs = typeRefsOf(analysis);

    // 走るのは**識別子トークンだけ**。以前は行を `/[A-Za-z_]\w*/g` で総なめし、
    // 文字列とコメントを `maskLine()` で潰していたが、それは lexer とズレる
    // （複数行 `"""…"""` の 2 行目以降・`m"…"` / `$…$`・`r"` のプレフィックス）。
    // いまは「何が識別子か」を lexer が答えるので、その手の誤検出が構造的に起きない。
    let line = -1;
    let byName = new Map<string, Symbol>();
    for (const t of analysis.tokens) {
        if (t.kind !== 'ident') continue;
        // 可視シンボルは行ごとに変わる。行が変わったときだけ引き直す。
        if (t.line !== line) {
            line = t.line;
            byName = new Map<string, Symbol>();
            for (const s of visibleSymbols(analysis, line)) {
                if (!byName.has(s.name)) byName.set(s.name, s);
            }
        }
        // 識別子は 1 行に収まる（複数行に跨るのは文字列だけ）。
        const length = t.endCol - t.col;
        const at = `${t.line}:${t.col}`;
        const decl = declAt.get(at);

        // 型位置の識別子は、**名前引きより先に**型として確定させる。
        // ここを通さないと `let x: int` の `int` が prelude の `fn int` に当たり、
        // 型名がキャスト関数として着色される。パーサが型位置だと言っている以上、
        // 名前が何であれ型が正しい。
        if (!decl && typeRefs.has(at)) {
            const named = byName.get(document.getText(tokenRange(t)));
            const typeTok = (named && TYPE_TOKEN_OF[named.kind]) || 'type';
            builder.push(t.line, t.col, length,
                SEMANTIC_TOKENS_LEGEND.tokenTypes.indexOf(typeTok), 0);
            continue;
        }

        const sym = decl ?? byName.get(document.getText(tokenRange(t)));
        if (!sym) continue;
        const type = TOKEN_TYPE_OF[sym.kind];
        if (!type) continue;
        builder.push(t.line, t.col, length, SEMANTIC_TOKENS_LEGEND.tokenTypes.indexOf(type),
            decl ? 1 : 0);
    }
    return builder.build();
}

// ===== 4. Completion =====

/**
 * `expr.` の `expr` が何型かを、直前の識別子から素直に引く。
 *
 * 戻り値は 3 通りを区別する:
 *   - `undefined`     … `.` の直後ではない（通常のスコープ補完）
 *   - `{ type: … }`   … 受け手の型が分かった（そのメンバを出す）
 *   - `{ type: null }`… `.` の直後だが型が分からない
 *
 * 3 つ目を 1 つ目と混同してはいけない。混同すると「型が引けなかった `.` の後ろに
 * スコープ内の全名前が出る」ことになり、`c.` に 54 件並んだ。
 */
function receiverTypeAt(
    analysis: Analysis,
    document: vscode.TextDocument,
    position: vscode.Position,
): { type: string | null } | undefined {
    // `.` の直前にある識別子トークンを探す。
    //
    // 旧実装は行を `maskLine()` で潰してから `/([A-Za-z_]\w*)\s*\.\s*$/` を当てていた。
    // トークンで見れば、コメント・文字列の中の `.` を誤認する余地が最初から無く
    // （`# functions.ar — …` で補完が出ていた）、`.` と名前の間に改行があっても効く。
    const dotIdx = tokenIndexBefore(analysis, position);
    if (dotIdx < 0) return undefined;
    const dot = analysis.tokens[dotIdx];
    if (dot.kind !== 'op' || document.getText(tokenRange(dot)) !== '.') return undefined;

    const recv = analysis.tokens[dotIdx - 1];
    if (!recv || recv.kind !== 'ident') return { type: null };
    const name = document.getText(tokenRange(recv));

    const known = (t: string | null | undefined) =>
        !!t && (analysis.members[t] !== undefined || preludeMembers[t] !== undefined);

    // 受け手そのものが型名（`MyEnum.` / `MyClass.`）ならその型。
    if (known(name)) return { type: name };

    const sym = lookup(analysis, name, recv.line);
    if (known(sym?.typeAnn)) return { type: sym!.typeAnn };
    if (known(sym?.inferred)) return { type: sym!.inferred };

    // 注釈が無ければ型検査器の推論型を使う（node の位置は式の末尾＝名前の終端）。
    const inferred = exprTypeAt(analysis, new vscode.Position(recv.line, recv.endCol));
    if (known(inferred)) return { type: inferred! };

    return { type: null };
}

/** 継承元トレイトのメンバも含めて集める。 */
function membersOf(analysis: Analysis, typeName: string, seen = new Set<string>()): Member[] {
    if (seen.has(typeName)) return [];
    seen.add(typeName);
    const table = analysis.members[typeName] ?? preludeMembers[typeName];
    if (!table) return [];
    const out = [...table.members];
    for (const base of table.bases) out.push(...membersOf(analysis, base, seen));
    return out;
}

export function provideCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position,
): vscode.CompletionItem[] {
    const analysis = getAnalysis(document);
    if (!analysis) return [];

    // コメント・文字列の中では何も出さない。
    // 旧実装は `maskLine()` で受け手の誤認（`# … functions.ar …` で `functions.` の
    // メンバが出た）だけは防いでいたが、スコープ補完はそのまま出ていた。
    // いまは lexer が「ここはコメント/文字列」と答えるので、まとめて止められる。
    const here = tokenAt(analysis, position);
    if (here && (here.kind === 'comment' || here.kind === 'str')) return [];

    const receiver = receiverTypeAt(analysis, document, position);
    if (receiver) {
        // `.` の直後で型が引けないなら**何も出さない**。スコープ内の名前を出すと
        // 明らかに無関係な候補が並ぶ（`c.` に 54 件出た）。
        if (receiver.type === null) return [];
        // 同名メンバは 1 件に畳む。クラスが明示 `__init__` を持ちつつ自動生成版も
        // AST に載る（シグネチャが違えば両方残る）ため、素直に出すと二重に見える。
        const uniq = new Map<string, Member>();
        for (const mem of membersOf(analysis, receiver.type)) {
            if (!uniq.has(mem.name)) uniq.set(mem.name, mem);
        }
        const resolved: string = receiver.type;
        void resolved;
        return [...uniq.values()].map(mem => {
            const item = new vscode.CompletionItem(mem.name, completionKindOf(mem.kind));
            item.detail = mem.params
                ? `${mem.name}(${mem.params.filter(p => p.name !== 'self').map(p => p.label).join(', ')})`
                  + (mem.type ? ` -> ${mem.type}` : '')
                : (mem.type ?? '');
            if (mem.doc) item.documentation = new vscode.MarkdownString(mem.doc);
            // private メンバは候補の末尾へ回す（消さないのは、クラス内からは正当なため）。
            item.sortText = mem.access === 'public' ? `0${mem.name}` : `9${mem.name}`;
            return item;
        });
    }

    // `.` の後ろでなければ、その位置から**見えている**名前だけを返す。
    // ただし型注釈位置なら型だけに絞る（`let x: ` の後ろに変数や関数を出さない）。
    const wantTypes = inTypePosition(analysis, document, position);
    return visibleSymbols(analysis, position.line)
        .filter(sym => !wantTypes || TYPE_TOKEN_OF[sym.kind] !== undefined)
        .map(sym => {
            const item = new vscode.CompletionItem(sym.name, completionKindOf(sym.kind));
            item.detail = renderSignature(sym);
            if (sym.doc) item.documentation = new vscode.MarkdownString(sym.doc);
            return item;
        });
}

/**
 * カーソルが型注釈位置にいるか。
 *
 * `typeRefs` は使えない — 補完は入力途中に呼ばれるので構文エラー中がほとんどで、
 * そのとき `typeRefs` は lastGood（**古いテキスト**）由来になり位置が合わない。
 * トークン列は現在のテキストのものが必ず来るので、そちらで判定する。
 *
 * 判定は「直前の意味のあるトークンが `:` か `->`」。`:` は辞書リテラルや
 * スライスでも使うが、そこで型候補が出ても実害が小さく、逆に
 * `let x: ` を取りこぼすほうが痛い。
 */
function inTypePosition(
    analysis: Analysis,
    document: vscode.TextDocument,
    position: vscode.Position,
): boolean {
    let i = tokenIndexBefore(analysis, position);
    // 打ちかけの識別子（`let x: in|`）は読み飛ばして、その手前を見る。
    if (i >= 0 && analysis.tokens[i].kind === 'ident') i--;
    if (i < 0) return false;
    const t = analysis.tokens[i];
    if (t.kind !== 'op') return false;
    const text = document.getText(tokenRange(t));
    return text === ':' || text === '->';
}

// ===== 5. Signature help =====

/**
 * カーソルが入っている呼び出しの「関数名」と「今何番目の引数か」を返す。
 *
 * トークン列を後ろ向きにたどるので、**呼び出しが複数行にまたがっていても効く**。
 * 旧実装は `document.lineAt(position.line)` で 1 行しか見ておらず、
 * 引数を改行で並べた呼び出しではシグネチャヘルプが出ないままだった。
 */
function callContext(
    analysis: Analysis,
    document: vscode.TextDocument,
    position: vscode.Position,
): { name: string; argIndex: number } | undefined {
    let i = tokenIndexBefore(analysis, position);
    let depth = 0;
    let argIndex = 0;
    for (; i >= 0; i--) {
        const t = analysis.tokens[i];
        if (t.kind !== 'op') continue;
        const text = document.getText(tokenRange(t));
        if (text === ')' || text === ']' || text === '}') depth++;
        else if (text === '[' || text === '{') {
            if (depth === 0) return undefined;   // 呼び出しではなく添字/辞書の中
            depth--;
        } else if (text === '(') {
            if (depth === 0) {
                const head = analysis.tokens[i - 1];
                if (!head || head.kind !== 'ident') return undefined;
                return { name: document.getText(tokenRange(head)), argIndex };
            }
            depth--;
        } else if (text === ',' && depth === 0) argIndex++;
    }
    return undefined;
}

export function provideSignatureHelp(
    document: vscode.TextDocument,
    position: vscode.Position,
): vscode.SignatureHelp | undefined {
    const analysis = getAnalysis(document);
    if (!analysis) return undefined;
    const ctx = callContext(analysis, document, position);
    if (!ctx) return undefined;

    // 関数・ジェネレータ、またはクラス名（＝自動生成コンストラクタ）。
    const sym = lookup(analysis, ctx.name, position.line);
    let label: string | undefined;
    let params: string[] = [];

    if (sym && (sym.kind === 'function' || sym.kind === 'generator') && sym.signature) {
        label = sym.signature;
        params = sym.bases.filter(p => p !== 'self');
    } else if (sym && sym.kind === 'class') {
        const init = membersOf(analysis, sym.name).find(m => m.name === '__init__');
        const ps = (init?.params ?? []).filter(p => p.name !== 'self');
        label = `${sym.name}(${ps.map(p => p.label).join(', ')})`;
        params = ps.map(p => p.label);
    }
    if (!label) return undefined;

    const info = new vscode.SignatureInformation(label);
    info.parameters = params.map(p => new vscode.ParameterInformation(p));
    if (sym?.doc) info.documentation = new vscode.MarkdownString(sym.doc);

    const help = new vscode.SignatureHelp();
    help.signatures = [info];
    help.activeSignature = 0;
    help.activeParameter = Math.min(ctx.argIndex, Math.max(params.length - 1, 0));
    return help;
}

// ===== 6. Go to definition =====

export function provideDefinition(
    document: vscode.TextDocument,
    position: vscode.Position,
): vscode.Location | undefined {
    const analysis = getAnalysis(document);
    if (!analysis) return undefined;
    const w = wordAt(document, analysis, position);
    if (!w) return undefined;

    // `obj.member` の member なら、受け手の型のメンバ宣言へ飛ばす。
    // 直前のトークンが `.` かをトークン列で見る（行頭が `.` の継続行でも効く）。
    const prev = analysis.tokens[tokenIndexAt(analysis, position) - 1];
    if (prev && prev.kind === 'op' && document.getText(tokenRange(prev)) === '.') {
        const recv = receiverTypeAt(analysis, document,
            new vscode.Position(prev.endLine, prev.endCol));
        if (recv?.type) {
            const member = analysis.symbols.find(s => s.container === recv.type && s.name === w.word);
            if (member) {
                return new vscode.Location(document.uri,
                    new vscode.Position(member.at.line, member.at.col));
            }
        }
    }

    const sym = lookup(analysis, w.word, position.line);
    if (!sym) return undefined;
    // 組み込み（builtins.ars 由来）はこのファイルの中に定義が無いので飛ばない。
    if (prelude.includes(sym)) return undefined;
    return new vscode.Location(document.uri, new vscode.Position(sym.at.line, sym.at.col));
}

// ===== 7. Document symbols（アウトライン） =====

export function provideDocumentSymbols(
    document: vscode.TextDocument,
): vscode.DocumentSymbol[] {
    const analysis = getAnalysis(document);
    if (!analysis) return [];

    const byScope = new Map<number, vscode.DocumentSymbol[]>();
    const containers = new Map<number, vscode.DocumentSymbol>();

    // 宣言順に作る（親は子より先に現れる）。
    for (const sym of analysis.symbols) {
        if (sym.kind === 'param') continue;
        const range = new vscode.Range(
            sym.at.line, sym.at.col,
            sym.at.line, sym.at.col + sym.name.length,
        );
        const node = new vscode.DocumentSymbol(
            sym.name, renderSignature(sym), symbolKindOf(sym.kind), range, range);
        if (sym.bodyScope !== null && sym.bodyScope !== undefined) {
            containers.set(sym.bodyScope, node);
        }
        const list = byScope.get(sym.scope) ?? [];
        list.push(node);
        byScope.set(sym.scope, list);
    }

    // 子スコープの宣言を、そのスコープを本体に持つ宣言へぶら下げる。
    for (const [scopeId, nodes] of byScope) {
        const parent = containers.get(scopeId);
        if (parent) parent.children = nodes;
    }
    return byScope.get(0) ?? [];
}

// ===== 8. Diagnostics =====

export function provideDiagnostics(document: vscode.TextDocument): vscode.Diagnostic[] {
    if (!isFrontendReady()) return [];

    const key = document.uri.toString();
    const entry = cache.get(key);
    // 解析を（必要なら）走らせてキャッシュを更新する。
    getAnalysis(document);
    const updated = cache.get(key);

    // 構文エラー中は型診断を出さない。壊れた AST から出るエラーは的外れになるうえ、
    // 打っている最中ずっと赤線が点滅する。構文エラー自体だけを 1 件出す。
    if (freshParseFailed(document)) {
        const raw = analyze(document.getText()) as Analysis | null;
        const message = raw?.parseError ?? 'parse error';
        // 位置はパーサが控えたもの（止まったトークン）。以前はエラー文章を
        // 正規表現で読み直していたが、あれは「メッセージの書き方」に依存する推測だった。
        const at = raw?.parseErrorAt ?? null;
        let range: vscode.Range;
        if (at) {
            // 波線はそのトークン 1 個分。トークン列は構文エラー中も現在のテキストのもの。
            const t = raw?.tokens
                ? tokenAt(raw, new vscode.Position(at.line, at.col))
                : undefined;
            range = t ? tokenRange(t) : new vscode.Range(at.line, at.col, at.line, at.col + 1);
        } else {
            range = document.lineAt(Math.max(document.lineCount - 1, 0)).range;
        }
        const d = new vscode.Diagnostic(range, message, vscode.DiagnosticSeverity.Error);
        d.source = 'arrow';
        return [d];
    }

    const analysis = updated?.fresh ?? entry?.fresh;
    if (!analysis) return [];
    return analysis.diagnostics.map(d => toDiagnostic(document, analysis, d));
}

function toDiagnostic(
    document: vscode.TextDocument,
    analysis: Analysis,
    d: WasmDiagnostic,
): vscode.Diagnostic {
    let range: vscode.Range;
    if (d.at) {
        // 波線の長さは、その位置にあるトークンの実際の長さ。旧実装は
        // `/[A-Za-z_]\w*/` を当てていたので、演算子や文字列リテラルを指す診断では
        // 常に 1 文字になっていた。
        const t = tokenAt(analysis, new vscode.Position(d.at.line, d.at.col));
        range = t
            ? tokenRange(t)
            : new vscode.Range(d.at.line, d.at.col, d.at.line, d.at.col + 1);
    } else {
        // 位置が無い診断（型検査器が span を持たないケース）はファイル先頭 1 文字に置く。
        // 捨てないのは、`class_trait_error.ar` の「mut フィールドに既定値」のように
        // 実際のエラーがここにしか現れないものがあるため。
        range = new vscode.Range(0, 0, 0, 1);
    }
    const diag = new vscode.Diagnostic(
        range, d.message,
        d.severity === 0 ? vscode.DiagnosticSeverity.Error : vscode.DiagnosticSeverity.Warning,
    );
    diag.source = d.source || 'arrow';
    return diag;
}
