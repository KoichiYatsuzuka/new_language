# pyexpat.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/pyexpat/__init__.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer, SupportsRead
from collections.abc import Callable
from pyexpat import errors as errors, model as model
from typing import Any, Final, TypeAlias, final
from typing_extensions import CapsuleType
from xml.parsers.expat import ExpatError as ExpatError
EXPAT_VERSION: Final[str]
version_info: tuple[int, int, int]
native_encoding: str
features: list[tuple[str, int]]
error = ExpatError
XML_PARAM_ENTITY_PARSING_NEVER: int = 0
XML_PARAM_ENTITY_PARSING_UNLESS_STANDALONE: int = 1
XML_PARAM_ENTITY_PARSING_ALWAYS: int = 2

@final
class XMLParserType:

    def Parse(self, data: str | ReadableBuffer, isfinal: bool=False, /) -> int:
        ...

    def ParseFile(self, file: SupportsRead[bytes], /) -> int:
        ...

    def SetBase(self, base: str, /) -> None:
        ...

    def GetBase(self) -> str | None:
        ...

    def GetInputContext(self) -> bytes | None:
        ...

    def ExternalEntityParserCreate(self, context: str | None, encoding: str=..., /) -> XMLParserType:
        ...

    def SetParamEntityParsing(self, flag: int, /) -> int:
        ...

    def UseForeignDTD(self, flag: bool=True, /) -> None:
        ...

    def GetReparseDeferralEnabled(self) -> bool:
        ...

    def SetReparseDeferralEnabled(self, enabled: bool, /) -> None:
        ...

    def SetAllocTrackerActivationThreshold(self, threshold: int, /) -> None:
        ...

    def SetAllocTrackerMaximumAmplification(self, max_factor: float, /) -> None:
        ...

    @property
    def intern(self) -> dict[str, str]:
        ...
    buffer_size: int
    buffer_text: bool
    buffer_used: int
    namespace_prefixes: bool
    ordered_attributes: bool
    specified_attributes: bool
    ErrorByteIndex: int
    ErrorCode: int
    ErrorColumnNumber: int
    ErrorLineNumber: int
    CurrentByteIndex: int
    CurrentColumnNumber: int
    CurrentLineNumber: int
    XmlDeclHandler: Callable[[str, str | None, int], Any] | None
    StartDoctypeDeclHandler: Callable[[str, str | None, str | None, bool], Any] | None
    EndDoctypeDeclHandler: Callable[[], Any] | None
    ElementDeclHandler: Callable[[str, tuple[int, int, str | None, tuple[Any, ...]]], Any] | None
    AttlistDeclHandler: Callable[[str, str, str, str | None, bool], Any] | None
    StartElementHandler: Callable[[str, dict[str, str]], Any] | Callable[[str, list[str]], Any] | Callable[[str, dict[str, str], list[str]], Any] | None
    EndElementHandler: Callable[[str], Any] | None
    ProcessingInstructionHandler: Callable[[str, str], Any] | None
    CharacterDataHandler: Callable[[str], Any] | None
    UnparsedEntityDeclHandler: Callable[[str, str | None, str, str | None, str], Any] | None
    EntityDeclHandler: Callable[[str, bool, str | None, str | None, str, str | None, str | None], Any] | None
    NotationDeclHandler: Callable[[str, str | None, str, str | None], Any] | None
    StartNamespaceDeclHandler: Callable[[str, str], Any] | None
    EndNamespaceDeclHandler: Callable[[str], Any] | None
    CommentHandler: Callable[[str], Any] | None
    StartCdataSectionHandler: Callable[[], Any] | None
    EndCdataSectionHandler: Callable[[], Any] | None
    DefaultHandler: Callable[[str], Any] | None
    DefaultHandlerExpand: Callable[[str], Any] | None
    NotStandaloneHandler: Callable[[], int] | None
    ExternalEntityRefHandler: Callable[[str, str | None, str | None, str | None], int] | None
    SkippedEntityHandler: Callable[[str, bool], Any] | None

def ErrorString(code: int, /) -> str:
    ...

def ParserCreate(encoding: str | None=None, namespace_separator: str | None=None, intern: dict[str, Any] | None=None) -> XMLParserType:
    ...
expat_CAPI: CapsuleType
