# 嵌入 fixture（T11）

`sentences.json` 是 BGE-M3 dense 路径的**小样本功能证据**，不是语料库，也不是基准数据：

| id | 语言 | 用途 |
| --- | --- | --- |
| `zh-notes` | 中文 | 与 `zh-search` 语义相近（同一主题的两句中文） |
| `zh-search` | 中文 | 同上；用于验证真实模型给相近句更高的余弦相似度 |
| `en-search` | 英文 | 与上两句主题相关但语言不同，用于跨语言冒烟 |
| `mixed-query` | 中英混排 | 确认 tokenizer 能同时处理 CJK 与拉丁字符 |

## 预期（只断言这些）

- `embed_dense([...])` 对四条文本返回 `4 x 1024` 的 float32 数值，每行 L2 范数为
  `1.0 ± 1e-4`。
- 同一条文本放进同一个 batch 两次，两行逐元素差 ≤ `1e-5`（确定性/近似确定性）。
- `cos(zh-notes, zh-search)` **大于** `cos(zh-notes, en-search)`：这是“模型确实加载了
  语义权重”的冒烟判据，host 实测为 `0.79` 对 `0.25`，留有余量。

## 不预期

- **不**用于性能、吞吐、延迟、显存或召回率基准；T11 明确不做这些断言。
- 不校验具体相似度数值（会随模型 revision 变化）；换 revision 必须更新
  `services/intelligence/src/knoted_intelligence/embedding/LOCK.md` 并重跑 `test_t11.py`。
- 不覆盖长文本（8192 token）或跨语言检索质量。
