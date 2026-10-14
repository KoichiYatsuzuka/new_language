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

// containers are copied (ffi_value_exchange_plan.md 5-2)
function obj() { return { a: 1, b: "x" }; }          // a plain object -> dict[str, ...]
function map_obj() { return new Map([[1, "one"], [2, "two"]]); }
function set_obj() { return new Set([1, 2, 2, 3]); }
function buf() { return Buffer.from("hi"); }
function pair() { return [1, 2.0]; }                 // declared tuple[int, float]: JS has no tuples
function sum_values(d) { return Object.values(d).reduce((a, b) => a + b, 0); }
function kind(x) {
    if (x === null) return "null";
    if (Array.isArray(x)) return "array:" + x.length;
    if (x instanceof Map) return "Map:" + x.size;
    if (x instanceof Set) return "Set:" + x.size;
    if (x instanceof Uint8Array) return "bytes:" + x.length;
    return typeof x;
}

// values the bridge cannot carry
function fn_value() { return function () { return 1; }; }
function big_int() { return 10n; }
function instance() { return new Date(0); }          // an instance of a class: not a plain object

// arguments
function echo(x) { return x; }
function type_of(x) { return x === null ? "null" : Array.isArray(x) ? "array" : typeof x; }
function count(xs) { return xs.length; }
function utf8() { return "日本語"; }
function nothing() { return undefined; }

module.exports = {
    whole_float, half, ints, floats, nested, mixed, big, not_a_number, infinite,
    obj, map_obj, set_obj, buf, pair, sum_values, kind, fn_value, big_int, instance,
    echo, type_of, count, utf8, nothing,
};
