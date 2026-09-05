// The sources page (3.0.0: a file, because the kit's CSP forbids inline
// scripts and inline event handlers). Buttons declare what they do —
// data-reveal="<id>" / data-copy="<id>" — and one delegated listener
// dispatches; forms declare data-confirm / data-busy and armForms() wires
// them (see below).
async function fetchToken(id) {
  const r = await fetch(`/dashboard/sources/${encodeURIComponent(id)}/token`);
  if (!r.ok) { throw new Error('could not fetch the token'); }
  return (await r.json()).token;
}
async function reveal(id) {
  const row = document.getElementById(`out-${id}`);
  const pre = document.getElementById(`pre-${id}`);
  try {
    pre.textContent = await fetchToken(id);
    row.classList.remove('d-none');
    setTimeout(() => { pre.textContent = ''; row.classList.add('d-none'); }, 10000);
  } catch (e) { pre.textContent = e.message; row.classList.remove('d-none'); }
}
// Two things every form that acts on the world gets, from one place.
//
// A confirmation when it destroys something: these buttons sit in table
// rows next to each other, and the cost of a mis-click ranges from
// re-issuing a token to losing a calendar and every event on it.
//
// And a busy state while it runs: several of these are a round trip to
// Google, and a button that looks idle invites a second click. That is
// not merely untidy — a second click on "Make calendar" used to make a
// second calendar.
//
// Driven by attributes so a new button gets both by declaring them,
// rather than by remembering to wire up JavaScript:
//   data-confirm="…"   ask this before submitting
//   data-busy="…"      say this while waiting
function armForms() {
  document.querySelectorAll('form[data-confirm], form[data-busy]').forEach(function (form) {
    form.addEventListener('submit', function (event) {
      const question = form.dataset.confirm;
      if (question && !window.confirm(question)) {
        event.preventDefault();
        return;
      }
      const button = form.querySelector('button[type=submit]');
      if (!button) { return; }
      // Disabled AFTER the browser has read the button, so the form
      // still submits: a disabled submit button is not sent.
      window.setTimeout(function () { button.disabled = true; }, 0);
      const spinner = button.querySelector('.spinner-border');
      const label = button.querySelector('.label');
      if (spinner) { spinner.classList.remove('d-none'); }
      if (label && form.dataset.busy) { label.textContent = ' ' + form.dataset.busy; }
    });
  });
}
document.addEventListener('DOMContentLoaded', armForms);
function selectAll(node) {
  const range = document.createRange();
  range.selectNodeContents(node);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
}
// Copies without assuming the page is in a secure context.
//
// navigator.clipboard exists ONLY in a secure context — https, or
// localhost. This dashboard is served over plain HTTP on the LAN, which
// is neither, so the object is simply absent and the button used to die
// with "navigator.clipboard is undefined". It could never have worked in
// the only way this page is ever opened, and nothing said so: the error
// appeared in the browser console, not on the page.
//
// So: the modern API when it is really there, the deprecated but
// http-friendly execCommand next, and failing both, put the command on
// screen already selected so it can be copied by hand.
function copyText(text) {
  if (window.isSecureContext && navigator.clipboard) {
    return navigator.clipboard.writeText(text).then(() => true, () => false);
  }
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.setAttribute('readonly', '');
  ta.style.position = 'fixed';
  ta.style.opacity = '0';
  document.body.appendChild(ta);
  ta.select();
  let ok = false;
  try { ok = document.execCommand('copy'); } catch (e) { ok = false; }
  document.body.removeChild(ta);
  return Promise.resolve(ok);
}
async function copyCmd(id) {
  const pre = document.getElementById(`pre-${id}`);
  const row = document.getElementById(`out-${id}`);
  let hideAfter = 4000;
  try {
    const token = await fetchToken(id);
    const cmd = `curl -X POST ${location.origin}/v1/ingest/${id} \\\n  -H 'Authorization: Bearer ${token}' \\\n  -H 'Content-Type: application/json' \\\n  -d '{"title":"test","start":"2026-01-01T09:00:00+00:00"}'`;
    if (await copyText(cmd)) {
      pre.textContent = 'Command copied to the clipboard (token not shown).';
    } else {
      // The token is on screen now, so it gets the same treatment as
      // Reveal: visible long enough to use, then gone.
      pre.textContent = cmd;
      row.classList.remove('d-none');
      selectAll(pre);
      hideAfter = 20000;
    }
  } catch (e) { pre.textContent = e.message; }
  row.classList.remove('d-none');
  setTimeout(() => { pre.textContent = ''; row.classList.add('d-none'); }, hideAfter);
}

document.addEventListener('click', function (event) {
  const reveal_ = event.target.closest('[data-reveal]');
  if (reveal_) { reveal(reveal_.dataset.reveal); return; }
  const copy = event.target.closest('[data-copy]');
  if (copy) { copyCmd(copy.dataset.copy); }
});
