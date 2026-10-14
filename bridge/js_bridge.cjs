'use strict';
// js_bridge.cjs — Node.js named-pipe IPC server for Arrow import[js-proc].
//
// Usage (called by Arrow runtime):
//   node js_bridge.cjs <pipe_name> <bridge_root>
//
// Protocol: newline-delimited JSON over a Windows named pipe.
//   Request:  {"id":N,"op":"call"|"list"|"quit","module":"a/b","fn":"fnName","args":[...]}
//   Response: {"id":N,"ok":{t,v}} | {"id":N,"err":"message"}
//
// Arg/result type tags:
//   "i" = int (a safe integer, |x| <= 2^53 - 1)    "f" = float (any other number)    "b" = bool
//   "s" = string "n" = null/undefined
//   "a" = array  "o" = (result only) a plain object, {key: {t,v}, ...} (Arrow: dict[str, ...])
//   "u" = (argument only) an Arrow tuple -> a JS array (JS has no tuples)
//   "d" = dict, [[key, value], ...]: an Arrow dict becomes a plain object when every key is a string, else a Map;
//         a JS Map comes back as "d"
//   "e" = set <-> Set    "y" = bytes <-> Buffer / Uint8Array (base64)          (ffi_value_exchange_plan.md 5-2)
//   NaN / Infinity / -Infinity travel as "f" with the string "NaN" / "Infinity" / "-Infinity" (JSON has no such numbers)
//   "x" = (result only) a value that cannot be carried (function / symbol / BigInt / an instance of a class), with its
//         typeof or class name as the payload. Arrow raises TypeError for it; these used to become strings silently
//         (ffi_value_exchange_plan.md 2-3).

const net  = require('net');
const { StringDecoder } = require('string_decoder');
const path = require('path');
const fs   = require('fs');

const PIPE_NAME  = process.argv[2];
const BRIDGE_ROOT = path.resolve(process.argv[3] || process.cwd());

if (!PIPE_NAME) {
    process.stderr.write('js_bridge: missing pipe_name argument\n');
    process.exit(1);
}

// ── vscode mock intercept ─────────────────────────────────────────────────────
// Redirect require('vscode') to out_debug/vscode_mock so that VS Code extension
// modules (analysis.js, type_infer.js, etc.) work outside the extension host.
const Module = require('module');
const _origLoad = Module._load.bind(Module);
Module._load = function (request, parent, isMain) {
    if (request === 'vscode') {
        const mockPath = path.join(BRIDGE_ROOT, 'out_debug', 'vscode_mock');
        try { return _origLoad(mockPath, parent, isMain); } catch (_) { return {}; }
    }
    return _origLoad(request, parent, isMain);
};

// ── Module registry ───────────────────────────────────────────────────────────
const loadedModules = {};

function loadModule(moduleName) {
    if (loadedModules[moduleName]) return loadedModules[moduleName];

    // Convert dot/slash separators to OS path separators and resolve.
    const relPath = moduleName.replace(/\./g, '/');
    const candidates = [
        path.resolve(BRIDGE_ROOT, relPath),
        path.resolve(BRIDGE_ROOT, relPath + '.js'),
        path.resolve(BRIDGE_ROOT, relPath + '.cjs'),
        // Also search in the bridge script's own directory
        path.resolve(__dirname, relPath),
        path.resolve(__dirname, relPath + '.js'),
        path.resolve(__dirname, relPath + '.cjs'),
        moduleName,   // bare npm package / Node.js built-in
    ];

    let mod, lastErr;
    for (const c of candidates) {
        try { mod = require(c); break; } catch (e) { lastErr = e; }
    }
    if (mod === undefined) throw lastErr;

    loadedModules[moduleName] = mod;
    return mod;
}

// ── Encoding / decoding ───────────────────────────────────────────────────────
function encodeValue(val) {
    if (val === null || val === undefined) return { t: 'n' };
    if (typeof val === 'boolean') return { t: 'b', v: val };
    if (typeof val === 'number') {
        if (Number.isNaN(val)) return { t: 'f', v: 'NaN' };
        if (!Number.isFinite(val)) return { t: 'f', v: val > 0 ? 'Infinity' : '-Infinity' };
        // Only safe integers go as "i": beyond 2^53 JSON.stringify prints the shortest round-trip digits
        // (2**60 -> "1152921504606847000"), which is not the integer the double holds. Those go as "f" and Arrow
        // turns them back into an exact int when the stub declares int.
        return Number.isSafeInteger(val) ? { t: 'i', v: val } : { t: 'f', v: val };
    }
    if (typeof val === 'string') return { t: 's', v: val };
    if (Array.isArray(val)) return { t: 'a', v: val.map(encodeValue) };
    if (val instanceof Map) return { t: 'd', v: [...val.entries()].map(([k, v]) => [encodeValue(k), encodeValue(v)]) };
    if (val instanceof Set) return { t: 'e', v: [...val].map(encodeValue) };
    if (val instanceof Uint8Array) return { t: 'y', v: Buffer.from(val.buffer, val.byteOffset, val.byteLength).toString('base64') };
    if (typeof val === 'object') {
        // Only plain objects become a dict. An instance of a class (Date, a user class ...) has behaviour that a dict
        // would silently drop, so it cannot be carried (5-2).
        const proto = Object.getPrototypeOf(val);
        if (proto !== Object.prototype && proto !== null) {
            return { t: 'x', v: `object of class ${(val.constructor && val.constructor.name) || '?'}` };
        }
        const obj = {};
        for (const [k, v] of Object.entries(val)) obj[k] = encodeValue(v);
        return { t: 'o', v: obj };
    }
    return { t: 'x', v: typeof val };
}

