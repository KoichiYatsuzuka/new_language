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
    int: 'int', uint: 'uint', float: 'float', str: 'str', bool: 'bool',
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
                while (this.cur().kind === 'OTHER' && this.cur().value === '.') {
                    this.eat();
                    if (this.cur().kind === 'IDENT') this.eat();
                    isChained = true;
                }
                // Capture type argument for template calls: Name[ConcreteType](...)
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
                    if (isChained) return 'unknown';
                    if (name in BUILTIN_RETURN_TYPES) return BUILTIN_RETURN_TYPES[name];
                    const retType = this.funcEnv.get(name) ?? 'unknown';
                    if (typeArg && retType !== 'unknown') {
                        // Constructor call: Box[MyInt](a) → Box[MyInt]
                        if (retType === name) return `${name}[${typeArg}]`;
                        // Template function: min_of[MyInt](x,y) with ->T → MyInt
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
    // Intercept alias.anything before ExprInferrer to avoid alias-name leaking as type
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
            // Uppercase name not found in known functions → treat as class constructor
            if (/^[A-Z]/.test(memberName)) return memberName;
        }
        return 'unknown';
    }
    return new ExprInferrer(tokenize(src), env, funcEnv, pyClassMethods, templateParams).infer();
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
// The optional (?:\[[^\]]*\])? skips template parameters like [T: Constraint].
const FUNC_DEF_RE = /^(\s*)(fn|gen)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:\s*$/;

// Variable declarations
const DECL_RE          = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*=(?!=)/;
const STATIC_DECL_RE   = /^(\s*)static\s+mut\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*=(?!=)\s*(.*)/;

const RETURN_RE        = /^return(?:\s+(.+))?$/;
const CLASS_NAME_RE    = /^\s*(?:class|trait|enum)\s+([A-Za-z_]\w*)/;
const NEW_TYPE_NAME_RE = /^\s*new_type\s+([A-Za-z_]\w*)\s*:/;

