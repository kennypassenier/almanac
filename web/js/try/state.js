// What the Try page remembers: the sources and the one the page's picker has chosen.
export const state = {
    /** @type {any[]} */
    sources: [],
    profileSource: '',
    editorSource: '',
    /** True when the last read of the sources failed, so Retry reads them again instead of asking. */
    sourcesFailed: false,
    /** The last Try's answer and the call it was for, read by the Copy menus; null before the first. @type {{ data: any, payload: unknown } | null} */
    last: null,
    /** Why Try is disabled by the JSON field ('' when the field is valid). */
    fieldReason: '',
};
export const sourceById = (/** @type {string} */ id) => state.sources.find((s) => s.source_id === id) ?? null;
