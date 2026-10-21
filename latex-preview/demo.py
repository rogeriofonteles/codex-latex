"""Offline smoke demo, using the same equation helper and overlay as Codex."""

import json
import os
from pathlib import Path
import subprocess
import time


def render_formula(formula, width):
    result = subprocess.run(
        ["node", os.environ["CODEX_LATEX_HELPER"], str(width)],
        input=formula,
        text=True,
        capture_output=True,
        check=True,
    )
    return json.loads(result.stdout)


print("\033[2J\033[HInline LaTeX rendering — offline integration test\r\n", flush=True)
for formula, prose in [
    (
        r"\int_0^1 x^2\,dx = \frac{1}{3}",
        "The integral appears between these sentences:",
    ),
    (
        r"\mathbf{A}=\begin{pmatrix}1&2\\3&4\end{pmatrix}",
        "Matrices should also render with sharp brackets and digits:",
    ),
    (r"x=\frac{-b\pm\sqrt{b^2-4ac}}{2a}", "And the quadratic formula:"),
]:
    print(prose + "\r\n", flush=True)
    print("\r\n".join(render_formula(formula, 90)), end="\r\n\r\n", flush=True)

print("LaTeX images inside table cells:\r\n", flush=True)
headers = ["Fraction", "Root", "Integral"]
formulas = [
    [r"\frac{a+b}{c+d}", r"\sqrt{x^2+y^2}", r"\int_0^1 x^2\,dx"],
    [
        r"\sum_{i=1}^n i",
        r"\frac{-b\pm\sqrt{b^2-4ac}}{2a}",
        r"\begin{pmatrix}1&2\\3&4\end{pmatrix}",
    ],
]
cell_width = 26
border = "+" + "+".join("-" * cell_width for _ in headers) + "+"
print(border, flush=True)
print("|" + "|".join(header.center(cell_width) for header in headers) + "|", flush=True)
print(border, flush=True)
for row_index, row in enumerate(formulas):
    images = [render_formula(formula, cell_width - 1) for formula in row]
    row_height = max(map(len, images))
    for line_index in range(row_height):
        cells = [image[line_index] if line_index < len(image) else "" for image in images]
        print("|" + "|".join(cell.ljust(cell_width) for cell in cells) + "|", flush=True)
    if row_index + 1 < len(formulas):
        print(border, flush=True)
print(border + "\r\n", flush=True)
print("Normal chat text continues below the equations.\r\n", flush=True)
Path("/tmp/codex-latex-demo-ready").touch()
# No model request and no conversation data are used by this demo.
time.sleep(40)
print(
    "\033[2J\033[HImages should disappear when their chat rows are cleared.\r\n",
    flush=True,
)
time.sleep(5)
