"""Rust crate importer (mirrors src/partial_compiler/rs_loader.rs).

Pipeline:
  1. find_config()        – read ar_config.json → crates_path
  2. find_crate_dir()     – pick versioned directory
  3. prepare_wrapper()    – create temp Cargo project, resolve crate src via `cargo metadata`
  4. scan_all_sigs()      – regex-scan .rs source for pub fn / pub struct
  5. make_stubs()         – emit StmtFnDef / StmtClassDef for the type checker
  6. generate_wrapper_lib_rs() – produce the _tl-suffixed Rust wrapper source
  7. compile_cdylib()     – cargo build --release → DLL bytes
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


# ---------------------------------------------------------------------------
# Data types
# ---------------------------------------------------------------------------

@dataclass
class RsParam:
    name: str
    rust_type: str


@dataclass
class RsFnSig:
    name: str
    params: list[RsParam]
    return_type: Optional[str]          # None means ()
    digest_type: Optional[str] = None  # set for RustCrypto digest wrappers


@dataclass
class RsField:
    name: str
    rust_type: str


@dataclass
class RsMethodSig:
    name: str
    params: list[RsParam]
    self_mutable: bool
    return_type: Optional[str]
    return_struct: Optional[str]        # return type is a struct in this crate


@dataclass
class RsStructSig:
    name: str
    fields: list[RsField]
    methods: list[RsMethodSig]
    ctor_params: list[RsParam]
    use_new_fn: bool


# ---------------------------------------------------------------------------
# ABI compatibility
# ---------------------------------------------------------------------------

_FIXED_BYTE_RE = re.compile(r'^\[u8;\s*\d+\]$')

def is_abi_compatible(t: str) -> bool:
    t = t.strip()
    if t in {
        "i8", "i16", "i32", "i64", "i128", "isize",
        "u8", "u16", "u32", "u64", "u128", "usize",
        "f32", "f64",
        "bool",
        "String", "&str", "&String",
        "&[u8]",
        "Vec<u8>",
    }:
        return True
    return bool(_FIXED_BYTE_RE.match(t))


def rust_type_to_ar(t: str) -> str:
    t = t.strip()
    if t in {"i8", "i16", "i32", "i64", "i128", "isize",
             "u8", "u16", "u32", "u64", "u128", "usize"}:
        return "int"
    if t in {"f32", "f64"}:
        return "float"
    if t == "bool":
        return "bool"
    if t in {"String", "&str", "&String", "&[u8]", "Vec<u8>"}:
        return "str"
    if _FIXED_BYTE_RE.match(t):
        return "str"
    return "Any"


# ---------------------------------------------------------------------------
# Config / crate directory resolution
# ---------------------------------------------------------------------------

def find_config(search_dirs: list[Path]) -> list[str]:
    """Read ar_config.json from the first directory that has one.

    Returns crates_path entries as absolute path strings.
    Relative paths in the JSON are resolved relative to the config file's directory.
    Raises RuntimeError if no config is found.
    """
    cwd = Path.cwd()
    dirs = list(search_dirs)
    if cwd not in dirs:
        dirs.append(cwd)

    for d in dirs:
        cfg = d / "ar_config.json"
        if not cfg.exists():
            continue
        try:
            obj = json.loads(cfg.read_text(encoding="utf-8"))
        except Exception as e:
            raise RuntimeError(f"import[rs]: cannot parse '{cfg}': {e}") from e
        rust = obj.get("rust", {})
        cp = rust.get("crates_path")
        if cp is None:
            raise RuntimeError(
                f"import[rs]: '{cfg}' has no 'rust.crates_path' key"
            )
        cfg_dir = cfg.parent

        def _resolve(p: str) -> str:
            resolved = Path(p)
            if not resolved.is_absolute():
                resolved = cfg_dir / resolved
            return str(resolved.resolve())

        if isinstance(cp, str):
            return [_resolve(cp)]
        if isinstance(cp, list):
            return [_resolve(str(p)) for p in cp]
        raise RuntimeError(f"import[rs]: 'rust.crates_path' must be a string or list")

    raise RuntimeError(
        "import[rs]: ar_config.json not found — "
        "add {\"rust\": {\"crates_path\": \"...\"}} next to your .ar file"
    )


def find_crate_dir(crates_paths: list[str], name: str, version: Optional[str]) -> Path:
    """Locate a crate directory under the given registry paths.

    Looks for a versioned directory '{name}-*', or a bare directory named '{name}'.
    Picks the latest version unless `version` is given.
    """
    for cp in crates_paths:
        base = Path(cp)
        if not base.is_dir():
            continue

        versioned = sorted(
            [d for d in base.iterdir() if d.is_dir() and d.name.startswith(f"{name}-")],
            key=lambda d: d.name,
        )
        if versioned:
            if version:
                match = next((d for d in versioned if version in d.name), None)
                if match:
                    return match
            # Latest by name sort
            return versioned[-1]

        bare = base / name
        if bare.is_dir():
            return bare

    paths_str = ", ".join(crates_paths)
    raise RuntimeError(
        f"import[rs]: crate '{name}' not found in crates_path ({paths_str})"
    )


# ---------------------------------------------------------------------------
# Wrapper project preparation
# ---------------------------------------------------------------------------

def prepare_wrapper(crate_dir: Path, stem: str, tmp: Path) -> tuple[Path, str]:
    """Create a temp Cargo project that depends on the crate.

    Returns (crate_src_dir, crate_ident).
    crate_ident is the crate name with '-' replaced by '_'.
    """
    src_dir = tmp / "src"
    src_dir.mkdir(parents=True, exist_ok=True)
    (src_dir / "lib.rs").write_text("", encoding="utf-8")

    # Determine crate name from Cargo.toml
    cargo_toml = crate_dir / "Cargo.toml"
    crate_name = stem  # fallback
    if cargo_toml.exists():
        for line in cargo_toml.read_text(encoding="utf-8").splitlines():
            m = re.match(r'^name\s*=\s*"([^"]+)"', line)
            if m:
                crate_name = m.group(1)
                break

    crate_ident = crate_name.replace("-", "_")

    # Use forward slashes in the path (works on all platforms for Cargo)
    abs_path = crate_dir.resolve()
    raw = str(abs_path)
    # Strip Windows extended-path prefix
    stripped = raw.lstrip("\\\\?\\").replace("\\\\?\\", "")
    path_str = stripped.replace("\\", "/")

    toml_content = (
        f'[package]\nname="ar_rs_{stem}"\nversion="0.0.0"\nedition="2021"\n'
        f'[lib]\ncrate-type=["cdylib"]\n'
        f'[dependencies]\n{crate_name}={{path="{path_str}"}}\n'
    )
    (tmp / "Cargo.toml").write_text(toml_content, encoding="utf-8")

    # Run cargo metadata to find the resolved src directory
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--manifest-path",
         str(tmp / "Cargo.toml")],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"import[rs]: cargo metadata failed:\n{result.stderr}")

    try:
        meta = json.loads(result.stdout)
    except Exception as e:
        raise RuntimeError(f"import[rs]: cannot parse cargo metadata: {e}") from e

    crate_src_dir: Optional[Path] = None
    for pkg in meta.get("packages", []):
        if pkg.get("name") == crate_name:
            manifest = pkg.get("manifest_path", "")
            if manifest:
                crate_src_dir = Path(manifest).parent / "src"
            break

    if crate_src_dir is None:
        raise RuntimeError(
            f"import[rs]: crate '{crate_name}' not found in cargo metadata"
        )

    return crate_src_dir, crate_ident


# ---------------------------------------------------------------------------
# Re-export whitelist
# ---------------------------------------------------------------------------

def _follow_pub_use(
    source: str,
    module_dir: Path,
    out: set[str],
    depth: int = 0,
) -> None:
    if depth > 6:
        return
    for raw in source.splitlines():
        line = raw.strip()
        if not line.startswith("pub use "):
            continue
        rest = line[len("pub use "):].rstrip(";")
        if rest.endswith("::*"):
            mod_path = rest.removesuffix("::*").lstrip("self::").replace("::", "/")
            candidates = [
                module_dir / mod_path / "mod.rs",
                module_dir / f"{mod_path}.rs",
            ]
            for cand in candidates:
                if cand.exists():
                    text = cand.read_text(encoding="utf-8", errors="replace")
                    for mline in text.splitlines():
                        if mline.startswith("pub fn "):
                            sig = _parse_single_line_sig(mline.strip())
                            if sig:
                                out.add(sig.name)
                        if mline.startswith("pub struct "):
                            n = _extract_struct_name(mline.strip())
                            if n:
                                out.add(n)
                    _follow_pub_use(text, cand.parent, out, depth + 1)
                    break
        else:
            name = rest.rsplit("::", 1)[-1]
            if name.startswith("{"):
                inner = name.strip("{}")
                for item in inner.split(","):
                    item = item.strip()
                    if item and item != "..":
                        out.add(item)
            elif "}" not in name and name:
                out.add(name)


def collect_reexports(src_dir: Path) -> set[str]:
    """Return the set of names explicitly pub-exported from lib.rs.

    Empty set means no restriction (accept everything).
    """
    lib_rs = src_dir / "lib.rs"
    if not lib_rs.exists():
        return set()
    text = lib_rs.read_text(encoding="utf-8", errors="replace")
    out: set[str] = set()
    for line in text.splitlines():
        if line.startswith("pub fn "):
            sig = _parse_single_line_sig(line.strip())
            if sig:
                out.add(sig.name)
        if line.startswith("pub struct "):
            n = _extract_struct_name(line.strip())
            if n:
                out.add(n)
    _follow_pub_use(text, src_dir, out)
    return out


# ---------------------------------------------------------------------------
# Signature parsing — free functions
# ---------------------------------------------------------------------------

def _find_matching(s: str, open_c: str, close_c: str) -> Optional[int]:
    depth = 0
    for i, c in enumerate(s):
        if c == open_c:
            depth += 1
        elif c == close_c:
            depth -= 1
            if depth == 0:
                return i
    return None


def _parse_params(s: str) -> list[RsParam]:
    s = s.strip()
    if not s:
        return []
    params = []
    depth = 0
    start = 0
    for i, c in enumerate(s):
        if c == "<":
            depth += 1
        elif c == ">":
            depth = max(0, depth - 1)
        elif c == "," and depth == 0:
            p = _parse_one_param(s[start:i])
            if p:
                params.append(p)
            start = i + 1
    p = _parse_one_param(s[start:])
    if p:
        params.append(p)
    return params


def _parse_one_param(s: str) -> Optional[RsParam]:
    s = s.strip()
    if s in {"self", "&self", "&mut self", "mut self"}:
        return None
    colon = s.find(":")
    if colon < 0:
        return None
    raw_name = s[:colon].strip()
    name = raw_name.lstrip("mut").strip()
    rust_type = s[colon + 1:].strip()
    if not name or not rust_type:
        return None
    return RsParam(name=name, rust_type=rust_type)


def _parse_self_params(params: str) -> Optional[tuple[bool, bool, str]]:
    """Return (has_self, is_mutable, rest_params) or None if no self."""
    t = params.strip()
    for prefix, mutable in [
        ("&mut self,", True), ("&mut self", True),
        ("mut self,", True),  ("mut self", True),
        ("&self,", False),    ("&self", False),
        ("self,", False),     ("self", False),
    ]:
        if t.startswith(prefix):
            rest = t[len(prefix):].lstrip(",").strip()
            return (True, mutable, rest)
    return None


def _extract_struct_name(line: str) -> Optional[str]:
    rest = line.removeprefix("pub struct ")
    if rest == line:
        return None
    m = re.match(r'^([A-Za-z_][A-Za-z0-9_]*)', rest)
    if not m:
        return None
    name = m.group(1)
    after = rest[len(name):].lstrip()
    if after.startswith("<"):
        return None
    return name


def _parse_single_line_sig(line: str) -> Optional[RsFnSig]:
    rest = line.removeprefix("pub fn ")
    if rest == line:
        return None
    m = re.match(r'^([A-Za-z_][A-Za-z0-9_]*)', rest)
    if not m:
        return None
    name = m.group(1)
    rest = rest[len(name):].lstrip()
    if rest.startswith("<"):
        return None
    if not rest.startswith("("):
        return None
    paren_end = _find_matching(rest, "(", ")")
    if paren_end is None:
        return None
    params_str = rest[1:paren_end]
    rest = rest[paren_end + 1:].lstrip()

    return_type: Optional[str] = None
    if rest.startswith("->"):
        ret = rest[2:].lstrip()
        end = next(
            (i for i, c in enumerate(ret) if c in ("{", ";", "w")),
            len(ret),
        )
        t = ret[:end].strip()
        if t:
            return_type = t

    if return_type is not None and not is_abi_compatible(return_type):
        return None

    params = _parse_params(params_str)
    for p in params:
        if not is_abi_compatible(p.rust_type):
            return None

    return RsFnSig(name=name, params=params, return_type=return_type)


def _parse_fn_sigs(source: str) -> list[RsFnSig]:
    sigs = []
    for line in source.splitlines():
        if line.startswith("pub fn "):
            sig = _parse_single_line_sig(line.strip())
            if sig:
                sigs.append(sig)
    return sigs


# ---------------------------------------------------------------------------
# Signature parsing — structs and impl blocks
# ---------------------------------------------------------------------------

def _collect_struct_fields(source: str) -> dict[str, list[RsField]]:
    result: dict[str, list[RsField]] = {}
    lines = source.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("pub struct "):
            name = _extract_struct_name(line.strip())
            if name and "{" in line:
                fields: list[RsField] = []
                depth = line.count("{") - line.count("}")
                i += 1
                while i < len(lines) and depth > 0:
                    fline = lines[i].strip()
                    if depth == 1:
                        f = _parse_struct_field_line(fline)
                        if f:
                            fields.append(f)
                    depth += fline.count("{") - fline.count("}")
                    i += 1
                if fields:
                    result[name] = fields
                continue
        i += 1
    return result


def _parse_struct_field_line(line: str) -> Optional[RsField]:
    rest = line.removeprefix("pub ")
    if rest == line:
        return None
    if rest.startswith("("):
        return None
    colon = rest.find(":")
    if colon < 0:
        return None
    name = rest[:colon].strip()
    if not name or not re.match(r'^[A-Za-z_][A-Za-z0-9_]*$', name):
        return None
    type_str = rest[colon + 1:].strip().rstrip(",").strip()
    if not is_abi_compatible(type_str):
        return None
    return RsField(name=name, rust_type=type_str)


def _collect_impl_methods(
    source: str, known_structs: list[str]
) -> dict[str, list[RsMethodSig]]:
    result: dict[str, list[RsMethodSig]] = {}
    lines = source.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        if (
            line.startswith("impl ")
            and " for " not in line
            and "<" not in line
        ):
            after = line[len("impl "):].strip()
            m = re.match(r'^([A-Za-z_][A-Za-z0-9_]*)', after)
            if m:
                struct_name = m.group(1)
                trimmed = line.strip()
                depth = trimmed.count("{") - trimmed.count("}")
                i += 1
                while i < len(lines) and depth > 0:
                    mline = lines[i]
                    mt = mline.strip()
                    if depth == 1 and mt.startswith("pub fn "):
                        msig = _parse_method_line(mt, known_structs)
                        if msig:
                            result.setdefault(struct_name, []).append(msig)
                    depth += mt.count("{") - mt.count("}")
                    i += 1
                continue
        i += 1
    return result


def _parse_method_line(
    line: str, known_structs: list[str]
) -> Optional[RsMethodSig]:
    rest = line.removeprefix("pub fn ")
    if rest == line:
        return None
    m = re.match(r'^([A-Za-z_][A-Za-z0-9_]*)', rest)
    if not m:
        return None
    name = m.group(1)
    rest = rest[len(name):].lstrip()
    if rest.startswith("<") or not rest.startswith("("):
        return None
    paren_end = _find_matching(rest, "(", ")")
    if paren_end is None:
        return None
    params_str = rest[1:paren_end]
    rest = rest[paren_end + 1:].lstrip()

    parsed_self = _parse_self_params(params_str)
    if not parsed_self:
        return None
    _, self_mutable, remaining = parsed_self

    params = _parse_params(remaining)
    for p in params:
        if not is_abi_compatible(p.rust_type):
            return None

    return_type: Optional[str] = None
    return_struct: Optional[str] = None
    if rest.startswith("->"):
        ret = rest[2:].lstrip()
        end = next((i for i, c in enumerate(ret) if c in ("{", ";")), len(ret))
        t = ret[:end].strip()
        if t == "Self":
            return None
        if t:
            if is_abi_compatible(t):
                return_type = t
            elif t in known_structs:
                return_struct = t
            else:
                return None

    return RsMethodSig(
        name=name,
        params=params,
        self_mutable=self_mutable,
        return_type=return_type,
        return_struct=return_struct,
    )


def _parse_struct_sigs(source: str) -> list[RsStructSig]:
    struct_fields = _collect_struct_fields(source)
    known_names = list(struct_fields.keys())
    impl_methods = _collect_impl_methods(source, known_names)

    result = []
    for struct_name, fields in struct_fields.items():
        if not fields:
            continue
        methods = impl_methods.get(struct_name, [])
        new_method = next(
            (m for m in methods if m.name == "new" and m.return_type is None
             and m.return_struct is None),
            None,
        )
        # Actually check return_type == "Self" in the original, but we skip
        # "new" where return is a known struct — so detect via name only
        new_method = next((m for m in methods if m.name == "new"), None)
        if new_method:
            ctor_params = [RsParam(p.name, p.rust_type) for p in new_method.params]
            use_new_fn = True
        else:
            ctor_params = [RsParam(f.name, f.rust_type) for f in fields]
            use_new_fn = False

        methods_filtered = [m for m in methods if m.name != "new"]
        result.append(RsStructSig(
            name=struct_name,
            fields=fields,
            methods=methods_filtered,
            ctor_params=ctor_params,
            use_new_fn=use_new_fn,
        ))
    return result


# ---------------------------------------------------------------------------
# Digest-pattern detection
# ---------------------------------------------------------------------------

def _to_snake_case(s: str) -> str:
    out = []
    for i, c in enumerate(s):
        if c.isupper() and i > 0:
            out.append("_")
        out.append(c.lower())
    return "".join(out)


def _source_exports_digest(source: str) -> bool:
    for line in source.splitlines():
        t = line.strip()
        if not t.startswith("pub use "):
            continue
        rest = t[len("pub use "):].rstrip(";")
        if rest.endswith("::Digest"):
            return True
        if "{" in rest:
            brace = rest.find("{")
            inner = rest[brace + 1:].rstrip("}")
            if any(item.strip() in {"Digest", "self, Digest"} for item in inner.split(",")):
                return True
    return False


def _collect_digest_fns(src_dir: Path, crate_ident: str) -> list[RsFnSig]:
    lib_rs = src_dir / "lib.rs"
    if not lib_rs.exists():
        return []
    source = lib_rs.read_text(encoding="utf-8", errors="replace")
    if not _source_exports_digest(source):
        return []
    fns = []
    for line in source.splitlines():
        t = line.strip()
        if not t.startswith("pub type "):
            continue
        rest = t[len("pub type "):]
        m = re.match(r'^([A-Za-z_][A-Za-z0-9_]*)', rest)
        if not m:
            continue
        alias_name = m.group(1)
        after = rest[len(alias_name):].lstrip()
        if not after.startswith("="):
            continue
        fn_name = _to_snake_case(alias_name)
        fns.append(RsFnSig(
            name=fn_name,
            params=[RsParam(name="input", rust_type="&str")],
            return_type="String",
            digest_type=f"{crate_ident}::{alias_name}",
        ))
    return fns


# ---------------------------------------------------------------------------
# Top-level scan
# ---------------------------------------------------------------------------

def _collect_sigs_recursive(
    directory: Path,
    fns: list[RsFnSig],
    structs: list[RsStructSig],
    seen_fns: set[str],
    seen_structs: set[str],
) -> None:
    try:
        entries = list(directory.iterdir())
    except OSError:
        return
    for entry in entries:
        if entry.is_dir():
            _collect_sigs_recursive(entry, fns, structs, seen_fns, seen_structs)
        elif entry.suffix == ".rs":
            try:
                text = entry.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            for sig in _parse_fn_sigs(text):
                if sig.name not in seen_fns:
                    seen_fns.add(sig.name)
                    fns.append(sig)
            for st in _parse_struct_sigs(text):
                if st.name not in seen_structs:
                    seen_structs.add(st.name)
                    structs.append(st)


def scan_all_sigs(
    src_dir: Path, crate_ident: str
) -> tuple[list[RsFnSig], list[RsStructSig]]:
    """Walk all .rs files under src_dir and return (fn_sigs, struct_sigs)."""
    whitelist = collect_reexports(src_dir)
    fns: list[RsFnSig] = []
    structs: list[RsStructSig] = []
    seen_fns: set[str] = set()
    seen_structs: set[str] = set()
    _collect_sigs_recursive(src_dir, fns, structs, seen_fns, seen_structs)

    # Digest-pattern wrappers
    for dig_fn in _collect_digest_fns(src_dir, crate_ident):
        if dig_fn.name not in seen_fns:
            seen_fns.add(dig_fn.name)
            fns.append(dig_fn)

    if whitelist:
        digest_names = {f.name for f in fns if f.digest_type is not None}
        fns = [f for f in fns if f.name in whitelist or f.name in digest_names]
        structs = [s for s in structs if s.name in whitelist]

    return fns, structs


# ---------------------------------------------------------------------------
# Stub generation (for the type checker)
# ---------------------------------------------------------------------------

def make_stubs(
    fns: list[RsFnSig], structs: list[RsStructSig]
) -> list:
    """Return list[Stmt] (StmtFnDef / StmtClassDef) for the type checker."""
    from ..ast import (
        StmtFnDef, StmtClassDef, StmtField,
        Param, FieldKind, Accessibility,
    )

    stmts = []

    for sig in fns:
        params = [
            Param(name=p.name, mutable=False,
                  type_ann=rust_type_to_ar(p.rust_type), default=None)
            for p in sig.params
        ]
        ret = rust_type_to_ar(sig.return_type) if sig.return_type else None
        stmts.append(StmtFnDef(
            name=sig.name,
            template_params=[],
            params=params,
            return_type=ret,
            body=[],
            is_abstract=True,
            is_static=False,
            is_class_method=False,
            decorators=[],
            access=Accessibility.PUBLIC,
        ))

    for st in structs:
        class_body = []

        # Internal handle field
        class_body.append(StmtField(
            name="__rs_handle__",
            kind=FieldKind.MUT,
            type_ann="int",
            default=None,
            access=Accessibility.PUBLIC,
        ))

        # Public fields
        for f in st.fields:
            class_body.append(StmtField(
                name=f.name,
                kind=FieldKind.MUT,
                type_ann=rust_type_to_ar(f.rust_type),
                default=None,
                access=Accessibility.PUBLIC,
            ))

        # __init__
        init_params = [Param("self", mutable=True, type_ann=None, default=None)] + [
            Param(p.name, mutable=False, type_ann=rust_type_to_ar(p.rust_type), default=None)
            for p in st.ctor_params
        ]
        class_body.append(StmtFnDef(
            name="__init__",
            template_params=[],
            params=init_params,
            return_type=None,
            body=[],
            is_abstract=True,
            is_static=False,
            is_class_method=False,
            decorators=[],
            access=Accessibility.PUBLIC,
        ))

        # drop
        class_body.append(StmtFnDef(
            name="drop",
            template_params=[],
            params=[Param("self", mutable=True, type_ann=None, default=None)],
            return_type=None,
            body=[],
            is_abstract=True,
            is_static=False,
            is_class_method=False,
            decorators=[],
            access=Accessibility.PUBLIC,
        ))

        # Field getter/setter stubs
        for f in st.fields:
            ar_t = rust_type_to_ar(f.rust_type)
            class_body.append(StmtFnDef(
                name=f"get_{f.name}",
                template_params=[],
                params=[Param("self", mutable=False, type_ann=None, default=None)],
                return_type=ar_t,
                body=[],
                is_abstract=True,
                is_static=False,
                is_class_method=False,
                decorators=[],
                access=Accessibility.PUBLIC,
            ))
            class_body.append(StmtFnDef(
                name=f"set_{f.name}",
                template_params=[],
                params=[
                    Param("self", mutable=True, type_ann=None, default=None),
                    Param("value", mutable=False, type_ann=ar_t, default=None),
                ],
                return_type=None,
                body=[],
                is_abstract=True,
                is_static=False,
                is_class_method=False,
                decorators=[],
                access=Accessibility.PUBLIC,
            ))

        # Method stubs
        for m in st.methods:
            m_params = [Param("self", mutable=m.self_mutable, type_ann=None, default=None)] + [
                Param(p.name, mutable=False, type_ann=rust_type_to_ar(p.rust_type), default=None)
                for p in m.params
            ]
            ret = rust_type_to_ar(m.return_type) if m.return_type else None
            class_body.append(StmtFnDef(
                name=m.name,
                template_params=[],
                params=m_params,
                return_type=ret,
                body=[],
                is_abstract=True,
                is_static=False,
                is_class_method=False,
                decorators=[],
                access=Accessibility.PUBLIC,
            ))

        stmts.append(StmtClassDef(
            name=st.name,
            template_params=[],
            bases=[],
            decorators=[],
            body=class_body,
        ))

    return stmts


# ---------------------------------------------------------------------------
# Wrapper Rust source generation (mirrors lib_rs() in rs_loader.rs)
# ---------------------------------------------------------------------------

_ABI_HEADER = r"""// Auto-generated — do not edit.
#![allow(dead_code, unused_variables, non_snake_case, unused_imports, unused_mut,
         clippy::missing_safety_doc)]

