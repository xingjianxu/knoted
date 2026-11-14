# OCR 解析 fixture（T10）

`sample_zh_en.png` 是**真实渲染**的中英混排文档图片，不是 mock：T10 的
`services/intelligence/tests/test_t10.py` 会让 Docling 的版面模型、TableFormer fast 与
RapidOCR（onnxruntime, `ch`/`small`）完整跑一遍，并断言下面的关键词被保留。

| 项 | 值 |
| --- | --- |
| 尺寸 | 1240 × 980 px |
| 体积 | 54,426 B（< 1 MiB，可安全入库） |
| 内容 | 中文标题、中英混排行、日期/数字、3×4 带线框表格 |

## 生成方式

```bash
cd tests/fixtures/ocr
python generate_sample.py --font /path/to/NotoSansCJK.ttc
```

- 字体：Noto Sans CJK（host 上为 `noto-fonts-cjk-sans-2.004`，
  `/nix/store/a9jx9h2bx20lnl3vrqmkz1q3jgpxqaaw-noto-fonts-cjk-sans-2.004/share/fonts/opentype/noto-cjk/NotoSansCJK-VF.otf.ttc`）。
  用仅含拉丁字形的字体会渲染出豆腐块，OCR 断言就失去意义。
- 脚本用 Pillow 绘制；除字体外无环境依赖。重新生成应得到同名文件（PNG 字节可能因
  Pillow/zlib 版本略有差异，关键词与版面不变）。
- 渲染文本的常量在 `generate_sample.py` 顶部，测试中的关键词必须与之一致。

## 预期关键词

中文（OCR 会丢掉 CJK 词间空格，因此按子串断言）：

```text
知识库  合同  笔记  检索  摘要  中文识别
```

英文 / 数字：

```text
docling  rapidocr  offline  table  2026-11-15
```

表格应被识别成 `table` 块（markdown 三列），标题行应成为 `heading` 块。若模型或
OCR 配置被替换，这些断言会失败——这是有意的：fixture 与 `parsing/LOCK.md` 的锁版
组合绑定，不用于容忍“换个模型也能过”。