// Hover-specific
const HOVER_DECL_RE = /^(\s*)(let|mut|const)\s+([A-Za-z_]\w*)\s*(?::\s*([^=#]+?))?\s*(?:=(?!=)\s*(.*))?$/;
const CLASS_DEF_RE  = /^(\s*)(class|trait|enum)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*(?:\(([^)]*)\))?\s*:/;
const NEW_TYPE_RE   = /^(\s*)new_type\s+([A-Za-z_]\w*)\s*:\s*([A-Za-z_][\w\[\], ]*)/;

// freeze(varName)
const FREEZE_RE = /^\s*freeze\s*\(\s*([A-Za-z_]\w*)\s*\)/;

// Access section markers: public: / private: / protected:
const ACCESS_SECTION_RE = /^(\s*)(public|private|protected)\s*:\s*$/;

// Tuple destructuring: let/mut a, b = expr
const TUPLE_DECL_RE = /^(\s*)(let|mut)\s+((?:[A-Za-z_]\w*\s*,\s*)+[A-Za-z_]\w*)\s*=(?!=)\s*(.*)/;

// All import variants — captures: 1=keyword, 2=module path, 3=stub name (opt), 4=alias
// Handles: import[py], import[py-int], import[tl], import[tlc], import[cpp-lib],
//          import[cpp-dll], and bare `import` (auto mode)
const IMPORT_RE = /^\s*(import(?:\[(?:py(?:-int)?|tlc?|cpp-(?:lib|dll))\])?)\s+([\w.]+)(?:\s+with\s+(\w+))?\s+as\s+([A-Za-z_]\w*)/;

// Typeguard — check is-not before is to avoid accidental match
const TYPEGUARD_IS_NOT_RE = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+not\s+([A-Za-z_]\w*)\s*:/;
const TYPEGUARD_IS_RE     = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+([A-Za-z_]\w*)\s*:/;

// ===== Types =====

type HoverKind = 'variable' | 'function' | 'class' | 'trait' | 'enum' | 'new_type' | 'module';

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
    access?: 'public' | 'private' | 'protected';  // class/trait members only
}

interface ScopeOverride {
    varName: string;
    narrowedType: LangType;
    startLine: number;
    endLine: number;        // exclusive
}

// ===== C++ / TL native module support =====

function importKindOf(keyword: string): 'py' | 'cpp' | 'tl' {
    if (keyword.includes('cpp')) return 'cpp';
    if (keyword === 'import' || keyword.startsWith('import[tl')) return 'tl';
    return 'py';
}

/** Map a C/C++ type string to the corresponding tl primitive type. */
function cTypeToTl(cType: string): LangType {
    const t = cType.replace(/\bconst\b/g, '').trim();
    if (!t || t === 'void') return 'None';
    if (/\bchar\b/.test(t) && /[*\[]/.test(t)) return 'str';
    if (/\b(?:double|float)\b/.test(t)) return 'float';
    if (/\bbool\b/.test(t)) return 'bool';
    if (/[*\[]/.test(t)) return 'int'; // pointer/array → opaque handle
    return 'int'; // int, long, DWORD, HWND, size_t, etc.
}

/** Convert a single C parameter declaration to a tl-style `name: type` string. */
function parseCParam(param: string, idx: number): string {
    const isPointer = param.includes('*');
    const clean = param.replace(/\bconst\b/g, '').replace(/\*/g, '').replace(/\s+/g, ' ').trim();
    const parts = clean.split(/\s+/);
    const rawName = parts.length > 1 ? parts[parts.length - 1] : '';
    const finalName = /^[A-Za-z_]\w*$/.test(rawName) ? rawName : `p${idx}`;
    const baseType = (parts.length > 1 ? parts.slice(0, -1).join(' ') : parts[0]).trim();
    const tlType: LangType = isPointer
        ? (/\bchar\b/.test(baseType) ? 'str' : 'int')
        : cTypeToTl(baseType);
    return `${finalName}: ${tlType}`;
}

interface CppClassInfo {
    fields: Map<string, LangType>;  // fieldName → tl type
    fieldSigs: string[];            // ["x: int", "y: int"] for display
}

interface NativeModuleInfo {
    funcs: Map<string, LangType>;
    sigs: Map<string, string>;
    classes: Map<string, CppClassInfo>;  // C++ class/struct types parsed from header
}

/** Parse a flat C header file (extern "C" { ... }) and extract function signatures. */
function parseCHeader(content: string, dir: string = '', _depth: number = 0): NativeModuleInfo {
    const funcs = new Map<string, LangType>();
    const sigs = new Map<string, string>();
    // Strip line comments and block comments
    const src = content.replace(/\/\/[^\n]*/g, '').replace(/\/\*[\s\S]*?\*\//g, '');
    // Match: RetType FuncName ( params ) ;
    const re = /\b([A-Za-z_][\w\s]*?(?:\s*\*)?)\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*;/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(src)) !== null) {
        const retCType = m[1].trim();
        const name = m[2].trim();
        const paramsRaw = m[3].trim();
        if (/^(?:typedef|struct|class|union|enum)\b/.test(retCType)) continue;
        const retType = cTypeToTl(retCType);
        funcs.set(name, retType);
        const tlParams = !paramsRaw || paramsRaw === 'void' ? '' :
            paramsRaw.split(',').map((p, i) => parseCParam(p.trim(), i)).join(', ');
        sigs.set(name, `fn ${name}(${tlParams}) -> ${retType}`);
    }
    const classes = parseCppClasses(src);
    // Follow #include "..." directives up to 2 levels deep
    if (dir && _depth < 2) {
        const includeRe = /^#include\s+"([^"]+)"/gm;
        let inc: RegExpExecArray | null;
        while ((inc = includeRe.exec(content)) !== null) {
            const incPath = path.join(dir, inc[1]);
            if (fs.existsSync(incPath)) {
                try {
                    const sub = parseCHeader(fs.readFileSync(incPath, 'utf8'), path.dirname(incPath), _depth + 1);
                    for (const [k, v] of sub.funcs) if (!funcs.has(k)) funcs.set(k, v);
                    for (const [k, v] of sub.sigs)  if (!sigs.has(k))  sigs.set(k, v);
                    for (const [k, v] of sub.classes) if (!classes.has(k)) classes.set(k, v);
                } catch { /* ignore unreadable sub-headers */ }
            }
        }
    }
    return { funcs, sigs, classes };
}

/**
 * Parse C++ class/struct definitions from a pre-stripped header source and
 * extract public field names with their tl-mapped types.
 *
 * Handles:
 *   class  Foo { public: int x; };          (class – private by default)
 *   struct Bar { float x, y; };             (struct – public by default)
 *   typedef struct { int r; } Color;        (typedef struct)
 */
function parseCppClasses(src: string): Map<string, CppClassInfo> {
    const classes = new Map<string, CppClassInfo>();

    interface ClassCtx {
        name: string;           // resolved later for typedef struct
        openDepth: number;      // brace depth when the body opened
        isPublic: boolean;      // current access section
        fields: Map<string, LangType>;
        fieldSigs: string[];
        isTyepdef: boolean;     // true when opened via typedef struct { ...
    }

    const classStack: ClassCtx[] = [];
    let depth = 0;
    // When we see `class Foo` or `struct Foo` without a `{` yet, keep the name here
    let pendingName: string | undefined;
    let pendingIsStruct = false;

    for (const rawLine of src.split('\n')) {
        const line = rawLine.trim();
        const prevDepth = depth;

        // Count brace depth changes on this line
        for (const ch of rawLine) {
            if (ch === '{') depth++;
            else if (ch === '}') depth--;
        }

        // If a class name was pending and we just entered a new brace level, activate it
        if (pendingName !== undefined && depth > prevDepth) {
            classStack.push({
                name: pendingName, openDepth: depth,
                isPublic: pendingIsStruct, fields: new Map(), fieldSigs: [], isTyepdef: false,
            });
            pendingName = undefined;
        }

        // Pop classes whose body has closed
        while (classStack.length > 0 && depth < classStack[classStack.length - 1].openDepth) {
            const cls = classStack.pop()!;
            // For typedef struct, look for trailing name on `} Name;` line
            if (cls.isTyepdef) {
                const tdName = line.match(/\}\s*([A-Za-z_]\w*)\s*;/)?.[1];
                if (tdName) cls.name = tdName;
            }
            if (cls.name && cls.fields.size > 0) {
                classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs });
            }
        }

        if (!line || line.startsWith('#')) continue;

        // typedef struct { ...
        const typedefM = line.match(/^typedef\s+(?:class|struct)\s*(?:[A-Za-z_]\w*)?\s*\{/);
        if (typedefM && depth > prevDepth) {
            classStack.push({
                name: '', openDepth: depth, isPublic: true,
                fields: new Map(), fieldSigs: [], isTyepdef: true,
            });
            continue;
        }

        // class/struct Foo [: base] { or  class/struct Foo [: base]
        const classRe = /\b(class|struct)\s+([A-Za-z_]\w*)\s*(?::[^{]*)?\{?/;
        const cm = line.match(classRe);
        if (cm && !/^(extern|typedef)/.test(line)) {
            const isStruct = cm[1] === 'struct';
            const name = cm[2];
            if (line.includes('{') && depth > prevDepth) {
                classStack.push({
                    name, openDepth: depth, isPublic: isStruct,
                    fields: new Map(), fieldSigs: [], isTyepdef: false,
                });
            } else if (!line.includes('{')) {
                pendingName = name;
                pendingIsStruct = isStruct;
            }
            continue;
        }

        // Process fields when at the direct body depth of the top class on the stack
        if (classStack.length === 0) continue;
        const ctx = classStack[classStack.length - 1];
        if (depth !== ctx.openDepth) continue;   // inside a nested block

        // Access modifier
        const accessM = line.match(/^(public|private|protected)\s*:/);
        if (accessM) { ctx.isPublic = accessM[1] === 'public'; continue; }
        if (!ctx.isPublic) continue;

        // Skip methods, constructors, destructors, using, typedef, etc.
        if (line.includes('(')) continue;
        if (/^(typedef|using|static|virtual|explicit|inline|friend|extern|template)/.test(line)) continue;
        if (line.startsWith('~') || line.startsWith(ctx.name + ' ') || line === ctx.name) continue;

        // Field declaration: [const] [unsigned] [long] TYPE [*] name1[, name2] [= val] ;
        const fieldRe = /^(?:(?:const|unsigned|long|short|signed)\s+)*([A-Za-z_]\w*(?:\s*\*+)?)\s+([A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)*)\s*(?:=\s*[^,;]*)?\s*;/;
        const fm = line.match(fieldRe);
        if (!fm) continue;
        const rawCType = fm[1].trim();
        const tlType = cTypeToTl(rawCType);
        for (const nameRaw of fm[2].split(',')) {
            const n = nameRaw.trim();
            if (/^[A-Za-z_]\w*$/.test(n)) {
                ctx.fields.set(n, tlType);
                ctx.fieldSigs.push(`${n}: ${tlType}`);
            }
        }
    }

    // Flush any unclosed classes (malformed header)
    for (const cls of classStack) {
        if (cls.name && cls.fields.size > 0) {
            classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs });
        }
    }

    return classes;
}

