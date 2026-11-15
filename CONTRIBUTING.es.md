# Cómo contribuir a Raptor 3

**Español** · [English](CONTRIBUTING.md)

Gracias por su interés en Raptor 3. Los aportes son bienvenidos por medio de *issues* y *pull requests*. El proyecto lo mantiene una sola persona en su tiempo libre, así que las revisiones pueden demorar, y un pull request puede no aceptarse si no encaja con el rumbo del proyecto.

## Cómo está organizado el proyecto

- **La rama `main` está protegida.** Los cambios de quienes contribuyen entran por pull request; nadie puede hacer push directo, forzar el push ni borrarla.
- **Cualquier persona puede abrir issues y pull requests** desde su propio fork.
- **Los colaboradores** son personas que aportan de forma habitual y a quienes invita el responsable del proyecto. Pueden crear ramas en este repositorio y revisar pull requests, pero sus cambios también entran por pull request.
- **Solo el responsable** publica versiones (etiquetas `v*` y releases).
- **Los workflows de los pull requests que vienen de forks** corren solo después de que el responsable los aprueba.

## Antes de empezar

- **Errores:** abra un issue con la versión (*Ayuda › Acerca de*), el sistema operativo, los pasos para reproducirlo y lo que esperaba que pasara. Si puede, adjunte la bitácora (`~/.mcp-raptor-player/logs/raptor.log`).
- **Ideas y funciones nuevas:** abra un issue para conversarlas **antes** de escribir código, así no invierte tiempo en algo que quizás no se acepte.
- **Problemas de seguridad:** no abra un issue público. Siga [SECURITY.es.md](SECURITY.es.md).

## Forma de trabajo

1. Haga un fork del repositorio y cree una rama desde `main` con un nombre descriptivo, por ejemplo `fix/silencio-en-crossfade` o `feat/podcast-programado`.
2. Haga los cambios, con un solo tema por pull request.
3. Compruebe que todo compila y que las pruebas pasan (ver más abajo).
4. Abra un pull request hacia `main` explicando **qué** cambia y **por qué**. Si cambia la interfaz, incluya capturas de antes y de después.
5. Responda los comentarios de la revisión. El responsable lo incorpora cuando esté listo.

### Mensajes de commit

Use un prefijo corto y describa el cambio:

- `[FEAT]` función nueva
- `[FIX]` corrección de un error
- `[DOCS]` documentación o manual
- `[REFACTOR]` cambio de código que no altera el comportamiento
- `[TEST]` pruebas
- `[BUILD]` compilación, dependencias o CI

Ejemplo: `[FIX] El crossfade ya no corta la outro de los temas cortos`.

## Convenciones del código

- **Idioma:** el código está escrito en **español**: nombres de tipos, funciones y variables, comentarios y comentarios de documentación. Por favor, manténgalo así, con nombres descriptivos.
- **Estilo:** siga el estilo del código que rodea su cambio. **No reformatee código que no modificó** (el proyecto todavía no tiene configuración de `rustfmt`, así que `cargo fmt` reescribiría archivos enteros).
- **Diseño:** prefiera funciones chicas y con un solo propósito, y mantenga la lógica del dominio (`src/dominio`, `src/servicio`) separada de la interfaz (`src/interfaz`, `ui/`).
- **Rendimiento:** nunca bloquee el hilo de audio ni la interfaz; las tareas largas van en su propio hilo.
- **Dependencias:** evite agregar nuevas salvo que hagan falta de verdad, y explique por qué en el pull request. Sus licencias deben ser compatibles con GPLv3.

## Interfaz y traducciones

- Todo texto que ve el usuario se escribe en español y se marca para traducir: `@tr("...")` en los archivos `.slint` y `tr!("...")` en Rust.
- Agregue la traducción al inglés en `lang/en/LC_MESSAGES/mcp-raptor-player.po`. `python herramientas/traducciones.py --faltantes faltantes.json` lista las que faltan; una prueba falla si falta alguna.
- Compruebe que las ventanas sigan entrando en las resoluciones soportadas (Full HD al 100 % y al 150 %) generando las capturas:

```bash
cargo test capturas -- --ignored --test-threads=1
```

- Los iconos nuevos salen de [Tabler Icons](https://tabler.io/icons) (MIT). Cualquier otra imagen debe declarar su licencia en [ASSETS-LICENSES.md](ASSETS-LICENSES.md).

## Pruebas

```bash
cargo test
```

- Todas las pruebas deben pasar en **Windows y Linux** (el CI comprueba los dos).
- Agregue pruebas para cada corrección o función nueva en las capas de dominio o de servicio.
- Las pruebas deben usar carpetas temporales con un nombre único y nunca tocar los datos del usuario. Para probar el programa a mano, defina `RAPTOR_CARPETA_DE_DATOS` con una carpeta de prueba.

## Documentación

Si cambia algo que el usuario ve, actualice el manual en los dos idiomas (`docs/manual/Raptor_Manual_es.html` y `Raptor_Manual_en.html`) y, si corresponde, los README. El README explica cómo regenerar las capturas y los PDF.

## Qué no incluir

- Claves de API, tokens o contraseñas (la clave de ElevenLabs se lee solo del entorno).
- Datos personales o rutas reales de su computadora.
- Archivos de audio: las locuciones del HTH y las bases musicales no forman parte del repositorio.
- Archivos generados: `target/`, `graphify-out/`.

## Licencia

Al contribuir, acepta que sus aportes se publiquen bajo la [GNU GPLv3 o posterior](LICENSE), la misma licencia del proyecto. Las imágenes siguen lo indicado en [ASSETS-LICENSES.md](ASSETS-LICENSES.md).
