/**
 * Minimal VS Code API mock for standalone execution outside the extension host.
 * Implements only what analysis.ts / type_infer.ts actually use at runtime.
 * Used exclusively by debug_runner.ts — never imported by extension code.
 */

// ── Core geometry ────────────────────────────────────────────────────────────

export class Position {
    constructor(readonly line: number, readonly character: number) {}
    translate(lineDelta = 0, characterDelta = 0): Position {
        return new Position(this.line + lineDelta, this.character + characterDelta);
    }
    with(line = this.line, character = this.character): Position {
        return new Position(line, character);
    }
    isBefore(o: Position): boolean {
        return this.line < o.line || (this.line === o.line && this.character < o.character);
    }
    isBeforeOrEqual(o: Position): boolean {
        return this.line === o.line ? this.character <= o.character : this.line < o.line;
    }
    isAfter(o: Position): boolean { return !this.isBeforeOrEqual(o); }
    isAfterOrEqual(o: Position): boolean { return !this.isBefore(o); }
    isEqual(o: Position): boolean {
        return this.line === o.line && this.character === o.character;
    }
    compareTo(o: Position): number {
        if (this.line !== o.line) return this.line - o.line;
        return this.character - o.character;
    }
}

export class Range {
    readonly start: Position;
    readonly end: Position;

    constructor(start: Position, end: Position);
    constructor(startLine: number, startChar: number, endLine: number, endChar: number);
    constructor(a: Position | number, b: Position | number, c?: number, d?: number) {
        if (typeof a === 'number') {
            this.start = new Position(a, b as number);
            this.end   = new Position(c!, d!);
        } else {
            this.start = a;
            this.end   = b as Position;
        }
    }
    get isEmpty(): boolean { return this.start.isEqual(this.end); }
    contains(pos: Position): boolean {
        return this.start.isBeforeOrEqual(pos) && pos.isBeforeOrEqual(this.end);
    }
}

export class Uri {
    static file(p: string): Uri { return new Uri(p); }
    static parse(v: string): Uri { return new Uri(v); }
    constructor(private readonly _path: string) {}
    get fsPath(): string { return this._path; }
    toString(): string { return `file://${this._path.replace(/\\/g, '/')}`; }
}

// ── MarkdownString / Hover ────────────────────────────────────────────────────

export class MarkdownString {
    value: string;
    isTrusted?: boolean;

