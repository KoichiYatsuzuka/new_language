# py_events.py — event_external_handler.ar から Signal ハンドラとして購読される Python 関数群
#
# 埋め込み Python の sys.stdout はプロセス終了時にフラッシュされないことがあるため、
# print には flush=True を付け、呼び出し履歴を _log にも記録して Arrow 側から検証できるようにする。

_log = []


def on_message(msg: str) -> None:
    _log.append(msg)
    print("[py] on_message:", msg, flush=True)


def on_message_once(msg: str) -> None:
    _log.append("once:" + msg)
    print("[py] on_message_once:", msg, flush=True)


def get_log() -> list:
    return list(_log)
