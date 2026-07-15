"""pygame renderer for Spider Solitaire (spider_solitaire.ar).

Requires: pip install pygame

Board string format (pipe-separated, up to 4 splits):
  score|foundation|stock_count|message|col_data

col_data: 10 columns separated by semicolons.
Each column: comma-separated card tokens.
Each token: 'Xn' = face-down rank n, 'n' = face-up rank n.

Mouse controls:
  Left-click a face-up card  → select it (and the sequence below it)
  Left-click another column  → execute the pending move
  Left-click STOCK button    → deal
  Right-click / Escape       → cancel selection
Keyboard: type a command and press Enter (m/d/h/q still work as before).
"""

import pygame

WIN_W = 1020
WIN_H = 780
CARD_W = 72
CARD_H = 96
COL_GAP = 98
MARGIN_X = 12
HEADER_H = 90
OVERLAP_DOWN = 16   # vertical gap between face-down cards
OVERLAP_UP   = 24   # vertical gap between face-up cards

# Stock / deal button (top-right corner)
STOCK_X = WIN_W - 115
STOCK_Y = 8
STOCK_W = 103
STOCK_H = 70

C_BG         = (  0,  90,  30)
C_FD         = ( 30,  30, 170)    # face-down fill
C_FU         = (255, 255, 255)    # face-up fill
C_SEL_FU     = (255, 255, 180)    # selected face-up fill
C_BORDER     = (  0,   0,   0)
C_SEL_BORDER = (255, 200,   0)    # selected card border
C_SUIT       = (  0,   0,   0)
C_HDR        = (255, 255, 180)
C_MSG        = (255, 240,  80)
C_SEL_MSG    = (100, 255, 100)
C_INPUT      = (200, 255, 200)
C_HINT       = (160, 200, 160)
C_EMPTY      = (  0,  60,  10)
C_FD_INNER   = ( 60,  60, 200)
C_STOCK_BG   = ( 30,  30, 120)

RANK_STR = ["", "A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K"]
COL_X = [MARGIN_X + i * COL_GAP for i in range(10)]

_screen      = None
_font_hdr    = None
_font_card   = None
_font_sm     = None
_clock       = None
_last_board  = ""
_last_parsed = None   # cached parse result for mouse hit-testing
_sel_col     = -1     # currently selected column (-1 = none)
_sel_row     = -1     # currently selected row


def _init():
    global _screen, _font_hdr, _font_card, _font_sm, _clock
    if _screen is not None:
        return
    pygame.init()
    _screen = pygame.display.set_mode((WIN_W, WIN_H))
    pygame.display.set_caption("Spider Solitaire")
    _font_hdr  = pygame.font.SysFont("consolas", 20, bold=True)
    _font_card = pygame.font.SysFont("consolas", 16, bold=True)
    _font_sm   = pygame.font.SysFont("consolas", 14)
    _clock = pygame.time.Clock()


def _parse(board_str):
    parts = board_str.split("|", 4)
    if len(parts) < 5:
        return None
    try:
        score      = int(parts[0])
        foundation = int(parts[1])
        stock      = int(parts[2])
    except ValueError:
        return None
    message = parts[3]
    columns = []
    for cs in parts[4].split(";"):
        col = []
        if cs:
            for tok in cs.split(","):
                if not tok:
                    continue
                try:
                    col.append((int(tok[1:]), False) if tok[0] == "X"
                               else (int(tok), True))
                except ValueError:
                    pass
        columns.append(col)
    return score, foundation, stock, message, columns


def _card_y_positions(col):
    """Return list of top-y for each card in a column."""
    ys = []
    y = HEADER_H
    for ri, (_, face_up) in enumerate(col):
        ys.append(y)
        if ri < len(col) - 1:
            y += OVERLAP_UP if face_up else OVERLAP_DOWN
    return ys


