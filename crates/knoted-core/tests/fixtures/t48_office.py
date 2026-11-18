"""Generate real Office inputs for Rust loopback/Worker tests, without model use."""
import sys
from pathlib import Path

from docx import Document
from openpyxl import Workbook
from pptx import Presentation
from pptx.util import Inches

root = Path(sys.argv[1])
root.mkdir(parents=True, exist_ok=True)
doc = Document()
doc.add_paragraph("before中文")
table = doc.add_table(rows=1, cols=1)
table.cell(0, 0).text = "table中文"
doc.add_paragraph("after中文")
doc.save(root / "sample.docx")
book = Workbook()
book.active.title = "中文'原名"
book.active["C3"] = "cell中文"
book.save(root / "sample.xlsx")
deck = Presentation()
deck.slides.add_slide(deck.slide_layouts[6])
slide = deck.slides.add_slide(deck.slide_layouts[6])
slide._element.set("show", "0")
slide.shapes.add_textbox(0, 0, Inches(2), Inches(1)).text = "slide中文"
deck.save(root / "sample.pptx")
