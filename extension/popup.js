const HELPER = 'http://localhost:8093';

const tabs = document.querySelectorAll('.tab');
const views = {
  compose: document.getElementById('view-compose'),
  recent: document.getElementById('view-recent'),
};
const form = document.getElementById('post-form');
const commentary = document.getElementById('commentary');
const visibility = document.getElementById('visibility');
const submit = document.getElementById('submit');
const composeStatus = document.getElementById('compose-status');
const recentStatus = document.getElementById('recent-status');
const recentList = document.getElementById('recent-list');

function setStatus(el, content, kind) {
  el.textContent = '';
  el.className = 'status' + (kind ? ' ' + kind : '');
  if (kind === 'success' && content && content.url) {
    el.appendChild(document.createTextNode('Posted: '));
    const a = document.createElement('a');
    a.href = content.url;
    a.target = '_blank';
    a.rel = 'noopener noreferrer';
    a.textContent = content.url;
    el.appendChild(a);
  } else {
    el.textContent = typeof content === 'string' ? content : '';
  }
}

function relTime(secs) {
  if (!secs) return '';
  const diff = Math.max(0, Math.floor(Date.now() / 1000 - secs));
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function switchTab(name) {
  tabs.forEach((t) => t.classList.toggle('active', t.dataset.tab === name));
  Object.entries(views).forEach(([k, v]) => v.classList.toggle('hidden', k !== name));
  if (name === 'recent') loadRecent();
}
tabs.forEach((t) => t.addEventListener('click', () => switchTab(t.dataset.tab)));

async function loadRecent() {
  setStatus(recentStatus, 'Loading…');
  recentList.innerHTML = '';
  try {
    const r = await fetch(HELPER + '/posts');
    const data = await r.json();
    if (!r.ok) throw new Error(data.error || 'helper returned ' + r.status);
    setStatus(recentStatus, '');
    renderRecent(data.posts || []);
  } catch (e) {
    setStatus(recentStatus, e.message || 'failed to load', 'error');
  }
}

function renderRecent(posts) {
  recentList.innerHTML = '';
  if (!posts.length) {
    const li = document.createElement('li');
    li.className = 'empty';
    li.textContent = 'No posts yet. Posts made via the extension show up here.';
    recentList.appendChild(li);
    return;
  }
  for (const p of posts) recentList.appendChild(renderItem(p));
}

function renderItem(post) {
  const li = document.createElement('li');
  li.className = 'recent-item';

  const meta = document.createElement('div');
  meta.className = 'meta';
  const ago = relTime(post.created_at);
  const edited = post.edited_at ? ` · edited ${relTime(post.edited_at)}` : '';
  meta.textContent = `${ago}${edited} · ${post.visibility}`;
  if (post.url) {
    meta.appendChild(document.createTextNode(' · '));
    const a = document.createElement('a');
    a.href = post.url;
    a.target = '_blank';
    a.rel = 'noopener noreferrer';
    a.textContent = 'open';
    meta.appendChild(a);
  }
  li.appendChild(meta);

  const text = document.createElement('div');
  text.className = 'text preview';
  text.textContent = post.commentary;
  li.appendChild(text);

  const actions = document.createElement('div');
  actions.className = 'recent-actions';
  const editBtn = document.createElement('button');
  editBtn.type = 'button';
  editBtn.className = 'btn-secondary';
  editBtn.textContent = 'Edit';
  const delBtn = document.createElement('button');
  delBtn.type = 'button';
  delBtn.className = 'btn-danger';
  delBtn.textContent = 'Delete';
  actions.appendChild(editBtn);
  actions.appendChild(delBtn);
  li.appendChild(actions);

  editBtn.addEventListener('click', () => enterEditMode(li, post));
  delBtn.addEventListener('click', () => handleDelete(li, post, delBtn));

  return li;
}

function enterEditMode(li, post) {
  const text = li.querySelector('.text');
  const actions = li.querySelector('.recent-actions');
  const ta = document.createElement('textarea');
  ta.rows = 6;
  ta.maxLength = 3000;
  ta.value = post.commentary;
  text.replaceWith(ta);
  actions.innerHTML = '';
  const saveBtn = document.createElement('button');
  saveBtn.type = 'button';
  saveBtn.textContent = 'Save';
  const cancelBtn = document.createElement('button');
  cancelBtn.type = 'button';
  cancelBtn.className = 'btn-secondary';
  cancelBtn.textContent = 'Cancel';
  actions.appendChild(saveBtn);
  actions.appendChild(cancelBtn);

  cancelBtn.addEventListener('click', () => loadRecent());
  saveBtn.addEventListener('click', async () => {
    const newText = ta.value.trim();
    if (!newText) return;
    saveBtn.disabled = true;
    saveBtn.textContent = 'Saving…';
    try {
      const r = await fetch(HELPER + '/posts/edit', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ urn: post.urn, commentary: newText }),
      });
      const data = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(data.error || 'helper returned ' + r.status);
      loadRecent();
    } catch (e) {
      setStatus(recentStatus, e.message || 'edit failed', 'error');
      saveBtn.disabled = false;
      saveBtn.textContent = 'Save';
    }
  });
}

let confirmTimeout = null;
async function handleDelete(li, post, btn) {
  if (!btn.classList.contains('confirming')) {
    btn.classList.add('confirming');
    btn.textContent = 'Confirm?';
    if (confirmTimeout) clearTimeout(confirmTimeout);
    confirmTimeout = setTimeout(() => {
      btn.classList.remove('confirming');
      btn.textContent = 'Delete';
    }, 3000);
    return;
  }
  btn.disabled = true;
  btn.textContent = 'Deleting…';
  try {
    const r = await fetch(HELPER + '/posts/delete', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ urn: post.urn }),
    });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.error || 'helper returned ' + r.status);
    loadRecent();
  } catch (e) {
    setStatus(recentStatus, e.message || 'delete failed', 'error');
    btn.disabled = false;
    btn.classList.remove('confirming');
    btn.textContent = 'Delete';
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
      setStatus(composeStatus, 'Helper has no valid token. Run `chrome2linkedin-helper auth` first.', 'error');
      submit.disabled = true;
    }
  } catch (e) {
    setStatus(composeStatus, 'Helper not reachable at ' + HELPER + '. Start chrome2linkedin-helper.', 'error');
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
  setStatus(composeStatus, 'Posting…');
  try {
    const r = await fetch(HELPER + '/post', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ commentary: text, visibility: visibility.value }),
    });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.error || 'helper returned ' + r.status);
    setStatus(composeStatus, { url: data.post_url }, 'success');
    commentary.value = '';
    await chrome.storage.local.remove('draft');
  } catch (err) {
    setStatus(composeStatus, err.message || 'Post failed', 'error');
  } finally {
    submit.disabled = false;
  }
});
