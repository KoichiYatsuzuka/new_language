"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.loadNativeModuleInfo = exports.resolveImportSourceFile = exports.parseRustLib = exports.detectPythonLibDirs = exports.getPythonSearchPaths = exports.parseTlStub = exports.parseCppClasses = exports.parseCHeader = exports.parseCParam = exports.cTypeToTl = exports.importKindOf = void 0;
const fs_1 = require("fs");
const path = require("path");
const child_process_1 = require("child_process");
const builtins_1 = require("./builtins");
const cs_assembly_1 = require("./cs_assembly");
// ===== C++ / native module support =====
function importKindOf(keyword) {
    if (keyword.includes('cpp'))
        return 'cpp';
    if (keyword === 'import[rs]')
        return 'rs';
    if (keyword === 'import[cs-dll]' || keyword === 'import[cs-proc]')
        return 'cs';
    if (keyword === 'import[js-proc]')
        return 'js';
    if (keyword === 'import' || keyword.startsWith('import[hv') ||
        keyword === 'import[ar]' || keyword === 'import[arc]')
        return 'ar';
    return 'py';
}
exports.importKindOf = importKindOf;
function cTypeToTl(cType) {
    const t = cType.replace(/\bconst\b/g, '').trim();
    if (!t || t === 'void')
        return 'None';
    if (/\bchar\b/.test(t) && /[*\[]/.test(t))
        return 'str';
    if (/\b(?:double|float)\b/.test(t))
        return 'float';
    if (/\bbool\b/.test(t))
        return 'bool';
    if (/[*\[]/.test(t))
        return 'int'; // pointer/array → opaque handle
    return 'int'; // int, long, DWORD, HWND, size_t, etc.
}
exports.cTypeToTl = cTypeToTl;
// Pointee type for a non-char pointer param, or null if the pointee is not a
// C primitive (i.e. a struct pointer). Mirrors the compiler's type-check stub
// (src/parser/imports/mod.rs ctype_to_tl_str): `double*` → float, `void*` → int
// (opaque handle), struct pointers → Any — nominal typing would break
// structurally-compatible shadow classes (c-abi-interop SKILL.md P3).
function pointeePrimType(baseType) {
    if (/\bvoid\b/.test(baseType))
        return 'int'; // void* → opaque handle
    if (/\b(?:double|float)\b/.test(baseType))
        return 'float';
    if (/\bbool\b/.test(baseType))
        return 'bool';
    if (/\b(?:short|int|long|unsigned|signed|size_t|u?int(?:8|16|32|64)_t|DWORD|WORD|BYTE)\b/.test(baseType))
        return 'int';
    return null; // struct pointee
}
function parseCParam(param, idx) {
    var _a;
    const isPointer = /[*&]/.test(param);
    const isConst = /\bconst\b/.test(param);
    // A non-const pointer/reference param can be written back through, so it maps to
    // a mutable Arrow param; a by-value or const pointer/reference is read-only → `let`.
    // (`int* y` → `mut y: int`, `const int* y` / `int x` → `let ...`.)
    // Same rule as the compiler stub (src/parser/imports/cpp.rs → Param::bridge).
    const mutability = isPointer && !isConst ? 'mut' : 'let';
    const clean = param.replace(/\bconst\b/g, '').replace(/[*&]/g, '').replace(/\s+/g, ' ').trim();
    const parts = clean.split(/\s+/);
    const rawName = parts.length > 1 ? parts[parts.length - 1] : '';
    const finalName = /^[A-Za-z_]\w*$/.test(rawName) ? rawName : `p${idx}`;
    const baseType = (parts.length > 1 ? parts.slice(0, -1).join(' ') : parts[0]).trim();
    const tlType = isPointer
        ? (/\bchar\b/.test(baseType) ? 'str' : ((_a = pointeePrimType(baseType)) !== null && _a !== void 0 ? _a : 'Any'))
        : cTypeToTl(baseType);
    return `${mutability} ${finalName}: ${tlType}`;
}
exports.parseCParam = parseCParam;
async function parseCHeader(content, dir = '', _depth = 0) {
    const funcs = new Map();
    const sigs = new Map();
    const docs = new Map();
    // Extract /// and /** */ doc-comments that immediately precede function declarations,
    // before stripping all comments from the source.
    const cHeaderLines = content.split('\n');
    let pendingDocLines = [];
    for (const raw of cHeaderLines) {
        const t = raw.trim();
        if (t.startsWith('///')) {
            // Strip leading /// and optional single space, also handle @brief/@param/@return
            const stripped = t.replace(/^\/\/\/\s?/, '').replace(/^@brief\s*/, '');
            pendingDocLines.push(stripped);
        }
        else if (pendingDocLines.length > 0) {
            // Next non-comment line: check if it looks like a function declaration
            const fnNameM = raw.match(/\b([A-Za-z_]\w*)\s*\(/);
            if (fnNameM && !raw.trimStart().startsWith('typedef') && !raw.trimStart().startsWith('//')) {
                const fnName = fnNameM[1];
                const docText = pendingDocLines.join('\n').trim();
                if (docText)
                    docs.set(fnName, docText);
            }
            pendingDocLines = [];
        }
        else {
            pendingDocLines = [];
        }
    }
    const src = content.replace(/\/\/[^\n]*/g, '').replace(/\/\*[\s\S]*?\*\//g, '');
    const re = /\b([A-Za-z_][\w\s]*?(?:\s*\*)?)\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*;/g;
    let m;
    while ((m = re.exec(src)) !== null) {
        const retCType = m[1].trim();
        const name = m[2].trim();
        const paramsRaw = m[3].trim();
        if (/^(?:typedef|struct|class|union|enum)\b/.test(retCType))
            continue;
        const retType = cTypeToTl(retCType);
        funcs.set(name, retType);
        const tlParams = !paramsRaw || paramsRaw === 'void' ? '' :
            paramsRaw.split(',').map((p, i) => parseCParam(p.trim(), i)).join(', ');
        sigs.set(name, `fn ${name}(${tlParams}) -> ${retType}`);
    }
    const classes = parseCppClasses(src);
    if (dir && _depth < 2) {
        const includeRe = /^#include\s+"([^"]+)"/gm;
        let inc;
        const subPromises = [];
        while ((inc = includeRe.exec(content)) !== null) {
            const incPath = path.join(dir, inc[1]);
            subPromises.push((async (p) => {
                try {
                    const subContent = await fs_1.promises.readFile(p, 'utf8');
                    const sub = await parseCHeader(subContent, path.dirname(p), _depth + 1);
                    for (const [k, v] of sub.funcs)
                        if (!funcs.has(k))
                            funcs.set(k, v);
                    for (const [k, v] of sub.sigs)
                        if (!sigs.has(k))
                            sigs.set(k, v);
                    for (const [k, v] of sub.classes)
                        if (!classes.has(k))
                            classes.set(k, v);
                }
                catch { /* ignore unreadable sub-headers */ }
            })(incPath));
        }
        await Promise.all(subPromises);
    }
    return { funcs, sigs, docs, classes };
}
exports.parseCHeader = parseCHeader;
function parseCppClasses(src) {
    var _a;
    const classes = new Map();
    const classStack = [];
    let depth = 0;
    let pendingName;
    let pendingIsStruct = false;
    for (const rawLine of src.split('\n')) {
        const line = rawLine.trim();
        const prevDepth = depth;
        for (const ch of rawLine) {
            if (ch === '{')
                depth++;
            else if (ch === '}')
                depth--;
        }
        if (pendingName !== undefined && depth > prevDepth) {
            classStack.push({
                name: pendingName, openDepth: depth,
                isPublic: pendingIsStruct, fields: new Map(), fieldSigs: [], isTyepdef: false,
            });
            pendingName = undefined;
        }
        while (classStack.length > 0 && depth < classStack[classStack.length - 1].openDepth) {
            const cls = classStack.pop();
            if (cls.isTyepdef) {
                const tdName = (_a = line.match(/\}\s*([A-Za-z_]\w*)\s*;/)) === null || _a === void 0 ? void 0 : _a[1];
                if (tdName)
                    cls.name = tdName;
            }
            if (cls.name && cls.fields.size > 0) {
                classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs, methods: new Map(), methodSigs: [], bases: [] });
            }
        }
        if (!line || line.startsWith('#'))
            continue;
        const typedefM = line.match(/^typedef\s+(?:class|struct)\s*(?:[A-Za-z_]\w*)?\s*\{/);
        if (typedefM && depth > prevDepth) {
            classStack.push({
                name: '', openDepth: depth, isPublic: true,
                fields: new Map(), fieldSigs: [], isTyepdef: true,
            });
            continue;
        }
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
            }
            else if (!line.includes('{')) {
                pendingName = name;
                pendingIsStruct = isStruct;
            }
            continue;
        }
        if (classStack.length === 0)
            continue;
        const ctx = classStack[classStack.length - 1];
        if (depth !== ctx.openDepth)
            continue;
        const accessM = line.match(/^(public|private|protected)\s*:/);
        if (accessM) {
            ctx.isPublic = accessM[1] === 'public';
            continue;
        }
        if (!ctx.isPublic)
            continue;
        if (line.includes('('))
            continue;
        if (/^(typedef|using|static|virtual|explicit|inline|friend|extern|template)/.test(line))
            continue;
        if (line.startsWith('~') || line.startsWith(ctx.name + ' ') || line === ctx.name)
            continue;
        const fieldRe = /^(?:(?:const|unsigned|long|short|signed)\s+)*([A-Za-z_]\w*(?:\s*\*+)?)\s+([A-Za-z_]\w*(?:\s*,\s*[A-Za-z_]\w*)*)\s*(?:=\s*[^,;]*)?\s*;/;
        const fm = line.match(fieldRe);
        if (!fm)
            continue;
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
    for (const cls of classStack) {
        if (cls.name && cls.fields.size > 0) {
            classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs, methods: new Map(), methodSigs: [], bases: [] });
        }
    }
    return classes;
}
exports.parseCppClasses = parseCppClasses;
const HVS_CLASS_RE = /^(\s*)class\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?(?:\(([^)]*)\))?(?:->[A-Za-z_]\w*)?\s*:/;
const HVS_FIELD_RE = /^(\s*)(?:let|mut|const)\s+([A-Za-z_]\w*)\s*:\s*(.+)/;
const HVS_METHOD_RE = /^(\s*)(fn|gen)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:/;
/** Extract a Python-style triple-quoted docstring starting at lines[startIdx].
 *  Handles both single-line `"""text"""` and multi-line forms. */
