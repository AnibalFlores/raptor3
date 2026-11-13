#!/usr/bin/env python3
"""Genera con ElevenLabs las locuciones pregrabadas de hora, temperatura y humedad.

El anuncio se arma después, sin internet, encadenando cuatro piezas:

    «Son las quince» + «y treinta.» + «Temperatura: veintidós grados.»
    + «Humedad: sesenta y cinco por ciento.»

Cada pieza es una frase completa (no palabras sueltas) para que la entonación
suene natural al unirlas. Quedan en la carpeta de salida así:

    hora/00.mp3 … hora/23.mp3
    minutos/00.mp3 … minutos/59.mp3      (00 = «en punto»)
    temperatura/-10.mp3 … temperatura/45.mp3
    humedad/000.mp3 … humedad/100.mp3
    locuciones.json                      (índice: texto y archivo de cada pieza)

Uso:
    python generar_locuciones.py --simular          # lista textos y caracteres, sin llamar a la API
    python generar_locuciones.py                    # genera lo que falte (se puede cortar y retomar)
    python generar_locuciones.py --forzar           # vuelve a generar todo
    python generar_locuciones.py --solo hora minutos

La clave y la voz pueden reemplazarse en las constantes de abajo o, mejor, con
las variables de entorno ELEVENLABS_API_KEY y ELEVENLABS_VOICE_ID.
Solo usa la biblioteca estándar de Python 3.9+.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

# --- Configuración: la clave nunca va en el código, solo en el entorno -----------

API_KEY = os.environ.get("ELEVENLABS_API_KEY", "")
VOICE_ID = os.environ.get("ELEVENLABS_VOICE_ID", "Je8d8oi82sj0l8L1VM0l")

MODELO = "eleven_multilingual_v2"
# MP3 a 44,1 kHz: Raptor lo decodifica sin conversiones extra.
FORMATO_DE_SALIDA = "mp3_44100_128"
# Voz estable y semilla fija para que todas las piezas suenen parejas.
AJUSTES_DE_VOZ = {
    "stability": 0.6,
    "similarity_boost": 0.8,
    "style": 0.0,
    "use_speaker_boost": True,
}
SEMILLA = 1997

TEMPERATURA_MINIMA = -10
TEMPERATURA_MAXIMA = 45

URL_DE_LA_API = "https://api.elevenlabs.io/v1/text-to-speech/{voz}?output_format={formato}"
REINTENTOS = 5
PAUSA_ENTRE_PEDIDOS_SEGUNDOS = 0.3

# --- Números en palabras ----------------------------------------------------------

_HASTA_VEINTINUEVE = [
    "cero", "uno", "dos", "tres", "cuatro", "cinco", "seis", "siete", "ocho", "nueve",
    "diez", "once", "doce", "trece", "catorce", "quince", "dieciséis", "diecisiete",
    "dieciocho", "diecinueve", "veinte", "veintiuno", "veintidós", "veintitrés",
    "veinticuatro", "veinticinco", "veintiséis", "veintisiete", "veintiocho", "veintinueve",
]
_DECENAS = {3: "treinta", 4: "cuarenta", 5: "cincuenta", 6: "sesenta",
            7: "setenta", 8: "ochenta", 9: "noventa"}


def numero_en_palabras(numero: int, concordancia: str = "neutra") -> str:
    """Escribe un número de 0 a 100 en palabras.

    concordancia:
      "neutra"    → uno, veintiuno, treinta y uno   (minutos, «uno por ciento»)
      "masculina" → un, veintiún, treinta y un      (antes de «grados»)
      "femenina"  → una, veintiuna, treinta y una   (las horas)
    """
    if not 0 <= numero <= 100:
        raise ValueError(f"Fuera de rango: {numero}")
    if numero == 100:
        return "cien"
    if numero < 30:
        palabra = _HASTA_VEINTINUEVE[numero]
    else:
        decena, unidad = divmod(numero, 10)
        palabra = _DECENAS[decena] + ("" if unidad == 0 else " y " + _HASTA_VEINTINUEVE[unidad])

    if concordancia == "masculina":
        if palabra.endswith("veintiuno"):
            return palabra[:-len("veintiuno")] + "veintiún"
        if palabra.endswith("uno"):
            return palabra[:-1]
    elif concordancia == "femenina" and palabra.endswith("uno"):
        return palabra[:-1] + "a"
    return palabra


# --- Piezas a generar -------------------------------------------------------------

@dataclass(frozen=True)
class Locucion:
    categoria: str
    valor: int
    texto: str
    archivo: str
    # Contexto que se le pasa a ElevenLabs (no se pronuncia): ayuda a que la
    # entonación encaje con la pieza que va antes o después al encadenarlas.
    texto_previo: str | None = None
    texto_siguiente: str | None = None


def texto_de_hora(hora: int) -> str:
    if hora == 1:
        return "Es la una"
    return "Son las " + numero_en_palabras(hora, "femenina")


def texto_de_minutos(minutos: int) -> str:
    if minutos == 0:
        return "en punto."
    return "y " + numero_en_palabras(minutos) + "."


def texto_de_temperatura(grados: int) -> str:
    if grados == 0:
        return "Temperatura: cero grados."
    magnitud = numero_en_palabras(abs(grados), "masculina")
    unidad = "grado" if abs(grados) == 1 else "grados"
    sufijo = " bajo cero" if grados < 0 else ""
    return f"Temperatura: {magnitud} {unidad}{sufijo}."


def texto_de_humedad(porcentaje: int) -> str:
    return f"Humedad: {numero_en_palabras(porcentaje)} por ciento."


def todas_las_locuciones() -> list[Locucion]:
    locuciones: list[Locucion] = []
    for hora in range(24):
        locuciones.append(Locucion("hora", hora, texto_de_hora(hora), f"hora/{hora:02d}.mp3",
                                   texto_siguiente="y treinta."))
    for minutos in range(60):
        locuciones.append(Locucion("minutos", minutos, texto_de_minutos(minutos),
                                   f"minutos/{minutos:02d}.mp3", texto_previo="Son las diez"))
    for grados in range(TEMPERATURA_MINIMA, TEMPERATURA_MAXIMA + 1):
        locuciones.append(Locucion("temperatura", grados, texto_de_temperatura(grados),
                                   f"temperatura/{grados}.mp3",
                                   texto_previo="Son las diez y cinco."))
    for porcentaje in range(101):
        locuciones.append(Locucion("humedad", porcentaje, texto_de_humedad(porcentaje),
                                   f"humedad/{porcentaje:03d}.mp3",
                                   texto_previo="Temperatura: veinte grados."))
    return locuciones


# --- Llamada a ElevenLabs ---------------------------------------------------------

class ErrorFatal(Exception):
    """Error que no se arregla reintentando (clave inválida, voz inexistente, sin crédito)."""


def sintetizar(locucion: Locucion) -> bytes:
    cuerpo = {
        "text": locucion.texto,
        "model_id": MODELO,
        "voice_settings": AJUSTES_DE_VOZ,
        "seed": SEMILLA,
    }
    if locucion.texto_previo:
        cuerpo["previous_text"] = locucion.texto_previo
    if locucion.texto_siguiente:
        cuerpo["next_text"] = locucion.texto_siguiente

    pedido = urllib.request.Request(
        URL_DE_LA_API.format(voz=VOICE_ID, formato=FORMATO_DE_SALIDA),
        data=json.dumps(cuerpo).encode("utf-8"),
        method="POST",
        headers={
            "xi-api-key": API_KEY,
            "Content-Type": "application/json",
            "Accept": "audio/mpeg",
        },
    )

    espera = 2.0
    for intento in range(1, REINTENTOS + 1):
        try:
            with urllib.request.urlopen(pedido, timeout=60) as respuesta:
                return respuesta.read()
        except urllib.error.HTTPError as error:
            detalle = error.read().decode("utf-8", errors="replace")[:300]
            # 429: demasiados pedidos; 5xx: falla temporal del servicio.
            if error.code == 429 or error.code >= 500:
                print(f"    HTTP {error.code}, reintento {intento}/{REINTENTOS} en {espera:.0f} s")
            else:
                raise ErrorFatal(f"HTTP {error.code}: {detalle}") from error
        except (urllib.error.URLError, TimeoutError) as error:
            print(f"    Sin conexión ({error}), reintento {intento}/{REINTENTOS} en {espera:.0f} s")
        time.sleep(espera)
        espera *= 2
    raise ErrorFatal(f"No se pudo generar «{locucion.texto}» tras {REINTENTOS} intentos")


def guardar(destino: Path, audio: bytes) -> None:
    """Escribe primero un temporal y lo renombra, para no dejar MP3 a medias si se corta."""
    destino.parent.mkdir(parents=True, exist_ok=True)
    temporal = destino.with_suffix(".parcial")
    temporal.write_bytes(audio)
    temporal.replace(destino)


def escribir_indice(carpeta: Path, locuciones: list[Locucion]) -> None:
    indice = {
        "modelo": MODELO,
        "voz": VOICE_ID,
        "formato": FORMATO_DE_SALIDA,
        "orden": ["hora", "minutos", "temperatura", "humedad"],
        "piezas": [
            {"categoria": l.categoria, "valor": l.valor, "texto": l.texto, "archivo": l.archivo}
            for l in locuciones
            if (carpeta / l.archivo).exists()
        ],
    }
    (carpeta / "locuciones.json").write_text(
        json.dumps(indice, ensure_ascii=False, indent=2), encoding="utf-8")


# --- Programa principal -----------------------------------------------------------

def main() -> int:
    argumentos = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    argumentos.add_argument("--carpeta", type=Path, default=Path("locuciones"),
                            help="carpeta de salida (por defecto ./locuciones)")
    argumentos.add_argument("--solo", nargs="+",
                            choices=["hora", "minutos", "temperatura", "humedad"],
                            help="generar solo estas categorías")
    argumentos.add_argument("--forzar", action="store_true",
                            help="regenerar aunque el archivo ya exista")
    argumentos.add_argument("--simular", action="store_true",
                            help="mostrar textos y caracteres sin llamar a la API")
    opciones = argumentos.parse_args()

    todas = todas_las_locuciones()
    elegidas = [l for l in todas if not opciones.solo or l.categoria in opciones.solo]
    pendientes = [l for l in elegidas
                  if opciones.forzar or not (opciones.carpeta / l.archivo).exists()]
    caracteres = sum(len(l.texto) for l in pendientes)

    print(f"{len(pendientes)} de {len(elegidas)} piezas por generar, {caracteres} caracteres "
          f"(ElevenLabs cobra por carácter).")

    if opciones.simular:
        for locucion in pendientes:
            print(f"  {locucion.archivo:<24} {locucion.texto}")
        return 0

    if not API_KEY or not VOICE_ID:
        print("Falta configurar la clave o la voz: definí ELEVENLABS_API_KEY y "
              "ELEVENLABS_VOICE_ID en el entorno (la clave no se guarda en el script).", file=sys.stderr)
        return 2

    generadas = 0
    try:
        for numero, locucion in enumerate(pendientes, start=1):
            print(f"[{numero}/{len(pendientes)}] {locucion.archivo}: {locucion.texto}")
            guardar(opciones.carpeta / locucion.archivo, sintetizar(locucion))
            generadas += 1
            time.sleep(PAUSA_ENTRE_PEDIDOS_SEGUNDOS)
    except ErrorFatal as error:
        print(f"Se detuvo: {error}", file=sys.stderr)
        print("Lo ya generado quedó guardado; al volver a ejecutar se retoma desde ahí.",
              file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\nInterrumpido; al volver a ejecutar se retoma desde ahí.", file=sys.stderr)
        return 130
    finally:
        opciones.carpeta.mkdir(parents=True, exist_ok=True)
        escribir_indice(opciones.carpeta, todas)

    print(f"Listo: {generadas} piezas nuevas en {opciones.carpeta.resolve()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
