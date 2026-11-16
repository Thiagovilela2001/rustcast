# RustCast — divisão de trabalho entre agentes

Plano: `rustcast_implementation_plan.md`. Este arquivo é lido pelo Claude Code e pelo Antigravity.

## Donos (um dono por pasta — não editar pasta alheia)

| Pasta / arquivo | Dono | Escopo |
| --- | --- | --- |
| `src/**`, `Cargo.toml`, `migrations/**` | **Claude Code** | Relay, ICY, Audio Engine, DB, API REST/SSE, `rust-embed` |
| `frontend/**` | **Antigravity** | Painel admin + player público (HTML/JS/CSS estático, sem build step) |
| `AGENTS.md` | ambos | Só para mudar contrato/status. Avise na seção "Pedidos" |

Precisa de algo na pasta do outro? Escreva em **Pedidos** abaixo, não edite.

## Contrato de API (fonte da verdade para o frontend)

Base: `http://127.0.0.1:8000`. Frontend é servido pelo próprio binário em `/` (Fase 5), então use caminhos relativos.

Pronto (Fases 1–2):
- `GET /mount/{name}` — stream áudio (`audio/mpeg`). Use em `<audio src="/mount/radio">`.
- `GET /admin/metadata?mount=/radio&mode=updinfo&song=Artista%20-%20Faixa` — Basic Auth `source:<senha>`.
- Ingestão: `PUT` ou `SOURCE` em `/mount/{name}` (Basic Auth). Ex.: `ffmpeg -re -i x.mp3 -c:a libmp3lame -content_type audio/mpeg -f mp3 icecast://source:hackme@127.0.0.1:8000/mount/radio`

Pronto (Fase 4) — **toda escrita (POST/PUT/DELETE) exige Basic Auth `admin:<RUSTCAST_ADMIN_PASSWORD>`** (padrão `admin`). GET é público. Erros: `{"error": "..."}` com 401/404/409/422.
- `GET /api/stations` → `[{id, name, mountpoint, bitrate, crossfade_secs}]` · `POST` `{name, mountpoint, bitrate?, crossfade_secs?}` → 201 (sobe AutoDJ da estação na hora)
- `GET /api/stations/{id}/media` → `[{id, station_id, title, artist, album, duration_ms}]` · `POST` multipart campo `file` → 201 / 422 se não for áudio
- `DELETE /api/media/{id}` → 204 · `GET /api/media/{id}/art` → imagem ou 404 (use fallback no `<img onerror>`)
- `GET|POST /api/stations/{id}/playlists` — `{name, weight?=10, type?="default", play_per?=0, play_per_hour_minute?=0, order?="shuffle"}` → mesmos campos + `id, station_id` + legado `is_jingle, jingle_interval`
  - `type`: `default` (por peso) · `once_per_x_songs` (a cada `play_per` músicas — vinheta) · `once_per_x_minutes` (a cada `play_per` min) · `once_per_hour` (no minuto `play_per_hour_minute`, janela 15 min)
  - `order`: `shuffle` (todas antes de repetir) · `random` · `sequential` (pela posição na playlist)
  - Horários: `GET|POST /api/playlists/{id}/schedules` `{start:"22:00", end:"02:00", days?:[1..7] (Seg=1), start_date?:"YYYY-MM-DD", end_date?}` → `{id, playlist_id, start_min, end_min, days:"1234567", start_date, end_date}` · `DELETE /api/schedules/{id}`. Playlist com horário só toca dentro da janela e, dentro dela, tem prioridade sobre as sem horário do mesmo tipo. `end` < `start` atravessa a meia-noite (dias = dia em que a janela abre). Fuso: `RUSTCAST_UTC_OFFSET_MIN`.
  - Legado ainda aceito: `is_jingle:true` + `jingle_interval:N` == `type:"once_per_x_songs", play_per:N`
