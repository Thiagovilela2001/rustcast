#!/usr/bin/env bash
# Envia todos os áudios de uma pasta para a estação. Uso: ./import.sh /pasta/musicas [station_id]
set -u
DIR="$1"; ST="${2:-1}"
: "${RUSTCAST_ADMIN_PASSWORD:?defina RUSTCAST_ADMIN_PASSWORD}"
ok=0; fail=0
while IFS= read -r -d '' f; do
  code=$(curl -s -o /dev/null -w '%{http_code}' -u "admin:$RUSTCAST_ADMIN_PASSWORD" -F "file=@\"$f\"" "http://127.0.0.1:8000/api/stations/$ST/media")
  if [ "$code" = 201 ]; then ok=$((ok+1)); else fail=$((fail+1)); echo "FALHOU ($code): $f"; fi
done < <(find "$DIR" -type f \( -iname '*.mp3' -o -iname '*.flac' -o -iname '*.wav' -o -iname '*.ogg' -o -iname '*.m4a' -o -iname '*.aac' -o -iname '*.mpeg' \) -print0)
echo "ok=$ok falhas=$fail"
