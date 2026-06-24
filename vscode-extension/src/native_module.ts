import * as fs from 'fs';
import { promises as fsPromises } from 'fs';
import * as path from 'path';
import type { LangType } from './builtins';
import { FUNC_DEF_RE } from './builtins';
import { parseNetAssembly } from './cs_assembly';

// ===== C++ / native module support =====

export function importKindOf(keyword: string): 'py' | 'cpp' | 'ar' | 'rs' | 'cs' {
    if (keyword.includes('cpp')) return 'cpp';
    if (keyword === 'import[rs]') return 'rs';
    if (keyword === 'import[cs-dll]' || keyword === 'import[cs-proc]') return 'cs';
    if (keyword === 'import' || keyword.startsWith('import[hv')) return 'ar';
    return 'py';
}

export function cTypeToTl(cType: string): LangType {
    const t = cType.replace(/\bconst\b/g, '').trim();
    if (!t || t === 'void') return 'None';
    if (/\bchar\b/.test(t) && /[*\[]/.test(t)) return 'str';
    if (/\b(?:double|float)\b/.test(t)) return 'float';
    if (/\bbool\b/.test(t)) return 'bool';
    if (/[*\[]/.test(t)) return 'int'; // pointer/array → opaque handle
    return 'int'; // int, long, DWORD, HWND, size_t, etc.
}

export function parseCParam(param: string, idx: number): string {
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

export interface CppClassInfo {
    fields: Map<string, LangType>;
    fieldSigs: string[];
    methods: Map<string, { ret: LangType; sig: string }>;
    methodSigs: string[];
}

export interface NativeModuleInfo {
    funcs: Map<string, LangType>;
    sigs: Map<string, string>;
    docs: Map<string, string>;
    classes: Map<string, CppClassInfo>;
}

export async function parseCHeader(content: string, dir: string = '', _depth: number = 0): Promise<NativeModuleInfo> {
    const funcs = new Map<string, LangType>();
    const sigs = new Map<string, string>();
    const src = content.replace(/\/\/[^\n]*/g, '').replace(/\/\*[\s\S]*?\*\//g, '');
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
    if (dir && _depth < 2) {
        const includeRe = /^#include\s+"([^"]+)"/gm;
        let inc: RegExpExecArray | null;
        const subPromises: Promise<void>[] = [];
        while ((inc = includeRe.exec(content)) !== null) {
            const incPath = path.join(dir, inc[1]);
            subPromises.push((async (p: string) => {
                try {
                    const subContent = await fsPromises.readFile(p, 'utf8');
                    const sub = await parseCHeader(subContent, path.dirname(p), _depth + 1);
                    for (const [k, v] of sub.funcs) if (!funcs.has(k)) funcs.set(k, v);
                    for (const [k, v] of sub.sigs)  if (!sigs.has(k))  sigs.set(k, v);
                    for (const [k, v] of sub.classes) if (!classes.has(k)) classes.set(k, v);
                } catch { /* ignore unreadable sub-headers */ }
            })(incPath));
        }
        await Promise.all(subPromises);
    }
    return { funcs, sigs, docs: new Map(), classes };
}

export function parseCppClasses(src: string): Map<string, CppClassInfo> {
    const classes = new Map<string, CppClassInfo>();

    interface ClassCtx {
        name: string;
        openDepth: number;
        isPublic: boolean;
        fields: Map<string, LangType>;
        fieldSigs: string[];
        isTyepdef: boolean;
    }

    const classStack: ClassCtx[] = [];
    let depth = 0;
    let pendingName: string | undefined;
    let pendingIsStruct = false;

    for (const rawLine of src.split('\n')) {
        const line = rawLine.trim();
        const prevDepth = depth;

        for (const ch of rawLine) {
            if (ch === '{') depth++;
            else if (ch === '}') depth--;
        }

        if (pendingName !== undefined && depth > prevDepth) {
            classStack.push({
                name: pendingName, openDepth: depth,
                isPublic: pendingIsStruct, fields: new Map(), fieldSigs: [], isTyepdef: false,
            });
            pendingName = undefined;
        }

        while (classStack.length > 0 && depth < classStack[classStack.length - 1].openDepth) {
            const cls = classStack.pop()!;
            if (cls.isTyepdef) {
                const tdName = line.match(/\}\s*([A-Za-z_]\w*)\s*;/)?.[1];
                if (tdName) cls.name = tdName;
            }
            if (cls.name && cls.fields.size > 0) {
                classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs, methods: new Map(), methodSigs: [] });
            }
        }

        if (!line || line.startsWith('#')) continue;

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
            } else if (!line.includes('{')) {
                pendingName = name;
                pendingIsStruct = isStruct;
            }
            continue;
        }

        if (classStack.length === 0) continue;
        const ctx = classStack[classStack.length - 1];
        if (depth !== ctx.openDepth) continue;

        const accessM = line.match(/^(public|private|protected)\s*:/);
        if (accessM) { ctx.isPublic = accessM[1] === 'public'; continue; }
        if (!ctx.isPublic) continue;

        if (line.includes('(')) continue;
        if (/^(typedef|using|static|virtual|explicit|inline|friend|extern|template)/.test(line)) continue;
        if (line.startsWith('~') || line.startsWith(ctx.name + ' ') || line === ctx.name) continue;

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

    for (const cls of classStack) {
        if (cls.name && cls.fields.size > 0) {
            classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs, methods: new Map(), methodSigs: [] });
        }
    }

    return classes;
}

