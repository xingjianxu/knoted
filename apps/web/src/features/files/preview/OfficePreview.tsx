import { useLayoutEffect, useRef, useState, type RefObject } from "react";
import { Alert, Button, Spinner } from "@bixlab/mtone";
import { ApiRequestError, type ApiClient } from "@knoted/api-client";
import { PREVIEW_BYTES, type FileSource } from "./inspect";

const previewEndpoint = "/files/{fileId}/office-preview";
const unavailable = "Office 预览失败，请重试或下载原文件。";
const timedOut = "Office 预览启动超时，请重试或下载原文件。";
const startupLimit = 45_000;

type Result = { kind: "loading" | "ready" } | { kind: "error"; message: string };
export function OfficePreview({ api, file, disposeRef, onUnauthorized }: {
    api: ApiClient;
    file: FileSource;
    disposeRef: RefObject<(() => void) | null>;
    onUnauthorized: () => void;
}) {
    const origin = import.meta.env.KNOTED_OFFICE_ORIGIN;
    const { id: fileId, content_version: version } = file;
    const disabled = file.status !== "present" ? "来源已丢失，无法预览。" :
        file.size > PREVIEW_BYTES ? "预览限 50 MiB，请下载原文件。" :
        !origin ? "Office 预览未启用，请下载原文件。" :
        !("credentialless" in HTMLIFrameElement.prototype) || typeof MessageChannel !== "function"
            ? "当前浏览器不支持无凭据隔离，无法使用 Office 预览。请下载原文件。" : null;
    const host = useRef<HTMLDivElement>(null);
    const [attempt, setAttempt] = useState(0);
    const [state, setState] = useState<{ api: ApiClient; attempt: number; result: Result }>();
    const current: Result = state?.api === api && state.attempt === attempt ? state.result : { kind: "loading" };

    useLayoutEffect(() => {
        if (disabled || !host.current) return;
        const container = host.current;
        let phase: "opening" | "creating" | "starting" | "ready" | "stopped" = "opening";
        let previewId: string | undefined;
        let frame: HTMLIFrameElement | undefined;
        let port: MessagePort | undefined;
        const deadline = performance.now() + startupLimit;
        function revoke() {
            if (!previewId) return;
            const id = previewId;
            previewId = undefined;
            void api.raw.DELETE(previewEndpoint, {
                params: { path: { fileId }, query: { preview_id: id } },
            }).catch(() => { /* Session/TTL bounds an unreachable revoke. */ });
        }
        function stop() {
            if (phase === "stopped") return;
            phase = "stopped";
            clearTimeout(timer);
            port?.close();
            port = undefined;
            frame?.remove(); // The only owned browsing context; never touch the vendor's iframe.
            frame = undefined;
            revoke();
        }
        disposeRef.current = stop;
        function fail(message: string) {
            if (phase === "stopped") return;
            stop();
            setState({ api, attempt, result: { kind: "error", message } });
        }
        function expired() {
            if (phase === "ready" || performance.now() < deadline) return false;
            fail(timedOut);
            return true;
        }
        async function create(channel: MessagePort) {
            try {
                // A dispatched POST may create a lease after close/timeout. Keep the
                // receipt observable so this generation can revoke its own ID.
                const { data, response } = await api.raw.POST(previewEndpoint, {
                    params: { path: { fileId } }, body: { expected_content_version: version },
                });
                previewId = data?.preview_id;
                if (phase === "stopped") { revoke(); return; }
                if (expired()) return;
                if (!data || !previewId || response.status !== 201) { fail(unavailable); return; }
                if (data.document_server_origin !== origin) {
                    fail("Office 预览来源不匹配，请下载原文件。");
                    return;
                }
                phase = "starting";
                channel.postMessage({ type: "init", origin, config: data.config });
            } catch (error) {
                if (phase === "stopped") return;
                if (error instanceof ApiRequestError && error.status === 401) {
                    stop();
                    onUnauthorized();
                } else {
                    fail(unavailable);
                }
            }
        }
        function connect() {
            if (phase === "stopped" || expired()) return;
            const target = frame?.contentWindow;
            if (!target || port) { fail(unavailable); return; } // Never reuse a reloaded/navigated host.
            const channel = new MessageChannel();
            port = channel.port1;
            channel.port1.onmessage = ({ data }: MessageEvent<unknown>) => {
                if (phase === "stopped" || expired() || !data || typeof data !== "object" ||
                    !Object.hasOwn(data, "type") || Object.keys(data).length !== 1) return;
                const { type } = data as { type: unknown };
                if (type === "connected" && phase === "opening") {
                    phase = "creating";
                    void create(channel.port1);
                } else if (type === "ready" && phase === "starting") {
                    phase = "ready";
                    clearTimeout(timer);
                    setState({ api, attempt, result: { kind: "ready" } });
                } else if (type === "error" && (phase === "starting" || phase === "ready")) {
                    fail(unavailable);
                }
            };
            try {
                target.postMessage({ type: "knoted-office-connect" }, window.location.origin, [channel.port2]);
            } catch {
                channel.port2.close();
                fail(unavailable);
            }
        }
        const timer = setTimeout(() => fail(timedOut), startupLimit);
        // Don't start a browsing context in StrictMode's discarded setup probe.
        queueMicrotask(() => {
            if (phase === "stopped" || expired()) return;
            const element = document.createElement("iframe") as HTMLIFrameElement & { credentialless: boolean };
            element.credentialless = true;
            if (element.credentialless !== true) { fail(unavailable); return; }
            element.title = "Office 只读预览";
            element.className = "h-full w-full border-0";
            element.addEventListener("load", connect);
            element.src = "/office-preview.html";
            frame = element;
            container.append(element);
        });
        return () => {
            stop();
            disposeRef.current = null;
        };
    }, [api, attempt, disabled, disposeRef, fileId, onUnauthorized, origin, version]);

    if (disabled) return <Alert tone="warning">{disabled}</Alert>;
    return <>
        {current.kind === "loading" && <Spinner label="正在读取 Office 预览"/>}
        {current.kind === "error" && <Alert tone="warning">{current.message} <Button onClick={() => setAttempt(value => value + 1)}>重试 Office 预览</Button></Alert>}
        <div ref={host} className={current.kind === "error" ? "hidden" : "h-[60vh] w-full"}/>
    </>;
}
