# _socket.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

AF_APPLETALK: int
AF_BLUETOOTH: int
AF_DECnet: int
AF_HYPERV: int
AF_INET: int
AF_INET6: int
AF_IPX: int
AF_IRDA: int
AF_LINK: int
AF_SNA: int
AF_UNSPEC: int
AI_ADDRCONFIG: int
AI_ALL: int
AI_CANONNAME: int
AI_NUMERICHOST: int
AI_NUMERICSERV: int
AI_PASSIVE: int
AI_V4MAPPED: int
BDADDR_ANY: str
BDADDR_LOCAL: str
BTPROTO_RFCOMM: int
CAPI: Any
EAI_AGAIN: int
EAI_BADFLAGS: int
EAI_FAIL: int
EAI_FAMILY: int
EAI_MEMORY: int
EAI_NODATA: int
EAI_NONAME: int
EAI_SERVICE: int
EAI_SOCKTYPE: int
HVSOCKET_ADDRESS_FLAG_PASSTHRU: int
HVSOCKET_CONNECTED_SUSPEND: int
HVSOCKET_CONNECT_TIMEOUT: int
HVSOCKET_CONNECT_TIMEOUT_MAX: int
HV_GUID_BROADCAST: str
HV_GUID_CHILDREN: str
HV_GUID_LOOPBACK: str
HV_GUID_PARENT: str
HV_GUID_WILDCARD: str
HV_GUID_ZERO: str
HV_PROTOCOL_RAW: int
INADDR_ALLHOSTS_GROUP: int
INADDR_ANY: int
INADDR_BROADCAST: int
INADDR_LOOPBACK: int
INADDR_MAX_LOCAL_GROUP: int
INADDR_NONE: int
INADDR_UNSPEC_GROUP: int
IPPORT_RESERVED: int
IPPORT_USERRESERVED: int
IPPROTO_AH: int
IPPROTO_CBT: int
IPPROTO_DSTOPTS: int
IPPROTO_EGP: int
IPPROTO_ESP: int
IPPROTO_FRAGMENT: int
IPPROTO_GGP: int
IPPROTO_HOPOPTS: int
IPPROTO_ICLFXBM: int
IPPROTO_ICMP: int
IPPROTO_ICMPV6: int
IPPROTO_IDP: int
IPPROTO_IGMP: int
IPPROTO_IGP: int
IPPROTO_IP: int
IPPROTO_IPV4: int
IPPROTO_IPV6: int
IPPROTO_L2TP: int
IPPROTO_MAX: int
IPPROTO_ND: int
IPPROTO_NONE: int
IPPROTO_PGM: int
IPPROTO_PIM: int
IPPROTO_PUP: int
IPPROTO_RAW: int
IPPROTO_RDP: int
IPPROTO_ROUTING: int
IPPROTO_SCTP: int
IPPROTO_ST: int
IPPROTO_TCP: int
IPPROTO_UDP: int
IPV6_CHECKSUM: int
IPV6_DONTFRAG: int
IPV6_HOPLIMIT: int
IPV6_HOPOPTS: int
IPV6_JOIN_GROUP: int
IPV6_LEAVE_GROUP: int
IPV6_MULTICAST_HOPS: int
IPV6_MULTICAST_IF: int
IPV6_MULTICAST_LOOP: int
IPV6_PKTINFO: int
IPV6_RECVRTHDR: int
IPV6_RECVTCLASS: int
IPV6_RTHDR: int
IPV6_TCLASS: int
IPV6_UNICAST_HOPS: int
IPV6_V6ONLY: int
IP_ADD_MEMBERSHIP: int
IP_ADD_SOURCE_MEMBERSHIP: int
IP_BLOCK_SOURCE: int
IP_DROP_MEMBERSHIP: int
IP_DROP_SOURCE_MEMBERSHIP: int
IP_HDRINCL: int
IP_MULTICAST_IF: int
IP_MULTICAST_LOOP: int
IP_MULTICAST_TTL: int
IP_OPTIONS: int
IP_PKTINFO: int
IP_RECVDSTADDR: int
IP_RECVTOS: int
IP_TOS: int
IP_TTL: int
IP_UNBLOCK_SOURCE: int
MSG_BCAST: int
MSG_CTRUNC: int
MSG_DONTROUTE: int
MSG_ERRQUEUE: int
MSG_MCAST: int
MSG_OOB: int
MSG_PEEK: int
MSG_TRUNC: int
MSG_WAITALL: int
NI_DGRAM: int
NI_MAXHOST: int
NI_MAXSERV: int
NI_NAMEREQD: int
NI_NOFQDN: int
NI_NUMERICHOST: int
NI_NUMERICSERV: int
RCVALL_MAX: int
RCVALL_OFF: int
RCVALL_ON: int
RCVALL_SOCKETLEVELONLY: int
SHUT_RD: int
SHUT_RDWR: int
SHUT_WR: int
SIO_KEEPALIVE_VALS: int
SIO_LOOPBACK_FAST_PATH: int
SIO_RCVALL: int
SOCK_DGRAM: int
SOCK_RAW: int
SOCK_RDM: int
SOCK_SEQPACKET: int
SOCK_STREAM: int
SOL_IP: int
SOL_SOCKET: int
SOL_TCP: int
SOL_UDP: int
SOMAXCONN: int
SO_ACCEPTCONN: int
SO_BROADCAST: int
SO_DEBUG: int
SO_DONTROUTE: int
SO_ERROR: int
SO_EXCLUSIVEADDRUSE: int
SO_KEEPALIVE: int
SO_LINGER: int
SO_OOBINLINE: int
SO_RCVBUF: int
SO_RCVLOWAT: int
SO_RCVTIMEO: int
SO_REUSEADDR: int
SO_SNDBUF: int
SO_SNDLOWAT: int
SO_SNDTIMEO: int
SO_TYPE: int
SO_USELOOPBACK: int
class SocketType:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __del__(self, *args: Any, **kwargs: Any) -> Any: ...
    def __repr__(self) -> Any: ...
    def bind(self, *args: Any, **kwargs: Any) -> Any: ...
    def close(self, *args: Any, **kwargs: Any) -> Any: ...
    def connect(self, *args: Any, **kwargs: Any) -> Any: ...
    def connect_ex(self, *args: Any, **kwargs: Any) -> Any: ...
    def detach(self, *args: Any, **kwargs: Any) -> Any: ...
    family: Any
    def fileno(self, *args: Any, **kwargs: Any) -> Any: ...
    def getblocking(self, *args: Any, **kwargs: Any) -> Any: ...
    def getpeername(self, *args: Any, **kwargs: Any) -> Any: ...
    def getsockname(self, *args: Any, **kwargs: Any) -> Any: ...
    def getsockopt(self, *args: Any, **kwargs: Any) -> Any: ...
    def gettimeout(self, *args: Any, **kwargs: Any) -> Any: ...
    def ioctl(self, *args: Any, **kwargs: Any) -> Any: ...
    def listen(self, *args: Any, **kwargs: Any) -> Any: ...
    proto: Any
    def recv(self, *args: Any, **kwargs: Any) -> Any: ...
    def recv_into(self, *args: Any, **kwargs: Any) -> Any: ...
    def recvfrom(self, *args: Any, **kwargs: Any) -> Any: ...
    def recvfrom_into(self, *args: Any, **kwargs: Any) -> Any: ...
    def send(self, *args: Any, **kwargs: Any) -> Any: ...
    def sendall(self, *args: Any, **kwargs: Any) -> Any: ...
    def sendto(self, *args: Any, **kwargs: Any) -> Any: ...
    def setblocking(self, *args: Any, **kwargs: Any) -> Any: ...
    def setsockopt(self, *args: Any, **kwargs: Any) -> Any: ...
    def settimeout(self, *args: Any, **kwargs: Any) -> Any: ...
    def share(self, *args: Any, **kwargs: Any) -> Any: ...
    def shutdown(self, *args: Any, **kwargs: Any) -> Any: ...
    timeout: Any
    type: Any
