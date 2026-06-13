import * as vscode from 'vscode';
import { BUILTIN_RETURN_TYPES, BUILTIN_TYPE_METHODS, BUILTIN_TYPE_NAMES, LANG_KEYWORDS, FUNC_DEF_RE } from './builtins';
import {
    type LangType, type HoverSymbol, type CppClassInfo,
    stripComment, splitComma, inferExprType, resolveSelf,
    cleanTypeAnnotation, extractTupleElemTypes, extractIterElemType, findBodyEndLine,
    getDocstringAfter, selectHoverSymbol,
    collectFuncDefs, collectConstructorTypes, collectTemplateParams, gatherFuncDefLines,
    DECL_RE, STATIC_DECL_RE, HOVER_DECL_RE, CLASS_DEF_RE, NEW_TYPE_RE,
    IMPORT_RE, TUPLE_DECL_RE, FOR_LOOP_RE,
    DocumentAnalysis, builtinStub, initBuiltinStub,
} from './analysis';

export { initBuiltinStub };

// ===== Semantic token legend =====

export const SEMANTIC_TOKENS_LEGEND = new vscode.SemanticTokensLegend(
    ['class', 'type', 'variable'],
    []
);

// ===== Diagnostics helpers =====

const REASSIGN_RE = /^(\s*)([A-Za-z_]\w*)\s*(?:[+\-*\/%&|^]?=(?!=))/;
const PRIMITIVE_TYPES: ReadonlySet<LangType> = new Set(['int', 'uint', 'float', 'str', 'bool', 'None']);

