# matrix_progress.tl — numpy matrix computation visualised with DxLib (async version).
#
# The computation runs in a background async thread via AsyncManager.
# The render loop runs at 60 fps and reads display state that the compute
# thread writes after every step — safe because Python's GIL serialises all
# attribute access, while numpy releases the GIL during BLAS so both threads
# make progress simultaneously.
#
# Architecture
# ────────────
#   SharedState (PyObject, Arc-shared)  ← written by compute thread
#       .step / .total / .done              read by render thread
#       .flat_vals / .flat_rgb
#
#   MatrixComputer.run_all()  ← called inside the async task body
#       loops: A = normalize(A @ B), then _push_display() updates SharedState
#
# Expected run time: 2-10 minutes (2000×2000 matmul, 200 steps).
# Adjust MatrixComputer(n=...) or SharedState(total=...) to change the load.
#
# Requirements:
#   - numpy in the active Python environment
#   - Visual Studio 2019 / 2022 for the DxLib cpp bridge

import[cpp-lib] DxLib.DxLib as dx
import[py-int] matrix_ops as mo

# ── Window (must precede DxLib_Init) ─────────────────────────────────────────
dx.ChangeWindowMode(1)
dx.SetGraphMode(760, 570, 32, 60)
dx.SetWaitVSyncFlag(0)

if dx.DxLib_Init() == -1:
    raise RuntimeError("DxLib_Init failed")

dx.SetWindowTextDX("Matrix Computation (async) - A = A x B  [2000x2000]")

# ── Shared state + matrix computer ────────────────────────────────────────────
# Both are PyObjects (Arc-wrapped).  deep_clone() for Arc = Arc::clone(), so
# the async task shares the exact same Python objects — no copying occurs.
let state = mo.SharedState(2000)
let mgr   = mo.MatrixComputer(2000, state)

# ── Layout ────────────────────────────────────────────────────────────────────
const DISPLAY_ROWS = 8
const DISPLAY_COLS = 8
const CELL_W = 85
const CELL_H = 44
const GRID_X = 42
const GRID_Y = 90
const BAR_X  = 30
const BAR_Y  = 478
const BAR_W  = 700
const BAR_H  = 28

# ── DxLib colour handles ──────────────────────────────────────────────────────
let white  = dx.GetColor(255, 255, 255)
let lgray  = dx.GetColor(160, 160, 160)
let dgray  = dx.GetColor( 55,  55,  55)
let green  = dx.GetColor(  0, 200,  80)
let yellow = dx.GetColor(255, 220,   0)
let blue   = dx.GetColor( 80, 160, 255)
let red    = dx.GetColor(220,  60,  60)

# ── Launch async computation ──────────────────────────────────────────────────
# mgr is a let PyObject; deep_clone() = Arc::clone() → same Python object.
# mgr.run_all() drives the iteration and updates state after every matmul step.
mut mng = AsyncManager(num_thread=1, raise_immediately=False)

mng <- async->int:
    block_return mgr.run_all()

# ── Render loop ───────────────────────────────────────────────────────────────
mut running = True

while running:
    if dx.ProcessMessage() != 0:
        running = False
        continue

    # Read shared state (GIL-protected; numpy releases GIL during BLAS so the
    # render thread can often read without waiting for the compute thread).
    let cur_step  = state.step
    let total_s   = state.total
    let is_done   = state.done
    let pct       = state.progress()
    let flat_vals = state.flat_vals   # Python list → tl list[float]
    let flat_rgb  = state.flat_rgb    # Python list of tuples → tl list[tuple]

    dx.ClearDrawScreen()

    # Header
    dx.DrawString(GRID_X, 10, "Matrix iteration (async): A = normalize( A x B )  [2000x2000]", white)
    dx.DrawString(GRID_X, 32, "Top-left 8x8 submatrix — colour map: cold (blue) to warm (red)", blue)
    dx.DrawString(GRID_X, 54, "Step " + str(cur_step) + " / " + str(total_s) + "   (" + str(pct) + "%)", lgray)

    # Column headers  C0 … C7
    mut hdr = 0
    while hdr < DISPLAY_COLS:
        dx.DrawString(GRID_X + hdr * CELL_W + 28, 75, "C" + str(hdr), dgray)
        hdr += 1

    # 8×8 matrix grid
    mut idx = 0
    mut ri  = 0
    while ri < DISPLAY_ROWS:
        dx.DrawString(GRID_X - 30, GRID_Y + ri * CELL_H + 14, "R" + str(ri), dgray)
        mut ci = 0
        while ci < DISPLAY_COLS:
            let cell_x = GRID_X + ci * CELL_W
            let cell_y = GRID_Y + ri * CELL_H

            # Coloured background
            let rgb        = flat_rgb[idx]
            let cell_color = dx.GetColor(rgb[0], rgb[1], rgb[2])
            dx.DrawFillBox(cell_x, cell_y, cell_x + CELL_W - 2, cell_y + CELL_H - 2, cell_color)

            # Numeric value (first 6 chars of float string, e.g. "0.4321")
            let v = flat_vals[idx]
            dx.DrawString(cell_x + 4, cell_y + 14, str(v)[0:6], white)

            idx += 1
            ci  += 1
        ri += 1

    # Progress bar
    dx.DrawFillBox(BAR_X, BAR_Y, BAR_X + BAR_W, BAR_Y + BAR_H, dgray)
    let fill_w = (pct * BAR_W) / 100
    if fill_w > 0:
        mut bar_col = green
        if is_done:
            bar_col = yellow
        dx.DrawFillBox(BAR_X, BAR_Y, BAR_X + fill_w, BAR_Y + BAR_H, bar_col)

    if is_done:
        dx.DrawString(BAR_X, BAR_Y + BAR_H + 12, "Done!  Completed " + str(total_s) + " steps.  Close window to exit.", yellow)
    else:
        dx.DrawString(BAR_X, BAR_Y + BAR_H + 12, "Computing in async thread...  Close window to abort.", red)

    dx.ScreenFlip()
    #dx.WaitTimer(16)   # 60 fps target; compute thread runs concurrently

# ── Cleanup ───────────────────────────────────────────────────────────────────
dx.DxLib_End()
mng.wait_for_finish()   # block at most one remaining matmul step (~0.3-2 s)
