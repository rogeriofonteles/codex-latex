import unittest
from bridge import Screen, positions, x11_window_id
import pyte


class WindowIdValidation(unittest.TestCase):
    def test_x11_ids(self):
        self.assertEqual(x11_window_id("123456"), 123456)
        self.assertEqual(x11_window_id("0xabcdef"), 0xABCDEF)
        self.assertEqual(x11_window_id("4294967295"), 0xFFFFFFFF)

    def test_invalid_ids_fail_before_xlib_serialization(self):
        for value in (None, "", "invalid", "0", "-1", "4294967296", "102118277949168"):
            with self.subTest(value=value):
                with self.assertRaisesRegex(RuntimeError, "codex-latex --window"):
                    x11_window_id(value)


class TerminalTracking(unittest.TestCase):
    def test_scroll_and_erase(self):
        screen = Screen(40, 4)
        stream = pyte.ByteStream(screen)
        stream.feed(b"before\r\n[m:0123456789ab:00]\r\n[m:0123456789ab:01]")
        self.assertEqual(
            positions(screen), [("0123456789ab", 0, 0, 1), ("0123456789ab", 1, 0, 2)]
        )
        stream.feed(b"\r\nafter\r\n")
        self.assertEqual(
            positions(screen), [("0123456789ab", 0, 0, 0), ("0123456789ab", 1, 0, 1)]
        )
        stream.feed(b"\x1b[2J")
        self.assertEqual(positions(screen), [])

    def test_alternate_screen_restores_markers(self):
        screen = Screen(40, 4)
        stream = pyte.ByteStream(screen)
        stream.feed(b"[m:0123456789ab:00]")
        stream.feed(b"\x1b[?1049h\x1b[2Jnew screen")
        self.assertEqual(positions(screen), [])
        stream.feed(b"\x1b[?1049l")
        self.assertEqual(positions(screen), [("0123456789ab", 0, 0, 0)])

    def test_wide_unicode_preserves_cell_coordinates(self):
        screen = Screen(40, 4)
        stream = pyte.ByteStream(screen)
        stream.feed("界 [m:0123456789ab:00]".encode())
        self.assertEqual(positions(screen), [("0123456789ab", 0, 3, 0)])


if __name__ == "__main__":
    unittest.main()