    constructor(value = '', _trustFromDocumentBaseUri = false) {
        this.value = value;
    }
    appendMarkdown(v: string): this { this.value += v; return this; }
    appendText(v: string): this {
        this.value += v.replace(/[\\`*_{}[\]()#+\-.!]/g, '\\$&');
        return this;
    }
    appendCodeblock(v: string, language = ''): this {
        this.value += `\`\`\`${language}\n${v}\n\`\`\``;
        return this;
    }
}

export class Hover {
    contents: MarkdownString | MarkdownString[];
    range?: Range;
    constructor(contents: MarkdownString | string | MarkdownString[], range?: Range) {
        if (typeof contents === 'string') {
            this.contents = new MarkdownString(contents);
        } else {
            this.contents = contents;
        }
        this.range = range;
    }
}

// ── Inlay hints ───────────────────────────────────────────────────────────────

export enum InlayHintKind { Type = 1, Parameter = 2 }

export class InlayHintLabelPart {
    constructor(readonly value: string) {}
}

export class InlayHint {
    label: string | InlayHintLabelPart[];
    kind?: InlayHintKind;
    paddingLeft?: boolean;
    paddingRight?: boolean;

    constructor(readonly position: Position, label: string | InlayHintLabelPart[], kind?: InlayHintKind) {
        this.label = label;
        this.kind  = kind;
    }
}

// ── Diagnostics ───────────────────────────────────────────────────────────────

export enum DiagnosticSeverity { Error = 0, Warning = 1, Information = 2, Hint = 3 }

export class Diagnostic {
    source?: string;
    tags?: number[];
    constructor(
        readonly range: Range,
        readonly message: string,
        readonly severity: DiagnosticSeverity = DiagnosticSeverity.Error,
    ) {}
}

// ── Semantic tokens ───────────────────────────────────────────────────────────

export class SemanticTokensLegend {
    constructor(readonly tokenTypes: readonly string[], readonly tokenModifiers: readonly string[]) {}
}

export interface SemanticTokenEntry {
    line: number; char: number; len: number; tokenType: number;
}

export class SemanticTokens {
    // Standard encoded data (delta-compressed)
    readonly data: Uint32Array;
    // Non-standard convenience list, used only by debug_runner
    readonly tokenList: SemanticTokenEntry[];

    constructor(data: Uint32Array, tokenList: SemanticTokenEntry[]) {
        this.data      = data;
        this.tokenList = tokenList;
    }
}

export class SemanticTokensBuilder {
    private readonly _list: SemanticTokenEntry[] = [];
    constructor(_legend?: SemanticTokensLegend) {}

    push(line: number, char: number, len: number, tokenType: number, _mods = 0): void {
        this._list.push({ line, char, len, tokenType });
    }

    build(_resultId?: string): SemanticTokens {
        // Encode to standard 5-integer-per-token delta format
        const data: number[] = [];
        let prevLine = 0, prevChar = 0;
        for (const t of this._list) {
            data.push(
                t.line - prevLine,
                t.line !== prevLine ? t.char : t.char - prevChar,
                t.len, t.tokenType, 0,
            );
            prevLine = t.line;
            prevChar = t.char;
        }
        return new SemanticTokens(new Uint32Array(data), [...this._list]);
    }
}

// ── Completion / symbols / signatures ────────────────────────────────────────

export enum CompletionItemKind {
    Text = 0, Method = 1, Function = 2, Constructor = 3, Field = 4, Variable = 5,
    Class = 6, Interface = 7, Module = 8, Property = 9, Unit = 10, Value = 11,
    Enum = 12, Keyword = 13, Snippet = 14, Color = 15, File = 16, Reference = 17,
    Folder = 18, EnumMember = 19, Constant = 20, Struct = 21, Event = 22,
    Operator = 23, TypeParameter = 24,
}

export class CompletionItem {
    detail?: string;
    documentation?: MarkdownString | string;
    insertText?: string;
    kind?: CompletionItemKind;
    constructor(readonly label: string, kind?: CompletionItemKind) { this.kind = kind; }
}

export enum SymbolKind {
    File = 0, Module = 1, Namespace = 2, Package = 3, Class = 4, Method = 5,
    Property = 6, Field = 7, Constructor = 8, Enum = 9, Interface = 10,
    Function = 11, Variable = 12, Constant = 13, String = 14, Number = 15,
    Boolean = 16, Array = 17, Object = 18, Key = 19, Null = 20, EnumMember = 21,
    Struct = 22, Event = 23, Operator = 24, TypeParameter = 25,
}

export class DocumentSymbol {
    children: DocumentSymbol[] = [];
    constructor(
        readonly name: string,
        readonly detail: string,
        readonly kind: SymbolKind,
        readonly range: Range,
        readonly selectionRange: Range,
    ) {}
}

export class SignatureInformation {
    parameters: ParameterInformation[] = [];
    documentation?: MarkdownString | string;
    constructor(readonly label: string, documentation?: MarkdownString | string) {
        this.documentation = documentation;
    }
}

export class ParameterInformation {
    documentation?: string;
    constructor(readonly label: string | [number, number], documentation?: string) {
        this.documentation = documentation;
    }
}

export class SignatureHelp {
    signatures: SignatureInformation[] = [];
    activeSignature = 0;
    activeParameter = 0;
}

export class Location {
    constructor(readonly uri: Uri, readonly range: Range) {}
}

// ── Workspace / window / languages stubs ─────────────────────────────────────

export const workspace = {
    getConfiguration(_section?: string) {
        return {
            get<T>(_key: string, defaultValue?: T): T { return defaultValue as T; },
        };
    },
    textDocuments: [] as unknown[],
    onDidOpenTextDocument() { return { dispose() {} }; },
    onDidChangeTextDocument() { return { dispose() {} }; },
    onDidCloseTextDocument() { return { dispose() {} }; },
};

export const window = {
    showWarningMessage(msg: string): void { process.stderr.write(`[WARN] ${msg}\n`); },
    showErrorMessage(msg: string): void   { process.stderr.write(`[ERROR] ${msg}\n`); },
    showInformationMessage(msg: string): void { process.stderr.write(`[INFO] ${msg}\n`); },
    activeTextEditor: undefined as unknown,
    terminals: [] as unknown[],
    createOutputChannel(_name: string) {
        return { appendLine() {}, show() {}, clear() {}, dispose() {} };
    },
    onDidCloseTerminal() { return { dispose() {} }; },
    createTerminal() { return { sendText() {}, show() {}, dispose() {} }; },
};

export const languages = {
    registerInlayHintsProvider()             { return { dispose() {} }; },
    registerHoverProvider()                  { return { dispose() {} }; },
    registerDocumentSemanticTokensProvider() { return { dispose() {} }; },
    registerCompletionItemProvider()         { return { dispose() {} }; },
    registerDocumentSymbolProvider()         { return { dispose() {} }; },
    registerSignatureHelpProvider()          { return { dispose() {} }; },
    registerDefinitionProvider()             { return { dispose() {} }; },
    createDiagnosticCollection()             { return { set() {}, delete() {}, dispose() {} }; },
};

export const commands = {
    registerCommand(_cmd: string, _cb: unknown) { return { dispose() {} }; },
};