/** Parse a .tls stub file and extract function signatures. */
function parseTlStub(content: string): NativeModuleInfo {
    const funcs = new Map<string, LangType>();
    const sigs = new Map<string, string>();
    for (const line of content.split('\n')) {
        const m = line.match(FUNC_DEF_RE);
        if (!m) continue;
        const [, , kw, name, params, retType] = m;
        const ret = retType?.trim() ?? 'unknown';
        funcs.set(name, ret);
        sigs.set(name, `${kw} ${name}(${params.trim()}) -> ${ret}`);
    }
    return { funcs, sigs, classes: new Map() };
}

/**
 * Load function signatures for one import statement.
 * - cpp-lib / cpp-dll: reads the `with stubname.h` header file; if no stub
 *   name is given, falls back to `<module/path>.h` next to the module source.
 * - tl / tlc / auto:   reads the adjacent `.tls` stub file
 */
function loadNativeModuleInfo(
    importKind: string,
    modulePath: string,
    stubName: string | undefined,
    docDir: string
): NativeModuleInfo {
    const empty: NativeModuleInfo = { funcs: new Map(), sigs: new Map(), classes: new Map() };
    if (importKindOf(importKind) === 'cpp') {
        // Prefer explicit stub name; fall back to auto-detecting <module-path>.h
        const candidates: string[] = [];
        if (stubName) {
            candidates.push(path.join(docDir, stubName + '.h'));
        }
        // Auto-detect: examples/test_modules/point.h for "test_modules.point"
        const parts = modulePath.split('.');
        candidates.push(path.join(docDir, ...parts) + '.h');
        // Also try just the last component in docDir
        candidates.push(path.join(docDir, parts[parts.length - 1] + '.h'));
        for (const hPath of candidates) {
            if (fs.existsSync(hPath)) {
                try { return parseCHeader(fs.readFileSync(hPath, 'utf8'), path.dirname(hPath)); } catch { /* ignore */ }
            }
        }
        return empty;
    }
    // tl / tlc / auto: look for .tls stub file adjacent to the module
    const filePath = path.join(docDir, ...modulePath.split('.'));
    const tlsPath = filePath + '.tls';
    if (fs.existsSync(tlsPath)) {
        try { return parseTlStub(fs.readFileSync(tlsPath, 'utf8')); } catch { /* ignore */ }
    }
    return empty;
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

// Resolve 'Self' to the enclosing class name, if known
function resolveSelf(type: LangType, enclosingClass: string | undefined): LangType {
    return type === 'Self' && enclosingClass ? enclosingClass : type;
}

// Extract per-element types from tuple[T1, T2, ...]; falls back to 'unknown' per slot
function extractTupleElemTypes(tupleType: LangType, count: number): LangType[] {
    const m = tupleType.match(/^tuple\[(.+)\]$/);
    if (!m) return Array(count).fill('unknown');
    const elems = splitComma(m[1]).map(e => e.trim());
    return elems;
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
    enclosingClass?: string;
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

// Maps function name → first template parameter name, e.g. "min_of" → "T"
function collectTemplateParams(document: vscode.TextDocument): Map<string, string> {
    const map = new Map<string, string>();
    const re = /^\s*(?:fn|gen)\s+([A-Za-z_]\w*)\[([A-Za-z_]\w*)/;
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(re);
        if (m) map.set(m[1], m[2]);
    }
    return map;
}

function inferBodyReturnType(
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

// ===== Import alias collection =====

function collectImportAliases(document: vscode.TextDocument): Map<string, string> {
    const aliases = new Map<string, string>(); // alias → module path
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(IMPORT_RE);
        if (m) aliases.set(m[4], m[2]); // group 4 = alias, group 2 = module path
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
    funcSigs: Map<string, Map<string, string>>;
    classMethods: Map<string, Map<string, string>>;
    cppClasses: Map<string, CppClassInfo>;
} {
    const funcTypes = new Map<string, Map<string, string>>();
    const funcSigs = new Map<string, Map<string, string>>();
    const classMethods = new Map<string, Map<string, string>>();
    const cppClasses = new Map<string, CppClassInfo>();
    const docDir = path.dirname(document.uri.fsPath);
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(IMPORT_RE);
        if (!m) continue;
        const [, importKind, modulePath, stubName, alias] = m;
        if (importKindOf(importKind) === 'py') {
            const info = collectPyModuleInfo(modulePath, docDir);
            funcTypes.set(alias, info.funcs);
            funcSigs.set(alias, info.funcs); // py stubs have no full param signatures
            for (const [cls, methods] of info.classes) classMethods.set(cls, methods);
        } else {
            const info = loadNativeModuleInfo(importKind, modulePath, stubName, docDir);
            if (info.funcs.size > 0) {
                funcTypes.set(alias, info.funcs);
                funcSigs.set(alias, info.sigs);
            }
            // Collect C++ classes from all cpp imports (regardless of function count)
            if (importKindOf(importKind) === 'cpp') {
                for (const [className, classInfo] of info.classes) {
                    cppClasses.set(className, classInfo);
                }
            }
        }
    }
    return { funcTypes, funcSigs, classMethods, cppClasses };
}

// ===== Hover symbol collection =====

function collectHoverSymbols(document: vscode.TextDocument): HoverSymbol[] {
    const symbols: HoverSymbol[] = [];
    const env = new Map<string, LangType>();

    // Collect import aliases first so type inference can use them throughout
    const importAliasMap = collectImportAliases(document);
    const importAliases = new Set(importAliasMap.keys());
    const { funcTypes: importFuncTypes, classMethods: pyClassMethods, cppClasses } = collectAllPyModuleInfo(document);

    const funcDefs = collectFuncDefs(document);
    const funcEnv = collectConstructorTypes(document);
    const templateParams = collectTemplateParams(document);

    // Register C++ class names as constructors so `POINT(x,y)` resolves to type `POINT`
    for (const [className] of cppClasses) {
        funcEnv.set(className, className);
    }

    for (const def of funcDefs) {
        const rawType = def.annotation ?? inferBodyReturnType(document, def.defLine, def.defIndent, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams);
        funcEnv.set(def.name, resolveSelf(rawType, def.enclosingClass));
    }

    // Stack tracking class/trait bodies for access modifier inheritance and Self resolution
    const classContextStack: Array<{ name: string; indent: number; bodyIndent: number; access: 'public' | 'private' | 'protected' }> = [];

    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = stripComment(raw);
        const trimmedLine = stripped.trim();

        const lineIndentLen = (stripped.match(/^(\s*)/)?.[1] ?? '').length;

        // Maintain class context stack on every non-empty line
        if (trimmedLine) {
            while (classContextStack.length > 0 && lineIndentLen <= classContextStack[classContextStack.length - 1].indent) {
                classContextStack.pop();
            }
            if (classContextStack.length > 0) {
                const top = classContextStack[classContextStack.length - 1];
                if (top.bodyIndent === -1) top.bodyIndent = lineIndentLen;
            }
        }

        // Access section marker (public: / private: / protected:) inside a class body
        const accessM = trimmedLine ? stripped.match(ACCESS_SECTION_RE) : null;
        if (accessM && classContextStack.length > 0) {
            const top = classContextStack[classContextStack.length - 1];
            if (top.bodyIndent !== -1 && lineIndentLen === top.bodyIndent) {
                top.access = accessM[2] as 'public' | 'private' | 'protected';
            }
            continue;
        }

        // Determine access for direct class/trait members at the body indent level
        const currentAccess: 'public' | 'private' | 'protected' | undefined = (() => {
            if (classContextStack.length === 0) return undefined;
            const top = classContextStack[classContextStack.length - 1];
            if (top.bodyIndent === -1 || lineIndentLen !== top.bodyIndent) return undefined;
            return top.access;
        })();

        // Import declaration (all variants: py, cpp-lib, cpp-dll, tl, tlc, auto)
        const importMatch = stripped.match(IMPORT_RE);
        if (importMatch) {
            const [, importKind, modulePath, , alias] = importMatch; // skip group 3 (stub name)
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

        // Tuple destructuring: let/mut a, b = expr
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
        const accessPrefix = symbol.access ? `${symbol.access} ` : '';
        md.appendCodeblock(`${accessPrefix}${mutability} ${symbol.name}: ${symbol.type ?? 'unknown'}`, 'tl');
        if (opts?.narrowedFrom) {
            md.appendMarkdown(`\n\n*narrowed from* \`${opts.narrowedFrom}\``);
        }
    } else if (symbol.kind === 'function') {
        const baseSig = symbol.signature ?? `fn ${symbol.name}() -> ${symbol.type ?? 'unknown'}`;
        md.appendCodeblock(symbol.access ? `${symbol.access} ${baseSig}` : baseSig, 'tl');
    } else if (symbol.kind === 'class') {
        md.appendCodeblock(`class ${symbol.name}`, 'tl');
    } else if (symbol.kind === 'enum') {
        md.appendCodeblock(`enum ${symbol.name}`, 'tl');
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

    // Detect member access: `obj.memberName` — check the character before the word
    const lineText = document.lineAt(position.line).text;
    const prefixStr = lineText.substring(0, range.start.character);
    const dotAccess = prefixStr.match(/([A-Za-z_]\w*)\.$/);
    if (dotAccess) {
        const objName = dotAccess[1];
        const { funcTypes, funcSigs, cppClasses } = collectAllPyModuleInfo(document);

        // Module function: `alias.funcName`
        const retType = funcTypes.get(objName)?.get(name);
        if (retType !== undefined) {
            const md = new vscode.MarkdownString(undefined, true);
            const sig = funcSigs.get(objName)?.get(name) ?? `fn ${name}() -> ${retType}`;
            md.appendCodeblock(sig, 'tl');
            return new vscode.Hover(md, range);
        }

        // C++ class field: `instance.fieldName` where instance has a cpp class type
        const objSymbols = collectHoverSymbols(document);
        const objSym = selectHoverSymbol(objSymbols, objName, position.line);
        if (objSym?.type) {
            const cppCls = cppClasses.get(objSym.type);
            if (cppCls) {
                const fieldType = cppCls.fields.get(name);
                if (fieldType !== undefined) {
                    const md = new vscode.MarkdownString(undefined, true);
                    md.appendCodeblock(`${name}: ${fieldType}`, 'tl');
                    md.appendMarkdown(`\n\n*field of* \`${objSym.type}\``);
                    return new vscode.Hover(md, range);
                }
            }
        }
    }

    // C++ class type hover: hovering over the class name itself (e.g. `POINT`)
    {
        const { cppClasses } = collectAllPyModuleInfo(document);
        const cppCls = cppClasses.get(name);
        if (cppCls) {
            const md = new vscode.MarkdownString(undefined, true);
            const body = cppCls.fieldSigs.length > 0
                ? cppCls.fieldSigs.map(s => `    ${s}`).join('\n')
                : '    (no public fields)';
            md.appendCodeblock(`class ${name} {\n${body}\n}`, 'cpp');
            return new vscode.Hover(md, range);
        }
    }

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
    const { funcTypes: importFuncTypes, classMethods: pyClassMethods, cppClasses } = collectAllPyModuleInfo(document);
    const funcDefs = collectFuncDefs(document);

    // Phase 2: build funcEnv
    const funcEnv = collectConstructorTypes(document);
    const templateParams = collectTemplateParams(document);
    // Register C++ class names as constructors so `POINT(x,y)` resolves to type `POINT`
    for (const [className] of cppClasses) {
        funcEnv.set(className, className);
    }
    for (const def of funcDefs) {
        const rawType = def.annotation ?? inferBodyReturnType(document, def.defLine, def.defIndent, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams);
        funcEnv.set(def.name, resolveSelf(rawType, def.enclosingClass));
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
                ?? (rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams) : 'unknown');
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

        // Tuple destructuring: let/mut a, b = expr
        const tupleM = line.match(TUPLE_DECL_RE);
        if (tupleM) {
            const [, indent, keyword, names, rhs] = tupleM;
            const rhsType = inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams);
            const nameList = names.split(',').map(n => n.trim()).filter(Boolean);
            const elemTypes = extractTupleElemTypes(rhsType, nameList.length);
            let searchFrom = indent.length + keyword.length;
            for (let idx = 0; idx < nameList.length; idx++) {
                const varName = nameList[idx];
                const elemType = elemTypes[idx] ?? 'unknown';
                env.set(varName, elemType);
                const nameStart = rawLine.indexOf(varName, searchFrom);
                if (nameStart >= 0) {
                    const pos = new vscode.Position(lineIdx, nameStart + varName.length);
                    const hint = new vscode.InlayHint(pos, `: ${elemType}`, vscode.InlayHintKind.Type);
                    hint.paddingLeft = true;
                    hints.push(hint);
                    searchFrom = nameStart + varName.length;
                }
            }
            continue;
        }

        // let/mut/const declarations — use HOVER_DECL_RE so annotated declarations
        // also update env (fixes type inference for variables used after a typed cast/annotation)
        const declM = line.match(HOVER_DECL_RE);
        if (!declM) continue;

        const [, indent, keyword, name, annotation, rhs] = declM;
        const type = cleanTypeAnnotation(annotation)
            ?? (rhs ? inferExprType(rhs.trim(), env, funcEnv, importAliases, importFuncTypes, pyClassMethods, templateParams) : 'unknown');
        env.set(name, type);

        // Emit hint only when there is no visible annotation
        if (!annotation && rhs) {
            const nameStart = rawLine.indexOf(name, (indent ?? '').length + keyword.length);
            if (nameStart >= 0) {
                const pos = new vscode.Position(lineIdx, nameStart + name.length);
                const hint = new vscode.InlayHint(pos, `: ${type}`, vscode.InlayHintKind.Type);
                hint.paddingLeft = true;
                hints.push(hint);
            }
        }
    }

    return hints;
}

// ===== Language keywords =====

const LANG_KEYWORDS = [
    'let', 'mut', 'const', 'fn', 'gen', 'class', 'trait', 'new_type',
    'return', 'if', 'elif', 'else', 'for', 'while', 'match', 'case',
    'and', 'or', 'not', 'in', 'is', 'True', 'False', 'None',
    'self', 'Self', 'static', 'freeze', 'block',
    'block_return', 'loop_yield', 'break', 'continue', 'yield', 'pass',
    'import', 'enumerate', 'zip',
    'public', 'private', 'protected',
    'uint',
];

// ===== Member completion helpers =====

function findEnclosingClass(document: vscode.TextDocument, fromLine: number): string | undefined {
    const fromIndent = (document.lineAt(fromLine).text.match(/^(\s*)/)?.[1] ?? '').length;
    for (let i = fromLine - 1; i >= 0; i--) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(CLASS_DEF_RE);
        if (!m) continue;
        const classIndent = (m[1] ?? '').length;
        if (classIndent < fromIndent) return m[3];
    }
    return undefined;
}

