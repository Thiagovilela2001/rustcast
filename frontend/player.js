(() => {
  const playerShell = document.getElementById('playerShell');
  const onAirSign = document.getElementById('onAirSign');
  const audio = document.getElementById('audio');
  const playBtn = document.getElementById('playBtn');
  const iconPlay = document.getElementById('iconPlay');
  const iconPause = document.getElementById('iconPause');
  const muteBtn = document.getElementById('muteBtn');
  const volWave = document.getElementById('volWave');
  const volumeSlider = document.getElementById('volumeSlider');
  const shareBtn = document.getElementById('shareBtn');

  const stationPickerRow = document.getElementById('stationPickerRow');
  const stationSelect = document.getElementById('stationSelect');
  const listenersCount = document.getElementById('listenersCount');
  const coverImg = document.getElementById('coverImg');
  const coverFallback = document.getElementById('coverFallback');
  const studioArt = document.getElementById('studioArt');
  const artGrid = document.getElementById('artGrid');
  const trackTitle = document.getElementById('trackTitle');
  const trackArtist = document.getElementById('trackArtist');
  const playerStatus = document.getElementById('playerStatus');

  const historyCard = document.getElementById('historyCard');
  const historyList = document.getElementById('historyList');

  let currentStationId = 1;
  let currentMount = 'radio';
  let eventSource = null;
  let isPlaying = false;
  let playStartAt = null;
  let timerHandle = null;
  let lastTrackKey = '';
  const recentTracks = [];

  audio.volume = Number(volumeSlider.value);

  // ── Pixel-grid reveal (36 células) enquanto a capa carrega ──
  const CELLS = 36;
  for (let i = 0; i < CELLS; i++) {
    const c = document.createElement('span');
    c.className = 'art-cell';
    c.style.animationDelay = `${(i % 6) * 90 + Math.floor(i / 6) * 60}ms`;
    artGrid.appendChild(c);
  }

  function showFallbackCover() {
    coverImg.classList.add('hidden');
    coverImg.removeAttribute('src');
    coverFallback.classList.remove('hidden');
    studioArt.classList.add('is-empty');
  }
  function showCover(src) {
    if (coverImg.getAttribute('src') !== src) coverImg.src = src;
    coverFallback.classList.add('hidden');
    studioArt.classList.remove('is-empty');
  }

  coverImg.addEventListener('error', showFallbackCover);
  coverImg.addEventListener('load', () => {
    coverFallback.classList.add('hidden');
    coverImg.classList.remove('hidden');
    studioArt.classList.remove('is-empty');
  });

  // ── History ──
  function pushHistory(title, artist) {
    if (!title) return;
    const key = `${artist || ''}::${title}`;
    if (key === lastTrackKey) return;
    if (lastTrackKey) {
      const [prevArtist, prevTitle] = lastTrackKey.split('::');
      const now = new Date();
      const hhmm = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`;
      recentTracks.unshift({ title: prevTitle, artist: prevArtist, time: hhmm });
      if (recentTracks.length > 5) recentTracks.pop();
      renderHistory();
    }
    lastTrackKey = key;
  }

  function renderHistory() {
    if (recentTracks.length === 0) {
      historyCard.classList.add('hidden');
      return;
    }
    historyCard.classList.remove('hidden');
    historyList.innerHTML = '';
    for (const item of recentTracks) {
      const li = document.createElement('li');
      li.className = 'history-row';
      const label = item.artist ? `${item.artist} — ${item.title}` : item.title;
      li.innerHTML = `<span class="history-names"><span class="history-track"></span></span><span class="history-when mono">${item.time}</span>`;
      li.querySelector('.history-track').textContent = label;
      historyList.appendChild(li);
    }
  }

  // ── Now playing ──
  function renderNowPlaying(np) {
    if (!np) return;
    const title = np.title || '';
    const artist = np.artist || '';
    trackTitle.textContent = title;
    trackArtist.textContent = artist;
    listenersCount.textContent = Number.isFinite(np.listeners) ? String(np.listeners) : '0';

    if (np.title) pushHistory(np.title, np.artist);

    const isLive = np.source === 'live';
    playerShell.classList.toggle('is-live', isLive);

    if (np.art_url) showCover(np.art_url);
    else showFallbackCover();

    document.title = np.title ? `${np.title}${artist ? ' — ' + artist : ''}` : 'BLACKMESA Fm';
  }

  function connectSse(stationId) {
    if (eventSource) { eventSource.close(); eventSource = null; }
    eventSource = new EventSource(`/api/stations/${stationId}/nowplaying`);
    eventSource.onmessage = (ev) => {
      try { renderNowPlaying(JSON.parse(ev.data)); } catch (_) {}
    };
  }

  async function fetchInitialNowPlaying(stationId) {
    try {
      const res = await fetch(`/api/stations/${stationId}/nowplaying.json`);
      if (res.ok) renderNowPlaying(await res.json());
    } catch (_) {}
  }

  // ── Playback ──
  function setPlayingUi(playing) {
    isPlaying = playing;
    playerShell.classList.toggle('is-playing', playing);
    onAirSign.classList.toggle('active', playing || playerShell.classList.contains('is-live'));
    iconPlay.classList.toggle('hidden', playing);
    iconPause.classList.toggle('hidden', !playing);
    playBtn.setAttribute('aria-label', playing ? 'Pausar' : 'Ouvir ao vivo');
    if (playing && !playStartAt) {
      playStartAt = Date.now();
    } else if (!playing) {
      playStartAt = null;
      if (timerHandle) clearInterval(timerHandle);
      timerHandle = null;
    }
  }

  async function startStream() {
    playerStatus.textContent = '';
    audio.src = `/mount/${currentMount}?t=${Date.now()}`;
    try {
      await audio.play();
      setPlayingUi(true);
    } catch (_) {
      setPlayingUi(false);
    }
  }

  function stopStream() {
    audio.pause();
    audio.removeAttribute('src');
    audio.load();
    setPlayingUi(false);
  }

  playBtn.addEventListener('click', () => {
    if (isPlaying) stopStream();
    else startStream();
  });

  audio.addEventListener('playing', () => setPlayingUi(true));
  audio.addEventListener('error', () => {
    if (isPlaying) setPlayingUi(false);
  });

  volumeSlider.addEventListener('input', () => {
    audio.volume = Number(volumeSlider.value);
    if (audio.volume > 0 && audio.muted) audio.muted = false;
    volWave.style.opacity = audio.muted || audio.volume === 0 ? '0.25' : '1';
  });

  muteBtn.addEventListener('click', () => {
    audio.muted = !audio.muted;
    volWave.style.opacity = audio.muted ? '0.25' : '1';
    muteBtn.classList.toggle('is-muted', audio.muted);
  });

  shareBtn.addEventListener('click', async () => {
    const url = `${window.location.origin}/mount/${currentMount}`;
    try {
      await navigator.clipboard.writeText(url);
      shareBtn.classList.add('is-copied');
      setTimeout(() => shareBtn.classList.remove('is-copied'), 1600);
    } catch (_) {
      prompt('Link do stream:', url);
    }
  });

  // ── Stations ──
  async function loadStations() {
    try {
      const res = await fetch('/api/stations');
      if (!res.ok) return;
      const stations = await res.json();
      if (!Array.isArray(stations) || stations.length === 0) return;

      const defaultStation = stations.find((s) => s.id === 1) || stations[0];
      currentStationId = defaultStation.id;
      currentMount = defaultStation.mountpoint;
      audio.src = `/mount/${currentMount}`;

      if (stations.length > 1) {
        stationPickerRow.classList.remove('hidden');
        stationSelect.innerHTML = '';
        for (const s of stations) {
          const opt = document.createElement('option');
          opt.value = String(s.id);
          opt.textContent = s.name;
          opt.dataset.mount = s.mountpoint;
          if (s.id === currentStationId) opt.selected = true;
          stationSelect.appendChild(opt);
        }
      }
    } catch (_) {}
  }

  stationSelect.addEventListener('change', () => {
    const opt = stationSelect.selectedOptions[0];
    if (!opt) return;
    currentStationId = Number(opt.value);
    currentMount = opt.dataset.mount || 'radio';
    lastTrackKey = '';
    recentTracks.length = 0;
    renderHistory();
    fetchInitialNowPlaying(currentStationId);
    connectSse(currentStationId);
    if (isPlaying) startStream();
    else audio.src = `/mount/${currentMount}`;
  });

  (async () => {
    await loadStations();
    fetchInitialNowPlaying(currentStationId);
    connectSse(currentStationId);
  })();
})();