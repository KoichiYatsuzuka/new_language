"use strict";
/**
 * type_infer.ts — VS Code language-feature providers for the Arrow language.
 *
 * Responsibilities:
 * - Register and implement VS Code providers (hover, inlay hints, semantic tokens,
 *   completions, document symbols, signature help, go-to-definition, diagnostics)
 * - Drive `DocumentAnalysis` (from analysis.ts) to get symbol/type information
 * - Render type information into VS Code UI objects (MarkdownString, InlayHint, …)
 *
 * Provider functions exported to `extension.ts`:
 *   provideHover, provideInlayHints, provideDocumentSemanticTokens,
 *   provideCompletionItems, provideDocumentSymbols, provideSignatureHelp,
 *   provideDefinition, provideDiagnostics
 */
Object.defineProperty(exports, "__esModule", { value: true });
exports.provideDocumentSemanticTokens = exports.provideDiagnostics = exports.provideDefinition = exports.provideSignatureHelp = exports.provideDocumentSymbols = exports.provideCompletionItems = exports.provideInlayHints = exports.provideHover = exports.SEMANTIC_TOKENS_LEGEND = exports.initBuiltinStub = void 0;
const vscode = require("vscode");
const fs_1 = require("fs");
const path = require("path");
const builtins_1 = require("./builtins");
const analysis_1 = require("./analysis");
Object.defineProperty(exports, "initBuiltinStub", { enumerable: true, get: function () { return analysis_1.initBuiltinStub; } });
const native_module_1 = require("./native_module");
// ===== Semantic token legend =====
exports.SEMANTIC_TOKENS_LEGEND = new vscode.SemanticTokensLegend(['class', 'type', 'variable'], []);
// ===== Shared rendering helpers =====
/**
 * Build an inlay type-hint with left padding.
 * Used for variable declarations, for-loop targets, and tuple unpacking.
 */