function isTypeCompatible(declared: LangType, inferred: LangType): boolean {
    if (declared === inferred) return true;
    if (inferred === 'unknown' || declared === 'unknown') return true;
    if (declared.includes('[') || inferred.includes('[')) return true;
    if (declared.startsWith('Union') || declared.startsWith('Option') || declared.startsWith('Optional')) return true;
    if (inferred === 'bool' && (declared === 'int' || declared === 'uint')) return true;
    if (declared === 'float' && (inferred === 'int' || inferred === 'uint' || inferred === 'bool')) return true;
    if ((declared === 'int' && inferred === 'uint') || (declared === 'uint' && inferred === 'int')) return true;
    return false;
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
        md.appendCodeblock(`${accessPrefix}${mutability} ${symbol.name}: ${symbol.type ?? 'unknown'}`, 'arrow');
        if (opts?.narrowedFrom) md.appendMarkdown(`\n\n*narrowed from* \`${opts.narrowedFrom}\``);
    } else if (symbol.kind === 'function') {
        const baseSig = symbol.signature ?? `fn ${symbol.name}() -> ${symbol.type ?? 'unknown'}`;
        md.appendCodeblock(symbol.access ? `${symbol.access} ${baseSig}` : baseSig, 'arrow');
    } else if (symbol.kind === 'class') {
        md.appendCodeblock(`class ${symbol.name}`, 'arrow');
    } else if (symbol.kind === 'enum') {
        md.appendCodeblock(`enum ${symbol.name}`, 'arrow');
    } else if (symbol.kind === 'trait') {
        md.appendCodeblock(`trait ${symbol.name}`, 'arrow');
    } else if (symbol.kind === 'module') {
        md.appendCodeblock(`${symbol.originalType ?? 'import[py] ?'} as ${symbol.name}`, 'arrow');
    } else {
        md.appendCodeblock(`new_type ${symbol.name}: ${symbol.originalType ?? 'unknown'}`, 'arrow');
    }

    if (symbol.traits && symbol.traits.length > 0) {
        md.appendMarkdown(`\n\nImplements: ${symbol.traits.map(t => `\`${t}\``).join(', ')}`);
    }
    if (opts?.classTraits && opts.classTraits.length > 0) {
        md.appendMarkdown(`\n\nTraits: ${opts.classTraits.map(t => `\`${t}\``).join(', ')}`);
    }
    if (symbol.doc) md.appendMarkdown(`\n\n---\n\n${symbol.doc}`);
    return md;
}

// ===== Completion member helpers =====

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

    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(NEW_TYPE_RE);
        if (m && m[2] === className) {
            return collectClassMemberItems(document, m[3].trim(), _visited);
        }
    }

    let classLine = -1;
    let classIndent = 0;
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(CLASS_DEF_RE);
        if (m && m[3] === className) { classLine = i; classIndent = (m[1] ?? '').length; break; }
    }
    if (classLine < 0) return [];

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

        if (lineIndent === memberIndent) {
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

            const funcMatch = stripped.match(/^(\s*)(fn|gen)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:\s*$/);
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

        const selfAttrMatch = stripped.match(/\bself\.([A-Za-z_]\w*)\s*(?:[+\-*\/%&|^]?=(?!=))/);
        if (selfAttrMatch && !seen.has(selfAttrMatch[1])) {
            seen.add(selfAttrMatch[1]);
            items.push(new vscode.CompletionItem(selfAttrMatch[1], vscode.CompletionItemKind.Field));
        }
    }

    return items;
}

function findClassMember(
    document: vscode.TextDocument,
    className: string,
    memberName: string,
    _visited: Set<string> = new Set()
): HoverSymbol | undefined {
    if (_visited.has(className)) return undefined;
    _visited.add(className);

    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(NEW_TYPE_RE);
        if (m && m[2] === className) {
            return findClassMember(document, m[3].trim(), memberName, _visited);
        }
    }

    let classLine = -1;
    let classIndent = 0;
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const m = stripped.match(CLASS_DEF_RE);
        if (m && m[3] === className) { classLine = i; classIndent = (m[1] ?? '').length; break; }
    }
    if (classLine < 0) return undefined;

    let memberIndent = -1;
    for (let i = classLine + 1; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        if (!raw.trim()) continue;
        const ind = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        if (ind <= classIndent) break;
        memberIndent = ind;
        break;
    }
    if (memberIndent < 0) return undefined;

    for (let i = classLine + 1; i < document.lineCount; i++) {
        const rawLine = document.lineAt(i).text;
        const stripped = stripComment(rawLine);
        if (!stripped.trim()) continue;

        const lineIndent = (rawLine.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= classIndent) break;

        if (lineIndent === memberIndent) {
            const funcMatch = stripped.match(/^(\s*)(fn|gen)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:\s*$/);
            if (funcMatch) {
                const [, indentStr, kw, name, params, retAnnotation] = funcMatch;
                if (name === memberName) {
                    const returnType = cleanTypeAnnotation(retAnnotation) ?? 'unknown';
                    const cleanParams = params
                        .replace(/^\s*(?:let\s+|mut\s+)?self\s*,\s*/, '')
                        .replace(/^\s*(?:let\s+|mut\s+)?self\s*$/, '');
                    return {
                        name,
                        kind: 'function',
                        line: i,
                        type: returnType,
                        signature: `${kw} ${name}(${cleanParams}) -> ${returnType}`,
                        doc: getDocstringAfter(document, i, (indentStr ?? '').length),
                    };
                }
                continue;
            }

            const fieldMatch = stripped.match(HOVER_DECL_RE);
            if (fieldMatch) {
                const [, , mutability, name, annotation] = fieldMatch;
                if (name === memberName) {
                    return {
                        name,
                        kind: 'variable',
                        line: i,
                        mutability: mutability ?? 'let',
                        type: annotation?.trim() ?? 'unknown',
                    };
                }
            }
        }

        const selfAttrMatch = stripped.match(/\bself\.([A-Za-z_]\w*)\s*(?:[+\-*\/%&|^]?=(?!=))/);
        if (selfAttrMatch && selfAttrMatch[1] === memberName) {
            return { name: memberName, kind: 'variable', line: i, mutability: 'mut', type: 'unknown' };
        }
    }

    return undefined;
}

async function resolveMemberItems(
    document: vscode.TextDocument,
    position: vscode.Position,
    objName: string
): Promise<vscode.CompletionItem[]> {
    if (objName === 'self') {
        const cls = findEnclosingClass(document, position.line);
        return cls ? collectClassMemberItems(document, cls) : [];
    }

    const a = await DocumentAnalysis.for(document);
    const sym = selectHoverSymbol(a.symbols, objName, position.line);
    if (!sym) return [];

    if (sym.kind === 'module') {
        const funcs = a.importFuncTypes.get(objName);
        const sigsMap = a.importFuncSigs.get(objName);
        if (!funcs) return [];
        return [...funcs.entries()].map(([name, retType]) => {
            const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
            item.detail = sigsMap?.get(name) ?? `→ ${retType}`;
            return item;
        });
    }

    if (sym.type) {
        const cppCls = (a.cppClasses as Map<string, CppClassInfo>).get(sym.type);
        if (cppCls) {
            const items: vscode.CompletionItem[] = [];
            for (const [fieldName, fieldType] of cppCls.fields) {
                const item = new vscode.CompletionItem(fieldName, vscode.CompletionItemKind.Field);
                item.detail = `: ${fieldType}`;
                items.push(item);
            }
            for (const [methodName, info] of cppCls.methods) {
                const item = new vscode.CompletionItem(methodName, vscode.CompletionItemKind.Method);
                item.detail = info.sig;
                items.push(item);
            }
            return items;
        }
        const baseType = sym.type.replace(/\[.*$/, '');
        const builtinMethods = BUILTIN_TYPE_METHODS[baseType];
        if (builtinMethods) {
            return Object.entries(builtinMethods).map(([methodName, info]) => {
                const item = new vscode.CompletionItem(methodName, vscode.CompletionItemKind.Method);
                item.detail = info.sig;
                return item;
            });
        }
        return collectClassMemberItems(document, sym.type);
    }

    return [];
}

// ===== Semantic token helpers =====

function stringLiteralRanges(line: string, codeEnd: number): Array<[number, number]> {
    const ranges: Array<[number, number]> = [];
    let i = 0;
    while (i < codeEnd) {
        const c = line[i];
        if (c === '"' || c === "'") {
            const q = c;
            const triple = line.startsWith(q + q + q, i);
            const start = i;
            i += triple ? 3 : 1;
            while (i < codeEnd) {
                if (line[i] === '\\') { i += 2; continue; }
                if (triple ? line.startsWith(q + q + q, i) : line[i] === q) { i += triple ? 3 : 1; break; }
                i++;
            }
            ranges.push([start, i]);
        } else {
            i++;
        }
    }
    return ranges;
}

function escapeRegex(s: string): string {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function parseTypeAt(line: string, pos: number, out: Set<number>): number {
    while (pos < line.length && (line[pos] === ' ' || line[pos] === '\t')) pos++;
    if (pos >= line.length || !/[A-Za-z_]/.test(line[pos])) return pos;
    out.add(pos);
    while (pos < line.length && /\w/.test(line[pos])) pos++;
    while (pos < line.length && (line[pos] === ' ' || line[pos] === '\t')) pos++;
    if (pos < line.length && line[pos] === '[') {
        pos++;
        while (pos < line.length && line[pos] !== ']') {
            if (',\t '.includes(line[pos])) { pos++; continue; }
            const newPos = parseTypeAt(line, pos, out);
            pos = newPos > pos ? newPos : pos + 1;  // always advance to prevent infinite loop
        }
        if (pos < line.length) pos++;
    }
    return pos;
}

function typeAnnotationPositions(rawLine: string, strRanges: Array<[number, number]>): Set<number> {
    const out = new Set<number>();
    const src = stripComment(rawLine);
    const inString = (pos: number): boolean => strRanges.some(([s, e]) => pos >= s && pos < e);
    const colonRe = /[A-Za-z_]\w*[ \t]*:(?!=)(?=[ \t]*[A-Za-z_])/g;
    let m: RegExpExecArray | null;
    while ((m = colonRe.exec(src)) !== null) {
        if (!inString(m.index)) parseTypeAt(src, m.index + m[0].length, out);
    }
    const arrowRe = /->[ \t]*/g;
    while ((m = arrowRe.exec(src)) !== null) {
        if (!inString(m.index)) parseTypeAt(src, m.index + m[0].length, out);
    }
    const isNotRe = /\bis[ \t]+not[ \t]+/g;
    while ((m = isNotRe.exec(src)) !== null) {
        if (!inString(m.index)) parseTypeAt(src, m.index + m[0].length, out);
    }
    const isRe = /\bis[ \t]+(?!not\b)/g;
    while ((m = isRe.exec(src)) !== null) {
        if (!inString(m.index)) parseTypeAt(src, m.index + m[0].length, out);
    }
    return out;
}

// ===== Providers =====

export async function provideHover(
    document: vscode.TextDocument,
    position: vscode.Position
): Promise<vscode.Hover | undefined> {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_]\w*/);
    if (!range) return undefined;

    const name = document.getText(range);
    const lineText = document.lineAt(position.line).text;
    const prefixStr = lineText.substring(0, range.start.character);
    const dotAccess = prefixStr.match(/([A-Za-z_]\w*)\.$/);

    if (dotAccess) {
        const objName = dotAccess[1];
        const a = await DocumentAnalysis.for(document);

        const retType = a.importFuncTypes.get(objName)?.get(name);
        if (retType !== undefined) {
            const md = new vscode.MarkdownString(undefined, true);
            const sig = a.importFuncSigs.get(objName)?.get(name) ?? `fn ${name}() -> ${retType}`;
            md.appendCodeblock(sig, 'arrow');
            return new vscode.Hover(md, range);
        }

        const objSym = selectHoverSymbol(a.symbols, objName, position.line);
        if (objSym?.type) {
            const cppCls = (a.cppClasses as Map<string, CppClassInfo>).get(objSym.type);
            if (cppCls) {
                const fieldType = cppCls.fields.get(name);
                if (fieldType !== undefined) {
                    const md = new vscode.MarkdownString(undefined, true);
                    md.appendCodeblock(`${name}: ${fieldType}`, 'arrow');
                    md.appendMarkdown(`\n\n*field of* \`${objSym.type}\``);
                    return new vscode.Hover(md, range);
                }
                const methodInfo = cppCls.methods.get(name);
                if (methodInfo !== undefined) {
                    const md = new vscode.MarkdownString(undefined, true);
                    md.appendCodeblock(methodInfo.sig, 'arrow');
                    md.appendMarkdown(`\n\n*method of* \`${objSym.type}\``);
                    return new vscode.Hover(md, range);
                }
            }
            const builtinMethod = BUILTIN_TYPE_METHODS[objSym.type]?.[name];
            if (builtinMethod) {
                const md = new vscode.MarkdownString(undefined, true);
                md.appendCodeblock(builtinMethod.sig, 'arrow');
                return new vscode.Hover(md, range);
            }
        }

        const hvClassName = objName === 'self'
            ? findEnclosingClass(document, position.line)
            : objSym?.type;
        if (hvClassName) {
            const memberSym = findClassMember(document, hvClassName, name);
            if (memberSym) return new vscode.Hover(renderHover(memberSym), range);
        }
        return undefined;
    }

    // Self type hover — resolves to the enclosing class
    if (name === 'Self') {
        const cls = findEnclosingClass(document, position.line);
        if (cls) {
            const md = new vscode.MarkdownString(undefined, true);
            md.appendCodeblock(`type Self = ${cls}`, 'arrow');
            return new vscode.Hover(md, range);
        }
        return undefined;
    }

    // Imported class type hover (C++/Rust)
    {
        const a = await DocumentAnalysis.for(document);
        const cppCls = (a.cppClasses as Map<string, CppClassInfo>).get(name);
        if (cppCls) {
            const md = new vscode.MarkdownString(undefined, true);
            const allSigs = [
                ...cppCls.fieldSigs,
                ...cppCls.methodSigs,
            ];
            const body = allSigs.length > 0
                ? allSigs.map(s => `    ${s}`).join('\n')
                : '    (no public members)';
            md.appendCodeblock(`class ${name} {\n${body}\n}`, 'cpp');
            return new vscode.Hover(md, range);
        }
    }

    const a = await DocumentAnalysis.for(document);
    const symbol = selectHoverSymbol(a.symbols, name, position.line);
    if (!symbol) {
        const builtinSig = builtinStub.sigs.get(name);
        if (builtinSig) {
            const md = new vscode.MarkdownString(undefined, true);
            md.appendCodeblock(builtinSig, 'arrow');
            const doc = builtinStub.docs.get(name);
            if (doc) md.appendMarkdown(`\n\n${doc}`);
            return new vscode.Hover(md, range);
        }
        return undefined;
    }

    const freezeLine = a.freezeLines.get(name);
    const isFrozen = freezeLine !== undefined && position.line >= freezeLine && symbol.mutability === 'mut';

    const override = a.scopeOverrides.find(o =>
        o.varName === name && position.line >= o.startLine && position.line < o.endLine
    );

    const effectiveType = override?.narrowedType ?? symbol.type;
    const rawClassTraits = effectiveType ? a.classTraitsMap.get(effectiveType) : undefined;
    const classTraits = rawClassTraits && rawClassTraits.length > 0 ? rawClassTraits : undefined;

    const displaySymbol = override ? { ...symbol, type: override.narrowedType } : symbol;
    return new vscode.Hover(renderHover(displaySymbol, {
        narrowedFrom: override ? symbol.type : undefined,
        isFrozen,
        classTraits,
    }), range);
}

