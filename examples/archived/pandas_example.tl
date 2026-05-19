## pandas_example.tl — pandas の基本的な使い方

import[py-int] warnings as w
w.simplefilter("ignore")   # pandas CoW FutureWarning を抑制する

import[py-int] pandas as pd

# ── 1. DataFrame 作成 ──────────────────────────────────────────────
let df = pd.DataFrame({
    "name":  ["Alice", "Bob", "Charlie", "Diana", "Eve"],
    "age":   [25, 30, 35, 28, 22],
    "score": [85, 92, 78, 95, 88]
})

print("=== DataFrame ===")
print(df)

# ── 2. 基本情報 ────────────────────────────────────────────────────
print("\n=== shape ===")
print(df.shape)

print("\n=== 行数 ===")
print(len(df))

# ── 3. 列アクセス ──────────────────────────────────────────────────
print("\n=== score 列 ===")
let scores = df["score"]
print(scores)

# ── 4. 統計 ────────────────────────────────────────────────────────
print("\n=== 平均スコア ===")
let mean_score = scores.mean()
print(mean_score)

print("\n=== 最高スコア ===")
print(scores.max())

print("\n=== describe ===")
print(df.describe())

# ── 5. 条件フィルタリング ──────────────────────────────────────────
print("\n=== score > 88 の行 ===")
let mask = scores > 88
let high = df[mask]
print(high)

# ── 6. 新しい列を追加（df["score"] を直接使うことで CoW 警告を回避） ──
df["pass"] = df["score"] >= 85
print("\n=== pass 列追加後 ===")
print(df)

# ── 7. Series のイテレーション ────────────────────────────────────
print("\n=== スコア一覧 ===")
for v in scores:
    print(v)

# ── 8. ソートと head ──────────────────────────────────────────────
print("\n=== スコア上位 3 件 ===")
let sorted = df.sort_values("score", ascending=False)
print(sorted.head(3))
