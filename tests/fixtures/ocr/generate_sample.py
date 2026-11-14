"""Regenerates the T10 Chinese/English OCR fixture.

The image is a real rendered document (not a mock): Docling's layout model,
TableFormer and RapidOCR all run on it in ``services/intelligence/tests/test_t10.py``.
Text is rendered with Noto Sans CJK so the Chinese glyphs exist in the raster;
a Latin-only font would produce tofu boxes and the OCR assertions would be
meaningless.

Usage (font path is host-specific, so it is an argument):

    python generate_sample.py [--font /path/to/NotoSansCJK.otf.ttc]

The committed PNG was produced on wks-lab with
``noto-fonts-cjk-sans-2.004`` at
``/nix/store/a9jx9h2bx20lnl3vrqmkz1q3jgpxqaaw-noto-fonts-cjk-sans-2.004/share/fonts/opentype/noto-cjk/NotoSansCJK-VF.otf.ttc``.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

DEFAULT_FONT = (
    "/nix/store/a9jx9h2bx20lnl3vrqmkz1q3jgpxqaaw-noto-fonts-cjk-sans-2.004"
    "/share/fonts/opentype/noto-cjk/NotoSansCJK-VF.otf.ttc"
)
OUTPUT = Path(__file__).with_name("sample_zh_en.png")

#: Keywords the OCR test asserts, one per rendered line. Chinese words are
#: separated so the recogniser cannot glue two keywords into one unknown token;
#: the test accepts either spacing because PP-OCR sometimes drops CJK spaces.
TITLE = "Knoted 中文识别验证 T10"
CHINESE_LINE = "知识库 合同 笔记 检索 摘要"
ENGLISH_LINE = "English keywords: docling rapidocr offline table"
MIXED_LINE = "混排 Mixed 文本 2026-11-15 端口 15603"
TABLE_ROWS = [
    ("组件", "版本", "状态"),
    ("Docling", "2.150.1", "ok"),
    ("RapidOCR", "3.10.0", "ok"),
    ("TableFormer", "v2.3.0", "ok"),
]


def render(font_path: Path, output: Path = OUTPUT) -> Path:
    image = Image.new("RGB", (1240, 980), "white")
    draw = ImageDraw.Draw(image)
    title_font = ImageFont.truetype(str(font_path), 44)
    body_font = ImageFont.truetype(str(font_path), 30)

    draw.text((60, 50), TITLE, font=title_font, fill="black")
    draw.text((60, 130), CHINESE_LINE, font=body_font, fill="black")
    draw.text((60, 190), ENGLISH_LINE, font=body_font, fill="black")
    draw.text((60, 250), MIXED_LINE, font=body_font, fill="black")

    # A ruled table gives TableFormer real structure to recover.
    left, top, row_height, column_width = 60, 360, 70, 330
    rows, columns = len(TABLE_ROWS), len(TABLE_ROWS[0])
    for row in range(rows + 1):
        y = top + row * row_height
        draw.line([(left, y), (left + columns * column_width, y)], fill="black", width=3)
    for column in range(columns + 1):
        x = left + column * column_width
        draw.line([(x, top), (x, top + rows * row_height)], fill="black", width=3)
    for row, cells in enumerate(TABLE_ROWS):
        for column, text in enumerate(cells):
            draw.text(
                (left + column * column_width + 12, top + row * row_height + 16),
                text,
                font=body_font,
                fill="black",
            )

    output.parent.mkdir(parents=True, exist_ok=True)
    image.save(output, optimize=True)
    return output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--font", type=Path, default=Path(DEFAULT_FONT))
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()
    path = render(args.font, args.output)
    print(f"wrote {path} ({path.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
