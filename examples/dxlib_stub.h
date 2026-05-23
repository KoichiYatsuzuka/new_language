// dxlib_stub.h — Flat C declarations for the DxLib bridge.
//
// Used as the `with` header in:
//   import[cpp-lib] DXLIB with STUB as dx
//
// The tl cpp bridge parser reads this file to discover function signatures.
// The MSVC shim generator uses these signatures to emit extern "C" wrappers
// that call the real DxLib:: namespace functions.
//
// Notes:
//   - ClearDrawScreen normally takes `const RECT*`; exposed here with no args
//     (the shim passes NULL, which clears the whole screen).
//   - GetColor / DrawFillBox use unsigned int for color; mapped to int here
//     since tl treats all integers uniformly as i64 handles.
//   - GetNowCount normally takes `int UseRDTSCFlag = FALSE`; exposed with no
//     args (the shim uses the default = 0).

extern "C" {

int DxLib_Init(void);
int DxLib_End(void);
int ProcessMessage(void);
int ChangeWindowMode(int flag);
int SetGraphMode(int width, int height, int bitDepth, int fps);
int SetWaitVSyncFlag(int flag);
int ClearDrawScreen(void);
int ScreenFlip(void);
int WaitTimer(int msec);
int GetColor(int r, int g, int b);
int DrawFillBox(int x1, int y1, int x2, int y2, int color);
int GetNowCount(void);

/* string-parameter functions */
/* SetWindowTextDX is DxLib's Windows-macro-safe alias for SetWindowText */
int SetWindowTextDX(const char* text);
int DrawString(int x, int y, const char* str, int color);

}
