//! Herramientas MCP para que un agente consulte y maneje el reproductor: la
//! lista y sus datos, los botones de transporte, el selector, los
//! deslizantes y opciones, y la botonera de efectos.
//!
//! El estado vive en el controlador de la ventana, asi que toda operacion se
//! traslada al hilo de la interfaz; las validaciones de disco se hacen antes,
//! en el hilo del servidor, para no bloquear la interfaz.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde_json::{Map, Value, json};

use crate::audio::{FormaDeOnda, anio_valido};
use crate::dominio::{
    EXTENSIONES_SOPORTADAS, Frecuencia, ModoDeEntrada, OrdenDePublicidad, PRIORIDAD_MAXIMA, PRIORIDAD_MINIMA, PRIORIDAD_POR_DEFECTO, Pista, Recurrencia, TOLERANCIA_POSTERIOR_POR_DEFECTO, TOLERANCIA_PREVIA_POR_DEFECTO, TandaProgramada, TipoDeElemento,
    parsear_dias, parsear_hora, audios_de_carpeta, PuntosDeMezcla, VentanaDeOverlay, formatear_tiempo, formatear_ventana, normalizar,
    parsear_tiempo, simplificar_texto,
};
use crate::interfaz::{AccionDeHoja, AccionDeTransporte, Ajustes, Controlador, ejecutar_en_interfaz, formatear_minutos};
use crate::servicio::{DatosDeMezcla, Localidad, LocalidadEncontrada, ObjetivoDeSonoridad, RANGO_DINAMICO_AMPLIO, buscar_localidades};
use crate::persistencia::{
    BPM_MAXIMO, BPM_MINIMO, CANTIDAD_DE_BOTONES, CambiosDeDatos, FADER_MAXIMO, FichaDeCancion, RANKING_MAXIMO,
    LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA, carpeta_de_listas, carpeta_del_usuario,
};

const LISTAR: &str = "listar_pistas";
const AGREGAR: &str = "agregar_pistas";
const QUITAR: &str = "quitar_pistas";
const DEFINIR_MEZCLA: &str = "definir_mezcla";
const DEFINIR_DATOS: &str = "definir_datos";
const ESTADO: &str = "estado_del_reproductor";
const CONTROLAR: &str = "controlar_reproduccion";
const SELECCIONAR: &str = "seleccionar_pista";
const AJUSTAR: &str = "ajustar_controles";
const ASIGNAR_EFECTO: &str = "asignar_efecto";
const QUITAR_EFECTO: &str = "quitar_efecto";
const DISPARAR_EFECTO: &str = "disparar_efecto";
const DETENER_EFECTO: &str = "detener_efecto";
const ABRIR_EDITOR: &str = "abrir_editor";
const SUGERIR: &str = "sugerir_puntos";
const MOVER: &str = "mover_pista";
const ORDENAR: &str = "ordenar_lista";
const GUARDAR_LISTA: &str = "guardar_lista";
const LOCALIDAD_DEL_CLIMA: &str = "localidad_del_clima";
const HOJAS: &str = "hojas_de_efectos";
const INSERTAR_HTH: &str = "insertar_hth";
const BASE_HTH: &str = "base_hth";
const BLOQUE_FIJO: &str = "bloque_fijo";
const BUSCAR_EN_CATALOGO: &str = "buscar_en_catalogo";
const ENCOLAR: &str = "encolar_pista";
const PROGRAMAR_TANDA: &str = "programar_tanda";
const LISTAR_TANDAS: &str = "listar_tandas";
const QUITAR_TANDA: &str = "quitar_tanda";
const ENCOLAR_TANDA: &str = "encolar_tanda";
const REGISTRO_DE_COMERCIALES: &str = "registro_de_comerciales";
const PROGRAMAR_ORDEN: &str = "programar_orden";
const LISTAR_ORDENES: &str = "listar_ordenes";
const QUITAR_ORDEN: &str = "quitar_orden";
const PAUTA_DEL_DIA: &str = "pauta_del_dia";
const CERTIFICADO: &str = "certificado_de_emision";
const REGISTRAR_ANUNCIANTE: &str = "registrar_anunciante";
const LISTAR_ANUNCIANTES: &str = "listar_anunciantes";
const QUITAR_ANUNCIANTE: &str = "quitar_anunciante";

const HERRAMIENTAS: &[&str] = &[
    LISTAR,
    AGREGAR,
    QUITAR,
    DEFINIR_MEZCLA,
    DEFINIR_DATOS,
    ESTADO,
    CONTROLAR,
    SELECCIONAR,
    AJUSTAR,
    ASIGNAR_EFECTO,
    QUITAR_EFECTO,
    DISPARAR_EFECTO,
    DETENER_EFECTO,
    ABRIR_EDITOR,
    SUGERIR,
    MOVER,
    ORDENAR,
    GUARDAR_LISTA,
    HOJAS,
    INSERTAR_HTH,
    BASE_HTH,
    LOCALIDAD_DEL_CLIMA,
    BLOQUE_FIJO,
    BUSCAR_EN_CATALOGO,
    ENCOLAR,
    PROGRAMAR_TANDA,
    LISTAR_TANDAS,
    ENCOLAR_TANDA,
    QUITAR_TANDA,
    REGISTRO_DE_COMERCIALES,
    PROGRAMAR_ORDEN,
    LISTAR_ORDENES,
    QUITAR_ORDEN,
    PAUTA_DEL_DIA,
    CERTIFICADO,
    REGISTRAR_ANUNCIANTE,
    LISTAR_ANUNCIANTES,
    QUITAR_ANUNCIANTE,
];

