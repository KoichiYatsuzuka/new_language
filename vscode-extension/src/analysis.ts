import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { type LangType, BUILTIN_RETURN_TYPES, BUILTIN_TYPE_METHODS, FUNC_DEF_RE } from './builtins';
import { type TK, type Token, tokenize } from './tokenizer';
import {
    type CppClassInfo, type NativeModuleInfo,
    importKindOf, loadNativeModuleInfo, parseTlStub,
} from './native_module';

export type { LangType, CppClassInfo, NativeModuleInfo };

// ===== Regex constants =====

const DECL_RE          = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*=(?!=)/;
const STATIC_DECL_RE   = /^(\s*)static\s+mut\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*=(?!=)\s*(.*)/;
const RETURN_RE        = /^return(?:\s+(.+))?$/;
const CLASS_NAME_RE    = /^\s*(?:class|trait|enum)\s+([A-Za-z_]\w*)/;
const NEW_TYPE_NAME_RE = /^\s*new_type\s+([A-Za-z_]\w*)\s*:/;
const HOVER_DECL_RE    = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*(?:=(?!=)\s*(.*))?$/;
const CLASS_DEF_RE     = /^(\s*)(class|trait|enum)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*(?:\(([^)]*)\))?\s*:/;
const NEW_TYPE_RE      = /^(\s*)new_type\s+([A-Za-z_]\w*)\s*:\s*([A-Za-z_][\w\[\], ]*)/;
const FREEZE_RE        = /^\s*freeze(?:\s*\(\s*|\s+)([A-Za-z_]\w*)(?:\s*\))?/;
const ACCESS_SECTION_RE = /^(\s*)(public|private|protected)\s*:\s*$/;
const TUPLE_DECL_RE    = /^(\s*)(let|mut)\s+((?:[A-Za-z_]\w*\s*,\s*)+[A-Za-z_]\w*)\s*=(?!=)\s*(.*)/;
const IMPORT_RE        = /^\s*(import(?:\[(?:py(?:-int)?|hvc?|cpp-(?:lib|dll))\])?)\s+([\w.]+)(?:\s+with\s+(\w+))?\s+as\s+([A-Za-z_]\w*)/;
const TYPEGUARD_IS_NOT_RE = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+not\s+([A-Za-z_]\w*)\s*:/;
const TYPEGUARD_IS_RE     = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+([A-Za-z_]\w*)\s*:/;

export { DECL_RE, STATIC_DECL_RE, HOVER_DECL_RE, CLASS_DEF_RE, NEW_TYPE_RE, IMPORT_RE, TUPLE_DECL_RE };

// ===== Types =====

export type HoverKind = 'variable' | 'function' | 'class' | 'trait' | 'enum' | 'new_type' | 'module';

export interface HoverSymbol {
    name: string;
    kind: HoverKind;
    line: number;
    scopeEndLine?: number;
    mutability?: string;
    type?: string;
    signature?: string;
    doc?: string;
    traits?: string[];
    originalType?: string;
    access?: 'public' | 'private' | 'protected';
}

export interface ScopeOverride {
    varName: string;
    narrowedType: LangType;
    startLine: number;
    endLine: number;
}

export interface FuncDef {
    name: string;
    defLine: number;
    defIndent: number;
    annotation: LangType | undefined;
    enclosingClass?: string;
}

type DocModuleInfo = {
    funcTypes: Map<string, Map<string, string>>;
    funcSigs:  Map<string, Map<string, string>>;
    classMethods: Map<string, Map<string, string>>;
    cppClasses: Map<string, CppClassInfo>;
};

// ===== String utilities =====

