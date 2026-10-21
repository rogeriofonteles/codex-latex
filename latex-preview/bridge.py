#!/usr/bin/env python3
"""Mirror terminal output in memory and attach click-through equation strips to Alacritty."""

import copy
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import sys
import tempfile
import termios
import tty
import subprocess
import time

import pyte
from PIL import Image
from Xlib import X, display
from Xlib.ext import shape
from zellij_view import ZellijView
from terminal_colors import query as query_colors

MARKER = re.compile(r"\[m:([0-9a-f]{12}):(\d{2})\]")
HERE = Path(__file__).resolve().parent


class Screen(pyte.Screen):
    """Track alternate-screen transitions as well as normal terminal scrolling."""

    saved = None

    def set_mode(self, *modes, **kwargs):
        if kwargs.get("private") and any(m in modes for m in (1049, 1047, 47)):
            if self.saved is None:
                self.saved = copy.deepcopy((self.buffer, self.cursor, self.margins))
                self.erase_in_display(2)
                self.cursor_position()
        super().set_mode(*modes, **kwargs)

    def reset_mode(self, *modes, **kwargs):
        if kwargs.get("private") and any(m in modes for m in (1049, 1047, 47)):
            if self.saved is not None:
                self.buffer, self.cursor, self.margins = self.saved
                self.saved = None
                self.dirty.update(range(self.lines))
        super().reset_mode(*modes, **kwargs)


def positions(screen):
    found = []
    for y in range(screen.lines):
        row = screen.buffer[y]
        for x in range(screen.columns - 18):
            if row[x].data == "[":
                text = "".join(row[col].data for col in range(x, x + 19))
                match = MARKER.fullmatch(text)
                if match:
                    found.append((match[1], int(match[2]), x, y))
    return found


def x11_window_id(value):
    try:
        window_id = int(value, 0)
    except (TypeError, ValueError):
        window_id = 0
    if not 0 < window_id <= 0xFFFFFFFF:
        raise RuntimeError(
            "WINDOWID is not a valid X11 window ID; use codex-latex --window "
            "to open an X11 preview window"
        )
    return window_id


class Overlay:
    def __init__(self, directory):
        window_id = x11_window_id(os.environ.get("WINDOWID"))
        self.connection = display.Display()
        if not self.connection.has_extension("SHAPE"):
            raise RuntimeError("X11 SHAPE extension is required")
        self.parent = self.connection.create_resource_object("window", window_id)
        self.directory = directory
        self.windows = {}
        self.images = {}
        self.cw = self.ch = 0
        self.offset_x = self.offset_y = 0
        self.visible = True
        self.terminal_fg = self.fg = "#d8d8d8"
        self.terminal_bg = self.bg = "#000000"

    def geometry(self, columns, rows):
        g = self.parent.get_geometry()
        self.cw, self.ch = g.width // columns, g.height // rows
        if g.depth not in (24, 32):
            raise RuntimeError("A 24-bit or 32-bit X11 display is required")
        self.depth = g.depth
        geometry = json.dumps({"cw": self.cw, "ch": self.ch})
        if geometry != getattr(self, "last_geometry", None):
            temporary = self.directory / "geometry.tmp"
            temporary.write_text(geometry)
            temporary.replace(self.directory / "geometry.json")
            self.last_geometry = geometry

    def update(self, screen):
        markers = []
        if self.visible and screen:
            markers = (
                screen.markers if hasattr(screen, "markers") else positions(screen)
            )
        signature = (
            tuple(markers),
            getattr(screen, "columns", 0),
            self.cw,
            self.ch,
            self.offset_x,
            self.offset_y,
            self.fg,
            self.bg,
        )
        if signature == getattr(self, "last_frame", None):
            return
        self.last_frame = signature
        desired = set()
        for identifier, index, x, y in markers:
            key = (
                identifier,
                index,
                x,
                y,
                self.cw,
                self.ch,
                screen.columns,
                self.offset_x,
                self.offset_y,
                self.fg,
                self.bg,
            )
            desired.add(key)
            if key in self.windows:
                continue
            try:
                if identifier not in self.images:
                    image = Image.open(self.directory / (identifier + ".png")).convert(
                        "RGBA"
                    )
                    metadata = json.loads(
                        (self.directory / (identifier + ".json")).read_text()
                    )
                    if len(self.images) >= 128:
                        self.images.clear()
                    self.images[identifier] = (image, metadata)
                image, metadata = self.images[identifier]
                if index >= metadata["rows"]:
                    continue
                source_ch = image.height // metadata["rows"]
                width = min(image.width, (screen.columns - x) * self.cw)
                strip = image.crop(
                    (0, index * source_ch, width, (index + 1) * source_ch)
                )
                if source_ch != self.ch:
                    strip = strip.resize((width, self.ch))
                ink = Image.new("RGB", strip.size, self.fg)
                background = Image.new("RGB", strip.size, self.bg)
                strip = Image.composite(ink, background, strip.getchannel("A"))
                pixmap = self.parent.create_pixmap(width, self.ch, self.depth)
                gc = pixmap.create_gc()
                # A strip fits inside X11's usual 256 KiB request limit.
                pixmap.put_image(
                    gc,
                    0,
                    0,
                    width,
                    self.ch,
                    X.ZPixmap,
                    self.depth,
                    0,
                    strip.convert("RGBA").tobytes("raw", "BGRA"),
                )
                window = self.parent.create_window(
                    self.offset_x + x * self.cw,
                    self.offset_y + y * self.ch,
                    width,
                    self.ch,
                    0,
                    X.CopyFromParent,
                    X.InputOutput,
                    X.CopyFromParent,
                    background_pixmap=pixmap,
                    override_redirect=True,
                )
                window.shape_rectangles(
                    shape.SO.Set, shape.SK.Input, X.Unsorted, 0, 0, []
                )
                window.map()
                self.windows[key] = (window, pixmap, gc)
            except (OSError, ValueError, KeyError):
                self.last_frame = None
                continue
        for key in self.windows.keys() - desired:
            window, pixmap, gc = self.windows.pop(key)
            window.destroy()
            pixmap.free()
            gc.free()
        self.connection.flush()

    def close(self):
        for window, pixmap, gc in self.windows.values():
            window.destroy()
            pixmap.free()
            gc.free()
        self.connection.close()