function extractDocstring(lines, startIdx) {
    for (let j = startIdx; j < lines.length; j++) {
        const next = lines[j].trim();
        if (!next)
            continue;
        for (const q of ['"""', "'''"]) {
            if (!next.startsWith(q))
                continue;
            const afterOpen = next.slice(q.length);
            const closeIdx = afterOpen.indexOf(q);
            if (closeIdx >= 0) {
                // Closes on the same line: """text"""
                return afterOpen.slice(0, closeIdx).trim() || undefined;
            }
            // Multi-line: collect until the closing triple-quote
            const docLines = [];
            if (afterOpen.trim())
                docLines.push(afterOpen.trim());
            for (let k = j + 1; k < lines.length; k++) {
                const ln = lines[k].trim();
                const ci = ln.indexOf(q);
                if (ci >= 0) {
                    const before = ln.slice(0, ci).trim();
                    if (before)
                        docLines.push(before);
                    break;
                }
                docLines.push(ln);
            }
            return docLines.join('\n').trim() || undefined;
        }
        break; // non-docstring line
    }
    return undefined;
}
function parseTlStub(content) {
    var _a, _b, _c, _d, _e;
    const funcs = new Map();
    const sigs = new Map();
    const docs = new Map();
    const classes = new Map();
    const lines = content.split('\n');
    let currentClass = null;
    let classIndent = 0;
    let bodyIndent = -1;
    for (let i = 0; i < lines.length; i++) {
        const raw = lines[i];
        const trimmed = raw.trim();
        if (!trimmed)
            continue;
        const lineIndent = ((_b = (_a = raw.match(/^(\s*)/)) === null || _a === void 0 ? void 0 : _a[1]) !== null && _b !== void 0 ? _b : '').length;
        // Exit class when we see a non-empty line at or before class indent
        if (currentClass !== null && lineIndent <= classIndent) {
            currentClass = null;
            bodyIndent = -1;
        }
        // Class definition: `class Name(Base1, Base2):` or `class Name->Name:`
        const classM = raw.match(HVS_CLASS_RE);
        if (classM && currentClass === null) {
            currentClass = classM[2];
            classIndent = ((_c = classM[1]) !== null && _c !== void 0 ? _c : '').length;
            bodyIndent = -1;
            const bases = classM[3]
                ? classM[3].split(',').map(b => b.trim()).filter(Boolean)
                : [];
            classes.set(currentClass, { fields: new Map(), fieldSigs: [], methods: new Map(), methodSigs: [], bases });
            // Look for class-level docstring on the next non-empty body line
            const cDoc = extractDocstring(lines, i + 1);
            if (cDoc)
                classes.get(currentClass).classDocs = cDoc;
            continue;
        }
        if (currentClass !== null) {
            if (bodyIndent === -1)
                bodyIndent = lineIndent;
            // Only process direct class members (skip nested)
            if (lineIndent !== bodyIndent)
                continue;
            if (/^(?:public|private|protected)\s*:/.test(trimmed) || trimmed === '...')
                continue;
            const cls = classes.get(currentClass);
            const fieldM = raw.match(HVS_FIELD_RE);
            if (fieldM) {
                const fname = fieldM[2];
                const ftype = fieldM[3].trim();
                cls.fields.set(fname, ftype);
                cls.fieldSigs.push(`${fname}: ${ftype}`);
                continue;
            }
            const methodM = raw.match(HVS_METHOD_RE);
            if (methodM) {
                const [, , kw, mname, mparams, mret] = methodM;
                if (mname === '__init__')
                    continue;
                const ret = (_d = mret === null || mret === void 0 ? void 0 : mret.trim()) !== null && _d !== void 0 ? _d : 'None';
                const cleanParams = mparams
                    .replace(/^\s*(?:let\s+|mut\s+)?self\s*,\s*/, '')
                    .replace(/^\s*(?:let\s+|mut\s+)?self\s*$/, '');
                const msig = `${kw} ${mname}(${cleanParams.trim()}) -> ${ret}`;
                // Look for docstring immediately after the method signature
                const mDoc = extractDocstring(lines, i + 1);
                cls.methods.set(mname, { ret, sig: msig, doc: mDoc });
                cls.methodSigs.push(msig);
                continue;
            }
        }
        else {
            // Top-level function
            const m = raw.match(builtins_1.FUNC_DEF_RE);
            if (!m)
                continue;
            const [, , kw, name, params, retType] = m;
            const ret = (_e = retType === null || retType === void 0 ? void 0 : retType.trim()) !== null && _e !== void 0 ? _e : 'unknown';
            funcs.set(name, ret);
            sigs.set(name, `${kw} ${name}(${params.trim()}) -> ${ret}`);
            const fnDoc = extractDocstring(lines, i + 1);
            if (fnDoc)
                docs.set(name, fnDoc);
        }
    }
    return { funcs, sigs, docs, classes };
}
exports.parseTlStub = parseTlStub;
/** ar_config.json の python.search_paths を読み込み、絶対パスに解決して返す。 */
async function getPythonSearchPaths(startDir) {
    var _a, _b;
    const result = await findArConfigDirWithKey(startDir, cfg => { var _a, _b; return !!((_b = (_a = cfg.python) === null || _a === void 0 ? void 0 : _a.search_paths) === null || _b === void 0 ? void 0 : _b.length); });
    if (!result)
        return [];
    const { config, dir } = result;
    return ((_b = (_a = config.python) === null || _a === void 0 ? void 0 : _a.search_paths) !== null && _b !== void 0 ? _b : []).map(p => path.isAbsolute(p) ? p : path.join(dir, p));
}
exports.getPythonSearchPaths = getPythonSearchPaths;
let _cachedPythonLibDirs;
/**
 * Python プロセスを実行して標準ライブラリと site-packages のパスを返す。
 * 初回のみサブプロセスが起動し、以降はキャッシュを返す。
 */
