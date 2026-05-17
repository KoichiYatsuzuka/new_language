"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.provideDocumentSemanticTokens = exports.SEMANTIC_TOKENS_LEGEND = exports.provideDefinition = exports.provideSignatureHelp = exports.provideDocumentSymbols = exports.provideCompletionItems = exports.provideInlayHints = exports.provideHover = exports.inferExprType = void 0;
const vscode = require("vscode");
const fs = require("fs");
const path = require("path");
function tokenize(src) {
    const tokens = [];
    let i = 0;
    while (i < src.length) {
        if (' \t\r\n'.includes(src[i])) {
            i++;
            continue;
        }
        if (src[i] === '"' || src[i] === "'") {
            const q = src[i];
            const triple = src.startsWith(q + q + q, i);
            let j = i + (triple ? 3 : 1);
            while (j < src.length) {
                if (src[j] === '\\') {
                    j += 2;
                    continue;
                }
                if (triple ? src.startsWith(q + q + q, j) : src[j] === q) {
                    j += triple ? 3 : 1;
                    break;
                }
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
                while (j < src.length && /[\da-fA-F_]/.test(src[j]))
                    j++;
                tokens.push({ kind: 'INT', value: src.slice(i, j) });
            }
            else {
                while (j < src.length && /[\d_]/.test(src[j]))
                    j++;
                let isFloat = false;
                if (j < src.length && src[j] === '.' && j + 1 < src.length && /\d/.test(src[j + 1])) {
                    isFloat = true;
                    j++;
                    while (j < src.length && /[\d_]/.test(src[j]))
                        j++;
                }
                if (j < src.length && 'eE'.includes(src[j])) {
                    isFloat = true;
                    j++;
                    if (j < src.length && '+-'.includes(src[j]))
                        j++;
                    while (j < src.length && /\d/.test(src[j]))
                        j++;
                }
                tokens.push({ kind: isFloat ? 'FLOAT' : 'INT', value: src.slice(i, j) });
            }
            i = j;
            continue;
        }
        if (/[A-Za-z_]/.test(src[i])) {
            let j = i;
            while (j < src.length && /\w/.test(src[j]))
                j++;
            const word = src.slice(i, j);
            const keywordMap = {
                True: 'TRUE', False: 'FALSE', None: 'NONE',
                and: 'AND', or: 'OR', not: 'NOT',
            };
            tokens.push({ kind: keywordMap[word] ?? 'IDENT', value: word });
            i = j;
            continue;
        }
        const s3 = src.slice(i, i + 3);
        if (['//=', '**=', '<<=', '>>='].includes(s3)) {
            tokens.push({ kind: 'OTHER', value: s3 });
            i += 3;
            continue;
        }
        const s2 = src.slice(i, i + 2);
        const op2 = {
            '**': 'STARSTAR', '//': 'SLASHSLASH', '==': 'EQEQ', '!=': 'NOTEQ',
            '<=': 'LTEQ', '>=': 'GTEQ', '<<': 'LTLT', '>>': 'GTGT',
        };
        if (op2[s2]) {
            tokens.push({ kind: op2[s2], value: s2 });
            i += 2;
            continue;
        }
        if (['+=', '-=', '*=', '/=', '%=', '&=', '|=', '^=', '@=', '->', ':='].includes(s2)) {
            tokens.push({ kind: 'OTHER', value: s2 });
            i += 2;
            continue;
        }
        const op1 = {
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
const BUILTIN_RETURN_TYPES = {
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
function mergeNumeric(a, b) {
    if (a === 'float' || b === 'float')
        return 'float';
    if (a === 'int' && b === 'int')
        return 'int';
    return 'unknown';
}
// Maps operator token kind → [dunder method, reverse dunder method]
const OP_DUNDER = {
    PLUS: ['__add__', '__radd__'],
    MINUS: ['__sub__', '__rsub__'],
    STAR: ['__mul__', '__rmul__'],
    SLASH: ['__truediv__', '__rtruediv__'],
    SLASHSLASH: ['__floordiv__', '__rfloordiv__'],
    PERCENT: ['__mod__', '__rmod__'],
    STARSTAR: ['__pow__', '__rpow__'],
    AMP: ['__and__', '__rand__'],
    PIPE: ['__or__', '__ror__'],
    CARET: ['__xor__', '__rxor__'],
    LTLT: ['__lshift__', '__rlshift__'],
    GTGT: ['__rshift__', '__rrshift__'],
};
class ExprInferrer {
    constructor(tokens, env, funcEnv, pyClassMethods = new Map()) {
        this.tokens = tokens;
        this.env = env;
        this.funcEnv = funcEnv;
        this.pyClassMethods = pyClassMethods;
        this.pos = 0;
    }
    cur() { return this.tokens[this.pos] ?? { kind: 'EOF', value: '' }; }
    eat() { return this.tokens[this.pos++] ?? { kind: 'EOF', value: '' }; }
    infer() { return this.parseOr(); }
    applyBinaryOp(left, right, op) {
        const dunder = OP_DUNDER[op];
        if (dunder) {
            const lm = this.pyClassMethods.get(left);
            if (lm?.has(dunder[0]))
                return lm.get(dunder[0]);
            const rm = this.pyClassMethods.get(right);
            if (rm?.has(dunder[1]))
                return rm.get(dunder[1]);
        }
        if (op === 'SLASH')
            return 'float';
        return mergeNumeric(left, right);
    }
    parseOr() {
        let t = this.parseAnd();
        while (this.cur().kind === 'OR') {
            this.eat();
            this.parseAnd();
            t = 'bool';
        }
        return t;
    }
    parseAnd() {
        let t = this.parseNot();
        while (this.cur().kind === 'AND') {
            this.eat();
            this.parseNot();
            t = 'bool';
        }
        return t;
    }
    parseNot() {
        if (this.cur().kind === 'NOT') {
            this.eat();
            this.parseNot();
            return 'bool';
        }
        return this.parseComparison();
    }
    parseComparison() {
        const left = this.parseBitOr();
        const cmpOps = ['EQEQ', 'NOTEQ', 'LT', 'GT', 'LTEQ', 'GTEQ'];
        if (cmpOps.includes(this.cur().kind)) {
            this.eat();
            this.parseBitOr();
            return 'bool';
        }
        return left;
    }
    parseBitOr() {
        let t = this.parseBitXor();
        while (this.cur().kind === 'PIPE') {
            this.eat();
            t = this.applyBinaryOp(t, this.parseBitXor(), 'PIPE');
        }
        return t;
    }
    parseBitXor() {
        let t = this.parseBitAnd();
        while (this.cur().kind === 'CARET') {
            this.eat();
            t = this.applyBinaryOp(t, this.parseBitAnd(), 'CARET');
        }
        return t;
    }
    parseBitAnd() {
        let t = this.parseShift();
        while (this.cur().kind === 'AMP') {
            this.eat();
            t = this.applyBinaryOp(t, this.parseShift(), 'AMP');
        }
        return t;
    }
    parseShift() {
        let t = this.parseAdditive();
        while (this.cur().kind === 'LTLT' || this.cur().kind === 'GTGT') {
            const op = this.eat().kind;
            t = this.applyBinaryOp(t, this.parseAdditive(), op);
        }
        return t;
    }
    parseAdditive() {
        let t = this.parseMultiplicative();
        while (this.cur().kind === 'PLUS' || this.cur().kind === 'MINUS') {
            const op = this.eat().kind;
            const r = this.parseMultiplicative();
            t = (op === 'PLUS' && t === 'str' && r === 'str') ? 'str' : this.applyBinaryOp(t, r, op);
        }
        return t;
    }
    parseMultiplicative() {
        let t = this.parseUnary();
        const ops = ['STAR', 'SLASH', 'SLASHSLASH', 'PERCENT'];
        while (ops.includes(this.cur().kind)) {
            const op = this.eat().kind;
            t = this.applyBinaryOp(t, this.parseUnary(), op);
        }
        return t;
    }
    parseUnary() {
        if (this.cur().kind === 'MINUS' || this.cur().kind === 'PLUS') {
            this.eat();
            return this.parseUnary();
        }
        if (this.cur().kind === 'TILDE') {
            this.eat();
            this.parseUnary();
            return 'int';
        }
        return this.parsePower();
    }
    parsePower() {
        const base = this.parsePrimary();
        if (this.cur().kind === 'STARSTAR') {
            this.eat();
            return this.applyBinaryOp(base, this.parseUnary(), 'STARSTAR');
        }
        return base;
    }
    parsePrimary() {
        const tok = this.cur();
        switch (tok.kind) {
            case 'INT':
                this.eat();
                return 'int';
            case 'FLOAT':
                this.eat();
                return 'float';
            case 'STR':
                this.eat();
                return 'str';
            case 'TRUE':
            case 'FALSE':
                this.eat();
                return 'bool';
            case 'NONE':
                this.eat();
                return 'None';
            case 'IDENT': {
                const name = tok.value;
                this.eat();
                let isChained = false;
                while (this.cur().kind === 'OTHER' && this.cur().value === '.') {
                    this.eat();
                    if (this.cur().kind === 'IDENT')
                        this.eat();
                    isChained = true;
                }
                if (this.cur().kind === 'LPAREN') {
                    this.eat();
                    while (this.cur().kind !== 'RPAREN' && this.cur().kind !== 'EOF') {
                        this.parseOr();
                        if (this.cur().kind === 'COMMA')
                            this.eat();
                        else
                            break;
                    }
                    if (this.cur().kind === 'RPAREN')
                        this.eat();
                    if (isChained)
                        return 'unknown';
                    if (name in BUILTIN_RETURN_TYPES)
                        return BUILTIN_RETURN_TYPES[name];
                    return this.funcEnv.get(name) ?? 'unknown';
                }
                if (isChained)
                    return 'unknown';
                return this.env.get(name) ?? 'unknown';
            }
            case 'LPAREN': {
                this.eat();
                if (this.cur().kind === 'RPAREN') {
                    this.eat();
                    return 'tuple';
                }
                const first = this.parseOr();
                if (this.cur().kind === 'COMMA') {
                    const types = [first];
                    while (this.cur().kind === 'COMMA') {
                        this.eat();
                        if (this.cur().kind === 'RPAREN')
                            break;
                        types.push(this.parseOr());
                    }
                    if (this.cur().kind === 'RPAREN')
                        this.eat();
                    return `tuple[${types.join(', ')}]`;
                }
                if (this.cur().kind === 'RPAREN')
                    this.eat();
                return first;
            }
            case 'LBRACKET': {
                this.eat();
                if (this.cur().kind === 'RBRACKET') {
                    this.eat();
                    return 'list';
                }
                const elemTypes = [];
                while (this.cur().kind !== 'RBRACKET' && this.cur().kind !== 'EOF') {
                    elemTypes.push(this.parseOr());
                    if (this.cur().kind === 'COMMA')
                        this.eat();
                    else
                        break;
                }
                if (this.cur().kind === 'RBRACKET')
                    this.eat();
                const uniqElems = [...new Set(elemTypes)];
                return uniqElems.length === 1 ? `list[${uniqElems[0]}]` : 'list';
            }
            case 'LBRACE': {
                this.eat();
                if (this.cur().kind === 'RBRACE') {
                    this.eat();
                    return 'dict';
                }
                const keyTypes = [];
                const valTypes = [];
                while (this.cur().kind !== 'RBRACE' && this.cur().kind !== 'EOF') {
                    keyTypes.push(this.parseOr());
                    if (this.cur().kind === 'COLON') {
                        this.eat();
                        valTypes.push(this.parseOr());
                    }
                    if (this.cur().kind === 'COMMA')
                        this.eat();
                    else
                        break;
                }
                if (this.cur().kind === 'RBRACE')
                    this.eat();
                const uniqK = [...new Set(keyTypes)];
                const uniqV = [...new Set(valTypes)];
                if (uniqK.length === 1 && uniqV.length === 1)
                    return `dict[${uniqK[0]}, ${uniqV[0]}]`;
                return 'dict';
            }
            default:
                this.eat();
                return 'unknown';
        }
    }
}
function inferExprType(src, env, funcEnv = new Map(), importAliases = new Set(), importFuncTypes = new Map(), pyClassMethods = new Map()) {
    const trimmed = src.trim();
    // Intercept alias.anything before ExprInferrer to avoid alias-name leaking as type
    const dotMatch = trimmed.match(/^([A-Za-z_]\w*)\./);
    if (dotMatch && importAliases.has(dotMatch[1])) {
        const alias = dotMatch[1];
        const callMatch = trimmed.match(/^[A-Za-z_]\w*\.([A-Za-z_]\w*)\s*\(/);
        if (callMatch) {
            const memberName = callMatch[1];
            if (/^[A-Z]/.test(memberName))
                return memberName;
            const pyTypes = importFuncTypes.get(alias);
            if (pyTypes)
                return pyTypes.get(memberName) ?? 'unknown';
        }
        return 'unknown';
    }
    return new ExprInferrer(tokenize(src), env, funcEnv, pyClassMethods).infer();
}
exports.inferExprType = inferExprType;
// ===== Strip comment (respecting strings) =====
function stripComment(line) {
    let inStr = false;
    let strChar = '';
    let triple = false;
    for (let i = 0; i < line.length; i++) {
        const c = line[i];
        if (!inStr) {
            if ((c === '"' || c === "'") && line.startsWith(c + c + c, i)) {
                inStr = true;
                strChar = c;
                triple = true;
                i += 2;
            }
            else if (c === '"' || c === "'") {
                inStr = true;
                strChar = c;
                triple = false;
            }
            else if (c === '#') {
                return line.slice(0, i);
            }
        }
        else {
            if (c === '\\') {
                i++;
                continue;
            }
            if (triple && line.startsWith(strChar + strChar + strChar, i)) {
                inStr = false;
                i += 2;
            }
            else if (!triple && c === strChar) {
                inStr = false;
            }
        }
    }
    return line;
}
// ===== Regex constants =====
// Function definition — RetType can be complex: function[T]->R, function{name:T}->R, etc.
const FUNC_DEF_RE = /^(\s*)(fn|gen)\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:\s*$/;
// Variable declarations
const DECL_RE = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*=(?!=)/;
const STATIC_DECL_RE = /^(\s*)static\s+mut\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*=(?!=)\s*(.*)/;
const RETURN_RE = /^return(?:\s+(.+))?$/;
const CLASS_NAME_RE = /^\s*(?:class|trait)\s+([A-Za-z_]\w*)/;
const NEW_TYPE_NAME_RE = /^\s*new_type\s+([A-Za-z_]\w*)\s*:/;
// Hover-specific
const HOVER_DECL_RE = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*(?:=(?!=)\s*(.*))?$/;
const CLASS_DEF_RE = /^(\s*)(class|trait)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*(?:\(([^)]*)\))?\s*:/;
const NEW_TYPE_RE = /^(\s*)new_type\s+([A-Za-z_]\w*)\s*:\s*([A-Za-z_][\w\[\], ]*)/;
// freeze(varName)
const FREEZE_RE = /^\s*freeze\s*\(\s*([A-Za-z_]\w*)\s*\)/;
// import[py] / import[py-int]  — captures: 1=keyword, 2=module, 3=alias
const IMPORT_RE = /^\s*(import\[(?:py(?:-int)?)\])\s+([A-Za-z_]\w*)\s+as\s+([A-Za-z_]\w*)/;
// Typeguard — check is-not before is to avoid accidental match
const TYPEGUARD_IS_NOT_RE = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+not\s+([A-Za-z_]\w*)\s*:/;
const TYPEGUARD_IS_RE = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+([A-Za-z_]\w*)\s*:/;
// ===== Utilities =====
// Split comma-separated items respecting nested brackets
function splitComma(s) {
    const result = [];
    let depth = 0;
    let start = 0;
    for (let i = 0; i < s.length; i++) {
        if ('[({'.includes(s[i]))
            depth++;
        else if ('])}'.includes(s[i]))
            depth--;
        else if (s[i] === ',' && depth === 0) {
            result.push(s.slice(start, i));
            start = i + 1;
        }
    }
    result.push(s.slice(start));
    return result;
}
// For Option[T] is-not-None → T; for Union[A,B] is-not-A → B
function computeIsNotNarrowedType(declaredType, removedType) {
    if (!declaredType)
        return undefined;
    const optMatch = declaredType.match(/^Option\[(.+)\]$/);
    if (optMatch && removedType === 'None')
        return optMatch[1].trim();
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
function findBlockBounds(document, ifLine, ifIndent) {
    const startLine = ifLine + 1;
    let endLine = document.lineCount;
    for (let j = startLine; j < document.lineCount; j++) {
        const raw = document.lineAt(j).text;
        if (!raw.trim())
            continue;
        const indent = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (indent <= ifIndent) {
            endLine = j;
            break;
        }
    }
    return { startLine, endLine };
}
// Find the first line past a function body (indent drops to defIndent or below)
function findBodyEndLine(document, defLine, defIndent) {
    for (let j = defLine + 1; j < document.lineCount; j++) {
        const raw = document.lineAt(j).text;
        if (!raw.trim())
            continue;
        const indent = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (indent <= defIndent)
            return j;
    }
    return document.lineCount;
}
// Parse function parameter list into hover symbols scoped to the function body
function parseParams(paramsStr, defLine, bodyEndLine) {
    const symbols = [];
    for (const part of splitComma(paramsStr)) {
        const trimmed = part.trim();
        if (!trimmed)
            continue;
        // [let|mut] name [: type]
        const m = trimmed.match(/^(?:(let|mut)\s+)?([A-Za-z_]\w*)\s*(?::\s*(.+))?$/);
        if (!m)
            continue;
        const [, mut, name, type] = m;
        if (name === 'self')
            continue;
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
function cleanTypeAnnotation(value) {
    const cleaned = value?.trim();
    return cleaned ? cleaned : undefined;
}
function cleanBaseName(value) {
    return value.trim().replace(/\[[^\]]*\]/g, '');
}
function parseTypeAnnotation(s) {
    if (!s)
        return undefined;
    const t = s.trim();
    return t || undefined;
}
// ===== Docstring extraction =====
function getDocstringAfter(document, line, indent) {
    for (let i = line + 1; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const trimmed = raw.trim();
        if (!trimmed)
            continue;
        const lineIndent = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= indent)
            return undefined;
        if (!trimmed.startsWith('"""') && !trimmed.startsWith("'''"))
            return undefined;
        const quote = trimmed.startsWith('"""') ? '"""' : "'''";
        let text = trimmed.slice(3);
        if (text.endsWith(quote) && text.length >= 3) {
            text = text.slice(0, -3);
            return text.trim() || undefined;
        }
        const lines = [];
        if (text)
            lines.push(text);
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
function collectConstructorTypes(document) {
    const constructors = new Map();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const classMatch = stripped.match(CLASS_NAME_RE);
        if (classMatch) {
            constructors.set(classMatch[1], classMatch[1]);
            continue;
        }
        const newTypeMatch = stripped.match(NEW_TYPE_NAME_RE);
        if (newTypeMatch) {
            constructors.set(newTypeMatch[1], newTypeMatch[1]);
        }
    }
    return constructors;
}
function collectFuncDefs(document) {
    const defs = [];
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(FUNC_DEF_RE);
        if (!m)
            continue;
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
function inferBodyReturnType(document, defLine, defIndent, funcEnv, importAliases = new Set(), importFuncTypes = new Map(), pyClassMethods = new Map()) {
    const localEnv = new Map();
    const returnTypes = [];
    for (let i = defLine + 1; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const trimmed = stripped.trim();
        if (!trimmed)
            continue;
        const lineIndent = (stripped.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= defIndent)
            break;
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
    if (returnTypes.length === 0)
        return 'None';
    const unique = [...new Set(returnTypes)];
    return unique.length === 1 ? unique[0] : 'unknown';
}
// ===== Import alias collection =====
function collectImportAliases(document) {
    const aliases = new Map(); // alias → python module name
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(IMPORT_RE);
        if (m)
            aliases.set(m[3], m[2]);
    }
    return aliases;
}
function collectPyModuleInfo(moduleName, docDir) {
    const funcs = new Map();
    const classes = new Map();
    const pyPath = path.join(docDir, moduleName + '.py');
    if (!fs.existsSync(pyPath))
        return { funcs, classes };
    let content;
    try {
        content = fs.readFileSync(pyPath, 'utf8');
    }
    catch {
        return { funcs, classes };
    }
    let currentClass = null;
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
        if (!trimmed || trimmed.startsWith('#'))
            continue;
        const lineIndent = (line.match(/^(\s*)/)?.[1] ?? '').length;
        if (currentClass !== null && lineIndent <= classIndent) {
            currentClass = null;
            classIndent = -1;
        }
        const funcM = line.match(funcRe);
        if (funcM?.[3]) {
            const retType = funcM[3].trim().replace(/^['"]|['"]$/g, '');
            if (currentClass !== null)
                classes.get(currentClass).set(funcM[2], retType);
            else
                funcs.set(funcM[2], retType);
        }
    }
    return { funcs, classes };
}
function collectAllPyModuleInfo(document) {
    const funcTypes = new Map();
    const classMethods = new Map();
    const aliasMap = collectImportAliases(document);
    const docDir = path.dirname(document.uri.fsPath);
    for (const [alias, moduleName] of aliasMap) {
        const info = collectPyModuleInfo(moduleName, docDir);
        funcTypes.set(alias, info.funcs);
        for (const [cls, methods] of info.classes)
            classMethods.set(cls, methods);
    }
    return { funcTypes, classMethods };
}
// ===== Hover symbol collection =====
function collectHoverSymbols(document) {
    const symbols = [];
    const env = new Map();
    // Collect import aliases first so type inference can use them throughout
    const importAliasMap = collectImportAliases(document);
    const importAliases = new Set(importAliasMap.keys());
    const { funcTypes: importFuncTypes, classMethods: pyClassMethods } = collectAllPyModuleInfo(document);
    const funcDefs = collectFuncDefs(document);
    const funcEnv = collectConstructorTypes(document);
    for (const def of funcDefs) {
        funcEnv.set(def.name, def.annotation ?? inferBodyReturnType(document, def.defLine, def.defIndent, funcEnv, importAliases, importFuncTypes, pyClassMethods));
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
function collectScopeOverrides(document, symbols) {
    const overrides = [];
    const declaredTypes = new Map();
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
function collectClassTraits(document) {
    const map = new Map();
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
function collectFreezeLines(document) {
    const map = new Map();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(FREEZE_RE);
        if (m) {
            map.set(m[1], i);
        }
    }
    return map;
}
// ===== Hover symbol selection =====
function selectHoverSymbol(symbols, name, line) {
    const matches = symbols.filter(s => s.name === name);
    const visible = matches
        .filter(s => s.line <= line && (s.scopeEndLine === undefined || line < s.scopeEndLine))
        .sort((a, b) => b.line - a.line);
    return visible[0] ?? matches[0];
}
// ===== Hover rendering =====
function renderHover(symbol, opts) {
    const md = new vscode.MarkdownString(undefined, true);
    md.isTrusted = false;
    const mutability = opts?.isFrozen ? 'frozen' : (symbol.mutability ?? 'value');
    if (symbol.kind === 'variable') {
        md.appendCodeblock(`${mutability} ${symbol.name}: ${symbol.type ?? 'unknown'}`, 'tl');
        if (opts?.narrowedFrom) {
            md.appendMarkdown(`\n\n*narrowed from* \`${opts.narrowedFrom}\``);
        }
    }
    else if (symbol.kind === 'function') {
        md.appendCodeblock(symbol.signature ?? `fn ${symbol.name}() -> ${symbol.type ?? 'unknown'}`, 'tl');
    }
    else if (symbol.kind === 'class') {
        md.appendCodeblock(`class ${symbol.name}`, 'tl');
    }
    else if (symbol.kind === 'trait') {
        md.appendCodeblock(`trait ${symbol.name}`, 'tl');
    }
    else if (symbol.kind === 'module') {
        md.appendCodeblock(`${symbol.originalType ?? 'import[py] ?'} as ${symbol.name}`, 'tl');
    }
    else {
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
function provideHover(document, position) {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_]\w*/);
    if (!range)
        return undefined;
    const name = document.getText(range);
    const symbols = collectHoverSymbols(document);
    const symbol = selectHoverSymbol(symbols, name, position.line);
    if (!symbol)
        return undefined;
    // Freeze detection: show 'frozen' if freeze(name) was called before hover line
    const freezeLines = collectFreezeLines(document);
    const freezeLine = freezeLines.get(name);
    const isFrozen = freezeLine !== undefined && position.line >= freezeLine && symbol.mutability === 'mut';
    // Typeguard narrowing
    const overrides = collectScopeOverrides(document, symbols);
    const override = overrides.find(o => o.varName === name &&
        position.line >= o.startLine &&
        position.line < o.endLine);
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
exports.provideHover = provideHover;
// ===== Inlay hints provider =====
function provideInlayHints(document, _range) {
    const hints = [];
    // Phase 1: collect import aliases and function definitions
    const importAliasMap = collectImportAliases(document);
    const importAliases = new Set(importAliasMap.keys());
    const { funcTypes: importFuncTypes, classMethods: pyClassMethods } = collectAllPyModuleInfo(document);
    const funcDefs = collectFuncDefs(document);
    // Phase 2: build funcEnv
    const funcEnv = collectConstructorTypes(document);
    for (const def of funcDefs) {
        funcEnv.set(def.name, def.annotation ?? inferBodyReturnType(document, def.defLine, def.defIndent, funcEnv, importAliases, importFuncTypes, pyClassMethods));
    }
    // Phase 3: inlay hints on function definition lines (only when no annotation)
    for (const def of funcDefs) {
        if (def.annotation !== undefined)
            continue;
        const returnType = funcEnv.get(def.name);
        const rawLine = document.lineAt(def.defLine).text;
        const rparenPos = rawLine.lastIndexOf(')');
        if (rparenPos < 0)
            continue;
        const pos = new vscode.Position(def.defLine, rparenPos + 1);
        const hint = new vscode.InlayHint(pos, ` -> ${returnType}`, vscode.InlayHintKind.Type);
        hints.push(hint);
    }
    // Phase 4: inlay hints on variable declarations
    const env = new Map();
    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const rawLine = document.lineAt(lineIdx).text;
        const line = stripComment(rawLine);
        // Skip import declarations (no inlay hint needed)
        if (line.match(IMPORT_RE))
            continue;
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
        if (!m)
            continue;
        const [full, indent, keyword, name] = m;
        const nameStart = rawLine.indexOf(name, indent.length + keyword.length);
        if (nameStart < 0)
            continue;
        const rhs = line.slice(full.length).trim();
        if (!rhs)
            continue;
        const type = inferExprType(rhs, env, funcEnv, importAliases, importFuncTypes, pyClassMethods);
        env.set(name, type);
        const pos = new vscode.Position(lineIdx, nameStart + name.length);
        const hint = new vscode.InlayHint(pos, `: ${type}`, vscode.InlayHintKind.Type);
        hint.paddingLeft = true;
        hints.push(hint);
    }
    return hints;
}
exports.provideInlayHints = provideInlayHints;
// ===== Language keywords =====
const LANG_KEYWORDS = [
    'let', 'mut', 'const', 'fn', 'gen', 'class', 'trait', 'new_type',
    'return', 'if', 'elif', 'else', 'for', 'while', 'match', 'case',
    'and', 'or', 'not', 'in', 'is', 'True', 'False', 'None',
    'self', 'Self', 'static', 'freeze', 'block',
    'block_return', 'loop_yield', 'break', 'continue', 'yield', 'pass',
    'import',
];
// ===== Member completion helpers =====
function findEnclosingClass(document, fromLine) {
    const fromIndent = (document.lineAt(fromLine).text.match(/^(\s*)/)?.[1] ?? '').length;
    for (let i = fromLine - 1; i >= 0; i--) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(CLASS_DEF_RE);
        if (!m)
            continue;
        const classIndent = (m[1] ?? '').length;
        if (classIndent < fromIndent)
            return m[3];
    }
    return undefined;
}
function collectClassMemberItems(document, className, _visited = new Set()) {
    if (_visited.has(className))
        return [];
    _visited.add(className);
    // Resolve new_type aliases to their base class
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(NEW_TYPE_RE);
        if (m && m[2] === className) {
            return collectClassMemberItems(document, m[3].trim(), _visited);
        }
    }
    // Find the class definition
    let classLine = -1;
    let classIndent = 0;
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(CLASS_DEF_RE);
        if (m && m[3] === className) {
            classLine = i;
            classIndent = (m[1] ?? '').length;
            break;
        }
    }
    if (classLine < 0)
        return [];
    // Determine the indentation of direct class body members
    let memberIndent = -1;
    for (let i = classLine + 1; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        if (!raw.trim())
            continue;
        const ind = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (ind <= classIndent)
            break;
        memberIndent = ind;
        break;
    }
    if (memberIndent < 0)
        return [];
    const items = [];
    const seen = new Set();
    for (let i = classLine + 1; i < document.lineCount; i++) {
        const rawLine = document.lineAt(i).text;
        const stripped = stripComment(rawLine);
        if (!stripped.trim())
            continue;
        const lineIndent = (rawLine.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= classIndent)
            break;
        // Direct class body members (fields and methods)
        if (lineIndent === memberIndent) {
            // Field declarations: `let`/`mut`/`const` name [: type] [= value]
            const fieldMatch = stripped.match(HOVER_DECL_RE);
            if (fieldMatch) {
                const [, , , name, type] = fieldMatch;
                if (!seen.has(name)) {
                    seen.add(name);
                    const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Field);
                    if (type)
                        item.detail = `: ${type.trim()}`;
                    items.push(item);
                }
            }
            // Method definitions
            const funcMatch = stripped.match(FUNC_DEF_RE);
            if (funcMatch) {
                const [, , kw, name, params, retType] = funcMatch;
                if (!seen.has(name)) {
                    seen.add(name);
                    const cleanParams = params
                        .replace(/^\s*(?:let\s+|mut\s+)?self\s*,\s*/, '')
                        .replace(/^\s*(?:let\s+|mut\s+)?self\s*$/, '');
                    const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Method);
                    item.detail = `${kw} ${name}(${cleanParams})${retType ? ' -> ' + retType.trim() : ''}`;
                    items.push(item);
                }
            }
        }
        // self.attr = value assignments inside methods (e.g. __init__)
        const selfAttrMatch = stripped.match(/\bself\.([A-Za-z_]\w*)\s*(?:[+\-*\/%&|^]?=(?!=))/);
        if (selfAttrMatch && !seen.has(selfAttrMatch[1])) {
            seen.add(selfAttrMatch[1]);
            const item = new vscode.CompletionItem(selfAttrMatch[1], vscode.CompletionItemKind.Field);
            items.push(item);
        }
    }
    return items;
}
function resolveMemberItems(document, position, objName) {
    // 'self' → members of the enclosing class
    if (objName === 'self') {
        const cls = findEnclosingClass(document, position.line);
        return cls ? collectClassMemberItems(document, cls) : [];
    }
    const symbols = collectHoverSymbols(document);
    const sym = selectHoverSymbol(symbols, objName, position.line);
    if (!sym)
        return [];
    // Import module → show py module functions
    if (sym.kind === 'module') {
        const { funcTypes } = collectAllPyModuleInfo(document);
        const funcs = funcTypes.get(objName);
        if (!funcs)
            return [];
        return [...funcs.entries()].map(([name, retType]) => {
            const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
            item.detail = `→ ${retType}`;
            return item;
        });
    }
    // Variable/parameter with a class type → show class members
    if (sym.type) {
        return collectClassMemberItems(document, sym.type);
    }
    return [];
}
// ===== Completion items provider =====
function provideCompletionItems(document, position) {
    const prefix = document.lineAt(position.line).text.substring(0, position.character);
    // Member access context: ends with `identifier.` or `identifier.partial`
    const dotMatch = prefix.match(/([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/);
    if (dotMatch) {
        return resolveMemberItems(document, position, dotMatch[1]);
    }
    // Normal (non-dot) completion
    const items = [];
    const seen = new Set();
    // Document symbols first so user-defined names take priority over built-ins
    const symbols = collectHoverSymbols(document);
    for (const sym of symbols) {
        if (sym.kind === 'variable' && sym.line > position.line)
            continue;
        if (seen.has(sym.name))
            continue;
        seen.add(sym.name);
        let kind;
        switch (sym.kind) {
            case 'function':
                kind = vscode.CompletionItemKind.Function;
                break;
            case 'class':
                kind = vscode.CompletionItemKind.Class;
                break;
            case 'trait':
                kind = vscode.CompletionItemKind.Interface;
                break;
            case 'new_type':
                kind = vscode.CompletionItemKind.TypeParameter;
                break;
            case 'module':
                kind = vscode.CompletionItemKind.Module;
                break;
            default: kind = vscode.CompletionItemKind.Variable;
        }
        const item = new vscode.CompletionItem(sym.name, kind);
        if (sym.signature)
            item.detail = sym.signature;
        else if (sym.type)
            item.detail = `: ${sym.type}`;
        if (sym.doc)
            item.documentation = new vscode.MarkdownString(sym.doc);
        items.push(item);
    }
    // Built-in functions
    for (const [name, retType] of Object.entries(BUILTIN_RETURN_TYPES)) {
        if (seen.has(name))
            continue;
        seen.add(name);
        const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
        item.detail = `→ ${retType}`;
        items.push(item);
    }
    // Language keywords
    for (const kw of LANG_KEYWORDS) {
        if (seen.has(kw))
            continue;
        seen.add(kw);
        items.push(new vscode.CompletionItem(kw, vscode.CompletionItemKind.Keyword));
    }
    return items;
}
exports.provideCompletionItems = provideCompletionItems;
// ===== Document symbols provider =====
function provideDocumentSymbols(document) {
    const result = [];
    const stack = [];
    for (let i = 0; i < document.lineCount; i++) {
        const rawLine = document.lineAt(i).text;
        const stripped = stripComment(rawLine);
        if (!stripped.trim())
            continue;
        const indent = (rawLine.match(/^(\s*)/)?.[1] ?? '').length;
        // Pop scopes that have ended at this indentation level
        while (stack.length > 0 && indent <= stack[stack.length - 1].indent) {
            stack.pop();
        }
        let name = '';
        let detail = '';
        let kind = vscode.SymbolKind.Variable;
        let isContainer = false;
        const funcMatch = stripped.match(FUNC_DEF_RE);
        if (funcMatch) {
            const [, , , funcName, , retType] = funcMatch;
            name = funcName;
            kind = vscode.SymbolKind.Function;
            detail = retType?.trim() ?? '';
            isContainer = true;
        }
        else {
            const classMatch = stripped.match(CLASS_DEF_RE);
            if (classMatch) {
                const [, , kw, className, bases] = classMatch;
                name = className;
                kind = kw === 'trait' ? vscode.SymbolKind.Interface : vscode.SymbolKind.Class;
                detail = bases ? `(${bases})` : '';
                isContainer = true;
            }
            else {
                const newTypeMatch = stripped.match(NEW_TYPE_RE);
                if (newTypeMatch) {
                    const [, , ntName, ntType] = newTypeMatch;
                    name = ntName;
                    kind = vscode.SymbolKind.TypeParameter;
                    detail = ntType.trim();
                }
                else {
                    const importMatch = stripped.match(IMPORT_RE);
                    if (importMatch) {
                        const [, importKind, moduleName, alias] = importMatch;
                        name = alias;
                        kind = vscode.SymbolKind.Module;
                        detail = `${importKind} ${moduleName}`;
                    }
                    else {
                        const staticMatch = stripped.match(STATIC_DECL_RE);
                        if (staticMatch) {
                            const [, , varName, annot] = staticMatch;
                            name = varName;
                            kind = vscode.SymbolKind.Variable;
                            detail = annot?.trim() ?? '';
                        }
                        else if (indent === 0) {
                            const declMatch = stripped.match(HOVER_DECL_RE);
                            if (declMatch) {
                                const [, , , varName, annot] = declMatch;
                                name = varName;
                                kind = vscode.SymbolKind.Variable;
                                detail = annot?.trim() ?? '';
                            }
                        }
                    }
                }
            }
        }
        if (!name)
            continue;
        const nameIdx = rawLine.indexOf(name, indent);
        const selRange = nameIdx >= 0
            ? new vscode.Range(i, nameIdx, i, nameIdx + name.length)
            : document.lineAt(i).range;
        const bodyEnd = isContainer ? findBodyEndLine(document, i, indent) : i + 1;
        const lastLine = Math.min(bodyEnd - 1, document.lineCount - 1);
        const bodyRange = new vscode.Range(i, 0, lastLine, document.lineAt(lastLine).text.length);
        const sym = new vscode.DocumentSymbol(name, detail, kind, bodyRange, selRange);
        if (stack.length === 0) {
            result.push(sym);
        }
        else {
            stack[stack.length - 1].sym.children.push(sym);
        }
        if (isContainer) {
            stack.push({ sym, indent });
        }
    }
    return result;
}
exports.provideDocumentSymbols = provideDocumentSymbols;
// ===== Signature help provider =====
function provideSignatureHelp(document, position) {
    const prefix = document.lineAt(position.line).text.substring(0, position.character);
    let depth = 0;
    let activeParam = 0;
    let funcName = '';
    for (let i = prefix.length - 1; i >= 0; i--) {
        const c = prefix[i];
        if (c === ')' || c === ']' || c === '}') {
            depth++;
            continue;
        }
        if (c === '(' || c === '[' || c === '{') {
            if (c === '(' && depth === 0) {
                const m = prefix.substring(0, i).trimEnd().match(/([A-Za-z_]\w*)$/);
                funcName = m?.[1] ?? '';
                break;
            }
            if (depth > 0)
                depth--;
            continue;
        }
        if (c === ',' && depth === 0)
            activeParam++;
    }
    if (!funcName)
        return undefined;
    const symbols = collectHoverSymbols(document);
    const funcSym = symbols.find(s => s.name === funcName && s.kind === 'function');
    if (!funcSym?.signature)
        return undefined;
    const sigInfo = new vscode.SignatureInformation(funcSym.signature, funcSym.doc ? new vscode.MarkdownString(funcSym.doc) : undefined);
    const paramsMatch = funcSym.signature.match(/\(([^)]*)\)/);
    if (paramsMatch?.[1]?.trim()) {
        for (const p of splitComma(paramsMatch[1])) {
            const trimmed = p.trim();
            if (trimmed)
                sigInfo.parameters.push(new vscode.ParameterInformation(trimmed));
        }
    }
    const help = new vscode.SignatureHelp();
    help.signatures = [sigInfo];
    help.activeSignature = 0;
    help.activeParameter = Math.min(activeParam, Math.max(0, sigInfo.parameters.length - 1));
    return help;
}
exports.provideSignatureHelp = provideSignatureHelp;
// ===== Definition provider =====
function provideDefinition(document, position) {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_]\w*/);
    if (!range)
        return undefined;
    const name = document.getText(range);
    const symbols = collectHoverSymbols(document);
    const symbol = selectHoverSymbol(symbols, name, position.line);
    if (!symbol)
        return undefined;
    const targetText = document.lineAt(symbol.line).text;
    const nameIdx = targetText.indexOf(symbol.name);
    const targetRange = nameIdx >= 0
        ? new vscode.Range(symbol.line, nameIdx, symbol.line, nameIdx + symbol.name.length)
        : document.lineAt(symbol.line).range;
    return new vscode.Location(document.uri, targetRange);
}
exports.provideDefinition = provideDefinition;
// ===== Semantic tokens (import alias highlighting) =====
exports.SEMANTIC_TOKENS_LEGEND = new vscode.SemanticTokensLegend(['class'], // token types: index 0 = 'class'
[] // modifiers: none
);
function escapeRegex(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
function provideDocumentSemanticTokens(document) {
    const builder = new vscode.SemanticTokensBuilder(exports.SEMANTIC_TOKENS_LEGEND);
    const aliases = collectImportAliases(document);
    if (aliases.size === 0)
        return builder.build();
    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const lineText = document.lineAt(lineIdx).text;
        const lineMatches = [];
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
exports.provideDocumentSemanticTokens = provideDocumentSemanticTokens;
//# sourceMappingURL=type_infer.js.map