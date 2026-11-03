# _ssl.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

ALERT_DESCRIPTION_ACCESS_DENIED: int
ALERT_DESCRIPTION_BAD_CERTIFICATE: int
ALERT_DESCRIPTION_BAD_CERTIFICATE_HASH_VALUE: int
ALERT_DESCRIPTION_BAD_CERTIFICATE_STATUS_RESPONSE: int
ALERT_DESCRIPTION_BAD_RECORD_MAC: int
ALERT_DESCRIPTION_CERTIFICATE_EXPIRED: int
ALERT_DESCRIPTION_CERTIFICATE_REVOKED: int
ALERT_DESCRIPTION_CERTIFICATE_UNKNOWN: int
ALERT_DESCRIPTION_CERTIFICATE_UNOBTAINABLE: int
ALERT_DESCRIPTION_CLOSE_NOTIFY: int
ALERT_DESCRIPTION_DECODE_ERROR: int
ALERT_DESCRIPTION_DECOMPRESSION_FAILURE: int
ALERT_DESCRIPTION_DECRYPT_ERROR: int
ALERT_DESCRIPTION_HANDSHAKE_FAILURE: int
ALERT_DESCRIPTION_ILLEGAL_PARAMETER: int
ALERT_DESCRIPTION_INSUFFICIENT_SECURITY: int
ALERT_DESCRIPTION_INTERNAL_ERROR: int
ALERT_DESCRIPTION_NO_RENEGOTIATION: int
ALERT_DESCRIPTION_PROTOCOL_VERSION: int
ALERT_DESCRIPTION_RECORD_OVERFLOW: int
ALERT_DESCRIPTION_UNEXPECTED_MESSAGE: int
ALERT_DESCRIPTION_UNKNOWN_CA: int
ALERT_DESCRIPTION_UNKNOWN_PSK_IDENTITY: int
ALERT_DESCRIPTION_UNRECOGNIZED_NAME: int
ALERT_DESCRIPTION_UNSUPPORTED_CERTIFICATE: int
ALERT_DESCRIPTION_UNSUPPORTED_EXTENSION: int
ALERT_DESCRIPTION_USER_CANCELLED: int
CERT_NONE: int
CERT_OPTIONAL: int
CERT_REQUIRED: int
class Certificate:
    def __init__(self) -> None: ...
    def __eq__(self, value: Any, /) -> Any: ...
    def __ge__(self, value: Any, /) -> Any: ...
    def __gt__(self, value: Any, /) -> Any: ...
    def __hash__(self) -> Any: ...
    def __le__(self, value: Any, /) -> Any: ...
    def __lt__(self, value: Any, /) -> Any: ...
    def __ne__(self, value: Any, /) -> Any: ...
    def __repr__(self) -> Any: ...
    def get_info(self) -> Any: ...
    def public_bytes(self, *args: Any, **kwargs: Any) -> Any: ...
ENCODING_DER: int
ENCODING_PEM: int
HAS_ALPN: bool
HAS_ECDH: bool
HAS_NPN: bool
HAS_SNI: bool
HAS_SSLv2: bool
HAS_SSLv3: bool
HAS_TLS_UNIQUE: bool
HAS_TLSv1: bool
HAS_TLSv1_1: bool
HAS_TLSv1_2: bool
HAS_TLSv1_3: bool
HOSTFLAG_ALWAYS_CHECK_SUBJECT: int
HOSTFLAG_MULTI_LABEL_WILDCARDS: int
HOSTFLAG_NEVER_CHECK_SUBJECT: int
HOSTFLAG_NO_PARTIAL_WILDCARDS: int
HOSTFLAG_NO_WILDCARDS: int
HOSTFLAG_SINGLE_LABEL_SUBDOMAINS: int
class MemoryBIO:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    eof: Any
    pending: Any
    def read(self, size: Any = ..., /) -> Any: ...
    def write(self, b: Any, /) -> Any: ...
    def write_eof(self) -> Any: ...
