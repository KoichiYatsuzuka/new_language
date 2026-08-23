# cfg_probe.py — ar_config.json の python.search_paths でしか到達できない位置に置いた
# Python モジュール（#61/#69 の検査用）。
#
# このファイルは examples/interop/ の**直下ではなく py_libs/ の中**にある。
# 既定の検索パス（.ar のあるディレクトリ）だけでは見つからず、
# examples/interop/ar_config.json の "python": {"search_paths": ["py_libs"]} を
# 読めて初めて解決できる。⇒ 設定の読み取りが壊れるとこの例題が落ちる。
#
# ⚠ 埋め込み Python の print は flush されないと消えるので、ここでは print せず値を返す。

MARKER = "search_path_ok"


def probe(n):
    return n * 3 + 1


def describe():
    return "cfg_probe from py_libs"