async function detectPythonLibDirs() {
    if (_cachedPythonLibDirs !== undefined)
        return _cachedPythonLibDirs;
    const script = [
        'import sysconfig',
        "paths = [sysconfig.get_path('stdlib'), sysconfig.get_path('purelib')]",
        "print('\\n'.join(p for p in paths if p))",
    ].join('; ');
    const candidates = process.platform === 'win32'
        ? ['py', 'python', 'python3']
        : ['python3', 'python'];
    for (const exe of candidates) {
        try {
            const stdout = await new Promise((resolve, reject) => {
                (0, child_process_1.exec)(`${exe} -c "${script}"`, (err, out) => err ? reject(err) : resolve(out));
            });
            const dirs = stdout.trim().split('\n').map(s => s.trim()).filter(Boolean);
            if (dirs.length > 0) {
                _cachedPythonLibDirs = dirs;
                return dirs;
            }
        }
        catch { /* try next */ }
    }
    _cachedPythonLibDirs = [];
    return [];
}
exports.detectPythonLibDirs = detectPythonLibDirs;
/** Walk up from startDir to find the directory containing ar_config.json. */
async function findArConfigDir(startDir) {
    let current = startDir;
    for (;;) {
        try {
            await fs_1.promises.access(path.join(current, 'ar_config.json'));
            return current;
        }
        catch {
            const parent = path.dirname(current);
            if (parent === current)
                return undefined;
            current = parent;
        }
    }
}
async function loadArConfig(startDir) {
    const dir = await findArConfigDir(startDir);
    if (!dir)
        return undefined;
    try {
        return JSON.parse(await fs_1.promises.readFile(path.join(dir, 'ar_config.json'), 'utf8'));
    }
    catch {
        return undefined;
    }
}
/**
 * Walk up from startDir to find the first ar_config.json that passes keyCheck.
 * Returns the parsed config and the directory it was found in.
 */
