# Raptor 3

**Español** · [English](README.md)

**MCP Radioautomation Software.** Reproductor de escritorio para automatización de radio, escrito en Rust con interfaz [Slint](https://slint.dev), con un **servidor MCP** integrado para que un asistente de IA pueda manejarlo.

Raptor nació en 1997 (Raptor 1, Delphi sobre Windows 95) y siguió con Raptor II hasta 2007. Raptor 3 es una reescritura completa.

## Qué hace

- **Lista en secuencia** con mezcla entre temas: intro, outro, ventana de overlay y crossfade inteligente según la sonoridad.
- **Nivelación de volumen** por sonoridad (LUFS, EBU R128) y fader.
- **Botonera de efectos** por hojas, con atenuación inteligente de la música (parámetro *Separación*).
- **Anuncio HTH**: hora, temperatura y humedad (clima de Open-Meteo) armado con locuciones pregrabadas, sobre una base musical.
- **Tandas comerciales** programadas, anunciantes, órdenes, pauta del día y registro de emisiones por mes (CSV) con certificado de emisión.
- **Catálogo** con búsqueda, tipos (Tema, Tema editado, Podcast, Comercial, Efecto, Seco, HTH…) y actualización de metadatos desde las etiquetas o desde MusicBrainz.
- **Editor de pistas** con forma de onda, zoom y atajos de teclado.
- Interfaz en **español e inglés**.
- **Servidor MCP** (Streamable HTTP en `http://127.0.0.1:8765/mcp`): una IA puede armar la lista, reproducir, mezclar, mover los controles, disparar efectos y programar tandas.

## Documentación

- Manual de usuario: [español (HTML)](docs/manual/Raptor_Manual_es.html) · [PDF](docs/manual/Raptor_Manual_es.pdf) — [inglés (HTML)](docs/manual/Raptor_Manual_en.html) · [PDF](docs/manual/Raptor_Manual_en.pdf)
- [Cómo conectar asistentes de IA por MCP](docs/conexion-mcp.md) (Claude, Gemini CLI, Cursor, VS Code, Kilo Code y otros)

## Descargas

Las versiones compiladas para Windows y Linux están en [Releases](https://github.com/AnibalFlores/raptor3/releases). Cada una trae el ejecutable, el manual en PDF en español y en inglés, y las licencias.

El ejecutable de Windows no está firmado digitalmente, así que SmartScreen puede advertir la primera vez que se abre: elija *Más información › Ejecutar de todas formas*.

## Compilar

Requiere **Rust 1.88 o posterior** (edición 2024), instalado con [rustup](https://rustup.rs).

### Windows

```bash
cargo build --release
```

El ejecutable queda en `target/release/mcp-raptor-player.exe`. *Ayuda › Manual* abre el PDF del idioma de la interfaz (`Raptor_Manual_es.pdf` o `Raptor_Manual_en.pdf`); desde la carpeta del proyecto los encuentra en `docs/manual`, y para distribuir el programa, cópielos junto al ejecutable.

### Linux (Ubuntu / Debian, también WSL2 con WSLg)

```bash
sudo apt install build-essential pkg-config libasound2-dev libfontconfig1-dev libgtk-3-dev libudev-dev libwayland-dev libx11-dev libxcb1-dev libxcb-render0-dev libxcb-shm0-dev libxkbcommon-dev libssl-dev
```

```bash
cargo build --release
```

```bash
recursos/linux/instalar.sh
```

El script instala el ejecutable en `~/.local/bin`, el icono y el lanzador del menú, sin permisos de root. El audio sale por PulseAudio (o PipeWire con su capa de PulseAudio).

## Pruebas

```bash
cargo test
```

Las pruebas usan carpetas temporales y no tocan los datos del usuario. Las capturas de todas las ventanas en distintas resoluciones se generan aparte, en `target/capturas`:

```bash
cargo test capturas -- --ignored --test-threads=1
```

Para probar el programa sin tocar sus datos reales, defina `RAPTOR_CARPETA_DE_DATOS` con una carpeta de prueba. El puerto del servidor MCP se cambia con `RAPTOR_MCP_PUERTO`.

## Manual

El manual se escribe en HTML (`docs/manual/Raptor_Manual_es.html` y `Raptor_Manual_en.html`) con capturas que generan las pruebas. Para actualizar las capturas y los PDF en los dos idiomas:

```bash
cargo test capturas -- --ignored --test-threads=1
```

```bash
RAPTOR_CAPTURAS_IDIOMA=en cargo test capturas -- --ignored --test-threads=1
```

```bash
python herramientas/manual.py
```

El script copia las capturas a `docs/manual/imagenes` e imprime los PDF con Chrome, Chromium o Edge.

## Traducciones

Los textos de la interfaz se escriben en español y se traducen con el catálogo `lang/en/LC_MESSAGES/mcp-raptor-player.po`. La herramienta `herramientas/traducciones.py` lista los textos sin traducir (`--faltantes`) y los agrega al catálogo (`--agregar`). Una prueba verifica que el catálogo esté completo.

## Locuciones del anuncio HTH

Las locuciones de hora, temperatura y humedad **no están en el repositorio**. Se generan con [ElevenLabs](https://elevenlabs.io) y una clave propia:

```bash
python herramientas/generar_locuciones.py --simular
```

```bash
ELEVENLABS_API_KEY=... ELEVENLABS_VOICE_ID=... python herramientas/generar_locuciones.py
```

La clave se lee **solo del entorno**: no la escriba en el script ni la suba al repositorio. Después, en el programa, elija la carpeta con clic derecho en el botón **HTH**.

## Seguridad

El servidor MCP escucha solo en `127.0.0.1`, rechaza pedidos de páginas web y **no tiene contraseña**: no lo exponga a internet.

## Contribuir y seguridad

Los aportes son bienvenidos: vea [CONTRIBUTING.es.md](CONTRIBUTING.es.md). Para reportar un problema de seguridad en privado, vea [SECURITY.es.md](SECURITY.es.md).

## Contacto y donaciones

jxlfybcrk@mozmail.com

Software gratuito, sin garantía ni soporte. Se aceptan donaciones y sugerencias: [donar con PayPal](https://www.paypal.com/donate/?hosted_button_id=SYCZZFULZ6TKE).

## Licencia

[GNU GPL v3](LICENSE) o posterior. Las imágenes y los iconos tienen sus propias licencias: vea [ASSETS-LICENSES.md](ASSETS-LICENSES.md).
