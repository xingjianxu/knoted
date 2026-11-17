import { Button } from "@bixlab/mtone";
import { useEffect, useReducer } from "react";

import type { NoteEditorInstance } from "./schema";

const styles = [
  ["bold", "加粗"], ["italic", "斜体"], ["underline", "下划线"],
  ["strike", "删除线"], ["code", "行内代码"],
] as const;

export function FormattingToolbar({ editor }: { editor: NoteEditorInstance }) {
  const [, refresh] = useReducer((value: number) => value + 1, 0);
  useEffect(() => {
    const selection = editor.onSelectionChange(() => refresh());
    const change = editor.onChange(() => refresh());
    return () => { selection(); change(); };
  }, [editor]);
  const activeStyles = editor.getActiveStyles();
  function blockType(level?: 1 | 2 | 3) {
    const blocks = editor.getSelection()?.blocks ?? [editor.getTextCursorPosition().block];
    // Images, code and list blocks keep their semantics; T28 owns block menus.
    const targets = blocks.filter((block) => block.type === "paragraph" || block.type === "heading");
    for (const block of targets) {
      editor.updateBlock(block, level ? { type: "heading", props: { level } } : { type: "paragraph" });
    }
    editor.focus();
  }
  return (
    <div role="toolbar" aria-label="笔记文字格式" className="flex flex-wrap gap-1">
      <Button onMouseDown={(event) => event.preventDefault()} onClick={() => blockType()}>正文</Button>
      {([1, 2, 3] as const).map((level) => (
        <Button key={level} onMouseDown={(event) => event.preventDefault()} onClick={() => blockType(level)}>标题 {level}</Button>
      ))}
      {styles.map(([style, label]) => (
        <Button key={style} aria-pressed={!!activeStyles[style]} onMouseDown={(event) => event.preventDefault()} onClick={() => { editor.toggleStyles({ [style]: true }); editor.focus(); }}>{label}</Button>
      ))}
      <Button onMouseDown={(event) => event.preventDefault()} onClick={() => { editor.undo(); editor.focus(); }}>撤销</Button>
      <Button onMouseDown={(event) => event.preventDefault()} onClick={() => { editor.redo(); editor.focus(); }}>重做</Button>
    </div>
  );
}
