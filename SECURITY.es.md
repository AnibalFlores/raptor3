# Política de seguridad

**Español** · [English](SECURITY.md)

## Versiones soportadas

Solo la **última versión publicada** en [Releases](https://github.com/AnibalFlores/raptor3/releases) recibe correcciones de seguridad. Si usa una versión anterior, actualícela antes de reportar.

## Cómo reportar una vulnerabilidad

**No abra un issue público** por problemas de seguridad. Repórtelos en privado por alguno de estos medios:

1. **Preferido:** el botón [**Report a vulnerability**](https://github.com/AnibalFlores/raptor3/security/advisories/new) de la pestaña *Security* del repositorio (reporte privado de vulnerabilidades).
2. Por correo a **jxlfybcrk@mozmail.com**, con el asunto `Raptor 3 seguridad`.

Incluya, por favor:

- La versión afectada y el sistema operativo.
- Una descripción del problema y de su impacto.
- Los pasos para reproducirlo, o una prueba de concepto.
- Si la tiene, una propuesta de corrección.

## Qué esperar

Raptor 3 lo mantiene una sola persona en su tiempo libre y no tiene soporte formal, así que los tiempos de respuesta son los posibles:

- Confirmación de que se recibió el reporte: por lo general, dentro de los **7 días**.
- Después de evaluarlo, se le informa si se acepta como vulnerabilidad y, en ese caso, el plan para corregirla.
- Una vez publicada la corrección, el problema se da a conocer en un aviso de seguridad (*security advisory*); se le da el crédito, salvo que prefiera quedar en el anonimato.

Por favor, dé un tiempo razonable para publicar la corrección antes de dar a conocer el problema.

## Alcance y decisiones de diseño

Estos comportamientos son intencionales y **no** se consideran vulnerabilidades por sí mismos:

- **El servidor MCP no tiene contraseña.** Escucha solo en `127.0.0.1` (puerto `8765`, o el de `RAPTOR_MCP_PUERTO`) y rechaza los pedidos con un encabezado `Origin` de páginas web. Cualquiera que pueda ejecutar programas en la misma computadora puede manejar el reproductor. No lo exponga a la red ni a internet (por ejemplo, con un túnel o una redirección de puertos).
- **El ejecutable de Windows no está firmado digitalmente**, así que SmartScreen puede advertir. Descárguelo solo desde los Releases de este repositorio.
- **Servicios externos:** el programa consulta Open-Meteo (clima) y, solo cuando el usuario lo pide, MusicBrainz (metadatos), sin enviar datos personales.

Estos **sí** interesan, por ejemplo:

- Cualquier forma de llegar al servidor MCP desde otra computadora o desde una página web.
- Leer o escribir archivos fuera de lo que eligió el usuario, por medio del servidor MCP o de listas, archivos del catálogo o archivos de audio armados a propósito.
- Cierres inesperados o ejecución de código causados por archivos de audio o de datos maliciosos.
- Claves o datos personales que terminen en el repositorio, en las bitácoras o en los paquetes publicados.

## Claves en el repositorio

El proyecto nunca guarda claves de API en el código: la clave y la voz de ElevenLabs que usa `herramientas/generar_locuciones.py` se leen solo del entorno. Si encuentra una clave en el repositorio o en su historial, repórtela en privado por los medios de arriba.