function decodeArg(a) {
    if (!a || typeof a.t !== 'string') throw new Error(`argument without a type tag: ${JSON.stringify(a)}`);
    switch (a.t) {
        case 'n': return null;
        case 'b': return Boolean(a.v);
        case 'i': case 'f': return Number(a.v);   // Number("NaN") / Number("Infinity") work too
        case 's': return String(a.v);
        case 'a': case 'u': return (a.v || []).map(decodeArg);
        case 'e': return new Set((a.v || []).map(decodeArg));
        case 'y': return Buffer.from(a.v || '', 'base64');
        case 'd': {
            const pairs = (a.v || []).map(([k, v]) => [decodeArg(k), decodeArg(v)]);
            // A plain object when every key is a string (what JS APIs usually take), else a Map
            if (pairs.every(([k]) => typeof k === 'string')) {
                const obj = {};
                for (const [k, v] of pairs) obj[k] = v;
                return obj;
            }
            return new Map(pairs);
        }
        default: throw new Error(`unknown argument type tag '${a.t}'`);
    }
}

// ── Request handler ───────────────────────────────────────────────────────────
async function handleRequest(req) {
    switch (req.op) {
        case 'list': {
            const mod = loadModule(req.module);
            const fns = Object.keys(mod).filter(k => typeof mod[k] === 'function');
            return { t: 'a', v: fns.map(f => ({ t: 's', v: f })) };
        }
        case 'call': {
            const mod = loadModule(req.module);
            const fn  = mod[req.fn];
            if (typeof fn !== 'function')
                throw new Error(`'${req.fn}' is not a function in module '${req.module}'`);
            const args   = (req.args || []).map(decodeArg);
            const result = await Promise.resolve(fn.apply(mod, args));
            return encodeValue(result);
        }
        case 'quit':
            return { t: 'n' };
        default:
            throw new Error(`unknown op: ${req.op}`);
    }
}

// ── Named-pipe server ─────────────────────────────────────────────────────────
const server = net.createServer((socket) => {
    let buf = '';
    // A multi-byte UTF-8 character can be split across two chunks; StringDecoder keeps the partial bytes
    // (chunk.toString('utf8') turned each half into U+FFFD).
    const decoder = new StringDecoder('utf8');

    socket.on('data', (chunk) => {
        buf += decoder.write(chunk);
        let nl;
        while ((nl = buf.indexOf('\n')) !== -1) {
            const line = buf.slice(0, nl).trim();
            buf = buf.slice(nl + 1);
            if (!line) continue;

            let req;
            try { req = JSON.parse(line); }
            catch (e) {
                socket.write(JSON.stringify({ id: 0, err: `JSON parse: ${e.message}` }) + '\n');
                continue;
            }

            handleRequest(req)
                .then(result => {
                    socket.write(JSON.stringify({ id: req.id, ok: result }) + '\n');
                    if (req.op === 'quit') {
                        socket.end();
                        server.close();
                        process.exit(0);
                    }
                })
                .catch(err => {
                    socket.write(JSON.stringify({ id: req.id, err: String(err) }) + '\n');
                });
        }
    });

    socket.on('error', (e) => process.stderr.write(`js_bridge socket error: ${e}\n`));

    // Arrow is the only client of this bridge. When it disconnects - normally, or because it crashed or was
    // killed - there is nobody left to serve, so exit (ffi_value_exchange_plan.md 2-4). The bridge used to keep
    // listening: Arrow keeps its bridges in a process-wide table that is never dropped, so the "quit" request
    // was never sent and one node process was left behind per run (holding the parent's stderr open, which
    // made a gate script wait forever).
    socket.on('close', () => {
        server.close();
        process.exit(0);
    });
});

server.on('error', (e) => {
    process.stderr.write(`js_bridge server error: ${e}\n`);
    process.exit(1);
});

// If Arrow never connects (it died between spawning us and opening the pipe), do not wait forever.
const CONNECT_TIMEOUT_MS = 60 * 1000;
const connectTimer = setTimeout(() => {
    process.stderr.write(`js_bridge: no connection within ${CONNECT_TIMEOUT_MS / 1000} s, exiting\n`);
    process.exit(1);
}, CONNECT_TIMEOUT_MS);
server.on('connection', () => clearTimeout(connectTimer));

server.listen(PIPE_NAME, () => {
    // Signal Arrow runtime that the pipe is ready.
    process.stdout.write('READY\n');
});
