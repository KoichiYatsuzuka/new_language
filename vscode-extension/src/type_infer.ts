import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';

export type LangType = string;

// ===== Tokenizer =====

type TK =
    | 'INT' | 'FLOAT' | 'STR'
    | 'TRUE' | 'FALSE' | 'NONE'
    | 'IDENT'
    | 'PLUS' | 'MINUS' | 'STAR' | 'SLASH' | 'SLASHSLASH' | 'PERCENT' | 'STARSTAR'
    | 'EQEQ' | 'NOTEQ' | 'LT' | 'GT' | 'LTEQ' | 'GTEQ'
    | 'AND' | 'OR' | 'NOT'
    | 'AMP' | 'PIPE' | 'CARET' | 'TILDE' | 'LTLT' | 'GTGT'
    | 'LPAREN' | 'RPAREN' | 'LBRACKET' | 'RBRACKET' | 'LBRACE' | 'RBRACE'
    | 'COMMA' | 'COLON'
    | 'OTHER' | 'EOF';

interface Token { kind: TK; value: string; }

function tokenize(src: string): Token[] {
    const tokens: Token[] = [];
    let i = 0;

    while (i < src.length) {
        if (' \t\r\n'.includes(src[i])) { i++; continue; }

        if (src[i] === '"' || src[i] === "'") {
            const q = src[i];
            const triple = src.startsWith(q + q + q, i);
            let j = i + (triple ? 3 : 1);
            while (j < src.length) {
                if (src[j] === '\\') { j += 2; continue; }
                if (triple ? src.startsWith(q + q + q, j) : src[j] === q) { j += triple ? 3 : 1; break; }
                j++;
            }
            tokens.push({ kind: 'STR', value: src.slice(i, j) });
            i = j;
            continue;
        }

        if (/\d/.test(src[i])) {
            let j = i;
            if (src[j] === '0' && j + 1 < src.length && 'xXoObB'.includes(src[j + 1])) {
                j += 2;
                while (j < src.length && /[\da-fA-F_]/.test(src[j])) j++;
                tokens.push({ kind: 'INT', value: src.slice(i, j) });
            } else {
                while (j < src.length && /[\d_]/.test(src[j])) j++;
                let isFloat = false;
                if (j < src.length && src[j] === '.' && j + 1 < src.length && /\d/.test(src[j + 1])) {
                    isFloat = true; j++;
                    while (j < src.length && /[\d_]/.test(src[j])) j++;
                }
                if (j < src.length && 'eE'.includes(src[j])) {
                    isFloat = true; j++;
                    if (j < src.length && '+-'.includes(src[j])) j++;
                    while (j < src.length && /\d/.test(src[j])) j++;
                }
                tokens.push({ kind: isFloat ? 'FLOAT' : 'INT', value: src.slice(i, j) });
            }
            i = j;
            continue;
        }

        if (/[A-Za-z_]/.test(src[i])) {
            let j = i;
            while (j < src.length && /\w/.test(src[j])) j++;
            const word = src.slice(i, j);
            const keywordMap: Record<string, TK> = {
                True: 'TRUE', False: 'FALSE', None: 'NONE',
                and: 'AND', or: 'OR', not: 'NOT',
            };
            tokens.push({ kind: keywordMap[word] ?? 'IDENT', value: word });
            i = j;
            continue;
        }

        const s3 = src.slice(i, i + 3);
        if (['//=', '**=', '<<=', '>>='].includes(s3)) { tokens.push({ kind: 'OTHER', value: s3 }); i += 3; continue; }
        const s2 = src.slice(i, i + 2);
        const op2: Record<string, TK> = {
            '**': 'STARSTAR', '//': 'SLASHSLASH', '==': 'EQEQ', '!=': 'NOTEQ',
            '<=': 'LTEQ', '>=': 'GTEQ', '<<': 'LTLT', '>>': 'GTGT',
        };
        if (op2[s2]) { tokens.push({ kind: op2[s2], value: s2 }); i += 2; continue; }
        if (['+=', '-=', '*=', '/=', '%=', '&=', '|=', '^=', '@=', '->', ':='].includes(s2)) {
            tokens.push({ kind: 'OTHER', value: s2 }); i += 2; continue;
        }

        const op1: Record<string, TK> = {
            '+': 'PLUS', '-': 'MINUS', '*': 'STAR', '/': 'SLASH', '%': 'PERCENT',
            '<': 'LT', '>': 'GT', '&': 'AMP', '|': 'PIPE', '^': 'CARET', '~': 'TILDE',
            '(': 'LPAREN', ')': 'RPAREN', ',': 'COMMA',
            '[': 'LBRACKET', ']': 'RBRACKET', '{': 'LBRACE', '}': 'RBRACE', ':': 'COLON',
        };
        tokens.push({ kind: op1[src[i]] ?? 'OTHER', value: src[i] });
        i++;
    }

    tokens.push({ kind: 'EOF', value: '' });
    return tokens;
}

// ===== Expression type inferrer =====

