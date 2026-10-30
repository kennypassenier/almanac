// What the status page remembers between reads.

/** `last` is the last journal answer, for a redraw by a toggle; `first` is true until a read has worked (a first failure fills the cards, a later one leaves what is shown). */
export const journal = { /** @type {any} */ last: null, first: true };
export const sources = { first: true };
