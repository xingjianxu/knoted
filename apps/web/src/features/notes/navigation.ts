import { createContext } from "react";

/** Notify the shell when its awaited navigation is cancelled by the notes blocker.
 * Router.navigate can remain pending after a blocked history transition. This
 * carries no draft, guard, or authentication state; the mounted blocker owns it.
 */
export const NotesNavigationBlocked = createContext<() => void>(() => {});
