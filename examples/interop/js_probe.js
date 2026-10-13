// js_probe.js — js-proc の値の受け渡しを確かめる JS モジュール（ffi_value_exchange_plan.md 2-3）。
// 型は隣の js_probe.ars に書く。Keep this file ASCII-only except for utf8().

// numbers: JS has only doubles, so 2.0 is indistinguishable from 2
function whole_float() { return 2.0; }
function half() { return 2.5; }
function ints() { return [1, 2, 3]; }
function floats() { return [1.0, 2.5]; }
function nested() { return [[1, 2], [3]]; }
function mixed() { return [1, "two"]; }          // declared list[int]: a lying stub
function big() { return 1e20; }                  // an integer that does not fit in int64
function not_a_number() { return NaN; }
function infinite() { return -Infinity; }

// values the bridge cannot carry yet (phase 5-2)
function obj() { return { a: 1, b: "x" }; }
function fn_value() { return function () { return 1; }; }
function big_int() { return 10n; }

// arguments
function echo(x) { return x; }
function type_of(x) { return x === null ? "null" : Array.isArray(x) ? "array" : typeof x; }
function count(xs) { return xs.length; }
function utf8() { return "日本語"; }
function nothing() { return undefined; }

module.exports = {
    whole_float, half, ints, floats, nested, mixed, big, not_a_number, infinite,
    obj, fn_value, big_int, echo, type_of, count, utf8, nothing,
};