async function findArConfigDirWithKey(startDir, keyCheck) {
    let current = startDir;
    for (;;) {
        try {
            const cfgPath = path.join(current, 'ar_config.json');
            const cfg = JSON.parse(await fs_1.promises.readFile(cfgPath, 'utf8'));
            if (keyCheck(cfg))
                return { config: cfg, dir: current };
        }
        catch { /* not found or key absent */ }
        const parent = path.dirname(current);
        if (parent === current)
            return undefined;
        current = parent;
    }
}
/** Convert a Rust type string to the equivalent Arrow LangType. */
function rsTypeToTl(rs, selfName) {
    // Strip reference/mut qualifiers
    const t = rs.trim().replace(/^&\s*(?:mut\s+)?/, '').replace(/^mut\s+/, '').trim();
    if (t === 'f32' || t === 'f64')
        return 'float';
    if (/^[iu](?:8|16|32|64|128|size)$/.test(t))
        return 'int';
    if (t === 'bool')
        return 'bool';
    if (t === 'String' || t === 'str' || t === '&str')
        return 'str';
    if (t === '()' || t === '')
        return 'None';
    if (t === 'Self')
        return selfName !== null && selfName !== void 0 ? selfName : 'unknown';
    const vecM = t.match(/^Vec\s*<\s*(.+)\s*>$/);
    if (vecM)
        return `list[${rsTypeToTl(vecM[1], selfName)}]`;
    const optM = t.match(/^Option\s*<\s*(.+)\s*>$/);
    if (optM)
        return `Option[${rsTypeToTl(optM[1], selfName)}]`;
    if (/^[A-Z]/.test(t))
        return t; // named struct/enum — use as-is
    return 'unknown';
}
/** Convert a Rust parameter list to Arrow parameter string. */
function rsParamsToHv(params, selfName) {
    const parts = [];
    for (const raw of params.split(',')) {
        const p = raw.trim().replace(/\s+/g, ' ');
        if (!p)
            continue;
        if (/^&?\s*self$/.test(p)) {
            parts.push('let self');
            continue;
        }
        if (/^&?\s*mut\s+self$/.test(p)) {
            parts.push('mut self');
            continue;
        }
        const withoutRef = p.replace(/^&\s*(?:mut\s+)?/, '');
        const colon = withoutRef.indexOf(':');
        if (colon < 0)
            continue;
        const pName = withoutRef.slice(0, colon).trim().replace(/^mut\s+/, '');
        const typeStr = withoutRef.slice(colon + 1).trim();
        // `&mut T` is writable through the reference → `mut`; `T` / `&T` → `let`.
        const mutability = /^&\s*mut\b/.test(typeStr) ? 'mut' : 'let';
        const pType = rsTypeToTl(typeStr, selfName);
        if (pName && pName !== 'self')
            parts.push(`${mutability} ${pName}: ${pType}`);
    }
    return parts.join(', ');
}
/** Return the index of the closing brace matching the opening brace at startIdx. */
function matchingBrace(src, startIdx) {
    let depth = 0;
    for (let i = startIdx; i < src.length; i++) {
        if (src[i] === '{')
            depth++;
        else if (src[i] === '}' && --depth === 0)
            return i;
    }
    return src.length - 1;
}
/**
 * Parse a Rust lib.rs file and extract public struct fields,
 * impl methods, and top-level free functions.
 */