def _hit_test_col(ci, mx, my, columns):
    """Return row index under (mx, my) in column ci, or -1."""
    if mx < COL_X[ci] or mx > COL_X[ci] + CARD_W:
        return -1
    if my < HEADER_H:
        return -1
    col = columns[ci]
    if not col:
        return -1
    ys = _card_y_positions(col)
    # scan from the last card (visually on top) backward
    for ri in range(len(col) - 1, -1, -1):
        y_top    = ys[ri]
        y_bottom = y_top + CARD_H if ri == len(col) - 1 else ys[ri + 1]
        if y_top <= my <= y_bottom:
            return ri
    return -1


def _draw_card_sprite(x, y, rank, face_up, selected=False):
    if face_up:
        pygame.draw.rect(_screen, C_SEL_FU if selected else C_FU,
                         (x, y, CARD_W, CARD_H), border_radius=4)
        pygame.draw.rect(_screen, C_SEL_BORDER if selected else C_BORDER,
                         (x, y, CARD_W, CARD_H), 3 if selected else 2, border_radius=4)
        label = RANK_STR[rank] if 1 <= rank <= 13 else "?"
        _screen.blit(_font_card.render(label, True, C_SUIT), (x + 5, y + 4))
        _screen.blit(_font_sm.render("♠", True, C_SUIT), (x + 5, y + 22))
    else:
        pygame.draw.rect(_screen, C_FD,
                         (x, y, CARD_W, CARD_H), border_radius=4)
        pygame.draw.rect(_screen, C_BORDER,
                         (x, y, CARD_W, CARD_H), 2, border_radius=4)
        pygame.draw.rect(_screen, C_FD_INNER,
                         (x + 5, y + 5, CARD_W - 10, CARD_H - 10), 2, border_radius=2)


