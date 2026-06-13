# life_game_ops.py — numpy/pandas helpers for life_game.ar
#
# Functions exposed to Arrow:
#   load_initial_grid(csv_path)  — read alive cells from CSV via pandas; return numpy grid
#   next_generation(grid)        — apply Conway's rules; return new numpy grid
#   get_alive_xy(grid)           — return [[x, y], ...] list of alive cell positions
#   count_alive(grid)            — return int count of alive cells
#
# Grid layout  : rows=ROWS (y-axis), cols=COLS (x-axis)
# Boundary rule: cells at the outer edge (row/col 0 and max) are always dead.

import numpy as np
import pandas as pd

ROWS = 480
COLS = 680


def load_initial_grid(csv_path: str):
    """Read (x, y) alive-cell positions from CSV and return a numpy uint8 grid.

    The CSV must have columns 'x' and 'y' (integer pixel coordinates).
    Positions outside the interior (boundary excluded) are silently ignored.
    """
    grid = np.zeros((ROWS, COLS), dtype=np.uint8)
    df = pd.read_csv(csv_path)
    for _, row in df.iterrows():
        x, y = int(row['x']), int(row['y'])
        if 1 <= x <= COLS - 2 and 1 <= y <= ROWS - 2:
            grid[y, x] = 1
    # Enforce dead boundary
    grid[0, :] = 0
    grid[-1, :] = 0
    grid[:, 0] = 0
    grid[:, -1] = 0
    return grid


def next_generation(grid):
    """Apply Conway's Game of Life rules and return the next-generation grid.

    Neighbour counting uses zero-padding so boundary cells have no living
    neighbours and cannot spawn new life — effectively keeping the boundary
    permanently dead without special-casing the rules.
    """
    padded = np.pad(grid, 1, mode='constant', constant_values=0)
    neighbors = (
        padded[:-2, :-2] + padded[:-2, 1:-1] + padded[:-2, 2:] +
        padded[1:-1, :-2] +                     padded[1:-1, 2:] +
        padded[2:,  :-2] + padded[2:,  1:-1] + padded[2:,  2:]
    )
    new_grid = np.zeros_like(grid)
    new_grid[(grid == 1) & ((neighbors == 2) | (neighbors == 3))] = 1
    new_grid[(grid == 0) & (neighbors == 3)] = 1
    # Guarantee dead boundary even if padding arithmetic produced spurious 1s
    new_grid[0, :] = 0
    new_grid[-1, :] = 0
    new_grid[:, 0] = 0
    new_grid[:, -1] = 0
    return new_grid


def get_alive_xy(grid) -> list:
    """Return a list of [x, y] integer pairs for every alive cell."""
    ys, xs = np.where(grid == 1)
    return [[int(x), int(y)] for x, y in zip(xs, ys)]


def count_alive(grid) -> int:
    """Return the total number of alive cells."""
    return int(np.sum(grid))