export async function provideInlayHints(
    document: vscode.TextDocument,
    _range: vscode.Range
): Promise<vscode.InlayHint[]> {
    const hints: vscode.InlayHint[] = [];
    const a = await DocumentAnalysis.for(document);

    // Inlay hints on function definition lines (only when no annotation)
    for (const def of a.funcDefs) {
        if (def.annotation !== undefined) continue;
        const returnType = a.funcEnv.get(def.name)!;
        const sigLine = def.sigEndLine ?? def.defLine;
        const rawLine = document.lineAt(sigLine).text;
        const rparenPos = rawLine.lastIndexOf(')');
        if (rparenPos < 0) continue;
        const pos = new vscode.Position(sigLine, rparenPos + 1);
        hints.push(new vscode.InlayHint(pos, ` -> ${returnType}`, vscode.InlayHintKind.Type));
    }

    // Inlay hints on variable declarations — requires a sequential per-line env
    const env = new Map<string, LangType>();
    let classContext: { name: string; indent: number } | undefined;
    let selfType: string | undefined;

    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const rawLine = document.lineAt(lineIdx).text;
        const line = stripComment(rawLine);

        if (line.match(IMPORT_RE)) continue;

        if (line.trim()) {
            const lineIndent = (rawLine.match(/^(\s*)/)?.[1] ?? '').length;
            if (classContext && lineIndent <= classContext.indent) {
                classContext = undefined;
                selfType = undefined;
            }
            const classM = line.match(CLASS_DEF_RE);
            if (classM) {
                classContext = { name: classM[3], indent: (classM[1] ?? '').length };
                selfType = undefined;
                continue;
            }
            const funcDefLines = gatherFuncDefLines(document, lineIdx);
            const funcM = funcDefLines?.fullLine.match(FUNC_DEF_RE);
            if (funcM) {
                selfType = classContext?.name;
                const params = funcM[4];
                for (const p of splitComma(params)) {
                    const pm = p.trim().match(/^(?:(?:let|mut)\s+)?([A-Za-z_]\w*)\s*(?::\s*(.+))?$/);
                    if (pm && pm[1] !== 'self' && pm[2]?.trim()) {
                        const pt = pm[2].trim();
                        env.set(pm[1], pt === 'Self' && selfType ? selfType : pt);
                    }
                }
                lineIdx = funcDefLines!.lastLine;
                continue;
            }
        }

        const staticMatch = line.match(STATIC_DECL_RE);
        if (staticMatch) {
            const [, indent, name, annotation, rhs] = staticMatch;
            const type = cleanTypeAnnotation(annotation)
                ?? (rhs ? inferExprType(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType) : 'unknown');
            env.set(name, type);
            if (!annotation) {
                const nameStart = rawLine.indexOf(name, indent.length + 'static mut '.length);
                if (nameStart >= 0) {
                    const hint = new vscode.InlayHint(new vscode.Position(lineIdx, nameStart + name.length), `: ${type}`, vscode.InlayHintKind.Type);
                    hint.paddingLeft = true;
                    hints.push(hint);
                }
            }
            continue;
        }

        const tupleM = line.match(TUPLE_DECL_RE);
        if (tupleM) {
            const [, indent, keyword, names, rhs] = tupleM;
            const rhsType = inferExprType(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType);
            const nameList = names.split(',').map(n => n.trim()).filter(Boolean);
            const elemTypes = extractTupleElemTypes(rhsType, nameList.length);
            let searchFrom = indent.length + keyword.length;
            for (let idx = 0; idx < nameList.length; idx++) {
                const varName = nameList[idx];
                const elemType = elemTypes[idx] ?? 'unknown';
                env.set(varName, elemType);
                const nameStart = rawLine.indexOf(varName, searchFrom);
                if (nameStart >= 0) {
                    const hint = new vscode.InlayHint(new vscode.Position(lineIdx, nameStart + varName.length), `: ${elemType}`, vscode.InlayHintKind.Type);
                    hint.paddingLeft = true;
                    hints.push(hint);
                    searchFrom = nameStart + varName.length;
                }
            }
            continue;
        }

        const forLoopM = line.match(FOR_LOOP_RE);
        if (forLoopM) {
            const [, indent, rawTargets, rawIter] = forLoopM;
            const iterExpr = rawIter.replace(/\s*(?:->[^:]+)?\s*:\s*$/, '').trim();
            let elemType: LangType;
            if (/^range\s*\(/.test(iterExpr)) {
                elemType = 'int';
            } else if (/^enumerate\s*\(/.test(iterExpr)) {
                const enumArgsM = iterExpr.match(/^enumerate\s*\((.+)\)\s*$/);
                if (enumArgsM) {
                    const innerType = inferExprType(splitComma(enumArgsM[1])[0].trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType);
                    elemType = `tuple[int, ${extractIterElemType(innerType)}]`;
                } else {
                    elemType = 'tuple[int, unknown]';
                }
            } else if (/^zip\s*\(/.test(iterExpr)) {
                const zipArgsM = iterExpr.match(/^zip\s*\((.+)\)\s*$/);
                if (zipArgsM) {
                    const elems = splitComma(zipArgsM[1]).map(arg => extractIterElemType(inferExprType(arg.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType)));
                    elemType = `tuple[${elems.join(', ')}]`;
                } else {
                    elemType = 'unknown';
                }
            } else {
                elemType = extractIterElemType(inferExprType(iterExpr, env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType));
            }
            const targetList = rawTargets.split(',').map(t => t.trim()).filter(Boolean);
            let searchFrom = (indent ?? '').length + 'for '.length;
            if (targetList.length === 1) {
                const varName = targetList[0];
                env.set(varName, elemType);
                if (elemType !== 'unknown') {
                    const nameStart = rawLine.indexOf(varName, searchFrom);
                    if (nameStart >= 0) {
                        const hint = new vscode.InlayHint(new vscode.Position(lineIdx, nameStart + varName.length), `: ${elemType}`, vscode.InlayHintKind.Type);
                        hint.paddingLeft = true;
                        hints.push(hint);
                    }
                }
            } else {
                const subTypes = extractTupleElemTypes(elemType, targetList.length);
                for (let idx = 0; idx < targetList.length; idx++) {
                    const varName = targetList[idx];
                    const varType = subTypes[idx] ?? 'unknown';
                    env.set(varName, varType);
                    if (varType !== 'unknown') {
                        const nameStart = rawLine.indexOf(varName, searchFrom);
                        if (nameStart >= 0) {
                            const hint = new vscode.InlayHint(new vscode.Position(lineIdx, nameStart + varName.length), `: ${varType}`, vscode.InlayHintKind.Type);
                            hint.paddingLeft = true;
                            hints.push(hint);
                            searchFrom = nameStart + varName.length;
                        }
                    }
                }
            }
            continue;
        }

        const declM = line.match(HOVER_DECL_RE);
        if (!declM) continue;

        const [, indent, keyword, name, annotation, rhs] = declM;
        const type = cleanTypeAnnotation(annotation)
            ?? (rhs ? inferExprType(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType) : 'unknown');
        env.set(name, type);

        if (!annotation && rhs) {
            const nameStart = rawLine.indexOf(name, (indent ?? '').length + keyword.length);
            if (nameStart >= 0) {
                const hint = new vscode.InlayHint(new vscode.Position(lineIdx, nameStart + name.length), `: ${type}`, vscode.InlayHintKind.Type);
                hint.paddingLeft = true;
                hints.push(hint);
            }
        }
    }

    return hints;
}

export async function provideCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position
): Promise<vscode.CompletionItem[]> {
    const prefix = document.lineAt(position.line).text.substring(0, position.character);

    const dotMatch = prefix.match(/([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/);
    if (dotMatch) {
        return resolveMemberItems(document, position, dotMatch[1]);
    }

    const items: vscode.CompletionItem[] = [];
    const seen = new Set<string>();

    const a = await DocumentAnalysis.for(document);
    for (const sym of a.symbols) {
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

    for (const [name, sig] of builtinStub.sigs) {
        if (seen.has(name)) continue;
        seen.add(name);
        const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
        item.detail = sig;
        const doc = builtinStub.docs.get(name);
        if (doc) item.documentation = new vscode.MarkdownString(doc);
        items.push(item);
    }
    for (const [name, retType] of Object.entries(BUILTIN_RETURN_TYPES)) {
        if (seen.has(name)) continue;
        seen.add(name);
        const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
        item.detail = `→ ${retType}`;
        items.push(item);
    }

    for (const kw of LANG_KEYWORDS) {
        if (seen.has(kw)) continue;
        seen.add(kw);
        items.push(new vscode.CompletionItem(kw, vscode.CompletionItemKind.Keyword));
    }

    return items;
}

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

        while (stack.length > 0 && indent <= stack[stack.length - 1].indent) {
            stack.pop();
        }

        let name = '';
        let detail = '';
        let kind = vscode.SymbolKind.Variable;
        let isContainer = false;

        const funcMatch = stripped.match(/^(\s*)(fn|gen)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:\s*$/);
        if (funcMatch) {
            const [, , , funcName, , retType] = funcMatch;
            name = funcName; kind = vscode.SymbolKind.Function; detail = retType?.trim() ?? ''; isContainer = true;
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
                    name = ntName; kind = vscode.SymbolKind.TypeParameter; detail = ntType.trim();
                } else {
                    const importMatch = stripped.match(IMPORT_RE);
                    if (importMatch) {
                        const [, importKind, modulePath, , , explicitAlias] = importMatch;
                        const alias = explicitAlias ?? (modulePath.split('.').pop() ?? modulePath);
                        name = alias; kind = vscode.SymbolKind.Module; detail = `${importKind} ${modulePath}`;
                    } else {
                        const staticMatch = stripped.match(STATIC_DECL_RE);
                        if (staticMatch) {
                            const [, , varName, annot] = staticMatch;
                            name = varName; kind = vscode.SymbolKind.Variable; detail = annot?.trim() ?? '';
                        } else if (indent === 0) {
                            const declMatch = stripped.match(HOVER_DECL_RE);
                            if (declMatch) {
                                const [, , , varName, annot] = declMatch;
                                name = varName; kind = vscode.SymbolKind.Variable; detail = annot?.trim() ?? '';
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

        if (isContainer) stack.push({ sym, indent });
    }

    return result;
}

export async function provideSignatureHelp(
    document: vscode.TextDocument,
    position: vscode.Position
): Promise<vscode.SignatureHelp | undefined> {
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

    const a = await DocumentAnalysis.for(document);
    const funcSym = a.symbols.find(s => s.name === funcName && s.kind === 'function');
    let sigStr: string | undefined;
    let sigDoc: vscode.MarkdownString | undefined;

    if (funcSym?.signature) {
        sigStr = funcSym.signature;
        if (funcSym.doc) sigDoc = new vscode.MarkdownString(funcSym.doc);
    } else {
        const builtinSig = builtinStub.sigs.get(funcName);
        if (builtinSig) {
            sigStr = builtinSig;
            const doc = builtinStub.docs.get(funcName);
            if (doc) sigDoc = new vscode.MarkdownString(doc);
        }
    }

    if (!sigStr) return undefined;

    const sigInfo = new vscode.SignatureInformation(sigStr, sigDoc);
    const paramsMatch = sigStr.match(/\(([^)]*)\)/);
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

export async function provideDefinition(
    document: vscode.TextDocument,
    position: vscode.Position
): Promise<vscode.Location | undefined> {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_]\w*/);
    if (!range) return undefined;

    const name = document.getText(range);
    const a = await DocumentAnalysis.for(document);
    const symbol = selectHoverSymbol(a.symbols, name, position.line);
    if (!symbol) return undefined;

    const targetText = document.lineAt(symbol.line).text;
    const nameIdx = targetText.indexOf(symbol.name);
    const targetRange = nameIdx >= 0
        ? new vscode.Range(symbol.line, nameIdx, symbol.line, nameIdx + symbol.name.length)
        : document.lineAt(symbol.line).range;

    return new vscode.Location(document.uri, targetRange);
}

export async function provideDiagnostics(document: vscode.TextDocument): Promise<vscode.Diagnostic[]> {
    const diagnostics: vscode.Diagnostic[] = [];
    const a = await DocumentAnalysis.for(document);
    const env = new Map<string, LangType>();

    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = stripComment(raw);
        if (!stripped.trim()) continue;

        if (stripped.match(/^(\s*)(fn|gen)\s+/) || stripped.match(CLASS_DEF_RE) || stripped.match(IMPORT_RE)) continue;

        const staticM = stripped.match(STATIC_DECL_RE);
        if (staticM) {
            const [, , name, annotation, rhs] = staticM;
            const type = cleanTypeAnnotation(annotation)
                ?? (rhs ? inferExprType(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams) : 'unknown');
            env.set(name, type);
            continue;
        }

        const tupleM = stripped.match(TUPLE_DECL_RE);
        if (tupleM) {
            const [, , , names, rhs] = tupleM;
            const rhsType = inferExprType(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams);
            const nameList = names.split(',').map(n => n.trim()).filter(Boolean);
            const elemTypes = extractTupleElemTypes(rhsType, nameList.length);
            for (let idx = 0; idx < nameList.length; idx++) env.set(nameList[idx], elemTypes[idx] ?? 'unknown');
            continue;
        }

        const declM = stripped.match(HOVER_DECL_RE);
        if (declM) {
            const [, indent, keyword, name, annotation, rhs] = declM;
            const declaredType = cleanTypeAnnotation(annotation);
            if (declaredType && rhs) {
                const inferredType = inferExprType(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams);
                env.set(name, declaredType);
                if (
                    PRIMITIVE_TYPES.has(declaredType) &&
                    PRIMITIVE_TYPES.has(inferredType) &&
                    !isTypeCompatible(declaredType, inferredType)
                ) {
                    const indentLen = (indent ?? '').length;
                    const colonIdx = raw.indexOf(':', indentLen + (keyword ?? '').length + name.length);
                    if (colonIdx >= 0) {
                        const annotStart = raw.indexOf(declaredType, colonIdx + 1);
                        if (annotStart >= 0) {
                            diagnostics.push(new vscode.Diagnostic(
                                new vscode.Range(i, annotStart, i, annotStart + declaredType.length),
                                `Type mismatch: declared '${declaredType}', but right-hand side has type '${inferredType}'`,
                                vscode.DiagnosticSeverity.Error
                            ));
                        }
                    }
                }
            } else {
                env.set(name, declaredType ?? (rhs
                    ? inferExprType(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams)
                    : 'unknown'));
            }
            continue;
        }

        const reassignM = stripped.match(REASSIGN_RE);
        if (!reassignM) continue;
        const [, indent, varName] = reassignM;

        const sym = selectHoverSymbol(a.symbols, varName, i);
        if (!sym || sym.kind !== 'variable') continue;

        const nameStart = raw.indexOf(varName, (indent ?? '').length);
        if (nameStart < 0) continue;
        const diagRange = new vscode.Range(i, nameStart, i, nameStart + varName.length);

        if (sym.mutability === 'let' || sym.mutability === 'const') {
            diagnostics.push(new vscode.Diagnostic(
                diagRange,
                `Cannot assign to '${sym.mutability}' variable '${varName}'`,
                vscode.DiagnosticSeverity.Error
            ));
        } else if (sym.mutability === 'mut') {
            const freezeLine = a.freezeLines.get(varName);
            if (freezeLine !== undefined && i > freezeLine) {
                diagnostics.push(new vscode.Diagnostic(
                    diagRange,
                    `Cannot assign to frozen variable '${varName}'`,
                    vscode.DiagnosticSeverity.Error
                ));
            }
        }
    }

    // Check 'is not' applied to a non-Union/Option variable (StaticTypeError: IsNotOnNonUnion)
    const IS_NOT_RE = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+not\s+([A-Za-z_]\w*)\s*:/;
    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = stripComment(raw);
        const m = stripped.match(IS_NOT_RE);
        if (!m) continue;
        const varName = m[2];
        const sym = selectHoverSymbol(a.symbols, varName, i);
        if (!sym?.type) continue;
        const t = sym.type;
        if (t === 'unknown' || t === 'Any' ||
            t.startsWith('Union[') || t.startsWith('Option[') || t.startsWith('Optional[')) continue;
        const indent = (m[1] ?? '').length;
        const keywordLen = stripped.slice(indent).startsWith('elif') ? 4 : 2;
        const nameStart = raw.indexOf(varName, indent + keywordLen);
        if (nameStart < 0) continue;
        diagnostics.push(new vscode.Diagnostic(
            new vscode.Range(i, nameStart, i, nameStart + varName.length),
            `'is not' requires a Union or Option type, but '${varName}' has type '${t}'`,
            vscode.DiagnosticSeverity.Error
        ));
    }

    return diagnostics;
}

export async function provideDocumentSemanticTokens(document: vscode.TextDocument): Promise<vscode.SemanticTokens> {
    const builder = new vscode.SemanticTokensBuilder(SEMANTIC_TOKENS_LEGEND);
    const a = await DocumentAnalysis.for(document);

    const userTypes = new Set<string>();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = stripComment(document.lineAt(i).text);
        const classM = stripped.match(CLASS_DEF_RE);
        if (classM) { userTypes.add(classM[3]); continue; }
        const ntM = stripped.match(NEW_TYPE_RE);
        if (ntM) userTypes.add(ntM[2]);
    }

    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const lineText = document.lineAt(lineIdx).text;
        const commentStart = stripComment(lineText).length;
        const strRanges = stringLiteralRanges(lineText, commentStart);
        const isLiveCode = (col: number): boolean =>
            col < commentStart && !strRanges.some(([s, e]) => col >= s && col < e);

        const typePositions = typeAnnotationPositions(lineText, strRanges);

        interface Hit { col: number; len: number; tokenType: number; }
        const hits: Hit[] = [];

        for (const name of BUILTIN_TYPE_NAMES) {
            const re = new RegExp(`\\b${name}\\b`, 'g');
            let m: RegExpExecArray | null;
            while ((m = re.exec(lineText)) !== null) {
                if (!isLiveCode(m.index)) continue;
                if (typePositions.has(m.index)) {
                    hits.push({ col: m.index, len: name.length, tokenType: 1 });
                } else {
                    let j = m.index + name.length;
                    while (j < lineText.length && (lineText[j] === ' ' || lineText[j] === '\t')) j++;
                    const next = lineText[j];
                    if (next !== '(' && next !== '[') {
                        hits.push({ col: m.index, len: name.length, tokenType: 2 });
                    }
                }
            }
        }

        for (const alias of a.importAliases) {
            const re = new RegExp(`\\b${escapeRegex(alias)}\\b`, 'g');
            let m: RegExpExecArray | null;
            while ((m = re.exec(lineText)) !== null) {
                if (!isLiveCode(m.index)) continue;
                hits.push({ col: m.index, len: alias.length, tokenType: 0 });
            }
        }

        for (const name of userTypes) {
            const re = new RegExp(`\\b${escapeRegex(name)}\\b`, 'g');
            let m: RegExpExecArray | null;
            while ((m = re.exec(lineText)) !== null) {
                if (!isLiveCode(m.index)) continue;
                hits.push({ col: m.index, len: name.length, tokenType: 0 });
            }
        }

        hits.sort((a, b) => a.col !== b.col ? a.col - b.col : a.tokenType - b.tokenType);

        let prevEnd = 0;
        for (const { col, len, tokenType } of hits) {
            if (col < prevEnd) continue;
            builder.push(lineIdx, col, len, tokenType, 0);
            prevEnd = col + len;
        }
    }

    return builder.build();
}