def _render(board_str):
    global _last_board, _last_parsed
    if board_str:
        _last_board = board_str
    parsed = _parse(_last_board) if _last_board else None
    _last_parsed = parsed

    _screen.fill(C_BG)
    if parsed is None:
        return

    score, foundation, stock_count, message, columns = parsed

    # Header
    hdr = f"Score: {score}    Completed: {foundation}/6"
    _screen.blit(_font_hdr.render(hdr, True, C_HDR), (10, 14))

    # Stock / deal button
    pygame.draw.rect(_screen, C_STOCK_BG,
                     (STOCK_X, STOCK_Y, STOCK_W, STOCK_H), border_radius=6)
    pygame.draw.rect(_screen, C_HDR,
                     (STOCK_X, STOCK_Y, STOCK_W, STOCK_H), 2, border_radius=6)
    _screen.blit(_font_sm.render("STOCK  [d]", True, C_HDR),
                 (STOCK_X + 8, STOCK_Y + 8))
    cnt_surf = _font_hdr.render(str(stock_count), True, C_HDR)
    _screen.blit(cnt_surf,
                 (STOCK_X + STOCK_W // 2 - cnt_surf.get_width() // 2, STOCK_Y + 34))

    # Column number labels
    for i in range(10):
        lbl = _font_sm.render(str(i + 1), True, C_HDR)
        _screen.blit(lbl, (COL_X[i] + CARD_W // 2 - 6, HEADER_H - 18))

    # Empty column outlines
    for i in range(10):
        pygame.draw.rect(_screen, C_EMPTY,
                         (COL_X[i], HEADER_H, CARD_W, CARD_H), 2, border_radius=4)

    # Card sprites
    for ci, col in enumerate(columns):
        y = HEADER_H
        for ri, (rank, face_up) in enumerate(col):
            sel = (_sel_col == ci and _sel_row >= 0 and ri >= _sel_row)
            _draw_card_sprite(COL_X[ci], y, rank, face_up, sel)
            if ri < len(col) - 1:
                y += OVERLAP_UP if face_up else OVERLAP_DOWN

    # Selection hint (shown only while a card is selected)
    if _sel_col >= 0 and _sel_row >= 0:
        s = f"Selected: col {_sel_col+1} row {_sel_row+1} — click destination  (right-click to cancel)"
        _screen.blit(_font_sm.render(s, True, C_SEL_MSG), (10, WIN_H - 82))

    # Message
    _screen.blit(_font_hdr.render(message, True, C_MSG), (10, WIN_H - 60))

    # Command hint
    hint = "m <col> <row> <to>  |  d  |  h  |  q       [mouse: click to select & move]"
    _screen.blit(_font_sm.render(hint, True, C_HINT), (10, WIN_H - 36))


def _handle_click(mx, my):
    """
    Process a left mouse click.
    Returns a command string to send to the game, or None if just updating selection.
    """
    global _sel_col, _sel_row, _last_parsed
    if _last_parsed is None:
        return None
    _, _, _, _, columns = _last_parsed

    # ── Stock button → deal ──────────────────────────────────────────────────
    if STOCK_X <= mx <= STOCK_X + STOCK_W and STOCK_Y <= my <= STOCK_Y + STOCK_H:
        _sel_col = _sel_row = -1
        return "d"

    # ── Identify clicked column (by x) and guard tableau y range ────────────
    clicked_col = -1
    for ci in range(10):
        if COL_X[ci] <= mx <= COL_X[ci] + CARD_W:
            clicked_col = ci
            break
    if clicked_col < 0 or my < HEADER_H:
        return None

    col = columns[clicked_col]

    if _sel_col < 0:
        # ── No selection: try to start one ───────────────────────────────────
        if not col:
            return None
        row = _hit_test_col(clicked_col, mx, my, columns)
        if row < 0 or row >= len(col):
            return None
        _, face_up = col[row]
        if not face_up:
            return None          # can't pick up a face-down card
        _sel_col = clicked_col
        _sel_row = row
        return None              # selection made, no command yet

    elif clicked_col == _sel_col:
        # ── Same column: change selection row or deselect ────────────────────
        row = _hit_test_col(clicked_col, mx, my, columns)
        if 0 <= row < len(col) and row != _sel_row:
            _, face_up = col[row]
            if face_up:
                _sel_row = row   # slide selection up/down
                return None
        _sel_col = _sel_row = -1
        return None

    else:
        # ── Different column: execute the pending move ───────────────────────
        fc, fr = _sel_col, _sel_row
        _sel_col = _sel_row = -1
        return f"m {fc+1} {fr+1} {clicked_col+1}"


def draw_game(board_str: str) -> int:
    """Render the full game board. Clears any pending selection. Returns 0."""
    global _sel_col, _sel_row
    _init()
    _sel_col = _sel_row = -1   # board changed → reset selection
    _render(board_str)
    pygame.display.flip()
    return 0


def get_event() -> str:
    """
    Block until a command is ready (mouse action or keyboard Enter).
    Redraws the board at 30 fps while waiting.
    Returns a command string, or 'q' on window close.
    """
    global _sel_col, _sel_row
    _init()
    buf = ""
    while True:
        _clock.tick(30)
        for evt in pygame.event.get():
            if evt.type == pygame.QUIT:
                return "q"

            elif evt.type == pygame.MOUSEBUTTONDOWN:
                if evt.button == 1:            # left click
                    result = _handle_click(*evt.pos)
                    if result:
                        return result
                elif evt.button == 3:          # right click → cancel selection
                    _sel_col = _sel_row = -1

            elif evt.type == pygame.KEYDOWN:
                if evt.key == pygame.K_RETURN:
                    cmd = buf.strip()
                    buf = ""
                    if cmd:
                        return cmd
                elif evt.key == pygame.K_BACKSPACE:
                    buf = buf[:-1]
                elif evt.key == pygame.K_ESCAPE:
                    if _sel_col >= 0:          # Escape cancels selection first
                        _sel_col = _sel_row = -1
                    else:
                        return "q"
                else:
                    ch = evt.unicode
                    if ch and ch.isprintable():
                        buf += ch

        # Redraw board + typing area
        _render("")
        pygame.draw.rect(_screen, (0, 40, 0), (0, WIN_H - 20, WIN_W, 20))
        _screen.blit(_font_sm.render("> " + buf + "_", True, C_INPUT), (10, WIN_H - 18))
        pygame.display.flip()


def close_window() -> int:
    """Quit pygame. Idempotent."""
    global _screen
    if _screen is not None:
        pygame.quit()
        _screen = None
    return 0