static DEFINICIONES: LazyLock<Value> = LazyLock::new(|| {
    let formatos = EXTENSIONES_SOPORTADAS.join(", ");
    let ultimo_boton = CANTIDAD_DE_BOTONES - 1;
    let pista_por_posicion_o_ruta = json!({
        "posicion": {"type": "integer", "minimum": 1, "description": "Posicion en la lista, empezando en 1"},
        "ruta": {"type": "string", "description": "Ruta absoluta de la pista"}
    });
    let boton = json!({"type": "integer", "minimum": 0, "maximum": ultimo_boton,
                       "description": format!("Numero del boton dentro de la hoja (0 a {ultimo_boton})")});
    let hoja = json!({"type": "integer", "minimum": 1,
                      "description": "Hoja de efectos, empezando en 1; si se omite, la que esta a la vista"});
    let con = |base: &Value, extra: Value| {
        let mut propiedades = base.as_object().cloned().unwrap_or_default();
        propiedades.extend(extra.as_object().cloned().unwrap_or_default());
        Value::Object(propiedades)
    };
    json!({"tools": [
        {
            "name": LISTAR,
            "description": "Devuelve las pistas de la lista con su posicion (empezando en 1), nombre, ruta, tipo, titulo, artista, genero, duracion, ranking (0 a 5), ultima reproduccion, inicio y fin del tema, intro, outro (indicando si son automaticos o manuales), ventana de overlay (inicio-fin, sugerida o manual) y los segundos de mezcla con la siguiente; indica cual esta sonando.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": AGREGAR,
            "description": format!("Agrega archivos de audio al final de la lista. Acepta rutas absolutas a archivos o a carpetas; de una carpeta se agregan los archivos de audio que contiene directamente (con 'subcarpetas', tambien los de todas sus subcarpetas), ordenados por nombre. Formatos: {formatos}. Las pistas ya presentes se ignoran. Se insertan despues de la pista seleccionada (o al final si no hay seleccion), salvo que se indique 'posicion'. Titulo, artista, genero y duracion se leen del archivo en segundo plano; despues se analiza cada tema y se le ponen solos el inicio y el fin (sin silencios), la intro, la outro y la ventana de overlay, salvo los que el usuario fijo a mano."),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "rutas": {"type": "array", "items": {"type": "string"}, "minItems": 1,
                              "description": "Rutas absolutas de archivos o carpetas"},
                    "subcarpetas": {"type": "boolean",
                                    "description": "true para agregar tambien los archivos de las subcarpetas de cada carpeta (por defecto, solo los que contiene directamente)"},
                    "posicion": {"type": "integer", "minimum": 1,
                                 "description": "Posicion (desde 1) donde empiezan las pistas nuevas"}
                },
                "required": ["rutas"]
            }
        },
        {
            "name": QUITAR,
            "description": "Quita pistas de la lista, indicadas por posicion (la que devuelve listar_pistas) o por ruta. Si se quita la pista que esta sonando, la reproduccion se detiene. No borra archivos del disco.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "posiciones": {"type": "array", "items": {"type": "integer", "minimum": 1},
                                   "description": "Posiciones en la lista, empezando en 1"},
                    "rutas": {"type": "array", "items": {"type": "string"},
                              "description": "Rutas absolutas de las pistas a quitar"}
                }
            },
            "annotations": {"destructiveHint": false}
        },
        {
            "name": DEFINIR_MEZCLA,
            "description": "Define el inicio y el fin del tema, la intro, la outro y/o la ventana de overlay de una pista de la lista, indicada por posicion o por ruta. Los tiempos se dan en segundos (80.5) o como texto m:ss (1:20.5). Todos salen solos del analisis del tema; lo que se fija aqui queda como manual y el analisis ya no lo cambia. 'automatico' devuelve inicio, fin, intro y outro a lo que propone el analisis, y 'overlay_automatico' hace lo mismo con la ventana. El tema suena entre su inicio y su fin (lo de afuera, normalmente silencio, se saltea). La intro se mide desde el comienzo del archivo y la outro hasta su final; entre dos pistas consecutivas se mezclan tantos segundos como el menor entre la outro de la que termina y la intro de la que entra, descontando lo que se saltea; 0 desactiva la mezcla de ese lado. La ventana de overlay (overlay_inicio y overlay_fin, medidos desde el comienzo del archivo) es el tramo dentro del cual un efecto de la botonera en modo trabado puede sonar entero: si se pulsa antes, espera a que se abra; si ya no cabe, espera la ventana de la pista siguiente. Sin ventana, el efecto suena enseguida. Los valores se recuerdan por archivo.",
            "inputSchema": {
                "type": "object",
                "properties": con(&pista_por_posicion_o_ruta, json!({
                    "inicio": {"type": ["number", "string"], "description": "Donde empieza a sonar el tema"},
                    "fin": {"type": ["number", "string"], "description": "Donde deja de sonar el tema"},
                    "intro": {"type": ["number", "string"], "description": "Duracion de la intro desde el comienzo del archivo"},
                    "outro": {"type": ["number", "string"], "description": "Duracion de la outro hasta el final del archivo"},
                    "automatico": {"type": "boolean", "description": "true para volver inicio, fin, intro y outro a lo que propone el analisis"},
                    "overlay_automatico": {"type": "boolean", "description": "true para usar la ventana de overlay que sugiere el analisis (o ninguna, si no sugiere)"},
                    "overlay_inicio": {"type": ["number", "string"], "description": "Comienzo de la ventana de overlay"},
                    "overlay_fin": {"type": ["number", "string"], "description": "Fin de la ventana de overlay"},
                    "quitar_overlay": {"type": "boolean", "description": "true para dejar la pista sin ventana de overlay"}
                }))
            }
        },
        {
            "name": DEFINIR_DATOS,
            "description": format!("Clasifica una pista por su tipo (Tema, Tema editado, Comercial, Anuncio, Publicidad, Podcast, Efecto o Seco; vacio la deja sin clasificar), corrige el titulo, el artista, el genero, el album, el anio o la nota de una pista (indicada por posicion o por ruta; por ruta sirve tambien para un efecto de la botonera o un tema que no esta en la lista), le pone un ranking de 0 a {RANKING_MAXIMO}, fija su BPM ({BPM_MINIMO} a {BPM_MAXIMO}; 0 vuelve al que estima el analisis) o le fija un efecto que suena solo con el tema, en su ventana de overlay. Un texto vacio (o anio 0) borra el dato. Se recuerda por archivo. El tipo decide como se trata: un Tema o un Tema editado (con locucion o edicion para la radio) se mezcla; los demas suenan sin mezcla, pueden repetirse y no admiten overlay ni efecto fijado; un Efecto solo va en la botonera; en las tandas van Comercial, Publicidad, Anuncio, Podcast (programa pregrabado que se pauta a una hora) o Seco; Comercial y Publicidad se anotan en el registro de comerciales. El genero es libre (estilo musical); si la pista esta sin clasificar y se indica un genero que nombra un tipo (\"Comercial\"), toma ese tipo."),
            "inputSchema": {
                "type": "object",
                "properties": con(&pista_por_posicion_o_ruta, json!({
                    "tipo": {"type": "string", "enum": ["", "Tema", "Tema editado", "Comercial", "Anuncio", "Publicidad", "Podcast", "Efecto", "Seco"], "description": "Vacio: sin clasificar (suena como un Tema)"},
                    "titulo": {"type": "string"},
                    "artista": {"type": "string"},
                    "genero": {"type": "string", "description": "Estilo musical, libre"},
                    "album": {"type": "string"},
                    "nota": {"type": "string", "description": "Nota u observacion libre sobre el tema o elemento; se ve en el tooltip del titulo en la lista y del boton de efecto. Vacio la borra"},
                    "anio": {"type": "integer", "minimum": 0, "maximum": 2999, "description": "Anio del tema; 0 lo borra"},
                    "efecto_fijado": {"type": "string", "description": "Ruta absoluta del efecto que suena con el tema; vacio lo quita"},
                    "ranking": {"type": "integer", "minimum": 0, "maximum": RANKING_MAXIMO},
                    "bpm": {"type": "number", "minimum": 0, "maximum": BPM_MAXIMO}
                }))
            }
        },
        {
            "name": ESTADO,
            "description": "Devuelve el estado del reproductor: si reproduce, esta en pausa o detenido; la pista actual con su posicion y tiempo; la proxima pista del boton Siguiente; volumen, fader, repetir lista, opciones de la botonera, el archivo de cada boton de efectos y el texto de la barra de estado.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": CONTROLAR,
            "description": "Acciona los botones de transporte. reproducir: empieza o reanuda. pausar: pausa al instante. alternar: lo mismo que el boton Reproducir/Pausa. detener: detiene con un fundido de los segundos del fader (una segunda vez, o con fader 0, detiene al instante); en modo AL AIRE sigue al aire salvo que se pase salir_del_aire: true, que pasa a modo EDITOR. siguiente: pasa a la pista marcada con el selector (o a la que sigue en la lista) mezclando los segundos del fader. anterior: vuelve a la pista anterior sin mezcla.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "accion": {"type": "string", "enum": ["reproducir", "pausar", "alternar", "detener", "siguiente", "anterior"]},
                    "salir_del_aire": {"type": "boolean", "description": "Solo con detener y en modo AL AIRE: true pasa a modo EDITOR; false (por defecto) sigue AL AIRE para cuando se vuelva a reproducir."}
                },
                "required": ["accion"]
            }
        },
        {
            "name": SELECCIONAR,
            "description": "Pone el selector de la lista en una pista. Esa pista queda marcada como la proxima: el boton Siguiente (o controlar_reproduccion con siguiente) pasara a ella mezclando con el fader.",
            "inputSchema": {
                "type": "object",
                "properties": {"posicion": {"type": "integer", "minimum": 1, "description": "Posicion en la lista, empezando en 1"}},
                "required": ["posicion"]
            }
        },
        {
            "name": AJUSTAR,
            "description": "Mueve deslizantes y opciones de la ventana. Solo cambia lo que se indique.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "volumen": {"type": "number", "minimum": 0, "maximum": 100, "description": "Volumen en porcentaje"},
                    "fader": {"type": "integer", "minimum": 0, "maximum": FADER_MAXIMO,
                              "description": "Segundos de mezcla al pasar con Siguiente y de fundido al Detener; 0 los desactiva"},
                    "posicion_segundos": {"type": "number", "minimum": 0, "description": "Salta a ese segundo de la pista actual"},
                    "repetir_lista": {"type": "boolean"},
                    "botonera_visible": {"type": "boolean"},
                    "modo_libre": {"type": "boolean", "description": "true: los efectos suenan al instante; false (trabado): esperan el punto de overlay"},
                    "bajar_musica": {"type": "boolean", "description": "Bajar la musica mientras suena un efecto"},
                    "nivel_musica": {"type": "number", "minimum": 0, "maximum": 100, "description": "Porcentaje al que baja la musica durante un efecto, sin ajuste inteligente"},
                    "atenuacion_inteligente": {"type": "boolean", "description": "Ajuste inteligente: lleva cada efecto a la sonoridad de la musica (mas el volumen del efecto) y baja la musica lo justo para que quede 10 dB por debajo, segun lo fuerte que suena ese tramo"},
                    "volumen_efecto": {"type": "number", "minimum": -20, "maximum": 6, "description": "Volumen de los efectos en dB (con ajuste inteligente, relativo a la sonoridad de la musica)"},
                    "atenua_en": {"type": "number", "minimum": 0.01, "maximum": 2, "description": "Segundos que tarda la musica en bajar; la bajada termina justo cuando arranca el efecto (0.04 = 1/25 s)"},
                    "separacion_db": {"type": "number", "minimum": 4, "maximum": 24, "description": "Con atenuacion inteligente, cuantos dB por debajo del efecto queda la musica (mas, la baja mas; por defecto 12). Vale tambien para la voz del anuncio HTH sobre su base"},
                    "recuperacion": {"type": "number", "minimum": 0.2, "maximum": 4, "description": "Segundos en que la musica vuelve, con una rampa suave, cuando el efecto deja de sonar"},
                    "crossfade_inteligente": {"type": "boolean", "description": "Mezclar segun el analisis de cada tema: saltea silencios, adapta la duracion (2 a 12 s), usa curvas de potencia constante y alinea los golpes si el tempo coincide"},
                    "nivelar_volumen": {"type": "boolean", "description": "Llevar cada tema a una misma sonoridad (la de objetivo_lufs)"},
                    "objetivo_lufs": {"type": "number", "enum": [-14, -11, -8], "description": "Sonoridad de la nivelacion: -14 (streaming, baja hasta 10 dB y conserva la dinamica), -11 (punto medio, 6 dB arriba o abajo) u -8 (radio o baile, sube hasta 8 dB y deja que el recorte suave redondee los picos)"}
                }
            }
        },
        {
            "name": ASIGNAR_EFECTO,
            "description": format!("Asigna un archivo de audio a un boton de una hoja de la botonera. Formatos: {formatos}."),
            "inputSchema": {
                "type": "object",
                "properties": {"boton": boton, "hoja": hoja, "ruta": {"type": "string", "description": "Ruta absoluta del archivo de sonido"}},
                "required": ["boton", "ruta"]
            }
        },
        {
            "name": QUITAR_EFECTO,
            "description": "Deja vacio un boton de la botonera. No borra el archivo del disco.",
            "inputSchema": {"type": "object", "properties": {"boton": boton, "hoja": hoja}, "required": ["boton"]},
            "annotations": {"destructiveHint": false}
        },
        {
            "name": DISPARAR_EFECTO,
            "description": "Pulsa un boton de la botonera: en modo libre el efecto suena ya; en modo trabado espera el punto de overlay de la pista. Pulsar el que esta en espera lo cancela.",
            "inputSchema": {"type": "object", "properties": {"boton": boton, "hoja": hoja}, "required": ["boton"]}
        },
        {
            "name": DETENER_EFECTO,
            "description": "Corta el efecto que suena y cancela el que esta en espera.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": ABRIR_EDITOR,
            "description": "Abre en pantalla el editor de una pista (forma de onda, ventana de overlay, inicio, fin, intro, outro y datos) para que el usuario la ajuste a mano.",
            "inputSchema": {
                "type": "object",
                "properties": {"posicion": {"type": "integer", "minimum": 1, "description": "Posicion en la lista, empezando en 1"}},
                "required": ["posicion"]
            }
        },
        {
            "name": SUGERIR,
            "description": "Analiza una pista (indicada por posicion o por ruta) y devuelve los puntos automaticos: inicio y fin (sin el silencio del principio y del final), intro (termina cuando el tema llega a la mitad de su nivel tipico), outro (empieza cuando baja de ese nivel por ultima vez) y la ventana de overlay sugerida, sobre el tramo instrumental mas claro (la intro, un puente o el final) segun un detector aproximado de voz, con su confianza. No cambia nada: los puntos automaticos ya se aplican solos salvo donde hay valores manuales; para volver a ellos use definir_mezcla con 'automatico' u 'overlay_automatico'.",
            "inputSchema": {"type": "object", "properties": pista_por_posicion_o_ruta.clone()},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": MOVER,
            "description": "Cambia el orden de la lista: lleva la pista de 'posicion' a 'nueva_posicion' (ambas empezando en 1). Para subir o bajar un lugar, use la posicion vecina. La pista que suena sigue sonando.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "posicion": {"type": "integer", "minimum": 1},
                    "nueva_posicion": {"type": "integer", "minimum": 1}
                },
                "required": ["posicion", "nueva_posicion"]
            }
        },
        {
            "name": ORDENAR,
            "description": "Cambia el orden de toda la lista. azar: la mezcla al azar (shuffle) y deja primera la pista que suena. bpm: la ordena por tempo, de menor a mayor, para agrupar los temas de BPM parecido; los que no tienen tempo (o no estan analizados) van al final. La pista que suena sigue sonando. Lo que esta en un bloque fijo (entre un marcador de inicio y uno de fin) no se mueve.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "orden": {"type": "string", "enum": ["azar", "bpm"]}
                },
                "required": ["orden"]
            }
        },
        {
            "name": GUARDAR_LISTA,
            "description": "Guarda la lista en un archivo, que queda como la lista abierta (su nombre se ve en el titulo de la ventana). 'nombre' es un nombre de archivo, que se guarda en la carpeta de listas del reproductor, o una ruta absoluta. Sin extension se guarda como lista del reproductor (.json); con .m3u o .m3u8, como M3U (solo las rutas). Sin 'nombre', guarda la lista abierta en su mismo archivo. 'con_fecha' antepone la fecha y hora local al nombre (AAAAMMDDhhmm_). No pisa un archivo existente salvo con 'sobrescribir'. Se guarda el orden de las pistas, la actual y repetir lista; los puntos de mezcla y los datos de cada tema se recuerdan aparte, por archivo.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "nombre": {"type": "string", "description": "Nombre de archivo (por ejemplo 'tarde_viernes') o ruta absoluta; vacio u omitido, la lista abierta"},
                    "con_fecha": {"type": "boolean", "description": "true para anteponer la fecha y hora local al nombre: AAAAMMDDhhmm_nombre"},
                    "sobrescribir": {"type": "boolean", "description": "true para reemplazar un archivo que ya existe (por defecto false)"}
                }
            }
        },
        {
            "name": HOJAS,
            "description": format!("Maneja las hojas de la botonera (10 botones cada una). ir: pone a la vista la hoja indicada. agregar: crea una hoja vacia al final y la muestra (con 'nombre', ya nombrada). quitar: elimina la hoja indicada con sus asignaciones (los archivos no se borran); siempre queda al menos una. renombrar: le pone 'nombre' a la hoja indicada (o a la que esta a la vista), hasta {LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA} caracteres; vacio vuelve al de siempre ('Hoja N'). El nombre de cada hoja se ve en estado_del_reproductor."),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "accion": {"type": "string", "enum": ["ir", "agregar", "quitar", "renombrar"]},
                    "hoja": {"type": "integer", "minimum": 1, "description": "Hoja, empezando en 1 (para ir y quitar; para renombrar, si se omite, la que esta a la vista)"},
                    "nombre": {"type": "string", "description": "Nombre de la hoja (para renombrar y agregar); vacio vuelve a 'Hoja N'"}
                },
                "required": ["accion"]
            }
        },
        {
            "name": INSERTAR_HTH,
            "description": "Inserta en la lista el anuncio de hora, temperatura y humedad (HTH). Se arma con locuciones pregrabadas justo antes de salir al aire, con la hora de ese momento y el ultimo clima consultado; suena sin mezcla. Si la carpeta de locuciones tiene bases musicales, inserta una base al azar (un tema comun, que se mezcla) y la voz se dice en su ventana de overlay. Sin 'posicion', entra despues de la pista seleccionada (o al final).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "posicion": {"type": "integer", "minimum": 1, "description": "Posicion en la lista que ocupara el anuncio, empezando en 1"}
                }
            }
        },
        {
            "name": BASE_HTH,
            "description": "Cambia la base musical de un anuncio HTH de la lista, o se la quita (la voz sola, sin mezcla). Las bases son los audios de la carpeta bases de las locuciones, numerados por nombre desde 1. El anuncio que suena no se puede cambiar.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "posicion": {"type": "integer", "minimum": 1, "description": "Posicion del anuncio en la lista, empezando en 1"},
                    "base": {"type": "integer", "minimum": 0, "description": "Numero de la base (desde 1); 0 la quita"}
                },
                "required": ["posicion", "base"]
            }
        },
        {
            "name": LOCALIDAD_DEL_CLIMA,
            "description": "Consulta o cambia la localidad de la que el anuncio HTH dice la temperatura y la humedad. Sin 'nombre', devuelve la localidad en uso con sus coordenadas. Con 'nombre', la busca en el geocodificador de Open-Meteo (necesita internet): si queda una sola coincidencia (despues de filtrar por 'region'), o se elige una con 'numero', pasa a ser la localidad del clima, se guarda y se consulta en el momento; si quedan varias, las lista con su region y coordenadas sin cambiar nada. Cambiarla solo se puede en modo EDITOR: la localidad fija la hora del reloj y de la columna Sono (al aire se rechaza).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "nombre": {"type": "string", "description": "Nombre de la localidad a buscar (por ejemplo 'Parana')"},
                    "region": {"type": "string", "description": "Palabras que deben aparecer en la region (provincia, departamento o pais), sin importar mayusculas ni acentos; por ejemplo 'Entre Rios'"},
                    "numero": {"type": "integer", "minimum": 1, "description": "Cual de las coincidencias usar (desde 1, en el orden en que se listan tras filtrar por region)"}
                }
            }
        },
        {
            "name": BLOQUE_FIJO,
            "description": "Encierra un tramo de la lista entre un marcador de inicio y uno de fin de bloque fijo: Mezclar y el orden por BPM no cambian lo que queda dentro. Los marcadores son filas de la lista que no suenan; se quitan con quitar_pistas.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "desde": {"type": "integer", "minimum": 1, "description": "Primera pista del bloque, empezando en 1"},
                    "hasta": {"type": "integer", "minimum": 1, "description": "Ultima pista del bloque, empezando en 1"}
                },
                "required": ["desde", "hasta"]
            }
        },
        {
            "name": BUSCAR_EN_CATALOGO,
            "description": "Busca en el catalogo (los temas que alguna vez entraron a la lista, con sus datos) por titulo, artista, tipo, genero, album o nombre de archivo: deben aparecer todas las palabras, sin importar mayusculas ni acentos. Admite prefijos para un dato en particular: titulo:, artista:, genero:, album:, tipo:, nota:, archivo: (texto; frases entre comillas), anio:, ranking:, bpm:, duracion: (en minutos o m:ss) con =, >, >=, <, <= o tramos a..b, y sonado: para la ultima reproduccion (hoy, ayer, nunca, 7d, >30d, 12h, 2s semanas, 3m meses, 1a, 2026-11-03, 03/11/2026, 2026-11, tramos a..b). Un - adelante niega la condicion (-genero:cumbia). Devuelve la ruta de cada tema, para insertarlo con agregar_pistas, y si ya esta en la lista. Los de tipo Efecto no aparecen (van en la botonera).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "texto": {"type": "string", "description": "Palabras o condiciones con prefijo (por ejemplo: artista:sanz ranking:>=4 -sonado:<30d); vacio lista todo el catalogo"},
                    "limite": {"type": "integer", "minimum": 1, "maximum": 500, "description": "Cantidad maxima de resultados (por defecto 50)"}
                },
                "required": ["texto"]
            }
        },
        {
            "name": ENCOLAR,
            "description": "Encola una pista como la siguiente: la lleva justo despues de la que suena (o al principio si no suena ninguna), asi el final de la actual se mezcla con ella. El resto de la lista sigue en su orden.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "posicion": {"type": "integer", "minimum": 1, "description": "Posicion de la pista en la lista, empezando en 1"},
                    "sacar_del_bloque": {"type": "boolean", "description": "Confirma que se encole aunque eso saque la pista de un bloque fijo (sin esto, se informa y no se mueve)"}
                },
                "required": ["posicion"]
            }
        },
        {
            "name": PROGRAMAR_TANDA,
            "description": "Programa una tanda (o reemplaza la del mismo nombre): sus avisos se encolan solos como bloque fijo a la hora pautada, en el corte entre dos temas mas cercano, dentro de la tolerancia (no se corta ningun tema). Los archivos deben tener tipo Comercial, Publicidad, Anuncio, Podcast o Seco (se fija con definir_datos). Cada aviso emitido se anota en el registro de comerciales con el nombre de la tanda.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "nombre": {"type": "string", "description": "Nombre de la tanda; identifica la tanda al reprogramarla o quitarla"},
                    "archivos": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Rutas absolutas de los avisos, en el orden en que salen"},
                    "inicio": {"type": "string", "description": "Primer horario del dia, HH:MM (por ejemplo 10:30)"},
                    "cada_minutos": {"type": "integer", "minimum": 1, "maximum": 1440, "description": "Repetir cada tantos minutos desde el inicio (sin esto, una vez por dia)"},
                    "hasta": {"type": "string", "description": "Ultimo horario del dia en que puede repetirse, HH:MM (solo con cada_minutos)"},
                    "dias": {"type": "array", "items": {"type": "string"}, "description": "Dias: lun, mar, mie, jue, vie, sab, dom, 'todos' o 'habiles' (por defecto todos)"},
                    "antes_minutos": {"type": "number", "minimum": 0, "maximum": 60, "description": "Cuanto antes de la hora puede entrar (por defecto 3)"},
                    "despues_minutos": {"type": "number", "minimum": 0, "maximum": 60, "description": "Cuanto despues de la hora puede entrar (por defecto 2)"},
                    "modo": {"type": "string", "enum": ["tolerancia", "exacta"], "description": "Sin un corte entre temas dentro de la tolerancia: 'tolerancia' espera el final del tema (sale tarde, por defecto); 'exacta' baja lo que suena con el fader a la hora pautada"},
                    "prioridad": {"type": "integer", "minimum": PRIORIDAD_MINIMA, "maximum": PRIORIDAD_MAXIMA, "description": "Con la misma hora, sale primero la de mayor prioridad (por defecto 5)"},
                    "duracion_maxima_segundos": {"type": "integer", "minimum": 0, "maximum": 1800, "description": "Lo que puede durar la tanda con los avisos de las ordenes de publicidad (0 o sin esto: sin limite)"},
                    "activa": {"type": "boolean", "description": "false la deja guardada sin emitirse (por defecto true)"}
                },
                "required": ["nombre", "archivos", "inicio"]
            }
        },
        {
            "name": LISTAR_TANDAS,
            "description": "Lista las tandas programadas: regla, tolerancia, avisos (y si falta alguno), proxima salida y ultima atendida.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": ENCOLAR_TANDA,
            "description": "Encola ya una tanda programada, fuera de su horario, en el proximo corte entre temas (no cambia su programacion).",
            "inputSchema": {
                "type": "object",
                "properties": {"nombre": {"type": "string", "description": "Nombre de la tanda"}},
                "required": ["nombre"]
            }
        },
        {
            "name": QUITAR_TANDA,
            "description": "Quita una tanda de la programacion. Lo que ya se encolo en la lista queda.",
            "inputSchema": {
                "type": "object",
                "properties": {"nombre": {"type": "string", "description": "Nombre de la tanda"}},
                "required": ["nombre"]
            }
        },
        {
            "name": REGISTRO_DE_COMERCIALES,
            "description": "Consulta las salidas al aire de la propaganda facturable (Comercial y Publicidad) entre dos fechas: cantidad, completas y cortadas, tiempo al aire y el detalle de cada una con su tanda. Se conservan el mes actual y los dos anteriores.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "desde": {"type": "string", "description": "Fecha inicial AAAA-MM-DD (por defecto, hoy)"},
                    "hasta": {"type": "string", "description": "Fecha final AAAA-MM-DD, inclusive (por defecto, igual a desde)"},
                    "texto": {"type": "string", "description": "Filtra por titulo, artista (anunciante), tanda o nombre de archivo"}
                }
            }
        },
        {
            "name": PROGRAMAR_ORDEN,
            "description": "Carga una orden de publicidad (o reemplaza la del mismo nombre): lo que contrato un anunciante. Sus avisos rotan y se reparten solos en las tandas programadas de cada dia, dentro de su franja, sin pasar la duracion maxima de la tanda ni juntar dos avisos del mismo rubro. Indique una sola frecuencia: cada_minutos, veces_por_dia u horarios.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "nombre": {"type": "string", "description": "Nombre de la orden (por ejemplo 'Sol - Noviembre')"},
                    "anunciante": {"type": "string"},
                    "rubro": {"type": "string", "description": "Rubro del anunciante: dos avisos del mismo rubro no van en la misma tanda"},
                    "desde": {"type": "string", "description": "Inicio de la vigencia, AAAA-MM-DD"},
                    "hasta": {"type": "string", "description": "Fin de la vigencia, AAAA-MM-DD (inclusive)"},
                    "dias": {"type": "array", "items": {"type": "string"}, "description": "lun, mar, mie, jue, vie, sab, dom, 'todos' o 'habiles' (por defecto todos)"},
                    "franja_desde": {"type": "string", "description": "Desde que hora puede salir, HH:MM (por defecto 00:00)"},
                    "franja_hasta": {"type": "string", "description": "Hasta que hora puede salir, HH:MM (por defecto 23:59)"},
                    "cada_minutos": {"type": "integer", "minimum": 5, "maximum": 1440, "description": "Una salida cada tantos minutos desde el inicio de la franja (120 = cada 2 horas)"},
                    "veces_por_dia": {"type": "integer", "minimum": 1, "maximum": 96, "description": "Tantas salidas por dia, una en cada parte igual de la franja"},
                    "horarios": {"type": "array", "items": {"type": "string"}, "description": "Menciones a hora fija, HH:MM (salen en la tanda mas cercana, hasta 15 minutos antes o despues)"},
                    "avisos": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Rutas absolutas de los avisos; rotan, uno por salida. Deben tener tipo Comercial, Publicidad, Anuncio, Podcast o Seco"},
                    "tandas": {"type": "array", "items": {"type": "string"}, "description": "Solo en estas tandas (por defecto, en cualquiera)"},
                    "prioridad": {"type": "integer", "minimum": PRIORIDAD_MINIMA, "maximum": PRIORIDAD_MAXIMA, "description": "Con la misma dificultad, se ubica antes la de mayor prioridad (por defecto 5)"},
                    "activa": {"type": "boolean", "description": "false la deja cargada sin repartirse (por defecto true)"}
                },
                "required": ["nombre", "anunciante", "desde", "hasta", "avisos"]
            }
        },
        {
            "name": LISTAR_ORDENES,
            "description": "Lista las ordenes de publicidad con su regla, sus avisos y cuantas salidas se ubicaron hoy de las que pide.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": QUITAR_ORDEN,
            "description": "Quita una orden de publicidad: sus avisos dejan de repartirse (lo emitido queda en el registro).",
            "inputSchema": {
                "type": "object",
                "properties": {"nombre": {"type": "string"}},
                "required": ["nombre"]
            }
        },
        {
            "name": PAUTA_DEL_DIA,
            "description": "La pauta de un dia: en que salida de que tanda va cada aviso de las ordenes (con la duracion de la tanda) y las salidas que no se pudieron ubicar, con su motivo.",
            "inputSchema": {
                "type": "object",
                "properties": {"fecha": {"type": "string", "description": "AAAA-MM-DD (por defecto, hoy)"}}
            }
        },
        {
            "name": CERTIFICADO,
            "description": "Certificado de emision de un anunciante entre dos fechas: por orden, salidas contratadas, emitidas completas, cortadas y el cumplimiento, con el detalle de cada salida. Tambien lo deja en un CSV en la carpeta certificados de los datos.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "anunciante": {"type": "string"},
                    "desde": {"type": "string", "description": "AAAA-MM-DD (por defecto, el primero del mes)"},
                    "hasta": {"type": "string", "description": "AAAA-MM-DD, inclusive (por defecto, hoy)"}
                },
                "required": ["anunciante"]
            }
        },
        {
            "name": REGISTRAR_ANUNCIANTE,
            "description": "Carga o actualiza la ficha de un anunciante (el nombre comercial que usan sus ordenes). Los datos que se omiten conservan su valor; un texto vacio los borra. Valida el CUIT (digito verificador) y el correo. Con 'nombre_anterior' lo renombra y sus ordenes pasan al nombre nuevo.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "nombre": {"type": "string", "description": "Nombre comercial (Supermercado Sol)"},
                    "nombre_anterior": {"type": "string", "description": "Para renombrar: el nombre que tiene hoy"},
                    "razon_social": {"type": "string"},
                    "cuit": {"type": "string", "description": "11 digitos, con o sin guiones (30-71234567-1)"},
                    "rubro": {"type": "string", "description": "Rubro que toman sus ordenes si no indican otro"},
                    "contacto": {"type": "string", "description": "Persona de contacto"},
                    "telefono": {"type": "string"},
                    "correo": {"type": "string"},
                    "direccion": {"type": "string"},
                    "agencia": {"type": "string"},
                    "notas": {"type": "string"},
                    "activo": {"type": "boolean", "description": "Inactivo: conserva la ficha pero no admite ordenes nuevas"}
                },
                "required": ["nombre"]
            }
        },
        {
            "name": LISTAR_ANUNCIANTES,
            "description": "Lista los anunciantes con su ficha (razon social, CUIT, rubro, contacto, agencia), cuantas ordenes tienen y lo que falta para facturarles; tambien los nombrados en ordenes que no tienen ficha.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": QUITAR_ANUNCIANTE,
            "description": "Quita la ficha de un anunciante que no tiene ordenes (si tiene, hay que quitarlas o desactivarlo).",
            "inputSchema": {
                "type": "object",
                "properties": {"nombre": {"type": "string"}},
                "required": ["nombre"]
            }
        }
    ]})
});

