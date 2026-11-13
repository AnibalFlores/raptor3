#!/bin/sh
# Instala MCP Raptor Player para el usuario actual (sin permisos de root):
# el ejecutable en ~/.local/bin, el icono en el tema hicolor y el lanzador en
# el menu de aplicaciones. El escritorio asocia la ventana con el lanzador por
# su identificador (mcp-raptor-player) y de ahi toma el icono de la barra.
#
# Uso: recursos/linux/instalar.sh [ruta del ejecutable]
# Por defecto usa target/release/mcp-raptor-player.
set -eu

raiz=$(cd "$(dirname "$0")/../.." && pwd)
ejecutable=${1:-"$raiz/target/release/mcp-raptor-player"}
datos=${XDG_DATA_HOME:-"$HOME/.local/share"}

if [ ! -x "$ejecutable" ]; then
    echo "No se encuentra el ejecutable $ejecutable (compile con: cargo build --release)" >&2
    exit 1
fi

install -D -m 755 "$ejecutable" "$HOME/.local/bin/mcp-raptor-player"
install -D -m 644 "$raiz/ui/iconos/ventana-raptor.svg" "$datos/icons/hicolor/scalable/apps/mcp-raptor-player.svg"
install -D -m 644 "$raiz/recursos/linux/mcp-raptor-player.desktop" "$datos/applications/mcp-raptor-player.desktop"

# Refresca los indices si las herramientas estan instaladas (no son obligatorias).
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$datos/icons/hicolor" || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$datos/applications" || true

echo "Instalado. Si ~/.local/bin no esta en el PATH, agreguelo para lanzar mcp-raptor-player desde el menu."
