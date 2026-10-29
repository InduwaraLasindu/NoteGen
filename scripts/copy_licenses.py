"""Preserve installed dependency license texts in a portable distribution."""

import json
import shutil
import sys
from importlib.metadata import distribution
from pathlib import Path


def main(destination: Path):
    repo = Path(__file__).resolve().parents[1]
    destination.mkdir(parents=True, exist_ok=True)
    manifest = {}
    for line in (repo / "requirements-lock.txt").read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        name = line.split("==")[0]
        package = distribution(name)
        copied = []
        for member in package.files or []:
            value = str(member).lower()
            if (
                not any(marker in value for marker in ("license", "copying", "notice", "copyright"))
                or ".." in member.parts
                or member.suffix in (".py", ".pyc", ".so", ".pyd")
            ):
                continue
            source = Path(package.locate_file(member))
            if not source.is_file():
                continue
            target = destination / name / str(member)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
            copied.append(str(member))
        manifest[name] = {
            "version": package.version,
            "license_files": copied,
            "license_expression": package.metadata.get("License-Expression", ""),
        }
    # Qt wheels do not consistently contain complete standalone license texts.
    shutil.copytree(
        repo / "third_party" / "licenses" / "Qt", destination / "Qt", dirs_exist_ok=True
    )
    for name in ("LICENSE.txt", "LICENSE"):
        source = Path(sys.base_prefix) / name
        if source.is_file():
            shutil.copyfile(source, destination / ("Python-" + name))
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"Dependency notices collected for {len(manifest)} packages")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
