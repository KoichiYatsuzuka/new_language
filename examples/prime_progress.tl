# prime_progress.tl — count primes up to 500,000 while showing a DxLib progress bar.
#
# The calculation runs in small chunks (CHUNK outer iterations per frame) inside
# the render loop so the DxLib window stays responsive throughout.  Progress is
# shown as a colour-filled bar plus live statistics.
#
# Expected run time in the tree-walk interpreter: roughly 2–10 minutes depending
# on hardware — well over the 1-minute requirement.
#
# Requirements: Visual Studio 2019 / 2022 (Community or higher) with MSVC.

import[cpp-lib] DxLib.DxLib as dx

# ── Window setup (must precede DxLib_Init) ────────────────────────────────────
dx.ChangeWindowMode(1)
dx.SetGraphMode(800, 320, 32, 60)
dx.SetWaitVSyncFlag(0)

if dx.DxLib_Init() == -1:
    raise RuntimeError("DxLib_Init failed")

dx.SetWindowTextDX("Long Calculation - Prime Counter")

# ── Calculation parameters ────────────────────────────────────────────────────
const N     = 500000   # count primes in [2, N]  — takes several minutes interpreted
const CHUNK = 100      # candidate numbers tested per render frame

# ── Mutable calculation state ─────────────────────────────────────────────────
mut candidate   = 2     # next number to trial-divide
mut prime_count = 0     # primes found so far
mut calc_done   = False # True once candidate has passed N

# ── DxLib colour handles ──────────────────────────────────────────────────────
let white  = dx.GetColor(255, 255, 255)
let gray   = dx.GetColor( 60,  60,  60)
let green  = dx.GetColor(  0, 200, 100)
let yellow = dx.GetColor(255, 220,   0)
let blue   = dx.GetColor( 80, 160, 255)
let red    = dx.GetColor(220,  60,  60)

# ── Layout constants ──────────────────────────────────────────────────────────
const BAR_X = 50
const BAR_Y = 140
const BAR_W = 700
const BAR_H = 40

# ── Main render loop ──────────────────────────────────────────────────────────
mut running = True

while running:
    if dx.ProcessMessage() != 0:
        running = False
        continue

    # ── Computation chunk ─────────────────────────────────────────────────────
    # Process CHUNK candidate numbers per frame via trial division.
    if not calc_done:
        mut done_chunk = 0
        while done_chunk < CHUNK and candidate <= N:
            mut is_prime = True
            mut j = 2
            while j * j <= candidate:
                if candidate % j == 0:
                    is_prime = False
                    break
                j += 1
            if is_prime:
                prime_count += 1
            candidate += 1
            done_chunk += 1
        if candidate > N:
            calc_done = True

    # ── Current progress (0–100) ──────────────────────────────────────────────
    mut progress = 0
    if calc_done:
        progress = 100
    else:
        progress = (candidate * 100) / N

    # ── Drawing ───────────────────────────────────────────────────────────────
    dx.ClearDrawScreen()

    # Title and subtitle
    dx.DrawString(BAR_X, 30, "Long Calculation: Counting Primes up to 500,000", white)
    dx.DrawString(BAR_X, 58, "Trial-division loop running in the interpreter (2-10 min)", blue)

    # Divider line above bar
    dx.DrawFillBox(BAR_X, BAR_Y - 12, BAR_X + BAR_W, BAR_Y - 10, gray)

    # Progress bar — background
    dx.DrawFillBox(BAR_X, BAR_Y, BAR_X + BAR_W, BAR_Y + BAR_H, gray)

    # Progress bar — fill (green while running, yellow when done)
    let fill_w = (progress * BAR_W) / 100
    if fill_w > 0:
        mut bar_col = green
        if calc_done:
            bar_col = yellow
        dx.DrawFillBox(BAR_X, BAR_Y, BAR_X + fill_w, BAR_Y + BAR_H, bar_col)

    # Stats below the bar
    dx.DrawString(BAR_X, BAR_Y + BAR_H + 10, str(progress) + "%", white)
    dx.DrawString(BAR_X + 100, BAR_Y + BAR_H + 10, "Checking: " + str(candidate) + " / " + str(N), blue)
    dx.DrawString(BAR_X, BAR_Y + BAR_H + 36, "Primes found so far: " + str(prime_count), white)

    # Status line at the bottom
    if calc_done:
        dx.DrawString(BAR_X, 276, "Done!  Found " + str(prime_count) + " primes up to " + str(N) + ".  Close window to exit.", yellow)
    else:
        dx.DrawString(BAR_X, 276, "Calculating... please wait.  (Close window to abort.)", red)

    dx.ScreenFlip()
    dx.WaitTimer(16)

# ── Cleanup ───────────────────────────────────────────────────────────────────
dx.DxLib_End()
