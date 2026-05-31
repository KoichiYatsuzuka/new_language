export type TK =
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

export interface Token { kind: TK; value: string; }

export function tokenize(src: string): Token[] {
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
