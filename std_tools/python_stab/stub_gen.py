"""Python → Arrow (.ars) stub file generator.

Converts a Python source file into a Arrow type stub, including:
- Free functions with let/mut parameter classification
- Classes with member declarations and methods
- Module-level annotated variables and constants
"""

import ast
from typing import Optional, Iterator

# ─── Type mapping ─────────────────────────────────────────────────────────────

PRIMITIVE_MAP: dict[str, str] = {
    "int": "int",
    "float": "float",
    "str": "str",
    "bool": "bool",
    "None": "None",
    "NoneType": "None",
    "Any": "Any",
    "object": "Any",
    "bytes": "str",
    "complex": "Any",
}


def convert_annotation(node: Optional[ast.expr]) -> str:
    """Convert a Python type annotation AST node to a Arrow type string."""
    if node is None:
        return "Any"
    if isinstance(node, ast.Constant):
        if node.value is None:
            return "None"
        if isinstance(node.value, str):
            return node.value  # forward reference
        return "Any"
    if isinstance(node, ast.Name):
        return PRIMITIVE_MAP.get(node.id, node.id)
    if isinstance(node, ast.Attribute):
        return PRIMITIVE_MAP.get(node.attr, node.attr)
    if isinstance(node, ast.Subscript):
        outer = convert_annotation(node.value)
        inner = _convert_inner(node.slice)
        if outer == "Optional":
            return f"Optional[{inner}]"
        if outer == "Union":
            return f"Union[{inner}]"
        if outer in ("List", "list"):
            return f"list[{inner}]"
        if outer in ("Dict", "dict"):
            return f"dict[{inner}]"
        if outer in ("Set", "set"):
            return f"set[{inner}]"
        if outer in ("Tuple", "tuple", "FrozenSet", "frozenset"):
            return "tuple"
        if outer in ("Callable", "Awaitable", "Coroutine", "Generator", "AsyncGenerator"):
            return "function"
        if outer in ("ClassVar", "Final", "Type", "Literal", "Annotated"):
            return inner
        return f"{outer}[{inner}]"
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.BitOr):
        left = convert_annotation(node.left)
        right = convert_annotation(node.right)
        if right == "None":
            return f"Optional[{left}]"
        if left == "None":
            return f"Optional[{right}]"
        return f"Union[{left}, {right}]"
    if isinstance(node, ast.Tuple):
        return ", ".join(convert_annotation(e) for e in node.elts)
    return "Any"


def _convert_inner(node: ast.expr) -> str:
    if isinstance(node, ast.Tuple):
        return ", ".join(convert_annotation(e) for e in node.elts)
    return convert_annotation(node)


def _convert_default(node: ast.expr) -> Optional[str]:
    """Convert a Python default value to a Arrow literal. Returns None if too complex."""
    if isinstance(node, ast.Constant):
        if node.value is None:
            return "None"
        if isinstance(node.value, bool):
            return "True" if node.value else "False"
        if isinstance(node.value, int):
            return str(node.value)
        if isinstance(node.value, float):
            return str(node.value)
        if isinstance(node.value, str):
            escaped = node.value.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
            return f'"{escaped}"'
    if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
        inner = _convert_default(node.operand)
        return f"-{inner}" if inner else None
    return None


# ─── AST walker that stops at nested function/class boundaries ────────────────

def _walk_no_nested(stmts: list[ast.stmt]) -> Iterator[ast.AST]:
    """Yield all AST nodes in stmts, but do not descend into nested FunctionDef/ClassDef bodies."""
    stack: list[ast.AST] = list(stmts)
    while stack:
        node = stack.pop()
        yield node
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            continue
        stack.extend(ast.iter_child_nodes(node))


# ─── Mutability analysis ──────────────────────────────────────────────────────

KNOWN_MUTATING_METHODS: frozenset[str] = frozenset({
    # list
    "append", "extend", "insert", "remove", "pop", "clear", "sort", "reverse",
    # dict
    "update", "setdefault", "popitem",
    # set
    "add", "discard",
    "intersection_update", "difference_update", "symmetric_difference_update",
    # dunder in-place
    "__setitem__", "__delitem__",
    "__iadd__", "__isub__", "__imul__", "__itruediv__",
    "__ifloordiv__", "__imod__", "__ipow__",
    "__iand__", "__ior__", "__ixor__", "__ilshift__", "__irshift__",
})