use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicI64, Ordering};
use std::collections::HashMap;

const TL_NONE:  i64 = 0;
const TL_TRUE:  i64 = 1;
const TL_FALSE: i64 = 2;

#[repr(C)]
struct ArCallbacks {
    make_int:      unsafe extern "C" fn(i64) -> i64,
    make_float:    unsafe extern "C" fn(f64) -> i64,
    make_bool:     unsafe extern "C" fn(i32) -> i64,
    make_str:      unsafe extern "C" fn(*const u8, i32) -> i64,
    make_list:     unsafe extern "C" fn(*const i64, i32) -> i64,
    make_tuple:    unsafe extern "C" fn(*const i64, i32) -> i64,
    make_dict:     unsafe extern "C" fn(*const i64, *const i64, i32) -> i64,
    make_none:     unsafe extern "C" fn() -> i64,
    is_truthy:     unsafe extern "C" fn(i64) -> i32,
    binop:         unsafe extern "C" fn(i32, i64, i64) -> i64,
    unop:          unsafe extern "C" fn(i32, i64) -> i64,
    call_fn:       unsafe extern "C" fn(i64, *const i64, i32) -> i64,
    get_attr:      unsafe extern "C" fn(i64, *const u8, i32) -> i64,
    set_attr:      unsafe extern "C" fn(i64, *const u8, i32, i64),
    subscript:     unsafe extern "C" fn(i64, i64) -> i64,
    get_global:    unsafe extern "C" fn(*const u8, i32) -> i64,
    iter_from:     unsafe extern "C" fn(i64) -> i64,
    iter_next:     unsafe extern "C" fn(i64) -> i64,
    is_type:       unsafe extern "C" fn(i64, *const u8, i32) -> i64,
    arena_save:    unsafe extern "C" fn() -> u64,
    arena_compact: unsafe extern "C" fn(i64, u64) -> i64,
    compact_many:  unsafe extern "C" fn(*const i64, i32, u64, *mut i64),
    to_int:        unsafe extern "C" fn(i64) -> i64,
    to_float:      unsafe extern "C" fn(i64) -> f64,
    deep_copy:     unsafe extern "C" fn(i64) -> i64,
    to_cstr:       unsafe extern "C" fn(i64) -> *const u8,
    write_handle:  unsafe extern "C" fn(i64, i64),
    list_append:   unsafe extern "C" fn(i64, i64) -> i64,
    raise_exc:     unsafe extern "C" fn(i64, i64) -> i64,
    make_cell:     unsafe extern "C" fn(i64) -> i64,
    get_cell:      unsafe extern "C" fn(i64) -> i64,
    set_cell:      unsafe extern "C" fn(i64, i64),
    call_method:   unsafe extern "C" fn(i64, *const u8, i32, *const i64, i32) -> i64,
}

