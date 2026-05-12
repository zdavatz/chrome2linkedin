const HELPER = 'http://localhost:8093';

const form = document.getElementById('post-form');
const commentary = document.getElementById('commentary');
const visibility = document.getElementById('visibility');
const submit = document.getElementById('submit');
const statusEl = document.getElementById('status');

function setStatus(content, kind) {
  statusEl.textContent = '';
  statusEl.className = 'status' + (kind ? ' ' + kind : '');
  if (kind === 'success' && content && content.url) {
    statusEl.appendChild(document.createTextNode('Posted: '));
    const a = document.createElement('a');
    a.href = content.url;
    a.target = '_blank';
    a.rel = 'noopener noreferrer';
    a.textContent = content.url;
    statusEl.appendChild(a);
  } else {
    statusEl.textContent = typeof content === 'string' ? content : '';
  }
}

(async () => {
  const saved = await chrome.storage.local.get(['draft', 'visibility']);
  if (saved.draft) commentary.value = saved.draft;
  if (saved.visibility) visibility.value = saved.visibility;

  try {
    const r = await fetch(HELPER + '/status');
    if (!r.ok) throw new Error('helper returned ' + r.status);
    const data = await r.json();
    if (!data.has_token || !data.person_id_present) {
      setStatus('Helper has no valid token. Run `li_push --auth` first.', 'error');
      submit.disabled = true;
    }
  } catch (e) {
    setStatus('Helper not reachable at ' + HELPER + '. Start chrome2linkedin-helper.', 'error');
    submit.disabled = true;
  }
})();

commentary.addEventListener('input', () => {
  chrome.storage.local.set({ draft: commentary.value });
});
visibility.addEventListener('change', () => {
  chrome.storage.local.set({ visibility: visibility.value });
});

form.addEventListener('submit', async (e) => {
  e.preventDefault();
  const text = commentary.value.trim();
  if (!text) return;
  submit.disabled = true;
  setStatus('Posting…');
  try {
    const r = await fetch(HELPER + '/post', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ commentary: text, visibility: visibility.value }),
    });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.error || 'helper returned ' + r.status);
    setStatus({ url: data.post_url }, 'success');
    commentary.value = '';
    await chrome.storage.local.remove('draft');
  } catch (err) {
    setStatus(err.message || 'Post failed', 'error');
  } finally {
    submit.disabled = false;
  }
});