const BUILTIN_RETURN_TYPES: Record<string, LangType> = {
    print: 'None', exec: 'None',
    len: 'int', id: 'int', hash: 'int', ord: 'int', round: 'int',
    chr: 'str', hex: 'str', oct: 'str', bin: 'str', repr: 'str', input: 'str', format: 'str',
    int: 'int', float: 'float', str: 'str', bool: 'bool',
    isinstance: 'bool', issubclass: 'bool', callable: 'bool', hasattr: 'bool',
    abs: 'unknown', max: 'unknown', min: 'unknown', sum: 'unknown',
    range: 'unknown', enumerate: 'unknown', zip: 'unknown', map: 'unknown', filter: 'unknown',
    sorted: 'unknown', reversed: 'unknown', getattr: 'unknown', next: 'unknown',
    iter: 'unknown', open: 'unknown', eval: 'unknown', globals: 'unknown',
    locals: 'unknown', vars: 'unknown', dir: 'unknown', super: 'unknown', type: 'unknown',
};

function mergeNumeric(a: LangType, b: LangType): LangType {
    if (a === 'float' || b === 'float') return 'float';
    if (a === 'int' && b === 'int') return 'int';
    return 'unknown';
}

// Maps operator token kind → [dunder method, reverse dunder method]
const OP_DUNDER: Partial<Record<TK, [string, string]>> = {
    PLUS:        ['__add__',      '__radd__'],
    MINUS:       ['__sub__',      '__rsub__'],
    STAR:        ['__mul__',      '__rmul__'],
    SLASH:       ['__truediv__',  '__rtruediv__'],
    SLASHSLASH:  ['__floordiv__', '__rfloordiv__'],
    PERCENT:     ['__mod__',      '__rmod__'],
    STARSTAR:    ['__pow__',      '__rpow__'],
    AMP:         ['__and__',      '__rand__'],
    PIPE:        ['__or__',       '__ror__'],
    CARET:       ['__xor__',      '__rxor__'],
    LTLT:        ['__lshift__',   '__rlshift__'],
    GTGT:        ['__rshift__',   '__rrshift__'],
};

class ExprInferrer {
    private pos = 0;
    constructor(
        private readonly tokens: Token[],
        private readonly env: Map<string, LangType>,
        private readonly funcEnv: ReadonlyMap<string, LangType>,
        private readonly pyClassMethods: ReadonlyMap<string, ReadonlyMap<string, string>> = new Map()
    ) {}

    private cur(): Token { return this.tokens[this.pos] ?? { kind: 'EOF', value: '' }; }
    private eat(): Token { return this.tokens[this.pos++] ?? { kind: 'EOF', value: '' }; }

    infer(): LangType { return this.parseOr(); }

    private applyBinaryOp(left: LangType, right: LangType, op: TK): LangType {
        const dunder = OP_DUNDER[op];
        if (dunder) {
            const lm = this.pyClassMethods.get(left);
            if (lm?.has(dunder[0])) return lm.get(dunder[0])!;
            const rm = this.pyClassMethods.get(right);
            if (rm?.has(dunder[1])) return rm.get(dunder[1])!;
        }
        if (op === 'SLASH') return 'float';
        return mergeNumeric(left, right);
    }

    private parseOr(): LangType {
        let t = this.parseAnd();
        while (this.cur().kind === 'OR') { this.eat(); this.parseAnd(); t = 'bool'; }
        return t;
    }

    private parseAnd(): LangType {
        let t = this.parseNot();
        while (this.cur().kind === 'AND') { this.eat(); this.parseNot(); t = 'bool'; }
        return t;
    }

    private parseNot(): LangType {
        if (this.cur().kind === 'NOT') { this.eat(); this.parseNot(); return 'bool'; }
        return this.parseComparison();
    }

    private parseComparison(): LangType {
        const left = this.parseBitOr();
        const cmpOps: TK[] = ['EQEQ', 'NOTEQ', 'LT', 'GT', 'LTEQ', 'GTEQ'];
        if (cmpOps.includes(this.cur().kind)) { this.eat(); this.parseBitOr(); return 'bool'; }
        return left;
    }

    private parseBitOr(): LangType {
        let t = this.parseBitXor();
        while (this.cur().kind === 'PIPE') { this.eat(); t = this.applyBinaryOp(t, this.parseBitXor(), 'PIPE'); }
        return t;
    }

    private parseBitXor(): LangType {
        let t = this.parseBitAnd();
        while (this.cur().kind === 'CARET') { this.eat(); t = this.applyBinaryOp(t, this.parseBitAnd(), 'CARET'); }
        return t;
    }

    private parseBitAnd(): LangType {
        let t = this.parseShift();
        while (this.cur().kind === 'AMP') { this.eat(); t = this.applyBinaryOp(t, this.parseShift(), 'AMP'); }
        return t;
    }

    private parseShift(): LangType {
        let t = this.parseAdditive();
        while (this.cur().kind === 'LTLT' || this.cur().kind === 'GTGT') {
            const op = this.eat().kind;
            t = this.applyBinaryOp(t, this.parseAdditive(), op);
        }
        return t;
    }

    private parseAdditive(): LangType {
        let t = this.parseMultiplicative();
        while (this.cur().kind === 'PLUS' || this.cur().kind === 'MINUS') {
            const op = this.eat().kind;
            const r = this.parseMultiplicative();
            t = (op === 'PLUS' && t === 'str' && r === 'str') ? 'str' : this.applyBinaryOp(t, r, op);
        }
        return t;
    }

    private parseMultiplicative(): LangType {
        let t = this.parseUnary();
        const ops: TK[] = ['STAR', 'SLASH', 'SLASHSLASH', 'PERCENT'];
        while (ops.includes(this.cur().kind)) {
            const op = this.eat().kind;
            t = this.applyBinaryOp(t, this.parseUnary(), op);
        }
        return t;
    }

