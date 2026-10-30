// What the deliveries page knows: the last answer, whether a read is pending, and the schedule's shape.

export const SECOND = 1000;
export const MINUTE = 60 * SECOND;

/**
 * `data` is the last answer, or null before the first. `status` is 'loading' until the first answer, then 'ready' or
 * 'error'. `shape` is the schedule's view: 'runs' or 'gantt'.
 * @type {{ data: any, status: 'loading' | 'ready' | 'error', shape: 'runs' | 'gantt' }}
 */
export const state = { data: null, status: 'loading', shape: 'runs' };
