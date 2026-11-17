import type { EditorBlock, NoteEditorInstance } from "../schema";

// NoteDocument limits: docs/plans/knoted-v1/data-and-api.md (backend contract).
const MAX_BLOCKS = 5000;
const MAX_DEPTH = 8;

export function canInsertChoice(editor: NoteEditorInstance, query: string) {
  const block = editor.getTextCursorPosition().block;
  const text = block.content.map((inline) => inline.type === "link"
    ? inline.content.map((run) => run.text).join("") : inline.text).join("");
  // An otherwise empty current block is converted, even at the block limit.
  if (text === `/${query}`) return true;
  let count = 0;
  editor.forEachBlock(() => { count++; return true; });
  return count < MAX_BLOCKS;
}

function subtreeDepth(block: EditorBlock): number {
  return 1 + Math.max(0, ...block.children.map(subtreeDepth));
}

export function canMoveBlock(editor: NoteEditorInstance, id: string, direction: "up" | "down") {
  const block = editor.getBlock(id);
  if (!block) return false;
  const sibling = direction === "up" ? editor.getPrevBlock(id) : editor.getNextBlock(id);
  const parent = editor.getParentBlock(id);
  if (!sibling && !parent) return false;
  // Native move enters an adjacent sibling's children; at a sibling boundary
  // it unnests the block. Predict that exact destination, including descendants.
  let destinationParent = sibling
    ? sibling.children.length ? sibling : parent
    : parent && editor.getParentBlock(parent);
  let parentDepth = 0;
  while (destinationParent) {
    parentDepth++;
    destinationParent = editor.getParentBlock(destinationParent);
  }
  return parentDepth + subtreeDepth(block) <= MAX_DEPTH;
}
