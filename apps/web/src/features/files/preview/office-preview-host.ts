import type { components } from "@knoted/api-client";

type SignedConfig = components["schemas"]["OfficeEditorConfig"];
type Editor = { destroyEditor(): void };
type OfficeWindow = Window & {
    readonly credentialless?: boolean;
    DocsAPI?: {
        DocEditor: new (id: string, config: SignedConfig & {
            width: string;
            height: string;
            events: { onDocumentReady(): void; onError(): void };
        }) => Editor;
    };
};

function envelope(value: unknown, keys: string[]): value is Record<string, unknown> {
    return value !== null && typeof value === "object" && Object.keys(value).length === keys.length &&
        keys.every(key => Object.hasOwn(value, key));
}

export function startOfficePreviewHost(host: OfficeWindow) {
    const origin = import.meta.env.KNOTED_OFFICE_ORIGIN;
    const parent = host.parent;
    const ownOrigin = host.location.origin;
    if (!origin || host.credentialless !== true || parent === host) return;
    try {
        if (parent.location.origin !== ownOrigin) return;
    } catch { return; }

    const doc = host.document;
    const mount = doc.getElementById("office-preview-host");
    if (!mount) return;
    let port: MessagePort | undefined;
    let script: HTMLScriptElement | undefined;
    let editor: Editor | undefined;
    let initialized = false;
    let stopped = false;
    let ready = false;

    function destroy(instance?: Editor) {
        try { instance?.destroyEditor(); } catch { /* Unload and remove the owned host anyway. */ }
    }
    function stop() {
        if (stopped) return;
        stopped = true;
        host.removeEventListener("message", connect);
        port?.close();
        if (script) {
            script.onload = null;
            script.onerror = null;
            script.remove();
        }
        destroy(editor);
        mount?.replaceChildren();
    }
    function fail() {
        if (stopped) return;
        port?.postMessage({ type: "error" });
        stop();
    }
    function start(config: SignedConfig) {
        const placeholder = doc.createElement("div");
        placeholder.id = "office-preview-editor";
        mount?.append(placeholder);
        script = doc.createElement("script");
        script.src = `${origin}/web-apps/apps/api/documents/api.js`;
        script.onerror = fail;
        script.onload = () => {
            if (stopped) return;
            try {
                const sdk = host.DocsAPI;
                if (typeof sdk?.DocEditor !== "function") { fail(); return; }
                const created = new sdk.DocEditor(placeholder.id, {
                    ...config,
                    width: "100%", height: "100%",
                    events: {
                        onDocumentReady() {
                            if (stopped || ready) return;
                            ready = true;
                            port?.postMessage({ type: "ready" });
                        },
                        onError: fail,
                    },
                });
                if (stopped) destroy(created); // onError can run in the constructor.
                else editor = created;
            } catch { fail(); }
        };
        doc.head.append(script);
    }
    function connect(event: MessageEvent<unknown>) {
        if (stopped || port || event.source !== parent || event.origin !== ownOrigin ||
            !envelope(event.data, ["type"]) || event.data.type !== "knoted-office-connect" || event.ports.length !== 1) return;
        port = event.ports[0]!;
        host.removeEventListener("message", connect);
        port.onmessage = ({ data }: MessageEvent<unknown>) => {
            if (stopped || initialized || !envelope(data, ["type", "origin", "config"]) || data.type !== "init" ||
                !data.config || typeof data.config !== "object" || Array.isArray(data.config)) return;
            initialized = true;
            if (data.origin !== origin) { fail(); return; }
            // The signed payload is opaque here. Do not rewrite it, log it, or
            // add it to the location or storage. The host does not use Knoted APIs.
            start(data.config as SignedConfig);
        };
        port.postMessage({ type: "connected" });
    }
    host.addEventListener("message", connect);
    host.addEventListener("pagehide", stop, { once: true });
}

startOfficePreviewHost(window);