const HVS_CLASS_RE = /^(\s*)class\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?(?:\([^)]*\))?(?:->[A-Za-z_]\w*)?\s*:/;
const HVS_FIELD_RE = /^(\s*)(?:let|mut|const)\s+([A-Za-z_]\w*)\s*:\s*(.+)/;
const HVS_METHOD_RE = /^(\s*)(fn|gen)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:/;

export function parseTlStub(content: string): NativeModuleInfo {
    const funcs = new Map<string, LangType>();
    const sigs = new Map<string, string>();
    const docs = new Map<string, string>();
    const classes = new Map<string, CppClassInfo>();
    const lines = content.split('\n');

    let currentClass: string | null = null;
    let classIndent = 0;
    let bodyIndent = -1;

    for (let i = 0; i < lines.length; i++) {
        const raw = lines[i];
        const trimmed = raw.trim();
        if (!trimmed) continue;

        const lineIndent = (raw.match(/^(\s*)/)?.[1] ?? '').length;

        // Exit class when we see a non-empty line at or before class indent
        if (currentClass !== null && lineIndent <= classIndent) {
            currentClass = null;
            bodyIndent = -1;
        }

        // Class definition: `class Name->Name:` (stub format) or `class Name:`
        const classM = raw.match(HVS_CLASS_RE);
        if (classM && currentClass === null) {
            currentClass = classM[2];
            classIndent = (classM[1] ?? '').length;
            bodyIndent = -1;
            classes.set(currentClass, { fields: new Map(), fieldSigs: [], methods: new Map(), methodSigs: [] });
            continue;
        }

        if (currentClass !== null) {
            if (bodyIndent === -1) bodyIndent = lineIndent;
            // Only process direct class members (skip nested)
            if (lineIndent !== bodyIndent) continue;
            if (/^(?:public|private|protected)\s*:/.test(trimmed) || trimmed === '...') continue;

            const cls = classes.get(currentClass)!;

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
                if (mname === '__init__') continue;
                const ret = mret?.trim() ?? 'None';
                const cleanParams = mparams
                    .replace(/^\s*(?:let\s+|mut\s+)?self\s*,\s*/, '')
                    .replace(/^\s*(?:let\s+|mut\s+)?self\s*$/, '');
                const msig = `${kw} ${mname}(${cleanParams.trim()}) -> ${ret}`;
                cls.methods.set(mname, { ret, sig: msig });
                cls.methodSigs.push(msig);
                continue;
            }
        } else {
            // Top-level function
            const m = raw.match(FUNC_DEF_RE);
            if (!m) continue;
            const [, , kw, name, params, retType] = m;
            const ret = retType?.trim() ?? 'unknown';
            funcs.set(name, ret);
            sigs.set(name, `${kw} ${name}(${params.trim()}) -> ${ret}`);
            for (let j = i + 1; j < lines.length; j++) {
                const next = lines[j].trim();
                if (!next) continue;
                const docM = next.match(/^(?:"""(.*?)"""|'''(.*?)''')$/);
                if (docM) docs.set(name, (docM[1] ?? docM[2]).trim());
                break;
            }
        }
    }
    return { funcs, sigs, docs, classes };
}

// ===== Rust source parser =====

interface ArConfig {
    rust?:    { crates_path?: string | string[] };
    csharp?:  { lib_paths?: string | string[] };
}

/** Walk up from startDir to find the directory containing ar_config.json. */
async function findArConfigDir(startDir: string): Promise<string | undefined> {
    let current = startDir;
    for (;;) {
        try {
            await fsPromises.access(path.join(current, 'ar_config.json'));
            return current;
        } catch {
            const parent = path.dirname(current);
            if (parent === current) return undefined;
            current = parent;
        }
    }
}

async function loadArConfig(startDir: string): Promise<ArConfig | undefined> {
    const dir = await findArConfigDir(startDir);
    if (!dir) return undefined;
    try { return JSON.parse(await fsPromises.readFile(path.join(dir, 'ar_config.json'), 'utf8')); }
    catch { return undefined; }
}

