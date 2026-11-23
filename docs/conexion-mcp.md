# MCP Raptor Player

Reproductor de escritorio con lista en secuencia, mezcla entre temas, fader, botonera de efectos, editor de pistas con forma de onda y un **servidor MCP** integrado. MCP (Model Context Protocol) es el estándar que usan los asistentes de IA para manejar programas externos. Con él, una IA puede armar la lista, poner play, mezclar, mover los controles y disparar efectos.

Esta guía explica cómo conectar Raptor con los asistentes de IA más usados.

---

## 1. Cómo funciona la conexión

```
  Asistente de IA  ──HTTP──▶  http://127.0.0.1:8765/mcp  ──▶  MCP Raptor Player (ventana abierta)
```

- El servidor MCP **vive dentro de la aplicación**. Si la ventana está cerrada, la IA no tiene nada a qué conectarse.
- Usa el transporte **Streamable HTTP** en la dirección `http://127.0.0.1:8765/mcp`.
- Solo escucha en tu propia máquina (127.0.0.1) y rechaza pedidos que vengan de páginas web. **No tiene contraseña**, así que no lo publiques en internet (ver [Seguridad](#8-seguridad)).

### Requisitos

1. Abrí el reproductor:
   - Windows: `MCPRaptorPlayer.exe`
   - Linux: el binario `mcp-raptor-player` compilado desde `rust/`
2. Usá un modelo de IA que sepa llamar herramientas ("tool calling"). Todos los modelos comerciales actuales lo hacen. Con modelos locales chicos puede fallar.

### Cambiar el puerto

Si el 8765 está ocupado, definí la variable `RAPTOR_MCP_PUERTO` antes de abrir el reproductor y usá ese puerto en todas las URLs de esta guía. Con `0` el servidor MCP queda apagado.

```powershell
$env:RAPTOR_MCP_PUERTO = "9000"; & "C:\[Ruta al ejecutable]\MCPRaptorPlayer.exe"
```

### Comprobar que el servidor responde

Con el reproductor abierto:

```powershell
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8765/mcp -ContentType "application/json" -Body '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | ConvertTo-Json -Depth 5
```

En Linux o macOS:

```bash
curl -s -X POST http://127.0.0.1:8765/mcp -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
```

La respuesta debe incluir `"name": "mcp-raptor-player"`.

---

## 2. Configuración por asistente

Las configuraciones marcadas **✅ probado** se verificaron con este reproductor. Las demás siguen la documentación oficial de cada herramienta, que cambia seguido. Si alguna no te funciona, usá el [puente universal](#3-puente-universal-mcp-remote).

### Kilo Code (CLI y extensión de VS Code) ✅ probado

Archivo `~/.config/kilo/kilo.json` (en Windows, `C:\Users\<usuario>\.config\kilo\kilo.json`). Agregá dentro de `"mcp"`:

```json
"mcp": {
  "raptor": {
    "type": "remote",
    "url": "http://127.0.0.1:8765/mcp",
    "enabled": true
  }
}
```

Para verificarlo, ejecutá `kilo mcp list`; tiene que aparecer `✓ raptor connected`. En VS Code, recargá la ventana después de editar el archivo.

Kilo funciona con modelos locales o con los de su propia cuenta, así que **no necesitás claves de Google ni de OpenAI**.

### opencode

Usa el mismo formato que Kilo, en `~/.config/opencode/opencode.json`:

```json
{
  "mcp": {
    "raptor": { "type": "remote", "url": "http://127.0.0.1:8765/mcp", "enabled": true }
  }
}
```

### Gemini CLI ✅ probado (versión 0.65)

```bash
gemini mcp add --scope user --transport http raptor http://127.0.0.1:8765/mcp
```

Ese comando escribe en `~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "raptor": { "httpUrl": "http://127.0.0.1:8765/mcp" }
  }
}
```

Para verificarlo: `gemini mcp list`, o `/mcp` dentro de una sesión.

> Gemini CLI necesita una cuenta o clave de Google que lo habilite. Si no la tenés, usá Kilo, opencode o un modelo local.

### Claude Code

```bash
claude mcp add --transport http --scope user raptor http://127.0.0.1:8765/mcp
```

Para verificarlo: `claude mcp list`, o `/mcp` dentro de una sesión.

### Claude Desktop

Claude Desktop no se conecta directo a un servidor HTTP local: sus "conectores" pasan por la nube y no ven tu `127.0.0.1`. Usá el [puente `mcp-remote`](#3-puente-universal-mcp-remote) en `claude_desktop_config.json`:

- Windows: `%APPDATA%\Claude\claude_desktop_config.json`
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "raptor": {
      "command": "npx",
      "args": ["-y", "mcp-remote", "http://127.0.0.1:8765/mcp", "--transport", "http-only"]
    }
  }
}
```

Reiniciá Claude Desktop por completo, desde el ícono de la bandeja. Las herramientas aparecen en el botón de herramientas del chat.

### Cursor

Archivo `~/.cursor/mcp.json` (global) o `.cursor/mcp.json` (por proyecto):

```json
{
  "mcpServers": {
    "raptor": { "url": "http://127.0.0.1:8765/mcp" }
  }
}
```

Después revisá *Settings → MCP*: `raptor` tiene que aparecer en verde.

### VS Code con GitHub Copilot (modo agente)

Archivo `.vscode/mcp.json` en tu proyecto, o *MCP: Open User Configuration* para todos los proyectos:

```json
{
  "servers": {
    "raptor": { "type": "http", "url": "http://127.0.0.1:8765/mcp" }
  }
}
```

Iniciá el servidor desde el botón **Start** que aparece sobre la entrada y elegí el modo **Agent** en el chat de Copilot.

### Windsurf

Archivo `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "raptor": { "serverUrl": "http://127.0.0.1:8765/mcp" }
  }
}
```

### Cline (VS Code)

Abrí *MCP Servers → Configure* y editá `cline_mcp_settings.json`:

```json
{
  "mcpServers": {
    "raptor": { "type": "streamableHttp", "url": "http://127.0.0.1:8765/mcp" }
  }
}
```

### Roo Code (VS Code)

Abrí *MCP Servers → Edit Global MCP* (`mcp_settings.json`):

```json
{
  "mcpServers": {
    "raptor": { "type": "streamable-http", "url": "http://127.0.0.1:8765/mcp" }
  }
}
```

### OpenAI Codex CLI

Archivo `~/.codex/config.toml`:

```toml
[mcp_servers.raptor]
url = "http://127.0.0.1:8765/mcp"
```

Si tu versión de Codex solo acepta servidores por comando, usá el puente:

```toml
[mcp_servers.raptor]
command = "npx"
args = ["-y", "mcp-remote", "http://127.0.0.1:8765/mcp", "--transport", "http-only"]
```

### LM Studio (modelos locales)

En *Program → Edit mcp.json*:

```json
{
  "mcpServers": {
    "raptor": { "url": "http://127.0.0.1:8765/mcp" }
  }
}
```

Elegí un modelo con soporte de herramientas (por ejemplo Qwen, Llama 3.1 o posterior, o Mistral) y aprobá cada llamada cuando LM Studio lo pida.

### ChatGPT (web o app de escritorio)

ChatGPT ejecuta los conectores desde los servidores de OpenAI, así que **no puede llegar a `127.0.0.1`**. Para usarlo habría que publicar el reproductor en internet con un túnel HTTPS, y eso **no se recomienda**: el servidor no tiene contraseña y cualquiera con la URL podría manejarlo. Si querés usar modelos de OpenAI, usá Codex CLI o un cliente local (Cursor, VS Code, Kilo) con esos modelos.

---

## 3. Puente universal: `mcp-remote`

Algunos clientes solo saben lanzar servidores MCP como programas ("stdio"). Para ellos existe `mcp-remote`, que traduce stdio a HTTP. Necesita Node.js 18 o posterior. En cualquier cliente que acepte `command` y `args`:

```json
{
  "command": "npx",
  "args": ["-y", "mcp-remote", "http://127.0.0.1:8765/mcp", "--transport", "http-only"]
}
```

La primera vez, `npx` descarga el paquete.

---

## 4. Qué puede hacer la IA

| Herramienta | Para qué sirve |
|---|---|
| `listar_pistas` | Ver la lista con número, ruta, título, artista, género, duración, ranking, última reproducción, intro, outro, ventana de overlay y mezcla. Marca cuál suena. |
| `agregar_pistas` | Agregar archivos o carpetas completas, con rutas absolutas. Entran después de la pista seleccionada (o al final), o en la `posicion` indicada. A los temas nuevos se les aplican intro y outro sugeridas. |
| `quitar_pistas` | Quitar pistas por número o por ruta. No borra archivos del disco. |
| `mover_pista` | Cambiar el orden de la lista: lleva una pista a otra posición. |
| `localidad_del_clima` | Ver o cambiar la localidad del clima del anuncio HTH. Con `nombre` la busca en Open-Meteo; `region` filtra las coincidencias (por ejemplo `Entre Rios`) y `numero` elige una. Si queda una sola, la fija y consulta el clima; si quedan varias, las lista sin cambiar nada. Cambiarla solo se puede en modo EDITOR, porque la localidad fija la hora del reloj y de la columna Sonó. |
| `guardar_lista` | Guardar la lista con un `nombre` (en la carpeta de listas, o una ruta absoluta; `.json` por defecto, o `.m3u`/`.m3u8`), que queda como la lista abierta. `con_fecha` antepone la fecha y hora local (`AAAAMMDDhhmm_`). No pisa un archivo existente salvo con `sobrescribir`. Sin `nombre`, guarda la lista abierta. |
| `definir_mezcla` | Fijar intro, outro y la ventana de overlay (`overlay_inicio`, `overlay_fin`), en segundos o como `m:ss`; `quitar_overlay` la elimina. |
| `sugerir_puntos` | Analizar la forma de onda y sugerir intro y outro, sin cambiar nada. |
| `abrir_editor` | Abrir en pantalla el editor de una pista para ajustarla a mano. |
| `definir_datos` | Corregir título, artista o género y poner un ranking de 0 a 5. |
| `estado_del_reproductor` | Ver qué suena, el tiempo, la próxima pista, la duración total de la lista y lo que falta, el volumen, el fader, las opciones, los botones de efectos y la barra de estado. |
| `controlar_reproduccion` | Botones de transporte: `reproducir`, `pausar`, `alternar`, `detener`, `siguiente`, `anterior`. |
| `seleccionar_pista` | Poner el selector en un número de pista, que pasa a ser la próxima del botón Siguiente. |
| `ajustar_controles` | Volumen (0–100), fader (0–10 s), posición dentro de la pista, repetir lista, mostrar el panel de efectos, modo libre, bajar la música y nivel de la música. |
| `asignar_efecto` / `quitar_efecto` | Cargar o vaciar los botones 0 a 9 de una hoja de efectos (`hoja`, desde 1; si se omite, la que está a la vista). |
| `disparar_efecto` / `detener_efecto` | Hacer sonar un efecto (de cualquier hoja) o cortarlo. |
| `hojas_de_efectos` | Ir a una hoja de efectos, agregar una (con `nombre`, ya nombrada), quitar una (siempre queda al menos una) o `renombrar` una: vacío vuelve a «Hoja N». |

### Ejemplos de pedidos

- "Agregá a la lista todos los temas de `D:\Musica\Fiesta`."
- "Mostrame la lista con artista y duración."
- "Poné play, y cuando falten 20 segundos para que termine la pista 3 mezclá a la 7."
- "Seleccioná la pista 12 y pasá a ella con 8 segundos de fader."
- "Bajá el volumen al 60 % y activá *repetir lista*."
- "Asigná `C:\Efectos\aplauso.wav` al botón 3 y dispáralo."
- "Creá una hoja de efectos nueva y poné la bocina en su botón 0."
- "Ponele 5 estrellas a la pista que está sonando."
- "Detené todo con un fundido de 10 segundos."
- "Sugerí intro y outro para la pista 4 y aplicalas."
- "En la pista 2 dejá que los efectos suenen solo entre 1:20 y 1:45."
- "Abrí el editor de la pista 9."
- "Subí la pista 5 al primer lugar."
- "¿Cuánto falta para que termine la lista?"

---

## 5. Comportamiento del fader

- El fader (0 a 10 s, 5 por defecto) se aplica a **Siguiente** (mezcla la pista actual con la próxima) y a **Detener** (fundido de salida).
- Con el fader en 0, las dos acciones son instantáneas.
- **Pausa** siempre es instantánea.
- Si tocás Detener otra vez durante el fundido, se detiene al momento.
- La pista marcada con el selector (o con `seleccionar_pista`) es la que entra con Siguiente. Si no hay ninguna marcada, entra la que sigue en la lista.
- Mientras dura el fundido, el botón que lo inició se resalta con el color de acento y cambia su texto ("Mezclando..." o "Deteniendo...").

### En la ventana principal

- **Subir / Bajar:** mueven la pista seleccionada un lugar.
- **Duración:** cada tema la muestra en minutos, segundos y décimas. Al pie de la lista se ven el **total** y, mientras suena, el **restante** hasta el final. Los dos descuentan los segundos que se superponen en cada mezcla.
- **Panel de detalles:** debajo de la lista. Muestra título, artista, género, ranking y última reproducción de la pista seleccionada o, si no hay ninguna, de la que suena. La última reproducción se ve como fecha en hora local (`23/10/2026 12:34`). Al pasar el mouse por encima, un tooltip dice cuánto pasó, sin las unidades en cero y en singular cuando valen 1: "Hace 3 horas 20 minutos y 15 segundos", "Hace 1 hora 15 minutos y 1 segundo".
- **Última reproducción:** se registra cuando la pista **termina de sonar** (o se la detiene, se pasa a otra o se cierra el programa), no al empezar.
- **Doble clic en el nombre grande** de la pista que suena: lleva el selector a esa pista en la lista.
- **Doble clic en los deslizantes:** el volumen vuelve al 80 %, el fader a 5 s y "Música al" (en el panel de efectos) al 50 %.
- **Efectos:** el botón muestra u oculta el panel de efectos y se pone **naranja** mientras un efecto espera su ventana, aunque el panel esté oculto.
- **Columnas de la lista:** número, tema, duración, género, overlay, intro, outro y mezcla. Un **doble clic en Overlay, Intro u Outro abre el editor** de la pista; en el resto de la fila, la reproduce. El clic derecho muestra Reproducir, Editar pista y Quitar de la lista.
- **Punto de inserción:** las pistas que se agregan (con Agregar, desde los efectos o por MCP) entran **después de la pista seleccionada**, o al final si no hay selección. El selector queda en la última insertada.
- **Intro y outro sugeridas:** la primera vez que se agrega un tema, se calcula su forma de onda y se le aplican la intro y la outro sugeridas. Si el tema ya se había usado, conserva sus valores guardados.
- **Botones de efectos:** muestran su número (0 a 9), la duración del efecto y el nombre del archivo en hasta tres renglones. El clic derecho permite **Asignar archivo**, **Vaciar** o **Insertar en la playlist**.
- **Hojas de efectos:** el panel tiene hojas de 10 botones (0 a 9). Con `‹` y `›` se pasa de hoja, con `+` se agrega una y con `−` se quita la que está a la vista. Si tiene efectos asignados, pide confirmación. Siempre queda al menos una.
- **Playlist guardada:** la lista, la pista actual y "Repetir lista" se guardan en cada cambio y al cerrar, y se recuperan al abrir el programa. Los archivos que ya no existen se omiten con un aviso.
- **Salir:** si hay música sonando, el programa pregunta antes de cerrarse.
- **Tema oscuro:** la interfaz usa siempre el tema oscuro, sin importar la configuración del sistema.
- **Ayuda → Manual:** abre `Raptor_Manual_es.pdf` con el visor de PDF del sistema. Lo busca junto al ejecutable y en la carpeta de trabajo; si no está, avisa dónde lo buscó.
- **Ayuda → Acerca de:** muestra la imagen `raptor_planet.png` de fondo (va incluida en el ejecutable), los créditos y la versión del programa.

---

## 6. Editor de pistas y ventana de overlay

### Ventana de overlay

El overlay de cada pista es una **ventana de tiempo** (por ejemplo, de 1:20 a 1:45, medido desde el comienzo del tema) dentro de la cual puede sonar un efecto de la botonera en **modo trabado**:

- Si el efecto se pulsa **antes** de la ventana, queda en espera y suena cuando se abre. La botonera muestra cuánto falta.
- **Dentro** de la ventana, suena si cabe **entero** antes del final. Un efecto de 4 s en una ventana que termina en 1:45 puede arrancar hasta las 1:41.
- Si ya **no cabe**, o la ventana pasó, espera la ventana de la **pista siguiente**.
- Si mientras espera cambia la pista, pasa a esperar la ventana de la nueva.
- Una pista **sin ventana**, o la música detenida, deja sonar el efecto enseguida.

En la tabla, la columna Overlay muestra la ventana con su duración (`1:50.25 - 2:00.3 | 10.05`: inicio, fin y largo). Un doble clic en la celda abre el editor de la pista para cambiarla. Los overlays guardados por versiones anteriores, que eran "segundos restantes", se convierten solos a la ventana equivalente hasta el final del tema.

### Editor

Se abre con el botón **Editar pista**, con clic derecho sobre una fila (**Editar pista...**) o con la herramienta MCP `abrir_editor`.

- **Forma de onda:** se calcula una sola vez por archivo y queda en caché.
- **Marcas sobre la onda:** zona de intro (verde), zona de outro (azul), ventana de overlay (naranja) y cursor. Se arrastran con el mouse; un clic fuera de las marcas mueve el cursor.
- **Campos de tiempo:** se escriben en segundos (`80.5`) o como `m:ss` (`1:20.5`). Cada uno tiene su botón **Fijar en el cursor**.
- **Sugerir intro y outro:** estima los puntos según los fundidos del tema. La intro termina cuando el tema llega a la mitad de su nivel típico; la outro empieza cuando baja de ese nivel por última vez.
- **Zoom:** rueda del mouse sobre la onda, o los botones Acercar, Alejar y Ver todo.
- **Datos:** título, artista, género y ranking se editan en la misma ventana.
- **Preescucha:** desde el cursor, la intro, la outro, o la ventana de overlay con un efecto de prueba de la botonera. Solo funciona con la música detenida o en pausa, porque comparte la salida de audio.
- **Guardar** aplica los cambios a la lista y a `mezcla.json`; **Cancelar** los descarta.

**Atajos de teclado** (con el foco en la onda):

| Tecla | Acción |
|---|---|
| `I` / `O` | Fijar la intro / la outro en el cursor |
| `V` / `B` | Fijar el comienzo / el fin de la ventana de overlay en el cursor (la crea si no existe) |
| `←` / `→` | Mover el cursor 0,1 s (con `Shift`, 1 s) |
| `+` / `-` | Acercar / alejar |
| `Espacio` | Preescuchar desde el cursor, o parar |

---

## 7. Solución de problemas

| Síntoma | Causa probable y solución |
|---|---|
| El cliente muestra `raptor` desconectado o con error de conexión | El reproductor no está abierto, o lo abriste después que el cliente. Abrilo y reconectá (`/mcp`, *Reload* o reiniciar el cliente). |
| Error 403 | El pedido llegó con un encabezado `Origin` de una página web. Raptor solo acepta clientes locales. |
| Error 405 | El cliente intentó un `GET`. Raptor solo acepta `POST` (Streamable HTTP sin flujo SSE). Configurá el transporte como `http`/`streamable-http`, no `sse`. |
| La IA responde pero nunca usa las herramientas | El modelo no soporta tool calling o es demasiado chico. Probá con otro modelo, o pedile explícitamente: "usá la herramienta listar_pistas". |
| "La interfaz no respondió a tiempo" | Había un diálogo abierto en el reproductor (por ejemplo, el de elegir archivos). Cerralo y repetí. |
| Puerto ocupado al abrir el reproductor | Otro programa usa el 8765. Usá `RAPTOR_MCP_PUERTO` (ver arriba). |
| «No se pudo iniciar sesión en raptor» | Raptor no usa inicio de sesión (solo atiende en 127.0.0.1). Ese aviso sale si se elige *Iniciar sesión* / *Authenticate* en el cliente; no hace falta: el conector funciona igual. |

---

## 8. Seguridad

- El servidor escucha **solo en 127.0.0.1**: ninguna otra computadora de la red puede conectarse.
- Rechaza pedidos con un `Origin` que no sea local, así una página web abierta en tu navegador no puede manejar el reproductor.
- **No tiene autenticación.** No lo expongas con túneles (ngrok, Cloudflare Tunnel, etc.) ni abras el puerto en el router.
- Las herramientas pueden agregar y quitar pistas de la lista, pero **nunca borran archivos del disco**.
- La mayoría de los clientes piden confirmación antes de cada llamada. Conviene mantenerla activa, al menos para `quitar_pistas` y `quitar_efecto`.
- En **modo AL AIRE** (botón junto al engranaje del transporte) el reproductor rechaza, también por MCP, lo que rompe la secuencia que sale: `anterior`, `ordenar_lista`, `posicion_segundos` y quitar más de una pista a la vez. La respuesta explica el motivo.
  Al aire, `controlar_reproduccion` con `detener` sigue en modo AL AIRE; con `"salir_del_aire": true` detiene y pasa a modo EDITOR. `estado_del_reproductor` informa el modo.

---

## 9. Datos que guarda el reproductor

Todo se guarda en `~/.mcp-raptor-player/` (en Windows, `C:\Users\<usuario>\.mcp-raptor-player\`). Con la variable `RAPTOR_CARPETA_DE_DATOS` podés usar otra carpeta.

| Archivo | Contenido |
|---|---|
| `mezcla.json` | Una ficha por canción: intro, outro, ventana de overlay (`overlayInicio`, `overlayFin`), título, artista, género, duración, ranking y última reproducción. |
| `botonera.json` | Las hojas de efectos con el archivo de cada botón (0 a 9), la hoja a la vista, el modo libre o trabado y la atenuación de la música. |
| `preferencias.json` | Los segundos del fader. |
| `lista.json` | La playlist: las pistas en orden, la actual y si se repite. |
| `ondas/` | Caché de las formas de onda (una por archivo). Se puede borrar: se vuelve a calcular cuando hace falta. |