    private parseUnary(): LangType {
        if (this.cur().kind === 'MINUS' || this.cur().kind === 'PLUS') { this.eat(); return this.parseUnary(); }
        if (this.cur().kind === 'TILDE') { this.eat(); this.parseUnary(); return 'int'; }
        return this.parsePower();
    }

    private parsePower(): LangType {
        const base = this.parsePrimary();
        if (this.cur().kind === 'STARSTAR') { this.eat(); return this.applyBinaryOp(base, this.parseUnary(), 'STARSTAR'); }
        return base;
    }

    private parsePrimary(): LangType {
        const tok = this.cur();
        switch (tok.kind) {
            case 'INT':   this.eat(); return 'int';
            case 'FLOAT': this.eat(); return 'float';
            case 'STR':   this.eat(); return 'str';
            case 'TRUE':
            case 'FALSE': this.eat(); return 'bool';
            case 'NONE':  this.eat(); return 'None';
            case 'IDENT': {
                const name = tok.value;
                this.eat();
                let isChained = false;
                while (this.cur().kind === 'OTHER' && this.cur().value === '.') {
                    this.eat();
                    if (this.cur().kind === 'IDENT') this.eat();
                    isChained = true;
                }
                if (this.cur().kind === 'LPAREN') {
                    this.eat();
                    while (this.cur().kind !== 'RPAREN' && this.cur().kind !== 'EOF') {
                        this.parseOr();
                        if (this.cur().kind === 'COMMA') this.eat(); else break;
                    }
                    if (this.cur().kind === 'RPAREN') this.eat();
                    if (isChained) return 'unknown';
                    if (name in BUILTIN_RETURN_TYPES) return BUILTIN_RETURN_TYPES[name];
                    return this.funcEnv.get(name) ?? 'unknown';
                }
                if (isChained) return 'unknown';
                return this.env.get(name) ?? 'unknown';
            }
            case 'LPAREN': {
                this.eat();
                if (this.cur().kind === 'RPAREN') { this.eat(); return 'tuple'; }
                const first = this.parseOr();
                if (this.cur().kind === 'COMMA') {
                    const types: LangType[] = [first];
                    while (this.cur().kind === 'COMMA') {
                        this.eat();
                        if (this.cur().kind === 'RPAREN') break;
                        types.push(this.parseOr());
                    }
                    if (this.cur().kind === 'RPAREN') this.eat();
                    return `tuple[${types.join(', ')}]`;
                }
                if (this.cur().kind === 'RPAREN') this.eat();
                return first;
            }
            case 'LBRACKET': {
                this.eat();
                if (this.cur().kind === 'RBRACKET') { this.eat(); return 'list'; }
                const elemTypes: LangType[] = [];
                while (this.cur().kind !== 'RBRACKET' && this.cur().kind !== 'EOF') {
                    elemTypes.push(this.parseOr());
                    if (this.cur().kind === 'COMMA') this.eat(); else break;
                }
                if (this.cur().kind === 'RBRACKET') this.eat();
                const uniqElems = [...new Set(elemTypes)];
                return uniqElems.length === 1 ? `list[${uniqElems[0]}]` : 'list';
            }
            case 'LBRACE': {
                this.eat();
                if (this.cur().kind === 'RBRACE') { this.eat(); return 'dict'; }
                const keyTypes: LangType[] = [];
                const valTypes: LangType[] = [];
                while (this.cur().kind !== 'RBRACE' && this.cur().kind !== 'EOF') {
                    keyTypes.push(this.parseOr());
                    if (this.cur().kind === 'COLON') { this.eat(); valTypes.push(this.parseOr()); }
                    if (this.cur().kind === 'COMMA') this.eat(); else break;
                }
                if (this.cur().kind === 'RBRACE') this.eat();
                const uniqK = [...new Set(keyTypes)];
                const uniqV = [...new Set(valTypes)];
                if (uniqK.length === 1 && uniqV.length === 1) return `dict[${uniqK[0]}, ${uniqV[0]}]`;
                return 'dict';
            }
            default:
                this.eat();
                return 'unknown';
        }
    }
}

