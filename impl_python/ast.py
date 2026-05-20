from __future__ import annotations
from dataclasses import dataclass, field
from enum import Enum, auto
from typing import Optional

from .token import Span


# ---------------------------------------------------------------------------
# Helpers shared by Expr / Stmt
# ---------------------------------------------------------------------------

class BinOp(Enum):
    ADD       = auto()   # +
    SUB       = auto()   # -
    MUL       = auto()   # *
    DIV       = auto()   # /
    FLOOR_DIV = auto()   # //
    MOD       = auto()   # %
    POW       = auto()   # **
    EQ        = auto()   # ==
    NOT_EQ    = auto()   # !=
    LT        = auto()   # <
    GT        = auto()   # >
    LT_EQ     = auto()   # <=
    GT_EQ     = auto()   # >=
    AND       = auto()   # and
    OR        = auto()   # or
    BIT_AND   = auto()   # &
    BIT_OR    = auto()   # |
    BIT_XOR   = auto()   # ^
    L_SHIFT   = auto()   # <<
    R_SHIFT   = auto()   # >>
    IN        = auto()   # in
    NOT_IN    = auto()   # not in

    def as_str(self) -> str:
        return {
            BinOp.ADD: "+", BinOp.SUB: "-", BinOp.MUL: "*",
            BinOp.DIV: "/", BinOp.FLOOR_DIV: "//", BinOp.MOD: "%",
            BinOp.POW: "**",
            BinOp.EQ: "==", BinOp.NOT_EQ: "!=",
            BinOp.LT: "<", BinOp.GT: ">", BinOp.LT_EQ: "<=", BinOp.GT_EQ: ">=",
            BinOp.AND: "and", BinOp.OR: "or",
            BinOp.BIT_AND: "&", BinOp.BIT_OR: "|", BinOp.BIT_XOR: "^",
            BinOp.L_SHIFT: "<<", BinOp.R_SHIFT: ">>",
            BinOp.IN: "in", BinOp.NOT_IN: "not in",
        }[self]


class UnaryOp(Enum):
    NEG     = auto()   # -x
    NOT     = auto()   # not x
    BIT_NOT = auto()   # ~x


class Accessibility(Enum):
    PUBLIC    = auto()
    PRIVATE   = auto()
    PROTECTED = auto()


class FieldKind(Enum):
    MUT        = auto()
    LET        = auto()
    CONST      = auto()
    STATIC_MUT = auto()


# ---------------------------------------------------------------------------
# Tuple unpack targets
# ---------------------------------------------------------------------------

@dataclass
class TupleTargetLet:
    name: str

@dataclass
class TupleTargetMut:
    name: str

@dataclass
class TupleTargetBare:
    name: str

@dataclass
class TupleTargetWildcard:
    pass

TupleTarget = TupleTargetLet | TupleTargetMut | TupleTargetBare | TupleTargetWildcard


# ---------------------------------------------------------------------------
# Template / parameter helpers
# ---------------------------------------------------------------------------

@dataclass
class TemplateParam:
    name: str
    constraints: list[str] = field(default_factory=list)


@dataclass
class Param:
    name: str
    mutable: bool = False
    type_ann: Optional[str] = None
    default: Optional["Expr"] = None


# ---------------------------------------------------------------------------
# Call arguments
# ---------------------------------------------------------------------------

@dataclass
class CallArgPositional:
    expr: "Expr"

    def get_expr(self) -> "Expr":
        return self.expr


@dataclass
class CallArgKeyword:
    name: str
    value: "Expr"

    def get_expr(self) -> "Expr":
        return self.value


CallArg = CallArgPositional | CallArgKeyword


# ---------------------------------------------------------------------------
# Match patterns and arms
# ---------------------------------------------------------------------------

@dataclass
class MatchPatternCase:
    expr: "Expr"


@dataclass
class MatchPatternIsType:
    type_name: str


