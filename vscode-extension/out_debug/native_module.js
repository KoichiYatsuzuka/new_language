"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.loadNativeModuleInfo = exports.parseTlStub = exports.parseCppClasses = exports.parseCHeader = exports.parseCParam = exports.cTypeToTl = exports.importKindOf = void 0;
const fs = require("fs");
const path = require("path");
const builtins_1 = require("./builtins");
// ===== C++ / native module support =====
function importKindOf(keyword) {
    if (keyword.includes('cpp'))
        return 'cpp';
    if (keyword === 'import[rs]')
        return 'rs';
    if (keyword === 'import' || keyword.startsWith('import[hv'))
        return 'hv';
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
function parseCParam(param, idx) {
    const isPointer = param.includes('*');
    const clean = param.replace(/\bconst\b/g, '').replace(/\*/g, '').replace(/\s+/g, ' ').trim();
    const parts = clean.split(/\s+/);
    const rawName = parts.length > 1 ? parts[parts.length - 1] : '';
    const finalName = /^[A-Za-z_]\w*$/.test(rawName) ? rawName : `p${idx}`;
    const baseType = (parts.length > 1 ? parts.slice(0, -1).join(' ') : parts[0]).trim();
    const tlType = isPointer
        ? (/\bchar\b/.test(baseType) ? 'str' : 'int')
        : cTypeToTl(baseType);
    return `${finalName}: ${tlType}`;
}
exports.parseCParam = parseCParam;
function parseCHeader(content, dir = '', _depth = 0) {
    const funcs = new Map();
    const sigs = new Map();
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
        while ((inc = includeRe.exec(content)) !== null) {
            const incPath = path.join(dir, inc[1]);
            if (fs.existsSync(incPath)) {
                try {
                    const sub = parseCHeader(fs.readFileSync(incPath, 'utf8'), path.dirname(incPath), _depth + 1);
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
            }
        }
    }
    return { funcs, sigs, docs: new Map(), classes };
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
                classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs, methods: new Map(), methodSigs: [] });
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
            classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs, methods: new Map(), methodSigs: [] });
        }
    }
    return classes;
}
exports.parseCppClasses = parseCppClasses;
const HVS_CLASS_RE = /^(\s*)class\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?(?:\([^)]*\))?(?:->[A-Za-z_]\w*)?\s*:/;
const HVS_FIELD_RE = /^(\s*)(?:let|mut|const)\s+([A-Za-z_]\w*)\s*:\s*(.+)/;
const HVS_METHOD_RE = /^(\s*)(fn|gen)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:/;
function parseTlStub(content) {
    var _a, _b, _c, _d, _e, _f;
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
        // Class definition: `class Name->Name:` (stub format) or `class Name:`
        const classM = raw.match(HVS_CLASS_RE);
        if (classM && currentClass === null) {
            currentClass = classM[2];
            classIndent = ((_c = classM[1]) !== null && _c !== void 0 ? _c : '').length;
            bodyIndent = -1;
            classes.set(currentClass, { fields: new Map(), fieldSigs: [], methods: new Map(), methodSigs: [] });
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
                cls.methods.set(mname, { ret, sig: msig });
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
            for (let j = i + 1; j < lines.length; j++) {
                const next = lines[j].trim();
                if (!next)
                    continue;
                const docM = next.match(/^(?:"""(.*?)"""|'''(.*?)''')$/);
                if (docM)
                    docs.set(name, ((_f = docM[1]) !== null && _f !== void 0 ? _f : docM[2]).trim());
                break;
            }
        }
    }
    return { funcs, sigs, docs, classes };
}
exports.parseTlStub = parseTlStub;
function loadNativeModuleInfo(importKind, modulePath, stubName, docDir) {
    const empty = { funcs: new Map(), sigs: new Map(), docs: new Map(), classes: new Map() };
    if (importKindOf(importKind) === 'cpp') {
        const candidates = [];
        if (stubName) {
            candidates.push(path.join(docDir, stubName + '.h'));
        }
        const parts = modulePath.split('.');
        candidates.push(path.join(docDir, ...parts) + '.h');
        candidates.push(path.join(docDir, parts[parts.length - 1] + '.h'));
        for (const hPath of candidates) {
            if (fs.existsSync(hPath)) {
                try {
                    return parseCHeader(fs.readFileSync(hPath, 'utf8'), path.dirname(hPath));
                }
                catch { /* ignore */ }
            }
        }
        return empty;
    }
    const filePath = path.join(docDir, ...modulePath.split('.'));
    const candidates = [
        filePath + '.hvs',
        filePath + '.hv',
        path.join(filePath, '__init__.hvs'),
        path.join(filePath, '__init__.hv'),
    ];
    for (const candidate of candidates) {
        if (fs.existsSync(candidate)) {
            try {
                return parseTlStub(fs.readFileSync(candidate, 'utf8'));
            }
            catch { /* ignore */ }
        }
    }
    return empty;
}
exports.loadNativeModuleInfo = loadNativeModuleInfo;
//# sourceMappingURL=native_module.js.map