/** Convert a Rust type string to the equivalent Arrow LangType. */
function rsTypeToTl(rs: string, selfName?: string): LangType {
    // Strip reference/mut qualifiers
    const t = rs.trim().replace(/^&\s*(?:mut\s+)?/, '').replace(/^mut\s+/, '').trim();
    if (t === 'f32' || t === 'f64') return 'float';
    if (/^[iu](?:8|16|32|64|128|size)$/.test(t)) return 'int';
    if (t === 'bool') return 'bool';
    if (t === 'String' || t === 'str' || t === '&str') return 'str';
    if (t === '()' || t === '') return 'None';
    if (t === 'Self') return selfName ?? 'unknown';
    const vecM  = t.match(/^Vec\s*<\s*(.+)\s*>$/);
    if (vecM) return `list[${rsTypeToTl(vecM[1], selfName)}]`;
    const optM  = t.match(/^Option\s*<\s*(.+)\s*>$/);
    if (optM) return `Option[${rsTypeToTl(optM[1], selfName)}]`;
    if (/^[A-Z]/.test(t)) return t;  // named struct/enum — use as-is
    return 'unknown';
}

/** Convert a Rust parameter list to Arrow parameter string. */
function rsParamsToHv(params: string, selfName?: string): string {
    const parts: string[] = [];
    for (const raw of params.split(',')) {
        const p = raw.trim().replace(/\s+/g, ' ');
        if (!p) continue;
        if (/^&?\s*self$/.test(p))     { parts.push('let self'); continue; }
        if (/^&?\s*mut\s+self$/.test(p)) { parts.push('mut self'); continue; }
        const withoutRef = p.replace(/^&\s*(?:mut\s+)?/, '');
        const colon = withoutRef.indexOf(':');
        if (colon < 0) continue;
        const pName = withoutRef.slice(0, colon).trim().replace(/^mut\s+/, '');
        const pType = rsTypeToTl(withoutRef.slice(colon + 1).trim(), selfName);
        if (pName && pName !== 'self') parts.push(`${pName}: ${pType}`);
    }
    return parts.join(', ');
}

/** Return the index of the closing brace matching the opening brace at startIdx. */
function matchingBrace(src: string, startIdx: number): number {
    let depth = 0;
    for (let i = startIdx; i < src.length; i++) {
        if (src[i] === '{') depth++;
        else if (src[i] === '}' && --depth === 0) return i;
    }
    return src.length - 1;
}

/**
 * Parse a Rust lib.rs file and extract public struct fields,
 * impl methods, and top-level free functions.
 */