export function inferExprType(
    src: string,
    env: Map<string, LangType>,
    funcEnv: ReadonlyMap<string, LangType> = new Map(),
    importAliases: ReadonlySet<string> = new Set(),
    importFuncTypes: ReadonlyMap<string, ReadonlyMap<string, string>> = new Map(),
    pyClassMethods: ReadonlyMap<string, ReadonlyMap<string, string>> = new Map()
): LangType {
    const trimmed = src.trim();
    // Intercept alias.anything before ExprInferrer to avoid alias-name leaking as type
    const dotMatch = trimmed.match(/^([A-Za-z_]\w*)\./);
    if (dotMatch && importAliases.has(dotMatch[1])) {
        const alias = dotMatch[1];
        const callMatch = trimmed.match(/^[A-Za-z_]\w*\.([A-Za-z_]\w*)\s*\(/);
        if (callMatch) {
            const memberName = callMatch[1];
            if (/^[A-Z]/.test(memberName)) return memberName;
            const pyTypes = importFuncTypes.get(alias);
            if (pyTypes) return pyTypes.get(memberName) ?? 'unknown';
        }
        return 'unknown';
    }
    return new ExprInferrer(tokenize(src), env, funcEnv, pyClassMethods).infer();
}

// ===== Strip comment (respecting strings) =====

function stripComment(line: string): string {
    let inStr = false;
    let strChar = '';
    let triple = false;
    for (let i = 0; i < line.length; i++) {
        const c = line[i];
        if (!inStr) {
            if ((c === '"' || c === "'") && line.startsWith(c + c + c, i)) {
                inStr = true; strChar = c; triple = true; i += 2;
            } else if (c === '"' || c === "'") {
                inStr = true; strChar = c; triple = false;
            } else if (c === '#') {
                return line.slice(0, i);
            }
        } else {
            if (c === '\\') { i++; continue; }
            if (triple && line.startsWith(strChar + strChar + strChar, i)) { inStr = false; i += 2; }
            else if (!triple && c === strChar) { inStr = false; }
        }
    }
    return line;
}

// ===== Regex constants =====

// Function definition — RetType can be complex: function[T]->R, function{name:T}->R, etc.
const FUNC_DEF_RE = /^(\s*)(fn|gen)\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:\s*$/;

// Variable declarations
const DECL_RE          = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*=(?!=)/;
const STATIC_DECL_RE   = /^(\s*)static\s+mut\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*=(?!=)\s*(.*)/;

const RETURN_RE        = /^return(?:\s+(.+))?$/;
const CLASS_NAME_RE    = /^\s*(?:class|trait)\s+([A-Za-z_]\w*)/;
const NEW_TYPE_NAME_RE = /^\s*new_type\s+([A-Za-z_]\w*)\s*:/;

// Hover-specific
const HOVER_DECL_RE = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*(?:=(?!=)\s*(.*))?$/;
const CLASS_DEF_RE  = /^(\s*)(class|trait)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*(?:\(([^)]*)\))?\s*:/;
const NEW_TYPE_RE   = /^(\s*)new_type\s+([A-Za-z_]\w*)\s*:\s*([A-Za-z_][\w\[\], ]*)/;

// freeze(varName)
const FREEZE_RE = /^\s*freeze\s*\(\s*([A-Za-z_]\w*)\s*\)/;

// import[py] / import[py-int]  — captures: 1=keyword, 2=module, 3=alias
const IMPORT_RE = /^\s*(import\[(?:py(?:-int)?)\])\s+([A-Za-z_]\w*)\s+as\s+([A-Za-z_]\w*)/;

// Typeguard — check is-not before is to avoid accidental match
const TYPEGUARD_IS_NOT_RE = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+not\s+([A-Za-z_]\w*)\s*:/;
const TYPEGUARD_IS_RE     = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+([A-Za-z_]\w*)\s*:/;

// ===== Types =====

type HoverKind = 'variable' | 'function' | 'class' | 'trait' | 'new_type' | 'module';

interface HoverSymbol {
    name: string;
    kind: HoverKind;
    line: number;
    scopeEndLine?: number;  // used for function params: line past function body
    mutability?: string;
    type?: string;
    signature?: string;
    doc?: string;
    traits?: string[];      // for class/trait defs: base traits they implement
    originalType?: string;  // for new_type: the base type
}

interface ScopeOverride {
    varName: string;
    narrowedType: LangType;
    startLine: number;
    endLine: number;        // exclusive
}

// ===== Utilities =====

// Split comma-separated items respecting nested brackets
function splitComma(s: string): string[] {
    const result: string[] = [];
    let depth = 0;
    let start = 0;
    for (let i = 0; i < s.length; i++) {
        if ('[({'.includes(s[i])) depth++;
        else if ('])}'.includes(s[i])) depth--;
        else if (s[i] === ',' && depth === 0) {
            result.push(s.slice(start, i));
            start = i + 1;
        }
    }
    result.push(s.slice(start));
    return result;
}

// For Option[T] is-not-None → T; for Union[A,B] is-not-A → B
function computeIsNotNarrowedType(declaredType: LangType | undefined, removedType: string): LangType | undefined {
    if (!declaredType) return undefined;
    const optMatch = declaredType.match(/^Option\[(.+)\]$/);
    if (optMatch && removedType === 'None') return optMatch[1].trim();
    const unionMatch = declaredType.match(/^Union\[(.+)\]$/);
    if (unionMatch) {
        const members = splitComma(unionMatch[1]).map(m => m.trim());
        const remaining = members.filter(m => m !== removedType);
        if (remaining.length < members.length) {
            return remaining.length === 1 ? remaining[0] : `Union[${remaining.join(', ')}]`;
        }
    }
    return undefined;
}

// Find (startLine, endLine) of the indented block that follows ifLine at ifIndent
function findBlockBounds(document: vscode.TextDocument, ifLine: number, ifIndent: number): { startLine: number; endLine: number } {
    const startLine = ifLine + 1;
    let endLine = document.lineCount;
    for (let j = startLine; j < document.lineCount; j++) {
        const raw = document.lineAt(j).text;
        if (!raw.trim()) continue;
        const indent = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (indent <= ifIndent) { endLine = j; break; }
    }
    return { startLine, endLine };
}

// Find the first line past a function body (indent drops to defIndent or below)
function findBodyEndLine(document: vscode.TextDocument, defLine: number, defIndent: number): number {
    for (let j = defLine + 1; j < document.lineCount; j++) {
        const raw = document.lineAt(j).text;
        if (!raw.trim()) continue;
        const indent = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (indent <= defIndent) return j;
    }
    return document.lineCount;
}

