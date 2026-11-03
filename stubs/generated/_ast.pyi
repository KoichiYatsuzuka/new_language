# _ast.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

class AST:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Add:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class And:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class AnnAssign:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    value: Any
    end_col_offset: Any
    end_lineno: Any
class Assert:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    msg: Any
    end_col_offset: Any
    end_lineno: Any
class Assign:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    type_comment: Any
    end_col_offset: Any
    end_lineno: Any
class AsyncFor:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    type_comment: Any
    end_col_offset: Any
    end_lineno: Any
class AsyncFunctionDef:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    returns: Any
    type_comment: Any
    end_col_offset: Any
    end_lineno: Any
class AsyncWith:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    type_comment: Any
    end_col_offset: Any
    end_lineno: Any
class Attribute:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class AugAssign:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Await:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class BinOp:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class BitAnd:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class BitOr:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class BitXor:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class BoolOp:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Break:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Call:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class ClassDef:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Compare:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Constant:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    kind: Any
    n: Any
    s: Any
    end_col_offset: Any
    end_lineno: Any
class Continue:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Del:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Delete:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Dict:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class DictComp:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Div:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Eq:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class ExceptHandler:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    name: Any
    type: Any
    end_col_offset: Any
    end_lineno: Any
class Expr:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Expression:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class FloorDiv:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class For:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    type_comment: Any
    end_col_offset: Any
    end_lineno: Any
class FormattedValue:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    format_spec: Any
    end_col_offset: Any
    end_lineno: Any
class FunctionDef:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    returns: Any
    type_comment: Any
    end_col_offset: Any
    end_lineno: Any
class FunctionType:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class GeneratorExp:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Global:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Gt:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class GtE:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class If:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class IfExp:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Import:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class ImportFrom:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    level: Any
    module: Any
    end_col_offset: Any
    end_lineno: Any
class In:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Interactive:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Invert:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Is:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class IsNot:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class JoinedStr:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class LShift:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Lambda:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class List:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class ListComp:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Load:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Lt:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class LtE:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class MatMult:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Match:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class MatchAs:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    name: Any
    pattern: Any
class MatchClass:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class MatchMapping:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    rest: Any
class MatchOr:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class MatchSequence:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class MatchSingleton:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class MatchStar:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    name: Any
class MatchValue:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Mod:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Module:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Mult:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Name:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class NamedExpr:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Nonlocal:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Not:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class NotEq:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class NotIn:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Or:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class ParamSpec:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Pass:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Pow:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
PyCF_ALLOW_TOP_LEVEL_AWAIT: int
PyCF_ONLY_AST: int
PyCF_TYPE_COMMENTS: int
class RShift:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Raise:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    cause: Any
    exc: Any
    end_col_offset: Any
    end_lineno: Any
class Return:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    value: Any
    end_col_offset: Any
    end_lineno: Any
class Set:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class SetComp:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Slice:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    lower: Any
    step: Any
    upper: Any
    end_col_offset: Any
    end_lineno: Any
class Starred:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Store:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Sub:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class Subscript:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Try:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class TryStar:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class Tuple:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    dims: Any
    end_col_offset: Any
    end_lineno: Any
class TypeAlias:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class TypeIgnore:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class TypeVar:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    bound: Any
class TypeVarTuple:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class UAdd:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class USub:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class UnaryOp:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class While:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class With:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    type_comment: Any
    end_col_offset: Any
    end_lineno: Any
class Yield:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    value: Any
    end_col_offset: Any
    end_lineno: Any
class YieldFrom:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class alias:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    asname: Any
    end_col_offset: Any
    end_lineno: Any
class arg:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    annotation: Any
    end_col_offset: Any
    end_lineno: Any
    type_comment: Any
class arguments:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    kwarg: Any
    vararg: Any
class boolop:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class cmpop:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class comprehension:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class excepthandler:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class expr:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class expr_context:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class keyword:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    arg: Any
    end_col_offset: Any
    end_lineno: Any
class match_case:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    guard: Any
class mod:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class operator:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class pattern:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class stmt:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    end_col_offset: Any
    end_lineno: Any
class type_ignore:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class type_param:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class unaryop:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
class withitem:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __match_args__: Any
    optional_vars: Any
