import { Alert } from "@bixlab/mtone";
import { BlockEditor, type BlockEditorBlock } from "@bixlab/mtone-blockeditor";
import type { ApiClient } from "@knoted/api-client";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { fromEditorBlocks, toEditorBlocks, type NoteDocument } from "./document-adapter";
import { uploadNoteImage } from "./image-upload";

export type NoteEditorProps = {
  /** Change to load a different document or reset epoch. */
  documentKey?: string | number;
  /** Initial value only. Key the component or change documentKey to open another note. */
  document: NoteDocument;
  onChange: (document: NoteDocument) => void;
  readOnly?: boolean;
  /** Optional API client enabling image uploads via BlockEditor's built-in image dialog. */
  client?: ApiClient;
  className?: string;
};

export function NoteEditor({
  documentKey,
  document,
  onChange,
  readOnly = false,
  client,
  className,
}: NoteEditorProps) {
  // Deliberately independent of props while documentKey is unchanged: parent
  // save/query rerenders must never replace the editor view, selection or IME.
  const [seed, setSeed] = useState(() => ({ key: documentKey, document }));
  const [inputError, setInputError] = useState("");
  if (!Object.is(seed.key, documentKey)) {
    setSeed({ key: documentKey, document });
    if (inputError) setInputError("");
  }
  const initialDocument = seed.document;
  const lifetime = useRef<AbortController | null>(null);

  useEffect(() => {
    if (readOnly) {
      lifetime.current?.abort();
      lifetime.current = null;
      return;
    }
    const controller = new AbortController();
    lifetime.current = controller;
    return () => {
      controller.abort();
      if (lifetime.current === controller) lifetime.current = null;
    };
  }, [documentKey, readOnly]);

  const initialContent = useMemo(
    () => (initialDocument.blocks.length ? toEditorBlocks(initialDocument) : undefined),
    [initialDocument],
  );

  const handleUploadImage = useMemo(() => {
    if (!client || readOnly) return undefined;
    return async (file: File) => {
      const signal = lifetime.current?.signal;
      try {
        const url = await uploadNoteImage(client, file, signal);
        if (!signal?.aborted) setInputError("");
        return url;
      } catch (cause) {
        if (!signal?.aborted) {
          setInputError(cause instanceof Error ? cause.message : "图片上传失败，请重试。");
        }
        throw cause;
      }
    };
  }, [client, readOnly]);

  const handleChange = useCallback(
    (blocks: BlockEditorBlock[]) => {
      if (readOnly) return;
      try {
        const next = fromEditorBlocks(blocks, initialDocument);
        setInputError("");
        onChange(next);
      } catch (cause) {
        const detail = cause instanceof Error
          ? cause.message.replace(/。+$/, "")
          : "内容含有无法保存的链接或格式";
        setInputError(
          `未应用此次编辑：${detail}，原内容已保留。请修正或撤销后重试；也可仅以纯文本粘贴链接名称文字。`,
        );
      }
    },
    [initialDocument, onChange, readOnly],
  );

  return (
    <section aria-label="笔记编辑器" className="space-y-2">
      {inputError && <Alert tone="warning">{inputError}</Alert>}
      <BlockEditor
        documentKey={documentKey}
        initialContent={initialContent}
        onChange={handleChange}
        readOnly={readOnly}
        label="笔记编辑器"
        locale="zh"
        className={className}
        uploadImage={handleUploadImage}
      />
    </section>
  );
}