// Parse function parameter list into hover symbols scoped to the function body
function parseParams(paramsStr: string, defLine: number, bodyEndLine: number): HoverSymbol[] {
    const symbols: HoverSymbol[] = [];
    for (const part of splitComma(paramsStr)) {
        const trimmed = part.trim();
        if (!trimmed) continue;
        // [let|mut] name [: type]
        const m = trimmed.match(/^(?:(let|mut)\s+)?([A-Za-z_]\w*)\s*(?::\s*(.+))?$/);
        if (!m) continue;
        const [, mut, name, type] = m;
        if (name === 'self') continue;
        symbols.push({
            name,
            kind: 'variable',
            line: defLine,
            scopeEndLine: bodyEndLine,
            mutability: mut ?? 'let',
            type: type?.trim() ?? 'unknown',
        });
    }
    return symbols;
}

function cleanTypeAnnotation(value: string | undefined): string | undefined {
    const cleaned = value?.trim();
    return cleaned ? cleaned : undefined;
}

function cleanBaseName(value: string): string {
    return value.trim().replace(/\[[^\]]*\]/g, '');
}

function parseTypeAnnotation(s: string | undefined): LangType | undefined {
    if (!s) return undefined;
    const t = s.trim();
    return t || undefined;
}

// ===== Docstring extraction =====

function getDocstringAfter(document: vscode.TextDocument, line: number, indent: number): string | undefined {
    for (let i = line + 1; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const trimmed = raw.trim();
        if (!trimmed) continue;
        const lineIndent = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= indent) return undefined;
        if (!trimmed.startsWith('"""') && !trimmed.startsWith("'''")) return undefined;

        const quote = trimmed.startsWith('"""') ? '"""' : "'''";
        let text = trimmed.slice(3);
        if (text.endsWith(quote) && text.length >= 3) {
            text = text.slice(0, -3);
            return text.trim() || undefined;
        }

        const lines: string[] = [];
        if (text) lines.push(text);
        for (let j = i + 1; j < document.lineCount; j++) {
            const docLine = document.lineAt(j).text.trim();
            const end = docLine.indexOf(quote);
            if (end >= 0) {
                lines.push(docLine.slice(0, end));
                return lines.join('\n').trim() || undefined;
            }
            lines.push(docLine);
        }
        return lines.join('\n').trim() || undefined;
    }
    return undefined;
}

// ===== Function definition scanning =====

interface FuncDef {
    name: string;
    defLine: number;
    defIndent: number;
    annotation: LangType | undefined;
}

function collectConstructorTypes(document: vscode.TextDocument): Map<string, LangType> {
    const constructors = new Map<string, LangType>();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const classMatch = stripped.match(CLASS_NAME_RE);
        if (classMatch) { constructors.set(classMatch[1], classMatch[1]); continue; }
        const newTypeMatch = stripped.match(NEW_TYPE_NAME_RE);
        if (newTypeMatch) { constructors.set(newTypeMatch[1], newTypeMatch[1]); }
    }
    return constructors;
}

function collectFuncDefs(document: vscode.TextDocument): FuncDef[] {
    const defs: FuncDef[] = [];
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(FUNC_DEF_RE);
        if (!m) continue;
        const [, indentStr, , name, , retAnnotation] = m;
        defs.push({
            name,
            defLine: i,
            defIndent: indentStr.length,
            annotation: parseTypeAnnotation(retAnnotation),
        });
    }
    return defs;
}

function inferBodyReturnType(
    document: vscode.TextDocument,
    defLine: number,
    defIndent: number,
    funcEnv: ReadonlyMap<string, LangType>,
    importAliases: ReadonlySet<string> = new Set(),
    importFuncTypes: ReadonlyMap<string, ReadonlyMap<string, string>> = new Map(),
    pyClassMethods: ReadonlyMap<string, ReadonlyMap<string, string>> = new Map()
): LangType {
    const localEnv = new Map<string, LangType>();
    const returnTypes: LangType[] = [];

    for (let i = defLine + 1; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const trimmed = stripped.trim();
        if (!trimmed) continue;
        const lineIndent = (stripped.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= defIndent) break;

        const declM = stripped.match(DECL_RE);
        if (declM) {
            const rhs = stripped.slice(declM[0].length).trim();
            localEnv.set(declM[3], inferExprType(rhs, localEnv, funcEnv, importAliases, importFuncTypes, pyClassMethods));
        }

        const retM = trimmed.match(RETURN_RE);
        if (retM) {
            const retExpr = retM[1]?.trim();
            returnTypes.push(retExpr ? inferExprType(retExpr, localEnv, funcEnv, importAliases, importFuncTypes, pyClassMethods) : 'None');
        }
    }

    if (returnTypes.length === 0) return 'None';
    const unique = [...new Set(returnTypes)];
    return unique.length === 1 ? unique[0] : 'unknown';
}

// ===== Import alias collection =====

function collectImportAliases(document: vscode.TextDocument): Map<string, string> {
    const aliases = new Map<string, string>(); // alias → python module name
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(IMPORT_RE);
        if (m) aliases.set(m[3], m[2]);
    }
    return aliases;
}

// ===== Python module type collection =====

interface PyModuleInfo {
    funcs: Map<string, string>;
    classes: Map<string, Map<string, string>>;
}