/// Resultado de tools/list.
pub fn definiciones() -> Value {
    DEFINICIONES.clone()
}

pub fn existe(nombre: &str) -> bool {
    HERRAMIENTAS.contains(&nombre)
}

/// Ejecuta una herramienta y devuelve el resultado de tools/call. Los errores
/// de uso se informan con isError para que el agente pueda corregir la llamada.
pub fn ejecutar(nombre: &str, argumentos: &Map<String, Value>) -> Value {
    let resultado = match nombre {
        LISTAR => listar_pistas().map(|salida| texto(&salida, false)),
        AGREGAR => agregar_pistas(argumentos),
        QUITAR => quitar_pistas(argumentos),
        DEFINIR_MEZCLA => definir_mezcla(argumentos),
        DEFINIR_DATOS => definir_datos(argumentos),
        ESTADO => ejecutar_en_interfaz(|controlador| controlador.resumen()).map(|salida| texto(&salida, false)),
        CONTROLAR => controlar(argumentos),
        SELECCIONAR => seleccionar(argumentos),
        AJUSTAR => ajustar(argumentos),
        ASIGNAR_EFECTO => asignar_efecto(argumentos),
        QUITAR_EFECTO => quitar_efecto(argumentos),
        DISPARAR_EFECTO => disparar_efecto(argumentos),
        DETENER_EFECTO => ejecutar_en_interfaz(|controlador| {
            controlador.detener_efecto();
            texto("Efecto detenido.", false)
        }),
        ABRIR_EDITOR => abrir_editor(argumentos),
        SUGERIR => sugerir_puntos(argumentos),
        MOVER => mover_pista(argumentos),
        ORDENAR => ordenar_lista(argumentos),
        GUARDAR_LISTA => guardar_lista(argumentos),
        HOJAS => hojas_de_efectos(argumentos),
        INSERTAR_HTH => insertar_hth(argumentos),
        BASE_HTH => base_hth(argumentos),
        LOCALIDAD_DEL_CLIMA => localidad_del_clima(argumentos),
        BLOQUE_FIJO => bloque_fijo(argumentos),
        BUSCAR_EN_CATALOGO => buscar_en_catalogo(argumentos),
        ENCOLAR => encolar_pista(argumentos),
        PROGRAMAR_TANDA => programar_tanda(argumentos),
        LISTAR_TANDAS => ejecutar_en_interfaz(|controlador| texto(&controlador.describir_tandas(), false)),
        QUITAR_TANDA => quitar_tanda(argumentos),
        ENCOLAR_TANDA => encolar_tanda(argumentos),
        REGISTRO_DE_COMERCIALES => registro_de_comerciales(argumentos),
        PROGRAMAR_ORDEN => programar_orden(argumentos),
        LISTAR_ORDENES => ejecutar_en_interfaz(|controlador| texto(&controlador.describir_ordenes(), false)),
        QUITAR_ORDEN => quitar_orden(argumentos),
        PAUTA_DEL_DIA => pauta_del_dia(argumentos),
        CERTIFICADO => certificado(argumentos),
        REGISTRAR_ANUNCIANTE => registrar_anunciante(argumentos),
        LISTAR_ANUNCIANTES => ejecutar_en_interfaz(|controlador| texto(&controlador.describir_anunciantes(), false)),
        QUITAR_ANUNCIANTE => quitar_anunciante(argumentos),
        otro => Err(format!("Herramienta desconocida: {otro}")),
    };
    resultado.unwrap_or_else(|error| texto(&error, true))
}

