"""Conservative scan enhancement; retain color for the vision model and figures."""

from dataclasses import dataclass

import cv2
import numpy as np
from PIL import Image


@dataclass
class Prepared:
    image: Image.Image
    ocr: Image.Image
    warnings: list[str]
    angle: float = 0


def prepare(image: Image.Image, deskew: bool = True) -> Prepared:
    rgb = np.array(image.convert("RGB"))
    gray = cv2.cvtColor(rgb, cv2.COLOR_RGB2GRAY)
    warnings = []
    # Relative to scan resolution; this is a warning heuristic, not a quality score.
    small = cv2.resize(
        gray, (min(1000, gray.shape[1]), max(1, int(gray.shape[0] * min(1, 1000 / gray.shape[1]))))
    )
    if cv2.Laplacian(small, cv2.CV_64F).var() < 45 and np.std(small) > 3:
        warnings.append("Possible blur or low detail. Check recognition against the source.")
    angle = 0.0
    if deskew:
        binary = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)[1]
        lines = cv2.HoughLinesP(
            binary,
            1,
            np.pi / 1800,
            threshold=90,
            minLineLength=max(60, gray.shape[1] // 6),
            maxLineGap=20,
        )
        if lines is not None:
            slopes = [
                np.degrees(np.arctan2(y2 - y1, x2 - x1))
                for x1, y1, x2, y2 in lines.reshape(-1, 4)
                if x2 != x1
            ]
            slopes = [a for a in slopes if abs(a) <= 7]
            if slopes:
                angle = float(np.median(slopes))
        if 0.2 < abs(angle) <= 7:
            h, w = gray.shape
            mat = cv2.getRotationMatrix2D((w / 2, h / 2), angle, 1)
            rgb = cv2.warpAffine(
                rgb,
                mat,
                (w, h),
                flags=cv2.INTER_CUBIC,
                borderMode=cv2.BORDER_CONSTANT,
                borderValue=(255, 255, 255),
            )
            gray = cv2.cvtColor(rgb, cv2.COLOR_RGB2GRAY)
    enhanced = cv2.createCLAHE(clipLimit=1.5, tileGridSize=(8, 8)).apply(gray)
    return Prepared(Image.fromarray(rgb), Image.fromarray(enhanced), warnings, angle)


def box_from_pixels(x: int, y: int, w: int, h: int, size: tuple[int, int]):
    from .models import Box

    iw, ih = size
    return Box(
        left=max(0, x / iw),
        top=max(0, y / ih),
        right=min(1, (x + w) / iw),
        bottom=min(1, (y + h) / ih),
    )