function collectPyModuleInfo(moduleName: string, docDir: string): PyModuleInfo {
    const funcs = new Map<string, string>();
    const classes = new Map<string, Map<string, string>>();
    const pyPath = path.join(docDir, moduleName + '.py');
    if (!fs.existsSync(pyPath)) return { funcs, classes };
    let content: string;
    try { content = fs.readFileSync(pyPath, 'utf8'); } catch { return { funcs, classes }; }

    let currentClass: string | null = null;
    let classIndent = -1;
    const classRe = /^(\s*)class\s+([A-Za-z_]\w*)/;
    const funcRe = /^(\s*)def\s+([A-Za-z_]\w*)\s*\([^)]*\)\s*(?:->\s*(.+?))?\s*:/;

    for (const line of content.split('\n')) {
        const classM = line.match(classRe);
        if (classM) {
            currentClass = classM[2];
            classIndent = classM[1].length;
            classes.set(currentClass, new Map());
            continue;
        }
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith('#')) continue;
        const lineIndent = (line.match(/^(\s*)/)?.[1] ?? '').length;
        if (currentClass !== null && lineIndent <= classIndent) {
            currentClass = null;
            classIndent = -1;
        }
        const funcM = line.match(funcRe);
        if (funcM?.[3]) {
            const retType = funcM[3].trim().replace(/^['"]|['"]$/g, '');
            if (currentClass !== null) classes.get(currentClass)!.set(funcM[2], retType);
            else funcs.set(funcM[2], retType);
        }
    }
    return { funcs, classes };
}

function collectAllPyModuleInfo(document: vscode.TextDocument): {
    funcTypes: Map<string, Map<string, string>>;
    classMethods: Map<string, Map<string, string>>;
} {
    const funcTypes = new Map<string, Map<string, string>>();
    const classMethods = new Map<string, Map<string, string>>();
    const aliasMap = collectImportAliases(document);
    const docDir = path.dirname(document.uri.fsPath);
    for (const [alias, moduleName] of aliasMap) {
        const info = collectPyModuleInfo(moduleName, docDir);
        funcTypes.set(alias, info.funcs);
        for (const [cls, methods] of info.classes) classMethods.set(cls, methods);
    }
    return { funcTypes, classMethods };
}

// ===== Hover symbol collection =====

function collectHoverSymbols(document: vscode.TextDocument): HoverSymbol[] {
    const symbols: HoverSymbol[] = [];
    const env = new Map<string, LangType>();

    // Collect import aliases first so type inference can use them throughout
    const importAliasMap = collectImportAliases(document);
    const importAliases = new Set(importAliasMap.keys());
    const { funcTypes: importFuncTypes, classMethods: pyClassMethods } = collectAllPyModuleInfo(document);

    const funcDefs = collectFuncDefs(document);
    const funcEnv = collectConstructorTypes(document);

    for (const def of funcDefs) {
        funcEnv.set(def.name,
            def.annotation ?? inferBodyReturnType(document, def.defLine, def.defIndent, funcEnv, importAliases, importFuncTypes, pyClassMethods)
        );
    }

    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = stripComment(raw);

        // Import declaration
        const importMatch = stripped.match(IMPORT_RE);
        if (importMatch) {
            const [, importKind, moduleName, alias] = importMatch;
            symbols.push({
                name: alias,
                kind: 'module',
                line: i,
                mutability: 'const',
                type: alias,
                originalType: `${importKind} ${moduleName}`,
            });
            continue;
        }

        const funcMatch = stripped.match(FUNC_DEF_RE);
        if (funcMatch) {
            const [, indentStr, kind, name, params, retAnnotation] = funcMatch;
            const returnType = cleanTypeAnnotation(retAnnotation) ?? funcEnv.get(name) ?? 'unknown';
            symbols.push({
                name,
                kind: 'function',
                line: i,
                type: returnType,
                signature: `${kind} ${name}(${params}) -> ${returnType}`,
                doc: getDocstringAfter(document, i, indentStr.length),
            });
            const bodyEndLine = findBodyEndLine(document, i, indentStr.length);
            for (const paramSym of parseParams(params, i, bodyEndLine)) {
                symbols.push(paramSym);
            }
            continue;
        }

        const classMatch = stripped.match(CLASS_DEF_RE);
        if (classMatch) {
            const [, indentStr, kind, name, bases] = classMatch;
            const traits = bases?.split(',').map(cleanBaseName).filter(Boolean);
            symbols.push({
                name,
                kind: kind === 'trait' ? 'trait' : 'class',
                line: i,
                traits,
                doc: getDocstringAfter(document, i, indentStr.length),
            });
            continue;
        }

        const newTypeMatch = stripped.match(NEW_TYPE_RE);
        if (newTypeMatch) {
            const [, indentStr, name, originalType] = newTypeMatch;
            symbols.push({
                name,
                kind: 'new_type',
                line: i,
                type: name,
                mutability: 'const',
                originalType: originalType.trim(),
                doc: getDocstringAfter(document, i, indentStr.length),
            });
            env.set(name, 'unknown');
            continue;
        }

        const staticMatch = stripped.match(STATIC_DECL_RE);
        if (staticMatch) {
            const [, , name, annotation, rhs] = staticMatch;
            const type = cleanTypeAnnotation(annotation)
                ?? (rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods) : 'unknown');
            symbols.push({ name, kind: 'variable', line: i, mutability: 'static', type });
            env.set(name, type);
            continue;
        }

        const declMatch = stripped.match(HOVER_DECL_RE);
        if (declMatch) {
            const [, , mutability, name, annotation, rhs] = declMatch;
            const type = cleanTypeAnnotation(annotation)
                ?? (rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods) : 'unknown');
            symbols.push({ name, kind: 'variable', line: i, mutability, type });
            env.set(name, parseTypeAnnotation(type) ?? 'unknown');
        }
    }

    return symbols;
}

