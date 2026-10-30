// The Try page's JSON calls. A non-2xx answer rejects with its status; callers turn that into a state on the page.

/** @param {string} url @param {RequestInit} [init] */
export async function api(url, init) {
    const response = await fetch(url, { credentials: 'same-origin', ...init, headers: { 'Content-Type': 'application/json', ...init?.headers } });
    if (!response.ok) throw new Error(`${url} answered ${response.status}`);
    return response.json();
}