TCP_FASTOPEN: int
TCP_KEEPCNT: int
TCP_KEEPIDLE: int
TCP_KEEPINTVL: int
TCP_MAXSEG: int
TCP_NODELAY: int
def close(*args: Any, **kwargs: Any) -> Any: ...
def dup(*args: Any, **kwargs: Any) -> Any: ...
class error:
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
class gaierror:
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
def getaddrinfo(*args: Any, **kwargs: Any) -> Any: ...
def getdefaulttimeout(*args: Any, **kwargs: Any) -> Any: ...
def gethostbyaddr(*args: Any, **kwargs: Any) -> Any: ...
def gethostbyname(*args: Any, **kwargs: Any) -> Any: ...
def gethostbyname_ex(*args: Any, **kwargs: Any) -> Any: ...
def gethostname(*args: Any, **kwargs: Any) -> Any: ...
def getnameinfo(*args: Any, **kwargs: Any) -> Any: ...
def getprotobyname(*args: Any, **kwargs: Any) -> Any: ...
def getservbyname(*args: Any, **kwargs: Any) -> Any: ...
def getservbyport(*args: Any, **kwargs: Any) -> Any: ...
has_ipv6: bool
class herror:
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
def htonl(*args: Any, **kwargs: Any) -> Any: ...
def htons(*args: Any, **kwargs: Any) -> Any: ...
def if_indextoname(*args: Any, **kwargs: Any) -> Any: ...
def if_nameindex(*args: Any, **kwargs: Any) -> Any: ...
def if_nametoindex(*args: Any, **kwargs: Any) -> Any: ...
def inet_aton(*args: Any, **kwargs: Any) -> Any: ...
def inet_ntoa(*args: Any, **kwargs: Any) -> Any: ...
def inet_ntop(*args: Any, **kwargs: Any) -> Any: ...
def inet_pton(*args: Any, **kwargs: Any) -> Any: ...
def ntohl(*args: Any, **kwargs: Any) -> Any: ...
def ntohs(*args: Any, **kwargs: Any) -> Any: ...
def setdefaulttimeout(*args: Any, **kwargs: Any) -> Any: ...
class socket:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __del__(self, *args: Any, **kwargs: Any) -> Any: ...
    def __repr__(self) -> Any: ...
    def bind(self, *args: Any, **kwargs: Any) -> Any: ...
    def close(self, *args: Any, **kwargs: Any) -> Any: ...
    def connect(self, *args: Any, **kwargs: Any) -> Any: ...
    def connect_ex(self, *args: Any, **kwargs: Any) -> Any: ...
    def detach(self, *args: Any, **kwargs: Any) -> Any: ...
    family: Any
    def fileno(self, *args: Any, **kwargs: Any) -> Any: ...
    def getblocking(self, *args: Any, **kwargs: Any) -> Any: ...
    def getpeername(self, *args: Any, **kwargs: Any) -> Any: ...
    def getsockname(self, *args: Any, **kwargs: Any) -> Any: ...
    def getsockopt(self, *args: Any, **kwargs: Any) -> Any: ...
    def gettimeout(self, *args: Any, **kwargs: Any) -> Any: ...
    def ioctl(self, *args: Any, **kwargs: Any) -> Any: ...
    def listen(self, *args: Any, **kwargs: Any) -> Any: ...
    proto: Any
    def recv(self, *args: Any, **kwargs: Any) -> Any: ...
    def recv_into(self, *args: Any, **kwargs: Any) -> Any: ...
    def recvfrom(self, *args: Any, **kwargs: Any) -> Any: ...
    def recvfrom_into(self, *args: Any, **kwargs: Any) -> Any: ...
    def send(self, *args: Any, **kwargs: Any) -> Any: ...
    def sendall(self, *args: Any, **kwargs: Any) -> Any: ...
    def sendto(self, *args: Any, **kwargs: Any) -> Any: ...
    def setblocking(self, *args: Any, **kwargs: Any) -> Any: ...
    def setsockopt(self, *args: Any, **kwargs: Any) -> Any: ...
    def settimeout(self, *args: Any, **kwargs: Any) -> Any: ...
    def share(self, *args: Any, **kwargs: Any) -> Any: ...
    def shutdown(self, *args: Any, **kwargs: Any) -> Any: ...
    timeout: Any
    type: Any
class timeout:
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
