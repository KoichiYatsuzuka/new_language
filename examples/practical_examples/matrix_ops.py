# matrix_ops.py — numpy matrix computation helper for matrix_progress.tl
#
# Two classes:
#   SharedState      — holds live display data and progress counters.
#                      All attributes are read by the render thread and written
#                      by the compute thread.  Python's GIL serialises every
#                      attribute read/write, so concurrent access is safe.
#                      numpy releases the GIL during BLAS, letting the render
#                      thread read state while heavy computation runs.
#
#   MatrixComputer   — owns the two N×N matrices and drives the iteration
#                      A = normalize(A @ B).  run_all() is the single method
#                      called from the tl async task; it loops until done,
#                      updating SharedState after every step.

import numpy as np


def _value_to_rgb(v: float) -> tuple:
    """Map a value in [0, 1] to an (r, g, b) int triple (cold→warm ramp)."""
    v = max(0.0, min(1.0, v))
    if v < 0.25:
        t = v * 4
        return int(20 + t * 20), int(20 + t * 80), int(180 - t * 40)
    if v < 0.5:
        t = (v - 0.25) * 4
        return int(40 - t * 30), int(100 + t * 120), int(140 - t * 100)
    if v < 0.75:
        t = (v - 0.5) * 4
        return int(10 + t * 220), int(220 - t * 50), int(40 - t * 30)
    t = (v - 0.75) * 4
    return int(230 + t * 20), int(170 - t * 140), int(10)


class SharedState:
    """Live display data shared between the compute thread and the render thread.

    All attribute reads/writes happen under Python's GIL, so they are
    automatically serialised — no extra locking is needed.

    Attributes
    ----------
    step       : int   current completed step (0 … total)
    total      : int   total number of steps
    done       : bool  True once step == total
    flat_vals  : list  row-major list of 64 float values (8×8 submatrix)
    flat_rgb   : list  parallel list of (r, g, b) int tuples for each cell
    """

    def __init__(self, total: int) -> None:
        self.step = 0
        self.total = int(total)
        self.done = False
        # Pre-fill with cold-blue placeholders so the grid looks sensible
        # even before the first computation step completes.
        self.flat_vals = [0.0] * 64
        self.flat_rgb  = [(20, 20, 180)] * 64

    def progress(self) -> int:
        """Return integer percentage 0-100."""
        if self.total == 0:
            return 100
        return min(100, (self.step * 100) // self.total)


class MatrixComputer:
    """Drives the iteration A = normalize(A @ B) on N×N float64 matrices.

    Parameters
    ----------
    n     : int          Matrix dimension.  2000 targets ~2-10 min of
                         computation; increase if it finishes too quickly.
    state : SharedState  Shared state object updated after every step.
    """

    def __init__(self, n: int, state: SharedState) -> None:
        rng = np.random.default_rng(42)
        self._A = rng.random((n, n), dtype=np.float64)
        self._B = rng.random((n, n), dtype=np.float64)
        self._n = n
        self._state = state

    def run_all(self) -> int:
        """Run all steps to completion, updating SharedState after each one.

        Called from the tl async task.  numpy releases the GIL during the
        BLAS matmul (the slow part), so the render thread can read SharedState
        attributes concurrently without stalling.
        """
        state = self._state
        while state.step < state.total:
            # GIL is released by numpy/BLAS during this line:
            self._A = self._A @ self._B
            m = float(self._A.max())
            if m > 0.0:
                self._A /= m
            state.step += 1          # GIL re-acquired for Python assignments
            self._push_display()
        state.done = True
        return state.step

    def _push_display(self) -> None:
        """Copy the top-left 8×8 block into SharedState for the render thread."""
        rows, cols = 8, 8
        r = min(rows, self._n)
        c = min(cols, self._n)
        sub = self._A[:r, :c]
        vals = []
        rgb  = []
        for i in range(r):
            for j in range(c):
                v = float(sub[i, j])
                vals.append(v)
                rgb.append(_value_to_rgb(v))
        self._state.flat_vals = vals
        self._state.flat_rgb  = rgb