fn listar_pistas() -> Result<String, String> {
    let (entradas, sonando, objetivo) = ejecutar_en_interfaz(|controlador| {
        let objetivo = controlador.objetivo_de_sonoridad();
        let entradas: Vec<(Pista, PuntosDeMezcla, Option<f64>, FichaDeCancion, Option<DatosDeMezcla>)> = controlador
            .pistas_en_lista()
            .into_iter()
            .map(|pista| {
                let puntos = controlador.puntos_de(&pista);
                let mezcla = controlador.segundos_de_mezcla_tras(&pista);
                let ficha = controlador.ficha_de(&pista);
                let datos = controlador.datos_de_mezcla(&pista);
                (pista, puntos, mezcla, ficha, datos)
            })
            .collect();
        (entradas, controlador.pista_en_reproduccion(), objetivo)
    })?;
    if entradas.is_empty() {
        return Ok("La lista esta vacia.".into());
    }
    let mut salida = format!("{} pista(s):\n", entradas.len());
    for (posicion, (pista, puntos, mezcla, ficha, datos)) in entradas.iter().enumerate() {
        salida.push_str(&format!("{}. {} | {}", posicion + 1, pista.nombre(), pista.archivo().display()));
        if let Some(tipo) = ficha.tipo {
            salida.push_str(&format!(" | tipo {}", tipo.nombre()));
        }
        for (campo, valor) in
            [("titulo", &ficha.titulo), ("artista", &ficha.artista), ("genero", &ficha.genero), ("album", &ficha.album), ("nota", &ficha.nota)]
        {
            if let Some(valor) = valor {
                salida.push_str(&format!(" | {campo} {valor}"));
            }
        }
        if let Some(anio) = ficha.anio {
            salida.push_str(&format!(" | anio {anio}"));
        }
        if let Some(efecto) = &ficha.efecto_fijado {
            salida.push_str(&format!(" | efecto fijado {}", efecto.display()));
        }
        if let Some(duracion) = ficha.duracion {
            salida.push_str(&format!(" | duracion {}", formatear_minutos(duracion)));
        }
        salida.push_str(&format!(" | ranking {}/{RANKING_MAXIMO}", ficha.ranking));
        if let Some(bpm) = ficha.bpm {
            salida.push_str(&format!(" | ritmo {bpm} BPM (manual)"));
        }
        if let Some(momento) = &ficha.ultima_reproduccion {
            salida.push_str(&format!(" | ultima reproduccion {momento}"));
        }
        let overlay = match puntos.overlay() {
            Some(_) => format!("{} ({})", formatear_ventana(puntos.overlay()), if puntos.overlay_manual() { "manual" } else { "sugerida" }),
            None if puntos.overlay_manual() => "sin ventana (manual)".into(),
            None => "sin ventana".into(),
        };
        let fin = ficha.duracion.map_or_else(|| "—".into(), |duracion| segundos(duracion - puntos.recorte_final()));
        salida.push_str(&format!(
            " | inicio {} | fin {fin} | intro {} | outro {} ({}) | overlay {overlay}",
            segundos(puntos.recorte_inicial()),
            segundos(puntos.intro()),
            segundos(puntos.outro()),
            if puntos.manuales() { "manuales" } else { "automaticos" }
        ));
        if let Some(mezcla) = mezcla {
            salida.push_str(&format!(" | mezcla con la siguiente {}", segundos(*mezcla)));
        }
        if let Some(datos) = datos {
            if let Some(sonoridad) = datos.sonoridad {
                if ficha.tipo.is_some_and(TipoDeElemento::a_volumen_maximo) {
                    // Un aviso suena al maximo volumen, este o no la nivelacion.
                    let ajuste = 20.0 * datos.ganancia_maxima().log10();
                    salida.push_str(&format!(" | sonoridad {sonoridad:.1} LUFS (aviso al maximo volumen {ajuste:+.1} dB)"));
                } else {
                    let ajuste = 20.0 * datos.ganancia_de_nivelacion(objetivo).log10();
                    salida.push_str(&format!(" | sonoridad {sonoridad:.1} LUFS (nivelacion {ajuste:+.1} dB)"));
                }
            }
            if let Some(pico) = datos.pico_real_db() {
                salida.push_str(&format!(" | pico real {pico:.1} dBTP"));
            }
            if let Some(rango) = datos.rango_dinamico {
                let amplio = if rango >= RANGO_DINAMICO_AMPLIO { ", muy dinamico" } else { "" };
                salida.push_str(&format!(" | rango dinamico {rango:.1} LU{amplio}"));
            }
            if let (None, Some(bpm)) = (ficha.bpm, datos.bpm()) {
                salida.push_str(&format!(" | ritmo {bpm:.0} BPM"));
            }
            if datos.silencio_inicial > 0.0 || datos.silencio_final > 0.0 {
                salida.push_str(&format!(
                    " | silencio al principio {} y al final {}",
                    segundos(datos.silencio_inicial),
                    segundos(datos.silencio_final)
                ));
            }
            if let Some(sugerencia) = datos.overlay_sugerido.filter(|sugerencia| Some(sugerencia.ventana) != puntos.overlay()) {
                salida.push_str(&format!(" | overlay sugerido {}", formatear_ventana(Some(sugerencia.ventana))));
            }
        }
        if sonando.as_ref() == Some(pista) {
            salida.push_str("  [sonando]");
        }
        salida.push('\n');
    }
    Ok(salida)
}

/// Pista indicada por "posicion" o por "ruta" (solo uno de los dos).
enum PistaPedida {
    Posicion(i64),
    Ruta(PathBuf),
}

fn pista_pedida(argumentos: &Map<String, Value>) -> Result<PistaPedida, String> {
    let posicion = entero_opcional(argumentos, "posicion")?;
    let ruta = texto_opcional(argumentos, "ruta")?;
    match (posicion, ruta) {
        (Some(posicion), None) => Ok(PistaPedida::Posicion(posicion)),
        (None, Some(ruta)) => Ok(PistaPedida::Ruta(ruta_absoluta(&ruta)?)),
        _ => Err("Indique 'posicion' o 'ruta' (solo uno de los dos).".into()),
    }
}

impl PistaPedida {
    /// Busca la pista en la lista actual; el error ya viene redactado.
    fn resolver(&self, controlador: &Controlador) -> Result<Pista, String> {
        let pistas = controlador.pistas_en_lista();
        match self {
            PistaPedida::Posicion(posicion) => usize::try_from(posicion - 1)
                .ok()
                .and_then(|indice| pistas.get(indice))
                .cloned()
                .ok_or_else(|| format!("Posicion {posicion} fuera de la lista (1 a {}).", pistas.len())),
            PistaPedida::Ruta(ruta) => {
                let pista = Pista::new(ruta);
                if pistas.contains(&pista) { Ok(pista) } else { Err(format!("{} no esta en la lista.", ruta.display())) }
            }
        }
    }

    /// Como `resolver`, pero por ruta acepta tambien un archivo de audio que
    /// no esta en la lista (un efecto de la botonera o un tema del catalogo),
    /// igual que el editor: sus datos se guardan en su ficha.
    fn resolver_para_datos(&self, controlador: &Controlador) -> Result<Pista, String> {
        match self {
            PistaPedida::Ruta(ruta) if !controlador.pistas_en_lista().contains(&Pista::new(ruta)) => {
                if ruta.is_file() && Pista::es_formato_soportado(ruta) {
                    Ok(Pista::new(ruta))
                } else {
                    Err(format!("{}: no esta en la lista y no es un archivo de audio accesible.", ruta.display()))
                }
            }
            _ => self.resolver(controlador),
        }
    }
}

fn definir_mezcla(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let pedida = pista_pedida(argumentos)?;
    let inicio = tiempo_opcional(argumentos, "inicio")?;
    let fin = tiempo_opcional(argumentos, "fin")?;
    let intro = tiempo_opcional(argumentos, "intro")?;
    let outro = tiempo_opcional(argumentos, "outro")?;
    let automatico = booleano_opcional(argumentos, "automatico")?.unwrap_or(false);
    let overlay_automatico = booleano_opcional(argumentos, "overlay_automatico")?.unwrap_or(false);
    let overlay_inicio = tiempo_opcional(argumentos, "overlay_inicio")?;
    let overlay_fin = tiempo_opcional(argumentos, "overlay_fin")?;
    let quitar_overlay = booleano_opcional(argumentos, "quitar_overlay")?.unwrap_or(false);
    let ventana_nueva = match (overlay_inicio, overlay_fin, quitar_overlay) {
        (None, None, false) => None,
        (None, None, true) => Some(None),
        (Some(inicio), Some(fin), false) => Some(Some(VentanaDeOverlay::nueva(inicio, fin)?)),
        (_, _, true) => return Err("Use 'quitar_overlay' o 'overlay_inicio'/'overlay_fin', no ambos.".into()),
        _ => return Err("Indique 'overlay_inicio' y 'overlay_fin' juntos.".into()),
    };
    if overlay_automatico && ventana_nueva.is_some() {
        return Err("Use 'overlay_automatico' o una ventana de overlay, no ambos.".into());
    }
    let hay_puntos = [inicio, fin, intro, outro].iter().any(Option::is_some);
    if automatico && hay_puntos {
        return Err("Use 'automatico' o 'inicio'/'fin'/'intro'/'outro', no ambos.".into());
    }
    if !hay_puntos && !automatico && !overlay_automatico && ventana_nueva.is_none() {
        return Err("Indique 'inicio', 'fin', 'intro', 'outro', la ventana de overlay, 'quitar_overlay', 'automatico' u 'overlay_automatico'.".into());
    }

    ejecutar_en_interfaz(move |controlador| {
        let pista = match pedida.resolver(controlador) {
            Ok(pista) => pista,
            Err(error) => return texto(&error, true),
        };
        let automaticos = controlador.datos_de_mezcla(&pista).map(|datos| datos.puntos_automaticos());
        if (automatico || overlay_automatico) && automaticos.is_none() {
            return texto("El tema todavia no esta analizado; pruebe en unos segundos.", true);
        }
        let duracion = controlador.duracion_de(&pista);
        if fin.is_some() && duracion.is_none() {
            return texto("Todavia no se conoce la duracion del tema para fijar su fin.", true);
        }
        let mut actuales = controlador.puntos_de(&pista);
        if let Some(automaticos) = automaticos.filter(|_| automatico) {
            actuales = actuales.como_manuales(false).completar_con(&automaticos).con_overlay(actuales.overlay());
        }
        if let Some(automaticos) = automaticos.filter(|_| overlay_automatico) {
            actuales = actuales.con_overlay(automaticos.overlay()).con_overlay_manual(false);
        }
        let recorte_final = fin.zip(duracion).map_or(actuales.recorte_final(), |(fin, duracion)| (duracion - fin).max(0.0));
        let puntos = PuntosDeMezcla::nuevos(
            intro.unwrap_or(actuales.intro()),
            outro.unwrap_or(actuales.outro()),
            ventana_nueva.unwrap_or(actuales.overlay()),
        )
        .and_then(|puntos| puntos.con_recortes(inicio.unwrap_or(actuales.recorte_inicial()), recorte_final))
        .map(|puntos| {
            puntos
                .como_manuales(actuales.manuales() || hay_puntos)
                .con_overlay_manual(actuales.overlay_manual() || ventana_nueva.is_some())
        })
        .and_then(|puntos| controlador.definir_puntos_de_mezcla(&pista, puntos).map(|_| puntos));
        match puntos {
            Ok(puntos) => texto(
                &format!(
                    "{}: inicio {}, fin {}, intro {}, outro {} ({}), overlay {} ({}).",
                    pista.nombre(),
                    segundos(puntos.recorte_inicial()),
                    duracion.map_or_else(|| "—".into(), |duracion| segundos(duracion - puntos.recorte_final())),
                    segundos(puntos.intro()),
                    segundos(puntos.outro()),
                    if puntos.manuales() { "manuales" } else { "automaticos" },
                    formatear_ventana(puntos.overlay()),
                    if puntos.overlay_manual() { "manual" } else { "sugerida" }
                ),
                false,
            ),
            Err(error) => texto(&error, true),
        }
    })
}

