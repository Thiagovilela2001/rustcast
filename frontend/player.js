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

  const stationNameEl = document.getElementById('stationName');
  const stationPickerRow = document.getElementById('stationPickerRow');
  const stationSelect = document.getElementById('stationSelect');
  const dialPill = document.getElementById('dialPill');

  const sourceBadge = document.getElementById('sourceBadge');
  const sourceText = document.getElementById('sourceText');
  const listenersCount = document.getElementById('listenersCount');
  const coverImg = document.getElementById('coverImg');
  const coverFallback = document.getElementById('coverFallback');
  const trackTitle = document.getElementById('trackTitle');
  const trackArtist = document.getElementById('trackArtist');
  const playerStatus = document.getElementById('playerStatus');
  const listenTimer = document.getElementById('listenTimer');

  const historyCard = document.getElementById('historyCard');
  const historyList = document.getElementById('historyList');
  const canvas = document.getElementById('visualizer');
  const ctx = canvas.getContext('2d');

  let currentStationId = 1;
  let currentMount = 'radio';
  let eventSource = null;
  let isPlaying = false;
  let playStartAt = null;
  let timerHandle = null;
  let lastTrackKey = '';
  const recentTracks = [];

  // Web Audio API visualizer state
  let audioCtx = null;
  let analyser = null;
  let freqData = null;
  let ambientPhase = 0;

  audio.volume = Number(volumeSlider.value);

  function initAudioVisualizer() {
    if (audioCtx) {
      if (audioCtx.state === 'suspended') audioCtx.resume().catch(() => {});
      return;
    }
    const Ctx = window.AudioContext || window.webkitAudioContext;
    if (!Ctx) return;
    try {
      audioCtx = new Ctx();
      const source = audioCtx.createMediaElementSource(audio);
      analyser = audioCtx.createAnalyser();
      analyser.fftSize = 64;
      analyser.smoothingTimeConstant = 0.78;
      source.connect(analyser);
      analyser.connect(audioCtx.destination);
      freqData = new Uint8Array(analyser.frequencyBinCount);
    } catch (_) {
      analyser = null;
    }
  }

  function resizeCanvas() {
    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = Math.max(1, Math.floor(rect.width * dpr));
    canvas.height = Math.max(1, Math.floor(rect.height * dpr));
  }
  window.addEventListener('resize', resizeCanvas);
  resizeCanvas();

  function drawVisualizer() {
    requestAnimationFrame(drawVisualizer);
    const w = canvas.width;
    const h = canvas.height;
    ctx.clearRect(0, 0, w, h);

    const bars = 28;
    const gap = Math.max(2, Math.floor(w * 0.008));
    const barW = Math.max(2, (w - gap * (bars - 1)) / bars);

    let hasLiveSignal = false;
    if (isPlaying && analyser && freqData) {
      analyser.getByteFrequencyData(freqData);
      for (let i = 0; i < freqData.length; i++) {
        if (freqData[i] > 2) {
          hasLiveSignal = true;
          break;
        }
      }
    }

    ambientPhase += isPlaying ? 0.09 : 0.03;

    for (let i = 0; i < bars; i++) {
      let norm = 0.08;
      if (hasLiveSignal) {
        const idx = Math.min(freqData.length - 1, Math.floor((i / bars) * (freqData.length * 0.78)));
        norm = Math.max(0.08, freqData[idx] / 255);
      } else if (isPlaying) {
        // Gentle reactive fallback if stream is buffering or silent
        norm = 0.14 + 0.25 * Math.abs(Math.sin(ambientPhase + i * 0.42));
      } else {
        norm = 0.06 + 0.05 * Math.abs(Math.sin(ambientPhase + i * 0.35));
      }

      const barH = Math.max(3, norm * (h - 4));
      const x = i * (barW + gap);
      const y = h - barH;

      const grad = ctx.createLinearGradient(0, y, 0, h);
      if (playerShell.classList.contains('is-live')) {
        grad.addColorStop(0, '#fca5a5');
        grad.addColorStop(1, '#ef4444');
      } else {
        grad.addColorStop(0, '#fdba74');
        grad.addColorStop(1, '#f97316');
      }

      ctx.fillStyle = isPlaying ? grad : 'rgba(148, 163, 184, 0.28)';
      ctx.beginPath();
      ctx.roundRect(x, y, barW, barH, 2);
      ctx.fill();
    }
  }
  requestAnimationFrame(drawVisualizer);

  function showFallbackCover() {
    coverImg.classList.add('hidden');
    coverImg.removeAttribute('src');
    coverFallback.classList.remove('hidden');
  }

  coverImg.addEventListener('error', showFallbackCover);
  coverImg.addEventListener('load', () => {
    coverFallback.classList.add('hidden');
    coverImg.classList.remove('hidden');
  });

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
      li.className = 'history-item';
      const label = item.artist ? `${item.artist} — ${item.title}` : item.title;
      li.innerHTML = `<span class="history-track"></span><span class="muted mono" style="font-size:0.76rem; flex-shrink:0;">${item.time}</span>`;
      li.querySelector('.history-track').textContent = label;
      historyList.appendChild(li);
    }
  }

  function renderNowPlaying(np) {
    if (!np) return;
    const title = np.title || '';
    const artist = np.artist || '';
    trackTitle.textContent = title;
    trackArtist.textContent = artist;
    listenersCount.textContent = Number.isFinite(np.listeners) ? np.listeners : 0;

    if (np.title) {
      pushHistory(np.title, np.artist);
    }

    const isLive = np.source === 'live';
    playerShell.classList.toggle('is-live', isLive);
    if (isLive) {
      sourceBadge.className = 'badge badge-live';
      sourceText.textContent = 'AO VIVO';
      onAirSign.classList.add('active');
    } else {
      sourceBadge.className = 'badge badge-autodj';
      sourceText.textContent = 'AutoDJ';
      onAirSign.classList.toggle('active', isPlaying);
    }

    if (np.art_url) {
      if (coverImg.getAttribute('src') !== np.art_url) {
        coverImg.src = np.art_url;
      }
    } else {
      showFallbackCover();
    }

    document.title = np.title ? `${np.title}${artist ? ' — ' + artist : ''}` : 'BLACKMESA Fm';
  }

  function connectSse(stationId) {
    if (eventSource) {
      eventSource.close();
      eventSource = null;
    }
    eventSource = new EventSource(`/api/stations/${stationId}/nowplaying`);

    eventSource.onopen = () => {
    };

    eventSource.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data);
        renderNowPlaying(data);
      } catch (_) {}
    };

    eventSource.onerror = () => {
    };
  }

  async function fetchInitialNowPlaying(stationId) {
    try {
      const res = await fetch(`/api/stations/${stationId}/nowplaying.json`);
      if (res.ok) {
        renderNowPlaying(await res.json());
      }
    } catch (_) {}
  }

  function updateTimer() {
    if (!playStartAt) {
      listenTimer.textContent = '00:00';
      return;
    }
    const sec = Math.floor((Date.now() - playStartAt) / 1000);
    const m = Math.floor(sec / 60);
    const s = sec % 60;
    listenTimer.textContent = `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  }

  function setPlayingUi(playing) {
    isPlaying = playing;
    playerShell.classList.toggle('is-playing', playing);
    onAirSign.classList.toggle('active', playing || playerShell.classList.contains('is-live'));
    iconPlay.classList.toggle('hidden', playing);
    iconPause.classList.toggle('hidden', !playing);
    playBtn.setAttribute('aria-label', playing ? 'Pausar stream' : 'Tocar stream');

    if (playing && !playStartAt) {
      playStartAt = Date.now();
      timerHandle = setInterval(updateTimer, 1000);
    } else if (!playing) {
      playStartAt = null;
      if (timerHandle) clearInterval(timerHandle);
      timerHandle = null;
      updateTimer();
    }
  }

  async function startStream() {
    initAudioVisualizer();
    playerStatus.textContent = '';
    audio.src = `/mount/${currentMount}?t=${Date.now()}`;
    try {
      await audio.play();
      setPlayingUi(true);
      playerStatus.textContent = '';
    } catch (_) {
      setPlayingUi(false);
      playerStatus.textContent = '';
    }
  }

  function stopStream() {
    audio.pause();
    audio.removeAttribute('src');
    audio.load();
    setPlayingUi(false);
    playerStatus.textContent = '';
  }

  playBtn.addEventListener('click', () => {
    if (isPlaying) stopStream();
    else startStream();
  });

  audio.addEventListener('playing', () => {
    setPlayingUi(true);
    playerStatus.textContent = '';
  });

  audio.addEventListener('waiting', () => {
    
  });

  audio.addEventListener('error', () => {
    if (isPlaying) {
      setPlayingUi(false);
      playerStatus.textContent = '';
    }
  });

  volumeSlider.addEventListener('input', () => {
    audio.volume = Number(volumeSlider.value);
    if (audio.volume > 0 && audio.muted) audio.muted = false;
    volWave.style.opacity = audio.muted || audio.volume === 0 ? '0.25' : '1';
  });

  muteBtn.addEventListener('click', () => {
    audio.muted = !audio.muted;
    volWave.style.opacity = audio.muted ? '0.25' : '1';
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

  async function loadStations() {
    try {
      const res = await fetch('/api/stations');
      if (!res.ok) return;
      const stations = await res.json();
      if (!Array.isArray(stations) || stations.length === 0) return;

      const defaultStation = stations.find((s) => s.id === 1) || stations[0];
      currentStationId = defaultStation.id;
      currentMount = defaultStation.mountpoint;
      stationNameEl.textContent = defaultStation.name;
      dialPill.textContent = `/${defaultStation.mountpoint} · ${defaultStation.bitrate}k`;
      audio.src = `/mount/${currentMount}`;

      if (stations.length > 1) {
        stationPickerRow.classList.remove('hidden');
        stationSelect.innerHTML = '';
        for (const s of stations) {
          const opt = document.createElement('option');
          opt.value = String(s.id);
          opt.textContent = `${s.name} (/${s.mountpoint})`;
          opt.dataset.mount = s.mountpoint;
          opt.dataset.bitrate = String(s.bitrate);
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
    stationNameEl.textContent = opt.textContent.split(' (/')[0];
    dialPill.textContent = `/${currentMount} · ${opt.dataset.bitrate || 128}k`;
    lastTrackKey = '';
    recentTracks.length = 0;
    renderHistory();
    fetchInitialNowPlaying(currentStationId);
    connectSse(currentStationId);
    if (isPlaying) {
      startStream();
    } else {
      audio.src = `/mount/${currentMount}`;
    }
  });

  (async () => {
    await loadStations();
    fetchInitialNowPlaying(currentStationId);
    connectSse(currentStationId);
  })();
})();
