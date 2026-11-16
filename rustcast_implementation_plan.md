# RustCast — Plano de Implementação Arquitetural

Clone autocontido do **AzuraCast + Icecast + Liquidsoap** em **Rust puro**, compilado em um único binário sem dependências externas de runtime.

---

## 1. Visão Geral da Arquitetura

O sistema consolida três subsistemas em um único processo assíncrono e multithreaded:

1. **Relay Engine (Clone do Icecast):** Servidor HTTP/ICY de baixa latência para ingestão de streams (AutoDJ e DJs ao vivo) e distribuição via `broadcast` + *Ring Buffer*.
2. **Audio Engine / AutoDJ (Clone do Liquidsoap):** Pipeline DSP em tempo real (`44.1kHz / f32 stereo`) com decodificação universal, *resampling*, mixer de 2 decks (*crossfade*) e codificação MP3/Opus.
3. **Control Plane (Clone do AzuraCast):** API REST, Server-Sent Events (SSE) para *Now Playing*, gerenciamento de filas/playlists, autenticação de DJs e painel web embutido.

```text
                       [ Ouvintes (Browser / App) ]
                                    ▲
                                    │ HTTP Chunked + ICY Metadata
                                    ▼
┌───────────────────────────────────────────────────────────────────────┐
│                         BINÁRIO ÚNICO EM RUST                         │
│                                                                       │
│  ┌─────────────────────┐   Bytes MP3   ┌───────────────────────────┐  │
│  │  Relay Engine       │◄──────────────┤  Audio Engine (AutoDJ)    │  │
│  │  (Clone Icecast)    │               │  (Clone Liquidsoap)       │  │
│  │                     │◄──┐           │  • Symphonia (Decode)     │  │
│  │  • Ring Buffer      │   │ HTTP PUT  │  • Rubato (Resample 44.1k)│  │
│  │  • ICY Injector     │   │ /SOURCE   │  • Crossfade / Mixer f32  │  │
│  │  • Broadcast Hub    │   │           │  • LAME / Opus (Encode)   │  │
│  └──────────┬──────────┘   │           └─────────────┬─────────────┘  │
│             │              │                         ▲                │
│             │ Metadados    │                         │ Fila de Faixas │
│             ▼              │                         │                │
│  ┌─────────────────────────┴─────────────────────────┴─────────────┐  │
│  │                 Control Plane (Clone AzuraCast)                 │  │
│  │  • Axum (API REST + SSE/WebSockets + Painel Web)                │  │
│  │  • SQLx + SQLite (Estações, Playlists, Mídias, Contas DJ)       │  │
│  │  • Lofty (Extração de tags ID3/FLAC)                            │  │
│  └─────────────────────────────────────────────────────────────────┘  │
└───────────────────────────────────────────────────────────────────────┘
                                    ▲
                                    │ Transmissão ao Vivo
                       [ DJ Externo (Mixxx / BUTT) ]
```

---

## 2. Stack Tecnológica & Crates Principais

| Camada | Crate / Tecnologia | Responsabilidade |
| :--- | :--- | :--- |
| **Runtime & Rede** | `tokio`, `hyper`, `axum` | I/O assíncrono, rotas HTTP, SSE, interceptação de método `SOURCE` |
| **Banco de Dados** | `sqlx` (SQLite) | Persistência leve de estações, mídias, playlists, histórico e DJs |
| **Metadados** | `lofty` | Leitura de tags ID3v2, Vorbis, FLAC e extração de capas de álbuns |
| **Decodificação** | `symphonia` | Decodificação pura em Rust de MP3, FLAC, OGG, WAV e AAC |
| **DSP / Resampling** | `rubato` | Conversão de taxa de amostragem em tempo real para `44.1kHz` |
| **Codificação** | `mp3lame-encoder` / `audiopus` | Compressão do buffer PCM mixado para stream MP3 ou Opus |
| **Empacotamento** | `rust-embed` | Embutir assets do frontend (HTML/JS/CSS) no binário final |

---

## 3. Estrutura de Diretórios do Projeto

```text
rustcast/
├── Cargo.toml
├── migrations/
│   └── 0001_initial_schema.sql
├── storage/
│   └── media/                  # Arquivos de áudio enviados pelos usuários
├── frontend/                   # Interface Web (SPA leve ou HTMX + Tailwind)
└── src/
    ├── main.rs                 # Bootstrap do servidor Hyper/Axum e workers
    ├── config.rs               # Configurações globais e variáveis de ambiente
    ├── db/
    │   ├── mod.rs
    │   └── models.rs           # Structs Station, Media, Playlist, DjAccount
    ├── relay/
    │   ├── mod.rs
    │   ├── mountpoint.rs       # RingBuffer, Broadcast Hub e estado da fonte
    │   ├── icy.rs              # Injetor de metadados ICY (icy-metaint)
    │   └── source_handler.rs   # Suporte a PUT e método legado SOURCE (Mixxx/BUTT)
    ├── audio/
    │   ├── mod.rs
    │   ├── engine.rs           # Loop de clock em tempo real (pacing 26.12ms)
    │   ├── deck.rs             # Leitura via Symphonia + Resampling via Rubato
    │   ├── mixer.rs            # Crossfade f32 entre Deck A e Deck B
    │   └── encoder.rs          # Saída LAME MP3 / Opus
    └── api/
        ├── mod.rs
        ├── media.rs            # Upload multipart e indexação via Lofty
        ├── playlists.rs        # CRUD de playlists e rotação
        └── nowplaying.rs       # Endpoint SSE/WebSocket em tempo real
```

