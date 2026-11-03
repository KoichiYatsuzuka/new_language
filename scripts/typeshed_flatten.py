"""typeshed の `.pyi` を、Arrow の行単位の型スタブの抽出器（`src/parser/py_stub_extract.rs`）で読める形にする
（pandas_import_plan.md の D-10・タスク 1-6）。`gen_builtin_py_stubs.py` から使う。

抽出器が読めないもの・読み違えるものを、生成の段で畳む:
- `if sys.version_info >= (3, 10):` / `if sys.platform == "win32":` の分岐は、生成した CPython（版と OS）に当てはまる枝だけ残す
  （抽出器はインデント 0 の宣言しか読まないので、分岐の中の宣言は落ちる）
- `_T = TypeVar("_T")` などの型変数は `Any`、`_Mode: TypeAlias = Literal["r", "w"]` などの型の別名は中身に置き換える
  （抽出器は大文字で始まらない名前を型の名前として残すので、`_T` が「`_T` というクラス」になる）
- `X: Final = 8` は値の型（`X: int`）
- クラスの中の `@overload` は 1 つの「引数を問わない」メソッドにまとめる（抽出器は最後の形だけを読み、ほかの形の呼び出しを
  誤りにしてしまう）。戻り値はどの形も同じならその型、違えば `Any`。モジュールの関数の `@overload` は抽出器が選ぶのでそのまま
- `__new__` しか無いクラスは `__init__` にする（抽出器はコンストラクタの引数を `__init__` から読む）
- 属性の操作のフック（`__getattr__` など）は書かない（Arrow のクラスは持てない・language-differences.md）
"""
import ast
import copy
import operator
import sys

_CMP = {
    ast.GtE: operator.ge, ast.Gt: operator.gt, ast.LtE: operator.le, ast.Lt: operator.lt,
    ast.Eq: operator.eq, ast.NotEq: operator.ne,
}
_TYPEVAR_CALLS = {"TypeVar", "ParamSpec", "TypeVarTuple", "NewType"}
_HOOKS = {"__getattr__", "__getattribute__", "__setattr__", "__delattr__"}


def _dotted(node):
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        base = _dotted(node.value)
        return f"{base}.{node.attr}" if base else None
    return None


def _cond(node):
    """分岐の条件を、生成している CPython で評価する。分からなければ `None`。"""
    if isinstance(node, ast.BoolOp):
        vals = [_cond(v) for v in node.values]
        if isinstance(node.op, ast.And):
            if any(v is False for v in vals):
                return False
            return None if any(v is None for v in vals) else True
        if any(v is True for v in vals):
            return True
        return None if any(v is None for v in vals) else False
    if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not):
        v = _cond(node.operand)
        return None if v is None else not v
    if isinstance(node, ast.Compare) and len(node.ops) == 1 and type(node.ops[0]) in _CMP:
        left, right, op = node.left, node.comparators[0], _CMP[type(node.ops[0])]
        name = _dotted(left)
        if name == "sys.version_info" and isinstance(right, ast.Tuple):
            want = tuple(e.value for e in right.elts if isinstance(e, ast.Constant))
            return op(tuple(sys.version_info[: len(want)]), want)
        if name == "sys.platform" and isinstance(right, ast.Constant):
            return op(sys.platform, right.value)
    if isinstance(node, ast.Call) and _dotted(node.func) == "sys.platform.startswith" and node.args:
        arg = node.args[0]
        if isinstance(arg, ast.Constant):
            return sys.platform.startswith(arg.value)
    if isinstance(node, ast.Name) and node.id == "TYPE_CHECKING":
        return True
    return None


def _flatten(body):
    out = []
    for st in body:
        if isinstance(st, ast.If):
            c = _cond(st.test)
            if c is False:
                out.extend(_flatten(st.orelse))
            else:
                # 分からない条件は最初の枝を残す（型の検査器も両方を見る・ここでは宣言が落ちないことを優先）
                out.extend(_flatten(st.body))
        elif isinstance(st, ast.ClassDef):
            st.body = _flatten(st.body)
            out.append(st)
        elif isinstance(st, ast.Try):
            out.extend(_flatten(st.body))
        else:
            out.append(st)
    return out


def _is_overload(fn):
    return any(_dotted(d) in ("overload", "typing.overload") for d in fn.decorator_list)


def _type_of_constant(node):
    if isinstance(node, ast.Constant):
        v = node.value
        if isinstance(v, bool):
            return "bool"
        for t in (int, float, str, bytes):
            if type(v) is t:
                return t.__name__
    if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub) and isinstance(node.operand, ast.Constant):
        return _type_of_constant(node.operand)
    return "Any"


