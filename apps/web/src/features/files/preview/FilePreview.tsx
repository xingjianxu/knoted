import { useCallback, useEffect, useRef, useState } from "react";
import { Alert, Button, Dialog, Image, Spinner } from "@bixlab/mtone";
import { ApiRequestError, type ApiClient } from "@knoted/api-client";
import { inspectPreview, isOfficeFile, PREVIEW_BYTES, previewKind, type FileSource } from "./inspect";
import { OfficePreview } from "./OfficePreview";
import { readPreviewBytes } from "./read";
export interface FilePreviewProps {
    api: ApiClient;
    file: FileSource | null;
    open: boolean;
    onOpenChange: (open: boolean) => void;
    /** Ordinary original download, deliberately independent of preview byte limits. */
    onDownload: (file: FileSource) => void | Promise<void>;
}
type Owner = {
    controller: AbortController;
    disposed: boolean;
    url?: string;
    dispose: () => void;
};
type State = {
    api: ApiClient;
} & ({
    kind: "loading";
} | {
    kind: "error";
    message: string;
} | {
    kind: "text";
    text: string;
} | {
    kind: "image" | "pdf";
    url: string;
});
export function FilePreview(props: FilePreviewProps) {
    if (!props.open || !props.file)
        return null;
    const file = props.file;
    // A key is also the render-time guard: changed identity cannot paint old state
    // even before effect cleanup. A→B→A creates three distinct component lifetimes.
    const identity = JSON.stringify([file.id, file.content_version, file.version, file.sha256,
        file.name, file.mime, file.size, file.status, file.relative_path]);
    return <OpenPreview key={identity} {...props} file={file}/>;
}
function OpenPreview({ api, file, onOpenChange, onDownload }: FilePreviewProps & {
    file: FileSource;
}) {
    const [snapshot] = useState(file);
    const [state, setState] = useState<State>({ api, kind: "loading" });
    const [closed, setClosed] = useState(false);
    const [unauthorized, setUnauthorized] = useState(false);
    const [downloading, setDownloading] = useState(false);
    const [downloadError, setDownloadError] = useState("");
    const lifetime = useRef<Owner | null>(null);
    const officeLifetime = useRef<(() => void) | null>(null);
    const office = isOfficeFile(snapshot);
    const markUnauthorized = useCallback(() => setUnauthorized(true), []);
    useEffect(() => {
        const owner: Owner = {
            controller: new AbortController(), disposed: false,
            dispose() {
                owner.disposed = true;
                owner.controller.abort();
                if (owner.url) {
                    URL.revokeObjectURL(owner.url);
                    owner.url = undefined;
                }
            },
        };
        lifetime.current = owner;
        async function load() {
            try {
                if (snapshot.status !== "present")
                    throw new Error("来源已丢失，无法预览。");
                previewKind(snapshot);
                if (snapshot.size > PREVIEW_BYTES)
                    throw new Error("预览限 50 MiB，请下载原文件。");
                const { response } = await api.raw.GET("/files/{fileId}/content", {
                    params: { path: { fileId: snapshot.id } }, parseAs: "stream", signal: owner.controller.signal,
                });
                if (owner.disposed || owner.controller.signal.aborted) {
                    await response.body?.cancel().catch(() => { });
                    return;
                }
                if (response.headers.get("etag") !== `"${snapshot.version}"`) {
                    void response.body?.cancel().catch(() => { });
                    throw new Error("文件元数据版本不一致，请刷新文件详情后重新预览。");
                }
                const bytes = await readPreviewBytes(response, owner.controller);
                if (owner.disposed || owner.controller.signal.aborted)
                    return;
                const preview = inspectPreview(snapshot, bytes);
                if (preview.kind === "text") {
                    setState({ api, ...preview });
                }
                else {
                    const url = URL.createObjectURL(new Blob([bytes], { type: preview.mime }));
                    owner.url = url;
                    if (owner.disposed || owner.controller.signal.aborted) {
                        URL.revokeObjectURL(url);
                        owner.url = undefined;
                        return;
                    }
                    setState({ api, kind: preview.kind, url });
                }
            }
            catch (error) {
                if (owner.disposed)
                    return;
                if (error instanceof ApiRequestError && error.status === 401) {
                    owner.dispose();
                    setUnauthorized(true);
                }
                else if (!(error instanceof DOMException && error.name === "AbortError")) {
                    setState({ api, kind: "error", message: error instanceof Error ? error.message : "预览失败，请下载原文件。" });
                }
            }
        }
        if (!office) void load();
        return owner.dispose;
    }, [api, office, snapshot]);
    useEffect(() => { if (unauthorized)
        onOpenChange(false); }, [unauthorized, onOpenChange]);
    function close() {
        officeLifetime.current?.();
        lifetime.current?.dispose();
        setClosed(true);
        onOpenChange(false);
    }
    async function download() {
        const owner = lifetime.current;
        if (!owner || owner.disposed || downloading)
            return;
        setDownloading(true);
        setDownloadError("");
        try {
            await onDownload(file);
        }
        catch (error) {
            if (owner.disposed)
                return;
            if (error instanceof ApiRequestError && error.status === 401) {
                officeLifetime.current?.();
                owner.dispose();
                setUnauthorized(true);
            }
            else
                setDownloadError("下载失败，请重试。");
        }
        finally {
            if (!owner.disposed)
                setDownloading(false);
        }
    }
    if (closed || unauthorized)
        return null;
    const current = state.api === api ? state : { kind: "loading" as const };
    return <Dialog open title={`文件预览：${file.name}`} onOpenChange={open => { if (!open)
        close(); }} className="w-[min(64rem,calc(100vw-2rem))]" footer={<><Button onClick={close}>关闭预览</Button><Button disabled={downloading || file.status !== "present"} onClick={() => void download()}>下载原文件</Button></>}>
    <div className="max-h-[max(3rem,min(70dvh,calc(100dvh-9rem)))] overflow-auto">
      {office ? <OfficePreview api={api} file={snapshot} disposeRef={officeLifetime} onUnauthorized={markUnauthorized}/> : <>
        {current.kind === "loading" && <Spinner label="正在读取预览"/>}
        {current.kind === "error" && <Alert tone="warning">{current.message}</Alert>}
        {current.kind === "text" && <pre aria-label="文件文本" className="whitespace-pre-wrap wrap-anywhere">{current.text}</pre>}
        {current.kind === "image" && <Image src={current.url} alt={file.name} preview={false} fit="contain" className="max-h-[60dvh] max-w-full" onError={() => {
            if (!lifetime.current || lifetime.current.disposed || lifetime.current.url !== current.url)
                return;
            URL.revokeObjectURL(current.url);
            lifetime.current.url = undefined;
            setState({ api, kind: "error", message: "图片解码失败，请下载原文件。" });
        }}/>}
        {current.kind === "pdf" && <><p>由浏览器 PDF 查看器显示；如无法显示，请下载原文件。</p><iframe title={`PDF 预览：${file.name}`} src={current.url} className="h-[max(3rem,min(60dvh,calc(100dvh-11rem)))] w-full"/></>}
      </>}
      {downloadError && <Alert tone="error">{downloadError}</Alert>}
    </div>
  </Dialog>;
}