fn definir_datos(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let pedida = pista_pedida(argumentos)?;
    let ranking = match entero_opcional(argumentos, "ranking")? {
        Some(ranking) if !(0..=i64::from(RANKING_MAXIMO)).contains(&ranking) => {
            return Err(format!("'ranking' debe estar entre 0 y {RANKING_MAXIMO}."));
        }
        otro => otro.map(|ranking| ranking as u8),
    };
    let bpm = match numero_opcional(argumentos, "bpm")? {
        Some(bpm) if bpm == 0.0 => Some(None),
        Some(bpm) if (BPM_MINIMO..=BPM_MAXIMO).contains(&bpm) => Some(Some(bpm)),
        Some(_) => return Err(format!("'bpm' debe estar entre {BPM_MINIMO} y {BPM_MAXIMO}, o ser 0.")),
        None => None,
    };
    let anio = match entero_opcional(argumentos, "anio")? {
        Some(0) => Some(None),
        Some(anio) if u16::try_from(anio).is_ok_and(anio_valido) => Some(Some(anio as u16)),
        Some(_) => return Err("'anio' debe estar entre 1000 y 2999, o ser 0.".into()),
        None => None,
    };
    let efecto_fijado = match texto_opcional(argumentos, "efecto_fijado")? {
        Some(ruta) if ruta.trim().is_empty() => Some(None),
        Some(ruta) => {
            let ruta = ruta_absoluta(&ruta)?;
            if !ruta.is_file() {
                return Err(format!("No existe el efecto {}.", ruta.display()));
            }
            Some(Some(ruta))
        }
        None => None,
    };
    let tipo = match texto_opcional(argumentos, "tipo")? {
        Some(nombre) if nombre.trim().is_empty() => Some(None),
        Some(nombre) => Some(Some(TipoDeElemento::desde_nombre(&nombre).ok_or_else(|| {
            format!("Tipo desconocido: {nombre}. Use Tema, Tema editado, Comercial, Anuncio, Publicidad, Podcast, Efecto o Seco (vacio: sin clasificar).")
        })?)),
        None => None,
    };
    let mut cambios = CambiosDeDatos {
        tipo,
        titulo: texto_opcional(argumentos, "titulo")?,
        artista: texto_opcional(argumentos, "artista")?,
        genero: texto_opcional(argumentos, "genero")?,
        album: texto_opcional(argumentos, "album")?,
        nota: texto_opcional(argumentos, "nota")?,
        anio,
        efecto_fijado,
        ranking,
        bpm,
        ultima_reproduccion: None,
    };
    if cambios == CambiosDeDatos::default() {
        return Err("Indique 'tipo', 'titulo', 'artista', 'genero', 'album', 'nota', 'anio', 'efecto_fijado', 'ranking' y/o 'bpm'.".into());
    }
    ejecutar_en_interfaz(move |controlador| match pedida.resolver_para_datos(controlador) {
        Ok(pista) => {
            if let Err(error) = controlador.validar_que_no_sea_el_anuncio_hth(&pista) {
                return texto(&error, true);
            }
            // Sin clasificar, un genero que nombra un tipo lo clasifica.
            if cambios.tipo.is_none() && controlador.ficha_de(&pista).tipo.is_none() {
                cambios.tipo = cambios
                    .genero
                    .as_deref()
                    .and_then(TipoDeElemento::segun_genero)
                    .map(Some);
            }
            if let Some(Err(error)) = cambios.tipo.map(|tipo| controlador.validar_tipo(&pista, tipo)) {
                return texto(&error, true);
            }
            let ficha = controlador.cambiar_datos(&pista, cambios);
            let dato = |valor: &Option<String>| valor.clone().unwrap_or_else(|| "-".into());
            texto(
                &format!(
                    "{}: tipo {}, titulo {}, artista {}, genero {}, album {}, anio {}, efecto fijado {}, ranking {}/{RANKING_MAXIMO}, BPM {}, nota {}.",
                    pista.nombre(),
                    ficha.tipo.map_or("sin clasificar", TipoDeElemento::nombre),
                    dato(&ficha.titulo),
                    dato(&ficha.artista),
                    dato(&ficha.genero),
                    dato(&ficha.album),
                    ficha.anio.map_or_else(|| "-".into(), |anio| anio.to_string()),
                    ficha.efecto_fijado.as_ref().map_or_else(|| "-".into(), |efecto| efecto.display().to_string()),
                    ficha.ranking,
                    ficha.bpm.map_or_else(|| "del analisis".into(), |bpm| bpm.to_string()),
                    dato(&ficha.nota)
                ),
                false,
            )
        }
        Err(error) => texto(&error, true),
    })
}

fn controlar(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let accion = match texto_opcional(argumentos, "accion")?.as_deref() {
        Some("reproducir") => AccionDeTransporte::Reproducir,
        Some("pausar") => AccionDeTransporte::Pausar,
        Some("alternar") => AccionDeTransporte::Alternar,
        Some("detener") => AccionDeTransporte::Detener { salir_del_aire: booleano_opcional(argumentos, "salir_del_aire")?.unwrap_or(false) },
        Some("siguiente") => AccionDeTransporte::Siguiente,
        Some("anterior") => AccionDeTransporte::Anterior,
        Some(otra) => return Err(format!("Accion desconocida: {otra}.")),
        None => return Err("Indique 'accion'.".into()),
    };
    ejecutar_en_interfaz(move |controlador| texto(&controlador.transporte(accion), false))
}

