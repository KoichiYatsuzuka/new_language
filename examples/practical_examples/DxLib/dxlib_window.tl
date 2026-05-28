# dxlib_window.tl — 60fps DxLib window demo (requires DxLib_x64.lib + MSVC)
#
# import[cpp-lib] DxLib.DxLib_x64 with dxlib_stub as dx
#   - DxLib.DxLib_x64 → examples/DxLib/DxLib_x64.lib (relative to this file)
#   - dxlib_stub      → examples/dxlib_stub.h (flat C signatures for the bridge)
#
# At import time the interpreter will:
#   1. Find vcvarsall.bat from your Visual Studio installation.
#   2. Compile a thin C++ shim DLL (shim.cpp) that wraps DxLib:: calls.
#   3. Compile a Rust wrapper that loads the shim and bridges handle-based ABI.
#   4. Expose all functions as tl native functions under the alias `dx`.
#
# Required: Visual Studio 2019 or 2022 (Community or higher).

import[cpp-lib] DxLib.DxLib as dx

# ── Setup (must be before DxLib_Init) ────────────────────────────────────────
dx.ChangeWindowMode(1)          # 1=TRUE → windowed (not full-screen)
dx.SetGraphMode(640, 480, 32, 60)
dx.SetWaitVSyncFlag(0)          # 0=FALSE → manual frame-rate control

# ── Initialize ───────────────────────────────────────────────────────────────
if dx.DxLib_Init() == -1:
    raise RuntimeError("DxLib_Init failed")

const FPS      = 60
const FRAME_MS = 1000 / FPS     # 16 ms per frame

mut frame_count = 0
mut running     = 1

while running == 1:
    if dx.ProcessMessage() != 0:
        running = 0
        continue

    dx.ClearDrawScreen()

    let offset = frame_count % 200
    let col    = dx.GetColor(255, 128, 0)
    dx.DrawFillBox(50 + offset, 100, 350 + offset, 380, col)

    dx.ScreenFlip()
    dx.WaitTimer(FRAME_MS)

    frame_count += 1

    if frame_count >= 600:  # run for 10 seconds at 60 fps
        running = 0

# ── Cleanup ───────────────────────────────────────────────────────────────────
dx.DxLib_End()
