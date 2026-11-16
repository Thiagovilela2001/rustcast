<div align="center">

# RustCast

### Rádio online self-hosted, leve e escrita em Rust.

**AutoDJ · transmissão ao vivo · painel administrativo · player público · SQLite**

[![Rust](https://img.shields.io/badge/Rust-2024-c7ff45?style=flat-square&logo=rust&logoColor=11150e)](https://www.rust-lang.org/)
[![Axum](https://img.shields.io/badge/Axum-0.8-c7ff45?style=flat-square)](https://github.com/tokio-rs/axum)
[![License](https://img.shields.io/badge/license-não%20definida-667068?style=flat-square)](#licença)

</div>

---

RustCast é um servidor de rádio via internet compacto, criado para substituir instalações pesadas em servidores pequenos. Um único processo cuida da biblioteca, programação automática, ingestão ao vivo, streaming, metadados e interface web.

O frontend não exige Node.js nem etapa de build: em desenvolvimento ele é lido diretamente de `frontend/`; na versão release, é incorporado ao binário.

> **Status:** projeto em desenvolvimento ativo. O núcleo de transmissão, AutoDJ, API, painel e player já estão funcionais.

## Destaques

- **AutoDJ nativo** com playlists ponderadas, vinhetas e crossfade.
- **Programação por horário** para controlar quando cada playlist pode tocar.
- **Normalização EBU R128** com volume-alvo configurável.
- **Entrada ao vivo compatível com Icecast**, usando `SOURCE` ou `PUT`.
- **Handover automático:** o AutoDJ pausa quando um DJ entra e retoma ao final da transmissão.
- **Contas individuais de DJ** com senhas protegidas por Argon2.
- **Metadados ICY** e atualização pelo endpoint legado `updinfo`.
- **Painel administrativo** para estações, mídias, playlists e DJs.
- **Player público responsivo** com capa, ouvintes e atualização em tempo real via SSE.
- **SQLite**, migrations automáticas e armazenamento local de mídia.
- **Endpoints compatíveis com AzuraCast** para facilitar integrações existentes.
- **Binário pequeno e econômico**, adequado para VPS com poucos recursos.

## Visão geral

```text
DJ / encoder ── SOURCE ou PUT ──┐
                               ▼
Biblioteca ── AutoDJ ──────► RustCast ──► ouvintes
   │                           │
   └── SQLite + arquivos       ├── Player público
                               ├── Painel administrativo
                               ├── API REST / SSE
                               └── Metadados ICY
```

## Começando

### Requisitos

- [Rust](https://rustup.rs/) com suporte à edição 2024 — Rust 1.85 ou mais recente.
- Um arquivo de áudio para testar a ingestão, caso queira transmitir via `ffmpeg`.

### Executar localmente

```bash
git clone https://github.com/Thiagovilela2001/rustcast.git
cd rustcast
cargo run
```

Depois, acesse:

| Interface | Endereço |
| --- | --- |
| Player público | <http://127.0.0.1:8000/> |
| Painel administrativo | <http://127.0.0.1:8000/admin/> |
| Stream padrão | <http://127.0.0.1:8000/mount/radio> |

No primeiro boot, o RustCast cria o banco em `storage/rustcast.db` e uma estação padrão chamada `radio`.

As credenciais padrão de desenvolvimento são:

```text
Administrador: admin / admin
Source:        source / hackme
```

> Troque as duas senhas antes de expor o servidor à internet.

## Configuração

O RustCast é configurado por variáveis de ambiente.

| Variável | Padrão | Descrição |
| --- | --- | --- |
| `RUSTCAST_BIND` | `0.0.0.0:8000` | Endereço e porta HTTP. |
| `RUSTCAST_PUBLIC_URL` | — | URL pública usada em respostas e integrações. |
| `RUSTCAST_CORS_ORIGIN` | — | Origem autorizada para consumir a API. |
| `RUSTCAST_ADMIN_PASSWORD` | `admin` | Senha do painel e das operações administrativas. |
| `RUSTCAST_SOURCE_PASSWORD` | `hackme` | Senha mestre para ingestão de áudio. |
| `RUSTCAST_DB` | `storage/rustcast.db` | Caminho do banco SQLite. |
| `RUSTCAST_MEDIA_DIR` | `storage/media` | Diretório da biblioteca de mídia. |
| `RUSTCAST_THREADS` | `2` | Quantidade de threads de processamento. |
| `RUSTCAST_UTC_OFFSET_MIN` | `0` | Deslocamento UTC da grade de programação, em minutos. |
| `RUSTCAST_TARGET_LUFS` | `-16` | Volume-alvo da normalização EBU R128. |

Exemplo no PowerShell:

```powershell
$env:RUSTCAST_ADMIN_PASSWORD = "uma-senha-forte"
$env:RUSTCAST_SOURCE_PASSWORD = "outra-senha-forte"
cargo run --release
```

Para produção, consulte o modelo completo em [`deploy/rustcast.env.example`](deploy/rustcast.env.example).

## Transmitindo ao vivo

Encoders compatíveis com Icecast podem se conectar ao mountpoint da estação usando uma conta de DJ criada no painel ou a senha mestre de source.

Exemplo com `ffmpeg`:

```bash
ffmpeg -re -i musica.mp3 \
  -c:a libmp3lame -content_type audio/mpeg -f mp3 \
  icecast://source:hackme@127.0.0.1:8000/mount/radio
```

Configuração típica no Mixxx ou BUTT:

| Campo | Valor |
| --- | --- |
| Tipo | Icecast 2 |
| Host | endereço do servidor |
| Porta | `8000` |
| Mount | `/radio` |
| Usuário | usuário da conta de DJ |
| Senha | senha da conta de DJ |

Quando a fonte ao vivo é conectada, o AutoDJ é interrompido automaticamente. Ao desconectar, a programação retorna sem intervenção manual.

## API essencial

Leituras públicas não exigem autenticação. Operações de escrita usam Basic Auth com `admin:<RUSTCAST_ADMIN_PASSWORD>`. Os endpoints de contas de DJ também exigem autenticação nas leituras.

| Método | Endpoint | Uso |
| --- | --- | --- |
| `GET` | `/api/stations` | Listar estações. |
| `POST` | `/api/stations` | Criar uma estação. |
| `GET` | `/api/stations/{id}/media` | Listar a biblioteca. |
| `POST` | `/api/stations/{id}/media` | Enviar um arquivo de áudio. |
| `GET` | `/api/stations/{id}/playlists` | Listar playlists. |
| `POST` | `/api/stations/{id}/playlists` | Criar uma playlist. |
| `GET` | `/api/stations/{id}/nowplaying` | Now playing em tempo real via SSE. |
| `GET` | `/api/stations/{id}/nowplaying.json` | Snapshot atual em JSON. |
| `GET` | `/api/stations/{id}/djs` | Listar contas de DJ — requer admin. |
| `GET` | `/mount/{name}` | Ouvir o stream MP3. |

Erros da API usam o formato:

```json
{ "error": "descrição do erro" }
```

### Compatibilidade com AzuraCast

Para integrações que já consomem o formato do AzuraCast:

```text
GET /api/nowplaying/radio
GET /api/nowplaying/1
GET /api/nowplaying_static/radio.json
```

## Estrutura do projeto

```text
rustcast/
├── src/            # servidor, relay, áudio, banco e API
├── frontend/       # player e painel estáticos
├── migrations/     # migrations do SQLite
├── deploy/         # systemd, Caddy e scripts de produção
├── storage/        # banco e mídia local — criado em runtime
└── Cargo.toml
```

## Build de produção

```bash
cargo build --release
```

O binário resultante estará em `target/release/rustcast`. A versão release incorpora o frontend, portanto não é necessário copiar a pasta `frontend/` para o servidor.

Há um guia para Linux com `systemd`, HTTPS via Caddy e build estático com musl em [`deploy/DEPLOY.md`](deploy/DEPLOY.md).

## Segurança

Antes de publicar uma instância:

1. Defina senhas fortes em `RUSTCAST_ADMIN_PASSWORD` e `RUSTCAST_SOURCE_PASSWORD`.
2. Sirva o player e o painel atrás de HTTPS.
3. Restrinja a porta direta de ingestão aos DJs que realmente precisam dela.
4. Proteja o arquivo de ambiente e o diretório `storage/`.
5. Faça backups periódicos do banco e da biblioteca.

## Contribuindo

Issues e pull requests são bem-vindos. Para mudanças maiores, abra uma issue descrevendo o problema e a solução proposta antes de começar a implementação.

```bash
cargo check
cargo test
```

## Licença

Este projeto ainda não possui uma licença definida. Até que um arquivo `LICENSE` seja adicionado, todos os direitos permanecem reservados ao autor.

---

<div align="center">

Feito em Rust para manter sua rádio no ar sem manter seu servidor no limite.

</div>