class Analyzer:
    """Analyses a Python module AST to determine parameter and member mutability."""

    def __init__(self, module: ast.Module) -> None:
        self.module = module
        # name → FunctionDef (top-level and "ClassName.method_name")
        self.functions: dict[str, ast.FunctionDef] = {}
        # name → ClassDef
        self.classes: dict[str, ast.ClassDef] = {}
        # Memoization: (id(func), param_idx) → is_mut
        self._memo: dict[tuple[int, int], bool] = {}
        # Cycle guard: keys currently being computed
        self._computing: set[tuple[int, int]] = set()
        self._collect()

    def _collect(self) -> None:
        for node in self.module.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                self.functions[node.name] = node  # type: ignore[assignment]
            elif isinstance(node, ast.ClassDef):
                self.classes[node.name] = node
                for item in node.body:
                    if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                        self.functions[f"{node.name}.{item.name}"] = item  # type: ignore[assignment]

    # ── Public interface ──────────────────────────────────────────────────────

    def is_param_mut(self, func: ast.FunctionDef, param_idx: int,
                     class_name: Optional[str] = None) -> bool:
        """Return True if the parameter at param_idx in func is mutated in the body."""
        key = (id(func), param_idx)
        if key in self._memo:
            return self._memo[key]
        if key in self._computing:
            return False  # cycle → conservatively assume let
        self._computing.add(key)

        all_params = func.args.posonlyargs + func.args.args
        if param_idx < len(all_params):
            pname = all_params[param_idx].arg
            is_self = (param_idx == 0 and pname == "self")
            result = self._body_mutates(pname, is_self, func.body, class_name)
        else:
            result = False

        self._computing.discard(key)
        self._memo[key] = result
        return result

    def get_class_members(self, cls: ast.ClassDef) -> dict[str, str]:
        """Return {member_name: type_str} gathered from __init__ and class-level annotations."""
        members: dict[str, str] = {}

        # Class-level annotations take first priority
        for item in cls.body:
            if isinstance(item, ast.AnnAssign) and isinstance(item.target, ast.Name):
                name = item.target.id
                if not name.startswith("_"):
                    members[name] = convert_annotation(item.annotation)

        # __init__ self.attr assignments
        init = self._find_method(cls, "__init__")
        if init:
            param_types = self._param_type_map(init)
            for node in _walk_no_nested(init.body):
                annotation: Optional[ast.expr] = None
                value_node: Optional[ast.expr] = None
                targets: list[ast.expr] = []

                if isinstance(node, ast.Assign):
                    targets, value_node = node.targets, node.value
                elif isinstance(node, ast.AugAssign):
                    targets = [node.target]
                elif isinstance(node, ast.AnnAssign) and node.value is not None:
                    targets, annotation, value_node = [node.target], node.annotation, node.value

                for target in targets:
                    if (isinstance(target, ast.Attribute) and
                            isinstance(target.value, ast.Name) and
                            target.value.id == "self"):
                        attr = target.attr
                        if attr not in members:
                            if annotation:
                                members[attr] = convert_annotation(annotation)
                            elif value_node:
                                members[attr] = self._infer_type(value_node, param_types)
                            else:
                                members[attr] = "Any"
        return members

    def is_member_mut(self, cls: ast.ClassDef, member: str) -> bool:
        """True if any method other than __init__ reassigns self.<member>."""
        for item in cls.body:
            if not isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            if item.name == "__init__":
                continue
            for node in _walk_no_nested(item.body):
                if isinstance(node, ast.Assign):
                    if any(self._is_self_attr(t, member) for t in node.targets):
                        return True
                elif isinstance(node, ast.AugAssign):
                    if self._is_self_attr(node.target, member):
                        return True
                elif isinstance(node, ast.AnnAssign) and node.value is not None:
                    if self._is_self_attr(node.target, member):
                        return True
        return False

    # ── Internal helpers ──────────────────────────────────────────────────────

    def _body_mutates(self, name: str, is_self: bool,
                      stmts: list[ast.stmt], class_name: Optional[str]) -> bool:
        for node in _walk_no_nested(stmts):
            if isinstance(node, ast.Assign):
                if any(self._target_mutates(t, name, is_self) for t in node.targets):
                    return True
            elif isinstance(node, ast.AugAssign):
                if self._target_mutates(node.target, name, is_self):
                    return True
            elif isinstance(node, ast.AnnAssign) and node.value is not None:
                if self._target_mutates(node.target, name, is_self):
                    return True
            elif isinstance(node, ast.Delete):
                if any(self._target_mutates(t, name, is_self) for t in node.targets):
                    return True
            elif isinstance(node, ast.Call):
                if self._call_mutates(node, name, is_self, class_name):
                    return True
        return False

    def _target_mutates(self, target: ast.expr, name: str, is_self: bool) -> bool:
        # name = ...
        if isinstance(target, ast.Name) and target.id == name:
            return True
        # name[...] = ...  or  name.attr[...] = ...
        if isinstance(target, ast.Subscript):
            if self._root_name(target) == name:
                return True
        # self.attr = ...  or  self.attr[...] = ...
        if is_self and isinstance(target, ast.Attribute):
            if self._root_name(target) == name:
                return True
        return False

    def _root_name(self, node: ast.expr) -> Optional[str]:
        if isinstance(node, ast.Name):
            return node.id
        if isinstance(node, (ast.Subscript, ast.Attribute)):
            return self._root_name(node.value)
        return None

    def _call_mutates(self, call: ast.Call, name: str,
                      is_self: bool, class_name: Optional[str]) -> bool:
        if isinstance(call.func, ast.Attribute):
            obj, method = call.func.value, call.func.attr
            # name.method(...)
            if isinstance(obj, ast.Name) and obj.id == name:
                return self._method_is_mutating(method, class_name)
            # self.attr.method(...) with a known mutating method
            if is_self and method in KNOWN_MUTATING_METHODS:
                if self._root_name(obj) == name:
                    return True
        else:
            # f(name, ...) — check whether f's corresponding parameter is mut
            for i, arg in enumerate(call.args):
                if isinstance(arg, ast.Name) and arg.id == name:
                    callee = self._resolve_func(call.func)
                    if callee and self.is_param_mut(callee, i):
                        return True
        return False

    def _method_is_mutating(self, method: str, class_name: Optional[str]) -> bool:
        if method in KNOWN_MUTATING_METHODS:
            return True
        # Check in the known class first
        if class_name:
            key = f"{class_name}.{method}"
            if key in self.functions:
                return self.is_param_mut(self.functions[key], 0, class_name)
        # Fall back to any class that defines this method
        for cls_name in self.classes:
            key = f"{cls_name}.{method}"
            if key in self.functions:
                if self.is_param_mut(self.functions[key], 0, cls_name):
                    return True
        return False

    def _resolve_func(self, expr: ast.expr) -> Optional[ast.FunctionDef]:
        if isinstance(expr, ast.Name):
            return self.functions.get(expr.id)
        return None

    def _find_method(self, cls: ast.ClassDef, name: str) -> Optional[ast.FunctionDef]:
        for item in cls.body:
            if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name == name:
                return item  # type: ignore[return-value]
        return None

    def _param_type_map(self, func: ast.FunctionDef) -> dict[str, str]:
        """Map param name → Arrow type string for all non-self params."""
        result: dict[str, str] = {}
        all_params = func.args.posonlyargs + func.args.args
        for p in all_params[1:]:
            result[p.arg] = convert_annotation(p.annotation)
        return result

    def _infer_type(self, node: ast.expr, param_types: dict[str, str]) -> str:
        if isinstance(node, ast.Constant):
            if isinstance(node.value, bool):
                return "bool"
            if isinstance(node.value, int):
                return "int"
            if isinstance(node.value, float):
                return "float"
            if isinstance(node.value, str):
                return "str"
            if node.value is None:
                return "None"
        if isinstance(node, ast.Name):
            return param_types.get(node.id, "Any")
        if isinstance(node, ast.List):
            return "list"
        if isinstance(node, ast.Dict):
            return "dict"
        if isinstance(node, ast.Set):
            return "set"
        if isinstance(node, ast.Tuple):
            return "tuple"
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name):
            n = node.func.id
            if n in PRIMITIVE_MAP:
                return PRIMITIVE_MAP[n]
            if n in ("list", "dict", "set", "tuple"):
                return n
            return n  # class constructor — use class name as type
        return "Any"

    def _is_self_attr(self, target: ast.expr, member: str) -> bool:
        return (isinstance(target, ast.Attribute) and
                target.attr == member and
                isinstance(target.value, ast.Name) and
                target.value.id == "self")