function parseRustLib(source) {
    var _a, _b, _c;
    const funcs = new Map();
    const sigs = new Map();
    const docs = new Map();
    const classes = new Map();
    // Build doc-comment map: line index → accumulated /// text (on the original source)
    const srcLines = source.split('\n');
    const lineDocMap = new Map(); // line index of `pub fn/struct` → doc string
    {
        let pending = [];
        for (let i = 0; i < srcLines.length; i++) {
            const lt = srcLines[i].trim();
            if (lt.startsWith('///')) {
                pending.push(lt.replace(/^\/\/\/ ?/, ''));
            }
            else {
                if (pending.length > 0 && /\bpub\b/.test(lt)) {
                    lineDocMap.set(i, pending.join('\n').trim());
                }
                pending = [];
            }
        }
    }
    // Strip block comments and line comments (preserving line structure for index math)
    const stripped = source
        .replace(/\/\*[\s\S]*?\*\//g, m => ' '.repeat(m.length))
        .replace(/\/\/[^\n]*/g, m => ' '.repeat(m.length));
    // Helper: convert character offset to 0-based line index
    function charToLine(idx) {
        var _a;
        return ((_a = stripped.slice(0, idx).match(/\n/g)) !== null && _a !== void 0 ? _a : []).length;
    }
    const src = stripped;
    // ── pub struct Name { … } ──────────────────────────────────────────────────
    const structRe = /\bpub\s+struct\s+([A-Za-z_]\w*)\s*\{/g;
    let m;
    while ((m = structRe.exec(src)) !== null) {
        const name = m[1];
        const openIdx = m.index + m[0].lastIndexOf('{');
        const closeIdx = matchingBrace(src, openIdx);
        const body = src.slice(openIdx + 1, closeIdx);
        const clsDoc = lineDocMap.get(charToLine(m.index));
        const cls = {
            fields: new Map(), fieldSigs: [], methods: new Map(), methodSigs: [],
            bases: [], classDocs: clsDoc,
        };
        classes.set(name, cls);
        // pub fieldname: Type,
        const fieldRe = /\bpub\s+([A-Za-z_]\w*)\s*:\s*([^,\n}]+)/g;
        let fm;
        while ((fm = fieldRe.exec(body)) !== null) {
            const fname = fm[1];
            const ftype = rsTypeToTl(fm[2].trim(), name);
            cls.fields.set(fname, ftype);
            cls.fieldSigs.push(`${fname}: ${ftype}`);
        }
    }
    // ── impl Trait for StructName { … } ──────────────────────────────────────
    const implTraitRe = /\bimpl\s+([A-Za-z_][\w:<>]*)\s+for\s+([A-Za-z_]\w*)\s*\{/g;
    while ((m = implTraitRe.exec(src)) !== null) {
        const traitName = (_c = (_b = (_a = m[1].split(':').pop()) === null || _a === void 0 ? void 0 : _a.split('<')[0]) === null || _b === void 0 ? void 0 : _b.trim()) !== null && _c !== void 0 ? _c : m[1];
        const structName = m[2];
        const cls = classes.get(structName);
        if (cls && !cls.bases.includes(traitName))
            cls.bases.push(traitName);
    }
    // ── impl StructName { … } ─────────────────────────────────────────────────
    const implRe = /\bimpl\s+([A-Za-z_]\w*)\s*\{/g;
    while ((m = implRe.exec(src)) !== null) {
        const implName = m[1];
        const openIdx = m.index + m[0].lastIndexOf('{');
        const closeIdx = matchingBrace(src, openIdx);
        const body = src.slice(openIdx + 1, closeIdx);
        const bodyStartLine = charToLine(openIdx + 1);
        const cls = classes.get(implName);
        if (!cls)
            continue;
        // pub fn name(params) -> RetType { … }
        const fnRe = /\bpub\s+fn\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*(?:->\s*([^{;,\n]+?))?\s*[{;]/g;
        let fm;
        while ((fm = fnRe.exec(body)) !== null) {
            const [, fname, paramsRaw, retRs] = fm;
            if (fname === 'new')
                continue;
            const ret = retRs ? rsTypeToTl(retRs.trim(), implName) : 'None';
            const hvPrms = rsParamsToHv(paramsRaw, implName);
            const msig = `fn ${fname}(${hvPrms}) -> ${ret}`;
            const fnLine = bodyStartLine + charToLine(fm.index);
            const fnDoc = lineDocMap.get(fnLine);
            cls.methods.set(fname, { ret, sig: msig, doc: fnDoc });
            cls.methodSigs.push(msig);
        }
    }
    // ── top-level pub fn (depth 0 only) ───────────────────────────────────────
    const topFnRe = /\bpub\s+fn\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*(?:->\s*([^{;,\n]+?))?\s*\{/g;
    while ((m = topFnRe.exec(src)) !== null) {
        let depth = 0;
        for (let i = 0; i < m.index; i++) {
            if (src[i] === '{')
                depth++;
            else if (src[i] === '}')
                depth--;
        }
        if (depth !== 0)
            continue;
        const [, fname, paramsRaw, retRs] = m;
        const ret = retRs ? rsTypeToTl(retRs.trim()) : 'None';
        const hvPrms = rsParamsToHv(paramsRaw);
        funcs.set(fname, ret);
        sigs.set(fname, `fn ${fname}(${hvPrms}) -> ${ret}`);
        const fnDoc = lineDocMap.get(charToLine(m.index));
        if (fnDoc)
            docs.set(fname, fnDoc);
    }
    return { funcs, sigs, docs, classes };
}
exports.parseRustLib = parseRustLib;
// ===== JS module parsing =====
/** Map a JavaScript type string (from JSDoc) to an Arrow LangType. */
function jsTypeToArrow(jsType) {
    switch (jsType.toLowerCase().trim()) {
        case 'string': return 'str';
        case 'number': return 'float';
        case 'boolean': return 'bool';
        case 'null':
        case 'undefined':
        case 'void': return 'None';
        case 'array': return 'List';
        case 'object': return 'unknown';
        default: return 'unknown';
    }
}
/**
 * Parse a JSDoc block and build an Arrow-style signature for `fnName`.
 * Falls back to using the raw parameter list from the source if no JSDoc.
 */
function buildJsSig(fnName, content, jsdocRaw) {
    // Try JSDoc @param / @returns
    if (jsdocRaw) {
        const params = [];
        const paramRe = /@param\s+\{([^}]+)\}\s+(\w+)/g;
        let pm;
        while ((pm = paramRe.exec(jsdocRaw)) !== null) {
            params.push(`${pm[2]}: ${jsTypeToArrow(pm[1])}`);
        }
        const retMatch = /@returns?\s+\{([^}]+)\}/.exec(jsdocRaw);
        const retType = retMatch ? jsTypeToArrow(retMatch[1]) : 'unknown';
        if (params.length > 0 || retMatch) {
            return `fn ${fnName}(${params.join(', ')}) -> ${retType}`;
        }
    }
    // Fallback: extract raw parameter list from function body
    const fnRe = new RegExp(`(?:async\\s+)?function\\s+${fnName}\\s*\\(([^)]*)\\)`, 'g');
    const fm = fnRe.exec(content);
    if (fm) {
        const params = fm[1].split(',')
            .map(p => p.trim())
            .filter(Boolean)
            .map(p => `${p}: unknown`);
        return `fn ${fnName}(${params.join(', ')}) -> unknown`;
    }
    return `fn ${fnName}(...) -> unknown`;
}
/**
 * Extract function names and signatures from a CJS/ESM JS file.
 *
 * Handles:
 *  - `module.exports = { fn1, fn2, fn3 }`
 *  - `module.exports = { fn1: ..., fn2: ... }`
 *  - `exports.fnName = function(...) { ... }`
 *  - `exports.fnName = async function(...) { ... }`
 *
 * Also harvests JSDoc `/** ... *\/` blocks preceding each function.
 */
function parseJsModule(content) {
    var _a;
    const funcs = new Map();
    const sigs = new Map();
    const docs = new Map();
    // Build name → JSDoc raw-text map
    const jsdocMap = new Map();
    const jsdocRe = /\/\*\*([\s\S]*?)\*\/\s*(?:async\s+)?function\s+([A-Za-z_]\w*)/g;
    let jm;
    while ((jm = jsdocRe.exec(content)) !== null) {
        const raw = jm[1].replace(/^\s*\*/gm, '').trim();
        jsdocMap.set(jm[2], raw);
    }
    // Helper: register a function name
    const register = (name) => {
        if (name === 'exports' || name === 'module')
            return;
        funcs.set(name, 'unknown');
        sigs.set(name, buildJsSig(name, content, jsdocMap.get(name)));
        const rawDoc = jsdocMap.get(name);
        if (rawDoc) {
            // Strip JSDoc tags to get plain description
            const desc = rawDoc.replace(/@\w+[^\n]*/g, '').trim();
            if (desc)
                docs.set(name, desc);
        }
    };
    // Pattern 1: module.exports = { fn1, fn2, fn3 }
    // Handles the simple single-line pattern used in our bridge modules.
    // For multi-line, we search for the last occurrence.
    const modExRe = /module\.exports\s*=\s*\{([^}]+)\}/gs;
    let em;
    while ((em = modExRe.exec(content)) !== null) {
        for (const part of em[1].split(',')) {
            // "fn1" or "fn1: someRef" or whitespace
            const name = (_a = part.trim().match(/^([A-Za-z_]\w*)(?:\s*:.*)?$/)) === null || _a === void 0 ? void 0 : _a[1];
            if (name)
                register(name);
        }
    }
    // Pattern 2: exports.fnName = function / exports.fnName = async function
    const exAssignRe = /^exports\.([A-Za-z_]\w*)\s*=\s*(?:async\s+)?function/gm;
    let ea;
    while ((ea = exAssignRe.exec(content)) !== null) {
        if (!funcs.has(ea[1]))
            register(ea[1]);
    }
    // Pattern 3: top-level named export arrow functions
    // exports.fnName = (...) =>
    const exArrowRe = /^exports\.([A-Za-z_]\w*)\s*=\s*(?:async\s+)?\(/gm;
    let arr;
    while ((arr = exArrowRe.exec(content)) !== null) {
        if (!funcs.has(arr[1]))
            register(arr[1]);
    }
    // Pattern 4: TypeScript CJS output — exports.fnName = identifier;
    // Handles `exports.stripComment = stripComment;` (tsc --module commonjs output)
    const exRefRe = /^exports\.([A-Za-z_]\w*)\s*=\s*[A-Za-z_]\w*\s*;/gm;
    let er;
    while ((er = exRefRe.exec(content)) !== null) {
        if (!funcs.has(er[1]))
            register(er[1]);
    }
    return { funcs, sigs, docs, classes: new Map() };
}
// ===== resolveImportSourceFile =====
/**
 * Resolve the most useful human-readable source file for an import (for "Go to Definition").
 * Priority: .ars stub > .ar source > language-specific source file
 * Returns the absolute path of the file to open, or undefined if not found.
 */
async function resolveImportSourceFile(importKind, modulePath, stubName, docDir, extraSearchPaths = []) {
    var _a, _b, _c, _d, _e, _f, _g, _h;
    const kind = importKindOf(importKind);
    const parts = modulePath.split('.');
    const filePath = path.join(docDir, ...parts);
    async function exists(p) {
        try {
            await fs_1.promises.access(p);
            return true;
        }
        catch {
            return false;
        }
    }
    if (kind === 'ar') {
        for (const c of [
            filePath + '.ars',
            filePath + '.ar',
            path.join(filePath, '__init__.ars'),
            path.join(filePath, '__init__.ar'),
        ]) {
            if (await exists(c))
                return c;
        }
        return undefined;
    }
    if (kind === 'py') {
        for (const searchDir of [docDir, ...extraSearchPaths]) {
            for (const ext of ['.pyi', '.py']) {
                for (const c of [
                    path.join(searchDir, ...modulePath.split('.')) + ext,
                    path.join(searchDir, modulePath + ext),
                ]) {
                    if (await exists(c))
                        return c;
                }
            }
        }
        return undefined;
    }
    if (kind === 'rs') {
        const cfgResult = await findArConfigDirWithKey(docDir, cfg => { var _a; return !!((_a = cfg.rust) === null || _a === void 0 ? void 0 : _a.crates_path); });
        const configDir = (_a = cfgResult === null || cfgResult === void 0 ? void 0 : cfgResult.dir) !== null && _a !== void 0 ? _a : docDir;
        const rawPaths = (_b = cfgResult === null || cfgResult === void 0 ? void 0 : cfgResult.config.rust) === null || _b === void 0 ? void 0 : _b.crates_path;
        const cratesPaths = Array.isArray(rawPaths) ? rawPaths : rawPaths ? [rawPaths] : [];
        for (const cratesPath of cratesPaths) {
            const resolved = path.isAbsolute(cratesPath) ? cratesPath : path.resolve(configDir, cratesPath);
            const libRs = path.join(resolved, modulePath, 'src', 'lib.rs');
            if (await exists(libRs))
                return libRs;
        }
        return undefined;
    }
    if (kind === 'cpp') {
        const candidates = [];
        if (stubName)
            candidates.push(path.join(docDir, stubName + '.h'));
        candidates.push(filePath + '.h');
        candidates.push(path.join(docDir, parts[parts.length - 1] + '.h'));
        for (const c of candidates) {
            if (await exists(c))
                return c;
        }
        return undefined;
    }
    if (kind === 'cs') {
        // C# DLLs are binary; prefer a generated .ars stub if available
        // 1. Flat: docDir/WpfShell.ars
        const arsPath = filePath + '.ars';
        if (await exists(arsPath))
            return arsPath;
        // 2. Subdirectory: docDir/WpfShell/WpfShell.ars (single-segment module in same-name dir)
        const lastName = parts[parts.length - 1];
        const subDirArs = path.join(filePath, lastName + '.ars');
        if (await exists(subDirArs))
            return subDirArs;
        // 3. ar_config.json csharp.lib_paths
        const cfgResult = await findArConfigDirWithKey(docDir, cfg => { var _a; return !!((_a = cfg.csharp) === null || _a === void 0 ? void 0 : _a.lib_paths); });
        if (cfgResult) {
            const rawCsPaths = (_c = cfgResult.config.csharp) === null || _c === void 0 ? void 0 : _c.lib_paths;
            const csPaths = Array.isArray(rawCsPaths) ? rawCsPaths : rawCsPaths ? [rawCsPaths] : [];
            for (const lp of csPaths) {
                const resolved = path.isAbsolute(lp) ? lp : path.resolve(cfgResult.dir, lp);
                const candidate = path.join(resolved, lastName + '.ars');
                if (await exists(candidate))
                    return candidate;
            }
        }
        return undefined;
    }
    if (kind === 'js') {
        // Prefer .ars stub
        const arsPath = filePath + '.ars';
        if (await exists(arsPath))
            return arsPath;
        const jsResult = await findArConfigDirWithKey(docDir, cfg => !!cfg.javascript);
        const config = jsResult === null || jsResult === void 0 ? void 0 : jsResult.config;
        const configDir = (_d = jsResult === null || jsResult === void 0 ? void 0 : jsResult.dir) !== null && _d !== void 0 ? _d : docDir;
        const bridgeScript = (_f = (_e = config === null || config === void 0 ? void 0 : config.javascript) === null || _e === void 0 ? void 0 : _e.bridge_script) !== null && _f !== void 0 ? _f : 'bridge/js_bridge.cjs';
        const bridgeRoot = (_h = (_g = config === null || config === void 0 ? void 0 : config.javascript) === null || _g === void 0 ? void 0 : _g.bridge_root) !== null && _h !== void 0 ? _h : '.';
        const bridgeDir = path.dirname(path.resolve(configDir, bridgeScript));
        const rootDir = path.resolve(configDir, bridgeRoot);
        const relPath = modulePath.replace(/\./g, '/');
        for (const c of [
            path.join(bridgeDir, relPath + '.cjs'),
            path.join(bridgeDir, relPath + '.js'),
            path.join(rootDir, relPath + '.cjs'),
            path.join(rootDir, relPath + '.js'),
        ]) {
            if (await exists(c))
                return c;
        }
        return undefined;
    }
    // 'ar' fallback (already handled above, but for safety)
    return undefined;
}
exports.resolveImportSourceFile = resolveImportSourceFile;
// ===== loadNativeModuleInfo =====
async function loadNativeModuleInfo(importKind, modulePath, stubName, docDir) {
    var _a, _b, _c, _d, _e, _f, _g, _h, _j, _k;
    const empty = { funcs: new Map(), sigs: new Map(), docs: new Map(), classes: new Map() };
    if (importKindOf(importKind) === 'cpp') {
        const candidates = [];
        if (stubName) {
            candidates.push(path.join(docDir, stubName + '.h'));
        }
        const parts = modulePath.split('.');
        const lastName = parts[parts.length - 1];
        candidates.push(path.join(docDir, ...parts) + '.h');
        candidates.push(path.join(docDir, lastName + '.h'));
        // Also try: subdir named after the module containing a same-named header
        candidates.push(path.join(docDir, ...parts, lastName + '.h'));
        for (const hPath of candidates) {
            try {
                const content = await fs_1.promises.readFile(hPath, 'utf8');
                return parseCHeader(content, path.dirname(hPath));
            }
            catch { /* try next candidate */ }
        }
        // Last resort: scan the module subdirectory for any .h file
        const subDir = path.join(docDir, ...parts);
        try {
            const entries = await fs_1.promises.readdir(subDir);
            const headers = entries.filter(e => e.endsWith('.h')).sort();
            for (const h of headers) {
                try {
                    const content = await fs_1.promises.readFile(path.join(subDir, h), 'utf8');
                    return parseCHeader(content, subDir);
                }
                catch { /* try next */ }
            }
        }
        catch { /* subDir doesn't exist */ }
        return empty;
    }
    // ── import[rs]: parse Rust source via ar_config.json crates_path ──────────
    if (importKindOf(importKind) === 'rs') {
        const config = await loadArConfig(docDir);
        const configDir = (_a = await findArConfigDir(docDir)) !== null && _a !== void 0 ? _a : docDir;
        const rawPaths = (_b = config === null || config === void 0 ? void 0 : config.rust) === null || _b === void 0 ? void 0 : _b.crates_path;
        const cratesPaths = Array.isArray(rawPaths) ? rawPaths : rawPaths ? [rawPaths] : [];
        for (const cratesPath of cratesPaths) {
            const resolved = path.isAbsolute(cratesPath)
                ? cratesPath
                : path.resolve(configDir, cratesPath);
            const libRs = path.join(resolved, modulePath, 'src', 'lib.rs');
            try {
                const content = await fs_1.promises.readFile(libRs, 'utf8');
                return parseRustLib(content);
            }
            catch { /* try next candidate */ }
        }
        return empty;
    }
    // ── import[cs-dll] / import[cs-proc]: read ECMA-335 metadata from .NET DLL ─
    if (importKindOf(importKind) === 'cs') {
        const lastName = (_c = modulePath.split('.').pop()) !== null && _c !== void 0 ? _c : modulePath;
        const dllName = lastName + '.dll';
        const parts = modulePath.split('.');
        const filePath = path.join(docDir, ...parts);
        // ① Check quick .ars candidates first (no config walk needed)
        const quickArsCandidates = [filePath + '.ars'];
        if (parts.length === 1) {
            quickArsCandidates.push(path.join(filePath, lastName + '.ars'));
        }
        for (const arsPath of quickArsCandidates) {
            try {
                const content = await fs_1.promises.readFile(arsPath, 'utf8');
                return parseTlStub(content);
            }
            catch { /* try next */ }
        }
        // ② Walk ar_config for lib_paths and try those .ars stubs
        const cfgForArs = await findArConfigDirWithKey(docDir, cfg => { var _a; return !!((_a = cfg.csharp) === null || _a === void 0 ? void 0 : _a.lib_paths); });
        const configDir = (_d = cfgForArs === null || cfgForArs === void 0 ? void 0 : cfgForArs.dir) !== null && _d !== void 0 ? _d : docDir;
        const rawCsPaths = (_e = cfgForArs === null || cfgForArs === void 0 ? void 0 : cfgForArs.config.csharp) === null || _e === void 0 ? void 0 : _e.lib_paths;
        const csLibPaths = Array.isArray(rawCsPaths) ? rawCsPaths : rawCsPaths ? [rawCsPaths] : [];
        for (const lp of csLibPaths) {
            const resolved = path.isAbsolute(lp) ? lp : path.resolve(configDir, lp);
            try {
                const content = await fs_1.promises.readFile(path.join(resolved, lastName + '.ars'), 'utf8');
                return parseTlStub(content);
            }
            catch { /* try next */ }
        }
        // ③ Fall back to parsing the DLL binary (slow path)
        const dllCandidates = [
            filePath + '.dll',
            path.join(docDir, dllName),
        ];
        if (parts.length === 1) {
            dllCandidates.push(path.join(filePath, dllName));
        }
        for (const lp of csLibPaths) {
            const resolved = path.isAbsolute(lp) ? lp : path.resolve(configDir, lp);
            dllCandidates.push(path.join(resolved, dllName));
        }
        for (const candidate of dllCandidates) {
            try {
                const buf = await fs_1.promises.readFile(candidate);
                let xmlContent;
                try {
                    const xmlPath = candidate.replace(/\.dll$/i, '.xml');
                    xmlContent = await fs_1.promises.readFile(xmlPath, 'utf-8');
                }
                catch { /* XML doc file is optional */ }
                return (0, cs_assembly_1.parseNetAssembly)(buf, xmlContent);
            }
            catch { /* try next */ }
        }
        return empty;
    }
    // ── import[js-proc]: find the JS/CJS file and parse its exports ──────────
    if (importKindOf(importKind) === 'js') {
        // 1. Check for an .ars stub file first (provides Arrow-typed signatures)
        const arsPath = path.join(docDir, ...modulePath.split('.')) + '.ars';
        try {
            const arsContent = await fs_1.promises.readFile(arsPath, 'utf8');
            return parseTlStub(arsContent);
        }
        catch { /* no stub — fall through to JS parsing */ }
        // 2. Find the JS file via ar_config.json — walk up to find a config with javascript section
        const jsResult = await findArConfigDirWithKey(docDir, cfg => !!cfg.javascript);
        const config = jsResult === null || jsResult === void 0 ? void 0 : jsResult.config;
        const configDir = (_f = jsResult === null || jsResult === void 0 ? void 0 : jsResult.dir) !== null && _f !== void 0 ? _f : docDir;
        const bridgeScript = (_h = (_g = config === null || config === void 0 ? void 0 : config.javascript) === null || _g === void 0 ? void 0 : _g.bridge_script) !== null && _h !== void 0 ? _h : 'bridge/js_bridge.cjs';
        const bridgeRoot = (_k = (_j = config === null || config === void 0 ? void 0 : config.javascript) === null || _j === void 0 ? void 0 : _j.bridge_root) !== null && _k !== void 0 ? _k : '.';
        const bridgeDir = path.dirname(path.resolve(configDir, bridgeScript));
        const rootDir = path.resolve(configDir, bridgeRoot);
        const relPath = modulePath.replace(/\./g, '/');
        const jsCandidates = [
            path.join(bridgeDir, relPath + '.cjs'),
            path.join(bridgeDir, relPath + '.js'),
            path.join(rootDir, relPath + '.cjs'),
            path.join(rootDir, relPath + '.js'),
            path.join(rootDir, relPath, 'index.cjs'),
            path.join(rootDir, relPath, 'index.js'),
        ];
        for (const jsPath of jsCandidates) {
            try {
                const content = await fs_1.promises.readFile(jsPath, 'utf8');
                return parseJsModule(content);
            }
            catch { /* try next */ }
        }
        return empty;
    }
    // ── import[ar] / import[arc]: look for .ars or .ar stub ──────────────────
    const filePath = path.join(docDir, ...modulePath.split('.'));
    const candidates = [
        filePath + '.ars',
        filePath + '.ar',
        path.join(filePath, '__init__.ars'),
        path.join(filePath, '__init__.ar'),
    ];
    for (const candidate of candidates) {
        try {
            const content = await fs_1.promises.readFile(candidate, 'utf8');
            return parseTlStub(content);
        }
        catch { /* try next candidate */ }
    }
    return empty;
}
exports.loadNativeModuleInfo = loadNativeModuleInfo;
//# sourceMappingURL=native_module.js.map