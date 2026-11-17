import { Select } from "@bixlab/mtone";
import { useBlockNoteEditor } from "@blocknote/react";
import { useEffect, useReducer } from "react";

import { noteEditorSchema } from "../schema";

const languages = [
  { value: "", label: "纯文本" },
  { value: "javascript", label: "JavaScript" },
  { value: "typescript", label: "TypeScript" },
  { value: "python", label: "Python" },
  { value: "rust", label: "Rust" },
  { value: "json", label: "JSON" },
  { value: "sql", label: "SQL" },
  { value: "bash", label: "Bash" },
];

/** Language metadata only: code always stays editor text, never HTML. */
export function CodeBlockControls({ blockId }: { blockId?: string }) {
  const editor = useBlockNoteEditor(noteEditorSchema);
  const [, refresh] = useReducer((value: number) => value + 1, 0);
  useEffect(() => {
    const change = editor.onChange(() => refresh());
    const selection = editor.onSelectionChange(() => refresh());
    return () => { change(); selection(); };
  }, [editor]);
  const block = blockId ? editor.getBlock(blockId) : editor.getTextCursorPosition().block;
  if (block?.type !== "codeBlock") return null;
  const value = block.props.language;
  const options = languages.some((option) => option.value === value) ? languages
    : [...languages, { value, label: value }];
  return <Select label="代码语言" value={value} options={options} onValueChange={(language) => {
    if (language === null || editor.getBlock(block.id)?.type !== "codeBlock" || language === value) return;
    editor.updateBlock(block.id, { props: { language } });
    editor.focus();
  }} />;
}
