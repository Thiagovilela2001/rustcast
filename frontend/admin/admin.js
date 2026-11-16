(() => {
  const STORAGE_KEY = 'rustcast_admin_auth';

  const loginView = document.getElementById('loginView');
  const appView = document.getElementById('appView');
  const loginForm = document.getElementById('loginForm');
  const loginPass = document.getElementById('loginPass');
  const loginError = document.getElementById('loginError');
  const logoutBtn = document.getElementById('logoutBtn');

  const stationSelect = document.getElementById('stationSelect');
  const toggleNewStationBtn = document.getElementById('toggleNewStationBtn');
  const closeNewStationBtn = document.getElementById('closeNewStationBtn');
  const newStationCard = document.getElementById('newStationCard');
  const newStationForm = document.getElementById('newStationForm');

  const npBadge = document.getElementById('npBadge');
  const npSource = document.getElementById('npSource');
  const npTrack = document.getElementById('npTrack');
  const npStationInfo = document.getElementById('npStationInfo');
  const npListeners = document.getElementById('npListeners');
  const skipTrackBtn = document.getElementById('skipTrackBtn');

  const dropzone = document.getElementById('dropzone');
  const mediaFileInput = document.getElementById('mediaFileInput');
  const uploadStatus = document.getElementById('uploadStatus');
  const mediaCount = document.getElementById('mediaCount');
  const mediaTableBody = document.getElementById('mediaTableBody');
  const refreshMediaBtn = document.getElementById('refreshMediaBtn');

  const playlistForm = document.getElementById('playlistForm');
  const playlistFormTitle = document.getElementById('playlistFormTitle');
  const cancelPlaylistEditBtn = document.getElementById('cancelPlaylistEditBtn');
  const plEditId = document.getElementById('plEditId');
  const plName = document.getElementById('plName');
  const plWeight = document.getElementById('plWeight');
  const plInterval = document.getElementById('plInterval');
  const plIsJingle = document.getElementById('plIsJingle');
  const plSubmitBtn = document.getElementById('plSubmitBtn');
  const playlistsTableBody = document.getElementById('playlistsTableBody');

  const playlistMediaTitle = document.getElementById('playlistMediaTitle');
  const playlistMediaEmpty = document.getElementById('playlistMediaEmpty');
  const playlistMediaPanel = document.getElementById('playlistMediaPanel');
  const addMediaToPlaylistForm = document.getElementById('addMediaToPlaylistForm');
  const addMediaSelect = document.getElementById('addMediaSelect');
  const playlistMediaBody = document.getElementById('playlistMediaBody');

  const djForm = document.getElementById('djForm');
  const djUser = document.getElementById('djUser');
  const djPass = document.getElementById('djPass');
  const djActive = document.getElementById('djActive');
  const djMountHint = document.getElementById('djMountHint');
  const djsTableBody = document.getElementById('djsTableBody');

  const stationsTableBody = document.getElementById('stationsTableBody');
  const toastStack = document.getElementById('toastStack');

  let authHeader = sessionStorage.getItem(STORAGE_KEY) || '';
  let stations = [];
  let currentStationId = null;
  let stationMedia = [];
  let stationPlaylists = [];
  let selectedPlaylistId = null;
  let npSourceSse = null;

  function toast(msg, type = 'ok') {
    const el = document.createElement('div');
    el.className = `toast ${type}`;
    el.textContent = msg;
    toastStack.appendChild(el);
    setTimeout(() => el.remove(), 4000);
  }

  function makeBasicAuth(user, pass) {
    const bytes = new TextEncoder().encode(`${user}:${pass}`);
    let bin = '';
    for (const b of bytes) bin += String.fromCharCode(b);
    return `Basic ${btoa(bin)}`;
  }

  function formatDuration(ms) {
    const totalSec = Math.max(0, Math.floor((ms || 0) / 1000));
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    return `${m}:${String(s).padStart(2, '0')}`;
  }

  function logout(msg) {
    fetch('/api/admin/logout', { method: 'POST' }).catch(() => {});
    authHeader = '';
    sessionStorage.removeItem(STORAGE_KEY);
    if (npSourceSse) {
      npSourceSse.close();
      npSourceSse = null;
    }
    appView.classList.add('hidden');
    loginView.classList.remove('hidden');
    if (msg) {
      loginError.textContent = msg;
      loginError.classList.remove('hidden');
    }
  }

  async function api(path, options = {}) {
    const headers = new Headers(options.headers || {});

    const res = await fetch(path, { ...options, headers });
    if (res.status === 401) {
      logout('Sessão expirada ou senha inválida.');
      throw new Error('Não autorizado (401)');
    }
    if (res.status === 204) return null;

    const text = await res.text();
    let data = null;
    if (text) {
      try { data = JSON.parse(text); } catch (_) { data = text; }
    }
    if (!res.ok) {
      const errMsg = (data && data.error) ? data.error : `Erro HTTP ${res.status}`;
      throw new Error(errMsg);
    }
    return data;
  }

  // ---------- Login ----------
  loginForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    loginError.classList.add('hidden');
    try {
      const res = await fetch('/api/admin/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ password: loginPass.value }),
      });
      if (res.status === 401) {
        loginError.textContent = 'Senha incorreta.';
        loginError.classList.remove('hidden');
        return;
      }
      if (!res.ok) {
        loginError.textContent = 'Falha ao entrar.';
        loginError.classList.remove('hidden');
        return;
      }
      authHeader = '';
      loginPass.value = '';
      await enterApp();
    } catch (err) {
      loginError.textContent = 'Falha ao conectar: ' + err.message;
      loginError.classList.remove('hidden');
    }
  });
      if (check.status === 401) {
        loginError.textContent = 'Senha incorreta.';
        loginError.classList.remove('hidden');
        return;
      }
      authHeader = candidate;
      sessionStorage.setItem(STORAGE_KEY, authHeader);
      loginPass.value = '';
      await enterApp();
    } catch (err) {
      loginError.textContent = `Falha ao conectar: ${err.message}`;
      loginError.classList.remove('hidden');
    }
  });

  logoutBtn.addEventListener('click', () => logout());

  // ---------- Tabs ----------
  document.querySelectorAll('.tab-btn').forEach((btn) => {
    btn.addEventListener('click', () => {
      document.querySelectorAll('.tab-btn').forEach((b) => b.classList.remove('active'));
      document.querySelectorAll('.tab-panel').forEach((p) => p.classList.add('hidden'));
      btn.classList.add('active');
      document.getElementById(`tab-${btn.dataset.tab}`).classList.remove('hidden');
    });
  });

  // ---------- Stations ----------
  toggleNewStationBtn.addEventListener('click', () => newStationCard.classList.toggle('hidden'));
  closeNewStationBtn.addEventListener('click', () => newStationCard.classList.add('hidden'));

  async function loadStations(selectId = null) {
    stations = await api('/api/stations');
    stationSelect.innerHTML = '';
    stationsTableBody.innerHTML = '';

    for (const s of stations) {
      const opt = document.createElement('option');
      opt.value = String(s.id);
      opt.textContent = `${s.name} (/${s.mountpoint})`;
      stationSelect.appendChild(opt);

      const tr = document.createElement('tr');
      tr.innerHTML = `
        <td class="mono">${s.id}</td>
        <td><strong>${escapeHtml(s.name)}</strong></td>
        <td class="mono">/mount/${escapeHtml(s.mountpoint)}</td>
        <td>${s.bitrate} kbps</td>
        <td>${s.crossfade_secs}s</td>
        <td><a class="btn btn-sm" href="/mount/${encodeURIComponent(s.mountpoint)}" target="_blank" rel="noopener">Ouvir</a></td>
      `;
      stationsTableBody.appendChild(tr);
    }

    if (stations.length > 0) {
      const target = selectId && stations.some((s) => s.id === selectId) ? selectId : (currentStationId && stations.some((s) => s.id === currentStationId) ? currentStationId : stations[0].id);
      stationSelect.value = String(target);
      await selectStation(target);
    }
  }

  stationSelect.addEventListener('change', () => {
    selectStation(Number(stationSelect.value));
  });

  async function selectStation(id) {
    currentStationId = id;
    selectedPlaylistId = null;
    resetPlaylistForm();
    playlistMediaPanel.classList.add('hidden');
    playlistMediaEmpty.classList.remove('hidden');

    const st = stations.find((s) => s.id === id);
    if (st) {
      npStationInfo.textContent = `/mount/${st.mountpoint} · ${st.bitrate} kbps · crossfade ${st.crossfade_secs}s`;
      djMountHint.textContent = `/mount/${st.mountpoint}`;
    }

    connectNowPlayingSse(id);
    await Promise.all([loadMedia(), loadPlaylists(), loadDjs()]);
  }

  newStationForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
      const created = await api('/api/stations', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: document.getElementById('stName').value.trim(),
          mountpoint: document.getElementById('stMount').value.trim(),
          bitrate: Number(document.getElementById('stBitrate').value) || 128,
          crossfade_secs: Number(document.getElementById('stCrossfade').value) || 4,
        }),
      });
      newStationForm.reset();
      newStationCard.classList.add('hidden');
      toast(`Estação "${created.name}" criada e AutoDJ iniciado!`);
      await loadStations(created.id);
    } catch (err) {
      toast(err.message, 'err');
    }
  });

  // ---------- Now Playing SSE ----------
  function connectNowPlayingSse(stationId) {
    if (npSourceSse) {
      npSourceSse.close();
      npSourceSse = null;
    }
    npSourceSse = new EventSource(`/api/stations/${stationId}/nowplaying`);
    npSourceSse.onmessage = (ev) => {
      try {
        const np = JSON.parse(ev.data);
        npTrack.textContent = `${np.artist || 'RustCast'} — ${np.title || 'Sem faixa'}`;
        npListeners.textContent = Number.isFinite(np.listeners) ? np.listeners : 0;
        if (np.source === 'live') {
          npBadge.className = 'badge badge-live';
          npSource.textContent = 'AO VIVO';
          skipTrackBtn.disabled = true;
        } else {
          npBadge.className = 'badge badge-autodj';
          npSource.textContent = 'AutoDJ';
          skipTrackBtn.disabled = false;
        }
      } catch (_) {}
    };
  }

  skipTrackBtn.addEventListener('click', async () => {
    if (!currentStationId) return;
    skipTrackBtn.disabled = true;
    try {
      await api(`/api/stations/${currentStationId}/skip`, { method: 'POST' });
      toast('Avançando para a próxima música...');
    } catch (err) {
      toast(err.message, 'err');
    } finally {
      skipTrackBtn.disabled = false;
    }
  });

  // ---------- Media ----------
  async function loadMedia() {
    if (!currentStationId) return;
    stationMedia = await api(`/api/stations/${currentStationId}/media`);
    mediaCount.textContent = String(stationMedia.length);
    mediaTableBody.innerHTML = '';

    if (stationMedia.length === 0) {
      mediaTableBody.innerHTML = '<tr><td colspan="6" class="muted">Nenhuma mídia cadastrada nesta estação.</td></tr>';
    } else {
      for (const m of stationMedia) {
        const tr = document.createElement('tr');
        tr.innerHTML = `
          <td>
            <img class="thumb" src="/api/media/${m.id}/art" alt="" onerror="this.style.visibility='hidden'">
          </td>
          <td><strong>${escapeHtml(m.title)}</strong></td>
          <td>${escapeHtml(m.artist)}</td>
          <td class="muted">${escapeHtml(m.album || '—')}</td>
          <td class="mono">${formatDuration(m.duration_ms)}</td>
          <td>
            <button class="btn btn-sm btn-danger" type="button" data-del-media="${m.id}">Excluir</button>
          </td>
        `;
        mediaTableBody.appendChild(tr);
      }
    }

    mediaTableBody.querySelectorAll('[data-del-media]').forEach((btn) => {
      btn.addEventListener('click', async () => {
        const id = Number(btn.dataset.delMedia);
        if (!confirm('Remover esta mídia?')) return;
        try {
          await api(`/api/media/${id}`, { method: 'DELETE' });
          toast('Mídia removida.');
          await loadMedia();
          if (selectedPlaylistId) await loadPlaylistMedia(selectedPlaylistId);
        } catch (err) {
          toast(err.message, 'err');
        }
      });
    });

    populateAddMediaSelect();
  }

  refreshMediaBtn.addEventListener('click', loadMedia);

  async function uploadFiles(files) {
    if (!currentStationId || !files || files.length === 0) return;
    let okCount = 0;
    for (let i = 0; i < files.length; i++) {
      const f = files[i];
      uploadStatus.textContent = `Enviando ${i + 1}/${files.length}: ${f.name}...`;
      const fd = new FormData();
      fd.append('file', f);
      try {
        await api(`/api/stations/${currentStationId}/media`, { method: 'POST', body: fd });
        okCount++;
      } catch (err) {
        toast(`Falha em ${f.name}: ${err.message}`, 'err');
      }
    }
    uploadStatus.textContent = okCount > 0 ? `${okCount} arquivo(s) enviado(s).` : '';
    if (okCount > 0) {
      toast(`${okCount} mídia(s) adicionada(s)!`);
      await loadMedia();
    }
    mediaFileInput.value = '';
  }

  dropzone.addEventListener('click', () => mediaFileInput.click());
  dropzone.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      mediaFileInput.click();
    }
  });
  mediaFileInput.addEventListener('change', () => uploadFiles(mediaFileInput.files));
  dropzone.addEventListener('dragover', (e) => {
    e.preventDefault();
    dropzone.classList.add('dragover');
  });
  dropzone.addEventListener('dragleave', () => dropzone.classList.remove('dragover'));
  dropzone.addEventListener('drop', (e) => {
    e.preventDefault();
    dropzone.classList.remove('dragover');
    uploadFiles(e.dataTransfer.files);
  });

  // ---------- Playlists ----------
  function resetPlaylistForm() {
    plEditId.value = '';
    plName.value = '';
    plWeight.value = '10';
    plInterval.value = '0';
    plIsJingle.checked = false;
    playlistFormTitle.textContent = 'Nova Playlist';
    plSubmitBtn.textContent = 'Criar Playlist';
    cancelPlaylistEditBtn.classList.add('hidden');
  }

  cancelPlaylistEditBtn.addEventListener('click', resetPlaylistForm);

  async function loadPlaylists() {
    if (!currentStationId) return;
    stationPlaylists = await api(`/api/stations/${currentStationId}/playlists`);
    playlistsTableBody.innerHTML = '';

    if (stationPlaylists.length === 0) {
      playlistsTableBody.innerHTML = '<tr><td colspan="4" class="muted">Nenhuma playlist. Sem playlists, o AutoDJ toca todo o catálogo.</td></tr>';
      return;
    }

    for (const p of stationPlaylists) {
      const tr = document.createElement('tr');
      const isSelected = p.id === selectedPlaylistId;
      if (isSelected) tr.style.background = 'rgba(249, 115, 22, 0.08)';
      const tipo = p.is_jingle
        ? `<span class="badge badge-live" style="background: var(--accent-soft); color: var(--accent-hover); border-color: rgba(249,115,22,0.35);">Vinheta (${p.jingle_interval || 0})</span>`
        : '<span class="badge badge-muted">Música</span>';

      tr.innerHTML = `
        <td><strong>${escapeHtml(p.name)}</strong></td>
        <td class="mono">${p.weight}</td>
        <td>${tipo}</td>
        <td class="row-actions">
          <button class="btn btn-sm" type="button" data-select-pl="${p.id}">Faixas</button>
          <button class="btn btn-sm" type="button" data-edit-pl="${p.id}">Editar</button>
          <button class="btn btn-sm btn-danger" type="button" data-del-pl="${p.id}">Excluir</button>
        </td>
      `;
      playlistsTableBody.appendChild(tr);
    }

    playlistsTableBody.querySelectorAll('[data-select-pl]').forEach((btn) => {
      btn.addEventListener('click', () => loadPlaylistMedia(Number(btn.dataset.selectPl)));
    });

    playlistsTableBody.querySelectorAll('[data-edit-pl]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const pl = stationPlaylists.find((x) => x.id === Number(btn.dataset.editPl));
        if (!pl) return;
        plEditId.value = String(pl.id);
        plName.value = pl.name;
        plWeight.value = String(pl.weight);
        plInterval.value = String(pl.jingle_interval || 0);
        plIsJingle.checked = Boolean(pl.is_jingle);
        playlistFormTitle.textContent = `Editando: ${pl.name}`;
        plSubmitBtn.textContent = 'Salvar Alterações';
        cancelPlaylistEditBtn.classList.remove('hidden');
      });
    });

    playlistsTableBody.querySelectorAll('[data-del-pl]').forEach((btn) => {
      btn.addEventListener('click', async () => {
        const id = Number(btn.dataset.delPl);
        if (!confirm('Excluir esta playlist?')) return;
        try {
          await api(`/api/playlists/${id}`, { method: 'DELETE' });
          toast('Playlist excluída.');
          if (selectedPlaylistId === id) {
            selectedPlaylistId = null;
            playlistMediaPanel.classList.add('hidden');
            playlistMediaEmpty.classList.remove('hidden');
          }
          await loadPlaylists();
        } catch (err) {
          toast(err.message, 'err');
        }
      });
    });
  }

  playlistForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const payload = {
      name: plName.value.trim(),
      weight: Number(plWeight.value) || 10,
      is_jingle: plIsJingle.checked,
      jingle_interval: Number(plInterval.value) || 0,
    };

    try {
      if (plEditId.value) {
        await api(`/api/playlists/${plEditId.value}`, {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        });
        toast('Playlist atualizada.');
      } else {
        await api(`/api/stations/${currentStationId}/playlists`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        });
        toast('Playlist criada.');
      }
      resetPlaylistForm();
      await loadPlaylists();
    } catch (err) {
      toast(err.message, 'err');
    }
  });

  function populateAddMediaSelect() {
    addMediaSelect.innerHTML = '';
    if (stationMedia.length === 0) {
      const opt = document.createElement('option');
      opt.value = '';
      opt.textContent = 'Envie mídias primeiro';
      addMediaSelect.appendChild(opt);
      return;
    }
    for (const m of stationMedia) {
      const opt = document.createElement('option');
      opt.value = String(m.id);
      opt.textContent = `${m.artist} — ${m.title}`;
      addMediaSelect.appendChild(opt);
    }
  }

  async function loadPlaylistMedia(playlistId) {
    selectedPlaylistId = playlistId;
    const pl = stationPlaylists.find((p) => p.id === playlistId);
    playlistMediaTitle.textContent = pl ? `Faixas: ${pl.name}` : 'Faixas da Playlist';
    playlistMediaEmpty.classList.add('hidden');
    playlistMediaPanel.classList.remove('hidden');
    populateAddMediaSelect();

    const items = await api(`/api/playlists/${playlistId}/media`);
    playlistMediaBody.innerHTML = '';
    if (items.length === 0) {
      playlistMediaBody.innerHTML = '<tr><td colspan="3" class="muted">Playlist vazia.</td></tr>';
    } else {
      for (const m of items) {
        const tr = document.createElement('tr');
        tr.innerHTML = `
          <td><strong>${escapeHtml(m.title)}</strong></td>
          <td>${escapeHtml(m.artist)}</td>
          <td><button class="btn btn-sm btn-danger" type="button" data-rm-pl-media="${m.id}">Remover</button></td>
        `;
        playlistMediaBody.appendChild(tr);
      }
    }

    playlistMediaBody.querySelectorAll('[data-rm-pl-media]').forEach((btn) => {
      btn.addEventListener('click', async () => {
        const mediaId = Number(btn.dataset.rmPlMedia);
        try {
          await api(`/api/playlists/${playlistId}/media/${mediaId}`, { method: 'DELETE' });
          toast('Faixa removida da playlist.');
          await loadPlaylistMedia(playlistId);
        } catch (err) {
          toast(err.message, 'err');
        }
      });
    });

    await loadPlaylists();
  }

  addMediaToPlaylistForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    if (!selectedPlaylistId || !addMediaSelect.value) return;
    try {
      await api(`/api/playlists/${selectedPlaylistId}/media`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ media_id: Number(addMediaSelect.value) }),
      });
      toast('Faixa adicionada à playlist.');
      await loadPlaylistMedia(selectedPlaylistId);
    } catch (err) {
      toast(err.message, 'err');
    }
  });

  // ---------- DJs ----------
  async function loadDjs() {
    if (!currentStationId) return;
    const djs = await api(`/api/stations/${currentStationId}/djs`);
    djsTableBody.innerHTML = '';

    if (djs.length === 0) {
      djsTableBody.innerHTML = '<tr><td colspan="3" class="muted">Nenhuma conta de DJ cadastrada.</td></tr>';
      return;
    }

    for (const dj of djs) {
      const tr = document.createElement('tr');
      const statusBadge = dj.is_active
        ? '<span class="badge badge-autodj">Ativo</span>'
        : '<span class="badge badge-muted">Inativo</span>';
      tr.innerHTML = `
        <td class="mono"><strong>${escapeHtml(dj.username)}</strong></td>
        <td>${statusBadge}</td>
        <td class="row-actions">
          <button class="btn btn-sm" type="button" data-toggle-dj="${dj.id}" data-active="${dj.is_active ? '1' : '0'}">
            ${dj.is_active ? 'Desativar' : 'Ativar'}
          </button>
          <button class="btn btn-sm" type="button" data-pass-dj="${dj.id}">Alterar Senha</button>
          <button class="btn btn-sm btn-danger" type="button" data-del-dj="${dj.id}">Excluir</button>
        </td>
      `;
      djsTableBody.appendChild(tr);
    }

    djsTableBody.querySelectorAll('[data-toggle-dj]').forEach((btn) => {
      btn.addEventListener('click', async () => {
        const id = Number(btn.dataset.toggleDj);
        const nextActive = btn.dataset.active !== '1';
        try {
          await api(`/api/djs/${id}`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ is_active: nextActive }),
          });
          toast(nextActive ? 'DJ ativado.' : 'DJ desativado.');
          await loadDjs();
        } catch (err) {
          toast(err.message, 'err');
        }
      });
    });

    djsTableBody.querySelectorAll('[data-pass-dj]').forEach((btn) => {
      btn.addEventListener('click', async () => {
        const id = Number(btn.dataset.passDj);
        const newPass = prompt('Nova senha (mínimo 8 caracteres):');
        if (!newPass) return;
        if (newPass.length < 8) {
          toast('A senha deve ter pelo menos 8 caracteres.', 'err');
          return;
        }
        try {
          await api(`/api/djs/${id}`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ password: newPass }),
          });
          toast('Senha do DJ atualizada.');
        } catch (err) {
          toast(err.message, 'err');
        }
      });
    });

    djsTableBody.querySelectorAll('[data-del-dj]').forEach((btn) => {
      btn.addEventListener('click', async () => {
        const id = Number(btn.dataset.delDj);
        if (!confirm('Excluir esta conta de DJ?')) return;
        try {
          await api(`/api/djs/${id}`, { method: 'DELETE' });
          toast('Conta de DJ excluída.');
          await loadDjs();
        } catch (err) {
          toast(err.message, 'err');
        }
      });
    });
  }

  djForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
      await api(`/api/stations/${currentStationId}/djs`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          username: djUser.value.trim(),
          password: djPass.value,
          is_active: djActive.checked,
        }),
      });
      djForm.reset();
      djActive.checked = true;
      toast('Conta de DJ criada.');
      await loadDjs();
    } catch (err) {
      toast(err.message, 'err');
    }
  });

  function escapeHtml(str) {
    return String(str ?? '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  async function enterApp() {
    loginView.classList.add('hidden');
    appView.classList.remove('hidden');
    await loadStations();
  }

  // Auto-enter if session token exists
  (async () => {
    if (authHeader) {
      try {
        await enterApp();
      } catch (_) {
        logout();
      }
    }
  })();
})();