def write_all(fd, data):
    while data:
        data = data[os.write(fd, data) :]


def run(command):
    if not os.environ.get("WINDOWID") or not os.environ.get("DISPLAY"):
        raise RuntimeError("Use the codex-latex launcher in an X11 desktop")
    original = termios.tcgetattr(0)
    colors, pending_input = query_colors()
    size = fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 8)
    rows, columns, _, _ = struct.unpack("HHHH", size)
    with tempfile.TemporaryDirectory(prefix="codex-latex-") as directory:
        directory = Path(directory)
        overlay = Overlay(directory)
        overlay.terminal_fg = overlay.fg = colors.get("fg", "#d8d8d8")
        overlay.terminal_bg = overlay.bg = colors.get("bg", "#000000")
        view = ZellijView() if os.environ.get("CODEX_LATEX_ZELLIJ") else None
        if view:
            try:
                view.refresh(overlay, force=True)
            except Exception:
                overlay.close()
                raise
        else:
            overlay.geometry(columns, rows)
        renderer = subprocess.Popen(
            ["node", str(HERE / "render.cjs"), "--serve"],
            env=dict(os.environ, CODEX_LATEX_CACHE=str(directory)),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.monotonic() + 3
        while not (directory / "renderer.sock").exists():
            if renderer.poll() is not None or time.monotonic() >= deadline:
                renderer.terminate()
                renderer.wait()
                overlay.close()
                raise RuntimeError(
                    "Formula worker failed to start; use codex-latex --plain resume"
                )
            time.sleep(0.01)
        screen = Screen(columns, rows)
        stream = pyte.ByteStream(screen)
        has_math = False
        marker_tail = b""
        child, master = pty.fork()
        if child == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, size)
            os.environ["CODEX_LATEX_CACHE"] = str(directory)
            os.environ["CODEX_LATEX_HELPER"] = str(HERE / "render.cjs")
            os.execvp(command[0], command)
        fcntl.ioctl(master, termios.TIOCSWINSZ, size)
        resized = False

        def resize(_signal, _frame):
            nonlocal resized
            resized = True

        signal.signal(signal.SIGWINCH, resize)

        def stop(_signal, _frame):
            raise SystemExit(0)

        signal.signal(signal.SIGHUP, stop)
        signal.signal(signal.SIGTERM, stop)
        try:
            tty.setraw(0, termios.TCSANOW)
            if pending_input:
                write_all(master, pending_input)
            while True:
                if resized:
                    size = fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 8)
                    rows, columns, _, _ = struct.unpack("HHHH", size)
                    if rows and columns:
                        screen.resize(lines=rows, columns=columns)
                        if view is None:
                            overlay.geometry(columns, rows)
                        fcntl.ioctl(master, termios.TIOCSWINSZ, size)
                    resized = False
                ready, _, _ = select.select([0, master], [], [], 0.05)
                if master in ready:
                    try:
                        data = os.read(master, 65536)
                    except OSError as error:
                        if error.errno == errno.EIO:
                            break
                        raise
                    if not data:
                        break
                    if view is None:
                        stream.feed(data)
                    else:
                        has_math = has_math or b"[m:" in marker_tail + data
                        marker_tail = data[-3:]
                    write_all(1, data)
                if 0 in ready:
                    data = os.read(0, 65536)
                    if not data:
                        break
                    write_all(master, data)
                overlay.update(
                    view.refresh(overlay, active=has_math, pane_size=(columns, rows))
                    if view
                    else screen
                )
        finally:
            try:
                termios.tcsetattr(0, termios.TCSADRAIN, original)
            except termios.error:
                pass
            if view:
                view.close()
            overlay.close()
            renderer.terminate()
            try:
                renderer.wait(timeout=2)
            except subprocess.TimeoutExpired:
                renderer.kill()
                renderer.wait()
            os.close(master)
            try:
                os.killpg(child, signal.SIGHUP)
            except ProcessLookupError:
                pass
            _, status = os.waitpid(child, 0)
        return os.waitstatus_to_exitcode(status)


if __name__ == "__main__":
    raise SystemExit(run(sys.argv[1:]))
