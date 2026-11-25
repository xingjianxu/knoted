import { act, fireEvent } from "@testing-library/react";

// jsdom dispatches input events but does not perform the contenteditable edit.
export async function typeSlash(editable: HTMLElement, paragraph: HTMLElement) {
  await act(async () => {
    editable.focus();
    const text = document.createTextNode(`${paragraph.textContent}/`);
    paragraph.replaceChildren(text);
    window.getSelection()!.collapse(text, text.length);
    fireEvent.input(paragraph, { inputType: "insertText", data: "/" });
  });
}
