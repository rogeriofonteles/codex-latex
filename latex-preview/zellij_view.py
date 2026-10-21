"""Use Zellij's displayed viewport, including pane scrollback and visibility."""

import json
import os
import subprocess
import time
import threading
from dataclasses import dataclass
import re
from wcwidth import wcswidth


def layout(panes, tabs, pane_id):
    pane = next(p for p in panes if p["id"] == pane_id and not p["is_plugin"])
    tab = next(t for t in tabs if t["tab_id"] == pane["tab_id"])
    visible = tab["active"] and not pane["is_suppressed"]
    if tab["is_fullscreen_active"] and not pane["is_fullscreen"]:
        visible = False
    # Hide under floating panes rather than paint over their contents.
    if tab["are_floating_panes_visible"]:
        visible = visible and pane["is_floating"] and pane["is_focused"]
    elif pane["is_floating"]:
        visible = False
    return pane, tab, visible


@dataclass(frozen=True)
class Viewport:
    columns: int
    lines: int
    markers: tuple


def viewport(text, columns, rows):
    # Parse only marker locations. Rebuilding every character of a pyte screen
    # on each poll was expensive, especially for large split panes.
    markers = []
    for y, line in enumerate(text.splitlines()[:rows]):
        for match in re.finditer(r"\[m:([0-9a-f]{12}):(\d{2})\]", line):
            x = wcswidth(line[: match.start()])
            if 0 <= x <= columns - 19:
                markers.append((match[1], int(match[2]), x, y))
    return Viewport(columns, rows, tuple(markers))


class ZellijView:
    def __init__(self):
        self.pane_id = int(os.environ["ZELLIJ_PANE_ID"])
        self.command = ["zellij", "-s", os.environ["ZELLIJ_SESSION_NAME"], "action"]
        self.screen = None
        self.state = None
        self.applied = None
        self.worker = None
        self.metadata_worker = None
        self.stopped = threading.Event()
        self.last_metadata = 0
        self.metadata = None

    def action(self, *args):
        # Pane queries can take over a second in sessions with many panes.
        # These run on the polling worker after startup, not the input thread.
        return subprocess.check_output(
            self.command + list(args), text=True, stderr=subprocess.DEVNULL, timeout=3
        )

    def read_metadata(self):
        panes = json.loads(self.action("list-panes", "--json", "--all"))
        tabs = json.loads(self.action("list-tabs", "--json", "--all"))
        self.metadata = layout(panes, tabs, self.pane_id)
        self.last_metadata = time.monotonic()

    def poll_metadata(self):
        while not self.stopped.wait(0.5):
            try:
                self.read_metadata()
            except (
                OSError,
                subprocess.SubprocessError,
                ValueError,
                KeyError,
                StopIteration,
            ):
                # The viewport worker expires stale metadata without blocking input.
                pass

    def read_state(self):
        if self.metadata is None:
            self.read_metadata()
        pane, tab, visible = self.metadata
        visible = visible and time.monotonic() - self.last_metadata < 4
        text = (
            self.action("dump-screen", "--pane-id", str(self.pane_id))
            if visible
            else ""
        )
        self.state = (
            pane,
            tab,
            visible,
            viewport(text, pane["pane_content_columns"], pane["pane_content_rows"]),
        )

    def poll(self):
        while not self.stopped.is_set():
            try:
                self.read_state()
                if self.metadata_worker is None:
                    self.metadata_worker = threading.Thread(
                        target=self.poll_metadata, daemon=True
                    )
                    self.metadata_worker.start()
            except (
                OSError,
                subprocess.SubprocessError,
                ValueError,
                KeyError,
                StopIteration,
            ):
                self.state = None
            self.stopped.wait(0.1)

    def refresh(self, overlay, force=False, active=True, pane_size=None):
        if force:
            try:
                self.read_state()
            except (
                OSError,
                subprocess.SubprocessError,
                ValueError,
                KeyError,
                StopIteration,
            ):
                raise RuntimeError(
                    "Cannot read this Zellij pane; JSON pane and tab queries are required"
                ) from None
        elif active and self.worker is None:
            # Start only after a math marker appears, and only after the PTY fork.
            self.worker = threading.Thread(target=self.poll, daemon=True)
            self.worker.start()
        state = self.state
        if state is not None and pane_size is not None:
            pane = state[0]
            if pane_size != (pane["pane_content_columns"], pane["pane_content_rows"]):
                overlay.visible = False
                self.applied = None
                return self.screen
        if state is self.applied:
            return self.screen
        self.applied = state
        if state is None:
            overlay.visible = False
            return self.screen
        pane, tab, visible, self.screen = state
        overlay.fg = pane.get("default_fg") or overlay.terminal_fg
        overlay.bg = pane.get("default_bg") or overlay.terminal_bg
        overlay.geometry(tab["display_area_columns"], tab["display_area_rows"])
        overlay.offset_x = pane["pane_content_x"] * overlay.cw
        overlay.offset_y = pane["pane_content_y"] * overlay.ch
        overlay.visible = visible
        return self.screen

    def close(self):
        self.stopped.set()
        if self.worker:
            self.worker.join(timeout=1.5)
        if self.metadata_worker:
            self.metadata_worker.join(timeout=1.5)
