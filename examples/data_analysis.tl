# data_analysis.tl — monthly sales analysis with pandas, numpy, and matplotlib.
#
# Input : examples/data/sales_data.csv
# Output: examples/data/sales_analysis.png
#
# Run:
#   cargo run -- examples/data_analysis.tl

import[py-int] warnings as _w
_w.simplefilter("ignore")

# Set non-interactive backend before importing pyplot (avoids display window).
import[py-int] matplotlib as mpl
mpl.use("Agg")

import[py-int] pandas as pd
import[py-int] numpy as np
import[py-int] matplotlib.pyplot as plt

# ── 1. Load ───────────────────────────────────────────────────────────────────

let df = pd.read_csv("examples/data/sales_data.csv")

print("=== Raw Data ===")
print(df)
print()

# ── 2. Data processing ────────────────────────────────────────────────────────

# Derived columns
df["total"]    = df["product_a"] + df["product_b"] + df["product_c"]
df["profit"]   = df["total"] - df["marketing_spend"]
df["roi"]      = df["profit"] / df["marketing_spend"] * 100

# 3-month rolling average on total sales
df["total_ma3"] = df["total"].rolling(window=3).mean()

print("=== Processed Data ===")
print(df[["month", "product_a", "product_b", "product_c", "total", "profit", "roi", "total_ma3"]])
print()

# ── 3. numpy statistics ───────────────────────────────────────────────────────

let a_arr   = np.array(df["product_a"].tolist())
let b_arr   = np.array(df["product_b"].tolist())
let c_arr   = np.array(df["product_c"].tolist())
let tot_arr = np.array(df["total"].tolist())
let mkt_arr = np.array(df["marketing_spend"].tolist())

print("=== Statistics (numpy) ===")
print("Product A     mean:", np.mean(a_arr),   " std:", np.std(a_arr))
print("Product B     mean:", np.mean(b_arr),   " std:", np.std(b_arr))
print("Product C     mean:", np.mean(c_arr),   " std:", np.std(c_arr))
print("Total         mean:", np.mean(tot_arr), " max:", np.max(tot_arr), " min:", np.min(tot_arr))
print("Marketing     mean:", np.mean(mkt_arr))
print()
print("Correlation A-B :", np.corrcoef(a_arr, b_arr)[0][1])
print("Correlation A-C :", np.corrcoef(a_arr, c_arr)[0][1])
print("Correlation total-marketing:", np.corrcoef(tot_arr, mkt_arr)[0][1])
print()

let best_month_idx = np.argmax(tot_arr)
let worst_month_idx = np.argmin(tot_arr)
print("Best  month index:", best_month_idx, " (", df["month"].tolist()[best_month_idx], ")")
print("Worst month index:", worst_month_idx, " (", df["month"].tolist()[worst_month_idx], ")")
print()

# ── 4. Visualization ──────────────────────────────────────────────────────────

let months = df["month"].tolist()
let x      = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]

plt.figure(figsize=(14, 10))
plt.suptitle("Monthly Sales Analysis 2024", fontsize=16, fontweight="bold")

# --- Plot 1: Line chart — sales per product ---
plt.subplot(2, 2, 1)
plt.plot(df["product_a"].tolist(), marker="o",  label="Product A", color="royalblue")
plt.plot(df["product_b"].tolist(), marker="s",  label="Product B", color="darkorange")
plt.plot(df["product_c"].tolist(), marker="^",  label="Product C", color="seagreen")
plt.xticks(x, months, rotation=45, ha="right")
plt.title("Monthly Sales by Product")
plt.xlabel("Month")
plt.ylabel("Sales")
plt.legend()
plt.grid(True, alpha=0.3)

# --- Plot 2: Grouped bar — total vs profit ---
plt.subplot(2, 2, 2)
plt.bar(x, df["total"].tolist(),  label="Total Sales", color="steelblue", alpha=0.85)
plt.bar(x, df["profit"].tolist(), label="Profit",      color="lightgreen", alpha=0.9)
plt.xticks(x, months, rotation=45, ha="right")
plt.title("Total Sales and Profit per Month")
plt.xlabel("Month")
plt.ylabel("Amount")
plt.legend()
plt.grid(True, axis="y", alpha=0.3)

# --- Plot 3: Pie chart — average sales share per product ---
plt.subplot(2, 2, 3)
let avg_a = np.mean(a_arr)
let avg_b = np.mean(b_arr)
let avg_c = np.mean(c_arr)
plt.pie(
    [avg_a, avg_b, avg_c],
    labels=["Product A", "Product B", "Product C"],
    autopct="%1.1f%%",
    startangle=90,
    colors=["royalblue", "darkorange", "seagreen"]
)
plt.title("Average Sales Share by Product")

# --- Plot 4: Total sales + 3-month moving average ---
plt.subplot(2, 2, 4)
plt.plot(df["total"].tolist(),    marker="o", label="Total Sales",  color="steelblue", alpha=0.6)
plt.plot(df["total_ma3"].tolist(), linewidth=2, label="3-Month MA", color="crimson")
plt.xticks(x, months, rotation=45, ha="right")
plt.title("Total Sales with 3-Month Moving Average")
plt.xlabel("Month")
plt.ylabel("Sales")
plt.legend()
plt.grid(True, alpha=0.3)

plt.tight_layout()

let out_path = "examples/data/sales_analysis.png"
plt.savefig(out_path, dpi=100, bbox_inches="tight")
plt.close()
print("Graph saved to:", out_path)