# ─── Stub generator ──────────────────────────────────────────────────────────

_SKIP_BASES = frozenset({"object", "ABC", "ABCMeta", "Enum", "IntEnum", "Protocol"})


def _has_decorator(func: ast.FunctionDef, name: str) -> bool:
    for d in func.decorator_list:
        if isinstance(d, ast.Name) and d.id == name:
            return True
        if isinstance(d, ast.Attribute) and d.attr == name:
            return True
    return False


class StubGenerator:
    def __init__(self, source: str) -> None:
        self.tree = ast.parse(source)
        self.analyzer = Analyzer(self.tree)
        self.out: list[str] = []

    def generate(self) -> str:
        for node in self.tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                if not node.name.startswith("_"):
                    self._emit_function(node, indent=0)  # type: ignore[arg-type]
                    self.out.append("")
            elif isinstance(node, ast.ClassDef):
                if not node.name.startswith("_"):
                    self._emit_class(node)
                    self.out.append("")
            elif isinstance(node, ast.AnnAssign):
                self._emit_ann_var(node, indent=0)
            elif isinstance(node, ast.Assign):
                self._emit_const_maybe(node)
        return "\n".join(self.out).rstrip("\n") + "\n"

    # ── Function ──────────────────────────────────────────────────────────────

    def _emit_function(self, func: ast.FunctionDef, indent: int,
                       class_name: Optional[str] = None) -> None:
        pad = "    " * indent
        params = self._format_params(func, class_name)
        if func.returns is None and func.name == "__init__":
            ret = "None"
        else:
            ret = convert_annotation(func.returns)
        self.out.append(f"{pad}fn {func.name}({params}) -> {ret}:")
        self.out.append(f"{pad}    ...")

    def _format_params(self, func: ast.FunctionDef,
                       class_name: Optional[str]) -> str:
        parts: list[str] = []
        args = func.args
        all_pos = args.posonlyargs + args.args
        default_start = len(all_pos) - len(args.defaults)

        is_static = _has_decorator(func, "staticmethod")
        is_cls_method = _has_decorator(func, "classmethod")

        for i, param in enumerate(all_pos):
            # Skip cls in classmethods
            if is_cls_method and i == 0:
                continue

            is_self = (not is_static and i == 0 and param.arg == "self")

            if is_self:
                mut = self.analyzer.is_param_mut(func, i, class_name)
                parts.append(("mut" if mut else "let") + " self")
                continue

            mut = self.analyzer.is_param_mut(func, i, class_name)
            qualifier = "mut" if mut else "let"
            type_str = convert_annotation(param.annotation)

            di = i - default_start
            if 0 <= di < len(args.defaults):
                dval = _convert_default(args.defaults[di])
                if dval is not None:
                    parts.append(f"{qualifier} {param.arg}: {type_str} = {dval}")
                else:
                    parts.append(f"{qualifier} {param.arg}: {type_str}")
            else:
                parts.append(f"{qualifier} {param.arg}: {type_str}")

        if args.vararg:
            type_str = convert_annotation(args.vararg.annotation)
            parts.append(f"*{args.vararg.arg}: {type_str}" if args.vararg.annotation else f"*{args.vararg.arg}")

        for i, kwp in enumerate(args.kwonlyargs):
            type_str = convert_annotation(kwp.annotation)
            kd = args.kw_defaults[i]
            if kd is not None:
                dval = _convert_default(kd)
                suffix = f" = {dval}" if dval is not None else ""
                parts.append(f"let {kwp.arg}: {type_str}{suffix}")
            else:
                parts.append(f"let {kwp.arg}: {type_str}")

        if args.kwarg:
            parts.append(f"**{args.kwarg.arg}")

        return ", ".join(parts)

    # ── Class ─────────────────────────────────────────────────────────────────

    def _emit_class(self, cls: ast.ClassDef) -> None:
        bases = []
        for b in cls.bases:
            name = b.id if isinstance(b, ast.Name) else (b.attr if isinstance(b, ast.Attribute) else None)
            if name and name not in _SKIP_BASES:
                bases.append(name)

        header = f"class {cls.name}({', '.join(bases)}):" if bases else f"class {cls.name}:"
        self.out.append(header)

        members = self.analyzer.get_class_members(cls)
        has_body = False

        for member, mtype in members.items():
            is_mut = self.analyzer.is_member_mut(cls, member)
            self.out.append(f"    {'mut' if is_mut else 'let'} {member}: {mtype}")
            has_body = True

        for item in cls.body:
            if not isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            name = item.name
            # Skip private non-dunder methods
            if name.startswith("_") and not (name.startswith("__") and name.endswith("__")):
                continue
            # Skip @overload duplicates (keep first occurrence)
            if _has_decorator(item, "overload"):  # type: ignore[arg-type]
                pass  # emit overloads as-is
            if has_body:
                self.out.append("")
            self._emit_function(item, indent=1, class_name=cls.name)  # type: ignore[arg-type]
            has_body = True

        if not has_body:
            self.out.append("    ...")

    # ── Module-level variables ────────────────────────────────────────────────

    def _emit_ann_var(self, node: ast.AnnAssign, indent: int) -> None:
        if not isinstance(node.target, ast.Name):
            return
        name = node.target.id
        if name.startswith("_"):
            return
        pad = "    " * indent
        type_str = convert_annotation(node.annotation)
        self.out.append(f"{pad}let {name}: {type_str}")

    def _emit_const_maybe(self, node: ast.Assign) -> None:
        for target in node.targets:
            if (isinstance(target, ast.Name) and
                    target.id.isupper() and
                    not target.id.startswith("_")):
                self.out.append(f"const {target.id}: Any")


# ─── Public entry point ───────────────────────────────────────────────────────

def generate_stub(path: str) -> str:
    """Read a Python source file and return its Arrow stub as a string."""
    with open(path, "r", encoding="utf-8-sig") as f:
        source = f.read()
    return StubGenerator(source).generate()
