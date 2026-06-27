"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.initBuiltinStub = exports.builtinStub = exports.DocumentAnalysis = exports.selectHoverSymbol = exports.inferBodyReturnType = exports.collectTemplateParams = exports.collectFuncDefs = exports.gatherFuncDefLines = exports.collectConstructorTypes = exports.collectImportAliases = exports.inferExprType = exports.getDocstringAfter = exports.cleanTypeAnnotation = exports.parseParams = exports.findBodyEndLine = exports.findBlockBounds = exports.extractIterElemType = exports.extractTupleElemTypes = exports.resolveSelf = exports.splitComma = exports.stripComment = exports.FOR_LOOP_RE = exports.TUPLE_DECL_RE = exports.IMPORT_RE = exports.NEW_TYPE_RE = exports.CLASS_DEF_RE = exports.HOVER_DECL_RE = exports.STATIC_DECL_RE = exports.DECL_RE = void 0;
const vscode = require("vscode");
const fs = require("fs");
const fs_1 = require("fs");
const path = require("path");
const builtins_1 = require("./builtins");
const tokenizer_1 = require("./tokenizer");
const native_module_1 = require("./native_module");
// ===== Regex constants =====
const DECL_RE = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*=(?!=)/;
exports.DECL_RE = DECL_RE;
const STATIC_DECL_RE = /^(\s*)static\s+mut\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*=(?!=)\s*(.*)/;
exports.STATIC_DECL_RE = STATIC_DECL_RE;
const RETURN_RE = /^return(?:\s+(.+))?$/;
const CLASS_NAME_RE = /^\s*(?:class|trait|enum)\s+([A-Za-z_]\w*)/;
const NEW_TYPE_NAME_RE = /^\s*new_type\s+([A-Za-z_]\w*)\s*:/;
const HOVER_DECL_RE = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*(?:=(?!=)\s*(.*))?$/;
exports.HOVER_DECL_RE = HOVER_DECL_RE;
const CLASS_DEF_RE = /^(\s*)(class|trait|enum)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*(?:\(([^)]*)\))?\s*:/;
exports.CLASS_DEF_RE = CLASS_DEF_RE;
const NEW_TYPE_RE = /^(\s*)new_type\s+([A-Za-z_]\w*)\s*:\s*([A-Za-z_][\w\[\], ]*)/;
exports.NEW_TYPE_RE = NEW_TYPE_RE;
const FREEZE_RE = /^\s*freeze(?:\s*\(\s*|\s+)([A-Za-z_]\w*)(?:\s*\))?/;
const ACCESS_SECTION_RE = /^(\s*)(public|private|protected)\s*:\s*$/;
const TUPLE_DECL_RE = /^(\s*)(let|mut)\s+((?:[A-Za-z_]\w*\s*,\s*)+[A-Za-z_]\w*)\s*=(?!=)\s*(.*)/;
exports.TUPLE_DECL_RE = TUPLE_DECL_RE;
// Groups: 1=indent, 2=targets (comma-sep idents), 3=iterable expression (including trailing ->type:)
const FOR_LOOP_RE = /^(\s*)for\s+((?:[A-Za-z_]\w*\s*,\s*)*[A-Za-z_]\w*)\s+in\s+(.+)$/;
exports.FOR_LOOP_RE = FOR_LOOP_RE;
// Groups: 1=kind, 2=path, 3=version[?], 4=with-stub[?], 5=alias[?]
const IMPORT_RE = /^\s*(import(?:\[(?:arc?|py(?:-int)?|rs|hvc?|cpp-(?:lib|dll)|cs-(?:dll|proc)|js-proc)\])?)\s+([\w.]+)(?:\[([^\]]*)\])?(?:\s+with\s+(\w+))?(?:\s+as\s+([A-Za-z_]\w*))?/;
exports.IMPORT_RE = IMPORT_RE;
const TYPEGUARD_IS_NOT_RE = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+not\s+([A-Za-z_]\w*)\s*:/;
const TYPEGUARD_IS_RE = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+([A-Za-z_]\w*)\s*:/;
/** Derive the binding alias from a module path and optional explicit alias.
 *  e.g. `libm` (no explicit) → `libm`; `test_modules.physics` → `physics`. */
