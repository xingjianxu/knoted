import type { EditorPartialBlock, NoteEditorInstance } from "../schema";

// Image creation belongs to T29: the persisted image block requires a real
// uploaded asset ID. Never insert an empty/temporary image through this menu.
export const blockChoices = [
  { key: "paragraph", label: "正文", aliases: "paragraph text", block: { type: "paragraph" } },
  { key: "h1", label: "标题 1", aliases: "heading h1", block: { type: "heading", props: { level: 1 } } },
  { key: "h2", label: "标题 2", aliases: "heading h2", block: { type: "heading", props: { level: 2 } } },
  { key: "h3", label: "标题 3", aliases: "heading h3", block: { type: "heading", props: { level: 3 } } },
  { key: "bullet", label: "无序列表", aliases: "bullet list", block: { type: "bulletListItem" } },
  { key: "numbered", label: "有序列表", aliases: "numbered list", block: { type: "numberedListItem" } },
  { key: "check", label: "待办", aliases: "check todo list", block: { type: "checkListItem" } },
  { key: "code", label: "代码块", aliases: "code", block: { type: "codeBlock" } },
] satisfies { key: string; label: string; aliases: string; block: EditorPartialBlock }[];
export type BlockChoice = (typeof blockChoices)[number];

export function insertChoice(editor: NoteEditorInstance, choice: BlockChoice) {
  const current = editor.getTextCursorPosition().block;
  const target = current.content.length === 0
    ? editor.updateBlock(current, choice.block)
    : editor.insertBlocks([choice.block], current, "after")[0];
  editor.setTextCursorPosition(target!, "start");
}
