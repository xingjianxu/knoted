import { queryOptions } from "@tanstack/react-query";
import type { components } from "@knoted/api-client";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
export type FileSource = components["schemas"]["FileSource"];
export function remember(bundle: QueryClientBundle, data: FileSource) {
    bundle.queryClient.setQueryData<FileSource>(queryKeys.files.detail(data.id), old => old && old.version > data.version ? old : data);
}
export function fileQuery(bundle: QueryClientBundle, id: string) {
    return queryOptions({ queryKey: queryKeys.files.detail(id), queryFn: async ({ signal }) => {
            const { data } = await bundle.api.raw.GET("/files/{fileId}", { params: { path: { fileId: id } }, signal });
            signal.throwIfAborted();
            if (!data || data.id !== id)
                throw new Error("文件详情响应无效");
            const old = bundle.queryClient.getQueryData<FileSource>(queryKeys.files.detail(id));
            return old && old.version > data.version ? old : data;
        } });
}
export function filesQuery(bundle: QueryClientBundle, cursor: string | null) {
    return queryOptions({ queryKey: queryKeys.files.list({ cursor: cursor ?? undefined }), queryFn: async ({ signal }) => {
            const { data } = await bundle.api.raw.GET("/files", { params: { query: { cursor: cursor ?? undefined, limit: 50 } }, signal });
            if (!data)
                throw new Error("文件列表响应无效");
            return data;
        } });
}
