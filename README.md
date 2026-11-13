# Raptor 3

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

- [Manual de usuario (HTML)](docs/manual/Raptor_Manual_es.html) · [PDF](docs/manual/Raptor_Manual_es.pdf)
- [Cómo conectar asistentes de IA por MCP](docs/conexion-mcp.md) (Claude, Gemini CLI, Cursor, VS Code, Kilo Code y otros)

## Descargas

Las versiones compiladas para Windows y Linux están en [Releases](https://github.com/AnibalFlores/raptor3/releases). Cada una trae el ejecutable, el manual en PDF y la licencia.

## Compilar

Requiere **Rust 1.88 o posterior** (edición 2024), instalado con [rustup](https://rustup.rs).

### Windows

```bash
cargo build --release
```

El ejecutable queda en `target/release/mcp-raptor-player.exe`. Desde la carpeta del proyecto el menú *Ayuda › Manual* encuentra el PDF en `docs/manual`; para distribuirlo, copie `Raptor_Manual_es.pdf` junto al ejecutable.

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

## Contacto

jxlfybcrk@mozmail.com

## Licencia

[GNU GPL v3](LICENSE) o posterior.
