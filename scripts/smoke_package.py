"""Exercise bundled DOCX resources, cells and figures with synthetic local data."""

import subprocess
import sys
import tempfile
from pathlib import Path

from docx import Document
from PIL import Image

from notegen.models import Block, Box, PageResult
from notegen.storage import Project


def main(executable: str):
    with tempfile.TemporaryDirectory(prefix="notegen-package-smoke-") as tmp:
        root = Path(tmp)
        project = Project(root / "project", "synthetic", "synthetic", [1])
        Image.new("RGB", (400, 600), "white").save(project.image_path(1))
        project.save(
            PageResult(
                page=1,
                engine="smoke-test",
                image="page-00001.png",
                blocks=[
                    Block(kind="text", text="Packaged export works.", confidence=0.99),
                    Block(kind="table", rows=[["Item", "Amount"], ["Sales", "1250"]]),
                    Block(
                        kind="figure",
                        box=Box(left=0, top=0, right=1, bottom=1),
                        description="Synthetic blank figure for resource verification.",
                    ),
                ],
            )
        )
        target = root / "smoke.docx"
        subprocess.run(
            [
                str(Path(executable).resolve()),
                "--debug",
                "export",
                str(project.root),
                "-o",
                str(target),
            ],
            check=True,
            timeout=45,
        )
        doc = Document(target)
        assert doc.tables[0].cell(1, 1).text == "1250"
        assert len(doc.inline_shapes) == 1
        assert any("Packaged export works." in p.text for p in doc.paragraphs)
        print("Packaged Word export: text, editable cells, source figure and templates passed")


if __name__ == "__main__":
    main(sys.argv[1])