// ===== Scope override collection (typeguard narrowing) =====

function collectScopeOverrides(document: vscode.TextDocument, symbols: HoverSymbol[]): ScopeOverride[] {
    const overrides: ScopeOverride[] = [];

    const declaredTypes = new Map<string, LangType>();
    for (const sym of symbols) {
        if (sym.kind === 'variable' && sym.type) {
            declaredTypes.set(sym.name, sym.type);
        }
    }

    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const indent = (stripped.match(/^(\s*)/)?.[1] ?? '').length;

        // Check is-not before is
        const isNotMatch = stripped.match(TYPEGUARD_IS_NOT_RE);
        if (isNotMatch) {
            const [, , varName, typeName] = isNotMatch;
            const { startLine, endLine } = findBlockBounds(document, i, indent);
            const narrowedType = computeIsNotNarrowedType(declaredTypes.get(varName), typeName);
            if (narrowedType) {
                overrides.push({ varName, narrowedType, startLine, endLine });
            }
            continue;
        }

        const isMatch = stripped.match(TYPEGUARD_IS_RE);
        if (isMatch) {
            const [, , varName, typeName] = isMatch;
            const { startLine, endLine } = findBlockBounds(document, i, indent);
            overrides.push({ varName, narrowedType: typeName, startLine, endLine });
        }
    }

    return overrides;
}

// ===== Class traits map =====

function collectClassTraits(document: vscode.TextDocument): Map<string, string[]> {
    const map = new Map<string, string[]>();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(CLASS_DEF_RE);
        if (m) {
            const [, , , name, bases] = m;
            const traits = bases?.split(',').map(cleanBaseName).filter(Boolean) ?? [];
            map.set(name, traits);
        }
    }
    return map;
}

// ===== Freeze line tracking =====

function collectFreezeLines(document: vscode.TextDocument): Map<string, number> {
    const map = new Map<string, number>();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(FREEZE_RE);
        if (m) { map.set(m[1], i); }
    }
    return map;
}

// ===== Hover symbol selection =====

function selectHoverSymbol(symbols: HoverSymbol[], name: string, line: number): HoverSymbol | undefined {
    const matches = symbols.filter(s => s.name === name);
    const visible = matches
        .filter(s => s.line <= line && (s.scopeEndLine === undefined || line < s.scopeEndLine))
        .sort((a, b) => b.line - a.line);
    return visible[0] ?? matches[0];
}

// ===== Hover rendering =====

function renderHover(symbol: HoverSymbol, opts?: {
    narrowedFrom?: string;
    isFrozen?: boolean;
    classTraits?: string[];
}): vscode.MarkdownString {
    const md = new vscode.MarkdownString(undefined, true);
    md.isTrusted = false;

    const mutability = opts?.isFrozen ? 'frozen' : (symbol.mutability ?? 'value');

    if (symbol.kind === 'variable') {
        md.appendCodeblock(`${mutability} ${symbol.name}: ${symbol.type ?? 'unknown'}`, 'tl');
        if (opts?.narrowedFrom) {
            md.appendMarkdown(`\n\n*narrowed from* \`${opts.narrowedFrom}\``);
        }
    } else if (symbol.kind === 'function') {
        md.appendCodeblock(symbol.signature ?? `fn ${symbol.name}() -> ${symbol.type ?? 'unknown'}`, 'tl');
    } else if (symbol.kind === 'class') {
        md.appendCodeblock(`class ${symbol.name}`, 'tl');
    } else if (symbol.kind === 'trait') {
        md.appendCodeblock(`trait ${symbol.name}`, 'tl');
    } else if (symbol.kind === 'module') {
        md.appendCodeblock(`${symbol.originalType ?? 'import[py] ?'} as ${symbol.name}`, 'tl');
    } else {
        md.appendCodeblock(`new_type ${symbol.name}: ${symbol.originalType ?? 'unknown'}`, 'tl');
    }

    // Class/trait definitions: show what they implement
    if (symbol.traits && symbol.traits.length > 0) {
        md.appendMarkdown(`\n\nImplements: ${symbol.traits.map(t => `\`${t}\``).join(', ')}`);
    }

    // Variable/param of a class type: show class's traits
    if (opts?.classTraits && opts.classTraits.length > 0) {
        md.appendMarkdown(`\n\nTraits: ${opts.classTraits.map(t => `\`${t}\``).join(', ')}`);
    }

    // new_type: original type already shown in code block; add it as context below too
    if (symbol.kind === 'new_type' && symbol.originalType) {
        md.appendMarkdown(`\n\nOriginal type: \`${symbol.originalType}\``);
    }

    if (symbol.doc) {
        md.appendMarkdown(`\n\n---\n\n${symbol.doc}`);
    }

    return md;
}

// ===== Hover provider =====

