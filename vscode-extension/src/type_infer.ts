import * as vscode from 'vscode';

export type LangType = 'int' | 'float' | 'str' | 'bool' | 'None' | 'unknown';

// ---------- Tokenizer ----------

type TK =
    | 'INT' | 'FLOAT' | 'STR'
    | 'TRUE' | 'FALSE' | 'NONE'
    | 'IDENT'
    | 'PLUS' | 'MINUS' | 'STAR' | 'SLASH' | 'SLASHSLASH' | 'PERCENT' | 'STARSTAR'
    | 'EQEQ' | 'NOTEQ' | 'LT' | 'GT' | 'LTEQ' | 'GTEQ'
    | 'AND' | 'OR' | 'NOT'
    | 'AMP' | 'PIPE' | 'CARET' | 'TILDE' | 'LTLT' | 'GTGT'
    | 'LPAREN' | 'RPAREN' | 'COMMA'
    | 'OTHER' | 'EOF';

interface Token { kind: TK; value: string; }

function tokenize(src: string): Token[] {
    const tokens: Token[] = [];
    let i = 0;

    while (i < src.length) {
        if (' \t\r\n'.includes(src[i])) { i++; continue; }

        // String literals
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

        // Numbers
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

        // Identifiers and keywords
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

        // Multi-char operators (longest match first)
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
        };
        tokens.push({ kind: op1[src[i]] ?? 'OTHER', value: src[i] });
        i++;
    }

    tokens.push({ kind: 'EOF', value: '' });
    return tokens;
}

// ---------- Expression type inferrer (recursive descent) ----------

function mergeNumeric(a: LangType, b: LangType): LangType {
    if (a === 'float' || b === 'float') return 'float';
    if (a === 'int' && b === 'int') return 'int';
    return 'unknown';
}

class ExprInferrer {
    private pos = 0;
    constructor(private readonly tokens: Token[], private readonly env: Map<string, LangType>) {}

    private cur(): Token { return this.tokens[this.pos] ?? { kind: 'EOF', value: '' }; }
    private eat(): Token { return this.tokens[this.pos++] ?? { kind: 'EOF', value: '' }; }

    infer(): LangType { return this.parseOr(); }

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
        while (this.cur().kind === 'PIPE') { this.eat(); t = mergeNumeric(t, this.parseBitXor()); }
        return t;
    }

    private parseBitXor(): LangType {
        let t = this.parseBitAnd();
        while (this.cur().kind === 'CARET') { this.eat(); t = mergeNumeric(t, this.parseBitAnd()); }
        return t;
    }

    private parseBitAnd(): LangType {
        let t = this.parseShift();
        while (this.cur().kind === 'AMP') { this.eat(); t = mergeNumeric(t, this.parseShift()); }
        return t;
    }

    private parseShift(): LangType {
        let t = this.parseAdditive();
        while (this.cur().kind === 'LTLT' || this.cur().kind === 'GTGT') {
            this.eat(); t = mergeNumeric(t, this.parseAdditive());
        }
        return t;
    }

    private parseAdditive(): LangType {
        let t = this.parseMultiplicative();
        while (this.cur().kind === 'PLUS' || this.cur().kind === 'MINUS') {
            const op = this.eat().kind;
            const r = this.parseMultiplicative();
            t = (op === 'PLUS' && t === 'str' && r === 'str') ? 'str' : mergeNumeric(t, r);
        }
        return t;
    }

    private parseMultiplicative(): LangType {
        let t = this.parseUnary();
        const ops: TK[] = ['STAR', 'SLASH', 'SLASHSLASH', 'PERCENT'];
        while (ops.includes(this.cur().kind)) {
            const op = this.eat().kind;
            const r = this.parseUnary();
            t = op === 'SLASH' ? 'float' : mergeNumeric(t, r);
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
        if (this.cur().kind === 'STARSTAR') { this.eat(); return mergeNumeric(base, this.parseUnary()); }
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
                // function call
                if (this.cur().kind === 'LPAREN') {
                    this.eat();
                    while (this.cur().kind !== 'RPAREN' && this.cur().kind !== 'EOF') {
                        this.parseOr();
                        if (this.cur().kind === 'COMMA') this.eat(); else break;
                    }
                    if (this.cur().kind === 'RPAREN') this.eat();
                    if (name === 'print') return 'None';
                    return 'unknown';
                }
                return this.env.get(name) ?? 'unknown';
            }
            case 'LPAREN': {
                this.eat();
                const t = this.parseOr();
                if (this.cur().kind === 'RPAREN') this.eat();
                return t;
            }
            default:
                this.eat();
                return 'unknown';
        }
    }
}

export function inferExprType(src: string, env: Map<string, LangType>): LangType {
    return new ExprInferrer(tokenize(src), env).infer();
}

// ---------- Strip comment (respecting strings) ----------

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

// ---------- Inlay hints provider ----------

// Matches `let/mut/const name = ...` (not `==`)
const DECL_RE = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*=(?!=)/;

export function provideInlayHints(
    document: vscode.TextDocument,
    _range: vscode.Range
): vscode.InlayHint[] {
    const hints: vscode.InlayHint[] = [];
    const env = new Map<string, LangType>();

    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const rawLine = document.lineAt(lineIdx).text;
        const line = stripComment(rawLine);

        const m = line.match(DECL_RE);
        if (!m) continue;

        const [full, indent, keyword, name] = m;
        // Find the column of `name` in the raw line
        const nameStart = rawLine.indexOf(name, indent.length + keyword.length);
        if (nameStart < 0) continue;

        const rhs = line.slice(full.length).trim();
        if (!rhs) continue;

        const type = inferExprType(rhs, env);
        env.set(name, type);

        const pos = new vscode.Position(lineIdx, nameStart + name.length);
        const hint = new vscode.InlayHint(pos, `: ${type}`, vscode.InlayHintKind.Type);
        hint.paddingLeft = true;
        hints.push(hint);
    }

    return hints;
}