MatchPattern = MatchPatternCase | MatchPatternIsType


@dataclass
class MatchArm:
    pattern: MatchPattern
    body: list["Stmt"]


# ---------------------------------------------------------------------------
# ExceptHandler
# ---------------------------------------------------------------------------

@dataclass
class ExceptHandler:
    exc_type: Optional[str]
    name: Optional[str]
    body: list["Stmt"]


# ---------------------------------------------------------------------------
# Expr variants
# ---------------------------------------------------------------------------

@dataclass
class ExprInt:
    value: int

@dataclass
class ExprFloat:
    value: float

@dataclass
class ExprStr:
    value: str

@dataclass
class ExprBool:
    value: bool

@dataclass
class ExprNone:
    pass

@dataclass
class ExprIdent:
    name: str

@dataclass
class ExprList:
    elements: list["Expr"]

@dataclass
class ExprAttr:
    object: "Expr"
    attr: str

@dataclass
class ExprTraitAccess:
    object: "Expr"
    trait_name: str
    attr: str

@dataclass
class ExprBinOp:
    op: BinOp
    left: "Expr"
    right: "Expr"
    span: Span

@dataclass
class ExprUnaryOp:
    op: UnaryOp
    operand: "Expr"

@dataclass
class ExprCall:
    func: "Expr"
    args: list[CallArg]

@dataclass
class ExprTemplateInstantiate:
    base: "Expr"
    type_args: list[str]

@dataclass
class ExprSubscript:
    object: "Expr"
    index: "Expr"

@dataclass
class ExprSlice:
    begin: Optional["Expr"]
    end: Optional["Expr"]
    step: Optional["Expr"]

@dataclass
class ExprDict:
    pairs: list[tuple["Expr", "Expr"]]

@dataclass
class ExprTuple:
    elements: list["Expr"]

@dataclass
class ExprSet:
    elements: list["Expr"]

@dataclass
class ExprBlock:
    stmts: list["Stmt"]
    return_type: Optional[str]

@dataclass
class ExprIfExpr:
    branches: list[tuple["Expr", list["Stmt"]]]
    else_body: Optional[list["Stmt"]]
    return_type: Optional[str]

@dataclass
class ExprForExpr:
    target: str
    iter: "Expr"
    body: list["Stmt"]
    return_type: Optional[str]

@dataclass
class ExprWhileExpr:
    cond: "Expr"
    body: list["Stmt"]
    return_type: Optional[str]

@dataclass
class ExprMatchExpr:
    subject: "Expr"
    arms: list[MatchArm]
    return_type: Optional[str]

@dataclass
class ExprIsType:
    expr: "Expr"
    negated: bool
    type_name: str
    span: Span


Expr = (
    ExprInt | ExprFloat | ExprStr | ExprBool | ExprNone | ExprIdent |
    ExprList | ExprAttr | ExprTraitAccess | ExprBinOp | ExprUnaryOp |
    ExprCall | ExprTemplateInstantiate | ExprSubscript | ExprSlice |
    ExprDict | ExprTuple | ExprSet | ExprBlock | ExprIfExpr |
    ExprForExpr | ExprWhileExpr | ExprMatchExpr | ExprIsType
)


# ---------------------------------------------------------------------------
# Stmt variants
# ---------------------------------------------------------------------------

@dataclass
class StmtExpr:
    expr: Expr

@dataclass
class StmtLet:
    name: str
    expr: Expr

@dataclass
class StmtConst:
    name: str
    expr: Expr

@dataclass
class StmtMut:
    name: str
    expr: Expr

@dataclass
class StmtStatic:
    name: str
    expr: Expr
    span: Span

@dataclass
class StmtAssign:
    name: str
    value: Expr
    span: Span

@dataclass
class StmtAttrAssign:
    target: Expr
    value: Expr

@dataclass
class StmtAttrCompoundAssign:
    target: Expr
    op: BinOp
    value: Expr

