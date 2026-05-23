# dxlib_string.tl — Verify that const char* parameters work correctly.
#
# Calls SetWindowText (sets the window title bar) and DrawString (renders
# text on screen) — both take a `const char*` that the bridge converts
# automatically from a tl `str` value via tl_to_cstr.

import[cpp-lib] DxLib.DxLib as dx

dx.ChangeWindowMode(1)
dx.SetGraphMode(640, 480, 32, 60)
dx.SetWaitVSyncFlag(0)

if dx.DxLib_Init() == -1:
    raise "DxLib_Init failed"

# Pass a tl string to a const char* parameter
dx.SetWindowTextDX("tl language - string test")

const FPS      = 60
const FRAME_MS = 1000 / FPS

let white  = dx.GetColor(255, 255, 255)
let yellow = dx.GetColor(255, 255,   0)
let cyan   = dx.GetColor(  0, 255, 255)

mut frame_count = 0
mut running     = 1

while running == 1:
    if dx.ProcessMessage() != 0:
        running = 0
        continue

    dx.ClearDrawScreen()

    # Draw several strings — each passes a tl str as const char*
    dx.DrawString(20,  40, "Hello from tl language!",    white)
    dx.DrawString(20,  70, "const char* works correctly", yellow)
    dx.DrawString(20, 100, "Automatic str -> C string",   cyan)

    dx.ScreenFlip()
    dx.WaitTimer(FRAME_MS)

    frame_count += 1
    if frame_count >= 300:   # run for 5 seconds at 60 fps
        running = 0

dx.DxLib_End()
