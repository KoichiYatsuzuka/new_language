// js_text.js — js_proc_test.ar / js_proc_async_test.ar が import[js-proc] で呼ぶ JS モジュール。
// 型は隣の js_text.ars に書く。

function stripComment(line) {
    const i = line.indexOf('#');
    return (i < 0 ? line : line.slice(0, i)).trimEnd();
}

function cleanTypeAnnotation(text) {
    return text.trim();
}

function splitComma(text) {
    return text.split(',').map(s => s.trim()).filter(s => s.length > 0);
}

module.exports = { stripComment, cleanTypeAnnotation, splitComma };
