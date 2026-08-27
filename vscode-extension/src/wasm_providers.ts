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
    members: Record<string, MemberTable>;
}

// ===== キャッシュ =====

interface CacheEntry {
    /** このバージョンの解析結果。構文エラーなら `null`。 */
    fresh: Analysis | null;
    /** 直近で構文が通ったときの結果。入力途中の穴埋めに使う。 */
    lastGood: Analysis | null;
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
        return entry.fresh ?? entry.lastGood;
    }

    const result = analyze(document.getText()) as Analysis | null;
    const lastGood = entry?.lastGood ?? null;

    if (!result) {
        // wasm 自体が使えない。旧実装へのフォールバックはしない（二重実装を残さないため）。
        cache.set(key, { fresh: null, lastGood, version: document.version });
        return lastGood;
    }
    if (!result.ok) {
        cache.set(key, { fresh: null, lastGood, version: document.version });
        return lastGood;
    }
    cache.set(key, { fresh: result, lastGood: result, version: document.version });
    return result;
}

/** ドキュメントが閉じられたらキャッシュを捨てる。 */
export function forgetDocument(document: vscode.TextDocument): void {
    cache.delete(document.uri.toString());
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

/** 位置 `pos` にある単語（識別子）とその範囲。識別子でなければ null。 */
function wordAt(document: vscode.TextDocument, pos: vscode.Position): { word: string; range: vscode.Range } | null {
    const range = document.getWordRangeAtPosition(pos, /[A-Za-z_]\w*/);
    if (!range) return null;
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
    const w = wordAt(document, position);
    if (!w) return undefined;

    const sym = declarationAt(analysis, position, w.word) ?? lookup(analysis, w.word, position.line);
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

/** コメントと文字列を空白に潰した行を返す（識別子走査で誤検出しないため）。 */
function maskLine(text: string): string {
    let out = '';
    let quote: string | null = null;
    for (let i = 0; i < text.length; i++) {
        const c = text[i];
        if (quote) {
            out += ' ';
            if (c === '\\') { out += ' '; i++; continue; }
            if (c === quote) quote = null;
            continue;
        }
        if (c === '"' || c === "'") { quote = c; out += ' '; continue; }
        if (c === '#') { out += ' '.repeat(text.length - i); break; }
        out += c;
    }
    return out;
}

export function provideDocumentSemanticTokens(
    document: vscode.TextDocument,
): vscode.SemanticTokens {
    const builder = new vscode.SemanticTokensBuilder(SEMANTIC_TOKENS_LEGEND);
    const analysis = getAnalysis(document);
    if (!analysis) return builder.build();

    // 宣言そのものには declaration 修飾を付ける。
    const declAt = new Map<string, Symbol>();
    for (const s of analysis.symbols) declAt.set(`${s.at.line}:${s.at.col}`, s);

    const ident = /[A-Za-z_]\w*/g;
    for (let line = 0; line < document.lineCount; line++) {
        const text = maskLine(document.lineAt(line).text);
        const visible = visibleSymbols(analysis, line);
        const byName = new Map<string, Symbol>();
        for (const s of visible) if (!byName.has(s.name)) byName.set(s.name, s);
        // メンバ名（`p.x` の `x`）も色を付けたいので、全クラスのメンバを名前で引けるようにする。
        ident.lastIndex = 0;
        let m: RegExpExecArray | null;
        while ((m = ident.exec(text)) !== null) {
            const decl = declAt.get(`${line}:${m.index}`);
            const sym = decl ?? byName.get(m[0]);
            if (!sym) continue;
            const type = TOKEN_TYPE_OF[sym.kind];
            if (!type) continue;
            builder.push(line, m.index, m[0].length, SEMANTIC_TOKENS_LEGEND.tokenTypes.indexOf(type),
                decl ? 1 : 0);
        }
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
    // コメント・文字列は潰してから見る。潰さないと `# functions.ar — …` のような
    // 行で `functions.` を受け手だと誤認する（実際に補完が出た）。
    const before = maskLine(document.lineAt(position.line).text).slice(0, position.character);
    const m = /([A-Za-z_]\w*)\s*\.\s*$/.exec(before);
    if (!m) return undefined;
    const name = m[1];

    const known = (t: string | null | undefined) =>
        !!t && (analysis.members[t] !== undefined || preludeMembers[t] !== undefined);

    // 受け手そのものが型名（`MyEnum.` / `MyClass.`）ならその型。
    if (known(name)) return { type: name };

    const sym = lookup(analysis, name, position.line);
    if (known(sym?.typeAnn)) return { type: sym!.typeAnn };
    if (known(sym?.inferred)) return { type: sym!.inferred };

    // 注釈が無ければ型検査器の推論型を使う。
    const inferred = exprTypeAt(analysis, new vscode.Position(position.line, m.index + name.length));
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
    return visibleSymbols(analysis, position.line).map(sym => {
        const item = new vscode.CompletionItem(sym.name, completionKindOf(sym.kind));
        item.detail = renderSignature(sym);
        if (sym.doc) item.documentation = new vscode.MarkdownString(sym.doc);
        return item;
    });
}

// ===== 5. Signature help =====

/** カーソルが入っている呼び出しの「関数名」と「今何番目の引数か」を返す。 */
function callContext(document: vscode.TextDocument, position: vscode.Position):
    { name: string; argIndex: number } | undefined {
    const text = maskLine(document.lineAt(position.line).text).slice(0, position.character);
    let depth = 0;
    let argIndex = 0;
    for (let i = text.length - 1; i >= 0; i--) {
        const c = text[i];
        if (c === ')') depth++;
        else if (c === '(') {
            if (depth === 0) {
                const head = /([A-Za-z_]\w*)\s*$/.exec(text.slice(0, i));
                return head ? { name: head[1], argIndex } : undefined;
            }
            depth--;
        } else if (c === ',' && depth === 0) argIndex++;
    }
    return undefined;
}

export function provideSignatureHelp(
    document: vscode.TextDocument,
    position: vscode.Position,
): vscode.SignatureHelp | undefined {
    const analysis = getAnalysis(document);
    if (!analysis) return undefined;
    const ctx = callContext(document, position);
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
    const w = wordAt(document, position);
    if (!w) return undefined;

    // `obj.member` の member なら、受け手の型のメンバ宣言へ飛ばす。
    const before = document.lineAt(position.line).text.slice(0, w.range.start.character);
    if (/\.\s*$/.test(before)) {
        const recvPos = new vscode.Position(position.line, Math.max(before.lastIndexOf('.'), 0));
        const recv = receiverTypeAt(analysis, document, new vscode.Position(recvPos.line, recvPos.character + 1));
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
        const raw = analyze(document.getText());
        const message = raw?.parseError ?? 'parse error';
        const at = parseErrorPosition(message);
        const range = at
            ? new vscode.Range(at.line, at.col, at.line, at.col + 1)
            : document.lineAt(Math.max(document.lineCount - 1, 0)).range;
        const d = new vscode.Diagnostic(range, message, vscode.DiagnosticSeverity.Error);
        d.source = 'arrow';
        return [d];
    }

    const analysis = updated?.fresh ?? entry?.fresh;
    if (!analysis) return [];
    return analysis.diagnostics.map(d => toDiagnostic(document, d));
}

/** `line 12, col 5` のような位置がメッセージに含まれていれば取り出す。 */
function parseErrorPosition(message: string): { line: number; col: number } | undefined {
    const m = /line\s+(\d+)[,:]?\s*(?:col(?:umn)?\s+(\d+))?/i.exec(message);
    if (!m) return undefined;
    return { line: Math.max(parseInt(m[1], 10) - 1, 0), col: Math.max(parseInt(m[2] ?? '1', 10) - 1, 0) };
}

function toDiagnostic(document: vscode.TextDocument, d: WasmDiagnostic): vscode.Diagnostic {
    let range: vscode.Range;
    if (d.at) {
        const lineText = document.lineAt(Math.min(d.at.line, document.lineCount - 1)).text;
        const wordEnd = /[A-Za-z_]\w*/.exec(lineText.slice(d.at.col));
        const len = wordEnd && wordEnd.index === 0 ? wordEnd[0].length : 1;
        range = new vscode.Range(d.at.line, d.at.col, d.at.line, d.at.col + len);
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