fn seleccionar(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let posicion = entero_opcional(argumentos, "posicion")?.ok_or("Indique 'posicion'.")?;
    let indice = usize::try_from(posicion - 1).map_err(|_| format!("Posicion {posicion} fuera de la lista."))?;
    ejecutar_en_interfaz(move |controlador| match controlador.seleccionar_posicion(indice) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn ajustar(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let porcentaje = |clave: &str| -> Result<Option<f64>, String> {
        match numero_opcional(argumentos, clave)? {
            Some(valor) if !(0.0..=100.0).contains(&valor) => Err(format!("'{clave}' debe estar entre 0 y 100.")),
            otro => Ok(otro.map(|valor| valor / 100.0)),
        }
    };
    let fader = match entero_opcional(argumentos, "fader")? {
        Some(fader) if !(0..=i64::from(FADER_MAXIMO)).contains(&fader) => {
            return Err(format!("'fader' debe estar entre 0 y {FADER_MAXIMO}."));
        }
        otro => otro.map(|fader| fader as u8),
    };
    let ajustes = Ajustes {
        volumen: porcentaje("volumen")?,
        fader,
        posicion: segundos_opcionales(argumentos, "posicion_segundos")?,
        repetir: booleano_opcional(argumentos, "repetir_lista")?,
        botonera_visible: booleano_opcional(argumentos, "botonera_visible")?,
        modo_libre: booleano_opcional(argumentos, "modo_libre")?,
        bajar_musica: booleano_opcional(argumentos, "bajar_musica")?,
        nivel_de_musica: porcentaje("nivel_musica")?,
        atenuacion_inteligente: booleano_opcional(argumentos, "atenuacion_inteligente")?,
        volumen_de_efecto: match numero_opcional(argumentos, "volumen_efecto")? {
            Some(decibeles) if !(-20.0..=6.0).contains(&decibeles) => {
                return Err("'volumen_efecto' debe estar entre -20 y 6 dB.".into());
            }
            otro => otro,
        },
        anticipacion: segundos_opcionales(argumentos, "atenua_en")?,
        recuperacion: segundos_opcionales(argumentos, "recuperacion")?,
        separacion: match numero_opcional(argumentos, "separacion_db")? {
            Some(decibeles) if !(4.0..=24.0).contains(&decibeles) => {
                return Err("'separacion_db' debe estar entre 4 y 24 dB.".into());
            }
            otro => otro,
        },
        crossfade_inteligente: booleano_opcional(argumentos, "crossfade_inteligente")?,
        nivelar_volumen: booleano_opcional(argumentos, "nivelar_volumen")?,
        objetivo_de_sonoridad: match numero_opcional(argumentos, "objetivo_lufs")? {
            Some(lufs) => Some(
                ObjetivoDeSonoridad::de_lufs(lufs).ok_or("'objetivo_lufs' debe ser -14, -11 u -8.")?,
            ),
            None => None,
        },
    };
    ejecutar_en_interfaz(move |controlador| match controlador.ajustar(ajustes) {
        Ok(()) => texto(&controlador.resumen(), false),
        Err(error) => texto(&error, true),
    })
}

fn abrir_editor(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let posicion = entero_opcional(argumentos, "posicion")?.ok_or("Indique 'posicion'.")?;
    let indice = usize::try_from(posicion - 1).map_err(|_| format!("Posicion {posicion} fuera de la lista."))?;
    ejecutar_en_interfaz(move |controlador| match controlador.abrir_editor(indice) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

/// La forma de onda se calcula (o se lee de la cache) en el hilo del
/// servidor, sin frenar la interfaz.
fn sugerir_puntos(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let pedida = pista_pedida(argumentos)?;
    let pista = ejecutar_en_interfaz(move |controlador| pedida.resolver(controlador))??;
    let onda = FormaDeOnda::cargar_o_calcular(pista.archivo(), &carpeta_del_usuario().join("ondas"))?;
    let datos = DatosDeMezcla::de_la_onda(&onda);
    let puntos = datos.puntos_automaticos();
    let overlay = match datos.overlay_sugerido {
        Some(sugerencia) => format!(
            "overlay sugerido {} sobre la {} (confianza {:.0} %)",
            formatear_ventana(Some(sugerencia.ventana)),
            sugerencia.zona.nombre(),
            sugerencia.confianza * 100.0
        ),
        None => "sin overlay sugerido (no hay un tramo instrumental claro de al menos 4 s)".into(),
    };
    Ok(texto(
        &format!(
            "{}: inicio {}, fin {}, intro {} (suena {}), outro {} (suena {}), {overlay}; duracion {}.",
            pista.nombre(),
            formatear_tiempo(puntos.recorte_inicial()),
            formatear_tiempo(onda.duracion() - puntos.recorte_final()),
            formatear_tiempo(puntos.intro()),
            formatear_tiempo(puntos.intro_audible()),
            formatear_tiempo(puntos.outro()),
            formatear_tiempo(puntos.outro_audible()),
            formatear_tiempo(onda.duracion())
        ),
        false,
    ))
}

fn mover_pista(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let indice = |clave: &str| -> Result<usize, String> {
        let posicion = entero_opcional(argumentos, clave)?.ok_or_else(|| format!("Indique '{clave}'."))?;
        usize::try_from(posicion - 1).map_err(|_| format!("'{clave}' debe ser 1 o mayor."))
    };
    let (desde, hasta) = (indice("posicion")?, indice("nueva_posicion")?);
    ejecutar_en_interfaz(move |controlador| match controlador.mover_pista(desde, hasta) {
        Ok(estado) => texto(&format!("Pista movida: {estado}."), false),
        Err(error) => texto(&error, true),
    })
}

fn ordenar_lista(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let por_bpm = match texto_opcional(argumentos, "orden")?.as_deref() {
        Some("azar") => false,
        Some("bpm") => true,
        _ => return Err("Indique 'orden': azar o bpm.".into()),
    };
    ejecutar_en_interfaz(move |controlador| {
        let resultado = if por_bpm { controlador.ordenar_lista_por_bpm() } else { controlador.mezclar_lista() };
        match resultado {
            Ok(estado) => texto(&format!("{estado}."), false),
            Err(error) => texto(&error, true),
        }
    })
}

/// Extensiones con que se puede guardar una lista: la del reproductor y M3U.
const EXTENSIONES_DE_LISTA: [&str; 3] = ["json", "m3u", "m3u8"];

/// Archivo donde guardar la lista pedida: un nombre suelto va a la carpeta de
/// listas; una ruta debe ser absoluta. Sin extension se usa .json. Con
/// `prefijo`, se antepone al nombre del archivo.
fn destino_de_lista(nombre: &str, prefijo: Option<&str>, carpeta_de_listas: &Path) -> Result<PathBuf, String> {
    let nombre = nombre.trim();
    if nombre.is_empty() || nombre.contains('\0') {
        return Err("Indique un 'nombre' de archivo valido.".into());
    }
    let pedido = Path::new(nombre);
    let es_nombre_suelto = pedido.components().count() == 1 && !pedido.has_root();
    let mut destino = if es_nombre_suelto {
        if nombre == "." || nombre == ".." {
            return Err(format!("Nombre de lista invalido: {nombre}"));
        }
        carpeta_de_listas.join(nombre)
    } else {
        ruta_absoluta(nombre)?
    };
    let archivo = destino
        .file_name()
        .and_then(|archivo| archivo.to_str())
        .filter(|archivo| !archivo.is_empty())
        .ok_or_else(|| format!("Falta el nombre del archivo en: {nombre}"))?
        .to_string();
    if let Some(prefijo) = prefijo {
        destino.set_file_name(format!("{prefijo}{archivo}"));
    }
    match destino.extension().and_then(|extension| extension.to_str()) {
        None => Ok(destino.with_extension("json")),
        Some(extension) if EXTENSIONES_DE_LISTA.iter().any(|valida| valida.eq_ignore_ascii_case(extension)) => Ok(destino),
        Some(_) => {
            // Un punto en el nombre ("lista 1.5") no es una extension de lista.
            let mut con_json = destino.into_os_string();
            con_json.push(".json");
            Ok(PathBuf::from(con_json))
        }
    }
}

/// Fecha y hora local como prefijo de nombre de archivo: AAAAMMDDhhmm_.
fn prefijo_de_fecha() -> String {
    jiff::Zoned::now().strftime("%Y%m%d%H%M_").to_string()
}

fn guardar_lista(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let nombre = texto_opcional(argumentos, "nombre")?.filter(|nombre| !nombre.trim().is_empty());
    let con_fecha = booleano_opcional(argumentos, "con_fecha")?.unwrap_or(false);
    let sobrescribir = booleano_opcional(argumentos, "sobrescribir")?.unwrap_or(false);
    let destino = match nombre {
        Some(nombre) => {
            let prefijo = con_fecha.then(prefijo_de_fecha);
            let destino = destino_de_lista(&nombre, prefijo.as_deref(), &carpeta_de_listas())?;
            if destino.is_dir() {
                return Err(format!("{} es una carpeta; indique un nombre de archivo.", destino.display()));
            }
            Some(destino)
        }
        None if con_fecha => return Err("Con 'con_fecha' indique tambien un 'nombre'.".into()),
        None => None,
    };
    ejecutar_en_interfaz(move |controlador| {
        let abierta = controlador.archivo_de_lista_abierta();
        let Some(destino) = destino.or_else(|| abierta.clone()) else {
            return texto("La lista no tiene nombre todavia: indique un 'nombre'.", true);
        };
        let es_la_abierta = abierta.as_deref().is_some_and(|abierta| normalizar(abierta) == normalizar(&destino));
        if destino.exists() && !sobrescribir && !es_la_abierta {
            return texto(
                &format!("Ya existe {}; use 'sobrescribir' para reemplazarlo o elija otro nombre.", destino.display()),
                true,
            );
        }
        match controlador.guardar_lista_en(&destino) {
            Ok(_) => texto(
                &format!("Lista guardada en {} ({} pista(s)).", destino.display(), controlador.pistas_en_lista().len()),
                false,
            ),
            Err(error) => texto(&error, true),
        }
    })
}

/// Localidad con su region y coordenadas, para mostrarla al agente.
fn describir_localidad(localidad: &Localidad, region: &str) -> String {
    let donde = if region.is_empty() { localidad.nombre.clone() } else { format!("{} ({region})", localidad.nombre) };
    format!("{donde} [{:.4}, {:.4}]", localidad.latitud, localidad.longitud)
}

/// Deja las localidades cuya region tiene todas las palabras pedidas, sin
/// importar mayusculas ni acentos; sin region, las deja todas.
fn filtrar_por_region(encontradas: Vec<LocalidadEncontrada>, region: Option<&str>) -> Vec<LocalidadEncontrada> {
    let palabras: Vec<String> = region.map(simplificar_texto).unwrap_or_default().split_whitespace().map(str::to_string).collect();
    encontradas
        .into_iter()
        .filter(|encontrada| {
            let donde = simplificar_texto(&encontrada.region);
            palabras.iter().all(|palabra| donde.contains(palabra.as_str()))
        })
        .collect()
}

fn localidad_del_clima(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let nombre = texto_opcional(argumentos, "nombre")?.filter(|nombre| !nombre.trim().is_empty());
    let region = texto_opcional(argumentos, "region")?.filter(|region| !region.trim().is_empty());
    let numero = entero_opcional(argumentos, "numero")?;
    if numero.is_some_and(|numero| numero < 1) {
        return Err("'numero' empieza en 1.".into());
    }
    let Some(nombre) = nombre else {
        if region.is_some() || numero.is_some() {
            return Err("Indique el 'nombre' de la localidad a buscar.".into());
        }
        let actual = ejecutar_en_interfaz(|controlador| controlador.localidad_del_clima())?;
        return Ok(texto(&format!("Localidad del clima del anuncio HTH: {}.", describir_localidad(&actual, "")), false));
    };
    // Al aire no se cambia la localidad (fija la hora): ni se busca.
    ejecutar_en_interfaz(|controlador| controlador.admitida(crate::dominio::OperacionSobreLaSecuencia::CambiarLocalidad))??;
    // La busqueda sale a la red: se hace en este hilo, sin trabar la interfaz.
    let encontradas = filtrar_por_region(buscar_localidades(&nombre)?, region.as_deref());
    let filtro = region.as_deref().map(|region| format!(" en «{region}»")).unwrap_or_default();
    let elegida = match (encontradas.len(), numero) {
        (0, _) => return Err(format!("No se encontro «{nombre}»{filtro}. Pruebe con otro nombre, sin abreviaturas, u otra region.")),
        (cantidad, Some(numero)) if numero as usize > cantidad => {
            return Err(format!("Hay {cantidad} coincidencia(s) para «{nombre}»{filtro}; 'numero' debe ser de 1 a {cantidad}."));
        }
        (_, Some(numero)) => encontradas[numero as usize - 1].clone(),
        (1, None) => encontradas[0].clone(),
        (cantidad, None) => {
            let mut salida = format!("{cantidad} localidad(es) para «{nombre}»{filtro}; no se cambio nada. Elija una con 'numero' o afine con 'region':");
            for (indice, encontrada) in encontradas.iter().enumerate() {
                salida.push_str(&format!("\n{}. {}", indice + 1, describir_localidad(&encontrada.localidad, &encontrada.region)));
            }
            return Ok(texto(&salida, false));
        }
    };
    let descripcion = describir_localidad(&elegida.localidad, &elegida.region);
    ejecutar_en_interfaz(move |controlador| {
        controlador
            .fijar_localidad_del_clima(elegida)
            .map(|_| texto(&format!("Localidad del clima del anuncio HTH: {descripcion}. Consultando el clima..."), false))
    })?
}

fn numero_de_boton(argumentos: &Map<String, Value>) -> Result<usize, String> {
    let ultimo = CANTIDAD_DE_BOTONES - 1;
    entero_opcional(argumentos, "boton")?
        .and_then(|boton| usize::try_from(boton).ok())
        .filter(|boton| *boton < CANTIDAD_DE_BOTONES)
        .ok_or_else(|| format!("Indique 'boton' entre 0 y {ultimo}."))
}

/// Hoja pedida (desde 0), o None para la que esta a la vista.
fn hoja_pedida(argumentos: &Map<String, Value>) -> Result<Option<usize>, String> {
    match entero_opcional(argumentos, "hoja")? {
        Some(hoja) if hoja >= 1 => Ok(Some(hoja as usize - 1)),
        Some(_) => Err("'hoja' empieza en 1.".into()),
        None => Ok(None),
    }
}

/// Resuelve la hoja contra la botonera: la pedida si existe, o la visible.
fn hoja_valida(controlador: &Controlador, hoja: Option<usize>) -> Result<usize, String> {
    let cantidad = controlador.cantidad_de_hojas();
    match hoja {
        Some(hoja) if hoja >= cantidad => Err(format!("No existe la hoja {} (hay {cantidad}).", hoja + 1)),
        Some(hoja) => Ok(hoja),
        None => Ok(controlador.hoja_actual()),
    }
}

fn asignar_efecto(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let boton = numero_de_boton(argumentos)?;
    let hoja = hoja_pedida(argumentos)?;
    let ruta = ruta_absoluta(&texto_opcional(argumentos, "ruta")?.ok_or("Indique 'ruta'.")?)?;
    if !ruta.is_file() {
        return Err(format!("{}: no existe o no es accesible.", ruta.display()));
    }
    if !Pista::es_formato_soportado(&ruta) {
        return Err(format!("{}: formato no soportado.", ruta.display()));
    }
    ejecutar_en_interfaz(move |controlador| match hoja_valida(controlador, hoja) {
        Ok(hoja) => match controlador.asignar_efecto(hoja, boton, &ruta) {
            Ok(()) => texto(&format!("Hoja {}, boton {boton}: {}.", hoja + 1, ruta.display()), false),
            Err(error) => texto(&error, true),
        },
        Err(error) => texto(&error, true),
    })
}

fn quitar_efecto(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let boton = numero_de_boton(argumentos)?;
    let hoja = hoja_pedida(argumentos)?;
    ejecutar_en_interfaz(move |controlador| match hoja_valida(controlador, hoja) {
        Ok(hoja) => {
            controlador.quitar_efecto(hoja, boton);
            texto(&format!("Hoja {}, boton {boton} vacio.", hoja + 1), false)
        }
        Err(error) => texto(&error, true),
    })
}

fn disparar_efecto(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let boton = numero_de_boton(argumentos)?;
    let hoja = hoja_pedida(argumentos)?;
    ejecutar_en_interfaz(move |controlador| match controlador.pulsar_efecto(hoja, boton) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn insertar_hth(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let posicion = match entero_opcional(argumentos, "posicion")? {
        Some(posicion) if posicion < 1 => return Err("'posicion' empieza en 1.".into()),
        otra => otra.map(|posicion| posicion as usize - 1),
    };
    ejecutar_en_interfaz(move |controlador| match controlador.insertar_anuncio_hth(posicion) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn encolar_pista(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let posicion = match entero_opcional(argumentos, "posicion")? {
        Some(posicion) if posicion >= 1 => posicion as usize - 1,
        _ => return Err("Indique 'posicion' (desde 1).".into()),
    };
    let sacar_del_bloque = match argumentos.get("sacar_del_bloque") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(valor)) => *valor,
        Some(_) => return Err("'sacar_del_bloque' debe ser true o false.".into()),
    };
    ejecutar_en_interfaz(move |controlador| {
        if !sacar_del_bloque
            && let Some(aviso) = controlador.aviso_de_salida_de_bloque(posicion)
        {
            return texto(&format!("{aviso} No se movio: repita con sacar_del_bloque en true para confirmarlo."), true);
        }
        match controlador.encolar_como_siguiente(posicion) {
            Ok(estado) => texto(&estado, false),
            Err(error) => texto(&error, true),
        }
    })
}

fn buscar_en_catalogo(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let texto_buscado = texto_opcional(argumentos, "texto")?.unwrap_or_default();
    let limite = match entero_opcional(argumentos, "limite")? {
        None => 50,
        Some(limite @ 1..=500) => limite as usize,
        Some(_) => return Err("'limite' va de 1 a 500.".into()),
    };
    let (temas, total) = ejecutar_en_interfaz(move |controlador| controlador.temas_del_catalogo(&texto_buscado, limite))??;
    if temas.is_empty() {
        return Ok(texto("Ningun tema del catalogo coincide.", false));
    }
    let mut salida = format!("{total} tema(s) coinciden; se muestran {}:\n", temas.len());
    for (orden, tema) in temas.iter().enumerate() {
        salida.push_str(&format!("{}. {}", orden + 1, tema.titulo));
        if let Some(tipo) = tema.ficha.tipo {
            salida.push_str(&format!(" | tipo {}", tipo.nombre()));
        }
        for (campo, valor) in [
            ("artista", &tema.ficha.artista),
            ("genero", &tema.ficha.genero),
            ("album", &tema.ficha.album),
            ("nota", &tema.ficha.nota),
        ] {
            if let Some(valor) = valor {
                salida.push_str(&format!(" | {campo} {valor}"));
            }
        }
        if let Some(anio) = tema.ficha.anio {
            salida.push_str(&format!(" | anio {anio}"));
        }
        if let Some(duracion) = tema.ficha.duracion {
            salida.push_str(&format!(" | duracion {}", formatear_minutos(duracion)));
        }
        if tema.en_lista {
            salida.push_str(" | ya esta en la lista");
        }
        if !tema.existe {
            salida.push_str(" | SIN ARCHIVO");
        }
        salida.push_str(&format!(" | ruta {}\n", tema.archivo.display()));
    }
    Ok(texto(&salida, false))
}

fn bloque_fijo(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let posicion = |clave: &str| match entero_opcional(argumentos, clave)? {
        Some(posicion) if posicion >= 1 => Ok(posicion as usize - 1),
        _ => Err(format!("Indique '{clave}' (desde 1).")),
    };
    let (desde, hasta) = (posicion("desde")?, posicion("hasta")?);
    ejecutar_en_interfaz(move |controlador| match controlador.fijar_bloque(desde, hasta) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

/// Minutos de tolerancia como segundos, entre 0 y una hora.
fn tolerancia(argumentos: &Map<String, Value>, clave: &str, por_defecto: u32) -> Result<u32, String> {
    match numero_opcional(argumentos, clave)? {
        None => Ok(por_defecto),
        Some(minutos) if (0.0..=60.0).contains(&minutos) => Ok((minutos * 60.0).round() as u32),
        Some(_) => Err(format!("'{clave}' va de 0 a 60 minutos.")),
    }
}

fn programar_tanda(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let nombre = texto_opcional(argumentos, "nombre")?.filter(|nombre| !nombre.trim().is_empty()).ok_or("Indique 'nombre'.")?;
    let rutas = textos(argumentos, "archivos")?;
    if rutas.is_empty() {
        return Err("Indique al menos un archivo en 'archivos'.".into());
    }
    let mut archivos = Vec::with_capacity(rutas.len());
    for ruta in &rutas {
        let archivo = ruta_absoluta(ruta)?;
        if !archivo.is_file() {
            return Err(format!("No existe el archivo {ruta}."));
        }
        archivos.push(archivo);
    }
    let inicio = parsear_hora(&texto_opcional(argumentos, "inicio")?.ok_or("Indique 'inicio' (HH:MM).")?)?;
    let hasta = texto_opcional(argumentos, "hasta")?.map(|hasta| parsear_hora(&hasta)).transpose()?;
    let cada = match entero_opcional(argumentos, "cada_minutos")? {
        Some(minutos) if (1..=1_440).contains(&minutos) => Some(minutos as u32),
        Some(_) => return Err("'cada_minutos' va de 1 a 1440.".into()),
        None => None,
    };
    let dias = match argumentos.get("dias") {
        None | Some(Value::Null) => [true; 7],
        Some(_) => parsear_dias(&textos(argumentos, "dias")?)?,
    };
    let activa = match argumentos.get("activa") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(activa)) => *activa,
        Some(_) => return Err("'activa' debe ser true o false.".into()),
    };
    let tanda = TandaProgramada {
        nombre: nombre.trim().to_string(),
        archivos,
        recurrencia: Recurrencia::nueva(inicio, cada, hasta, dias)?,
        antes: tolerancia(argumentos, "antes_minutos", TOLERANCIA_PREVIA_POR_DEFECTO)?,
        despues: tolerancia(argumentos, "despues_minutos", TOLERANCIA_POSTERIOR_POR_DEFECTO)?,
        modo: texto_opcional(argumentos, "modo")?.map(|modo| ModoDeEntrada::desde_clave(&modo)).transpose()?.unwrap_or_default(),
        prioridad: match entero_opcional(argumentos, "prioridad")? {
            None => PRIORIDAD_POR_DEFECTO,
            Some(prioridad) if (i64::from(PRIORIDAD_MINIMA)..=i64::from(PRIORIDAD_MAXIMA)).contains(&prioridad) => prioridad as u8,
            Some(_) => return Err(format!("'prioridad' va de {PRIORIDAD_MINIMA} a {PRIORIDAD_MAXIMA}.")),
        },
        duracion_maxima: match entero_opcional(argumentos, "duracion_maxima_segundos")? {
            None | Some(0) => None,
            Some(segundos) if (1..=1_800).contains(&segundos) => Some(segundos as u32),
            Some(_) => return Err("'duracion_maxima_segundos' va de 0 (sin limite) a 1800.".into()),
        },
        activa,
        ultima_salida: None,
    };
    ejecutar_en_interfaz(move |controlador| match controlador.programar_tanda(tanda) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn encolar_tanda(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let nombre = texto_opcional(argumentos, "nombre")?.ok_or("Indique 'nombre'.")?;
    ejecutar_en_interfaz(move |controlador| match controlador.encolar_tanda_ahora(&nombre) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn quitar_tanda(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let nombre = texto_opcional(argumentos, "nombre")?.ok_or("Indique 'nombre'.")?;
    ejecutar_en_interfaz(move |controlador| match controlador.quitar_tanda(&nombre) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn fecha_opcional(argumentos: &Map<String, Value>, clave: &str) -> Result<Option<jiff::civil::Date>, String> {
    texto_opcional(argumentos, clave)?
        .map(|texto| texto.trim().parse::<jiff::civil::Date>().map_err(|_| format!("'{clave}' debe ser una fecha AAAA-MM-DD.")))
        .transpose()
}

fn programar_orden(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let obligatorio = |clave: &str| -> Result<String, String> {
        texto_opcional(argumentos, clave)?.filter(|valor| !valor.trim().is_empty()).map(|valor| valor.trim().to_string()).ok_or_else(|| format!("Indique '{clave}'."))
    };
    let frecuencias = [argumentos.contains_key("cada_minutos"), argumentos.contains_key("veces_por_dia"), argumentos.contains_key("horarios")];
    let frecuencia = match frecuencias {
        [true, false, false] => Frecuencia::CadaMinutos(entero_opcional(argumentos, "cada_minutos")?.unwrap_or(0).clamp(0, 100_000) as u32),
        [false, true, false] => Frecuencia::VecesPorDia(entero_opcional(argumentos, "veces_por_dia")?.unwrap_or(0).clamp(0, 1_000) as u32),
        [false, false, true] => Frecuencia::EnHorarios(textos(argumentos, "horarios")?.iter().map(|horario| parsear_hora(horario)).collect::<Result<_, _>>()?),
        _ => return Err("Indique una sola frecuencia: 'cada_minutos', 'veces_por_dia' u 'horarios'.".into()),
    };
    let mut avisos = Vec::new();
    for ruta in textos(argumentos, "avisos")? {
        let archivo = ruta_absoluta(&ruta)?;
        if !archivo.is_file() {
            return Err(format!("No existe el archivo {ruta}."));
        }
        avisos.push(archivo);
    }
    let hora = |clave: &str, por_defecto: &str| -> Result<u32, String> {
        parsear_hora(texto_opcional(argumentos, clave)?.as_deref().unwrap_or(por_defecto))
    };
    let orden = OrdenDePublicidad {
        nombre: obligatorio("nombre")?,
        anunciante: obligatorio("anunciante")?,
        rubro: texto_opcional(argumentos, "rubro")?.unwrap_or_default().trim().to_string(),
        desde: fecha_opcional(argumentos, "desde")?.ok_or("Indique 'desde' (AAAA-MM-DD).")?,
        hasta: fecha_opcional(argumentos, "hasta")?.ok_or("Indique 'hasta' (AAAA-MM-DD).")?,
        dias: match argumentos.get("dias") {
            None | Some(Value::Null) => [true; 7],
            Some(_) => parsear_dias(&textos(argumentos, "dias")?)?,
        },
        franja: (hora("franja_desde", "00:00")?, hora("franja_hasta", "23:59")?),
        frecuencia,
        avisos,
        tandas: match argumentos.get("tandas") {
            None | Some(Value::Null) => Vec::new(),
            Some(_) => textos(argumentos, "tandas")?,
        },
        prioridad: match entero_opcional(argumentos, "prioridad")? {
            None => PRIORIDAD_POR_DEFECTO,
            Some(prioridad) if (i64::from(PRIORIDAD_MINIMA)..=i64::from(PRIORIDAD_MAXIMA)).contains(&prioridad) => prioridad as u8,
            Some(_) => return Err(format!("'prioridad' va de {PRIORIDAD_MINIMA} a {PRIORIDAD_MAXIMA}.")),
        },
        activa: match argumentos.get("activa") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(activa)) => *activa,
            Some(_) => return Err("'activa' debe ser true o false.".into()),
        },
    };
    orden.validar()?;
    ejecutar_en_interfaz(move |controlador| match controlador.programar_orden(orden) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn quitar_orden(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let nombre = texto_opcional(argumentos, "nombre")?.ok_or("Indique 'nombre'.")?;
    ejecutar_en_interfaz(move |controlador| match controlador.quitar_orden(&nombre) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn registrar_anunciante(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let nombre = texto_opcional(argumentos, "nombre")?.ok_or("Indique 'nombre'.")?;
    let anterior = texto_opcional(argumentos, "nombre_anterior")?;
    let mut cambios = Vec::new();
    for clave in ["razon_social", "cuit", "rubro", "contacto", "telefono", "correo", "direccion", "agencia", "notas"] {
        if let Some(valor) = texto_opcional(argumentos, clave)? {
            cambios.push((clave, valor));
        }
    }
    let activo = booleano_opcional(argumentos, "activo")?;
    ejecutar_en_interfaz(move |controlador| {
        // Parte de la ficha que tiene (la anterior si se renombra) y cambia lo indicado.
        let base = anterior.as_deref().unwrap_or(&nombre);
        let guardada = controlador.ficha_de_anunciante(base).cloned();
        // Sin renombrar, el nombre solo identifica la ficha: conserva como esta escrito.
        let conserva_el_nombre = anterior.is_none() && guardada.is_some();
        let mut ficha = guardada.unwrap_or(crate::dominio::Anunciante { activo: true, ..Default::default() });
        if !conserva_el_nombre {
            ficha.nombre = nombre.clone();
        }
        for (clave, valor) in cambios {
            let campo = match clave {
                "razon_social" => &mut ficha.razon_social,
                "cuit" => &mut ficha.cuit,
                "rubro" => &mut ficha.rubro,
                "contacto" => &mut ficha.contacto,
                "telefono" => &mut ficha.telefono,
                "correo" => &mut ficha.correo,
                "direccion" => &mut ficha.direccion,
                "agencia" => &mut ficha.agencia,
                _ => &mut ficha.notas,
            };
            *campo = valor;
        }
        if let Some(activo) = activo {
            ficha.activo = activo;
        }
        match controlador.registrar_anunciante(ficha, anterior.as_deref()) {
            Ok(estado) => texto(&estado, false),
            Err(error) => texto(&error, true),
        }
    })
}

fn quitar_anunciante(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let nombre = texto_opcional(argumentos, "nombre")?.ok_or("Indique 'nombre'.")?;
    ejecutar_en_interfaz(move |controlador| match controlador.quitar_anunciante(&nombre) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn pauta_del_dia(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let fecha = fecha_opcional(argumentos, "fecha")?.unwrap_or_else(|| jiff::Zoned::now().date());
    ejecutar_en_interfaz(move |controlador| texto(&controlador.describir_pauta_del_dia(fecha), false))
}

fn certificado(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let anunciante = texto_opcional(argumentos, "anunciante")?.filter(|nombre| !nombre.trim().is_empty()).ok_or("Indique 'anunciante'.")?;
    let hoy = jiff::Zoned::now().date();
    let desde = fecha_opcional(argumentos, "desde")?.unwrap_or(hoy.first_of_month());
    let hasta = fecha_opcional(argumentos, "hasta")?.unwrap_or(hoy);
    if hasta < desde {
        return Err("'hasta' es anterior a 'desde'.".into());
    }
    ejecutar_en_interfaz(move |controlador| match controlador.certificado_de_emision(&anunciante, desde, hasta) {
        Ok((resumen, _)) => texto(&resumen, false),
        Err(error) => texto(&error, true),
    })
}

fn registro_de_comerciales(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let fecha = |clave: &str| -> Result<Option<jiff::civil::Date>, String> {
        texto_opcional(argumentos, clave)?
            .map(|texto| texto.trim().parse::<jiff::civil::Date>().map_err(|_| format!("'{clave}' debe ser una fecha AAAA-MM-DD.")))
            .transpose()
    };
    let desde = fecha("desde")?.unwrap_or_else(|| jiff::Zoned::now().date());
    let hasta = fecha("hasta")?.unwrap_or(desde);
    let filtro = texto_opcional(argumentos, "texto")?;
    ejecutar_en_interfaz(move |controlador| match controlador.consultar_emisiones(desde, hasta, filtro.as_deref()) {
        Ok(salida) => texto(&salida, false),
        Err(error) => texto(&error, true),
    })
}

fn base_hth(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let posicion = match entero_opcional(argumentos, "posicion")? {
        Some(posicion) if posicion >= 1 => posicion as usize - 1,
        _ => return Err("Indique 'posicion' (desde 1).".into()),
    };
    let base = match entero_opcional(argumentos, "base")? {
        Some(0) => None,
        Some(base) if base >= 1 => Some(base as usize - 1),
        _ => return Err("Indique 'base': el numero de la base (desde 1) o 0 para quitarla.".into()),
    };
    ejecutar_en_interfaz(move |controlador| match controlador.cambiar_base_hth(posicion, base) {
        Ok(estado) => texto(&estado, false),
        Err(error) => texto(&error, true),
    })
}

fn hojas_de_efectos(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let hoja = hoja_pedida(argumentos)?;
    let nombre = texto_opcional(argumentos, "nombre")?;
    if nombre.as_deref().is_some_and(|nombre| nombre.trim().chars().count() > LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA) {
        return Err(format!("'nombre' puede tener hasta {LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA} caracteres."));
    }
    let accion = match (texto_opcional(argumentos, "accion")?.as_deref(), hoja) {
        (Some("agregar"), _) => AccionDeHoja::Agregar,
        (Some("ir"), Some(hoja)) => AccionDeHoja::Ir(hoja),
        (Some("quitar"), Some(hoja)) => AccionDeHoja::Quitar(hoja),
        (Some("renombrar"), hoja) => {
            let nombre = nombre.clone().ok_or("Indique 'nombre' (vacio vuelve a 'Hoja N').")?;
            AccionDeHoja::Renombrar(hoja, nombre)
        }
        (Some("ir" | "quitar"), None) => return Err("Indique 'hoja'.".into()),
        (Some(otra), _) => return Err(format!("Accion desconocida: {otra}.")),
        (None, _) => return Err("Indique 'accion'.".into()),
    };
    let es_agregar = matches!(accion, AccionDeHoja::Agregar);
    ejecutar_en_interfaz(move |controlador| {
        let mut resultado = controlador.hojas_de_efectos(accion);
        // La hoja agregada queda a la vista: se le pone el nombre pedido.
        if let (true, Ok(agregada), Some(nombre)) = (es_agregar, &resultado, nombre) {
            let agregada = agregada.clone();
            resultado = controlador
                .hojas_de_efectos(AccionDeHoja::Renombrar(None, nombre))
                .map(|nombrada| format!("{agregada} {nombrada}"));
        }
        match resultado {
            Ok(estado) => texto(&estado, false),
            Err(error) => texto(&error, true),
        }
    })
}

fn agregar_pistas(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let rutas = textos(argumentos, "rutas")?;
    if rutas.is_empty() {
        return Err("Indique al menos una ruta en 'rutas'.".into());
    }
    let posicion = match entero_opcional(argumentos, "posicion")? {
        Some(posicion) if posicion >= 1 => Some(posicion as usize - 1),
        Some(_) => return Err("'posicion' empieza en 1.".into()),
        None => None,
    };

    let subcarpetas = argumentos.get("subcarpetas").and_then(Value::as_bool).unwrap_or(false);
    let mut archivos: Vec<PathBuf> = Vec::new();
    let mut omitidas = Vec::new();
    for texto_de_ruta in &rutas {
        let ruta = match ruta_absoluta(texto_de_ruta) {
            Ok(ruta) => ruta,
            Err(error) => {
                omitidas.push(format!("{texto_de_ruta}: {error}"));
                continue;
            }
        };
        if ruta.is_dir() {
            match audios_de_carpeta(&ruta, subcarpetas) {
                Ok(de_carpeta) if de_carpeta.archivos.is_empty() => {
                    omitidas.push(format!("{texto_de_ruta}: la carpeta no tiene archivos de audio"));
                }
                Ok(de_carpeta) => archivos.extend(de_carpeta.archivos),
                Err(error) => omitidas.push(format!("{texto_de_ruta}: {error}")),
            }
        } else if !ruta.is_file() {
            omitidas.push(format!("{texto_de_ruta}: no existe o no es accesible"));
        } else if !Pista::es_formato_soportado(&ruta) {
            omitidas.push(format!("{texto_de_ruta}: formato no soportado"));
        } else {
            archivos.push(ruta);
        }
    }
    let mut vistas = HashSet::new();
    archivos.retain(|archivo| vistas.insert(Pista::new(archivo)));

    let candidatas = archivos.len();
    let (agregadas, de_la_botonera) = if archivos.is_empty() {
        (0, Vec::new())
    } else {
        ejecutar_en_interfaz(move |controlador| {
            // Los de tipo Efecto solo pueden estar en la botonera.
            let (admitidos, de_la_botonera): (Vec<PathBuf>, Vec<PathBuf>) =
                archivos.into_iter().partition(|archivo| !controlador.es_solo_para_la_botonera(archivo));
            let agregadas = if admitidos.is_empty() { 0 } else { controlador.agregar_pistas_en(admitidos, posicion) };
            (agregadas, de_la_botonera)
        })?
    };
    for archivo in &de_la_botonera {
        omitidas.push(format!("{}: es de tipo Efecto, solo puede estar en la botonera", archivo.display()));
    }
    let duplicadas = candidatas - agregadas - de_la_botonera.len();

    let mut salida = format!("Se agregaron {agregadas} pista(s).");
    if duplicadas > 0 {
        salida.push_str(&format!(" {duplicadas} ya estaban en la lista."));
    }
    if !omitidas.is_empty() {
        salida.push_str("\nOmitidas:");
        for motivo in &omitidas {
            salida.push_str(&format!("\n- {motivo}"));
        }
    }
    Ok(texto(&salida, candidatas == 0))
}

fn quitar_pistas(argumentos: &Map<String, Value>) -> Result<Value, String> {
    let posiciones = enteros(argumentos, "posiciones")?;
    let rutas = textos(argumentos, "rutas")?;
    if posiciones.is_empty() && rutas.is_empty() {
        return Err("Indique 'posiciones' o 'rutas' de las pistas a quitar.".into());
    }
    let rutas_pedidas = rutas.iter().map(|ruta| ruta_absoluta(ruta)).collect::<Result<Vec<_>, _>>()?;

    // Se resuelven todas las pistas sobre la misma foto de la lista antes de
    // quitar, para que las posiciones no se corran entre una y otra.
    ejecutar_en_interfaz(move |controlador| {
        let pistas = controlador.pistas_en_lista();
        // Una posicion quita esa fila; una ruta, todas sus copias.
        let mut a_quitar: BTreeSet<usize> = BTreeSet::new();
        let mut no_encontradas = Vec::new();
        for posicion in posiciones {
            match usize::try_from(posicion - 1).ok().filter(|&indice| indice < pistas.len()) {
                Some(indice) => {
                    a_quitar.insert(indice);
                }
                None => no_encontradas.push(format!("posicion {posicion}: fuera de la lista (1 a {})", pistas.len())),
            }
        }
        for ruta in rutas_pedidas {
            let pista = Pista::new(&ruta);
            let copias: Vec<usize> = (0..pistas.len()).filter(|&indice| pistas[indice] == pista).collect();
            if copias.is_empty() {
                no_encontradas.push(format!("{}: no esta en la lista", ruta.display()));
            }
            a_quitar.extend(copias);
        }

        let posiciones: Vec<usize> = a_quitar.into_iter().collect();
        if let Err(motivo) = controlador.admitida(crate::dominio::OperacionSobreLaSecuencia::Quitar(posiciones.len())) {
            return texto(&motivo, true);
        }
        let quitadas = if posiciones.is_empty() { 0 } else { controlador.quitar_posiciones(&posiciones) };
        let mut salida = format!("Se quitaron {quitadas} pista(s)");
        if !posiciones.is_empty() {
            let nombres: Vec<String> = posiciones.iter().map(|&indice| pistas[indice].nombre()).collect();
            salida.push_str(&format!(": {}", nombres.join(", ")));
        }
        salida.push('.');
        if !no_encontradas.is_empty() {
            salida.push_str("\nNo encontradas:");
            for motivo in &no_encontradas {
                salida.push_str(&format!("\n- {motivo}"));
            }
        }
        texto(&salida, quitadas == 0)
    })
}

fn ruta_absoluta(texto: &str) -> Result<PathBuf, String> {
    let ruta = Path::new(texto);
    if texto.is_empty() || texto.contains('\0') {
        return Err(format!("Ruta invalida: {texto}"));
    }
    if !ruta.is_absolute() {
        return Err(format!("La ruta debe ser absoluta: {texto}"));
    }
    Ok(normalizar(ruta))
}

fn segundos(valor: f64) -> String {
    let texto = format!("{valor:.2}");
    format!("{} s", texto.trim_end_matches('0').trim_end_matches('.'))
}

fn arreglo<'a>(argumentos: &'a Map<String, Value>, clave: &str) -> Result<&'a [Value], String> {
    match argumentos.get(clave) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(valores)) => Ok(valores),
        Some(_) => Err(format!("'{clave}' debe ser una lista.")),
    }
}

fn textos(argumentos: &Map<String, Value>, clave: &str) -> Result<Vec<String>, String> {
    arreglo(argumentos, clave)?
        .iter()
        .map(|valor| valor.as_str().map(str::to_string).ok_or_else(|| format!("'{clave}' debe ser una lista de textos.")))
        .collect()
}

fn enteros(argumentos: &Map<String, Value>, clave: &str) -> Result<Vec<i64>, String> {
    arreglo(argumentos, clave)?
        .iter()
        .map(|valor| {
            valor
                .as_f64()
                .filter(|numero| numero.fract() == 0.0)
                .map(|numero| numero as i64)
                .ok_or_else(|| format!("'{clave}' debe ser una lista de numeros enteros."))
        })
        .collect()
}

fn texto_opcional(argumentos: &Map<String, Value>, clave: &str) -> Result<Option<String>, String> {
    match argumentos.get(clave) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(valor)) => Ok(Some(valor.clone())),
        Some(_) => Err(format!("'{clave}' debe ser un texto.")),
    }
}

fn booleano_opcional(argumentos: &Map<String, Value>, clave: &str) -> Result<Option<bool>, String> {
    match argumentos.get(clave) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(valor)) => Ok(Some(*valor)),
        Some(_) => Err(format!("'{clave}' debe ser true o false.")),
    }
}

fn numero_opcional(argumentos: &Map<String, Value>, clave: &str) -> Result<Option<f64>, String> {
    match argumentos.get(clave) {
        None | Some(Value::Null) => Ok(None),
        Some(valor) => valor.as_f64().map(Some).ok_or_else(|| format!("'{clave}' debe ser un numero.")),
    }
}

fn entero_opcional(argumentos: &Map<String, Value>, clave: &str) -> Result<Option<i64>, String> {
    match numero_opcional(argumentos, clave)? {
        Some(numero) if numero.fract() != 0.0 => Err(format!("'{clave}' debe ser un numero entero.")),
        otro => Ok(otro.map(|numero| numero as i64)),
    }
}

/// Tiempo dado como numero de segundos o como texto "m:ss".
fn tiempo_opcional(argumentos: &Map<String, Value>, clave: &str) -> Result<Option<f64>, String> {
    match argumentos.get(clave) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(texto)) => parsear_tiempo(texto).map(Some).map_err(|error| format!("'{clave}': {error}")),
        Some(_) => segundos_opcionales(argumentos, clave),
    }
}

fn segundos_opcionales(argumentos: &Map<String, Value>, clave: &str) -> Result<Option<f64>, String> {
    match numero_opcional(argumentos, clave)? {
        Some(numero) if !numero.is_finite() || numero < 0.0 => {
            Err(format!("'{clave}' debe ser un numero de segundos mayor o igual a 0."))
        }
        otro => Ok(otro),
    }
}

fn texto(mensaje: &str, es_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": mensaje }], "isError": es_error })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn cada_herramienta_tiene_definicion() {
        let definidas: Vec<String> = definiciones()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|herramienta| herramienta["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(definidas, HERRAMIENTAS);
    }

    #[test]
    fn valida_argumentos_antes_de_tocar_la_interfaz() {
        let argumentos = |valor: Value| valor.as_object().cloned().unwrap();
        let es_error = |resultado: Value| resultado["isError"] == true;
        assert!(es_error(ejecutar(CONTROLAR, &argumentos(json!({"accion": "volar"})))));
        assert!(es_error(ejecutar(AJUSTAR, &argumentos(json!({"volumen": 150})))));
        assert!(es_error(ejecutar(AJUSTAR, &argumentos(json!({"fader": 11})))));
        assert!(es_error(ejecutar(DISPARAR_EFECTO, &argumentos(json!({"boton": 10})))));
        assert!(es_error(ejecutar(DISPARAR_EFECTO, &argumentos(json!({"boton": 1, "hoja": 0})))));
        assert!(es_error(ejecutar(HOJAS, &argumentos(json!({"accion": "ir"})))));
        assert!(es_error(ejecutar(HOJAS, &argumentos(json!({"accion": "borrar_todo"})))));
        assert!(es_error(ejecutar(HOJAS, &argumentos(json!({"accion": "renombrar", "hoja": 1})))));
        let largo = "x".repeat(LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA + 1);
        assert!(es_error(ejecutar(HOJAS, &argumentos(json!({"accion": "renombrar", "nombre": largo})))));
        assert!(es_error(ejecutar(ASIGNAR_EFECTO, &argumentos(json!({"boton": 0, "ruta": "relativa.wav"})))));
        assert!(es_error(ejecutar(DEFINIR_DATOS, &argumentos(json!({"posicion": 1, "ranking": 6})))));
        assert!(es_error(ejecutar(DEFINIR_DATOS, &argumentos(json!({"posicion": 1})))));
        assert!(es_error(ejecutar(DEFINIR_MEZCLA, &argumentos(json!({"posicion": 1, "overlay_inicio": 10})))));
        let invertida = json!({"posicion": 1, "overlay_inicio": "1:45", "overlay_fin": "1:20"});
        assert!(es_error(ejecutar(DEFINIR_MEZCLA, &argumentos(invertida))));
        assert!(es_error(ejecutar(DEFINIR_MEZCLA, &argumentos(json!({"posicion": 1, "intro": "uno"})))));
        assert!(es_error(ejecutar(GUARDAR_LISTA, &argumentos(json!({"con_fecha": true})))));
        assert!(es_error(ejecutar(GUARDAR_LISTA, &argumentos(json!({"nombre": "relativa/lista"})))));
        assert!(es_error(ejecutar(GUARDAR_LISTA, &argumentos(json!({"nombre": ".."})))));
        assert!(es_error(ejecutar(GUARDAR_LISTA, &argumentos(json!({"nombre": 5})))));
        assert!(es_error(ejecutar(LOCALIDAD_DEL_CLIMA, &argumentos(json!({"region": "Entre Rios"})))));
        assert!(es_error(ejecutar(LOCALIDAD_DEL_CLIMA, &argumentos(json!({"nombre": "Parana", "numero": 0})))));
        assert!(es_error(ejecutar(LOCALIDAD_DEL_CLIMA, &argumentos(json!({"numero": 2})))));
    }

    #[test]
    fn filtra_localidades_por_region_sin_acentos_ni_mayusculas() {
        let encontrada = |nombre: &str, region: &str| LocalidadEncontrada {
            localidad: Localidad { nombre: nombre.into(), latitud: -31.73, longitud: -60.52, zona_horaria: None },
            region: region.into(),
        };
        let encontradas = vec![
            encontrada("Paraná", "Paraná, Entre Ríos, Argentina"),
            encontrada("Paranaíba", "Mato Grosso do Sul, Brasil"),
        ];
        let filtradas = filtrar_por_region(encontradas.clone(), Some("entre RIOS"));
        assert_eq!(filtradas, vec![encontradas[0].clone()]);
        assert_eq!(filtrar_por_region(encontradas.clone(), None), encontradas);
        assert!(filtrar_por_region(encontradas, Some("Cordoba")).is_empty());
    }

    #[test]
    fn destino_de_lista_resuelve_nombre_carpeta_y_extension() {
        let carpeta = std::env::temp_dir().join("listas-de-prueba");
        let destino = |nombre: &str, prefijo: Option<&str>| destino_de_lista(nombre, prefijo, &carpeta).unwrap();
        assert_eq!(destino("tarde", None), carpeta.join("tarde.json"));
        assert_eq!(destino("tarde.m3u8", None), carpeta.join("tarde.m3u8"));
        assert_eq!(destino("tarde.JSON", None), carpeta.join("tarde.JSON"));
        assert_eq!(destino("lista 1.5", None), carpeta.join("lista 1.5.json"));
        assert_eq!(destino("prueba", Some("202611161952_")), carpeta.join("202611161952_prueba.json"));
        let absoluta = std::env::temp_dir().join("otra").join("noche.m3u");
        let pedida = absoluta.to_string_lossy().into_owned();
        assert_eq!(destino(&pedida, Some("X_")), std::env::temp_dir().join("otra").join("X_noche.m3u"));
        assert!(destino_de_lista("  ", None, &carpeta).is_err());
    }

    #[test]
    fn prefijo_de_fecha_tiene_doce_digitos_y_guion_bajo() {
        let prefijo = prefijo_de_fecha();
        assert_eq!(prefijo.len(), 13);
        assert!(prefijo[..12].chars().all(|caracter| caracter.is_ascii_digit()));
        assert!(prefijo.ends_with('_'));
    }
}
