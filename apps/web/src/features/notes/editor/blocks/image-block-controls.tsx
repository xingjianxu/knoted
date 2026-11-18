import { Upload, type UploadAdapter } from "@bixlab/mtone";
import { useBlockNoteEditor } from "@blocknote/react";
import type { ApiClient } from "@knoted/api-client";
import { useCallback, useEffect, useRef } from "react";

import { noteEditorSchema } from "../schema";

type UploadIntent = {
  key: string;
  targetId: string;
  type: string;
  assetId?: string;
  valid: boolean;
  controller: AbortController;
  unsubscribe: () => void;
};

function invalidateIntent(intent: UploadIntent) {
  intent.valid = false;
  intent.controller.abort();
  intent.unsubscribe();
}

export type ImageBlockControlsProps = {
  client: ApiClient;
  /** Replace this image; otherwise capture the cursor block when choosing a file. */
  blockId?: string;
};

/** Compose inside NoteEditor, whose note-ID key owns this upload lifecycle. */
export function ImageBlockControls({ client, blockId }: ImageBlockControlsProps) {
  const editor = useBlockNoteEditor(noteEditorSchema);
  const intents = useRef(new WeakMap<File, UploadIntent>());
  const pending = useRef(new Set<UploadIntent>());
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    lifetime.current = new AbortController();
    const active = pending.current;
    return () => {
      lifetime.current.abort();
      for (const intent of active) {
        invalidateIntent(intent);
      }
      active.clear();
    };
  }, [editor, blockId]);

  const upload = useCallback<UploadAdapter>(async (file, { signal }) => {
    let intent = intents.current.get(file);
    if (!intent) {
      const block = blockId ? editor.getBlock(blockId) : editor.getTextCursorPosition().block;
      if (!block || (blockId && block.type !== "image")) throw new Error("图片目标已失效，请重新选择文件");
      // A newer explicit choice for this block takes precedence, regardless
      // of response order. A retry of the same File keeps its original intent.
      for (const previous of pending.current) {
        if (previous.targetId === block.id) {
          invalidateIntent(previous);
          pending.current.delete(previous);
        }
      }
      intent = { key: crypto.randomUUID(), targetId: block.id, type: block.type,
        assetId: block.props.assetId, valid: true, controller: new AbortController(), unsubscribe: () => {} };
      const captured = intent;
      captured.unsubscribe = editor.onChange(() => {
        const current = editor.getBlock(captured.targetId);
        if (!current || current.type !== captured.type || current.props.assetId !== captured.assetId) {
          captured.valid = false;
          captured.controller.abort();
        }
      });
      intents.current.set(file, captured);
      pending.current.add(captured);
    }
    const validTarget = () => {
      const block = editor.getBlock(intent.targetId);
      if (!intent.valid || !block || block.type !== intent.type || block.props.assetId !== intent.assetId) return false;
      if (intent.type === "image") return true;
      let count = 0;
      editor.forEachBlock(() => { count++; return true; });
      // Insertion is a sibling with no children, so it cannot deepen a tree.
      return count < 5000;
    };
    if (!validTarget()) throw new Error("图片目标已失效或笔记块已达上限，请重新选择文件");
    intent.controller = new AbortController();
    const requestSignal = AbortSignal.any([signal, lifetime.current.signal, intent.controller.signal]);
    const form = new FormData();
    form.append("file", file);
    const { data } = await client.raw.POST("/assets", {
      params: { header: { "Idempotency-Key": intent.key } },
      // Generated binary fields are strings. The serializer sends the actual
      // File as multipart; fetch owns Content-Type including its boundary.
      body: { file: file.name },
      bodySerializer: () => form,
      signal: requestSignal,
    });
    if (requestSignal.aborted || !validTarget()) throw new Error("图片上传已取消或目标已失效");
    if (data?.state === "deleted") throw new Error("图片已回收，请重新选择文件");
    if (!data || data.state !== "ready" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(data.id)) throw new Error("图片资产响应无效");
    // Stop watching before our own single atomic document change.
    intent.unsubscribe();
    pending.current.delete(intent);
    intent.valid = false;
    if (intent.type === "image") editor.updateBlock(intent.targetId, { props: { assetId: data.id } });
    else editor.insertBlocks([{ type: "image", props: { assetId: data.id } }], intent.targetId, "after");
    return data;
  }, [editor, client, blockId]);

  return <div role="group" aria-label="笔记图片上传"><Upload key={blockId ?? "cursor"} upload={upload} multiple={false} accept="image/png,image/jpeg,image/webp">选择图片；上传完成后插入或替换原图片</Upload></div>;
}
