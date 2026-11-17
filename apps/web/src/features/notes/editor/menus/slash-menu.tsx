import { Button, Card, List, ListItem } from "@bixlab/mtone";
import { SuggestionMenu } from "@blocknote/core/extensions";
import { GenericPopover, useBlockNoteEditor, useExtension, useExtensionState, useSuggestionMenuKeyboardNavigation } from "@blocknote/react";
import { useCallback, useEffect, useId, useMemo } from "react";

import { noteEditorSchema } from "../schema";
import { blockChoices, insertChoice, type BlockChoice } from "./block-choices";
import { canInsertChoice } from "./command-limits";

function SlashChoices({ query }: { query: string }) {
  const editor = useBlockNoteEditor(noteEditorSchema);
  const menuId = useId();
  const suggestions = useExtension(SuggestionMenu);
  const items = useMemo(() => blockChoices.filter((item) => `${item.label} ${item.aliases}`.toLowerCase().includes(query.trim().toLowerCase())), [query]);
  const canInsert = canInsertChoice(editor, query);
  const choose = useCallback((choice: BlockChoice) => {
    if (!canInsertChoice(editor, query)) return;
    // Controller's default wrapper clears the query before invoking a command.
    // Group both public operations here so consumers see one document change.
    editor.transact((tr) => {
      const end = tr.selection.from;
      const start = end - query.length - 1;
      // Only consume the typed slash/query that opened this business menu.
      // The public clearQuery command dispatches its own Tiptap transaction.
      if (start < 0 || tr.doc.textBetween(start, end) !== `/${query}`) return;
      tr.delete(start, end);
      suggestions.closeMenu();
      insertChoice(editor, choice);
    });
    editor.focus();
  }, [editor, suggestions, query]);
  const { selectedIndex } = useSuggestionMenuKeyboardNavigation(editor, query, items, choose);
  useEffect(() => {
    if (items.length === 0) suggestions.closeMenu();
  }, [items.length, suggestions]);
  useEffect(() => {
    const editable = editor.domElement;
    editable?.setAttribute("aria-expanded", "true");
    editable?.setAttribute("aria-controls", menuId);
    if (selectedIndex !== undefined) {
      const optionId = `${menuId}-option-${selectedIndex}`;
      editable?.setAttribute("aria-activedescendant", optionId);
      editable?.ownerDocument.getElementById(optionId)?.scrollIntoView?.({ block: "nearest" });
    }
    return () => {
      editable?.setAttribute("aria-expanded", "false");
      editable?.removeAttribute("aria-controls");
      editable?.removeAttribute("aria-activedescendant");
    };
  }, [editor, selectedIndex, menuId]);
  return <Card className="min-w-48 max-h-[40vh] overflow-y-auto" size="small"><List id={menuId} role="listbox" aria-label="插入块">
    {items.map((item, index) => <ListItem key={item.key} role="presentation">
      <Button id={`${menuId}-option-${index}`} role="option" tabIndex={-1} disabled={!canInsert} aria-disabled={!canInsert}
        variant={index === selectedIndex ? "soft" : "ghost"} className="w-full justify-start" aria-selected={index === selectedIndex}
        onMouseDown={(event) => event.preventDefault()} onClick={() => choose(item)}>{item.label}</Button>
    </ListItem>)}
  </List></Card>;
}

/** Render inside NoteEditor children (BlockNote context). */
export function SlashMenu() {
  const editor = useBlockNoteEditor(noteEditorSchema);
  const suggestions = useExtension(SuggestionMenu);
  const state = useExtensionState(SuggestionMenu);
  useEffect(() => {
    suggestions.addSuggestionMenu({ triggerCharacter: "/", shouldOpen: () => editor.getTextCursorPosition().block.type !== "codeBlock" });
    return () => suggestions.removeSuggestionMenu("/");
  }, [editor, suggestions]);
  const open = !!state?.show && state.triggerCharacter === "/";
  const reference = useMemo(() => ({
    element: editor.domElement?.firstElementChild ?? undefined,
    getBoundingClientRect: () => state?.referencePos ?? new DOMRect(),
  }), [editor, state?.referencePos]);
  return <GenericPopover reference={reference}
    useFloatingOptions={{ open, placement: state && state.referencePos.y > window.innerHeight / 2 ? "top-start" : "bottom-start", onOpenChange: (value) => { if (!value) suggestions.closeMenu(); } }}
    focusManagerProps={{ disabled: true }}
    elementProps={{ onMouseDownCapture: (event) => event.preventDefault(), style: { zIndex: 80 } }}>
    {open && <SlashChoices query={state.query} />}
  </GenericPopover>;
}
