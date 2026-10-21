import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image

HERE = Path(__file__).resolve().parent


class FormulaRendering(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.cache = Path(self.directory.name)
        (self.cache / "geometry.json").write_text('{"cw":9,"ch":18}')

    def render(self, source):
        return subprocess.run(
            [str(HERE / "runtime/bin/node"), str(HERE / "render.cjs"), "80"],
            input=source,
            capture_output=True,
            text=True,
            timeout=10,
            env=dict(os.environ, CODEX_LATEX_CACHE=str(self.cache)),
        )

    def test_formula_png_and_cache(self):
        formula = r"\int_0^1 x^2\,dx=\frac{1}{3}"
        first = self.render(formula)
        self.assertEqual(first.returncode, 0, first.stderr)
        markers = json.loads(first.stdout)
        png = next(self.cache.glob("*.png"))
        before = png.stat().st_mtime_ns
        with Image.open(png) as image:
            self.assertEqual(image.getpixel((0, 0))[3], 0)
            self.assertEqual(image.height, len(markers) * 18)
            self.assertLessEqual(image.width, 80 * 9)
        second = self.render(formula)
        self.assertEqual(
            (second.returncode, second.stdout, png.stat().st_mtime_ns),
            (0, first.stdout, before),
        )

    def test_matrix(self):
        result = self.render(r"\begin{pmatrix}1&2\\3&4\end{pmatrix}")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertGreater(len(json.loads(result.stdout)), 1)

    def test_invalid_and_oversized_input_fall_back(self):
        for source in [r"\unknowncodexcommand{x}", "x" * 8193, r"\require{html}"]:
            with self.subTest(source=source[:30]):
                result = self.render(source)
                self.assertEqual((result.returncode, result.stdout), (1, ""))


if __name__ == "__main__":
    unittest.main()
