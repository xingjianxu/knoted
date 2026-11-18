import { useId, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { Alert, Button, Spinner } from "@bixlab/mtone";
import { ApiRequestError, type ApiClient, type components } from "@knoted/api-client";
import { PREVIEW_BYTES, type FileSource } from "./inspect";

type Editor = { destroyEditor(): void };
type DocsApi = {
    DocEditor: new (id: string, config: components["schemas"]["OfficeEditorConfig"] & {
        width: string;
        height: string;
        events: { onDocumentReady(): void; onError(): void };
    }) => Editor;
};
let sdkLoad: Promise<DocsApi> | undefined;
function loadSdk(origin: string): Promise<DocsApi> {
    // One immutable built origin per page; retry only a failed script load.
    sdkLoad ??= new Promise<DocsApi>((resolve, reject) => {
        const script = document.createElement("script");
        script.src = `${origin}/web-apps/apps/api/documents/api.js`;
        script.async = true;
        function fail() {
            script.remove();
            reject(new Error("Office SDK unavailable"));
        }
        script.onerror = fail;
        script.onload = () => {
            const sdk = (window as Window & { DocsAPI?: DocsApi }).DocsAPI;
            if (typeof sdk?.DocEditor === "function") resolve(sdk);
            else fail();
        };
        document.head.append(script);
    }).catch(error => {
        sdkLoad = undefined;
        throw error;
    });
    return sdkLoad;
}

const previewEndpoint = "/files/{fileId}/office-preview";
const unavailable = "Office 预览失败，请重试或下载原文件。";
function destroy(editor?: Editor) {
    // Teardown failures must not stop the host removal or lease revocation.
    try { editor?.destroyEditor(); } catch { /* The SDK host is removed separately. */ }
}

type Result = { kind: "loading" | "ready" } | { kind: "error"; message: string };
export function OfficePreview({ api, file, disposeRef, onUnauthorized }: {
    api: ApiClient;
    file: FileSource;
    disposeRef: RefObject<(() => void) | null>;
    onUnauthorized: () => void;
}) {
    const { KNOTED_OFFICE_ORIGIN: origin } = import.meta.env;
    const { id: fileId, content_version: version } = file;
    const disabled = file.status !== "present" ? "来源已丢失，无法预览。" :
        file.size > PREVIEW_BYTES ? "预览限 50 MiB，请下载原文件。" :
        !origin ? "Office 预览未启用，请下载原文件。" : null;
    const host = useRef<HTMLDivElement>(null);
    const hostId = useId();
    const [attempt, setAttempt] = useState(0);
    const [state, setState] = useState<{ api: ApiClient; attempt: number; result: Result }>();
    const current: Result = state?.api === api && state.attempt === attempt ? state.result : { kind: "loading" };

    useLayoutEffect(() => {
        if (disabled || !host.current) return;
        const container = host.current;
        let stopped = false;
        let previewId: string | undefined;
        let editor: Editor | undefined;
        let mount: HTMLDivElement | undefined;
        function revoke() {
            if (!previewId) return;
            const id = previewId;
            previewId = undefined;
            // No abort signal: the request remains valid after this owner exits.
            void api.raw.DELETE(previewEndpoint, {
                params: { path: { fileId }, query: { preview_id: id } },
            }).catch(() => { /* Session/TTL still bounds an unreachable revoke. */ });
        }
        function stop() {
            if (stopped) return;
            stopped = true;
            destroy(editor);
            editor = undefined;
            mount?.remove();
            mount = undefined;
            revoke();
        }
        disposeRef.current = stop;
        function fail(message: string) {
            if (stopped) return;
            stop();
            setState({ api, attempt, result: { kind: "error", message } });
        }
        async function load() {
            if (stopped) return;
            try {
                // A dispatched POST may create a lease even if the dialog closes.
                // Await its receipt without aborting so this owner can revoke its own ID.
                const { data, response } = await api.raw.POST(previewEndpoint, {
                    params: { path: { fileId } }, body: { expected_content_version: version },
                });
                previewId = data?.preview_id;
                if (stopped) { revoke(); return; }
                if (!data || !previewId || response.status !== 201) { fail(unavailable); return; }
                if (data.document_server_origin !== origin) {
                    fail("Office 预览来源不匹配，请下载原文件。");
                    return;
                }
                const sdk = await loadSdk(origin);
                if (stopped) return;
                // SDK code owns this subtree, including replacing its placeholder.
                mount = document.createElement("div");
                const placeholder = document.createElement("div");
                placeholder.id = hostId;
                mount.className = "h-full";
                mount.append(placeholder);
                container.append(mount);
                const created = new sdk.DocEditor(hostId, {
                    ...data.config,
                    width: "100%", height: "100%",
                    events: {
                        onDocumentReady() {
                            if (!stopped) setState({ api, attempt, result: { kind: "ready" } });
                        },
                        onError() { fail(unavailable); },
                    },
                });
                if (stopped) destroy(created); // A constructor can invoke onError synchronously.
                else editor = created;
            } catch (error) {
                if (stopped) return;
                if (error instanceof ApiRequestError && error.status === 401) {
                    stop();
                    onUnauthorized();
                } else {
                    fail(unavailable);
                }
            }
        }
        // Don't spend a lease in StrictMode's setup/cleanup probe.
        queueMicrotask(() => { void load(); });
        return () => {
            stop();
            disposeRef.current = null;
        };
    }, [api, attempt, disabled, disposeRef, fileId, hostId, onUnauthorized, origin, version]);

    if (disabled) return <Alert tone="warning">{disabled}</Alert>;
    return <>
        {current.kind === "loading" && <Spinner label="正在读取 Office 预览"/>}
        {current.kind === "error" && <Alert tone="warning">{current.message} <Button onClick={() => setAttempt(value => value + 1)}>重试 Office 预览</Button></Alert>}
        <div ref={host} className={current.kind === "error" ? "hidden" : "h-[60vh] w-full"}/>
    </>;
}
