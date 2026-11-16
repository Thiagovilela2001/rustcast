# Deploy — Oracle AMD Micro (1 GB) · radio.blackmesa.works

RAM medida no Linux: 8,8 MB pico (AutoDJ tocando, 7 threads). Caddy soma ~30 MB.

## 1. Gerar o binário Linux (no PC, via WSL — uma vez)

```bash
# dentro do WSL (Ubuntu)
sudo apt update && sudo apt install -y build-essential musl-tools curl
curl https://sh.rustup.rs -sSf | sh -s -- -y
source ~/.cargo/env && rustup target add x86_64-unknown-linux-musl
cd /mnt/c/Users/Thiago/Documents/rustcast
cargo build --release --target x86_64-unknown-linux-musl
# o WSL já está configurado; rebuild:
#   CARGO_TARGET_DIR=~/rc-target cargo build --release --target x86_64-unknown-linux-musl
#   cp ~/rc-target/x86_64-unknown-linux-musl/release/rustcast dist/rustcast-linux-x86_64
# resultado: dist/rustcast-linux-x86_64 (estático, roda em qualquer distro)
```

## 2. Preparar a VM

```bash
# Parar o AzuraCast (libera RAM e as portas 80/443). Ele fica instalado, dá pra voltar.
cd /var/azuracast && sudo docker compose down

sudo useradd --system --home /opt/rustcast --shell /usr/sbin/nologin rustcast
sudo mkdir -p /opt/rustcast/storage && sudo chown -R rustcast:rustcast /opt/rustcast
```

Copie do PC: `scp dist/rustcast-linux-x86_64 deploy/* ubuntu@<IP>:/tmp/` e renomeie: `mv /tmp/rustcast-linux-x86_64 /tmp/rustcast`

```bash
sudo install -m 755 /tmp/rustcast /opt/rustcast/rustcast
sudo install -m 600 /tmp/rustcast.env.example /etc/rustcast.env   # EDITE as senhas!
sudo install -m 644 /tmp/rustcast.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now rustcast
journalctl -u rustcast -f
```

## 3. HTTPS com Caddy

```bash
sudo apt install -y caddy
sudo cp /tmp/Caddyfile /etc/caddy/Caddyfile && sudo systemctl reload caddy
```

DNS: `radio.blackmesa.works` → IP da VM (registro A). Caddy emite o certificado sozinho.

## 4. Portas na Oracle (duas camadas!)

1. Console → VCN → Security List: liberar TCP **80, 443** (e **8000** se houver DJ ao vivo).
2. No próprio Ubuntu da Oracle (iptables vem bloqueando tudo):
   ```bash
   sudo iptables -I INPUT 6 -p tcp -m multiport --dports 80,443,8000 -j ACCEPT
   sudo netfilter-persistent save
   ```

## 5. Músicas

```bash
sudo RUSTCAST_ADMIN_PASSWORD='...' bash /tmp/import.sh /var/azuracast/stations/<estacao>/media
```

## URLs (compatíveis com AzuraCast)

| Uso | URL |
| --- | --- |
| Stream | `https://radio.blackmesa.works/listen/radio/radio.mp3` |
| Now playing (formato AzuraCast) | `https://radio.blackmesa.works/api/nowplaying/radio` (ou `/api/nowplaying/1`, `/api/nowplaying_static/radio.json`) |
| Now playing tempo real (SSE) | `https://radio.blackmesa.works/api/stations/1/nowplaying` |
| DJ ao vivo (Mixxx/BUTT) | host `radio.blackmesa.works`, porta `8000`, mount `/radio`, usuário/senha da conta DJ |

Se o shortcode da estação no AzuraCast não for `radio`, crie a estação com esse mountpoint
(o shortcode aqui = mountpoint) ou mude o site.

DJ conecta direto na porta 8000 (HTTP puro, igual Icecast): o Caddy não repassa o método `SOURCE`.