export function provideHover(
    document: vscode.TextDocument,
    position: vscode.Position
): vscode.Hover | undefined {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_]\w*/);
    if (!range) return undefined;

    const name = document.getText(range);
    const symbols = collectHoverSymbols(document);
    const symbol = selectHoverSymbol(symbols, name, position.line);
    if (!symbol) return undefined;

    // Freeze detection: show 'frozen' if freeze(name) was called before hover line
    const freezeLines = collectFreezeLines(document);
    const freezeLine = freezeLines.get(name);
    const isFrozen = freezeLine !== undefined && position.line >= freezeLine && symbol.mutability === 'mut';

    // Typeguard narrowing
    const overrides = collectScopeOverrides(document, symbols);
    const override = overrides.find(o =>
        o.varName === name &&
        position.line >= o.startLine &&
        position.line < o.endLine
    );

    // Class traits for the effective type
    const classTraitsMap = collectClassTraits(document);
    const effectiveType = override?.narrowedType ?? symbol.type;
    const rawClassTraits = effectiveType ? classTraitsMap.get(effectiveType) : undefined;
    const classTraits = rawClassTraits && rawClassTraits.length > 0 ? rawClassTraits : undefined;

    const displaySymbol = override ? { ...symbol, type: override.narrowedType } : symbol;

    return new vscode.Hover(renderHover(displaySymbol, {
        narrowedFrom: override ? symbol.type : undefined,
        isFrozen,
        classTraits,
    }), range);
}

// ===== Inlay hints provider =====

export function provideInlayHints(
    document: vscode.TextDocument,
    _range: vscode.Range
): vscode.InlayHint[] {
    const hints: vscode.InlayHint[] = [];

    // Phase 1: collect import aliases and function definitions
    const importAliasMap = collectImportAliases(document);
    const importAliases = new Set(importAliasMap.keys());
    const { funcTypes: importFuncTypes, classMethods: pyClassMethods } = collectAllPyModuleInfo(document);
    const funcDefs = collectFuncDefs(document);

    // Phase 2: build funcEnv
    const funcEnv = collectConstructorTypes(document);
    for (const def of funcDefs) {
        funcEnv.set(def.name,
            def.annotation ?? inferBodyReturnType(document, def.defLine, def.defIndent, funcEnv, importAliases, importFuncTypes, pyClassMethods)
        );
    }

    // Phase 3: inlay hints on function definition lines (only when no annotation)
    for (const def of funcDefs) {
        if (def.annotation !== undefined) continue;
        const returnType = funcEnv.get(def.name)!;
        const rawLine = document.lineAt(def.defLine).text;
        const rparenPos = rawLine.lastIndexOf(')');
        if (rparenPos < 0) continue;
        const pos = new vscode.Position(def.defLine, rparenPos + 1);
        const hint = new vscode.InlayHint(pos, ` -> ${returnType}`, vscode.InlayHintKind.Type);
        hints.push(hint);
    }

    // Phase 4: inlay hints on variable declarations
    const env = new Map<string, LangType>();

    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const rawLine = document.lineAt(lineIdx).text;
        const line = stripComment(rawLine);

        // Skip import declarations (no inlay hint needed)
        if (line.match(IMPORT_RE)) continue;

        // static mut declarations
        const staticMatch = line.match(STATIC_DECL_RE);
        if (staticMatch) {
            const [, indent, name, annotation, rhs] = staticMatch;
            const type = cleanTypeAnnotation(annotation)
                ?? (rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods) : 'unknown');
            env.set(name, type);
            if (!annotation) {
                const nameStart = rawLine.indexOf(name, indent.length + 'static mut '.length);
                if (nameStart >= 0) {
                    const pos = new vscode.Position(lineIdx, nameStart + name.length);
                    const hint = new vscode.InlayHint(pos, `: ${type}`, vscode.InlayHintKind.Type);
                    hint.paddingLeft = true;
                    hints.push(hint);
                }
            }
            continue;
        }

        // let/mut/const declarations
        const m = line.match(DECL_RE);
        if (!m) continue;

        const [full, indent, keyword, name] = m;
        const nameStart = rawLine.indexOf(name, indent.length + keyword.length);
        if (nameStart < 0) continue;

        const rhs = line.slice(full.length).trim();
        if (!rhs) continue;

        const type = inferExprType(rhs, env, funcEnv, importAliases, importFuncTypes, pyClassMethods);
        env.set(name, type);

        const pos = new vscode.Position(lineIdx, nameStart + name.length);
        const hint = new vscode.InlayHint(pos, `: ${type}`, vscode.InlayHintKind.Type);
        hint.paddingLeft = true;
        hints.push(hint);
    }

    return hints;
}

// ===== Semantic tokens (import alias highlighting) =====

export const SEMANTIC_TOKENS_LEGEND = new vscode.SemanticTokensLegend(
    ['class'],  // token types: index 0 = 'class'
    []          // modifiers: none
);

function escapeRegex(s: string): string {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

export function provideDocumentSemanticTokens(document: vscode.TextDocument): vscode.SemanticTokens {
    const builder = new vscode.SemanticTokensBuilder(SEMANTIC_TOKENS_LEGEND);
    const aliases = collectImportAliases(document);
    if (aliases.size === 0) return builder.build();

    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const lineText = document.lineAt(lineIdx).text;
        const lineMatches: { col: number; len: number }[] = [];

        for (const [alias] of aliases) {
            const re = new RegExp(`\\b${escapeRegex(alias)}\\b`, 'g');
            let m;
            while ((m = re.exec(lineText)) !== null) {
                lineMatches.push({ col: m.index, len: alias.length });
            }
        }

        lineMatches.sort((a, b) => a.col - b.col);
        for (const { col, len } of lineMatches) {
            builder.push(lineIdx, col, len, 0, 0);
        }
    }

    return builder.build();
}
