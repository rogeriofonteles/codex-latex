import unittest
from terminal_colors import extract


class TerminalColors(unittest.TestCase):
    def test_rgb16_and_rgb8_with_typed_input(self):
        colors, pending = extract(
            b"hello\x1b]10;rgb:d8d8/8080/ffff\x07!\x1b]11;rgb:12/34/56\x1b\\"
        )
        self.assertEqual(colors, {"fg": "#d880ff", "bg": "#123456"})
        self.assertEqual(pending, b"hello!")

    def test_no_response_keeps_input(self):
        self.assertEqual(extract(b"hello\r"), ({}, b"hello\r"))
