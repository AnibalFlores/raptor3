"""Mantiene el catalogo de traducciones de MCP Raptor Player.

Junta los textos marcados para traducir (``@tr("...")`` en ui/*.slint y
``tr!("...")`` en src/**/*.rs) y reescribe lang/<idioma>/LC_MESSAGES/
mcp-raptor-player.po conservando las traducciones que ya tiene. Los textos que
ya no se usan se quitan.

Uso:
    python herramientas/traducciones.py                  # actualiza y avisa lo que falta
    python herramientas/traducciones.py --agregar t.json # suma traducciones {"español": "inglés"}
    python herramientas/traducciones.py --faltantes f.json # escribe los textos sin traducir

El contenido de cada msgid queda tal como esta escrito en el codigo (con sus
escapes), porque el compilador de Rust compara los literales asi.
"""

import argparse
import json
import pathlib
import re
import sys

RAIZ = pathlib.Path(__file__).resolve().parent.parent
DOMINIO = "mcp-raptor-player"
# Un literal de cadena: comillas, y adentro cualquier cosa salvo comillas o
# barras sueltas.
LITERAL = r'"((?:[^"\\]|\\.)*)"'
PATRONES = {
    ".slint": re.compile(r"@tr\(\s*" + LITERAL),
    ".rs": re.compile(r"\btr!\(\s*" + LITERAL),
}


def desescapar(texto):
    return re.sub(r"\\(.)", lambda m: {"n": "\n", "t": "\t"}.get(m.group(1), m.group(1)), texto)


def escapar(texto):
    return texto.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t")


def textos_marcados():
    """Textos marcados, en orden de aparicion, con el archivo donde estan."""
    encontrados = {}
    archivos = sorted((RAIZ / "ui").glob("*.slint")) + sorted((RAIZ / "src").rglob("*.rs"))
    for archivo in archivos:
        contenido = archivo.read_text(encoding="utf-8")
        for coincidencia in PATRONES[archivo.suffix].finditer(contenido):
            texto = coincidencia.group(1)
            if texto:
                encontrados.setdefault(texto, set()).add(archivo.relative_to(RAIZ).as_posix())
    return encontrados


def leer_po(ruta):
    """Pares msgid -> msgstr (contenido crudo, con escapes)."""
    entradas = {}
    if not ruta.exists():
        return entradas
    actual, campo = None, None
    for linea in ruta.read_text(encoding="utf-8").splitlines():
        linea = linea.strip()
        if linea.startswith("msgid "):
            actual = linea[6:].strip()[1:-1]
            entradas[actual] = ""
            campo = "id"
        elif linea.startswith("msgstr "):
            entradas[actual] = linea[7:].strip()[1:-1]
            campo = "str"
        elif linea.startswith('"') and actual is not None:
            if campo == "id":
                valor = entradas.pop(actual)
                actual += linea[1:-1]
                entradas[actual] = valor
            else:
                entradas[actual] += linea[1:-1]
    entradas.pop("", None)
    return entradas


def escribir_po(ruta, idioma, marcados, traducciones):
    lineas = [
        "# Traducciones de MCP Raptor Player. Lo actualiza herramientas/traducciones.py.",
        'msgid ""',
        'msgstr ""',
        '"Content-Type: text/plain; charset=UTF-8\\n"',
        f'"Language: {idioma}\\n"',
        '"Plural-Forms: nplurals=2; plural=(n != 1);\\n"',
        "",
    ]
    for texto, archivos in marcados.items():
        lineas.append("#: " + " ".join(sorted(archivos)))
        lineas.append(f'msgid "{texto}"')
        lineas.append(f'msgstr "{traducciones.get(texto, "")}"')
        lineas.append("")
    ruta.parent.mkdir(parents=True, exist_ok=True)
    ruta.write_text("\n".join(lineas), encoding="utf-8", newline="\n")


def main():
    argumentos = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    argumentos.add_argument("--idioma", default="en")
    argumentos.add_argument("--agregar", help="JSON {texto en español: traduccion}")
    argumentos.add_argument("--faltantes", help="Escribe un JSON con los textos sin traducir")
    opciones = argumentos.parse_args()

    ruta = RAIZ / "lang" / opciones.idioma / "LC_MESSAGES" / f"{DOMINIO}.po"
    marcados = textos_marcados()
    traducciones = {texto: traduccion for texto, traduccion in leer_po(ruta).items() if traduccion}
    if opciones.agregar:
        nuevas = json.loads(pathlib.Path(opciones.agregar).read_text(encoding="utf-8"))
        por_texto = {desescapar(texto): texto for texto in marcados}
        sin_uso = []
        for espanol, traduccion in nuevas.items():
            crudo = por_texto.get(espanol)
            if crudo is None:
                sin_uso.append(espanol)
            else:
                traducciones[crudo] = escapar(traduccion)
        if sin_uso:
            print(f"{len(sin_uso)} traducciones no corresponden a ningun texto marcado:", file=sys.stderr)
            for texto in sin_uso[:20]:
                print("  " + texto, file=sys.stderr)
    escribir_po(ruta, opciones.idioma, marcados, traducciones)
    faltan = [texto for texto in marcados if texto not in traducciones]
    if opciones.faltantes:
        pathlib.Path(opciones.faltantes).write_text(
            json.dumps({desescapar(texto): "" for texto in faltan}, ensure_ascii=False, indent=1), encoding="utf-8"
        )
    print(f"{len(marcados)} textos, {len(faltan)} sin traducir -> {ruta.relative_to(RAIZ)}")


if __name__ == "__main__":
    main()
