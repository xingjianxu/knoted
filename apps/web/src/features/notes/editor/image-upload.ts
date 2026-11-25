import type { ApiClient } from "@knoted/api-client";

import { assetIdToUrl, isAssetUuid } from "./document-adapter";

export async function uploadNoteImage(
  client: ApiClient,
  file: File,
  signal?: AbortSignal,
): Promise<string> {
  const form = new FormData();
  form.append("file", file);
  const { data } = await client.raw.POST("/assets", {
    params: { header: { "Idempotency-Key": crypto.randomUUID() } },
    // Generated binary fields are strings. The serializer sends the actual
    // File as multipart; fetch owns Content-Type including its boundary.
    body: { file: file.name },
    bodySerializer: () => form,
    signal,
  });
  if (signal?.aborted) throw new Error("图片上传已取消");
  if (data?.state === "deleted") throw new Error("图片已回收，请重新选择文件");
  if (!data || data.state !== "ready" || !isAssetUuid(data.id)) {
    throw new Error("图片资产响应无效");
  }
  return assetIdToUrl(data.id);
}
