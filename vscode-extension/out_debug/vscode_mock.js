"use strict";
/**
 * Minimal VS Code API mock for standalone execution outside the extension host.
 * Implements only what analysis.ts / type_infer.ts actually use at runtime.
 * Used exclusively by debug_runner.ts — never imported by extension code.
 */
Object.defineProperty(exports, "__esModule", { value: true });
exports.commands = exports.languages = exports.window = exports.workspace = exports.Location = exports.SignatureHelp = exports.ParameterInformation = exports.SignatureInformation = exports.DocumentSymbol = exports.SymbolKind = exports.CompletionItem = exports.CompletionItemKind = exports.SemanticTokensBuilder = exports.SemanticTokens = exports.SemanticTokensLegend = exports.Diagnostic = exports.DiagnosticSeverity = exports.InlayHint = exports.InlayHintLabelPart = exports.InlayHintKind = exports.Hover = exports.MarkdownString = exports.Uri = exports.Range = exports.Position = void 0;
// ── Core geometry ────────────────────────────────────────────────────────────
class Position {
    constructor(line, character) {
        this.line = line;
        this.character = character;
    }
    translate(lineDelta = 0, characterDelta = 0) {
        return new Position(this.line + lineDelta, this.character + characterDelta);
    }
    with(line = this.line, character = this.character) {
        return new Position(line, character);
    }
    isBefore(o) {
        return this.line < o.line || (this.line === o.line && this.character < o.character);
    }
    isBeforeOrEqual(o) {
        return this.line === o.line ? this.character <= o.character : this.line < o.line;
    }
    isAfter(o) { return !this.isBeforeOrEqual(o); }
    isAfterOrEqual(o) { return !this.isBefore(o); }
    isEqual(o) {
        return this.line === o.line && this.character === o.character;
    }
    compareTo(o) {
        if (this.line !== o.line)
            return this.line - o.line;
        return this.character - o.character;
    }
}
exports.Position = Position;
class Range {
    constructor(a, b, c, d) {
        if (typeof a === 'number') {
            this.start = new Position(a, b);
            this.end = new Position(c, d);
        }
        else {
            this.start = a;
            this.end = b;
        }
    }
    get isEmpty() { return this.start.isEqual(this.end); }
    contains(pos) {
        return this.start.isBeforeOrEqual(pos) && pos.isBeforeOrEqual(this.end);
    }
}
exports.Range = Range;
class Uri {
    static file(p) { return new Uri(p); }
    static parse(v) { return new Uri(v); }
    constructor(_path) {
        this._path = _path;
    }
    get fsPath() { return this._path; }
    toString() { return `file://${this._path.replace(/\\/g, '/')}`; }
}
exports.Uri = Uri;
// ── MarkdownString / Hover ────────────────────────────────────────────────────
class MarkdownString {
    constructor(value = '', _trustFromDocumentBaseUri = false) {
        this.value = value;
    }
    appendMarkdown(v) { this.value += v; return this; }
    appendText(v) {
        this.value += v.replace(/[\\`*_{}[\]()#+\-.!]/g, '\\$&');
        return this;
    }
    appendCodeblock(v, language = '') {
        this.value += `\`\`\`${language}\n${v}\n\`\`\``;
        return this;
    }
}
exports.MarkdownString = MarkdownString;
class Hover {
    constructor(contents, range) {
        if (typeof contents === 'string') {
            this.contents = new MarkdownString(contents);
        }
        else {
            this.contents = contents;
        }
        this.range = range;
    }
}
exports.Hover = Hover;
// ── Inlay hints ───────────────────────────────────────────────────────────────
var InlayHintKind;
(function (InlayHintKind) {
    InlayHintKind[InlayHintKind["Type"] = 1] = "Type";
    InlayHintKind[InlayHintKind["Parameter"] = 2] = "Parameter";
})(InlayHintKind = exports.InlayHintKind || (exports.InlayHintKind = {}));
class InlayHintLabelPart {
    constructor(value) {
        this.value = value;
    }
}
exports.InlayHintLabelPart = InlayHintLabelPart;
class InlayHint {
    constructor(position, label, kind) {
        this.position = position;
        this.label = label;
        this.kind = kind;
    }
}
exports.InlayHint = InlayHint;
// ── Diagnostics ───────────────────────────────────────────────────────────────
var DiagnosticSeverity;
(function (DiagnosticSeverity) {
    DiagnosticSeverity[DiagnosticSeverity["Error"] = 0] = "Error";
    DiagnosticSeverity[DiagnosticSeverity["Warning"] = 1] = "Warning";
    DiagnosticSeverity[DiagnosticSeverity["Information"] = 2] = "Information";
    DiagnosticSeverity[DiagnosticSeverity["Hint"] = 3] = "Hint";
})(DiagnosticSeverity = exports.DiagnosticSeverity || (exports.DiagnosticSeverity = {}));
class Diagnostic {
    constructor(range, message, severity = DiagnosticSeverity.Error) {
        this.range = range;
        this.message = message;
        this.severity = severity;
    }
}
exports.Diagnostic = Diagnostic;
// ── Semantic tokens ───────────────────────────────────────────────────────────
class SemanticTokensLegend {
    constructor(tokenTypes, tokenModifiers) {
        this.tokenTypes = tokenTypes;
        this.tokenModifiers = tokenModifiers;
    }
}
exports.SemanticTokensLegend = SemanticTokensLegend;
class SemanticTokens {
    constructor(data, tokenList) {
        this.data = data;
        this.tokenList = tokenList;
    }
}
exports.SemanticTokens = SemanticTokens;
class SemanticTokensBuilder {
    constructor(_legend) {
        this._list = [];
    }
    push(line, char, len, tokenType, _mods = 0) {
        this._list.push({ line, char, len, tokenType });
    }
    build(_resultId) {
        // Encode to standard 5-integer-per-token delta format
        const data = [];
        let prevLine = 0, prevChar = 0;
        for (const t of this._list) {
            data.push(t.line - prevLine, t.line !== prevLine ? t.char : t.char - prevChar, t.len, t.tokenType, 0);
            prevLine = t.line;
            prevChar = t.char;
        }
        return new SemanticTokens(new Uint32Array(data), [...this._list]);
    }
}
exports.SemanticTokensBuilder = SemanticTokensBuilder;
// ── Completion / symbols / signatures ────────────────────────────────────────
var CompletionItemKind;
(function (CompletionItemKind) {
    CompletionItemKind[CompletionItemKind["Text"] = 0] = "Text";
    CompletionItemKind[CompletionItemKind["Method"] = 1] = "Method";
    CompletionItemKind[CompletionItemKind["Function"] = 2] = "Function";
    CompletionItemKind[CompletionItemKind["Constructor"] = 3] = "Constructor";
    CompletionItemKind[CompletionItemKind["Field"] = 4] = "Field";
    CompletionItemKind[CompletionItemKind["Variable"] = 5] = "Variable";
    CompletionItemKind[CompletionItemKind["Class"] = 6] = "Class";
    CompletionItemKind[CompletionItemKind["Interface"] = 7] = "Interface";
    CompletionItemKind[CompletionItemKind["Module"] = 8] = "Module";
    CompletionItemKind[CompletionItemKind["Property"] = 9] = "Property";
    CompletionItemKind[CompletionItemKind["Unit"] = 10] = "Unit";
    CompletionItemKind[CompletionItemKind["Value"] = 11] = "Value";
    CompletionItemKind[CompletionItemKind["Enum"] = 12] = "Enum";
    CompletionItemKind[CompletionItemKind["Keyword"] = 13] = "Keyword";
    CompletionItemKind[CompletionItemKind["Snippet"] = 14] = "Snippet";
    CompletionItemKind[CompletionItemKind["Color"] = 15] = "Color";
    CompletionItemKind[CompletionItemKind["File"] = 16] = "File";
    CompletionItemKind[CompletionItemKind["Reference"] = 17] = "Reference";
    CompletionItemKind[CompletionItemKind["Folder"] = 18] = "Folder";
    CompletionItemKind[CompletionItemKind["EnumMember"] = 19] = "EnumMember";
    CompletionItemKind[CompletionItemKind["Constant"] = 20] = "Constant";
    CompletionItemKind[CompletionItemKind["Struct"] = 21] = "Struct";
    CompletionItemKind[CompletionItemKind["Event"] = 22] = "Event";
    CompletionItemKind[CompletionItemKind["Operator"] = 23] = "Operator";
    CompletionItemKind[CompletionItemKind["TypeParameter"] = 24] = "TypeParameter";
})(CompletionItemKind = exports.CompletionItemKind || (exports.CompletionItemKind = {}));
class CompletionItem {
    constructor(label, kind) {
        this.label = label;
        this.kind = kind;
    }
}
exports.CompletionItem = CompletionItem;
var SymbolKind;
(function (SymbolKind) {
    SymbolKind[SymbolKind["File"] = 0] = "File";
    SymbolKind[SymbolKind["Module"] = 1] = "Module";
    SymbolKind[SymbolKind["Namespace"] = 2] = "Namespace";
    SymbolKind[SymbolKind["Package"] = 3] = "Package";
    SymbolKind[SymbolKind["Class"] = 4] = "Class";
    SymbolKind[SymbolKind["Method"] = 5] = "Method";
    SymbolKind[SymbolKind["Property"] = 6] = "Property";
    SymbolKind[SymbolKind["Field"] = 7] = "Field";
    SymbolKind[SymbolKind["Constructor"] = 8] = "Constructor";
    SymbolKind[SymbolKind["Enum"] = 9] = "Enum";
    SymbolKind[SymbolKind["Interface"] = 10] = "Interface";
    SymbolKind[SymbolKind["Function"] = 11] = "Function";
    SymbolKind[SymbolKind["Variable"] = 12] = "Variable";
    SymbolKind[SymbolKind["Constant"] = 13] = "Constant";
    SymbolKind[SymbolKind["String"] = 14] = "String";
    SymbolKind[SymbolKind["Number"] = 15] = "Number";
    SymbolKind[SymbolKind["Boolean"] = 16] = "Boolean";
    SymbolKind[SymbolKind["Array"] = 17] = "Array";
    SymbolKind[SymbolKind["Object"] = 18] = "Object";
    SymbolKind[SymbolKind["Key"] = 19] = "Key";
    SymbolKind[SymbolKind["Null"] = 20] = "Null";
    SymbolKind[SymbolKind["EnumMember"] = 21] = "EnumMember";
    SymbolKind[SymbolKind["Struct"] = 22] = "Struct";
    SymbolKind[SymbolKind["Event"] = 23] = "Event";
    SymbolKind[SymbolKind["Operator"] = 24] = "Operator";
    SymbolKind[SymbolKind["TypeParameter"] = 25] = "TypeParameter";
})(SymbolKind = exports.SymbolKind || (exports.SymbolKind = {}));
class DocumentSymbol {
    constructor(name, detail, kind, range, selectionRange) {
        this.name = name;
        this.detail = detail;
        this.kind = kind;
        this.range = range;
        this.selectionRange = selectionRange;
        this.children = [];
    }
}
exports.DocumentSymbol = DocumentSymbol;
class SignatureInformation {
    constructor(label, documentation) {
        this.label = label;
        this.parameters = [];
        this.documentation = documentation;
    }
}
exports.SignatureInformation = SignatureInformation;
class ParameterInformation {
    constructor(label, documentation) {
        this.label = label;
        this.documentation = documentation;
    }
}
exports.ParameterInformation = ParameterInformation;
class SignatureHelp {
    constructor() {
        this.signatures = [];
        this.activeSignature = 0;
        this.activeParameter = 0;
    }
}
exports.SignatureHelp = SignatureHelp;
class Location {
    constructor(uri, range) {
        this.uri = uri;
        this.range = range;
    }
}
exports.Location = Location;
// ── Workspace / window / languages stubs ─────────────────────────────────────
exports.workspace = {
    getConfiguration(_section) {
        return {
            get(_key, defaultValue) { return defaultValue; },
        };
    },
    textDocuments: [],
    onDidOpenTextDocument() { return { dispose() { } }; },
    onDidChangeTextDocument() { return { dispose() { } }; },
    onDidCloseTextDocument() { return { dispose() { } }; },
};
exports.window = {
    showWarningMessage(msg) { process.stderr.write(`[WARN] ${msg}\n`); },
    showErrorMessage(msg) { process.stderr.write(`[ERROR] ${msg}\n`); },
    showInformationMessage(msg) { process.stderr.write(`[INFO] ${msg}\n`); },
    activeTextEditor: undefined,
    terminals: [],
    createOutputChannel(_name) {
        return { appendLine() { }, show() { }, clear() { }, dispose() { } };
    },
    onDidCloseTerminal() { return { dispose() { } }; },
    createTerminal() { return { sendText() { }, show() { }, dispose() { } }; },
};
exports.languages = {
    registerInlayHintsProvider() { return { dispose() { } }; },
    registerHoverProvider() { return { dispose() { } }; },
    registerDocumentSemanticTokensProvider() { return { dispose() { } }; },
    registerCompletionItemProvider() { return { dispose() { } }; },
    registerDocumentSymbolProvider() { return { dispose() { } }; },
    registerSignatureHelpProvider() { return { dispose() { } }; },
    registerDefinitionProvider() { return { dispose() { } }; },
    createDiagnosticCollection() { return { set() { }, delete() { }, dispose() { } }; },
};
exports.commands = {
    registerCommand(_cmd, _cb) { return { dispose() { } }; },
};
//# sourceMappingURL=vscode_mock.js.map