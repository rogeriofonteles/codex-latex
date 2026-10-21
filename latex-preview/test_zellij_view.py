import unittest
from zellij_view import layout


class PaneVisibility(unittest.TestCase):
    def setUp(self):
        self.pane = dict(
            id=7,
            is_plugin=False,
            tab_id=2,
            is_suppressed=False,
            is_fullscreen=False,
            is_floating=False,
            is_focused=True,
        )
        self.tab = dict(
            tab_id=2,
            active=True,
            is_fullscreen_active=False,
            are_floating_panes_visible=False,
        )

    def visible(self):
        return layout([self.pane], [self.tab], 7)[2]

    def test_current_tab_and_hidden_tab(self):
        self.assertTrue(self.visible())
        self.tab["active"] = False
        self.assertFalse(self.visible())

    def test_fullscreen_other_pane_and_suppression(self):
        self.tab["is_fullscreen_active"] = True
        self.assertFalse(self.visible())
        self.pane["is_fullscreen"] = True
        self.assertTrue(self.visible())
        self.pane["is_suppressed"] = True
        self.assertFalse(self.visible())

    def test_float_occlusion_and_hidden_float(self):
        self.tab["are_floating_panes_visible"] = True
        self.assertFalse(self.visible())
        self.pane["is_floating"] = True
        self.assertTrue(self.visible())
        self.tab["are_floating_panes_visible"] = False
        self.assertFalse(self.visible())

    def test_plugin_id_cannot_match_terminal(self):
        plugin = dict(self.pane, is_plugin=True)
        self.assertIs(layout([plugin, self.pane], [self.tab], 7)[0], self.pane)


class ResponsivePolling(unittest.TestCase):
    def test_viewport_preserves_wide_character_offsets(self):
        from zellij_view import viewport

        frame = viewport("界 [m:0123456789ab:00]\nplain text", 40, 4)
        self.assertEqual(frame.markers, (("0123456789ab", 0, 3, 0),))

    def test_slow_queries_do_not_block_refresh(self):
        import os
        import time
        from types import SimpleNamespace
        from unittest.mock import patch
        from zellij_view import ZellijView

        with patch.dict(
            os.environ, {"ZELLIJ_PANE_ID": "1", "ZELLIJ_SESSION_NAME": "test"}
        ):
            view = ZellijView()

        def slow_read():
            view.stopped.wait(0.5)

        view.read_state = slow_read
        overlay = SimpleNamespace(visible=True)
        start = time.monotonic()
        for _ in range(20):
            view.refresh(overlay)
        elapsed = time.monotonic() - start
        view.close()
        self.assertLess(elapsed, 0.1)

    def test_no_polling_before_math(self):
        import os
        from types import SimpleNamespace
        from unittest.mock import patch
        from zellij_view import ZellijView

        with patch.dict(
            os.environ, {"ZELLIJ_PANE_ID": "1", "ZELLIJ_SESSION_NAME": "test"}
        ):
            view = ZellijView()
        for _ in range(20):
            view.refresh(SimpleNamespace(), active=False)
        self.assertIsNone(view.worker)


class SlowPaneQueries(unittest.TestCase):
    def test_polling_survives_a_pane_query_longer_than_old_timeout(self):
        import json
        import os
        import subprocess
        import sys
        import threading
        from unittest.mock import patch
        from zellij_view import ZellijView

        with patch.dict(
            os.environ, {"ZELLIJ_PANE_ID": "7", "ZELLIJ_SESSION_NAME": "test"}
        ):
            view = ZellijView()
        pane = dict(
            id=7,
            is_plugin=False,
            tab_id=2,
            is_suppressed=False,
            is_fullscreen=False,
            is_floating=False,
            is_focused=True,
            pane_content_columns=80,
            pane_content_rows=24,
        )
        tab = dict(
            tab_id=2,
            active=True,
            is_fullscreen_active=False,
            are_floating_panes_visible=False,
        )
        real_check_output = subprocess.check_output
        completed = threading.Event()

        def query(command, **kwargs):
            action = command[command.index("action") + 1]
            if action == "list-panes":
                # Exercise the real subprocess timeout, rather than mocking sleep.
                return real_check_output(
                    [
                        sys.executable,
                        "-c",
                        "import time; time.sleep(0.6); print("
                        + repr(json.dumps([pane]))
                        + ")",
                    ],
                    **kwargs,
                )
            if action == "list-tabs":
                return json.dumps([tab])
            view.stopped.set()
            completed.set()
            return "[m:0123456789ab:00]"

        with patch("zellij_view.subprocess.check_output", side_effect=query):
            worker = threading.Thread(target=view.poll, daemon=True)
            worker.start()
            try:
                self.assertTrue(completed.wait(4), "pane query repeatedly timed out")
            finally:
                view.stopped.set()
                worker.join(4)
        self.assertIsNotNone(view.state)
        self.assertTrue(view.state[2])
        self.assertEqual(view.state[3].markers, (("0123456789ab", 0, 0, 0),))


class ResizeRecovery(unittest.TestCase):
    def test_slow_metadata_does_not_pause_viewport_tracking(self):
        import threading
        import time
        from unittest.mock import patch
        from zellij_view import ZellijView

        with patch.dict("os.environ", ZELLIJ_PANE_ID="7", ZELLIJ_SESSION_NAME="test"):
            view = ZellijView()
        view.metadata = (
            {"pane_content_columns": 80, "pane_content_rows": 24},
            {},
            True,
        )
        view.last_metadata = time.monotonic()
        querying = threading.Event()
        tracked = threading.Event()

        def slow_metadata():
            querying.set()
            view.stopped.wait(2)

        def dump(*args):
            if querying.is_set():
                tracked.set()
            return "[m:0123456789ab:00]"

        view.read_metadata = slow_metadata
        view.action = dump
        view.worker = threading.Thread(target=view.poll, daemon=True)
        view.worker.start()
        try:
            self.assertTrue(querying.wait(1))
            self.assertTrue(tracked.wait(0.5), "viewport waited for the metadata query")
        finally:
            view.close()

    def test_resize_waits_for_matching_pane_geometry_then_recovers(self):
        from unittest.mock import Mock, patch
        from zellij_view import ZellijView, Viewport

        with patch.dict("os.environ", ZELLIJ_PANE_ID="7", ZELLIJ_SESSION_NAME="test"):
            view = ZellijView()
        pane = dict(
            pane_content_columns=80,
            pane_content_rows=24,
            pane_content_x=81,
            pane_content_y=2,
        )
        tab = dict(display_area_columns=162, display_area_rows=28)
        frame = Viewport(80, 24, (("0123456789ab", 0, 2, 4),))
        view.state = (pane, tab, True, frame)
        overlay = Mock(cw=8, ch=16, visible=True)
        view.refresh(overlay, active=False, pane_size=(60, 24))
        self.assertFalse(overlay.visible)
        overlay.geometry.assert_not_called()
        pane = dict(pane, pane_content_columns=60, pane_content_x=61)
        view.state = (pane, tab, True, frame)
        self.assertEqual(view.refresh(overlay, active=False, pane_size=(60, 24)), frame)
        self.assertEqual(
            (overlay.visible, overlay.offset_x, overlay.offset_y), (True, 488, 32)
        )
