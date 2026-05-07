"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.inferExprType = inferExprType;
exports.provideHover = provideHover;
exports.provideInlayHints = provideInlayHints;
const vscode = require("vscode");
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
        };
        tokens.push({ kind: op1[src[i]] ?? 'OTHER', value: src[i] });
        i++;
    }
    tokens.push({ kind: 'EOF', value: '' });
    return tokens;
}
// ===== Expression type inferrer =====
// Known return types for built-in functions
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
class ExprInferrer {
    constructor(tokens, env, funcEnv) {
        this.tokens = tokens;
        this.env = env;
        this.funcEnv = funcEnv;
        this.pos = 0;
    }
    cur() { return this.tokens[this.pos] ?? { kind: 'EOF', value: '' }; }
    eat() { return this.tokens[this.pos++] ?? { kind: 'EOF', value: '' }; }
    infer() { return this.parseOr(); }
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
            t = mergeNumeric(t, this.parseBitXor());
        }
        return t;
    }
    parseBitXor() {
        let t = this.parseBitAnd();
        while (this.cur().kind === 'CARET') {
            this.eat();
            t = mergeNumeric(t, this.parseBitAnd());
        }
        return t;
    }
    parseBitAnd() {
        let t = this.parseShift();
        while (this.cur().kind === 'AMP') {
            this.eat();
            t = mergeNumeric(t, this.parseShift());
        }
        return t;
    }
    parseShift() {
        let t = this.parseAdditive();
        while (this.cur().kind === 'LTLT' || this.cur().kind === 'GTGT') {
            this.eat();
            t = mergeNumeric(t, this.parseAdditive());
        }
        return t;
    }
    parseAdditive() {
        let t = this.parseMultiplicative();
        while (this.cur().kind === 'PLUS' || this.cur().kind === 'MINUS') {
            const op = this.eat().kind;
            const r = this.parseMultiplicative();
            t = (op === 'PLUS' && t === 'str' && r === 'str') ? 'str' : mergeNumeric(t, r);
        }
        return t;
    }
    parseMultiplicative() {
        let t = this.parseUnary();
        const ops = ['STAR', 'SLASH', 'SLASHSLASH', 'PERCENT'];
        while (ops.includes(this.cur().kind)) {
            const op = this.eat().kind;
            const r = this.parseUnary();
            t = op === 'SLASH' ? 'float' : mergeNumeric(t, r);
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
            return mergeNumeric(base, this.parseUnary());
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
                    if (name in BUILTIN_RETURN_TYPES)
                        return BUILTIN_RETURN_TYPES[name];
                    return this.funcEnv.get(name) ?? 'unknown';
                }
                return this.env.get(name) ?? 'unknown';
            }
            case 'LPAREN': {
                this.eat();
                const t = this.parseOr();
                if (this.cur().kind === 'RPAREN')
                    this.eat();
                return t;
            }
            default:
                this.eat();
                return 'unknown';
        }
    }
}
function inferExprType(src, env, funcEnv = new Map()) {
    return new ExprInferrer(tokenize(src), env, funcEnv).infer();
}
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
// ===== Function definition scanning =====
// Matches: fn/gen name(params) -> RetType:
// Groups:  1=indent  2=kind  3=name  4=params  5=return annotation (optional)
const FUNC_DEF_RE = /^(\s*)(fn|gen)\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*(?:->\s*([A-Za-z_][\w\[\], ]*))?:/;
const DECL_RE = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*=(?!=)/;
const RETURN_RE = /^return(?:\s+(.+))?$/;
const CLASS_NAME_RE = /^\s*(?:class|trait)\s+([A-Za-z_]\w*)/;
const NEW_TYPE_NAME_RE = /^\s*new_type\s+([A-Za-z_]\w*)\s*:/;
function parseTypeAnnotation(s) {
    if (!s)
        return undefined;
    const t = s.trim();
    return t || undefined;
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
// Scan function body to infer the return type from `return` statements.
// Uses funcEnv so calls to already-known functions resolve correctly.
function inferBodyReturnType(document, defLine, defIndent, funcEnv) {
    const localEnv = new Map();
    const returnTypes = [];
    for (let i = defLine + 1; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const trimmed = stripped.trim();
        if (!trimmed)
            continue;
        const lineIndent = (stripped.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= defIndent)
            break; // exited function body
        // Build local variable env to resolve identifiers in return expressions
        const declM = stripped.match(DECL_RE);
        if (declM) {
            const rhs = stripped.slice(declM[0].length).trim();
            localEnv.set(declM[3], inferExprType(rhs, localEnv, funcEnv));
        }
        const retM = trimmed.match(RETURN_RE);
        if (retM) {
            const retExpr = retM[1]?.trim();
            returnTypes.push(retExpr ? inferExprType(retExpr, localEnv, funcEnv) : 'None');
        }
    }
    if (returnTypes.length === 0)
        return 'None';
    const unique = [...new Set(returnTypes)];
    return unique.length === 1 ? unique[0] : 'unknown';
}
const HOVER_DECL_RE = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*(?:=(?!=)\s*(.*))?$/;
const CLASS_DEF_RE = /^(\s*)(class|trait)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*(?:\(([^)]*)\))?\s*:/;
const NEW_TYPE_RE = /^(\s*)new_type\s+([A-Za-z_]\w*)\s*:\s*([A-Za-z_][\w\[\], ]*)/;
function cleanTypeAnnotation(value) {
    const cleaned = value?.trim();
    return cleaned ? cleaned : undefined;
}
function cleanBaseName(value) {
    return value.trim().replace(/\[[^\]]*\]/g, '');
}
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
function collectHoverSymbols(document) {
    const symbols = [];
    const env = new Map();
    const funcDefs = collectFuncDefs(document);
    const funcEnv = collectConstructorTypes(document);
    for (const def of funcDefs) {
        funcEnv.set(def.name, def.annotation ?? inferBodyReturnType(document, def.defLine, def.defIndent, funcEnv));
    }
    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = stripComment(raw);
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
            const [, , name, originalType] = newTypeMatch;
            symbols.push({
                name,
                kind: 'new_type',
                line: i,
                type: name,
                mutability: 'const',
                originalType: originalType.trim(),
            });
            env.set(name, 'unknown');
            continue;
        }
        const declMatch = stripped.match(HOVER_DECL_RE);
        if (declMatch) {
            const [, , mutability, name, annotation, rhs] = declMatch;
            const type = cleanTypeAnnotation(annotation)
                ?? (rhs ? inferExprType(rhs.trim(), env, funcEnv) : 'unknown');
            symbols.push({
                name,
                kind: 'variable',
                line: i,
                mutability,
                type,
            });
            env.set(name, parseTypeAnnotation(type) ?? 'unknown');
        }
    }
    return symbols;
}
function selectHoverSymbol(symbols, name, line) {
    const matches = symbols.filter(symbol => symbol.name === name);
    const visible = matches
        .filter(symbol => symbol.line <= line)
        .sort((a, b) => b.line - a.line);
    return visible[0] ?? matches[0];
}
function renderHover(symbol) {
    const md = new vscode.MarkdownString(undefined, true);
    md.isTrusted = false;
    if (symbol.kind === 'variable') {
        md.appendCodeblock(`${symbol.mutability ?? 'value'} ${symbol.name}: ${symbol.type ?? 'unknown'}`, 'tl');
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
    else {
        md.appendCodeblock(`new_type ${symbol.name}: ${symbol.originalType ?? 'unknown'}`, 'tl');
    }
    if (symbol.traits && symbol.traits.length > 0) {
        md.appendMarkdown(`\n\nTraits: ${symbol.traits.map(t => `\`${t}\``).join(', ')}`);
    }
    if (symbol.originalType) {
        md.appendMarkdown(`\n\nOriginal type: \`${symbol.originalType}\``);
    }
    if (symbol.doc) {
        md.appendMarkdown(`\n\n${symbol.doc}`);
    }
    return md;
}
function provideHover(document, position) {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_]\w*/);
    if (!range)
        return undefined;
    const name = document.getText(range);
    const symbol = selectHoverSymbol(collectHoverSymbols(document), name, position.line);
    if (!symbol)
        return undefined;
    return new vscode.Hover(renderHover(symbol), range);
}
// ===== Inlay hints provider =====
function provideInlayHints(document, _range) {
    const hints = [];
    // Phase 1: collect all function definitions
    const funcDefs = collectFuncDefs(document);
    // Phase 2: build funcEnv — constructors and annotations take priority, otherwise infer from body
    const funcEnv = collectConstructorTypes(document);
    for (const def of funcDefs) {
        funcEnv.set(def.name, def.annotation ?? inferBodyReturnType(document, def.defLine, def.defIndent, funcEnv));
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
    // Phase 4: inlay hints on variable declarations, using funcEnv for call resolution
    const env = new Map();
    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const rawLine = document.lineAt(lineIdx).text;
        const line = stripComment(rawLine);
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
        const type = inferExprType(rhs, env, funcEnv);
        env.set(name, type);
        const pos = new vscode.Position(lineIdx, nameStart + name.length);
        const hint = new vscode.InlayHint(pos, `: ${type}`, vscode.InlayHintKind.Type);
        hint.paddingLeft = true;
        hints.push(hint);
    }
    return hints;
}
//# sourceMappingURL=type_infer.js.map