static mut CB: *const ArCallbacks = std::ptr::null();

#[no_mangle]
pub unsafe extern "C" fn ar_init(cb: *const ArCallbacks) { CB = cb; }

#[inline(always)] unsafe fn cb_make_int(n: i64) -> i64   { ((*CB).make_int)(n) }
#[inline(always)] unsafe fn cb_make_float(f: f64) -> i64  { ((*CB).make_float)(f) }
#[inline(always)] unsafe fn cb_make_str(p: *const u8, l: i32) -> i64 { ((*CB).make_str)(p, l) }
#[inline(always)] unsafe fn cb_to_int(h: i64) -> i64     { ((*CB).to_int)(h) }
#[inline(always)] unsafe fn cb_to_float(h: i64) -> f64   { ((*CB).to_float)(h) }
#[inline(always)] unsafe fn cb_to_cstr(h: i64) -> *const u8 { ((*CB).to_cstr)(h) }

#[inline(always)]
unsafe fn cb_get_attr(obj_h: i64, name: &[u8]) -> i64 {
    ((*CB).get_attr)(obj_h, name.as_ptr(), name.len() as i32)
}

#[inline(always)]
unsafe fn cb_set_attr(obj_h: i64, name: &[u8], val_h: i64) {
    ((*CB).set_attr)(obj_h, name.as_ptr(), name.len() as i32, val_h)
}

