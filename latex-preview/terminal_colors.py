"""Read terminal default colors without changing them; retain any typed input."""

import os
import re
import select
import termios
import time
import tty

REPLY = re.compile(
    rb"\x1b\](10|11);rgb:([0-9a-fA-F]{1,4})/([0-9a-fA-F]{1,4})/([0-9a-fA-F]{1,4})(?:\x07|\x1b\\)"
)


def extract(data):
    colors = {}

    def consume(match):
        channels = [
            round(int(c, 16) * 255 / (16 ** len(c) - 1)) for c in match.groups()[1:]
        ]
        colors["fg" if match[1] == b"10" else "bg"] = "#" + "".join(
            f"{c:02x}" for c in channels
        )
        return b""

    return colors, REPLY.sub(consume, data)


def query(input_fd=0, output_fd=1):
    original = termios.tcgetattr(input_fd)
    data = b""
    colors = {}
    try:
        tty.setraw(input_fd, termios.TCSANOW)
        os.write(output_fd, b"\x1b]10;?\x07\x1b]11;?\x07")
        deadline = time.monotonic() + 0.6
        while time.monotonic() < deadline:
            if select.select([input_fd], [], [], max(0, deadline - time.monotonic()))[
                0
            ]:
                chunk = os.read(input_fd, 4096)
                if not chunk:
                    break
                data += chunk
                colors, _ = extract(data)
                if len(colors) == 2:
                    break
        return extract(data)
    finally:
        termios.tcsetattr(input_fd, termios.TCSADRAIN, original)