function collectClassMemberItems(
    document: vscode.TextDocument,
    className: string,
    _visited: Set<string> = new Set()
): vscode.CompletionItem[] {
    if (_visited.has(className)) return [];
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
    if (classLine < 0) return [];

    // Determine the indentation of direct class body members
    let memberIndent = -1;
    for (let i = classLine + 1; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        if (!raw.trim()) continue;
        const ind = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (ind <= classIndent) break;
        memberIndent = ind;
        break;
    }
    if (memberIndent < 0) return [];

    const items: vscode.CompletionItem[] = [];
    const seen = new Set<string>();

    for (let i = classLine + 1; i < document.lineCount; i++) {
        const rawLine = document.lineAt(i).text;
        const stripped = stripComment(rawLine);
        if (!stripped.trim()) continue;

        const lineIndent = (rawLine.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= classIndent) break;

        // Direct class body members (fields and methods)
        if (lineIndent === memberIndent) {
            // Field declarations: `let`/`mut`/`const` name [: type] [= value]
            const fieldMatch = stripped.match(HOVER_DECL_RE);
            if (fieldMatch) {
                const [, , , name, type] = fieldMatch;
                if (!seen.has(name)) {
                    seen.add(name);
                    const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Field);
                    if (type) item.detail = `: ${type.trim()}`;
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

function resolveMemberItems(
    document: vscode.TextDocument,
    position: vscode.Position,
    objName: string
): vscode.CompletionItem[] {
    // 'self' → members of the enclosing class
    if (objName === 'self') {
        const cls = findEnclosingClass(document, position.line);
        return cls ? collectClassMemberItems(document, cls) : [];
    }

    const symbols = collectHoverSymbols(document);
    const sym = selectHoverSymbol(symbols, objName, position.line);
    if (!sym) return [];

    // Import module (py, cpp-lib, cpp-dll, tl, tlc, auto) → show exported functions
    if (sym.kind === 'module') {
        const { funcTypes, funcSigs } = collectAllPyModuleInfo(document);
        const funcs = funcTypes.get(objName);
        const sigsMap = funcSigs.get(objName);
        if (!funcs) return [];
        return [...funcs.entries()].map(([name, retType]) => {
            const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
            const sig = sigsMap?.get(name);
            // Show full signature when available (cpp/tl stubs), else just return type
            item.detail = sig ?? `→ ${retType}`;
            return item;
        });
    }

    // Variable/parameter with a class type → check C++ header classes first, then .tl classes
    if (sym.type) {
        const { cppClasses } = collectAllPyModuleInfo(document);
        const cppCls = cppClasses.get(sym.type);
        if (cppCls) {
            return [...cppCls.fields.entries()].map(([fieldName, fieldType]) => {
                const item = new vscode.CompletionItem(fieldName, vscode.CompletionItemKind.Field);
                item.detail = `: ${fieldType}`;
                return item;
            });
        }
        return collectClassMemberItems(document, sym.type);
    }

    return [];
}

// ===== Completion items provider =====

export function provideCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position
): vscode.CompletionItem[] {
    const prefix = document.lineAt(position.line).text.substring(0, position.character);

    // Member access context: ends with `identifier.` or `identifier.partial`
    const dotMatch = prefix.match(/([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/);
    if (dotMatch) {
        return resolveMemberItems(document, position, dotMatch[1]);
    }

    // Normal (non-dot) completion
    const items: vscode.CompletionItem[] = [];
    const seen = new Set<string>();

    // Document symbols first so user-defined names take priority over built-ins
    const symbols = collectHoverSymbols(document);
    for (const sym of symbols) {
        if (sym.kind === 'variable' && sym.line > position.line) continue;
        if (seen.has(sym.name)) continue;
        seen.add(sym.name);

        let kind: vscode.CompletionItemKind;
        switch (sym.kind) {
            case 'function': kind = vscode.CompletionItemKind.Function; break;
            case 'class':    kind = vscode.CompletionItemKind.Class; break;
            case 'trait':    kind = vscode.CompletionItemKind.Interface; break;
            case 'new_type': kind = vscode.CompletionItemKind.TypeParameter; break;
            case 'module':   kind = vscode.CompletionItemKind.Module; break;
            default:         kind = vscode.CompletionItemKind.Variable;
        }

        const item = new vscode.CompletionItem(sym.name, kind);
        if (sym.signature) item.detail = sym.signature;
        else if (sym.type) item.detail = `: ${sym.type}`;
        if (sym.doc) item.documentation = new vscode.MarkdownString(sym.doc);
        items.push(item);
    }

    // Built-in functions
    for (const [name, retType] of Object.entries(BUILTIN_RETURN_TYPES)) {
        if (seen.has(name)) continue;
        seen.add(name);
        const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
        item.detail = `→ ${retType}`;
        items.push(item);
    }

    // Language keywords
    for (const kw of LANG_KEYWORDS) {
        if (seen.has(kw)) continue;
        seen.add(kw);
        items.push(new vscode.CompletionItem(kw, vscode.CompletionItemKind.Keyword));
    }

    return items;
}

// ===== Document symbols provider =====

export function provideDocumentSymbols(
    document: vscode.TextDocument
): vscode.DocumentSymbol[] {
    const result: vscode.DocumentSymbol[] = [];
    const stack: Array<{ sym: vscode.DocumentSymbol; indent: number }> = [];

    for (let i = 0; i < document.lineCount; i++) {
        const rawLine = document.lineAt(i).text;
        const stripped = stripComment(rawLine);
        if (!stripped.trim()) continue;

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
        } else {
            const classMatch = stripped.match(CLASS_DEF_RE);
            if (classMatch) {
                const [, , kw, className, bases] = classMatch;
                name = className;
                kind = kw === 'trait' ? vscode.SymbolKind.Interface : vscode.SymbolKind.Class;
                detail = bases ? `(${bases})` : '';
                isContainer = true;
            } else {
                const newTypeMatch = stripped.match(NEW_TYPE_RE);
                if (newTypeMatch) {
                    const [, , ntName, ntType] = newTypeMatch;
                    name = ntName;
                    kind = vscode.SymbolKind.TypeParameter;
                    detail = ntType.trim();
                } else {
                    const importMatch = stripped.match(IMPORT_RE);
                    if (importMatch) {
                        const [, importKind, modulePath, , alias] = importMatch; // skip stub (group 3)
                        name = alias;
                        kind = vscode.SymbolKind.Module;
                        detail = `${importKind} ${modulePath}`;
                    } else {
                        const staticMatch = stripped.match(STATIC_DECL_RE);
                        if (staticMatch) {
                            const [, , varName, annot] = staticMatch;
                            name = varName;
                            kind = vscode.SymbolKind.Variable;
                            detail = annot?.trim() ?? '';
                        } else if (indent === 0) {
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

        if (!name) continue;

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
        } else {
            stack[stack.length - 1].sym.children.push(sym);
        }

        if (isContainer) {
            stack.push({ sym, indent });
        }
    }

    return result;
}

// ===== Signature help provider =====

export function provideSignatureHelp(
    document: vscode.TextDocument,
    position: vscode.Position
): vscode.SignatureHelp | undefined {
    const prefix = document.lineAt(position.line).text.substring(0, position.character);
    let depth = 0;
    let activeParam = 0;
    let funcName = '';

    for (let i = prefix.length - 1; i >= 0; i--) {
        const c = prefix[i];
        if (c === ')' || c === ']' || c === '}') { depth++; continue; }
        if (c === '(' || c === '[' || c === '{') {
            if (c === '(' && depth === 0) {
                const m = prefix.substring(0, i).trimEnd().match(/([A-Za-z_]\w*)$/);
                funcName = m?.[1] ?? '';
                break;
            }
            if (depth > 0) depth--;
            continue;
        }
        if (c === ',' && depth === 0) activeParam++;
    }

    if (!funcName) return undefined;

    const symbols = collectHoverSymbols(document);
    const funcSym = symbols.find(s => s.name === funcName && s.kind === 'function');
    if (!funcSym?.signature) return undefined;

    const sigInfo = new vscode.SignatureInformation(
        funcSym.signature,
        funcSym.doc ? new vscode.MarkdownString(funcSym.doc) : undefined
    );

    const paramsMatch = funcSym.signature.match(/\(([^)]*)\)/);
    if (paramsMatch?.[1]?.trim()) {
        for (const p of splitComma(paramsMatch[1])) {
            const trimmed = p.trim();
            if (trimmed) sigInfo.parameters.push(new vscode.ParameterInformation(trimmed));
        }
    }

    const help = new vscode.SignatureHelp();
    help.signatures = [sigInfo];
    help.activeSignature = 0;
    help.activeParameter = Math.min(activeParam, Math.max(0, sigInfo.parameters.length - 1));
    return help;
}

// ===== Definition provider =====

export function provideDefinition(
    document: vscode.TextDocument,
    position: vscode.Position
): vscode.Location | undefined {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_]\w*/);
    if (!range) return undefined;

    const name = document.getText(range);
    const symbols = collectHoverSymbols(document);
    const symbol = selectHoverSymbol(symbols, name, position.line);
    if (!symbol) return undefined;

    const targetText = document.lineAt(symbol.line).text;
    const nameIdx = targetText.indexOf(symbol.name);
    const targetRange = nameIdx >= 0
        ? new vscode.Range(symbol.line, nameIdx, symbol.line, nameIdx + symbol.name.length)
        : document.lineAt(symbol.line).range;

    return new vscode.Location(document.uri, targetRange);
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
