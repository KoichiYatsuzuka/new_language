"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.tokenize = void 0;
function tokenize(src) {
    var _a, _b;
    const tokens = [];
    let i = 0;
    while (i < src.length) {
        if (' \t\r\n'.includes(src[i])) {
            i++;
            continue;
        }
        if (src[i] === '"' || src[i] === "'") {
            const q = src[i];
            const triple = src.startsWith(q + q + q, i);
            let j = i + (triple ? 3 : 1);
            while (j < src.length) {
                if (src[j] === '\\') {
                    j += 2;
                    continue;
                }
                if (triple ? src.startsWith(q + q + q, j) : src[j] === q) {
                    j += triple ? 3 : 1;
                    break;
                }
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
                while (j < src.length && /[\da-fA-F_]/.test(src[j]))
                    j++;
                tokens.push({ kind: 'INT', value: src.slice(i, j) });
            }
            else {
                while (j < src.length && /[\d_]/.test(src[j]))
                    j++;
                let isFloat = false;
                if (j < src.length && src[j] === '.' && j + 1 < src.length && /\d/.test(src[j + 1])) {
                    isFloat = true;
                    j++;
                    while (j < src.length && /[\d_]/.test(src[j]))
                        j++;
                }
                if (j < src.length && 'eE'.includes(src[j])) {
                    isFloat = true;
                    j++;
                    if (j < src.length && '+-'.includes(src[j]))
                        j++;
                    while (j < src.length && /\d/.test(src[j]))
                        j++;
                }
                tokens.push({ kind: isFloat ? 'FLOAT' : 'INT', value: src.slice(i, j) });
            }
            i = j;
            continue;
        }
        if (/[A-Za-z_]/.test(src[i])) {
            let j = i;
            while (j < src.length && /\w/.test(src[j]))
                j++;
            const word = src.slice(i, j);
            const keywordMap = {
                True: 'TRUE', False: 'FALSE', None: 'NONE',
                and: 'AND', or: 'OR', not: 'NOT',
            };
            tokens.push({ kind: (_a = keywordMap[word]) !== null && _a !== void 0 ? _a : 'IDENT', value: word });
            i = j;
            continue;
        }
        const s3 = src.slice(i, i + 3);
        if (['//=', '**=', '<<=', '>>='].includes(s3)) {
            tokens.push({ kind: 'OTHER', value: s3 });
            i += 3;
            continue;
        }
        const s2 = src.slice(i, i + 2);
        const op2 = {
            '**': 'STARSTAR', '//': 'SLASHSLASH', '==': 'EQEQ', '!=': 'NOTEQ',
            '<=': 'LTEQ', '>=': 'GTEQ', '<<': 'LTLT', '>>': 'GTGT',
        };
        if (op2[s2]) {
            tokens.push({ kind: op2[s2], value: s2 });
            i += 2;
            continue;
        }
        if (['+=', '-=', '*=', '/=', '%=', '&=', '|=', '^=', '@=', '->', ':='].includes(s2)) {
            tokens.push({ kind: 'OTHER', value: s2 });
            i += 2;
            continue;
        }
        const op1 = {
            '+': 'PLUS', '-': 'MINUS', '*': 'STAR', '/': 'SLASH', '%': 'PERCENT',
            '<': 'LT', '>': 'GT', '&': 'AMP', '|': 'PIPE', '^': 'CARET', '~': 'TILDE',
            '(': 'LPAREN', ')': 'RPAREN', ',': 'COMMA',
            '[': 'LBRACKET', ']': 'RBRACKET', '{': 'LBRACE', '}': 'RBRACE', ':': 'COLON',
        };
        tokens.push({ kind: (_b = op1[src[i]]) !== null && _b !== void 0 ? _b : 'OTHER', value: src[i] });
        i++;
    }
    tokens.push({ kind: 'EOF', value: '' });
    return tokens;
}
exports.tokenize = tokenize;
//# sourceMappingURL=tokenizer.js.map