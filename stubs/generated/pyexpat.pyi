# pyexpat.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

EXPAT_VERSION: str
def ErrorString(code: Any, /) -> Any: ...
class ExpatError:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    def __str__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
def ParserCreate(*args: Any, **kwargs: Any) -> Any: ...
class XMLParserType:
    def __init__(self) -> None: ...
    AttlistDeclHandler: Any
    CharacterDataHandler: Any
    CommentHandler: Any
    CurrentByteIndex: Any
    CurrentColumnNumber: Any
    CurrentLineNumber: Any
    DefaultHandler: Any
    DefaultHandlerExpand: Any
    ElementDeclHandler: Any
    EndCdataSectionHandler: Any
    EndDoctypeDeclHandler: Any
    EndElementHandler: Any
    EndNamespaceDeclHandler: Any
    EntityDeclHandler: Any
    ErrorByteIndex: Any
    ErrorCode: Any
    ErrorColumnNumber: Any
    ErrorLineNumber: Any
    def ExternalEntityParserCreate(self, *args: Any, **kwargs: Any) -> Any: ...
    ExternalEntityRefHandler: Any
    def GetBase(self) -> Any: ...
    def GetInputContext(self) -> Any: ...
    NotStandaloneHandler: Any
    NotationDeclHandler: Any
    def Parse(self, data: Any, isfinal: Any = ..., /) -> Any: ...
    def ParseFile(self, file: Any, /) -> Any: ...
    ProcessingInstructionHandler: Any
    def SetBase(self, base: Any, /) -> Any: ...
    def SetParamEntityParsing(self, flag: Any, /) -> Any: ...
    SkippedEntityHandler: Any
    StartCdataSectionHandler: Any
    StartDoctypeDeclHandler: Any
    StartElementHandler: Any
    StartNamespaceDeclHandler: Any
    UnparsedEntityDeclHandler: Any
    def UseForeignDTD(self, flag: Any = ..., /) -> Any: ...
    XmlDeclHandler: Any
    buffer_size: Any
    buffer_text: Any
    buffer_used: Any
    intern: Any
    namespace_prefixes: Any
    ordered_attributes: Any
    specified_attributes: Any
XML_PARAM_ENTITY_PARSING_ALWAYS: int
XML_PARAM_ENTITY_PARSING_NEVER: int
XML_PARAM_ENTITY_PARSING_UNLESS_STANDALONE: int
class error:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    def __str__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
expat_CAPI: Any
features: list[Any]
native_encoding: str
version_info: Any