function importAlias(modulePath, explicit) {
    if (explicit)
        return explicit;
    // Filter empty segments (e.g. "WpfShell." trailing dot → last segment is "").
    const parts = modulePath.split('.').filter(Boolean);
    return parts.length > 0 ? parts[parts.length - 1] : modulePath;
}
// ===== String utilities =====
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
exports.stripComment = stripComment;
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
exports.splitComma = splitComma;
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
function resolveSelf(type, enclosingClass) {
    return type === 'Self' && enclosingClass ? enclosingClass : type;
}
exports.resolveSelf = resolveSelf;
function extractTupleElemTypes(tupleType, count) {
    const m = tupleType.match(/^tuple\[(.+)\]$/);
    if (!m)
        return Array(count).fill('unknown');
    return splitComma(m[1]).map(e => e.trim());
}
exports.extractTupleElemTypes = extractTupleElemTypes;
function extractIterElemType(containerType) {
    const listSetM = containerType.match(/^(?:list|set)\[(.+)\]$/);
    if (listSetM)
        return listSetM[1].trim();
    if (containerType === 'str')
        return 'str';
    if (containerType === 'range')
        return 'int';
    const dictM = containerType.match(/^dict\[(.+)\]$/);
    if (dictM) {
        const parts = splitComma(dictM[1]);
        return parts.length >= 1 ? parts[0].trim() : 'unknown';
    }
    return 'unknown';
}
exports.extractIterElemType = extractIterElemType;
function findBlockBounds(document, ifLine, ifIndent) {
    var _a, _b;
    const startLine = ifLine + 1;
    let endLine = document.lineCount;
    for (let j = startLine; j < document.lineCount; j++) {
        const raw = document.lineAt(j).text;
        if (!raw.trim())
            continue;
        const indent = ((_b = (_a = raw.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
        if (indent <= ifIndent) {
            endLine = j;
            break;
        }
    }
    return { startLine, endLine };
}
exports.findBlockBounds = findBlockBounds;
function findBodyEndLine(document, defLine, defIndent) {
    var _a, _b;
    for (let j = defLine + 1; j < document.lineCount; j++) {
        const raw = document.lineAt(j).text;
        if (!raw.trim())
            continue;
        const indent = ((_b = (_a = raw.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
        if (indent <= defIndent)
            return j;
    }
    return document.lineCount;
}
exports.findBodyEndLine = findBodyEndLine;
function parseParams(paramsStr, defLine, bodyEndLine) {
    var _a;
    const symbols = [];
    for (const part of splitComma(paramsStr)) {
        const trimmed = part.trim();
        if (!trimmed)
            continue;
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
            mutability: mut !== null && mut !== void 0 ? mut : 'let',
            type: (_a = type === null || type === void 0 ? void 0 : type.trim()) !== null && _a !== void 0 ? _a : 'unknown',
        });
    }
    return symbols;
}
exports.parseParams = parseParams;
function cleanTypeAnnotation(value) {
    const cleaned = value === null || value === void 0 ? void 0 : value.trim();
    return cleaned ? cleaned : undefined;
}
exports.cleanTypeAnnotation = cleanTypeAnnotation;
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
    var _a, _b;
    for (let i = line + 1; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const trimmed = raw.trim();
        if (!trimmed)
            continue;
        const lineIndent = ((_b = (_a = raw.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
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
exports.getDocstringAfter = getDocstringAfter;
// ===== Expression type inferrer =====
function mergeNumeric(a, b) {
    if (a === 'float' || b === 'float')
        return 'float';
    if (a === 'int' && b === 'int')
        return 'int';
    return 'unknown';
}
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
    constructor(tokens, env, funcEnv, pyClassMethods = new Map(), templateParams = new Map(), classFieldTypes = new Map(), selfType = undefined) {
        this.tokens = tokens;
        this.env = env;
        this.funcEnv = funcEnv;
        this.pyClassMethods = pyClassMethods;
        this.templateParams = templateParams;
        this.classFieldTypes = classFieldTypes;
        this.selfType = selfType;
        this.pos = 0;
    }
    cur() { var _a; return (_a = this.tokens[this.pos]) !== null && _a !== void 0 ? _a : { kind: 'EOF', value: '' }; }
    eat() { var _a; return (_a = this.tokens[this.pos++]) !== null && _a !== void 0 ? _a : { kind: 'EOF', value: '' }; }
    infer() { return this.parseOr(); }
    applyBinaryOp(left, right, op) {
        const dunder = OP_DUNDER[op];
        if (dunder) {
            const lm = this.pyClassMethods.get(left);
            if (lm === null || lm === void 0 ? void 0 : lm.has(dunder[0]))
                return lm.get(dunder[0]);
            const rm = this.pyClassMethods.get(right);
            if (rm === null || rm === void 0 ? void 0 : rm.has(dunder[1]))
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
    parseCastType() {
        if (this.cur().kind !== 'IDENT')
            return 'unknown';
        let typeName = this.eat().value;
        if (this.cur().kind === 'LBRACKET') {
            this.eat();
            const parts = [];
            let depth = 1;
            while (depth > 0 && this.cur().kind !== 'EOF') {
                const tok = this.eat();
                if (tok.kind === 'LBRACKET') {
                    depth++;
                    parts.push('[');
                }
                else if (tok.kind === 'RBRACKET') {
                    depth--;
                    if (depth > 0)
                        parts.push(']');
                }
                else {
                    parts.push(tok.value);
                }
            }
            typeName = `${typeName}[${parts.join('')}]`;
        }
        return typeName;
    }
    parseCast() {
        var _a;
        let t = this.parsePower();
        while (this.cur().kind === 'OTHER' && this.cur().value === '=' &&
            ((_a = this.tokens[this.pos + 1]) === null || _a === void 0 ? void 0 : _a.kind) === 'GT') {
            this.eat(); // eat '='
            this.eat(); // eat '>'
            t = this.parseCastType();
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
        return this.parseCast();
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
        var _a, _b, _c, _d, _e, _f, _g, _h, _j, _k, _l;
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
                let lastMember = '';
                let penultimateMember = ''; // for alias.ClassName.method() 3-level chains
                while (this.cur().kind === 'OTHER' && this.cur().value === '.') {
                    this.eat();
                    if (this.cur().kind === 'IDENT') {
                        penultimateMember = lastMember;
                        lastMember = this.cur().value;
                        this.eat();
                    }
                    isChained = true;
                }
                let typeArg;
                if (!isChained && this.cur().kind === 'LBRACKET') {
                    this.eat();
                    // Capture the full type-argument list (including commas and nested brackets)
                    const argParts = [];
                    let depth = 1;
                    while (depth > 0 && this.cur().kind !== 'EOF') {
                        const t = this.eat();
                        if (t.kind === 'LBRACKET') {
                            depth++;
                            argParts.push('[');
                        }
                        else if (t.kind === 'RBRACKET') {
                            if (--depth > 0)
                                argParts.push(']');
                        }
                        else if (t.kind === 'COMMA')
                            argParts.push(', ');
                        else
                            argParts.push(t.value);
                    }
                    if (argParts.length > 0)
                        typeArg = argParts.join('');
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
                    if (isChained) {
                        const baseType = (_a = (name === 'self' ? this.selfType : this.env.get(name))) !== null && _a !== void 0 ? _a : 'unknown';
                        const builtinRet = (_c = (_b = builtins_1.BUILTIN_TYPE_METHODS[baseType]) === null || _b === void 0 ? void 0 : _b[lastMember]) === null || _c === void 0 ? void 0 : _c.ret;
                        if (builtinRet !== undefined)
                            return builtinRet;
                        // Check user-defined / cpp / rs / cs class methods
                        const classRet = (_d = this.pyClassMethods.get(baseType)) === null || _d === void 0 ? void 0 : _d.get(lastMember);
                        if (classRet !== undefined)
                            return classRet;
                        // 3-level chain: alias.ClassName.method() — try penultimate as class name.
                        // Needed for static calls like `svc.Calculator.Add(a, b)` where
                        // baseType resolves to "unknown" because the alias is not in env.
                        if (baseType === 'unknown' && penultimateMember) {
                            const staticRet = (_e = this.pyClassMethods.get(penultimateMember)) === null || _e === void 0 ? void 0 : _e.get(lastMember);
                            if (staticRet !== undefined)
                                return staticRet;
                        }
                        return (_f = this.funcEnv.get(lastMember)) !== null && _f !== void 0 ? _f : 'unknown';
                    }
                    if (name === 'Self' && this.selfType)
                        return this.selfType;
                    // Resolve from builtins first, then funcEnv; apply typeArg for both
                    const retType = (_h = (_g = (name in builtins_1.BUILTIN_RETURN_TYPES ? builtins_1.BUILTIN_RETURN_TYPES[name] : undefined)) !== null && _g !== void 0 ? _g : this.funcEnv.get(name)) !== null && _h !== void 0 ? _h : 'unknown';
                    if (typeArg && retType !== 'unknown') {
                        if (retType === name)
                            return `${name}[${typeArg}]`;
                        const tParam = this.templateParams.get(name);
                        if (tParam && retType === tParam)
                            return typeArg;
                    }
                    return retType;
                }
                if (isChained) {
                    if (lastMember) {
                        const baseType = (_j = (name === 'self' ? this.selfType : this.env.get(name))) !== null && _j !== void 0 ? _j : 'unknown';
                        if (baseType !== 'unknown') {
                            const fieldType = (_k = this.classFieldTypes.get(baseType)) === null || _k === void 0 ? void 0 : _k.get(lastMember);
                            if (fieldType !== undefined)
                                return fieldType;
                        }
                    }
                    return this.funcEnv.has(name) ? name : 'unknown';
                }
                if (name === 'Self' && this.selfType)
                    return this.selfType;
                return (_l = this.env.get(name)) !== null && _l !== void 0 ? _l : 'unknown';
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
function inferExprType(src, env, funcEnv = new Map(), importAliases = new Set(), importFuncTypes = new Map(), pyClassMethods = new Map(), templateParams = new Map(), classFieldTypes = new Map(), selfType = undefined) {
    var _a;
    const trimmed = src.trim();
    // Block-expression RHS (if/for/while/match/block): extract -> ReturnType annotation
    if (/^(?:if|for|while|match|block)\b/.test(trimmed)) {
        const arrowM = trimmed.match(/->[ \t]*([A-Za-z_][\w\[\], ]*)[ \t]*:?\s*$/);
        return arrowM ? arrowM[1].trim() : 'unknown';
    }
    const dotMatch = trimmed.match(/^([A-Za-z_]\w*)\./);
    if (dotMatch && importAliases.has(dotMatch[1])) {
        const alias = dotMatch[1];
        const isPyModule = importFuncTypes.has(alias);
        const callMatch = trimmed.match(/^[A-Za-z_]\w*\.([A-Za-z_]\w*)\s*\(/);
        if (callMatch) {
            const memberName = callMatch[1];
            const pyTypes = importFuncTypes.get(alias);
            if (pyTypes) {
                const retType = pyTypes.get(memberName);
                if (retType !== undefined)
                    return retType;
            }
            if (/^[A-Z]/.test(memberName))
                return memberName;
        }
        // 3-level chain: alias.ClassName.staticMethod(...) — no callMatch because the
        // 2-level regex stops at ClassName before seeing the second dot.
        const staticCallM = trimmed.match(/^[A-Za-z_]\w*\.([A-Za-z_]\w*)\.([A-Za-z_]\w*)\s*\(/);
        if (staticCallM) {
            const className = staticCallM[1];
            const methodName = staticCallM[2];
            const ret = (_a = pyClassMethods.get(className)) === null || _a === void 0 ? void 0 : _a.get(methodName);
            if (ret !== undefined)
                return ret;
        }
        return isPyModule ? 'Any' : 'unknown';
    }
    return new ExprInferrer((0, tokenizer_1.tokenize)(src), env, funcEnv, pyClassMethods, templateParams, classFieldTypes, selfType).infer();
}
exports.inferExprType = inferExprType;
// ===== Collection functions =====
function collectImportAliases(document) {
    const aliases = new Map();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(IMPORT_RE);
        if (m) {
            const alias = importAlias(m[2], m[5]);
            aliases.set(alias, m[2]);
        }
    }
    return aliases;
}
exports.collectImportAliases = collectImportAliases;
// ──────────────────────────────────────────────────────────────────────────────
// Python source analysis helpers
// ──────────────────────────────────────────────────────────────────────────────
function pyLineIndent(line) {
    var _a, _b;
    return ((_b = (_a = line.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
}
function inferPyLiteralType(expr) {
    const e = expr.trim();
    if (!e || e === 'None')
        return 'None';
    if (e === 'True' || e === 'False')
        return 'bool';
    if (/^-?\d+$/.test(e))
        return 'int';
    if (/^-?\d*\.\d+([eE][+-]?\d+)?$/.test(e) || /^-?\d+[eE][+-]?\d+$/.test(e))
        return 'float';
    if (/^[bBfFrRuU]{0,2}["']/.test(e) || e.startsWith('"""') || e.startsWith("'''"))
        return 'str';
    if (e === '[]' || e.startsWith('['))
        return 'list';
    if (e.startsWith('('))
        return 'tuple';
    if (e === '{}' || (e.startsWith('{') && e.includes(':')))
        return 'dict';
    if (e.startsWith('{'))
        return 'set';
    return 'Any';
}
/** Parse a (possibly multi-line) Python `def` or `async def` starting at startIdx. */
function parsePyFuncDef(lines, startIdx) {
    var _a;
    const startLine = lines[startIdx];
    const defM = startLine.match(/^(\s*)(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(/);
    if (!defM)
        return undefined;
    const indent = defM[1].length;
    const name = defM[2];
    let depth = 0, combined = '', bodyLine = startIdx + 1;
    for (let i = startIdx; i < Math.min(lines.length, startIdx + 40); i++) {
        const raw = lines[i].replace(/#[^"']*$/, ''); // strip inline comments (simplified)
        combined += (i === startIdx ? '' : ' ') + raw.trim();
        for (const ch of raw) {
            if (ch === '(')
                depth++;
            else if (ch === ')')
                depth--;
        }
        if (depth <= 0) {
            bodyLine = i + 1;
            break;
        }
    }
    // Find the first `(` and its matching `)` within the combined text
    let pDepth = 0, openIdx = -1, closeIdx = -1;
    for (let j = 0; j < combined.length; j++) {
        if (combined[j] === '(') {
            if (openIdx === -1)
                openIdx = j;
            pDepth++;
        }
        else if (combined[j] === ')') {
            pDepth--;
            if (pDepth === 0) {
                closeIdx = j;
                break;
            }
        }
    }
    if (openIdx === -1 || closeIdx === -1)
        return undefined;
    const params = combined.slice(openIdx + 1, closeIdx).trim();
    const after = combined.slice(closeIdx + 1).trim(); // e.g. "-> str:" or ":"
    const retM = after.match(/^->\s*(.+?)\s*:/);
    return { name, params, retAnnot: (_a = retM === null || retM === void 0 ? void 0 : retM[1]) === null || _a === void 0 ? void 0 : _a.trim(), bodyLine, indent };
}
/** Scan a function body for `return` statements and infer the return type from literals. */
function inferPyBodyReturnType(lines, bodyStart, funcIndent) {
    var _a;
    const types = new Set();
    let hasReturn = false;
    for (let i = bodyStart; i < lines.length; i++) {
        const line = lines[i];
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith('#'))
            continue;
        const indent = pyLineIndent(line);
        if (indent <= funcIndent)
            break; // left function body
        // Skip nested def/class blocks to avoid their `return`s
        if (trimmed.startsWith('def ') || trimmed.startsWith('async def ') || trimmed.startsWith('class ')) {
            const nestedIndent = indent;
            i++;
            while (i < lines.length) {
                const t = lines[i].trim();
                if (t && pyLineIndent(lines[i]) <= nestedIndent)
                    break;
                i++;
            }
            i--;
            continue;
        }
        if (trimmed.startsWith('yield ') || trimmed === 'yield')
            return 'Any';
        const retM = trimmed.match(/^return(?:\s+(.+))?$/);
        if (retM) {
            hasReturn = true;
            const expr = ((_a = retM[1]) !== null && _a !== void 0 ? _a : '').trim();
            types.add(inferPyLiteralType(expr || 'None'));
        }
    }
    if (!hasReturn)
        return 'None';
    const all = [...types];
    const nonNone = all.filter(t => t !== 'None');
    if (nonNone.length === 0)
        return 'None';
    if (all.length === 1)
        return all[0];
    return 'Any';
}
/** Parse `__init__` body for `self.attr: type` or `self.attr = value` patterns. */
function parsePyInitAttrs(lines, bodyStart, funcIndent, target) {
    for (let i = bodyStart; i < lines.length; i++) {
        const line = lines[i];
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith('#'))
            continue;
        if (pyLineIndent(line) <= funcIndent)
            break;
        const annM = trimmed.match(/^self\.([A-Za-z_]\w*)\s*:\s*([A-Za-z_][\w\[\], |.]*)/);
        if (annM) {
            target.set(annM[1], annM[2].trim());
            continue;
        }
        const assignM = trimmed.match(/^self\.([A-Za-z_]\w*)\s*=\s*(.+)/);
        if (assignM && !target.has(assignM[1])) {
            target.set(assignM[1], inferPyLiteralType(assignM[2].trim()));
        }
    }
}
async function collectPyModuleInfo(moduleName, docDir, extraPaths = []) {
    const funcs = new Map();
    const sigs = new Map();
    const classes = new Map();
    let content;
    outer: for (const searchDir of [docDir, ...extraPaths]) {
        for (const ext of ['.pyi', '.py']) {
            const segments = moduleName.split('.');
            for (const candidate of [
                path.join(searchDir, ...segments) + ext,
                path.join(searchDir, moduleName + ext),
                path.join(searchDir, ...segments, '__init__' + ext),
            ]) {
                try {
                    content = await fs_1.promises.readFile(candidate, 'utf8');
                    break outer;
                }
                catch { /* try next */ }
            }
        }
    }
    if (content === undefined)
        return { funcs, sigs, classes };
    const lines = content.split('\n');
    const moduleVarRe = /^([A-Za-z_]\w*)\s*:\s*([A-Za-z_][\w\[\], |.]*)/;
    let i = 0;
    let currentClass = null;
    let classIndent = -1;
    while (i < lines.length) {
        const line = lines[i];
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith('#') || trimmed.startsWith('@')) {
            i++;
            continue;
        }
        const lineIndent = pyLineIndent(line);
        // Exit class scope when indentation returns to base level
        if (currentClass !== null && lineIndent <= classIndent) {
            currentClass = null;
            classIndent = -1;
        }
        // Top-level class definition
        const classM = line.match(/^(\s*)class\s+([A-Za-z_]\w*)/);
        if (classM && classM[1].length === 0) {
            currentClass = classM[2];
            classIndent = 0;
            if (!classes.has(currentClass)) {
                classes.set(currentClass, new Map());
                // Class name as callable → returns an instance of that class
                funcs.set(currentClass, currentClass);
                sigs.set(currentClass, `class[py] ${currentClass}`);
            }
            i++;
            continue;
        }
        // Function / method definition (handles multi-line signatures)
        if (trimmed.startsWith('def ') || trimmed.startsWith('async def ')) {
            const parsed = parsePyFuncDef(lines, i);
            if (parsed) {
                const { name, params, retAnnot, bodyLine, indent } = parsed;
                const retType = retAnnot
                    ? retAnnot.replace(/^['"]|['"]$/g, '').trim()
                    : inferPyBodyReturnType(lines, bodyLine, indent);
                if (currentClass !== null) {
                    const methodMap = classes.get(currentClass);
                    if (name === '__init__') {
                        parsePyInitAttrs(lines, bodyLine, indent, methodMap);
                    }
                    else if (!name.startsWith('_')) {
                        methodMap.set(name, retType);
                    }
                }
                else if (lineIndent === 0 && !name.startsWith('_')) {
                    funcs.set(name, retType);
                    sigs.set(name, `fn[py] ${name}(${params}) -> ${retType}`);
                }
                i = bodyLine;
                continue;
            }
        }
        // Module-level typed variable annotation
        if (currentClass === null && lineIndent === 0) {
            const varM = trimmed.match(moduleVarRe);
            if (varM) {
                funcs.set(varM[1], varM[2].trim());
                sigs.set(varM[1], `${varM[1]}: ${varM[2].trim()}`);
            }
        }
        i++;
    }
    return { funcs, sigs, classes };
}
async function collectAllPyModuleInfo(document) {
    const funcTypes = new Map();
    const funcSigs = new Map();
    const funcDocs = new Map();
    const classMethods = new Map();
    const cppClasses = new Map();
    const classSourceMap = new Map();
    const moduleClasses = new Map();
    const docDir = path.dirname(document.uri.fsPath);
    const pythonLibPaths = vscode.workspace.getConfiguration('arrow').get('pythonLibraryPaths', []);
    const [arConfigPyPaths, stdlibPaths] = await Promise.all([
        (0, native_module_1.getPythonSearchPaths)(docDir),
        (0, native_module_1.detectPythonLibDirs)(),
    ]);
    const allPyPaths = [...pythonLibPaths, ...arConfigPyPaths, ...stdlibPaths];
    const imports = [];
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(IMPORT_RE);
        if (!m)
            continue;
        const [, importKind, modulePath, , stubName, explicitAlias] = m;
        const alias = importAlias(modulePath, explicitAlias);
        imports.push([importKind, modulePath, stubName, alias]);
    }
    await Promise.all(imports.map(async ([importKind, modulePath, stubName, alias]) => {
        if ((0, native_module_1.importKindOf)(importKind) === 'py') {
            const info = await collectPyModuleInfo(modulePath, docDir, allPyPaths);
            funcTypes.set(alias, info.funcs);
            funcSigs.set(alias, info.sigs);
            for (const [cls, methods] of info.classes)
                classMethods.set(cls, methods);
        }
        else {
            const info = await (0, native_module_1.loadNativeModuleInfo)(importKind, modulePath, stubName, docDir);
            if (info.funcs.size > 0 || info.classes.size > 0) {
                funcTypes.set(alias, info.funcs);
                funcSigs.set(alias, info.sigs);
                if (info.docs.size > 0)
                    funcDocs.set(alias, info.docs);
            }
            const kind = (0, native_module_1.importKindOf)(importKind);
            if (kind === 'cpp' || kind === 'rs' || kind === 'cs') {
                const classList = [];
                for (const [className, classInfo] of info.classes) {
                    cppClasses.set(className, classInfo);
                    classSourceMap.set(className, { importKind, modulePath, stubName });
                    classList.push(className);
                }
                if (classList.length > 0)
                    moduleClasses.set(alias, classList);
            }
        }
    }));
    return { funcTypes, funcSigs, funcDocs, classMethods, cppClasses, classSourceMap, moduleClasses };
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
exports.collectConstructorTypes = collectConstructorTypes;
function gatherFuncDefLines(document, startLine) {
    const stripped = stripComment(document.lineAt(startLine).text);
    if (!/^\s*(fn|gen)\s/.test(stripped))
        return undefined;
    let depth = 0;
    let foundOpen = false;
    for (const ch of stripped) {
        if (ch === '(') {
            depth++;
            foundOpen = true;
        }
        else if (ch === ')')
            depth--;
    }
    if (!foundOpen)
        return undefined;
    if (depth === 0)
        return { fullLine: stripped, lastLine: startLine };
    let combined = stripped;
    for (let j = startLine + 1; j < document.lineCount; j++) {
        const cont = stripComment(document.lineAt(j).text).trim();
        for (const ch of cont) {
            if (ch === '(')
                depth++;
            else if (ch === ')')
                depth--;
        }
        combined += ' ' + cont;
        if (depth <= 0)
            return { fullLine: combined, lastLine: j };
    }
    return undefined;
}
exports.gatherFuncDefLines = gatherFuncDefLines;
function collectFuncDefs(document) {
    var _a, _b, _c, _d;
    const defs = [];
    const classStack = [];
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        if (!stripped.trim())
            continue;
        const lineIndent = ((_b = (_a = stripped.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
        while (classStack.length > 0 && lineIndent <= classStack[classStack.length - 1].indent) {
            classStack.pop();
        }
        const classM = stripped.match(CLASS_DEF_RE);
        if (classM) {
            classStack.push({ name: classM[3], indent: ((_c = classM[1]) !== null && _c !== void 0 ? _c : '').length });
            continue;
        }
        const funcDef = gatherFuncDefLines(document, i);
        const m = funcDef === null || funcDef === void 0 ? void 0 : funcDef.fullLine.match(builtins_1.FUNC_DEF_RE);
        if (!m)
            continue;
        const [, indentStr, kind, name, , retAnnotation] = m;
        defs.push({
            name,
            kind: kind,
            defLine: i,
            defIndent: indentStr.length,
            annotation: parseTypeAnnotation(retAnnotation),
            enclosingClass: (_d = classStack.at(-1)) === null || _d === void 0 ? void 0 : _d.name,
            sigEndLine: funcDef.lastLine,
        });
        i = funcDef.lastLine;
    }
    return defs;
}
exports.collectFuncDefs = collectFuncDefs;
function collectTemplateParams(document) {
    const map = new Map();
    const re = /^\s*(?:fn|gen)\s+([A-Za-z_]\w*)\[([A-Za-z_]\w*)/;
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(re);
        if (m)
            map.set(m[1], m[2]);
    }
    return map;
}
exports.collectTemplateParams = collectTemplateParams;
function inferBodyReturnType(document, defLine, defIndent, funcEnv, importAliases = new Set(), importFuncTypes = new Map(), pyClassMethods = new Map(), templateParams = new Map()) {
    var _a, _b, _c;
    const localEnv = new Map();
    const returnTypes = [];
    for (let i = defLine + 1; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const trimmed = stripped.trim();
        if (!trimmed)
            continue;
        const lineIndent = ((_b = (_a = stripped.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
        if (lineIndent <= defIndent)
            break;
        const declM = stripped.match(DECL_RE);
        if (declM) {
            const rhs = stripped.slice(declM[0].length).trim();
            localEnv.set(declM[3], inferExprType(rhs, localEnv, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams));
        }
        const retM = trimmed.match(RETURN_RE);
        if (retM) {
            const retExpr = (_c = retM[1]) === null || _c === void 0 ? void 0 : _c.trim();
            returnTypes.push(retExpr ? inferExprType(retExpr, localEnv, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams) : 'None');
        }
    }
    if (returnTypes.length === 0)
        return 'None';
    const unique = [...new Set(returnTypes)];
    return unique.length === 1 ? unique[0] : 'unknown';
}
exports.inferBodyReturnType = inferBodyReturnType;
// ===== Symbol collection =====
// Accepts pre-built funcEnv from DocumentAnalysis to avoid redundant work.
function collectHoverSymbols(document, funcEnv, importAliases, importFuncTypes, pyClassMethods, cppClasses, templateParams, classFieldTypes) {
    var _a, _b, _c, _d, _e, _f, _g, _h, _j, _k, _l, _m;
    const symbols = [];
    const env = new Map();
    const classContextStack = [];
    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = stripComment(raw);
        const trimmedLine = stripped.trim();
        const lineIndentLen = ((_b = (_a = stripped.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
        if (trimmedLine) {
            while (classContextStack.length > 0 && lineIndentLen <= classContextStack[classContextStack.length - 1].indent) {
                classContextStack.pop();
            }
            if (classContextStack.length > 0) {
                const top = classContextStack[classContextStack.length - 1];
                if (top.bodyIndent === -1)
                    top.bodyIndent = lineIndentLen;
            }
        }
        const accessM = trimmedLine ? stripped.match(ACCESS_SECTION_RE) : null;
        if (accessM && classContextStack.length > 0) {
            const top = classContextStack[classContextStack.length - 1];
            if (top.bodyIndent !== -1 && lineIndentLen === top.bodyIndent) {
                top.access = accessM[2];
            }
            continue;
        }
        const currentAccess = (() => {
            if (classContextStack.length === 0)
                return undefined;
            const top = classContextStack[classContextStack.length - 1];
            if (top.bodyIndent === -1 || lineIndentLen !== top.bodyIndent)
                return undefined;
            return top.access;
        })();
        const importMatch = stripped.match(IMPORT_RE);
        if (importMatch) {
            const [, importKind, modulePath, , , explicitAlias] = importMatch;
            const alias = importAlias(modulePath, explicitAlias);
            symbols.push({
                name: alias,
                kind: 'module',
                line: i,
                mutability: 'const',
                type: alias,
                originalType: `${importKind} ${modulePath}`,
            });
            continue;
        }
        const funcDefLines = gatherFuncDefLines(document, i);
        const funcMatch = funcDefLines === null || funcDefLines === void 0 ? void 0 : funcDefLines.fullLine.match(builtins_1.FUNC_DEF_RE);
        if (funcMatch) {
            const [, indentStr, kind, name, params, retAnnotation] = funcMatch;
            const enclosingClass = (_c = classContextStack.at(-1)) === null || _c === void 0 ? void 0 : _c.name;
            const rawReturnType = (_e = (_d = cleanTypeAnnotation(retAnnotation)) !== null && _d !== void 0 ? _d : funcEnv.get(name)) !== null && _e !== void 0 ? _e : 'unknown';
            const returnType = resolveSelf(rawReturnType, enclosingClass);
            const defLine = i;
            i = funcDefLines.lastLine;
            symbols.push({
                name,
                kind: 'function',
                line: defLine,
                type: returnType,
                signature: `${kind} ${name}(${params}) -> ${returnType}`,
                doc: getDocstringAfter(document, defLine, indentStr.length),
                access: currentAccess,
            });
            const bodyEndLine = findBodyEndLine(document, i, indentStr.length);
            for (const paramSym of parseParams(params, defLine, bodyEndLine)) {
                const resolvedType = resolveSelf((_f = paramSym.type) !== null && _f !== void 0 ? _f : 'unknown', enclosingClass);
                const resolvedSym = resolvedType !== paramSym.type ? { ...paramSym, type: resolvedType } : paramSym;
                symbols.push(resolvedSym);
                if (resolvedType && resolvedType !== 'unknown')
                    env.set(paramSym.name, resolvedType);
            }
            continue;
        }
        const classMatch = stripped.match(CLASS_DEF_RE);
        if (classMatch) {
            const [, indentStr, kind, name, bases] = classMatch;
            const traits = bases === null || bases === void 0 ? void 0 : bases.split(',').map(cleanBaseName).filter(Boolean);
            symbols.push({
                name,
                kind: kind === 'trait' ? 'trait' : (kind === 'enum' ? 'enum' : 'class'),
                line: i,
                traits,
                doc: getDocstringAfter(document, i, indentStr.length),
            });
            classContextStack.push({ name, indent: indentStr.length, bodyIndent: -1, access: 'public' });
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
        const selfType = (_g = classContextStack.at(-1)) === null || _g === void 0 ? void 0 : _g.name;
        const staticMatch = stripped.match(STATIC_DECL_RE);
        if (staticMatch) {
            const [, , name, annotation, rhs] = staticMatch;
            const type = (_h = cleanTypeAnnotation(annotation)) !== null && _h !== void 0 ? _h : (rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams, classFieldTypes, selfType) : 'unknown');
            symbols.push({ name, kind: 'variable', line: i, mutability: 'static', type, access: currentAccess });
            env.set(name, type);
            continue;
        }
        const tupleM = stripped.match(TUPLE_DECL_RE);
        if (tupleM) {
            const [, , mutability, names, rhs] = tupleM;
            const rhsType = rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams, classFieldTypes, selfType) : 'unknown';
            const nameList = names.split(',').map(n => n.trim()).filter(Boolean);
            const elemTypes = extractTupleElemTypes(rhsType, nameList.length);
            for (let idx = 0; idx < nameList.length; idx++) {
                const varName = nameList[idx];
                const elemType = (_j = elemTypes[idx]) !== null && _j !== void 0 ? _j : 'unknown';
                symbols.push({ name: varName, kind: 'variable', line: i, mutability, type: elemType, access: currentAccess });
                env.set(varName, elemType);
            }
            continue;
        }
        const forLoopM = stripped.match(FOR_LOOP_RE);
        if (forLoopM) {
            const [, , rawTargets, rawIter] = forLoopM;
            const iterExpr = rawIter.replace(/\s*(?:->[^:]+)?\s*:\s*$/, '').trim();
            let elemType;
            if (/^range\s*\(/.test(iterExpr)) {
                elemType = 'int';
            }
            else if (/^enumerate\s*\(/.test(iterExpr)) {
                const enumArgsM = iterExpr.match(/^enumerate\s*\((.+)\)\s*$/);
                if (enumArgsM) {
                    const innerType = inferExprType(splitComma(enumArgsM[1])[0].trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams, classFieldTypes, selfType);
                    elemType = `tuple[int, ${extractIterElemType(innerType)}]`;
                }
                else {
                    elemType = 'tuple[int, unknown]';
                }
            }
            else if (/^zip\s*\(/.test(iterExpr)) {
                const zipArgsM = iterExpr.match(/^zip\s*\((.+)\)\s*$/);
                if (zipArgsM) {
                    const elems = splitComma(zipArgsM[1]).map(arg => extractIterElemType(inferExprType(arg.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams, classFieldTypes, selfType)));
                    elemType = `tuple[${elems.join(', ')}]`;
                }
                else {
                    elemType = 'unknown';
                }
            }
            else {
                elemType = extractIterElemType(inferExprType(iterExpr, env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams, classFieldTypes, selfType));
            }
            const targetList = rawTargets.split(',').map(t => t.trim()).filter(Boolean);
            if (targetList.length === 1) {
                symbols.push({ name: targetList[0], kind: 'variable', line: i, mutability: 'let', type: elemType, access: currentAccess });
                env.set(targetList[0], elemType);
            }
            else {
                const subTypes = extractTupleElemTypes(elemType, targetList.length);
                for (let idx = 0; idx < targetList.length; idx++) {
                    const varType = (_k = subTypes[idx]) !== null && _k !== void 0 ? _k : 'unknown';
                    symbols.push({ name: targetList[idx], kind: 'variable', line: i, mutability: 'let', type: varType, access: currentAccess });
                    env.set(targetList[idx], varType);
                }
            }
            continue;
        }
        const declMatch = stripped.match(HOVER_DECL_RE);
        if (declMatch) {
            const [, , mutability, name, annotation, rhs] = declMatch;
            const type = (_l = cleanTypeAnnotation(annotation)) !== null && _l !== void 0 ? _l : (rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams, classFieldTypes, selfType) : 'unknown');
            symbols.push({ name, kind: 'variable', line: i, mutability, type, access: currentAccess });
            env.set(name, (_m = parseTypeAnnotation(type)) !== null && _m !== void 0 ? _m : 'unknown');
        }
    }
    return symbols;
}
function collectScopeOverrides(document, symbols) {
    var _a, _b;
    const overrides = [];
    const declaredTypes = new Map();
    for (const sym of symbols) {
        if (sym.kind === 'variable' && sym.type) {
            declaredTypes.set(sym.name, sym.type);
        }
    }
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const indent = ((_b = (_a = stripped.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
        const isNotMatch = stripped.match(TYPEGUARD_IS_NOT_RE);
        if (isNotMatch) {
            const [, , varName, typeName] = isNotMatch;
            const { startLine, endLine } = findBlockBounds(document, i, indent);
            const narrowedType = computeIsNotNarrowedType(declaredTypes.get(varName), typeName);
            if (narrowedType)
                overrides.push({ varName, narrowedType, startLine, endLine });
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
function collectClassTraits(document) {
    var _a;
    const map = new Map();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(CLASS_DEF_RE);
        if (m) {
            const [, , , name, bases] = m;
            const traits = (_a = bases === null || bases === void 0 ? void 0 : bases.split(',').map(cleanBaseName).filter(Boolean)) !== null && _a !== void 0 ? _a : [];
            map.set(name, traits);
        }
    }
    return map;
}
function collectClassFieldTypes(document) {
    var _a, _b, _c;
    const result = new Map();
    let currentClass = null;
    let classIndent = -1;
    let bodyIndent = -1;
    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = stripComment(raw);
        if (!stripped.trim())
            continue;
        const lineIndent = ((_b = (_a = raw.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
        if (currentClass !== null && lineIndent <= classIndent) {
            currentClass = null;
            classIndent = -1;
            bodyIndent = -1;
        }
        const classM = stripped.match(CLASS_DEF_RE);
        if (classM) {
            currentClass = classM[3];
            classIndent = ((_c = classM[1]) !== null && _c !== void 0 ? _c : '').length;
            bodyIndent = -1;
            result.set(currentClass, new Map());
            continue;
        }
        if (currentClass === null)
            continue;
        if (bodyIndent === -1)
            bodyIndent = lineIndent;
        if (lineIndent !== bodyIndent)
            continue;
        const fieldM = stripped.match(HOVER_DECL_RE);
        if (fieldM) {
            const [, , , name, annotation] = fieldM;
            if (annotation)
                result.get(currentClass).set(name, annotation.trim());
        }
    }
    return result;
}
function collectFreezeLines(document) {
    const map = new Map();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(FREEZE_RE);
        if (m)
            map.set(m[1], i);
    }
    return map;
}
// ===== Symbol selection =====
function selectHoverSymbol(symbols, name, line) {
    var _a;
    const matches = symbols.filter(s => s.name === name);
    const visible = matches
        .filter(s => s.line <= line && (s.scopeEndLine === undefined || line < s.scopeEndLine))
        .sort((a, b) => b.line - a.line);
    return (_a = visible[0]) !== null && _a !== void 0 ? _a : matches[0];
}
exports.selectHoverSymbol = selectHoverSymbol;
// ===== DocumentAnalysis — single cached analysis per document version =====
class DocumentAnalysis {
    static for(document) {
        const key = document.uri.toString();
        const version = document.version;
        const cached = DocumentAnalysis._cache.get(key);
        if ((cached === null || cached === void 0 ? void 0 : cached.version) === version)
            return Promise.resolve(cached.data);
        const pending = DocumentAnalysis._pending.get(key);
        if ((pending === null || pending === void 0 ? void 0 : pending.version) === version)
            return pending.promise;
        // Start a fresh build for this version (previous pending was for a different version).
        const promise = DocumentAnalysis._build(document).then(data => {
            var _a;
            // Only write the cache if it's not already holding a newer result.
            const cur = DocumentAnalysis._cache.get(key);
            if (!cur || cur.version <= version) {
                DocumentAnalysis._cache.set(key, { version, data });
            }
            if (((_a = DocumentAnalysis._pending.get(key)) === null || _a === void 0 ? void 0 : _a.version) === version) {
                DocumentAnalysis._pending.delete(key);
            }
            return data;
        }).catch(_err => {
            var _a;
            if (((_a = DocumentAnalysis._pending.get(key)) === null || _a === void 0 ? void 0 : _a.version) === version) {
                DocumentAnalysis._pending.delete(key);
            }
            return DocumentAnalysis._empty(document);
        });
        DocumentAnalysis._pending.set(key, { version, promise });
        // If stale cached data exists, return it immediately so providers are never
        // blocked while the background build catches up.
        if (cached)
            return Promise.resolve(cached.data);
        return promise;
    }
    /** Remove cached data for a document (call when document is closed). */
    static evict(uri) {
        const key = uri.toString();
        DocumentAnalysis._cache.delete(key);
        DocumentAnalysis._pending.delete(key);
    }
    static async _build(document) {
        const moduleInfo = await collectAllPyModuleInfo(document);
        return new DocumentAnalysis(document, moduleInfo);
    }
    static _empty(document) {
        return new DocumentAnalysis(document, {
            funcTypes: new Map(), funcSigs: new Map(), funcDocs: new Map(),
            classMethods: new Map(), cppClasses: new Map(), classSourceMap: new Map(),
            moduleClasses: new Map(),
        });
    }
    constructor(document, moduleInfo) {
        var _a;
        // Phase 1: import aliases
        const importAliasMap = collectImportAliases(document);
        this.importAliases = new Set(importAliasMap.keys());
        // Phase 2: module info (pre-fetched asynchronously)
        this.importFuncTypes = moduleInfo.funcTypes;
        this.importFuncSigs = moduleInfo.funcSigs;
        this.importFuncDocs = moduleInfo.funcDocs;
        this.cppClasses = moduleInfo.cppClasses;
        this.classSourceMap = moduleInfo.classSourceMap;
        this.moduleClasses = moduleInfo.moduleClasses;
        // Merge cpp/rs class method return types into classMethods so inferExprType
        // can resolve calls like v.length() when v: Vec2 (Rust-imported struct)
        const mergedMethods = new Map(moduleInfo.classMethods);
        for (const [className, classInfo] of moduleInfo.cppClasses) {
            const methodRets = new Map();
            for (const [methodName, info] of classInfo.methods) {
                methodRets.set(methodName, info.ret);
            }
            if (methodRets.size > 0)
                mergedMethods.set(className, methodRets);
        }
        this.classMethods = mergedMethods;
        // Phase 3: function type environment
        this.funcEnv = collectConstructorTypes(document);
        this.templateParams = collectTemplateParams(document);
        for (const [className] of this.cppClasses) {
            this.funcEnv.set(className, className);
        }
        this.funcDefs = collectFuncDefs(document);
        for (const def of this.funcDefs) {
            const rawType = (_a = def.annotation) !== null && _a !== void 0 ? _a : inferBodyReturnType(document, def.defLine, def.defIndent, this.funcEnv, this.importAliases, this.importFuncTypes, this.classMethods, this.templateParams);
            const resolvedType = resolveSelf(rawType, def.enclosingClass);
            // gen functions return a generator object; store generator[T] so callers
            // get the right variable type (e.g. let g = range_step(...) → generator[int])
            let envType = resolvedType;
            if (def.kind === 'gen') {
                envType = (resolvedType !== 'unknown' && resolvedType !== 'None')
                    ? `generator[${resolvedType}]`
                    : 'generator';
            }
            this.funcEnv.set(def.name, envType);
        }
        // Phase 4: hover symbols (uses pre-built funcEnv — no duplicate work)
        this.classFieldTypes = collectClassFieldTypes(document);
        this.symbols = collectHoverSymbols(document, this.funcEnv, this.importAliases, this.importFuncTypes, this.classMethods, this.cppClasses, this.templateParams, this.classFieldTypes);
        // Phase 5: secondary analysis
        this.freezeLines = collectFreezeLines(document);
        this.scopeOverrides = collectScopeOverrides(document, this.symbols);
        this.classTraitsMap = collectClassTraits(document);
    }
}
exports.DocumentAnalysis = DocumentAnalysis;
DocumentAnalysis._cache = new Map();
DocumentAnalysis._pending = new Map();
// ===== Built-in stub (shared state initialised from extension.ts) =====
exports.builtinStub = {
    funcs: new Map(), sigs: new Map(), docs: new Map(), classes: new Map(),
};
function initBuiltinStub(tlsPath) {
    if (!fs.existsSync(tlsPath))
        return;
    try {
        exports.builtinStub = (0, native_module_1.parseTlStub)(fs.readFileSync(tlsPath, 'utf8'));
    }
    catch { /* ignore unreadable stub */ }
}
exports.initBuiltinStub = initBuiltinStub;
//# sourceMappingURL=analysis.js.map