class _Rewrite(ast.NodeTransformer):
    """注釈の中の型変数を `Any`、型の別名を中身にする。"""

    def __init__(self, typevars, aliases):
        self.typevars = typevars
        self.aliases = aliases
        self.depth = 0

    def visit_Name(self, node):
        if node.id in self.typevars:
            return ast.copy_location(ast.Name(id="Any", ctx=ast.Load()), node)
        if node.id in self.aliases and self.depth < 10:
            self.depth += 1
            new = self.visit(copy.deepcopy(self.aliases[node.id]))
            self.depth -= 1
            return ast.copy_location(new, node)
        return node


def _rewrite_annotations(tree, rw):
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            a = node.args
            for arg in a.posonlyargs + a.args + a.kwonlyargs + [a.vararg, a.kwarg]:
                if arg is not None and arg.annotation is not None:
                    arg.annotation = rw.visit(arg.annotation)
            if node.returns is not None:
                node.returns = rw.visit(node.returns)
        elif isinstance(node, ast.AnnAssign):
            node.annotation = rw.visit(node.annotation)


def _merge_overloaded_methods(cls):
    groups = {}
    for st in cls.body:
        if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef)) and _is_overload(st):
            groups.setdefault(st.name, []).append(st)
    if not groups:
        return
    body = []
    done = set()
    for st in cls.body:
        name = getattr(st, "name", None)
        if name in groups:
            if name in done:
                continue
            done.add(name)
            variants = groups[name]
            rets = {ast.dump(v.returns) if v.returns is not None else "" for v in variants}
            ret = copy.deepcopy(variants[0].returns) if len(rets) == 1 and variants[0].returns is not None else ast.Name("Any")
            keep = [d for d in variants[0].decorator_list if _dotted(d) in ("staticmethod", "classmethod")]
            static = any(_dotted(d) == "staticmethod" for d in keep)
            first = [] if static else [ast.arg(arg="cls" if name == "__new__" else "self")]
            args = ast.arguments(
                posonlyargs=[], args=first, vararg=ast.arg(arg="args", annotation=ast.Name("Any")),
                kwonlyargs=[], kw_defaults=[], kwarg=ast.arg(arg="kwargs", annotation=ast.Name("Any")), defaults=[],
            )
            body.append(ast.FunctionDef(name=name, args=args, body=[ast.Expr(ast.Constant(...))],
                                        decorator_list=keep, returns=ret, type_params=[]))
        else:
            body.append(st)
    cls.body = body


def _new_to_init(cls):
    names = {getattr(st, "name", None) for st in cls.body}
    if "__new__" not in names:
        return
    if "__init__" in names:
        cls.body = [st for st in cls.body if getattr(st, "name", None) != "__new__"]
        return
    for st in cls.body:
        if isinstance(st, ast.FunctionDef) and st.name == "__new__":
            st.name = "__init__"
            st.decorator_list = [d for d in st.decorator_list if _dotted(d) != "staticmethod"]
            first = st.args.posonlyargs or st.args.args
            if first:
                first[0].arg = "self"
                first[0].annotation = None
            st.returns = ast.Constant(None)


def flatten(source, skip_classes=False):
    """typeshed の `.pyi` の中身 `source` を、抽出器で読める `.pyi` の中身にする。"""
    tree = ast.parse(source)
    tree.body = _flatten(tree.body)
    typevars, aliases, body = set(), {}, []
    for st in tree.body:
        if isinstance(st, ast.Assign) and len(st.targets) == 1 and isinstance(st.targets[0], ast.Name):
            name = st.targets[0].id
            if isinstance(st.value, ast.Call) and (_dotted(st.value.func) or "").split(".")[-1] in _TYPEVAR_CALLS:
                typevars.add(name)
                continue
            if name.startswith("_") and isinstance(st.value, (ast.Subscript, ast.BinOp, ast.Name, ast.Attribute)):
                aliases[name] = st.value
                continue
        if isinstance(st, ast.AnnAssign) and isinstance(st.target, ast.Name):
            ann = _dotted(st.annotation)
            if ann in ("TypeAlias", "typing.TypeAlias", "typing_extensions.TypeAlias") and st.value is not None:
                aliases[st.target.id] = st.value
                continue
            if ann in ("Final", "typing.Final"):
                st.annotation = ast.Name(_type_of_constant(st.value) if st.value is not None else "Any")
        if isinstance(st, ast.TypeAlias):
            aliases[st.name.id] = st.value
            continue
        if isinstance(st, ast.ClassDef):
            if skip_classes:
                continue
            st.body = [m for m in st.body if getattr(m, "name", None) not in _HOOKS]
            _merge_overloaded_methods(st)
            _new_to_init(st)
            if not st.body:
                st.body = [ast.Expr(ast.Constant(...))]
        body.append(st)
    tree.body = body
    _rewrite_annotations(tree, _Rewrite(typevars, aliases))
    ast.fix_missing_locations(tree)
    return ast.unparse(tree)