export function parseRustLib(source: string): NativeModuleInfo {
    const funcs   = new Map<string, LangType>();
    const sigs    = new Map<string, string>();
    const docs    = new Map<string, string>();
    const classes = new Map<string, CppClassInfo>();

    // Strip block comments and line comments (preserving line structure)
    const src = source
        .replace(/\/\*[\s\S]*?\*\//g, ' ')
        .replace(/\/\/[^\n]*/g, '');

    // ── pub struct Name { … } ──────────────────────────────────────────────────
    const structRe = /\bpub\s+struct\s+([A-Za-z_]\w*)\s*\{/g;
    let m: RegExpExecArray | null;
    while ((m = structRe.exec(src)) !== null) {
        const name     = m[1];
        const openIdx  = m.index + m[0].lastIndexOf('{');
        const closeIdx = matchingBrace(src, openIdx);
        const body     = src.slice(openIdx + 1, closeIdx);

        const cls: CppClassInfo = { fields: new Map(), fieldSigs: [], methods: new Map(), methodSigs: [] };
        classes.set(name, cls);

        // pub fieldname: Type,
        const fieldRe = /\bpub\s+([A-Za-z_]\w*)\s*:\s*([^,\n}]+)/g;
        let fm: RegExpExecArray | null;
        while ((fm = fieldRe.exec(body)) !== null) {
            const fname = fm[1];
            const ftype = rsTypeToTl(fm[2].trim(), name);
            cls.fields.set(fname, ftype);
            cls.fieldSigs.push(`${fname}: ${ftype}`);
        }
    }

    // ── impl StructName { … } ─────────────────────────────────────────────────
    const implRe = /\bimpl\s+([A-Za-z_]\w*)\s*\{/g;
    while ((m = implRe.exec(src)) !== null) {
        const implName = m[1];
        const openIdx  = m.index + m[0].lastIndexOf('{');
        const closeIdx = matchingBrace(src, openIdx);
        const body     = src.slice(openIdx + 1, closeIdx);

        const cls = classes.get(implName);
        if (!cls) continue;

        // pub fn name(params) -> RetType { … }
        const fnRe = /\bpub\s+fn\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*(?:->\s*([^{;,\n]+?))?\s*[{;]/g;
        let fm: RegExpExecArray | null;
        while ((fm = fnRe.exec(body)) !== null) {
            const [, fname, paramsRaw, retRs] = fm;
            if (fname === 'new') continue;  // constructor → handled as class call
            const ret    = retRs ? rsTypeToTl(retRs.trim(), implName) : 'None';
            const hvPrms = rsParamsToHv(paramsRaw, implName);
            const msig   = `fn ${fname}(${hvPrms}) -> ${ret}`;
            cls.methods.set(fname, { ret, sig: msig });
            cls.methodSigs.push(msig);
        }
    }

    // ── top-level pub fn (depth 0 only) ───────────────────────────────────────
    const topFnRe = /\bpub\s+fn\s+([A-Za-z_]\w*)\s*\(([^)]*)\)\s*(?:->\s*([^{;,\n]+?))?\s*\{/g;
    while ((m = topFnRe.exec(src)) !== null) {
        // Skip if inside any {} block
        let depth = 0;
        for (let i = 0; i < m.index; i++) {
            if (src[i] === '{') depth++;
            else if (src[i] === '}') depth--;
        }
        if (depth !== 0) continue;

        const [, fname, paramsRaw, retRs] = m;
        const ret    = retRs ? rsTypeToTl(retRs.trim()) : 'None';
        const hvPrms = rsParamsToHv(paramsRaw);
        funcs.set(fname, ret);
        sigs.set(fname, `fn ${fname}(${hvPrms}) -> ${ret}`);
    }

    return { funcs, sigs, docs, classes };
}

// ===== loadNativeModuleInfo =====

export async function loadNativeModuleInfo(
    importKind: string,
    modulePath: string,
    stubName: string | undefined,
    docDir: string
): Promise<NativeModuleInfo> {
    const empty: NativeModuleInfo = { funcs: new Map(), sigs: new Map(), docs: new Map(), classes: new Map() };
    if (importKindOf(importKind) === 'cpp') {
        const candidates: string[] = [];
        if (stubName) {
            candidates.push(path.join(docDir, stubName + '.h'));
        }
        const parts = modulePath.split('.');
        candidates.push(path.join(docDir, ...parts) + '.h');
        candidates.push(path.join(docDir, parts[parts.length - 1] + '.h'));
        for (const hPath of candidates) {
            try {
                const content = await fsPromises.readFile(hPath, 'utf8');
                return parseCHeader(content, path.dirname(hPath));
            } catch { /* try next candidate */ }
        }
        return empty;
    }
    // ── import[rs]: parse Rust source via ar_config.json crates_path ──────────
    if (importKindOf(importKind) === 'rs') {
        const config    = await loadArConfig(docDir);
        const configDir = await findArConfigDir(docDir) ?? docDir;
        const rawPaths  = config?.rust?.crates_path;
        const cratesPaths: string[] = Array.isArray(rawPaths) ? rawPaths : rawPaths ? [rawPaths] : [];
        for (const cratesPath of cratesPaths) {
            const resolved = path.isAbsolute(cratesPath)
                ? cratesPath
                : path.resolve(configDir, cratesPath);
            const libRs = path.join(resolved, modulePath, 'src', 'lib.rs');
            try {
                const content = await fsPromises.readFile(libRs, 'utf8');
                return parseRustLib(content);
            } catch { /* try next candidate */ }
        }
        return empty;
    }
    // ── import[cs-dll] / import[cs-proc]: read ECMA-335 metadata from .NET DLL ─
    if (importKindOf(importKind) === 'cs') {
        const lastName = modulePath.split('.').pop() ?? modulePath;
        const dllName  = lastName + '.dll';
        // Search candidates: sub-path, flat, single-segment package dir, ar_config lib_paths
        const parts = modulePath.split('.');
        const candidates: string[] = [
            path.join(docDir, ...parts) + '.dll',
            path.join(docDir, dllName),
        ];
        if (parts.length === 1) {
            candidates.push(path.join(docDir, lastName, dllName));
        }
        const config = await loadArConfig(docDir);
        const configDir = await findArConfigDir(docDir) ?? docDir;
        const rawCsPaths = config?.csharp?.lib_paths;
        const csLibPaths: string[] = Array.isArray(rawCsPaths) ? rawCsPaths : rawCsPaths ? [rawCsPaths] : [];
        for (const lp of csLibPaths) {
            const resolved = path.isAbsolute(lp) ? lp : path.resolve(configDir, lp);
            candidates.push(path.join(resolved, dllName));
        }
        for (const candidate of candidates) {
            try {
                const buf = await fsPromises.readFile(candidate) as Buffer;
                return parseNetAssembly(buf);
            } catch { /* try next */ }
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
            const content = await fsPromises.readFile(candidate, 'utf8');
            return parseTlStub(content);
        } catch { /* try next candidate */ }
    }
    return empty;
}