function makeInlayHint(line, col, type) {
    const hint = new vscode.InlayHint(new vscode.Position(line, col), `: ${type}`, vscode.InlayHintKind.Type);
    hint.paddingLeft = true;
    return hint;
}
// ===== Diagnostics helpers =====
const REASSIGN_RE = /^(\s*)([A-Za-z_]\w*)\s*(?:[+\-*\/%&|^]?=(?!=))/;
const PRIMITIVE_TYPES = new Set(['int', 'uint', 'float', 'str', 'bool', 'None']);
function isTypeCompatible(declared, inferred) {
    if (declared === inferred)
        return true;
    if (inferred === 'unknown' || declared === 'unknown')
        return true;
    if (declared.includes('[') || inferred.includes('['))
        return true;
    if (declared.startsWith('Union') || declared.startsWith('Option') || declared.startsWith('Optional'))
        return true;
    if (inferred === 'bool' && (declared === 'int' || declared === 'uint'))
        return true;
    if (declared === 'float' && (inferred === 'int' || inferred === 'uint' || inferred === 'bool'))
        return true;
    if ((declared === 'int' && inferred === 'uint') || (declared === 'uint' && inferred === 'int'))
        return true;
    return false;
}
// ===== Hover rendering =====
function renderExternalClassHover(name, cppCls) {
    const md = new vscode.MarkdownString(undefined, true);
    md.isTrusted = false;
    const basesStr = cppCls.bases.length > 0 ? `(${cppCls.bases.join(', ')})` : '';
    const header = basesStr ? `class ${name}${basesStr}:` : `class ${name}:`;
    const allSigs = [...cppCls.fieldSigs, ...cppCls.methodSigs];
    const body = allSigs.length > 0
        ? allSigs.map(s => `    ${s}`).join('\n')
        : '    ...';
    md.appendCodeblock(`${header}\n${body}`, 'arrow');
    if (cppCls.classDocs)
        md.appendMarkdown(`\n\n${cppCls.classDocs}`);
    return md;
}
function renderHover(symbol, opts) {
    const md = new vscode.MarkdownString(undefined, true);
    md.isTrusted = false;
    const mutability = opts?.isFrozen ? 'frozen' : (symbol.mutability ?? 'value');
    if (symbol.kind === 'variable') {
        const accessPrefix = symbol.access ? `${symbol.access} ` : '';
        md.appendCodeblock(`${accessPrefix}${mutability} ${symbol.name}: ${symbol.type ?? 'unknown'}`, 'arrow');
        if (opts?.narrowedFrom)
            md.appendMarkdown(`\n\n*narrowed from* \`${opts.narrowedFrom}\``);
    }
    else if (symbol.kind === 'function') {
        const baseSig = symbol.signature ?? `fn ${symbol.name}() -> ${symbol.type ?? 'unknown'}`;
        md.appendCodeblock(symbol.access ? `${symbol.access} ${baseSig}` : baseSig, 'arrow');
    }
    else if (symbol.kind === 'class') {
        md.appendCodeblock(`class ${symbol.name}`, 'arrow');
    }
    else if (symbol.kind === 'enum') {
        md.appendCodeblock(`enum ${symbol.name}`, 'arrow');
    }
    else if (symbol.kind === 'trait') {
        md.appendCodeblock(`trait ${symbol.name}`, 'arrow');
    }
    else if (symbol.kind === 'module') {
        md.appendCodeblock(`${symbol.originalType ?? 'import[py] ?'} as ${symbol.name}`, 'arrow');
    }
    else {
        md.appendCodeblock(`new_type ${symbol.name}: ${symbol.originalType ?? 'unknown'}`, 'arrow');
    }
    if (symbol.traits && symbol.traits.length > 0) {
        md.appendMarkdown(`\n\nImplements: ${symbol.traits.map(t => `\`${t}\``).join(', ')}`);
    }
    if (opts?.classTraits && opts.classTraits.length > 0) {
        md.appendMarkdown(`\n\nTraits: ${opts.classTraits.map(t => `\`${t}\``).join(', ')}`);
    }
    if (symbol.doc)
        md.appendMarkdown(`\n\n---\n\n${symbol.doc}`);
    return md;
}
// ===== Completion member helpers =====
function findEnclosingClass(document, fromLine) {
    const fromIndent = (document.lineAt(fromLine).text.match(/^(\s*)/)?.[1] ?? '').length;
    for (let i = fromLine - 1; i >= 0; i--) {
        const stripped = (0, analysis_1.stripComment)(document.lineAt(i).text);
        const m = stripped.match(analysis_1.CLASS_DEF_RE);
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
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = (0, analysis_1.stripComment)(document.lineAt(i).text);
        const m = stripped.match(analysis_1.NEW_TYPE_RE);
        if (m && m[2] === className) {
            return collectClassMemberItems(document, m[3].trim(), _visited);
        }
    }
    let classLine = -1;
    let classIndent = 0;
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = (0, analysis_1.stripComment)(document.lineAt(i).text);
        const m = stripped.match(analysis_1.CLASS_DEF_RE);
        if (m && m[3] === className) {
            classLine = i;
            classIndent = (m[1] ?? '').length;
            break;
        }
    }
    if (classLine < 0)
        return [];
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
        const stripped = (0, analysis_1.stripComment)(rawLine);
        if (!stripped.trim())
            continue;
        const lineIndent = (rawLine.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= classIndent)
            break;
        if (lineIndent === memberIndent) {
            const fieldMatch = stripped.match(analysis_1.HOVER_DECL_RE);
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
function findClassMember(document, className, memberName, _visited = new Set()) {
    if (_visited.has(className))
        return undefined;
    _visited.add(className);
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = (0, analysis_1.stripComment)(document.lineAt(i).text);
        const m = stripped.match(analysis_1.NEW_TYPE_RE);
        if (m && m[2] === className) {
            return findClassMember(document, m[3].trim(), memberName, _visited);
        }
    }
    let classLine = -1;
    let classIndent = 0;
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = (0, analysis_1.stripComment)(document.lineAt(i).text);
        const m = stripped.match(analysis_1.CLASS_DEF_RE);
        if (m && m[3] === className) {
            classLine = i;
            classIndent = (m[1] ?? '').length;
            break;
        }
    }
    if (classLine < 0)
        return undefined;
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
        return undefined;
    for (let i = classLine + 1; i < document.lineCount; i++) {
        const rawLine = document.lineAt(i).text;
        const stripped = (0, analysis_1.stripComment)(rawLine);
        if (!stripped.trim())
            continue;
        const lineIndent = (rawLine.match(/^(\s*)/)?.[1] ?? '').length;
        if (lineIndent <= classIndent)
            break;
        if (lineIndent === memberIndent) {
            const funcMatch = stripped.match(/^(\s*)(fn|gen)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:\s*$/);
            if (funcMatch) {
                const [, indentStr, kw, name, params, retAnnotation] = funcMatch;
                if (name === memberName) {
                    const returnType = (0, analysis_1.cleanTypeAnnotation)(retAnnotation) ?? 'unknown';
                    const cleanParams = params
                        .replace(/^\s*(?:let\s+|mut\s+)?self\s*,\s*/, '')
                        .replace(/^\s*(?:let\s+|mut\s+)?self\s*$/, '');
                    return {
                        name,
                        kind: 'function',
                        line: i,
                        type: returnType,
                        signature: `${kw} ${name}(${cleanParams}) -> ${returnType}`,
                        doc: (0, analysis_1.getDocstringAfter)(document, i, (indentStr ?? '').length),
                    };
                }
                continue;
            }
            const fieldMatch = stripped.match(analysis_1.HOVER_DECL_RE);
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
async function resolveMemberItems(document, position, objName) {
    if (objName === 'self') {
        const cls = findEnclosingClass(document, position.line);
        return cls ? collectClassMemberItems(document, cls) : [];
    }
    const a = await analysis_1.DocumentAnalysis.for(document);
    const sym = (0, analysis_1.selectHoverSymbol)(a.symbols, objName, position.line);
    // No local symbol — check if objName is a class from a cs/cpp/rs import (e.g. `wpf.WpfApp.`)
    if (!sym) {
        const cppCls = a.cppClasses.get(objName);
        if (!cppCls)
            return [];
        const items = [];
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
    if (sym.kind === 'module') {
        const items = [];
        // Top-level functions (py modules / cpp with exported funcs)
        const funcs = a.importFuncTypes.get(objName);
        const sigsMap = a.importFuncSigs.get(objName);
        if (funcs) {
            for (const [name, retType] of funcs) {
                const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
                item.detail = sigsMap?.get(name) ?? `→ ${retType}`;
                items.push(item);
            }
        }
        // Classes exported by a cs/cpp/rs module (e.g. wpf.WpfApp, wpf.FontFinder)
        const classes = a.moduleClasses.get(objName);
        if (classes) {
            for (const className of classes) {
                const item = new vscode.CompletionItem(className, vscode.CompletionItemKind.Class);
                const cppCls = a.cppClasses.get(className);
                if (cppCls?.classDocs)
                    item.detail = cppCls.classDocs;
                items.push(item);
            }
        }
        return items;
    }
    if (sym.type) {
        const cppCls = a.cppClasses.get(sym.type);
        if (cppCls) {
            const items = [];
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
        const builtinMethods = builtins_1.BUILTIN_TYPE_METHODS[baseType];
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
function stringLiteralRanges(line, codeEnd) {
    const ranges = [];
    let i = 0;
    while (i < codeEnd) {
        const c = line[i];
        if (c === '"' || c === "'") {
            const q = c;
            const triple = line.startsWith(q + q + q, i);
            const start = i;
            i += triple ? 3 : 1;
            while (i < codeEnd) {
                if (line[i] === '\\') {
                    i += 2;
                    continue;
                }
                if (triple ? line.startsWith(q + q + q, i) : line[i] === q) {
                    i += triple ? 3 : 1;
                    break;
                }
                i++;
            }
            ranges.push([start, i]);
        }
        else {
            i++;
        }
    }
    return ranges;
}
function escapeRegex(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
function parseTypeAt(line, pos, out) {
    while (pos < line.length && (line[pos] === ' ' || line[pos] === '\t'))
        pos++;
    if (pos >= line.length || !/[A-Za-z_]/.test(line[pos]))
        return pos;
    out.add(pos);
    while (pos < line.length && /\w/.test(line[pos]))
        pos++;
    while (pos < line.length && (line[pos] === ' ' || line[pos] === '\t'))
        pos++;
    if (pos < line.length && line[pos] === '[') {
        pos++;
        while (pos < line.length && line[pos] !== ']') {
            if (',\t '.includes(line[pos])) {
                pos++;
                continue;
            }
            const newPos = parseTypeAt(line, pos, out);
            pos = newPos > pos ? newPos : pos + 1; // always advance to prevent infinite loop
        }
        if (pos < line.length)
            pos++;
    }
    return pos;
}
function typeAnnotationPositions(rawLine, strRanges) {
    const out = new Set();
    const src = (0, analysis_1.stripComment)(rawLine);
    const inString = (pos) => strRanges.some(([s, e]) => pos >= s && pos < e);
    const colonRe = /[A-Za-z_]\w*[ \t]*:(?!=)(?=[ \t]*[A-Za-z_])/g;
    let m;
    while ((m = colonRe.exec(src)) !== null) {
        if (!inString(m.index))
            parseTypeAt(src, m.index + m[0].length, out);
    }
    const arrowRe = /->[ \t]*/g;
    while ((m = arrowRe.exec(src)) !== null) {
        if (!inString(m.index))
            parseTypeAt(src, m.index + m[0].length, out);
    }
    const isNotRe = /\bis[ \t]+not[ \t]+/g;
    while ((m = isNotRe.exec(src)) !== null) {
        if (!inString(m.index))
            parseTypeAt(src, m.index + m[0].length, out);
    }
    const isRe = /\bis[ \t]+(?!not\b)/g;
    while ((m = isRe.exec(src)) !== null) {
        if (!inString(m.index))
            parseTypeAt(src, m.index + m[0].length, out);
    }
    return out;
}
// ===== Providers =====
/**
 * Hover provider — show type/signature information for the symbol under the cursor.
 *
 * Resolution order:
 *  1. Dot-access: `module.name`, `module.Class.method`, `instance.field`
 *  2. `Self` keyword → resolves to the enclosing class name
 *  3. Imported external class (C++/Rust/C#)
 *  4. Local symbol from `DocumentAnalysis`
 *  5. Built-in stub (builtins.ars)
 */
async function provideHover(document, position) {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_]\w*/);
    if (!range)
        return undefined;
    const name = document.getText(range);
    const lineText = document.lineAt(position.line).text;
    const prefixStr = lineText.substring(0, range.start.character);
    const dotAccess = prefixStr.match(/([A-Za-z_]\w*)\.$/);
    if (dotAccess) {
        const objName = dotAccess[1];
        const a = await analysis_1.DocumentAnalysis.for(document);
        // ── 1-level: module.name ──────────────────────────────────────────────
        // Top-level function in external module
        const retType = a.importFuncTypes.get(objName)?.get(name);
        if (retType !== undefined) {
            const md = new vscode.MarkdownString(undefined, true);
            const sig = a.importFuncSigs.get(objName)?.get(name) ?? `fn ${name}() -> ${retType}`;
            md.appendCodeblock(sig, 'arrow');
            const docText = a.importFuncDocs.get(objName)?.get(name);
            if (docText)
                md.appendMarkdown(`\n\n${docText}`);
            return new vscode.Hover(md, range);
        }
        // Class name under a module alias: `module.ClassName`
        {
            const objModSym = (0, analysis_1.selectHoverSymbol)(a.symbols, objName, position.line);
            if (objModSym?.kind === 'module') {
                const cppCls = a.cppClasses.get(name);
                if (cppCls) {
                    return new vscode.Hover(renderExternalClassHover(name, cppCls), range);
                }
            }
        }
        // ── 2-level: module.ClassName.method (cursor is on `method`) ─────────
        const deeperMatch = prefixStr.match(/\b([A-Za-z_]\w*)\.([A-Za-z_]\w*)\.$/);
        if (deeperMatch) {
            const modAlias = deeperMatch[1];
            const className = deeperMatch[2];
            const modSym = (0, analysis_1.selectHoverSymbol)(a.symbols, modAlias, position.line);
            if (modSym?.kind === 'module') {
                const cppCls = a.cppClasses.get(className);
                if (cppCls) {
                    const methodInfo = cppCls.methods.get(name);
                    if (methodInfo) {
                        const md = new vscode.MarkdownString(undefined, true);
                        md.appendCodeblock(methodInfo.sig, 'arrow');
                        md.appendMarkdown(`\n\n*method of* \`${className}\``);
                        if (methodInfo.doc)
                            md.appendMarkdown(`\n\n${methodInfo.doc}`);
                        return new vscode.Hover(md, range);
                    }
                    const fieldType = cppCls.fields.get(name);
                    if (fieldType !== undefined) {
                        const md = new vscode.MarkdownString(undefined, true);
                        md.appendCodeblock(`${name}: ${fieldType}`, 'arrow');
                        md.appendMarkdown(`\n\n*field of* \`${className}\``);
                        return new vscode.Hover(md, range);
                    }
                }
            }
        }
        // ── Instance method/field on typed variable ────────────────────────────
        const objSym = (0, analysis_1.selectHoverSymbol)(a.symbols, objName, position.line);
        if (objSym?.type) {
            const cppCls = a.cppClasses.get(objSym.type);
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
                    if (methodInfo.doc)
                        md.appendMarkdown(`\n\n${methodInfo.doc}`);
                    return new vscode.Hover(md, range);
                }
            }
            const builtinMethod = builtins_1.BUILTIN_TYPE_METHODS[objSym.type]?.[name];
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
            if (memberSym)
                return new vscode.Hover(renderHover(memberSym), range);
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
    // Imported class type hover (C++/Rust/C#)
    {
        const a = await analysis_1.DocumentAnalysis.for(document);
        const cppCls = a.cppClasses.get(name);
        if (cppCls) {
            return new vscode.Hover(renderExternalClassHover(name, cppCls), range);
        }
    }
    const a = await analysis_1.DocumentAnalysis.for(document);
    const symbol = (0, analysis_1.selectHoverSymbol)(a.symbols, name, position.line);
    if (!symbol) {
        const builtinSig = analysis_1.builtinStub.sigs.get(name);
        if (builtinSig) {
            const md = new vscode.MarkdownString(undefined, true);
            md.appendCodeblock(builtinSig, 'arrow');
            const doc = analysis_1.builtinStub.docs.get(name);
            if (doc)
                md.appendMarkdown(`\n\n${doc}`);
            return new vscode.Hover(md, range);
        }
        return undefined;
    }
    const freezeLine = a.freezeLines.get(name);
    const isFrozen = freezeLine !== undefined && position.line >= freezeLine && symbol.mutability === 'mut';
    const override = a.scopeOverrides.find(o => o.varName === name && position.line >= o.startLine && position.line < o.endLine);
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
exports.provideHover = provideHover;
/**
 * Inlay hints provider — insert inferred type labels next to variable names.
 *
 * Two passes:
 *  1. Function return types: insert `-> T` after the closing `)` of each `fn`/`gen`
 *     that has no explicit return-type annotation.
 *  2. Variable declarations: walk lines sequentially, updating a local `env` map,
 *     and emit `: T` after the variable name when no annotation is present.
 */