export function stripComment(line: string): string {
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

export function splitComma(s: string): string[] {
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

export function resolveSelf(type: LangType, enclosingClass: string | undefined): LangType {
    return type === 'Self' && enclosingClass ? enclosingClass : type;
}

export function extractTupleElemTypes(tupleType: LangType, count: number): LangType[] {
    const m = tupleType.match(/^tuple\[(.+)\]$/);
    if (!m) return Array(count).fill('unknown');
    return splitComma(m[1]).map(e => e.trim());
}

export function findBlockBounds(document: vscode.TextDocument, ifLine: number, ifIndent: number): { startLine: number; endLine: number } {
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

export function findBodyEndLine(document: vscode.TextDocument, defLine: number, defIndent: number): number {
    for (let j = defLine + 1; j < document.lineCount; j++) {
        const raw = document.lineAt(j).text;
        if (!raw.trim()) continue;
        const indent = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (indent <= defIndent) return j;
    }
    return document.lineCount;
}

export function parseParams(paramsStr: string, defLine: number, bodyEndLine: number): HoverSymbol[] {
    const symbols: HoverSymbol[] = [];
    for (const part of splitComma(paramsStr)) {
        const trimmed = part.trim();
        if (!trimmed) continue;
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

export function cleanTypeAnnotation(value: string | undefined): string | undefined {
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

export function getDocstringAfter(document: vscode.TextDocument, line: number, indent: number): string | undefined {
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

// ===== Expression type inferrer =====

function mergeNumeric(a: LangType, b: LangType): LangType {
    if (a === 'float' || b === 'float') return 'float';
    if (a === 'int' && b === 'int') return 'int';
    return 'unknown';
}

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
        private readonly pyClassMethods: ReadonlyMap<string, ReadonlyMap<string, string>> = new Map(),
        private readonly templateParams: ReadonlyMap<string, string> = new Map()
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

    private parseCastType(): LangType {
        if (this.cur().kind !== 'IDENT') return 'unknown';
        let typeName = this.eat().value;
        if (this.cur().kind === 'LBRACKET') {
            this.eat();
            const parts: string[] = [];
            let depth = 1;
            while (depth > 0 && this.cur().kind !== 'EOF') {
                const tok = this.eat();
                if (tok.kind === 'LBRACKET') { depth++; parts.push('['); }
                else if (tok.kind === 'RBRACKET') { depth--; if (depth > 0) parts.push(']'); }
                else { parts.push(tok.value); }
            }
            typeName = `${typeName}[${parts.join('')}]`;
        }
        return typeName;
    }

    private parseCast(): LangType {
        let t = this.parsePower();
        while (
            this.cur().kind === 'OTHER' && this.cur().value === '=' &&
            this.tokens[this.pos + 1]?.kind === 'GT'
        ) {
            this.eat(); // eat '='
            this.eat(); // eat '>'
            t = this.parseCastType();
        }
        return t;
    }

    private parseUnary(): LangType {
        if (this.cur().kind === 'MINUS' || this.cur().kind === 'PLUS') { this.eat(); return this.parseUnary(); }
        if (this.cur().kind === 'TILDE') { this.eat(); this.parseUnary(); return 'int'; }
        return this.parseCast();
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
                let lastMember = '';
                while (this.cur().kind === 'OTHER' && this.cur().value === '.') {
                    this.eat();
                    if (this.cur().kind === 'IDENT') { lastMember = this.cur().value; this.eat(); }
                    isChained = true;
                }
                let typeArg: string | undefined;
                if (!isChained && this.cur().kind === 'LBRACKET') {
                    this.eat();
                    if (this.cur().kind === 'IDENT') { typeArg = this.cur().value; this.eat(); }
                    let depth = 1;
                    while (depth > 0 && this.cur().kind !== 'EOF') {
                        if (this.cur().kind === 'LBRACKET') depth++;
                        else if (this.cur().kind === 'RBRACKET') depth--;
                        this.eat();
                    }
                }
                if (this.cur().kind === 'LPAREN') {
                    this.eat();
                    while (this.cur().kind !== 'RPAREN' && this.cur().kind !== 'EOF') {
                        this.parseOr();
                        if (this.cur().kind === 'COMMA') this.eat(); else break;
                    }
                    if (this.cur().kind === 'RPAREN') this.eat();
                    if (isChained) {
                        const baseType = this.env.get(name) ?? 'unknown';
                        return BUILTIN_TYPE_METHODS[baseType]?.[lastMember]?.ret ?? 'unknown';
                    }
                    if (name in BUILTIN_RETURN_TYPES) return BUILTIN_RETURN_TYPES[name];
                    const retType = this.funcEnv.get(name) ?? 'unknown';
                    if (typeArg && retType !== 'unknown') {
                        if (retType === name) return `${name}[${typeArg}]`;
                        const tParam = this.templateParams.get(name);
                        if (tParam && retType === tParam) return typeArg;
                    }
                    return retType;
                }
                if (isChained) return this.funcEnv.has(name) ? name : 'unknown';
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
    pyClassMethods: ReadonlyMap<string, ReadonlyMap<string, string>> = new Map(),
    templateParams: ReadonlyMap<string, string> = new Map()
): LangType {
    const trimmed = src.trim();
    const dotMatch = trimmed.match(/^([A-Za-z_]\w*)\./);
    if (dotMatch && importAliases.has(dotMatch[1])) {
        const alias = dotMatch[1];
        const callMatch = trimmed.match(/^[A-Za-z_]\w*\.([A-Za-z_]\w*)\s*\(/);
        if (callMatch) {
            const memberName = callMatch[1];
            const pyTypes = importFuncTypes.get(alias);
            if (pyTypes) {
                const retType = pyTypes.get(memberName);
                if (retType !== undefined) return retType;
            }
            if (/^[A-Z]/.test(memberName)) return memberName;
        }
        return 'unknown';
    }
    return new ExprInferrer(tokenize(src), env, funcEnv, pyClassMethods, templateParams).infer();
}

// ===== Collection functions =====

export function collectImportAliases(document: vscode.TextDocument): Map<string, string> {
    const aliases = new Map<string, string>();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(IMPORT_RE);
        if (m) aliases.set(m[4], m[2]);
    }
    return aliases;
}

interface PyModuleInfo {
    funcs: Map<string, string>;
    sigs: Map<string, string>;
    classes: Map<string, Map<string, string>>;
}

function collectPyModuleInfo(moduleName: string, docDir: string, extraPaths: string[] = []): PyModuleInfo {
    const funcs = new Map<string, string>();
    const sigs = new Map<string, string>();
    const classes = new Map<string, Map<string, string>>();

    let content: string | undefined;
    for (const searchDir of [docDir, ...extraPaths]) {
        for (const ext of ['.pyi', '.py']) {
            const candidate = path.join(searchDir, moduleName + ext);
            if (fs.existsSync(candidate)) {
                try { content = fs.readFileSync(candidate, 'utf8'); break; } catch { /* ignore */ }
            }
        }
        if (content !== undefined) break;
    }
    if (content === undefined) return { funcs, sigs, classes };

    let currentClass: string | null = null;
    let classIndent = -1;
    const classRe = /^(\s*)class\s+([A-Za-z_]\w*)/;
    const funcRe = /^(\s*)def\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:/;
    const moduleVarRe = /^([A-Za-z_]\w*)\s*:\s*([A-Za-z_][\w\[\], |.]*)/;

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
        if (funcM) {
            const [, , name, params, retAnnot] = funcM;
            const retType = retAnnot ? retAnnot.trim().replace(/^['"]|['"]$/g, '') : 'None';
            if (currentClass !== null) {
                classes.get(currentClass)!.set(name, retType);
            } else {
                funcs.set(name, retType);
                sigs.set(name, `def ${name}(${params.trim()}) -> ${retType}`);
            }
            continue;
        }
        if (currentClass === null && lineIndent === 0) {
            const varM = line.match(moduleVarRe);
            if (varM) {
                funcs.set(varM[1], varM[2].trim());
                sigs.set(varM[1], `${varM[1]}: ${varM[2].trim()}`);
            }
        }
    }
    return { funcs, sigs, classes };
}

function collectAllPyModuleInfo(document: vscode.TextDocument): DocModuleInfo {
    const funcTypes = new Map<string, Map<string, string>>();
    const funcSigs  = new Map<string, Map<string, string>>();
    const classMethods = new Map<string, Map<string, string>>();
    const cppClasses = new Map<string, CppClassInfo>();
    const docDir = path.dirname(document.uri.fsPath);
    const pythonLibPaths: string[] = vscode.workspace.getConfiguration('havakyrie').get('pythonLibraryPaths', []);
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(IMPORT_RE);
        if (!m) continue;
        const [, importKind, modulePath, stubName, alias] = m;
        if (importKindOf(importKind) === 'py') {
            const info = collectPyModuleInfo(modulePath, docDir, pythonLibPaths);
            funcTypes.set(alias, info.funcs);
            funcSigs.set(alias, info.sigs);
            for (const [cls, methods] of info.classes) classMethods.set(cls, methods);
        } else {
            const info = loadNativeModuleInfo(importKind, modulePath, stubName, docDir);
            if (info.funcs.size > 0) {
                funcTypes.set(alias, info.funcs);
                funcSigs.set(alias, info.sigs);
            }
            if (importKindOf(importKind) === 'cpp') {
                for (const [className, classInfo] of info.classes) {
                    cppClasses.set(className, classInfo);
                }
            }
        }
    }
    return { funcTypes, funcSigs, classMethods, cppClasses };
}

export function collectConstructorTypes(document: vscode.TextDocument): Map<string, LangType> {
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

export function collectFuncDefs(document: vscode.TextDocument): FuncDef[] {
    const defs: FuncDef[] = [];
    const classStack: Array<{ name: string; indent: number }> = [];
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        if (!stripped.trim()) continue;
        const lineIndent = (stripped.match(/^(\s*)/)?.[1] ?? '').length;
        while (classStack.length > 0 && lineIndent <= classStack[classStack.length - 1].indent) {
            classStack.pop();
        }
        const classM = stripped.match(CLASS_DEF_RE);
        if (classM) { classStack.push({ name: classM[3], indent: (classM[1] ?? '').length }); continue; }
        const m = stripped.match(FUNC_DEF_RE);
        if (!m) continue;
        const [, indentStr, , name, , retAnnotation] = m;
        defs.push({
            name,
            defLine: i,
            defIndent: indentStr.length,
            annotation: parseTypeAnnotation(retAnnotation),
            enclosingClass: classStack.at(-1)?.name,
        });
    }
    return defs;
}

export function collectTemplateParams(document: vscode.TextDocument): Map<string, string> {
    const map = new Map<string, string>();
    const re = /^\s*(?:fn|gen)\s+([A-Za-z_]\w*)\[([A-Za-z_]\w*)/;
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(re);
        if (m) map.set(m[1], m[2]);
    }
    return map;
}

export function inferBodyReturnType(
    document: vscode.TextDocument,
    defLine: number,
    defIndent: number,
    funcEnv: ReadonlyMap<string, LangType>,
    importAliases: ReadonlySet<string> = new Set(),
    importFuncTypes: ReadonlyMap<string, ReadonlyMap<string, string>> = new Map(),
    pyClassMethods: ReadonlyMap<string, ReadonlyMap<string, string>> = new Map(),
    templateParams: ReadonlyMap<string, string> = new Map()
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
            localEnv.set(declM[3], inferExprType(rhs, localEnv, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams));
        }

        const retM = trimmed.match(RETURN_RE);
        if (retM) {
            const retExpr = retM[1]?.trim();
            returnTypes.push(retExpr ? inferExprType(retExpr, localEnv, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams) : 'None');
        }
    }

    if (returnTypes.length === 0) return 'None';
    const unique = [...new Set(returnTypes)];
    return unique.length === 1 ? unique[0] : 'unknown';
}

// ===== Symbol collection =====

// Accepts pre-built funcEnv from DocumentAnalysis to avoid redundant work.
function collectHoverSymbols(
    document: vscode.TextDocument,
    funcEnv: Map<string, LangType>,
    importAliases: ReadonlySet<string>,
    importFuncTypes: ReadonlyMap<string, ReadonlyMap<string, string>>,
    pyClassMethods: ReadonlyMap<string, ReadonlyMap<string, string>>,
    cppClasses: ReadonlyMap<string, CppClassInfo>,
    templateParams: ReadonlyMap<string, string>
): HoverSymbol[] {
    const symbols: HoverSymbol[] = [];
    const env = new Map<string, LangType>();

    const classContextStack: Array<{ name: string; indent: number; bodyIndent: number; access: 'public' | 'private' | 'protected' }> = [];

    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = stripComment(raw);
        const trimmedLine = stripped.trim();

        const lineIndentLen = (stripped.match(/^(\s*)/)?.[1] ?? '').length;

        if (trimmedLine) {
            while (classContextStack.length > 0 && lineIndentLen <= classContextStack[classContextStack.length - 1].indent) {
                classContextStack.pop();
            }
            if (classContextStack.length > 0) {
                const top = classContextStack[classContextStack.length - 1];
                if (top.bodyIndent === -1) top.bodyIndent = lineIndentLen;
            }
        }

        const accessM = trimmedLine ? stripped.match(ACCESS_SECTION_RE) : null;
        if (accessM && classContextStack.length > 0) {
            const top = classContextStack[classContextStack.length - 1];
            if (top.bodyIndent !== -1 && lineIndentLen === top.bodyIndent) {
                top.access = accessM[2] as 'public' | 'private' | 'protected';
            }
            continue;
        }

        const currentAccess: 'public' | 'private' | 'protected' | undefined = (() => {
            if (classContextStack.length === 0) return undefined;
            const top = classContextStack[classContextStack.length - 1];
            if (top.bodyIndent === -1 || lineIndentLen !== top.bodyIndent) return undefined;
            return top.access;
        })();

        const importMatch = stripped.match(IMPORT_RE);
        if (importMatch) {
            const [, importKind, modulePath, , alias] = importMatch;
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

        const funcMatch = stripped.match(FUNC_DEF_RE);
        if (funcMatch) {
            const [, indentStr, kind, name, params, retAnnotation] = funcMatch;
            const enclosingClass = classContextStack.at(-1)?.name;
            const rawReturnType = cleanTypeAnnotation(retAnnotation) ?? funcEnv.get(name) ?? 'unknown';
            const returnType = resolveSelf(rawReturnType, enclosingClass);
            symbols.push({
                name,
                kind: 'function',
                line: i,
                type: returnType,
                signature: `${kind} ${name}(${params}) -> ${returnType}`,
                doc: getDocstringAfter(document, i, indentStr.length),
                access: currentAccess,
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

        const staticMatch = stripped.match(STATIC_DECL_RE);
        if (staticMatch) {
            const [, , name, annotation, rhs] = staticMatch;
            const type = cleanTypeAnnotation(annotation)
                ?? (rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams) : 'unknown');
            symbols.push({ name, kind: 'variable', line: i, mutability: 'static', type, access: currentAccess });
            env.set(name, type);
            continue;
        }

        const tupleM = stripped.match(TUPLE_DECL_RE);
        if (tupleM) {
            const [, , mutability, names, rhs] = tupleM;
            const rhsType = rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams) : 'unknown';
            const nameList = names.split(',').map(n => n.trim()).filter(Boolean);
            const elemTypes = extractTupleElemTypes(rhsType, nameList.length);
            for (let idx = 0; idx < nameList.length; idx++) {
                const varName = nameList[idx];
                const elemType = elemTypes[idx] ?? 'unknown';
                symbols.push({ name: varName, kind: 'variable', line: i, mutability, type: elemType, access: currentAccess });
                env.set(varName, elemType);
            }
            continue;
        }

        const declMatch = stripped.match(HOVER_DECL_RE);
        if (declMatch) {
            const [, , mutability, name, annotation, rhs] = declMatch;
            const type = cleanTypeAnnotation(annotation)
                ?? (rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams) : 'unknown');
            symbols.push({ name, kind: 'variable', line: i, mutability, type, access: currentAccess });
            env.set(name, parseTypeAnnotation(type) ?? 'unknown');
        }
    }

    return symbols;
}

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
        const isNotMatch = stripped.match(TYPEGUARD_IS_NOT_RE);
        if (isNotMatch) {
            const [, , varName, typeName] = isNotMatch;
            const { startLine, endLine } = findBlockBounds(document, i, indent);
            const narrowedType = computeIsNotNarrowedType(declaredTypes.get(varName), typeName);
            if (narrowedType) overrides.push({ varName, narrowedType, startLine, endLine });
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

function collectFreezeLines(document: vscode.TextDocument): Map<string, number> {
    const map = new Map<string, number>();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(FREEZE_RE);
        if (m) map.set(m[1], i);
    }
    return map;
}

// ===== Symbol selection =====

export function selectHoverSymbol(symbols: HoverSymbol[], name: string, line: number): HoverSymbol | undefined {
    const matches = symbols.filter(s => s.name === name);
    const visible = matches
        .filter(s => s.line <= line && (s.scopeEndLine === undefined || line < s.scopeEndLine))
        .sort((a, b) => b.line - a.line);
    return visible[0] ?? matches[0];
}

// ===== DocumentAnalysis — single cached analysis per document version =====

export class DocumentAnalysis {
    readonly symbols: HoverSymbol[];
    readonly funcDefs: FuncDef[];
    readonly funcEnv: Map<string, LangType>;
    readonly importAliases: ReadonlySet<string>;
    readonly importFuncTypes: ReadonlyMap<string, ReadonlyMap<string, string>>;
    readonly importFuncSigs: ReadonlyMap<string, ReadonlyMap<string, string>>;
    readonly classMethods: ReadonlyMap<string, ReadonlyMap<string, string>>;
    readonly cppClasses: ReadonlyMap<string, CppClassInfo>;
    readonly templateParams: ReadonlyMap<string, string>;
    readonly freezeLines: ReadonlyMap<string, number>;
    readonly scopeOverrides: readonly ScopeOverride[];
    readonly classTraitsMap: ReadonlyMap<string, string[]>;

    private static readonly _cache = new Map<string, { version: number; data: DocumentAnalysis }>();

    static for(document: vscode.TextDocument): DocumentAnalysis {
        const key = document.uri.toString();
        const cached = DocumentAnalysis._cache.get(key);
        if (cached?.version === document.version) return cached.data;
        const data = new DocumentAnalysis(document);
        DocumentAnalysis._cache.set(key, { version: document.version, data });
        return data;
    }

    private constructor(document: vscode.TextDocument) {
        // Phase 1: import aliases
        const importAliasMap = collectImportAliases(document);
        this.importAliases = new Set(importAliasMap.keys());

        // Phase 2: module info
        const moduleInfo = collectAllPyModuleInfo(document);
        this.importFuncTypes = moduleInfo.funcTypes;
        this.importFuncSigs  = moduleInfo.funcSigs;
        this.classMethods    = moduleInfo.classMethods;
        this.cppClasses      = moduleInfo.cppClasses;

        // Phase 3: function type environment
        this.funcEnv = collectConstructorTypes(document);
        this.templateParams = collectTemplateParams(document);
        for (const [className] of this.cppClasses) {
            this.funcEnv.set(className, className);
        }
        this.funcDefs = collectFuncDefs(document);
        for (const def of this.funcDefs) {
            const rawType = def.annotation ?? inferBodyReturnType(
                document, def.defLine, def.defIndent, this.funcEnv,
                this.importAliases, this.importFuncTypes, this.classMethods, this.templateParams
            );
            this.funcEnv.set(def.name, resolveSelf(rawType, def.enclosingClass));
        }

        // Phase 4: hover symbols (uses pre-built funcEnv — no duplicate work)
        this.symbols = collectHoverSymbols(
            document, this.funcEnv, this.importAliases,
            this.importFuncTypes, this.classMethods, this.cppClasses, this.templateParams
        );

        // Phase 5: secondary analysis
        this.freezeLines    = collectFreezeLines(document);
        this.scopeOverrides = collectScopeOverrides(document, this.symbols);
        this.classTraitsMap = collectClassTraits(document);
    }
}

// ===== Built-in stub (shared state initialised from extension.ts) =====

export let builtinStub: NativeModuleInfo = {
    funcs: new Map(), sigs: new Map(), docs: new Map(), classes: new Map(),
};

export function initBuiltinStub(tlsPath: string): void {
    if (!fs.existsSync(tlsPath)) return;
    try {
        builtinStub = parseTlStub(fs.readFileSync(tlsPath, 'utf8'));
    } catch { /* ignore unreadable stub */ }
}
