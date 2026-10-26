# speed_modes — 3 つの実行モードの速度の調査のベンチ

`native_object_access_plan.md` の速度の調査（2026-10-25）で使ったベンチ（7-1 で取り込んだ）。同じ計算を

| モード | 何を走らせるか |
|---|---|
| `interp` | `sim_core.ar` をインタープリタ（VM）で |
| `native` | 同じ `sim_core.ar` を `--compile` した `.arc`（コンパイル済み） |
| `cpp` | `cpp/sim_core.ar`。数値の計算を C++ のカーネル（`cpp/sim_kernels.cpp`・`import[cpp-dll]`）で |

で走らせ、区間ごとの時間と、区間の結果（`CHECK`）がモード間で一致するかを出す。

## 走らせ方

```powershell
./scripts/speed_modes.ps1                               # target/release の exe で 3 モード
./scripts/speed_modes.ps1 -Exe <a.exe> -B <b.exe>       # 2 つの exe をモードごとに A/B（各 exe が自分で .arc を作る）
./scripts/speed_modes.ps1 -Exe <a.exe> -B <a.exe>       # 同じ exe 同士（ノイズの幅の対照）
./scripts/speed_modes.ps1 -Micro                        # 1 操作の単価（ns）
```

例題はリポジトリの外（`%TEMP%\arrow_speed_modes`）へ写して走らせる。C++ のモードは `ar_config.json` の
`cpp.msvc` で `sim_kernels.dll` を作る（作れなければそのモードを飛ばして理由を出す）。

## ファイル

| ファイル | 内容 |
|---|---|
| `bench.ar` | 区間を測る本体（全モード共通）。`AR_BENCH_SCALE` で回数を倍にし、`AR_BENCH_ONLY=k2_energy` などで区間を絞る |
| `sim_core.ar` | `interp` / `native` の計算（`Vec3` / `Body` / `World` と `run_*`） |
| `micro.ar` / `micro_core.ar` | 1 操作の単価（`(t(操作のループ) - t(空のループ)) / n`）。`interp` / `native` |
| `cpp/sim_core.ar` | `cpp` の計算（同じクラス。数値の計算だけ C++） |
| `cpp/sim_kernels.cpp` / `.h` / `.def` | C++ のカーネル（演算の順は `sim_core.ar` と同じなので結果がビット単位で一致する） |
| `cpp/micro_ffi.ar` / `cpp/micro_ffi_bench.ar` | C++ の呼び出しの単価 |

区間: `k1_step`（数値のメソッド）・`k2_energy`（`list[Body]` の反復と `let self` のメソッドの連鎖）・`k3_record`
（`list[float]` への追加と集計）・`k4_labels`（文字列の連結・`int` → `str`）・`k5_spawn`（生成）・`k6_boundary`
（インタープリタからコンパイル済みのメソッドを呼ぶ境界）・`process_wall`（起動を含むプロセス全体）。

⚠ この階層は `scan_examples` などの例題ゲートの対象外（各カテゴリの直下だけを見る）。`cpp/` は DLL を作らないと
走らない。
