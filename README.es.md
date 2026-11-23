# Raptor 3

**Español** · [English](README.md)

**Versión 1.1.0** · [Novedades](#novedades-de-la-versión-110)

**MCP Radioautomation Software.** Reproductor de escritorio para automatización de radio, escrito en Rust con interfaz [Slint](https://slint.dev), con un **servidor MCP** integrado para que un asistente de IA pueda manejarlo.

Raptor nació en 1997 (Raptor 1, Delphi sobre Windows 95) y siguió con Raptor II hasta 2007. Raptor 3 es una reescritura completa.

## Qué hace

- **Lista en secuencia** con mezcla entre temas: intro, outro, ventana de overlay y crossfade inteligente según la sonoridad.
- **Nivelación de volumen** por sonoridad (LUFS, EBU R128) y fader.
- **Botonera de efectos** por hojas, con atenuación inteligente de la música (parámetro *Separación*).
- **Anuncio HTH**: hora, temperatura y humedad (clima de Open-Meteo) armado con locuciones pregrabadas, sobre una base musical.
- **Tandas comerciales** programadas, anunciantes, órdenes, pauta del día y registro de emisiones por mes (CSV) con certificado de emisión.
- **Catálogo** con búsqueda, tipos (Tema, Tema editado, Podcast, Comercial, Efecto, Seco, HTH…), actualización de metadatos desde las etiquetas o desde internet (MusicBrainz, Deezer e iTunes) y carátulas en el panel de detalle.
- **Editor de pistas** con forma de onda, zoom y atajos de teclado.
- Interfaz en **español e inglés**.
- **Servidor MCP** (Streamable HTTP en `http://127.0.0.1:8765/mcp`): una IA puede armar la lista, reproducir, mezclar, mover los controles, disparar efectos y programar tandas.

## Documentación

- Manual de usuario: [español (HTML)](docs/manual/Raptor_Manual_es.html) · [PDF](docs/manual/Raptor_Manual_es.pdf) — [inglés (HTML)](docs/manual/Raptor_Manual_en.html) · [PDF](docs/manual/Raptor_Manual_en.pdf)
- [Cómo conectar asistentes de IA por MCP](docs/conexion-mcp.md) (Claude, Gemini CLI, Cursor, VS Code, Kilo Code y otros)

## Descargas

Las versiones compiladas para Windows, Linux y macOS están en [Releases](https://github.com/AnibalFlores/raptor3/releases). Cada una trae el programa, el manual en PDF en español y en inglés, y las licencias.

El ejecutable de Windows no está firmado digitalmente, así que SmartScreen puede advertir la primera vez que se abre: elija *Más información › Ejecutar de todas formas*.

### macOS (experimental)

`MCPRaptorPlayer-macos-universal.zip` trae *MCP Raptor Player.app*, una app universal para Mac con Apple Silicon e Intel, con macOS 11 o posterior. Todavía no se probó en una Mac real: el audio, los diálogos de archivos y la interfaz pueden tener problemas. Los reportes son bienvenidos.

1. Descomprima el archivo y arrastre *MCP Raptor Player.app* a *Aplicaciones*.
2. La app no está notarizada por Apple, así que la primera vez macOS no la abre. Ábrala una vez, cierre el aviso y vaya a *Configuración del Sistema › Privacidad y seguridad*: abajo, elija *Abrir igualmente* y confirme. En macOS 14 o anterior alcanza con clic derecho en la app › *Abrir*.
3. Otra forma, en la Terminal:

```bash
xattr -dr com.apple.quarantine "/Applications/MCP Raptor Player.app"
```

## Novedades de la versión 1.1.0

Cambios desde la 1.0.2-rc1.

### Funciones nuevas

#### Modos AL AIRE y EDITOR
- Nuevo botón de modo, siempre a la vista junto al tema que suena: gris **EDITOR** (todo permitido) o rojo **AL AIRE**. El modo elegido se recuerda.
- AL AIRE protege la secuencia que sale al aire:
  - Permitido: Detener, Siguiente con el fader y agregar o quitar temas de a uno.
  - Bloqueado: adelantar o retroceder dentro del tema, Anterior, saltar a otra pista, mezclar u ordenar, vaciar o reemplazar la lista, y mover o quitar varias pistas a la vez.
- Al detener estando AL AIRE, o al hacer clic en el botón rojo, pregunta si se vuelve a EDITOR.
- Los comandos MCP siguen las mismas reglas y, si algo está bloqueado, responden con el motivo:
  - `controlar_reproduccion` con `detener` sigue AL AIRE, salvo que se pase `salir_del_aire: true`.
  - `estado_del_reproductor` informa el modo.

#### Columna «Sonó»: horas de comienzo y estimadas
- Nueva columna **Sonó** en la lista, entre Duración y Tipo:
  - en los temas que ya sonaron, la fecha y hora en que **empezaron** a sonar por última vez (ahora se anota al empezar, no al terminar);
  - en los que siguen al que suena, la hora **estimada** de comienzo en cursiva, descontando las mezclas y actualizada cada segundo;
  - el globo dice cuánto hace que sonó o cuánto falta para que empiece.
- La última reproducción se puede corregir o borrar en la solapa **Datos** del editor. No admite fechas futuras.
- El catálogo muestra la fecha y hora completas, con segundos, en lugar de solo el día.

#### Zona horaria y formato de fecha
- Todas las fechas y horas siguen la zona horaria de la localidad del HTH: el reloj, la voz del HTH, la columna Sonó, el catálogo, el panel de detalle y la búsqueda `sonado:`.
- La localidad solo se cambia en modo EDITOR, desde la ventana o por MCP. Al cambiarla se recalculan todas las horas en pantalla.
- Nuevo menú **Ventana › Formato de fecha**, junto al de idioma: dd/mm/aaaa, mm/dd/aaaa o aaaa-mm-dd. Rige en todos los lugares donde se muestran o se escriben fechas:
  - la búsqueda `sonado:` lee las fechas en el orden elegido, también con el año corto, como `03/11/26`;
  - la columna **Próxima salida** de la pauta ahora incluye el año.
- Los archivos guardados y los registros siguen en ISO 8601.

#### Carátulas y datos de internet
- Carátulas en el panel de detalle y en el catálogo. Salen de las etiquetas del archivo, después de una imagen en la carpeta del tema o en la del álbum y, si se habilita, de internet (Deezer, iTunes o MusicBrainz con el Cover Art Archive). Al pasar el puntero se ven ampliadas, con su origen.
- Sin carátula, el panel muestra la «R» de Raptor en el color del tipo de elemento.
- **Completar desde internet** ahora también consulta Deezer e iTunes después de MusicBrainz. La coincidencia es estricta, se ignoran los compilados y se quitan las notas de edición del nombre del álbum.
- El catálogo recuerda qué datos corregiste a mano, para que las actualizaciones desde las etiquetas o desde internet no los pisen por defecto.
- Las tareas largas del catálogo (búsquedas en internet y carátulas desde 30 temas, relectura de etiquetas desde 300) estiman cuánto van a tardar y preguntan antes de empezar.

#### Ventana principal
- Renglón grande de lo que suena: ícono de reproducir, ícono del tipo en el color de su franja, número, título y artista, justificados a la izquierda. AL AIRE se alterna con el tema siguiente, marcado con el ícono de doble play.
- Lo que suena se ve en cian y el siguiente, en el color normal del texto.
- Todas las casillas de la ventana principal y de la botonera pasaron a ser botones con ícono y texto, en verde cuando están activos. **Modo libre** muestra un candado abierto o cerrado.
- Las opciones de reproducción pasaron junto al volumen y el fader, con los nombres **Repetir**, **SmartMix** y **Autonivel**. Si falta lugar quedan solo con el ícono, y el globo conserva el nombre.
- La columna **Ranking** ahora se llama **Rate**.
- Los avisos de efecto «En espera» muestran el nombre sin la extensión del archivo, con décimas de segundo.
- La barra de progreso muestra las marcas de overlay, outro y mezcla en los dos modos.

#### Reproducción y tandas
- Comerciales, Publicidades y Anuncios suenan siempre al máximo volumen sin distorsión: el pico real sube hasta −1 dBTP, hasta +6 dB, y nunca se baja.
- El efecto fijado de un tema, o la voz del HTH sobre su base musical, ahora espera su ventana por separado y ya no reemplaza al efecto que espera en la botonera.
- Si una orden de publicidad se carga o se cambia después de encolada su tanda, sus avisos se agregan a esa tanda si todavía no sonó o pasan a la próxima tanda posible.
- Las hojas de efectos se pueden renombrar con clic derecho o doble clic sobre el nombre de la hoja. Por MCP, `hojas_de_efectos` también nombra y renombra hojas.

#### MCP
- Nueva herramienta `guardar_lista`: guarda la lista en JSON o M3U, con la fecha y hora como prefijo si se pide, y nunca pisa un archivo sin que se le pida.
- Nueva herramienta `localidad_del_clima` para ver o cambiar la localidad del clima.
- El conector ahora muestra el ícono de Raptor en los clientes MCP.

### Correcciones
- Después de quitar pistas, el selector queda en el mismo lugar en vez de saltar al tema que suena. Con Desplazar activo, la lista no se mueve.
- **Limpiar** ahora pide confirmación.
- En el menú contextual **Actualizar datos**, «Completar desde internet» ahora va antes que «Releer las etiquetas».
- Al cerrar la ventana principal, primero se cierran todas las ventanas hijas: editor, catálogo, órdenes y las demás.
- Los botones de abajo de la ventana de órdenes de publicidad se veían cortados.
- El renglón grande tenía espacio vertical de más y el artista desalineado.
- Varios textos salían en español con la interfaz en inglés, entre ellos «Guardado», la ayuda del editor, «inactiva» y «(en …)».

### Documentación
- El manual de usuario (español e inglés, HTML y PDF) y sus capturas se actualizaron con todo lo anterior.

## Compilar

Requiere **Rust 1.88 o posterior** (edición 2024), instalado con [rustup](https://rustup.rs).

### Windows

```bash
cargo build --release
```

El ejecutable queda en `target/release/mcp-raptor-player.exe`. *Ayuda › Manual* abre el PDF del idioma de la interfaz (`Raptor_Manual_es.pdf` o `Raptor_Manual_en.pdf`); desde la carpeta del proyecto los encuentra en `docs/manual`, y para distribuir el programa, cópielos junto al ejecutable.

### macOS

Requiere las herramientas de línea de comandos de Xcode (`xcode-select --install`). Para armar la `.app` universal y su `.zip` en `target/`:

```bash
sh recursos/macos/empaquetar.sh
```

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
