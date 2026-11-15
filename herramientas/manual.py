#!/usr/bin/env python3
"""Actualiza las imagenes y los PDF del manual en español y en inglés.

Pasos:

1. Las capturas de las ventanas se generan con las pruebas (en español y en
   inglés):

       cargo test capturas -- --ignored --test-threads=1
       RAPTOR_CAPTURAS_IDIOMA=en cargo test capturas -- --ignored --test-threads=1

2. Este script copia las que usa el manual a docs/manual/imagenes (español)
   y docs/manual/imagenes/en (inglés), y arma los PDF con Chrome o Edge sin
   ventana:

       python herramientas/manual.py            # imagenes y PDF
       python herramientas/manual.py --pdf      # solo los PDF
       python herramientas/manual.py --imagenes # solo las imagenes

Solo usa la biblioteca estándar de Python 3.9+.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
CAPTURAS = RAIZ / "target" / "capturas"
MANUAL = RAIZ / "docs" / "manual"
IDIOMAS = ("es", "en")

# Imagen del manual -> captura que la genera (en target/capturas o en /en).
IMAGENES = {
    "ventana-principal.png": "principal-fhd-100.png",
    "ventana-con-efectos.png": "principal-fhd-100-botonera.png",
    "editor-solapa0.png": "editor-inicial-solapa0.png",
    "editor-solapa1.png": "editor-inicial-solapa1.png",
    "editor-solapa2.png": "editor-inicial-solapa2.png",
    "buscador.png": "buscador-inicial.png",
    "mantenimiento.png": "mantenimiento-inicial.png",
    "pauta.png": "pauta-inicial.png",
    "ordenes.png": "ordenes-inicial.png",
    "avisos.png": "avisos-inicial.png",
    "anunciantes.png": "anunciantes-inicial.png",
    "localidad.png": "localidad-inicial.png",
    "actualizacion-etiquetas.png": "actualizacion-etiquetas.png",
    "actualizacion-musicbrainz.png": "actualizacion-musicbrainz.png",
}

# Navegadores que imprimen a PDF sin ventana, en orden de preferencia.
NAVEGADORES = [
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    "google-chrome",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
]


def copiar_imagenes() -> None:
    for idioma in IDIOMAS:
        origen = CAPTURAS if idioma == "es" else CAPTURAS / idioma
        destino = MANUAL / "imagenes" if idioma == "es" else MANUAL / "imagenes" / idioma
        destino.mkdir(parents=True, exist_ok=True)
        faltan = [captura for captura in IMAGENES.values() if not (origen / captura).is_file()]
        if faltan:
            sys.exit(f"Faltan capturas en {origen}: {', '.join(faltan)}. Genérelas primero (ver la ayuda del script).")
        for imagen, captura in IMAGENES.items():
            shutil.copyfile(origen / captura, destino / imagen)
        print(f"{len(IMAGENES)} imagen(es) -> {destino.relative_to(RAIZ)}")


def navegador() -> str:
    for candidato in NAVEGADORES:
        if os.path.isabs(candidato) and Path(candidato).is_file():
            return candidato
        encontrado = shutil.which(candidato)
        if encontrado:
            return encontrado
    sys.exit("No se encontró Chrome, Chromium ni Edge para generar los PDF.")


def generar_pdf() -> None:
    programa = navegador()
    for idioma in IDIOMAS:
        html = MANUAL / f"Raptor_Manual_{idioma}.html"
        pdf = MANUAL / f"Raptor_Manual_{idioma}.pdf"
        # Un perfil temporal evita chocar con una ventana del navegador abierta.
        with tempfile.TemporaryDirectory() as perfil:
            subprocess.run(
                [
                    programa,
                    "--headless=new",
                    "--disable-gpu",
                    "--no-pdf-header-footer",
                    f"--user-data-dir={perfil}",
                    f"--print-to-pdf={pdf}",
                    html.as_uri(),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=180,
            )
        if not pdf.is_file() or pdf.stat().st_size < 10_000:
            sys.exit(f"No se generó {pdf}")
        print(f"{pdf.relative_to(RAIZ)}: {pdf.stat().st_size // 1024} KB")


def main() -> int:
    argumentos = argparse.ArgumentParser(description="Imagenes y PDF del manual en español y en inglés.")
    argumentos.add_argument("--imagenes", action="store_true", help="solo copia las imagenes")
    argumentos.add_argument("--pdf", action="store_true", help="solo genera los PDF")
    opciones = argumentos.parse_args()
    todo = not opciones.imagenes and not opciones.pdf
    if todo or opciones.imagenes:
        copiar_imagenes()
    if todo or opciones.pdf:
        generar_pdf()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
