export type CursorState = {
  readonly filterKey: string;
  readonly cursor: string | null;
  readonly previous: readonly (string | null)[];
};

export type CursorAction =
  | { type: "next"; nextCursor: string | null }
  | { type: "previous" }
  | { type: "filter"; filterKey: string };

export function createCursorState(filterKey = ""): CursorState {
  return { filterKey, cursor: null, previous: [] };
}

/** Keep API cursors opaque; only the application owns the previous-page stack. */
export function cursorReducer(state: CursorState, action: CursorAction): CursorState {
  switch (action.type) {
    case "next":
      return action.nextCursor === null ? state : {
        ...state,
        cursor: action.nextCursor,
        previous: [...state.previous, state.cursor],
      };
    case "previous": {
      if (state.previous.length === 0) return state;
      const cursor = state.previous[state.previous.length - 1]!;
      return { ...state, cursor, previous: state.previous.slice(0, -1) };
    }
    case "filter":
      return action.filterKey === state.filterKey ? state : createCursorState(action.filterKey);
  }
}