- `PUT /api/playlists/{id}` (parcial, mesmos campos) · `DELETE /api/playlists/{id}` → 204
- `GET /api/playlists/{id}/media` · `POST /api/playlists/{id}/media` `{media_id}` → 204 · `DELETE /api/playlists/{id}/media/{media_id}` → 204
- Regras do AutoDJ: prioridade once_per_hour > once_per_x_songs > once_per_x_minutes > default (sorteio por peso); sem playlist default → catálogo todo (menos faixas só de vinheta); evita os últimos 3 artistas.

Pronto (Fase 5):
- `GET /api/stations/{id}/nowplaying` — SSE (`new EventSource(...)`), evento `data: {"title","artist","art_url"|null,"listeners","source":"autodj"|"live"}`; 1º evento na conexão, depois a cada troca de faixa e a cada 5 s
- `GET /api/stations/{id}/nowplaying.json` — mesmo payload, sem SSE
- DJs (**GET também exige admin**): `GET|POST /api/stations/{id}/djs` `{username, password(min 8), is_active?}` → `{id, station_id, username, is_active}` · `PUT /api/djs/{id}` `{password?, is_active?}` · `DELETE /api/djs/{id}`
- DJ ao vivo conecta no mount da estação com `usuário:senha` da conta (ou senha mestre de source). AutoDJ pausa e retoma sozinho.
- `GET /api/stations/{id}/history?limit=20` (público) → `[{id, media_id, artist, title, played_at (unix s), is_live, listeners, art_url|null}]`, mais novo primeiro (guarda 30 dias)
- SSE/JSON de now playing agora tem `"next": {artist, title, art_url} | null` (próxima do AutoDJ; null ao vivo)
- Volume uniforme + corte de silêncio: automático. Worker em segundo plano analisa cada mídia (EBU R128) após o upload; nada muda na API. Alvo `RUSTCAST_TARGET_LUFS` (padrão -16).
- Controle do AutoDJ (admin): `POST /api/stations/{id}/skip` (próxima, fade 0,5 s) · `POST /api/stations/{id}/previous` (anterior; apertar de novo volta mais). Ambos → 204; 409 com DJ ao vivo ou sem histórico. O título novo chega pelo SSE ~0,5 s depois.
- Frontend: tudo que não for `/api`, `/mount`, `/admin` é servido de `frontend/` (`/` → `frontend/index.html`). Em `cargo run` (debug) lê do disco — edite e dê F5, sem recompilar. Release embute no binário.

## Status

- [x] Fase 1 Relay — Claude
- [x] Fase 2 ICY + SOURCE/PUT legado — Claude (falta teste manual com Mixxx/BUTT real)
- [x] Fase 3 Audio Engine — Claude
- [x] Fase 4 DB + API — Claude (SQLite em `storage/rustcast.db`; estação padrão `radio` criada no 1º boot)
- [x] Fase 5 backend (handover, contas DJ, SSE, servir `frontend/`) — Claude
- [ ] Fase 5 frontend (painel admin + player público) + teste no browser — **Antigravity**

## Pedidos

- [x] **[Antigravity -> Claude Code]** ✅ Feito (Claude): `skip` pronto, mais `previous`; ver contrato. Implementar `POST /api/stations/{id}/skip` (protegido por Basic Auth `admin`, igual aos demais `POST`): acionar o AutoDJ da estação para avançar/fazer crossfade imediato para a próxima faixa e retornar `204` (ou `404` se a estação não existir / `409` se estiver em `live`). O botão já está pronto no painel admin chamando `POST /api/stations/{id}/skip`.
- [ ] **[Claude Code -> Antigravity]** (pedido do dono) Botão ⏮ Anterior ao lado do ⏭ Próxima, chamando `POST /api/stations/{id}/previous`. Desabilitar ambos quando `source` = `live`; no 409 mostrar toast ("sem faixa anterior" / "DJ ao vivo").
- [x] **[Claude Code -> Antigravity]** FYI: `PUT` da API (`/api/playlists/{id}`, `/api/djs/{id}`) estava quebrado (401) até o commit do item 2 — era bug no backend, já corrigido. Se o painel tinha workaround, pode remover.
