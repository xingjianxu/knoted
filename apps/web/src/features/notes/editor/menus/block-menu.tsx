import { Button, DropdownMenu } from "@bixlab/mtone";
import { SideMenuExtension } from "@blocknote/core/extensions";
import { SideMenuController, useBlockNoteEditor, useExtension, useExtensionState } from "@blocknote/react";
import { useState } from "react";

import { noteEditorSchema } from "../schema";
import { blockChoices } from "./block-choices";
import { canMoveBlock } from "./command-limits";

function BlockActions() {
  const editor = useBlockNoteEditor(noteEditorSchema);
  const sideMenu = useExtension(SideMenuExtension);
  const state = useExtensionState(SideMenuExtension);
  const [open, setOpen] = useState(false);
  const id = state?.block.id;
  function close() {
    setOpen(false);
    sideMenu.unfreezeMenu();
    // DropdownMenu owns popup keyboard/focus. Restore the editor after its
    // return-to-trigger lifecycle, without changing the ProseMirror selection.
    requestAnimationFrame(() => editor.focus());
  }
  if (!id) return null;
  function run(operation: () => void) {
    editor.transact(operation);
  }
  return <DropdownMenu open={open} onOpenChange={(next) => {
    if (next) { sideMenu.freezeMenu(); setOpen(true); }
    else close();
  }} trigger={<Button aria-label="块操作" onMouseDown={(event) => event.preventDefault()}>⋮</Button>}
    items={[
      ...blockChoices.map((choice) => ({ key: choice.key, label: `转换为${choice.label}`, onSelect: () => run(() => { editor.updateBlock(id, choice.block); }) })),
      { key: "up", label: "上移", disabled: !canMoveBlock(editor, id, "up"), onSelect: () => run(() => { if (canMoveBlock(editor, id, "up")) editor.moveBlocksUp(id); }) },
      { key: "down", label: "下移", disabled: !canMoveBlock(editor, id, "down"), onSelect: () => run(() => { if (canMoveBlock(editor, id, "down")) editor.moveBlocksDown(id); }) },
      { key: "delete", label: "删除块", danger: true, onSelect: () => run(() => { editor.removeBlocks([id]); }) },
    ]} />;
}

/** Hover trigger positioning/hit testing stays with the public core controller. */
export function BlockMenu() {
  return <SideMenuController sideMenu={BlockActions} />;
}