OPENSSL_VERSION: str
OPENSSL_VERSION_INFO: Any
OPENSSL_VERSION_NUMBER: int
OP_ALL: int
OP_CIPHER_SERVER_PREFERENCE: int
OP_ENABLE_KTLS: int
OP_ENABLE_MIDDLEBOX_COMPAT: int
OP_IGNORE_UNEXPECTED_EOF: int
OP_LEGACY_SERVER_CONNECT: int
OP_NO_COMPRESSION: int
OP_NO_RENEGOTIATION: int
OP_NO_SSLv2: int
OP_NO_SSLv3: int
OP_NO_TICKET: int
OP_NO_TLSv1: int
OP_NO_TLSv1_1: int
OP_NO_TLSv1_2: int
OP_NO_TLSv1_3: int
OP_SINGLE_DH_USE: int
OP_SINGLE_ECDH_USE: int
PROTOCOL_SSLv23: int
PROTOCOL_TLS: int
PROTOCOL_TLS_CLIENT: int
PROTOCOL_TLS_SERVER: int
PROTOCOL_TLSv1: int
PROTOCOL_TLSv1_1: int
PROTOCOL_TLSv1_2: int
PROTO_MAXIMUM_SUPPORTED: int
PROTO_MINIMUM_SUPPORTED: int
PROTO_SSLv3: int
PROTO_TLSv1: int
PROTO_TLSv1_1: int
PROTO_TLSv1_2: int
PROTO_TLSv1_3: int
def RAND_add(string: Any, entropy: Any, /) -> Any: ...
def RAND_bytes(n: Any, /) -> Any: ...
def RAND_status() -> Any: ...
class SSLCertVerificationError:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __str__(self) -> Any: ...
    characters_written: Any
    errno: Any
    filename: Any
    filename2: Any
    strerror: Any
    winerror: Any
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
class SSLEOFError:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __str__(self) -> Any: ...
    characters_written: Any
    errno: Any
    filename: Any
    filename2: Any
    strerror: Any
    winerror: Any
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
class SSLError:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __str__(self) -> Any: ...
    characters_written: Any
    errno: Any
    filename: Any
    filename2: Any
    strerror: Any
    winerror: Any
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
class SSLSession:
    def __init__(self) -> None: ...
    def __eq__(self, value: Any, /) -> Any: ...
    def __ge__(self, value: Any, /) -> Any: ...
    def __gt__(self, value: Any, /) -> Any: ...
    __hash__: Any
    def __le__(self, value: Any, /) -> Any: ...
    def __lt__(self, value: Any, /) -> Any: ...
    def __ne__(self, value: Any, /) -> Any: ...
    has_ticket: Any
    id: Any
    ticket_lifetime_hint: Any
    time: Any
    timeout: Any
class SSLSyscallError:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __str__(self) -> Any: ...
    characters_written: Any
    errno: Any
    filename: Any
    filename2: Any
    strerror: Any
    winerror: Any
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
class SSLWantReadError:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __str__(self) -> Any: ...
    characters_written: Any
    errno: Any
    filename: Any
    filename2: Any
    strerror: Any
    winerror: Any
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
class SSLWantWriteError:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __str__(self) -> Any: ...
    characters_written: Any
    errno: Any
    filename: Any
    filename2: Any
    strerror: Any
    winerror: Any
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
class SSLZeroReturnError:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __str__(self) -> Any: ...
    characters_written: Any
    errno: Any
    filename: Any
    filename2: Any
    strerror: Any
    winerror: Any
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
SSL_ERROR_EOF: int
SSL_ERROR_INVALID_ERROR_CODE: int
SSL_ERROR_SSL: int
SSL_ERROR_SYSCALL: int
SSL_ERROR_WANT_CONNECT: int
SSL_ERROR_WANT_READ: int
SSL_ERROR_WANT_WRITE: int
SSL_ERROR_WANT_X509_LOOKUP: int
SSL_ERROR_ZERO_RETURN: int
VERIFY_ALLOW_PROXY_CERTS: int
VERIFY_CRL_CHECK_CHAIN: int
VERIFY_CRL_CHECK_LEAF: int
VERIFY_DEFAULT: int
VERIFY_X509_PARTIAL_CHAIN: int
VERIFY_X509_STRICT: int
VERIFY_X509_TRUSTED_FIRST: int
def enum_certificates(store_name: Any) -> Any: ...
def enum_crls(store_name: Any) -> Any: ...
def get_default_verify_paths() -> Any: ...
def nid2obj(nid: Any, /) -> Any: ...
def txt2obj(txt: Any, name: Any = ...) -> Any: ...