---

## 4. Roadmap de Implementação

### Fase 1: Relay Engine (Clone do Icecast)
- [ ] Criar a struct `MountPoint` gerenciada via `Arc<MountPoint>` contendo:
  - `ring_buffer`: `RwLock<VecDeque<Bytes>>` (~128 KB para *burst-on-connect*).
  - `tx`: `tokio::sync::broadcast::Sender<Bytes>` para distribuição live.
  - `metadata`: `Arc<RwLock<String>>` com o título da faixa atual.
- [ ] Implementar endpoint de ingestão `PUT /mount/:name` com validação `Basic Auth`.
- [ ] Implementar endpoint de consumo `GET /mount/:name` retornando stream contínuo (`Transfer-Encoding: chunked`).
- [ ] **Critério de Conclusão:** Transmitir um MP3 via `ffmpeg` para `PUT /mount/radio` e ouvir sem cortes no navegador via `GET /mount/radio`.

### Fase 2: Protocolo ICY & Compatibilidade com Softwares de DJ
- [ ] Interceptar conexões TCP no nível do `hyper::service::service_fn` para aceitar o método não-padrão `SOURCE /mount HTTP/1.0` utilizado por **Mixxx**, **BUTT** e **VirtualDJ**.
- [ ] Implementar parsing do header `Icy-MetaData: 1` nas requisições dos ouvintes.
- [ ] Criar wrapper de stream que injeta blocos `StreamTitle='Artista - Faixa';` a cada `16000` bytes (`icy-metaint: 16000`).
- [ ] **Critério de Conclusão:** Conectar o Mixxx diretamente no servidor Rust e visualizar o nome da música mudando em tempo real no VLC/player web.

### Fase 3: Audio Engine & AutoDJ (Clone do Liquidsoap)
- [ ] Criar thread dedicada (`std::thread::spawn`) com relógio monotônico (`std::time::Instant`) operando em **44.100 Hz, 2 canais, `f32`** (gerando 1152 amostras a cada `~26.12 ms`).
- [ ] Implementar `Deck` capaz de abrir arquivos via `symphonia` e normalizar qualquer taxa de amostragem para `44.1kHz` usando `rubato`.
- [ ] Implementar `Mixer` com dois decks (`Deck A` e `Deck B`) aplicando curva de ganho (*crossfade*) nos últimos *N* segundos de cada faixa.
- [ ] Implementar fallback de silêncio digital (`0.0f32`) quando a fila estiver vazia para evitar queda de conexão dos ouvintes.
- [ ] Codificar saída PCM com `mp3lame-encoder` e publicar diretamente no canal `tx` do `MountPoint`.
- [ ] **Critério de Conclusão:** Rádio tocando uma pasta de músicas variadas (MP3, FLAC, WAV) continuamente com transição suave entre faixas.

### Fase 4: Control Plane & Banco de Dados (Clone do AzuraCast)
- [ ] Configurar `SQLx` com SQLite e criar migrations para `stations`, `media`, `playlists`, `playlist_items` e `dj_accounts`.
- [ ] Criar rota `POST /api/stations/:id/media` para upload de arquivos via `Multipart`, extraindo duração, artista, título e capa via `lofty`.
- [ ] Implementar **Scheduler** em Rust para selecionar a próxima música baseado em pesos de playlist, inserção periódica de vinhetas (*jingles*) e regra anti-repetição de artista.
- [ ] **Critério de Conclusão:** Gerenciar catálogo de músicas e playlists via API REST refletindo imediatamente na fila do AutoDJ.

### Fase 5: Live DJ Handover & Interface Web
- [ ] Implementar máquina de estados no `MountPoint` (`StreamSource::AutoDJ` vs `StreamSource::LiveDJ`).
- [ ] Ao detectar conexão de DJ autenticado, aplicar *fade-out* rápido no AutoDJ, pausar consumo de faixas e chavear o `broadcast` para o stream ao vivo; reverter automaticamente na desconexão do DJ.
- [ ] Criar endpoint `GET /api/stations/:id/nowplaying` via **Server-Sent Events (SSE)** emitindo JSON com faixa atual, arte de capa e contagem de ouvintes (`tx.receiver_count()`).
- [ ] Embutir interface web de administração e player público no binário usando `rust-embed`.
- [ ] **Critério de Conclusão:** Compilar via `cargo build --release` gerando um executável único funcional pronto para deploy.

---

## 5. Modelagem Inicial do Banco de Dados (SQLite)

```sql
CREATE TABLE stations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    mountpoint TEXT NOT NULL UNIQUE,
    bitrate INTEGER NOT NULL DEFAULT 128,
    crossfade_secs REAL NOT NULL DEFAULT 4.0,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE media (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    station_id INTEGER NOT NULL REFERENCES stations(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    title TEXT NOT NULL,
    artist TEXT NOT NULL,
    album TEXT,
    duration_ms INTEGER NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE playlists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    station_id INTEGER NOT NULL REFERENCES stations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    weight INTEGER NOT NULL DEFAULT 10,
    is_jingle BOOLEAN NOT NULL DEFAULT 0,
    jingle_interval INTEGER DEFAULT 0
);

CREATE TABLE playlist_media (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (playlist_id, media_id)
);

CREATE TABLE dj_accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    station_id INTEGER NOT NULL REFERENCES stations(id) ON DELETE CASCADE,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT 1
);
```