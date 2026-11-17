import type { components } from "@knoted/api-client";
import { inspectImage } from "./images";
export type FileSource = components["schemas"]["FileSource"];
export type Preview = {
    kind: "text";
    text: string;
} | {
    kind: "image" | "pdf";
    mime: string;
};
export const PREVIEW_BYTES = 52428800;
// Keep aligned with crates/knoted-core/src/files/text.rs::supports (T46).
const textExtensions = new Set("txt md markdown rs py js jsx ts tsx c h cpp hpp cc java go rb php sh bash zsh fish sql html htm css scss xml json toml yaml yml ini cfg conf csv tsv log nix swift kt kts lua r pl ex exs erl hrl vue svelte".split(" "));
const textNames = new Set(["readme", "license", "makefile", "dockerfile", "justfile"]);
const imageMimes: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", webp: "image/webp" };
export function previewKind(file: FileSource): "text" | "pdf" | "image" {
    const name = file.name.replace(/[A-Z]/g, c => c.toLowerCase());
    const extension = name.lastIndexOf(".") > 0 ? name.slice(name.lastIndexOf(".") + 1) : "";
    const mime = file.mime?.split(";", 1)[0]?.trim().toLowerCase();
    const generic = !mime || mime === "application/octet-stream";
    if (extension === "pdf" && (generic || mime === "application/pdf"))
        return "pdf";
    if (imageMimes[extension] && (generic || mime === imageMimes[extension]))
        return "image";
    if ((textNames.has(name) || textExtensions.has(extension)) &&
        (generic || mime?.startsWith("text/") || ["application/json", "application/xml", "application/javascript", "application/x-sh", "application/toml", "application/yaml"].includes(mime!)))
        return "text";
    throw new Error("此格式暂不支持预览，请下载原文件。");
}
export function inspectPreview(file: FileSource, bytes: Uint8Array): Preview {
    if (bytes.length > PREVIEW_BYTES)
        throw new Error("预览限 50 MiB，请下载原文件。");
    const kind = previewKind(file);
    if (kind === "image") {
        const extension = file.name.toLowerCase().split(".").at(-1)!;
        const mime = imageMimes[extension]!;
        inspectImage(bytes, mime);
        return { kind, mime };
    }
    if (kind === "pdf") {
        if (bytes.length < 8 || String.fromCharCode(...bytes.subarray(0, 5)) !== "%PDF-" ||
            !/^\d\.\d$/.test(String.fromCharCode(...bytes.subarray(5, 8)))) {
            throw new Error("PDF 文件签名无效，请下载原文件。");
        }
        // Signature is a format gate, not PDF sanitization or proof of page rendering.
        return { kind, mime: "application/pdf" };
    }
    if ((bytes[0] === 0xff && bytes[1] === 0xfe && bytes[2] === 0 && bytes[3] === 0) ||
        (bytes[0] === 0 && bytes[1] === 0 && bytes[2] === 0xfe && bytes[3] === 0xff)) {
        throw new Error("不支持 UTF-32 文本编码，请下载原文件。");
    }
    const encoding = bytes[0] === 0xff && bytes[1] === 0xfe ? "utf-16le" :
        bytes[0] === 0xfe && bytes[1] === 0xff ? "utf-16be" : "utf-8";
    try {
        // Decode the original bytes once: TextDecoder removes only the first BOM.
        const text = new TextDecoder(encoding, { fatal: true }).decode(bytes);
        if (text.includes("\0"))
            throw new Error("NUL");
        return { kind, text };
    }
    catch {
        throw new Error("文本编码无效；仅支持 UTF-8 或带 BOM 的 UTF-16，请下载原文件。");
    }
}