async function provideInlayHints(document, _range) {
    const hints = [];
    const a = await analysis_1.DocumentAnalysis.for(document);
    // Inlay hints on function definition lines (only when no annotation)
    for (const def of a.funcDefs) {
        if (def.annotation !== undefined)
            continue;
        const returnType = a.funcEnv.get(def.name);
        const sigLine = def.sigEndLine ?? def.defLine;
        const rawLine = document.lineAt(sigLine).text;
        const rparenPos = rawLine.lastIndexOf(')');
        if (rparenPos < 0)
            continue;
        const pos = new vscode.Position(sigLine, rparenPos + 1);
        hints.push(new vscode.InlayHint(pos, ` -> ${returnType}`, vscode.InlayHintKind.Type));
    }
    // Inlay hints on variable declarations — requires a sequential per-line env
    const env = new Map();
    let classContext;
    let selfType;
    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const rawLine = document.lineAt(lineIdx).text;
        const line = (0, analysis_1.stripComment)(rawLine);
        if (line.match(analysis_1.IMPORT_RE))
            continue;
        if (line.trim()) {
            const lineIndent = (rawLine.match(/^(\s*)/)?.[1] ?? '').length;
            if (classContext && lineIndent <= classContext.indent) {
                classContext = undefined;
                selfType = undefined;
            }
            const classM = line.match(analysis_1.CLASS_DEF_RE);
            if (classM) {
                classContext = { name: classM[3], indent: (classM[1] ?? '').length };
                selfType = undefined;
                continue;
            }
            const funcDefLines = (0, analysis_1.gatherFuncDefLines)(document, lineIdx);
            const funcM = funcDefLines?.fullLine.match(builtins_1.FUNC_DEF_RE);
            if (funcM) {
                selfType = classContext?.name;
                const params = funcM[4];
                for (const p of (0, analysis_1.splitComma)(params)) {
                    const pm = (0, analysis_1.stripParamDefault)(p).trim().match(/^(?:(?:let|mut)\s+)?([A-Za-z_]\w*)\s*(?::\s*(.+))?$/);
                    if (pm && pm[1] !== 'self' && pm[2]?.trim()) {
                        const pt = pm[2].trim();
                        env.set(pm[1], pt === 'Self' && selfType ? selfType : pt);
                    }
                }
                lineIdx = funcDefLines.lastLine;
                continue;
            }
        }
        const staticMatch = line.match(analysis_1.STATIC_DECL_RE);
        if (staticMatch) {
            const [, indent, name, annotation, rhs] = staticMatch;
            const type = (0, analysis_1.cleanTypeAnnotation)(annotation)
                ?? (rhs ? (0, analysis_1.inferExprType)(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType) : 'unknown');
            env.set(name, type);
            if (!annotation) {
                const nameStart = rawLine.indexOf(name, indent.length + 'static mut '.length);
                if (nameStart >= 0)
                    hints.push(makeInlayHint(lineIdx, nameStart + name.length, type));
            }
            continue;
        }
        const tupleM = line.match(analysis_1.TUPLE_DECL_RE);
        if (tupleM) {
            const [, indent, keyword, names, rhs] = tupleM;
            const rhsType = (0, analysis_1.inferExprType)(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType);
            const nameList = names.split(',').map(n => n.trim()).filter(Boolean);
            const elemTypes = (0, analysis_1.extractTupleElemTypes)(rhsType, nameList.length);
            let searchFrom = indent.length + keyword.length;
            for (let idx = 0; idx < nameList.length; idx++) {
                const varName = nameList[idx];
                const elemType = elemTypes[idx] ?? 'unknown';
                env.set(varName, elemType);
                const nameStart = rawLine.indexOf(varName, searchFrom);
                if (nameStart >= 0) {
                    hints.push(makeInlayHint(lineIdx, nameStart + varName.length, elemType));
                    searchFrom = nameStart + varName.length;
                }
            }
            continue;
        }
        const forLoopM = line.match(analysis_1.FOR_LOOP_RE);
        if (forLoopM) {
            const [, indent, rawTargets, rawIter] = forLoopM;
            const iterExpr = rawIter.replace(/\s*(?:->[^:]+)?\s*:\s*$/, '').trim();
            const elemType = (0, analysis_1.inferForLoopElemType)(iterExpr, env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType);
            const targetList = rawTargets.split(',').map(t => t.trim()).filter(Boolean);
            let searchFrom = (indent ?? '').length + 'for '.length;
            if (targetList.length === 1) {
                const varName = targetList[0];
                env.set(varName, elemType);
                if (elemType !== 'unknown') {
                    const nameStart = rawLine.indexOf(varName, searchFrom);
                    if (nameStart >= 0)
                        hints.push(makeInlayHint(lineIdx, nameStart + varName.length, elemType));
                }
            }
            else {
                const subTypes = (0, analysis_1.extractTupleElemTypes)(elemType, targetList.length);
                for (let idx = 0; idx < targetList.length; idx++) {
                    const varName = targetList[idx];
                    const varType = subTypes[idx] ?? 'unknown';
                    env.set(varName, varType);
                    if (varType !== 'unknown') {
                        const nameStart = rawLine.indexOf(varName, searchFrom);
                        if (nameStart >= 0) {
                            hints.push(makeInlayHint(lineIdx, nameStart + varName.length, varType));
                            searchFrom = nameStart + varName.length;
                        }
                    }
                }
            }
            continue;
        }
        const declM = line.match(analysis_1.HOVER_DECL_RE);
        if (!declM)
            continue;
        const [, indent, keyword, name, annotation, rhs] = declM;
        const type = (0, analysis_1.cleanTypeAnnotation)(annotation)
            ?? (rhs ? (0, analysis_1.inferExprType)(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams, a.classFieldTypes, selfType) : 'unknown');
        env.set(name, type);
        if (!annotation && rhs) {
            const nameStart = rawLine.indexOf(name, (indent ?? '').length + keyword.length);
            if (nameStart >= 0)
                hints.push(makeInlayHint(lineIdx, nameStart + name.length, type));
        }
    }
    return hints;
}
exports.provideInlayHints = provideInlayHints;
/**
 * Completion provider — suggest symbols, built-ins, and keywords.
 *
 * When the cursor follows a `.`, delegate to `resolveMemberItems` to offer
 * fields/methods of the preceding object.  Otherwise suggest all in-scope
 * symbols, built-in stubs, built-in functions, and language keywords.
 */
