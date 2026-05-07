# ============================================================
# exceptions__errors.tl — 例外処理の意図的なエラー例
# 各ブロックは 1 つのエラーを示す。実行すると最初のエラーで停止する。
# ============================================================

# --- 期待エラー: 未キャッチの例外がトップレベルに到達 ---
# 出力例:
#   Traceback (most recent call last):
#     File "examples/exceptions__errors.tl", line N, col M, in <module>
#       raise ValueError("uncaught!")
#   ValueError: uncaught!

raise ValueError("uncaught!")
