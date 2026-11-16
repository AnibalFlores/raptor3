#!/bin/sh
# Arma la app de macOS: "MCP Raptor Player.app" universal (Apple Silicon e
# Intel en un solo ejecutable), con su icono, los manuales en PDF y una firma
# ad hoc, y la comprime en un .zip junto con la licencia y los README.
#
# Corre en una Mac (o en las maquinas macOS de GitHub Actions), desde la raiz
# del proyecto:
#
#   recursos/macos/empaquetar.sh [nombre del paquete]
#
# Por defecto el paquete se llama MCPRaptorPlayer-macos-universal.
#
# La firma ad hoc no reemplaza la de Apple: sin una cuenta de Apple Developer
# la app no esta notarizada y macOS pide confirmarla la primera vez que se
# abre (ver el README). Sin ninguna firma, una Mac con Apple Silicon ni
# siquiera la ejecuta.
set -eu

raiz=$(cd "$(dirname "$0")/../.." && pwd)
cd "$raiz"
paquete=${1:-MCPRaptorPlayer-macos-universal}
nombre_de_la_app="MCP Raptor Player"
ejecutable=mcp-raptor-player
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
destinos="aarch64-apple-darwin x86_64-apple-darwin"

# Las dos arquitecturas y el ejecutable universal.
rustup target add $destinos
binarios=""
for destino in $destinos; do
    cargo build --release --locked --target "$destino"
    binarios="$binarios target/$destino/release/$ejecutable"
done
mkdir -p target/universal
lipo -create -output "target/universal/$ejecutable" $binarios
lipo -info "target/universal/$ejecutable"

# El paquete .app.
salida="target/$paquete"
app="$salida/$nombre_de_la_app.app"
rm -rf "$salida" "target/$paquete.zip"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/universal/$ejecutable" "$app/Contents/MacOS/$ejecutable"
cp recursos/macos/raptor.icns "$app/Contents/Resources/raptor.icns"
# Ayuda > Manual busca los PDF en Contents/Resources.
cp docs/manual/Raptor_Manual_es.pdf docs/manual/Raptor_Manual_en.pdf "$app/Contents/Resources/"
sed "s/@VERSION@/$version/g" recursos/macos/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"

# Firma ad hoc de todo el paquete (sin identidad de desarrollador).
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict --verbose=2 "$app"

# Documentos junto a la app y el .zip (ditto conserva los atributos del paquete).
cp LICENSE ASSETS-LICENSES.md README.md README.es.md docs/manual/Raptor_Manual_es.pdf docs/manual/Raptor_Manual_en.pdf "$salida/"
(cd target && ditto -c -k --keepParent "$paquete" "$paquete.zip")
echo "Listo: target/$paquete.zip (version $version)"
