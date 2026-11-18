import type { JobState } from "./queries";

export const jobStates = {
  queued: "排队中",
  running: "运行中",
  retry_wait: "等待重试",
  succeeded: "已完成",
  failed: "失败",
  cancel_requested: "正在取消",
  cancelled: "已取消",
} satisfies Record<JobState, string>;

const errorMessages: Record<string, string> = {
  unsupported_format: "确认文件格式受支持后再重试。",
  invalid_text_encoding: "请检查源文件的文本编码。",
  input_too_large: "请检查文件是否超过 100 MiB。",
  image_too_large: "请检查图片分辨率是否超过 40M 像素。",
  too_many_pages: "请检查 PDF 是否超过 500 页。",
  output_too_large: "请检查文档或解析产物是否超过处理上限。",
  deadline_exceeded: "请检查文件是否需要拆分后再处理。",
  invalid_input: "请检查源文件是否损坏或为空。",
  file_scanning_disabled: "请检查文件扫描配置。",
  scan_unavailable: "请检查源目录挂载、权限及读取状态。",
  source_changed: "请等待文件内容稳定后再扫描。",
  model_unavailable: "请检查解析模型与依赖服务的配置。",
  job_failed: "请确认依赖服务可用后重试。",
};

export function jobProblem(error: string) {
  const code = Object.hasOwn(errorMessages, error) ? error : "job_failed";
  return { code, message: errorMessages[code] };
}

export const jobStages = {
  extraction: "采集与解析",
  fulltext: "全文索引",
  vector: "向量索引",
  other: "其他任务",
} as const;

export function jobStage(kind: string): keyof typeof jobStages {
  if (kind.startsWith("file.")) return "extraction";
  if (kind.startsWith("fulltext.") || kind.startsWith("rag_chunks.")) return "fulltext";
  if (kind.startsWith("rag_vectors.")) return "vector";
  return "other";
}

const jobNames: Record<string, string> = {
  "file.scan": "扫描文件",
  "file.capture": "采集文件",
  "file.extract": "解析文件",
  "fulltext.publish": "发布全文索引",
  "fulltext.cleanup": "清理全文索引",
  "rag_chunks.build": "生成检索分块",
  "rag_chunks.publish": "发布分块词法索引",
  "rag_vectors.embed": "嵌入与发布向量",
};

export function jobName(kind: string) {
  return Object.hasOwn(jobNames, kind) ? jobNames[kind] : "后台任务";
}
