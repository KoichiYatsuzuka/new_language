import * as fs from 'fs';
import * as path from 'path';
import type { LangType } from './builtins';
import { FUNC_DEF_RE } from './builtins';

// ===== C++ / native module support =====

export function importKindOf(keyword: string): 'py' | 'cpp' | 'hv' {
    if (keyword.includes('cpp')) return 'cpp';
    if (keyword === 'import' || keyword.startsWith('import[hv')) return 'hv';
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
}

export interface NativeModuleInfo {
    funcs: Map<string, LangType>;
    sigs: Map<string, string>;
    docs: Map<string, string>;
    classes: Map<string, CppClassInfo>;
}

export function parseCHeader(content: string, dir: string = '', _depth: number = 0): NativeModuleInfo {
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
                classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs });
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
            classes.set(cls.name, { fields: cls.fields, fieldSigs: cls.fieldSigs });
        }
    }

    return classes;
}

export function parseTlStub(content: string): NativeModuleInfo {
    const funcs = new Map<string, LangType>();
    const sigs = new Map<string, string>();
    const docs = new Map<string, string>();
    const lines = content.split('\n');
    for (let i = 0; i < lines.length; i++) {
        const m = lines[i].match(FUNC_DEF_RE);
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
    return { funcs, sigs, docs, classes: new Map() };
}

export function loadNativeModuleInfo(
    importKind: string,
    modulePath: string,
    stubName: string | undefined,
    docDir: string
): NativeModuleInfo {
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
            if (fs.existsSync(hPath)) {
                try { return parseCHeader(fs.readFileSync(hPath, 'utf8'), path.dirname(hPath)); } catch { /* ignore */ }
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
            try { return parseTlStub(fs.readFileSync(candidate, 'utf8')); } catch { /* ignore */ }
        }
    }
    return empty;
}
