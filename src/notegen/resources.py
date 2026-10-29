"""Best-effort local telemetry and conservative memory guards."""

import csv
import io
import os
import shutil
import subprocess
from dataclasses import asdict, dataclass

import psutil

from .errors import NoteGenError


@dataclass
class Resources:
    ram_free_gib: float
    app_rss_gib: float
    gpu_name: str = "NVIDIA telemetry unavailable"
    vram_used_mib: float = 0
    vram_total_mib: float = 0

    def as_dict(self):
        return asdict(self)


def snapshot() -> Resources:
    ram = psutil.virtual_memory()
    result = Resources(ram.available / 2**30, psutil.Process().memory_info().rss / 2**30)
    executable = shutil.which("nvidia-smi")
    if not executable:
        return result
    try:
        process = subprocess.run(
            [
                executable,
                "--query-gpu=name,memory.total,memory.used",
                "--format=csv,noheader,nounits",
            ],
            capture_output=True,
            text=True,
            timeout=3,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
        )
        if process.returncode == 0:
            row = next(csv.reader(io.StringIO(process.stdout)))
            result.gpu_name = row[0].strip()
            result.vram_total_mib, result.vram_used_mib = float(row[1]), float(row[2])
    except (OSError, ValueError, StopIteration, subprocess.TimeoutExpired):
        pass
    return result


def guard(resources: Resources):
    if resources.ram_free_gib < 1.0 or resources.app_rss_gib > 4.0:
        raise NoteGenError("NG300", "RAM safety margin reached; completed pages are preserved")
    if resources.vram_total_mib and resources.vram_used_mib > min(
        14848, resources.vram_total_mib - 512
    ):
        raise NoteGenError("NG300", "VRAM safety margin reached; completed pages are preserved")