@dataclass
class StmtCompoundAssign:
    name: str
    op: BinOp
    value: Expr
    span: Span

@dataclass
class StmtIf:
    branches: list[tuple[Expr, list["Stmt"]]]
    else_body: Optional[list["Stmt"]]

@dataclass
class StmtMatch:
    subject: Expr
    arms: list[MatchArm]
    span: Span

@dataclass
class StmtWhile:
    cond: Expr
    body: list["Stmt"]

@dataclass
class StmtFor:
    targets: list[str]
    iter: Expr
    body: list["Stmt"]

@dataclass
class StmtLetTuple:
    targets: list  # list[TupleTarget]
    value: "Expr"
    span: Span

@dataclass
class StmtAsyncAssign:
    target: str
    return_type: Optional[str]
    stmts: list["Stmt"]

@dataclass
class StmtBlock:
    stmts: list["Stmt"]

@dataclass
class StmtReturn:
    expr: Optional[Expr]

@dataclass
class StmtBreak:
    pass

@dataclass
class StmtContinue:
    pass

@dataclass
class StmtPass:
    pass

@dataclass
class StmtBlockReturn:
    expr: Expr

@dataclass
class StmtLoopYield:
    expr: Expr

@dataclass
class StmtYield:
    expr: Expr

@dataclass
class StmtFreeze:
    name: str
    span: Span

@dataclass
class StmtFnDef:
    name: str
    template_params: list[TemplateParam]
    params: list[Param]
    return_type: Optional[str]
    body: list["Stmt"]
    is_abstract: bool = False
    is_static: bool = False
    is_class_method: bool = False
    decorators: list[Expr] = field(default_factory=list)
    access: Accessibility = Accessibility.PUBLIC

@dataclass
class StmtGenDef:
    name: str
    template_params: list[TemplateParam]
    params: list[Param]
    yield_type: Optional[str]
    body: list["Stmt"]
    access: Accessibility = Accessibility.PUBLIC

@dataclass
class StmtClassDef:
    name: str
    template_params: list[TemplateParam]
    bases: list[str]
    decorators: list[Expr]
    body: list["Stmt"]

@dataclass
class StmtTraitDef:
    name: str
    template_params: list[TemplateParam]
    body: list["Stmt"]

@dataclass
class StmtField:
    name: str
    kind: FieldKind
    type_ann: str
    default: Optional[Expr]
    access: Accessibility = Accessibility.PUBLIC

@dataclass
class StmtNewTypeDef:
    name: str
    original: str

@dataclass
class StmtEnumDef:
    name: str
    variants: list[tuple[str, Optional[Expr]]]

@dataclass
class StmtTry:
    body: list["Stmt"]
    handlers: list[ExceptHandler]
    finally_body: Optional[list["Stmt"]]

@dataclass
class StmtRaise:
    exc: Optional[Expr]
    span: Span

@dataclass
class StmtImport:
    lang: str
    module: list[str]
    alias: Optional[str]
    body: list["Stmt"]

@dataclass
class StmtFromImport:
    lang: str
    module: list[str]
    names: list[tuple[str, Optional[str]]]
    body: list["Stmt"]


Stmt = (
    StmtExpr | StmtLet | StmtConst | StmtMut | StmtStatic |
    StmtAssign | StmtAttrAssign | StmtAttrCompoundAssign | StmtCompoundAssign |
    StmtIf | StmtMatch | StmtWhile | StmtFor | StmtBlock |
    StmtReturn | StmtBreak | StmtContinue | StmtPass |
    StmtBlockReturn | StmtLoopYield | StmtYield | StmtFreeze |
    StmtFnDef | StmtGenDef | StmtClassDef | StmtTraitDef | StmtField |
    StmtNewTypeDef | StmtEnumDef | StmtTry | StmtRaise |
    StmtImport | StmtFromImport |
    StmtLetTuple | StmtAsyncAssign
)