async function provideCompletionItems(document, position) {
    const prefix = document.lineAt(position.line).text.substring(0, position.character);
    const dotMatch = prefix.match(/([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/);
    if (dotMatch) {
        return resolveMemberItems(document, position, dotMatch[1]);
    }
    const items = [];
    const seen = new Set();
    const a = await analysis_1.DocumentAnalysis.for(document);
    for (const sym of a.symbols) {
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
    for (const [name, sig] of analysis_1.builtinStub.sigs) {
        if (seen.has(name))
            continue;
        seen.add(name);
        const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
        item.detail = sig;
        const doc = analysis_1.builtinStub.docs.get(name);
        if (doc)
            item.documentation = new vscode.MarkdownString(doc);
        items.push(item);
    }
    for (const [name, retType] of Object.entries(builtins_1.BUILTIN_RETURN_TYPES)) {
        if (seen.has(name))
            continue;
        seen.add(name);
        const item = new vscode.CompletionItem(name, vscode.CompletionItemKind.Function);
        item.detail = `→ ${retType}`;
        items.push(item);
    }
    for (const kw of builtins_1.LANG_KEYWORDS) {
        if (seen.has(kw))
            continue;
        seen.add(kw);
        items.push(new vscode.CompletionItem(kw, vscode.CompletionItemKind.Keyword));
    }
    return items;
}
exports.provideCompletionItems = provideCompletionItems;
/**
 * Try to extract a symbol from one (already comment-stripped) source line.
 * Returns `undefined` when the line contains no documentable symbol.
 * Using early-return keeps nesting flat instead of chaining else-if blocks.
 */
function matchDocumentSymbol(stripped, indent) {
    const funcMatch = stripped.match(/^(\s*)(fn|gen)\s+([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\(([^)]*)\)\s*(?:->\s*(.+?))?\s*:\s*$/);
    if (funcMatch) {
        const [, , , funcName, , retType] = funcMatch;
        return { name: funcName, detail: retType?.trim() ?? '', kind: vscode.SymbolKind.Function, isContainer: true };
    }
    const classMatch = stripped.match(analysis_1.CLASS_DEF_RE);
    if (classMatch) {
        const [, , kw, className, bases] = classMatch;
        return {
            name: className,
            detail: bases ? `(${bases})` : '',
            kind: kw === 'trait' ? vscode.SymbolKind.Interface : vscode.SymbolKind.Class,
            isContainer: true,
        };
    }
    const newTypeMatch = stripped.match(analysis_1.NEW_TYPE_RE);
    if (newTypeMatch) {
        const [, , ntName, ntType] = newTypeMatch;
        return { name: ntName, detail: ntType.trim(), kind: vscode.SymbolKind.TypeParameter, isContainer: false };
    }
    const importMatch = stripped.match(analysis_1.IMPORT_RE);
    if (importMatch) {
        const [, importKind, modulePath, , , explicitAlias] = importMatch;
        const alias = explicitAlias ?? (modulePath.split('.').pop() ?? modulePath);
        return { name: alias, detail: `${importKind} ${modulePath}`, kind: vscode.SymbolKind.Module, isContainer: false };
    }
    const staticMatch = stripped.match(analysis_1.STATIC_DECL_RE);
    if (staticMatch) {
        const [, , varName, annot] = staticMatch;
        return { name: varName, detail: annot?.trim() ?? '', kind: vscode.SymbolKind.Variable, isContainer: false };
    }
    // Only surface top-level `let`/`mut`/`const` declarations in the outline
    if (indent === 0) {
        const declMatch = stripped.match(analysis_1.HOVER_DECL_RE);
        if (declMatch) {
            const [, , , varName, annot] = declMatch;
            return { name: varName, detail: annot?.trim() ?? '', kind: vscode.SymbolKind.Variable, isContainer: false };
        }
    }
    return undefined;
}
/** Provide the document outline (breadcrumbs, Outline view) for an Arrow file. */
function provideDocumentSymbols(document) {
    const result = [];
    const stack = [];
    for (let i = 0; i < document.lineCount; i++) {
        const rawLine = document.lineAt(i).text;
        const stripped = (0, analysis_1.stripComment)(rawLine);
        if (!stripped.trim())
            continue;
        const indent = (rawLine.match(/^(\s*)/)?.[1] ?? '').length;
        // Pop containers that ended before this indentation level
        while (stack.length > 0 && indent <= stack[stack.length - 1].indent)
            stack.pop();
        const info = matchDocumentSymbol(stripped, indent);
        if (!info)
            continue;
        const { name, detail, kind, isContainer } = info;
        const nameIdx = rawLine.indexOf(name, indent);
        const selRange = nameIdx >= 0
            ? new vscode.Range(i, nameIdx, i, nameIdx + name.length)
            : document.lineAt(i).range;
        const bodyEnd = isContainer ? (0, analysis_1.findBodyEndLine)(document, i, indent) : i + 1;
        const lastLine = Math.min(bodyEnd - 1, document.lineCount - 1);
        const bodyRange = new vscode.Range(i, 0, lastLine, document.lineAt(lastLine).text.length);
        const sym = new vscode.DocumentSymbol(name, detail, kind, bodyRange, selRange);
        if (stack.length === 0)
            result.push(sym);
        else
            stack[stack.length - 1].sym.children.push(sym);
        if (isContainer)
            stack.push({ sym, indent });
    }
    return result;
}
exports.provideDocumentSymbols = provideDocumentSymbols;
/**
 * Signature help provider — show parameter list while the user types inside `(`.
 *
 * Scans backwards from the cursor to find the nearest unclosed `(` and the
 * function name that precedes it.  Resolves the signature from local symbols
 * or the built-in stub, then highlights the active parameter.
 */
async function provideSignatureHelp(document, position) {
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
    const a = await analysis_1.DocumentAnalysis.for(document);
    const funcSym = a.symbols.find(s => s.name === funcName && s.kind === 'function');
    let sigStr;
    let sigDoc;
    if (funcSym?.signature) {
        sigStr = funcSym.signature;
        if (funcSym.doc)
            sigDoc = new vscode.MarkdownString(funcSym.doc);
    }
    else {
        const builtinSig = analysis_1.builtinStub.sigs.get(funcName);
        if (builtinSig) {
            sigStr = builtinSig;
            const doc = analysis_1.builtinStub.docs.get(funcName);
            if (doc)
                sigDoc = new vscode.MarkdownString(doc);
        }
    }
    if (!sigStr)
        return undefined;
    const sigInfo = new vscode.SignatureInformation(sigStr, sigDoc);
    const paramsMatch = sigStr.match(/\(([^)]*)\)/);
    if (paramsMatch?.[1]?.trim()) {
        for (const p of (0, analysis_1.splitComma)(paramsMatch[1])) {
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
// ===== Go-to-Definition helpers =====
/** Parse "importKind modulePath" stored in HoverSymbol.originalType */
function parseModuleOriginalType(originalType) {
    const m = originalType.match(/^(import(?:\[[^\]]*\])?)\s+([\w.]+)$/);
    if (!m)
        return undefined;
    return { importKind: m[1], modulePath: m[2] };
}
/**
 * Search for the line number of a named member inside an external source file.
 * Uses language-specific patterns so the user lands on the right declaration.
 */
async function findMemberLineInExternalFile(filePath, memberName, importKind) {
    let content;
    try {
        content = await fs_1.promises.readFile(filePath, 'utf8');
    }
    catch {
        return 0;
    }
    const lines = content.split('\n');
    const kind = (0, native_module_1.importKindOf)(importKind);
    const esc = memberName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        if (kind === 'rs') {
            if (/\bpub\s+(?:fn|struct|enum|type)\b/.test(line) && new RegExp(`\\b${esc}\\b`).test(line))
                return i;
        }
        else if (kind === 'cpp') {
            if (new RegExp(`\\b(?:struct|class)\\s+${esc}\\b|\\b${esc}\\s*\\(`).test(line))
                return i;
        }
        else if (kind === 'py') {
            if (new RegExp(`^\\s*(?:def|class)\\s+${esc}\\b`).test(line))
                return i;
        }
        else {
            // Arrow (.ar / .ars) and JS bridge stubs
            if (new RegExp(`\\b(?:fn|gen|class|trait|enum|new_type)\\s+${esc}\\b`).test(line))
                return i;
        }
    }
    return 0;
}
/**
 * Go-to-definition provider — jump to the declaration of the symbol under the cursor.
 *
 * Resolution order:
 *  1. Dot-access on a module alias → jump to the module source file
 *  2. Dot-access on an instance → jump to the member in the external source
 *  3. Module alias itself → jump to the module file
 *  4. Any other symbol → jump to its declaration line in the current document
 */
async function provideDefinition(document, position) {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_]\w*/);
    if (!range)
        return undefined;
    const name = document.getText(range);
    const lineText = document.lineAt(position.line).text;
    const prefixStr = lineText.substring(0, range.start.character);
    const dotAccess = prefixStr.match(/([A-Za-z_]\w*)\.$/);
    const a = await analysis_1.DocumentAnalysis.for(document);
    const docDir = path.dirname(document.uri.fsPath);
    /** Resolve an import to a vscode.Location, optionally seeking a specific member. */
    async function jumpToImport(importKind, modulePath, stubName, memberName) {
        const pyLibPaths = vscode.workspace.getConfiguration('arrow').get('pythonLibraryPaths', []);
        const filePath = await (0, native_module_1.resolveImportSourceFile)(importKind, modulePath, stubName, docDir, pyLibPaths);
        if (!filePath)
            return undefined;
        const uri = vscode.Uri.file(filePath);
        if (!memberName)
            return new vscode.Location(uri, new vscode.Position(0, 0));
        const lineNum = await findMemberLineInExternalFile(filePath, memberName, importKind);
        try {
            const fileLines = (await fs_1.promises.readFile(filePath, 'utf8')).split('\n');
            const targetLine = fileLines[lineNum] ?? '';
            const col = Math.max(0, targetLine.indexOf(memberName));
            return new vscode.Location(uri, new vscode.Range(lineNum, col, lineNum, col + memberName.length));
        }
        catch {
            return new vscode.Location(uri, new vscode.Position(lineNum, 0));
        }
    }
    // ── Dot-access: `module.member`, `module.Class.method`, or `instance.method` ─
    if (dotAccess) {
        const objName = dotAccess[1];
        const directSym = (0, analysis_1.selectHoverSymbol)(a.symbols, objName, position.line);
        // Find the module symbol (direct or 2-level deeper: module.Class.method)
        let moduleSym = directSym?.kind === 'module' ? directSym : undefined;
        if (!moduleSym) {
            const deeperMatch = prefixStr.match(/\b([A-Za-z_]\w*)\.[A-Za-z_]\w*\.$/);
            if (deeperMatch) {
                const deeperSym = (0, analysis_1.selectHoverSymbol)(a.symbols, deeperMatch[1], position.line);
                if (deeperSym?.kind === 'module')
                    moduleSym = deeperSym;
            }
        }
        if (moduleSym?.originalType) {
            const parsed = parseModuleOriginalType(moduleSym.originalType);
            if (parsed) {
                const importLine = (0, analysis_1.stripComment)(document.lineAt(moduleSym.line).text);
                const importMatch = importLine.match(analysis_1.IMPORT_RE);
                const stubName = importMatch?.[3];
                const loc = await jumpToImport(parsed.importKind, parsed.modulePath, stubName, name);
                if (loc)
                    return loc;
                return undefined;
            }
        }
        // Instance method on external class: calc0.GetAccumulated where calc0: Calculator
        const varType = directSym?.type;
        if (varType) {
            const src = a.classSourceMap.get(varType);
            if (src) {
                const loc = await jumpToImport(src.importKind, src.modulePath, src.stubName, name);
                if (loc)
                    return loc;
                return undefined;
            }
        }
        // Non-module dotAccess: fall through to same-file definition lookup
    }
    const symbol = (0, analysis_1.selectHoverSymbol)(a.symbols, name, position.line);
    if (!symbol)
        return undefined;
    // ── Module alias: jump to module file ───────────────────────────────────────
    if (symbol.kind === 'module' && symbol.originalType) {
        const parsed = parseModuleOriginalType(symbol.originalType);
        if (parsed) {
            const importLine = (0, analysis_1.stripComment)(document.lineAt(symbol.line).text);
            const importMatch = importLine.match(analysis_1.IMPORT_RE);
            const stubName = importMatch?.[3];
            const loc = await jumpToImport(parsed.importKind, parsed.modulePath, stubName);
            if (loc)
                return loc;
        }
    }
    // ── Default: definition in current document ──────────────────────────────────
    const targetText = document.lineAt(symbol.line).text;
    const nameIdx = targetText.indexOf(symbol.name);
    const targetRange = nameIdx >= 0
        ? new vscode.Range(symbol.line, nameIdx, symbol.line, nameIdx + symbol.name.length)
        : document.lineAt(symbol.line).range;
    return new vscode.Location(document.uri, targetRange);
}
exports.provideDefinition = provideDefinition;
/**
 * Diagnostics provider — report errors without running the Arrow compiler.
 *
 * Checks performed:
 *  - Re-declaration of a variable within the same scope (indent-based scope stack)
 *  - Assignment to `let` or `const` variables
 *  - Assignment to a `freeze`d variable
 *  - Type mismatch between a declared primitive type and the inferred RHS type
 *  - `is not` guard applied to a non-Union/Option variable
 */
async function provideDiagnostics(document) {
    const diagnostics = [];
    const a = await analysis_1.DocumentAnalysis.for(document);
    const env = new Map();
    const scopeStack = [{ indent: -1, vars: new Map() }]; // index 0 = global
    let prevIndent = -1; // indent of the most recent non-empty line
    /** Search all frames (inner → outer) for name. Returns declaration line or undefined. */
    function findInScope(name) {
        for (let fi = scopeStack.length - 1; fi >= 0; fi--) {
            const found = scopeStack[fi].vars.get(name);
            if (found !== undefined)
                return found;
        }
        return undefined;
    }
    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = (0, analysis_1.stripComment)(raw);
        if (!stripped.trim())
            continue;
        const lineIndent = (raw.match(/^(\s*)/)?.[1] ?? '').length;
        // Adjust scope stack based on indentation change
        if (prevIndent >= 0 && lineIndent > prevIndent) {
            scopeStack.push({ indent: lineIndent, vars: new Map() });
        }
        else if (lineIndent < prevIndent) {
            while (scopeStack.length > 1 && scopeStack[scopeStack.length - 1].indent >= lineIndent) {
                scopeStack.pop();
            }
        }
        prevIndent = lineIndent;
        if (stripped.match(/^(\s*)(fn|gen)\s+/) || stripped.match(analysis_1.CLASS_DEF_RE) || stripped.match(analysis_1.IMPORT_RE))
            continue;
        const staticM = stripped.match(analysis_1.STATIC_DECL_RE);
        if (staticM) {
            const [, , name, annotation, rhs] = staticM;
            const type = (0, analysis_1.cleanTypeAnnotation)(annotation)
                ?? (rhs ? (0, analysis_1.inferExprType)(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams) : 'unknown');
            env.set(name, type);
            continue;
        }
        const tupleM = stripped.match(analysis_1.TUPLE_DECL_RE);
        if (tupleM) {
            const [, indentStr, , names, rhs] = tupleM;
            const rhsType = (0, analysis_1.inferExprType)(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams);
            const nameList = names.split(',').map(n => n.trim()).filter(Boolean);
            const elemTypes = (0, analysis_1.extractTupleElemTypes)(rhsType, nameList.length);
            let searchFrom = (indentStr ?? '').length;
            for (let idx = 0; idx < nameList.length; idx++) {
                const varName = nameList[idx];
                env.set(varName, elemTypes[idx] ?? 'unknown');
                if (varName !== '_') {
                    const existingLine = findInScope(varName);
                    const nameStart = raw.indexOf(varName, searchFrom);
                    if (existingLine !== undefined && nameStart >= 0) {
                        diagnostics.push(new vscode.Diagnostic(new vscode.Range(i, nameStart, i, nameStart + varName.length), `Variable '${varName}' is already declared (line ${existingLine + 1})`, vscode.DiagnosticSeverity.Error));
                    }
                    else {
                        scopeStack[scopeStack.length - 1].vars.set(varName, i);
                    }
                    if (nameStart >= 0)
                        searchFrom = nameStart + varName.length;
                }
            }
            continue;
        }
        const declM = stripped.match(analysis_1.HOVER_DECL_RE);
        if (declM) {
            const [, indent, keyword, name, annotation, rhs] = declM;
            const declaredType = (0, analysis_1.cleanTypeAnnotation)(annotation);
            // Redeclaration check
            if (name !== '_') {
                const existingLine = findInScope(name);
                if (existingLine !== undefined) {
                    const nameStart = raw.indexOf(name, (indent ?? '').length + (keyword ?? '').length);
                    if (nameStart >= 0) {
                        diagnostics.push(new vscode.Diagnostic(new vscode.Range(i, nameStart, i, nameStart + name.length), `Variable '${name}' is already declared (line ${existingLine + 1})`, vscode.DiagnosticSeverity.Error));
                    }
                }
                else {
                    scopeStack[scopeStack.length - 1].vars.set(name, i);
                }
            }
            if (declaredType && rhs) {
                const inferredType = (0, analysis_1.inferExprType)(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams);
                env.set(name, declaredType);
                if (PRIMITIVE_TYPES.has(declaredType) &&
                    PRIMITIVE_TYPES.has(inferredType) &&
                    !isTypeCompatible(declaredType, inferredType)) {
                    const indentLen = (indent ?? '').length;
                    const colonIdx = raw.indexOf(':', indentLen + (keyword ?? '').length + name.length);
                    if (colonIdx >= 0) {
                        const annotStart = raw.indexOf(declaredType, colonIdx + 1);
                        if (annotStart >= 0) {
                            diagnostics.push(new vscode.Diagnostic(new vscode.Range(i, annotStart, i, annotStart + declaredType.length), `Type mismatch: declared '${declaredType}', but right-hand side has type '${inferredType}'`, vscode.DiagnosticSeverity.Error));
                        }
                    }
                }
            }
            else {
                env.set(name, declaredType ?? (rhs
                    ? (0, analysis_1.inferExprType)(rhs.trim(), env, a.funcEnv, a.importAliases, a.importFuncTypes, a.classMethods, a.templateParams)
                    : 'unknown'));
            }
            continue;
        }
        const reassignM = stripped.match(REASSIGN_RE);
        if (!reassignM)
            continue;
        const [, indent, varName] = reassignM;
        const sym = (0, analysis_1.selectHoverSymbol)(a.symbols, varName, i);
        if (!sym || sym.kind !== 'variable')
            continue;
        const nameStart = raw.indexOf(varName, (indent ?? '').length);
        if (nameStart < 0)
            continue;
        const diagRange = new vscode.Range(i, nameStart, i, nameStart + varName.length);
        if (sym.mutability === 'let' || sym.mutability === 'const') {
            diagnostics.push(new vscode.Diagnostic(diagRange, `Cannot assign to '${sym.mutability}' variable '${varName}'`, vscode.DiagnosticSeverity.Error));
        }
        else if (sym.mutability === 'mut') {
            const freezeLine = a.freezeLines.get(varName);
            if (freezeLine !== undefined && i > freezeLine) {
                diagnostics.push(new vscode.Diagnostic(diagRange, `Cannot assign to frozen variable '${varName}'`, vscode.DiagnosticSeverity.Error));
            }
        }
    }
    // Check 'is not' applied to a non-Union/Option variable (StaticTypeError: IsNotOnNonUnion)
    const IS_NOT_RE = /^(\s*)(?:if|elif)\s+([A-Za-z_]\w*)\s+is\s+not\s+([A-Za-z_]\w*)\s*:/;
    for (let i = 0; i < document.lineCount; i++) {
        const raw = document.lineAt(i).text;
        const stripped = (0, analysis_1.stripComment)(raw);
        const m = stripped.match(IS_NOT_RE);
        if (!m)
            continue;
        const varName = m[2];
        const sym = (0, analysis_1.selectHoverSymbol)(a.symbols, varName, i);
        if (!sym?.type)
            continue;
        const t = sym.type;
        if (t === 'unknown' || t === 'Any' ||
            t.startsWith('Union[') || t.startsWith('Option[') || t.startsWith('Optional['))
            continue;
        const indent = (m[1] ?? '').length;
        const keywordLen = stripped.slice(indent).startsWith('elif') ? 4 : 2;
        const nameStart = raw.indexOf(varName, indent + keywordLen);
        if (nameStart < 0)
            continue;
        diagnostics.push(new vscode.Diagnostic(new vscode.Range(i, nameStart, i, nameStart + varName.length), `'is not' requires a Union or Option type, but '${varName}' has type '${t}'`, vscode.DiagnosticSeverity.Error));
    }
    return diagnostics;
}
exports.provideDiagnostics = provideDiagnostics;
/**
 * Semantic tokens provider — colour class names, imported identifiers, and built-in types.
 *
 * Token types (indices match `SEMANTIC_TOKENS_LEGEND`):
 *  0 = 'class'    — user-defined class/trait/enum names and import aliases
 *  1 = 'type'     — built-in type names in type-annotation positions
 *  2 = 'variable' — built-in type names used as values (e.g. `int(x)`)
 */
async function provideDocumentSemanticTokens(document) {
    const builder = new vscode.SemanticTokensBuilder(exports.SEMANTIC_TOKENS_LEGEND);
    const a = await analysis_1.DocumentAnalysis.for(document);
    const userTypes = new Set();
    for (let i = 0; i < document.lineCount; i++) {
        const stripped = (0, analysis_1.stripComment)(document.lineAt(i).text);
        const classM = stripped.match(analysis_1.CLASS_DEF_RE);
        if (classM) {
            userTypes.add(classM[3]);
            continue;
        }
        const ntM = stripped.match(analysis_1.NEW_TYPE_RE);
        if (ntM)
            userTypes.add(ntM[2]);
    }
    for (let lineIdx = 0; lineIdx < document.lineCount; lineIdx++) {
        const lineText = document.lineAt(lineIdx).text;
        const commentStart = (0, analysis_1.stripComment)(lineText).length;
        const strRanges = stringLiteralRanges(lineText, commentStart);
        // The `[tag]` of an `import[tag]` is part of the import keyword, not an
        // expression — never emit semantic tokens inside it (e.g. the `int` in
        // `import[py-int]` must not be re-colored as the `int` cast/type).
        const importTagM = lineText.match(/\bimport\[[^\]]*\]/);
        const importTagRange = importTagM
            ? [importTagM.index, importTagM.index + importTagM[0].length]
            : null;
        const isLiveCode = (col) => col < commentStart
            && !strRanges.some(([s, e]) => col >= s && col < e)
            && !(importTagRange !== null && col >= importTagRange[0] && col < importTagRange[1]);
        const typePositions = typeAnnotationPositions(lineText, strRanges);
        const hits = [];
        for (const name of builtins_1.BUILTIN_TYPE_NAMES) {
            const re = new RegExp(`\\b${name}\\b`, 'g');
            let m;
            while ((m = re.exec(lineText)) !== null) {
                if (!isLiveCode(m.index))
                    continue;
                if (typePositions.has(m.index)) {
                    hits.push({ col: m.index, len: name.length, tokenType: 1 });
                }
                else {
                    let j = m.index + name.length;
                    while (j < lineText.length && (lineText[j] === ' ' || lineText[j] === '\t'))
                        j++;
                    const next = lineText[j];
                    if (next !== '(' && next !== '[') {
                        hits.push({ col: m.index, len: name.length, tokenType: 2 });
                    }
                }
            }
        }
        for (const alias of a.importAliases) {
            if (!alias)
                continue; // guard: empty alias would create /\b\b/g infinite loop
            const re = new RegExp(`\\b${escapeRegex(alias)}\\b`, 'g');
            let m;
            while ((m = re.exec(lineText)) !== null) {
                if (!isLiveCode(m.index))
                    continue;
                hits.push({ col: m.index, len: alias.length, tokenType: 0 });
            }
        }
        for (const name of userTypes) {
            if (!name)
                continue;
            const re = new RegExp(`\\b${escapeRegex(name)}\\b`, 'g');
            let m;
            while ((m = re.exec(lineText)) !== null) {
                if (!isLiveCode(m.index))
                    continue;
                hits.push({ col: m.index, len: name.length, tokenType: 0 });
            }
        }
        hits.sort((a, b) => a.col !== b.col ? a.col - b.col : a.tokenType - b.tokenType);
        let prevEnd = 0;
        for (const { col, len, tokenType } of hits) {
            if (col < prevEnd)
                continue;
            builder.push(lineIdx, col, len, tokenType, 0);
            prevEnd = col + len;
        }
    }
    return builder.build();
}
exports.provideDocumentSemanticTokens = provideDocumentSemanticTokens;
//# sourceMappingURL=type_infer.js.map