unsafe fn handle_to_string(h: i64) -> String {
    let ptr = cb_to_cstr(h);
    if ptr.is_null() { return String::new(); }
    std::ffi::CStr::from_ptr(ptr as *const i8).to_string_lossy().into_owned()
}

"""


def _param_conversion(i: int, name: str, rust_type: str) -> str:
    t = rust_type.strip()
    if t in ("i64", "isize"):
        return f"    let {name}: i64 = cb_to_int(*args.add({i}));\n"
    if t in ("i32", "i16", "i8", "u64", "u32", "u16", "u8", "usize", "u128", "i128"):
        return f"    let {name}: {t} = cb_to_int(*args.add({i})) as {t};\n"
    if t == "f64":
        return f"    let {name}: f64 = cb_to_float(*args.add({i}));\n"
    if t == "f32":
        return f"    let {name}: f32 = cb_to_float(*args.add({i})) as f32;\n"
    if t == "bool":
        return f"    let {name}: bool = *args.add({i}) == TL_TRUE;\n"
    if t == "String":
        return f"    let {name}: String = handle_to_string(*args.add({i}));\n"
    if t in ("&str", "&String"):
        return (
            f"    let _owned_{name}: String = handle_to_string(*args.add({i}));\n"
            f"    let {name}: &str = &_owned_{name};\n"
        )
    if t == "&[u8]":
        return (
            f"    let _bytes_{name}: String = handle_to_string(*args.add({i}));\n"
            f"    let {name}: &[u8] = _bytes_{name}.as_bytes();\n"
        )
    return f"    let {name}: i64 = *args.add({i});\n"


def _return_conversion_expr(call: str, rust_type: Optional[str]) -> str:
    t = (rust_type or "").strip() if rust_type else None
    if t is None or t == "()":
        return f"{call};\n            TL_NONE"
    if t in ("i64", "isize"):
        return f"cb_make_int({call})"
    if t in ("i32", "i16", "i8", "u64", "u32", "u16", "u8", "usize", "u128", "i128"):
        return f"cb_make_int(({call}) as i64)"
    if t == "f64":
        return f"cb_make_float({call})"
    if t == "f32":
        return f"cb_make_float(({call}) as f64)"
    if t == "bool":
        return f"if {call} {{ TL_TRUE }} else {{ TL_FALSE }}"
    if t == "String":
        return f"{{ let _r: String = {call};\n            cb_make_str(_r.as_ptr(), _r.len() as i32) }}"
    if t == "&str":
        return f"{{ let _r: &str = {call};\n            cb_make_str(_r.as_ptr(), _r.len() as i32) }}"
    if t == "Vec<u8>":
        return (
            f'{{ let _r: Vec<u8> = {call};\n'
            f'            let _hex = _r.iter().map(|b| format!("{{:02x}}", b)).collect::<String>();\n'
            f'            cb_make_str(_hex.as_ptr(), _hex.len() as i32) }}'
        )
    if _FIXED_BYTE_RE.match(t):
        return (
            f'{{ let _r = {call};\n'
            f'            let _hex = _r.iter().map(|b| format!("{{:02x}}", b)).collect::<String>();\n'
            f'            cb_make_str(_hex.as_ptr(), _hex.len() as i32) }}'
        )
    return f"{{ let _r: i64 = {call} as i64;\n            cb_make_int(_r) }}"


def _rust_value_to_handle_of(expr: str, rust_type: str) -> str:
    t = rust_type.strip()
    if t in ("i64", "isize"):
        return f"cb_make_int({expr})"
    if t in ("i32", "i16", "i8", "u64", "u32", "u16", "u8", "usize", "u128", "i128"):
        return f"cb_make_int({expr} as i64)"
    if t == "f64":
        return f"cb_make_float({expr})"
    if t == "f32":
        return f"cb_make_float({expr} as f64)"
    if t == "bool":
        return f"if {expr} {{ TL_TRUE }} else {{ TL_FALSE }}"
    if t == "String":
        return f"cb_make_str({expr}.as_ptr(), {expr}.len() as i32)"
    if t in ("&str", "&String"):
        return f"cb_make_str({expr}.as_ptr(), {expr}.len() as i32)"
    return f"cb_make_int({expr} as i64)"


def _digest_wrapper(fn_name: str, type_path: str) -> str:
    return (
        f'#[no_mangle]\n'
        f'pub unsafe extern "C" fn {fn_name}_tl(args: *const i64, _n: i32) -> i64 {{\n'
        f'    let _s: String = handle_to_string(*args.add(0));\n'
        f'    use {type_path} as _HvHasher;\n'
        f'    use digest::Digest as _HvDigest;\n'
        f'    let _result = _HvHasher::digest(_s.as_bytes());\n'
        f'    let _hex: String = _result.iter().map(|b| format!("{{:02x}}", b)).collect();\n'
        f'    cb_make_str(_hex.as_ptr(), _hex.len() as i32)\n'
        f'}}\n\n'
    )


def _fn_wrapper(sig: RsFnSig, crate_ident: str) -> str:
    if sig.digest_type:
        return _digest_wrapper(sig.name, sig.digest_type)

    out = (
        f'#[no_mangle]\n'
        f'pub unsafe extern "C" fn {sig.name}_tl(args: *const i64, _n: i32) -> i64 {{\n'
    )
    for i, p in enumerate(sig.params):
        out += _param_conversion(i, p.name, p.rust_type)

    call = f"{crate_ident}::{sig.name}({', '.join(p.name for p in sig.params)})"
    out += f"    {_return_conversion_expr(call, sig.return_type)}\n"
    out += "}\n\n"
    return out


def _arena_lock_ident(struct_name: str) -> str:
    return f"ARENA_LOCK_{struct_name.upper()}"


def _arena_getter_fn(struct_name: str) -> str:
    return f"get_arena_{struct_name.lower()}"


def _counter_ident(struct_name: str) -> str:
    return f"COUNTER_{struct_name.upper()}"


def _struct_arena_decl(struct_name: str, crate_ident: str) -> str:
    lock = _arena_lock_ident(struct_name)
    getter = _arena_getter_fn(struct_name)
    counter = _counter_ident(struct_name)
    return (
        f"static {lock}: OnceLock<Mutex<HashMap<i64, {crate_ident}::{struct_name}>>> = OnceLock::new();\n"
        f"fn {getter}() -> &'static Mutex<HashMap<i64, {crate_ident}::{struct_name}>> {{\n"
        f"    {lock}.get_or_init(|| Mutex::new(HashMap::new()))\n"
        f"}}\n"
        f"static {counter}: AtomicI64 = AtomicI64::new(1);\n\n"
    )


def _struct_init_wrapper(st: RsStructSig, crate_ident: str) -> str:
    name = st.name
    arena = f"{_arena_getter_fn(name)}()"
    counter = _counter_ident(name)
    sym = f"{name}____init___tl"
    out = (
        f'#[no_mangle]\n'
        f'pub unsafe extern "C" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n'
        f'    let self_h = *args.add(0);\n'
    )
    for i, p in enumerate(st.ctor_params):
        out += _param_conversion(i + 1, p.name, p.rust_type)
    if st.use_new_fn:
        args_str = ", ".join(p.name for p in st.ctor_params)
        out += f"    let _instance = {crate_ident}::{name}::new({args_str});\n"
    else:
        field_inits = ", ".join(f"{p.name}: {p.name}" for p in st.ctor_params)
        out += f"    let _instance = {crate_ident}::{name} {{ {field_inits} }};\n"
    out += (
        f"    let _key = {counter}.fetch_add(1, Ordering::SeqCst);\n"
        f"    {arena}.lock().unwrap().insert(_key, _instance);\n"
        f"    let _key_h = cb_make_int(_key);\n"
        f'    cb_set_attr(self_h, b"__rs_handle__", _key_h);\n'
    )
    for f in st.fields:
        out += (
            f"    {{\n"
            f"        let _arena = {arena}.lock().unwrap();\n"
            f"        if let Some(_obj) = _arena.get(&_key) {{\n"
            f"            let _fh = {_rust_value_to_handle_of(f'_obj.{f.name}', f.rust_type)};\n"
            f'            cb_set_attr(self_h, b"{f.name}", _fh);\n'
            f"        }}\n"
            f"    }}\n"
        )
    out += "    TL_NONE\n}\n\n"
    return out


def _struct_drop_wrapper(struct_name: str) -> str:
    arena = f"{_arena_getter_fn(struct_name)}()"
    sym = f"{struct_name}__drop_tl"
    return (
        f'#[no_mangle]\n'
        f'pub unsafe extern "C" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n'
        f'    let self_h = *args.add(0);\n'
        f'    let _key = cb_to_int(cb_get_attr(self_h, b"__rs_handle__"));\n'
        f'    {arena}.lock().unwrap().remove(&_key);\n'
        f'    TL_NONE\n}}\n\n'
    )


def _struct_getter_wrapper(struct_name: str, f: RsField) -> str:
    arena = f"{_arena_getter_fn(struct_name)}()"
    sym = f"{struct_name}__get_{f.name}_tl"
    handle_expr = _rust_value_to_handle_of(f"_obj.{f.name}", f.rust_type)
    return (
        f'#[no_mangle]\n'
        f'pub unsafe extern "C" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n'
        f'    let self_h = *args.add(0);\n'
        f'    let _key = cb_to_int(cb_get_attr(self_h, b"__rs_handle__"));\n'
        f'    let _arena = {arena}.lock().unwrap();\n'
        f'    if let Some(_obj) = _arena.get(&_key) {{ {handle_expr} }} else {{ TL_NONE }}\n'
        f'}}\n\n'
    )


def _struct_setter_wrapper(struct_name: str, f: RsField) -> str:
    arena = f"{_arena_getter_fn(struct_name)}()"
    sym = f"{struct_name}__set_{f.name}_tl"
    decode = _param_conversion(1, "_val", f.rust_type)
    return (
        f'#[no_mangle]\n'
        f'pub unsafe extern "C" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n'
        f'    let self_h = *args.add(0);\n'
        f'{decode}'
        f'    let _key = cb_to_int(cb_get_attr(self_h, b"__rs_handle__"));\n'
        f'    let mut _arena = {arena}.lock().unwrap();\n'
        f'    if let Some(_obj) = _arena.get_mut(&_key) {{ _obj.{f.name} = _val; }}\n'
        f'    TL_NONE\n}}\n\n'
    )


def _struct_method_wrapper(
    struct_name: str,
    m: RsMethodSig,
    crate_ident: str,
    all_structs: list[RsStructSig],
) -> str:
    arena = f"{_arena_getter_fn(struct_name)}()"
    sym = f"{struct_name}__{m.name}_tl"
    out = (
        f'#[no_mangle]\n'
        f'pub unsafe extern "C" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n'
        f'    let self_h = *args.add(0);\n'
        f'    let _key = cb_to_int(cb_get_attr(self_h, b"__rs_handle__"));\n'
    )
    for i, p in enumerate(m.params):
        out += _param_conversion(i + 1, p.name, p.rust_type)
    args_str = ", ".join(p.name for p in m.params)

    if m.return_struct:
        # Method returns a struct — extract its ctor fields, then call cb_call_fn
        ret_struct = next((s for s in all_structs if s.name == m.return_struct), None)
        ctor_params: list[RsParam] = ret_struct.ctor_params if ret_struct else []
        lock_kw = "mut " if m.self_mutable else ""
        borrow = "get_mut(&_key)" if m.self_mutable else "get(&_key)"
        out += f"    let {lock_kw}_arena = {arena}.lock().unwrap();\n"
        out += f"    let _found = _arena.{borrow};\n"
        out += "    if _found.is_none() { return TL_NONE; }\n"
        out += "    let _obj = _found.unwrap();\n"
        out += f"    let _rval = _obj.{m.name}({args_str});\n"
        for i, p in enumerate(ctor_params):
            out += f"    let _ret_{i}: {p.rust_type} = _rval.{p.name};\n"
        out += "    drop(_arena);\n"
        rn = m.return_struct
        out += f'    let _cls_h = ((*CB).get_global)("{rn}".as_ptr(), {len(rn)});\n'
        if ctor_params:
            handle_exprs = ", ".join(
                _rust_value_to_handle_of(f"_ret_{i}", p.rust_type)
                for i, p in enumerate(ctor_params)
            )
            out += f"    let _ctor = [{handle_exprs}];\n"
        else:
            out += "    let _ctor: [i64; 0] = [];\n"
        out += "    ((*CB).call_fn)(_cls_h, _ctor.as_ptr(), _ctor.len() as i32)\n"
        out += "}\n\n"
    else:
        if m.self_mutable:
            out += f"    let mut _arena = {arena}.lock().unwrap();\n"
            out += f"    match _arena.get_mut(&_key) {{\n"
        else:
            out += f"    let _arena = {arena}.lock().unwrap();\n"
            out += f"    match _arena.get(&_key) {{\n"
        out += "        None => TL_NONE,\n"
        call = f"_obj.{m.name}({args_str})"
        out += f"        Some(_obj) => {{\n            {_return_conversion_expr(call, m.return_type)}\n        }},\n    }}\n}}\n\n"
    return out


def generate_wrapper_lib_rs(
    fns: list[RsFnSig],
    structs: list[RsStructSig],
    crate_ident: str,
) -> str:
    """Generate the complete lib.rs wrapper source (same ABI as rs_loader.rs)."""
    out = _ABI_HEADER
    for st in structs:
        out += _struct_arena_decl(st.name, crate_ident)
    for sig in fns:
        out += _fn_wrapper(sig, crate_ident)
    for st in structs:
        out += _struct_init_wrapper(st, crate_ident)
        out += _struct_drop_wrapper(st.name)
        for f in st.fields:
            out += _struct_getter_wrapper(st.name, f)
            out += _struct_setter_wrapper(st.name, f)
        for m in st.methods:
            out += _struct_method_wrapper(st.name, m, crate_ident, structs)
    return out


# ---------------------------------------------------------------------------
# Compilation
# ---------------------------------------------------------------------------

def native_lib_ext() -> str:
    if sys.platform == "win32":
        return "dll"
    if sys.platform == "darwin":
        return "dylib"
    return "so"


def _detect_digest_version(crate_src_dir: Path) -> str:
    cargo_toml = crate_src_dir.parent / "Cargo.toml"
    if not cargo_toml.exists():
        return "*"
    text = cargo_toml.read_text(encoding="utf-8", errors="replace")
    for line in text.splitlines():
        m = re.match(r'^digest\s*=\s*"([^"]+)"', line)
        if m:
            return m.group(1)
        m = re.match(r'^digest\s*=\s*\{[^}]*version\s*=\s*"([^"]+)"', line)
        if m:
            return m.group(1)
    return "*"


def _patch_cargo_toml_digest(toml_path: Path, version: str) -> None:
    text = toml_path.read_text(encoding="utf-8")
    if "digest" not in text:
        text += f'\ndigest = "{version}"\n'
        toml_path.write_text(text, encoding="utf-8")


def compile_cdylib(
    tmp: Path,
    wrapper_src: str,
    fns: list[RsFnSig],
) -> bytes:
    """Write wrapper lib.rs, run cargo build --release, return DLL bytes."""
    src_dir = tmp / "src"
    src_dir.mkdir(parents=True, exist_ok=True)
    (src_dir / "lib.rs").write_text(wrapper_src, encoding="utf-8")

    # Patch in digest dependency if needed
    if any(f.digest_type for f in fns):
        # We need the crate src dir to detect the version — skip for now, use "*"
        _patch_cargo_toml_digest(tmp / "Cargo.toml", "*")

    print(f"RsImport: running cargo build --release in '{tmp}' …", file=sys.stderr)
    result = subprocess.run(
        ["cargo", "build", "--release"],
        cwd=str(tmp),
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"import[rs]: cargo build failed:\n{result.stderr}")

    stem = tmp.name  # e.g. "ar_rs_libm"
    lib_prefix = "" if sys.platform == "win32" else "lib"
    ext = native_lib_ext()
    dll_name = f"{lib_prefix}{stem}.{ext}"
    dll_path = tmp / "target" / "release" / dll_name
    if not dll_path.exists():
        raise RuntimeError(
            f"import[rs]: DLL not found after build: '{dll_path}'"
        )

    dll_bytes = dll_path.read_bytes()
    print(f"RsImport: compiled '{dll_name}' ({len(dll_bytes)} bytes)", file=sys.stderr)
    return dll_bytes


# ---------------------------------------------------------------------------
# Module-level DLL cache
# ---------------------------------------------------------------------------

# crate_name → bytes
_RS_DLL_CACHE: dict[str, bytes] = {}


def load(
    crate_name: str,
    search_dirs: list[Path],
    version: Optional[str] = None,
) -> tuple[list, bytes]:
    """Full pipeline: scan → stubs + DLL bytes.

    Returns (stmts, dll_bytes).
    Caches the DLL bytes in _RS_DLL_CACHE[crate_name].
    """
    if crate_name in _RS_DLL_CACHE:
        # stubs are regenerated each parse (from cache key on interpreter side)
        pass

    crates_paths = find_config(search_dirs)
    crate_dir = find_crate_dir(crates_paths, crate_name, version)

    stem = crate_name.replace(".", "_").replace("-", "_")
    tmp = Path(tempfile.gettempdir()) / f"ar_rs_{stem}"

    crate_src_dir, crate_ident = prepare_wrapper(crate_dir, stem, tmp)
    fns, structs = scan_all_sigs(crate_src_dir, crate_ident)

    if not fns and not structs:
        raise RuntimeError(
            f"import[rs] '{crate_name}': no compatible pub fn or pub struct found "
            "(only primitive types int/float/bool/str/&[u8]/Vec<u8>/[u8;N] are supported)"
        )

    wrapper_src = generate_wrapper_lib_rs(fns, structs, crate_ident)
    dll_bytes = compile_cdylib(tmp, wrapper_src, fns)

    # Clean up temp dir
    import shutil
    try:
        shutil.rmtree(str(tmp), ignore_errors=True)
    except Exception:
        pass

    _RS_DLL_CACHE[crate_name] = dll_bytes
    stmts = make_stubs(fns, structs)
    return stmts, dll_bytes
