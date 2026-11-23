//! Persistencia local en ~/.mcp-raptor-player, con los mismos archivos y
//! formatos JSON que la version Java del reproductor, mas los datos nuevos
//! de cada cancion y las preferencias.

use crate::formato_de_fecha::FormatoDeFecha;
use crate::idioma::Idioma;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::audio::{Analisis, SalidaDeAudio, anio_valido};
use crate::dominio::{CampoDescriptivo, CamposCorregidos, DatosDescriptivos, ModoDeOperacion, OrigenDeLosDatos, Propuesta};
use crate::dominio::{
    Anunciante, DIAS_DE_LA_SEMANA, Frecuencia, ModoDeEntrada, OrdenDePublicidad, PautaDelDia, Reposicion, PRIORIDAD_MAXIMA, PRIORIDAD_MINIMA, PRIORIDAD_POR_DEFECTO, PuntosDeMezcla, Recurrencia, TOLERANCIA_MAXIMA, TOLERANCIA_POSTERIOR_POR_DEFECTO, TOLERANCIA_PREVIA_POR_DEFECTO,
    TandaProgramada, TipoDeElemento, VentanaDeOverlay, formatear_hora, nombre_de_archivo, normalizar, parsear_dias, parsear_hora,
};
use jiff::Span;
use jiff::civil::{Date, DateTime};
use crate::servicio::{AjustesDeEfecto, Localidad, ModoDeOverlay, ObjetivoDeSonoridad};
use crate::servicio::{SEPARACION_MAXIMA_DB, SEPARACION_MINIMA_DB};

pub const CANTIDAD_DE_BOTONES: usize = 10;
pub const RANKING_MAXIMO: u8 = 5;
pub const FADER_POR_DEFECTO: u8 = 5;
pub const FADER_MAXIMO: u8 = 15;
/// Rango de tempos que se aceptan al fijar el BPM a mano.
pub const BPM_MINIMO: f64 = 20.0;
pub const BPM_MAXIMO: f64 = 300.0;
const NIVEL_DE_MUSICA_POR_DEFECTO: f64 = 0.3;
const VOLUMEN_DE_EFECTO_MINIMO: f64 = -20.0;
const VOLUMEN_DE_EFECTO_MAXIMO: f64 = 6.0;
const ANTICIPACION_MINIMA: f64 = 0.01;
const ANTICIPACION_MAXIMA: f64 = 2.0;
const RECUPERACION_MINIMA: f64 = 0.2;
const RECUPERACION_MAXIMA: f64 = 4.0;

/// Ajustes de los efectos de una botonera nueva.
fn ajustes_de_efecto_por_defecto() -> AjustesDeEfecto {
    AjustesDeEfecto { nivel_de_musica: NIVEL_DE_MUSICA_POR_DEFECTO, ..AjustesDeEfecto::default() }
}

/// Lleva los ajustes a sus rangos validos.
fn acotar_ajustes(ajustes: AjustesDeEfecto) -> AjustesDeEfecto {
    AjustesDeEfecto {
        volumen_db: ajustes.volumen_db.clamp(VOLUMEN_DE_EFECTO_MINIMO, VOLUMEN_DE_EFECTO_MAXIMO),
        nivel_de_musica: ajustes.nivel_de_musica.clamp(0.0, 1.0),
        anticipacion: ajustes.anticipacion.clamp(ANTICIPACION_MINIMA, ANTICIPACION_MAXIMA),
        recuperacion: ajustes.recuperacion.clamp(RECUPERACION_MINIMA, RECUPERACION_MAXIMA),
        separacion_db: ajustes.separacion_db.clamp(SEPARACION_MINIMA_DB, SEPARACION_MAXIMA_DB),
        ..ajustes
    }
}
/// Permite usar otra carpeta de datos, por ejemplo para pruebas.
const VARIABLE_DE_CARPETA: &str = "RAPTOR_CARPETA_DE_DATOS";

/// Carpeta de datos del reproductor (~/.mcp-raptor-player o la indicada en
/// RAPTOR_CARPETA_DE_DATOS).
pub fn carpeta_del_usuario() -> PathBuf {
    if let Some(carpeta) = std::env::var_os(VARIABLE_DE_CARPETA).filter(|carpeta| !carpeta.is_empty()) {
        return PathBuf::from(carpeta);
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".mcp-raptor-player")
}

/// Mueve un archivo de datos de una version anterior a su lugar nuevo, si
/// todavia no hay uno ahi.
fn mudar_si_falta(anterior: &Path, nuevo: &Path) {
    if !anterior.is_file() || nuevo.exists() {
        return;
    }
    let mudado = nuevo.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::rename(anterior, nuevo));
    match mudado {
        Ok(()) => log::info!("{} se movio a {}", anterior.display(), nuevo.display()),
        Err(error) => log::warn!("No se pudo mover {} a {}: {error}", anterior.display(), nuevo.display()),
    }
}

/// Escribe en un temporal y lo reemplaza, para no dejar un JSON a medio
/// escribir si la aplicacion se cierra en el medio.
fn escribir_json(archivo: &Path, contenido: &Value) -> std::io::Result<()> {
    if let Some(carpeta) = archivo.parent() {
        fs::create_dir_all(carpeta)?;
    }
    let mut temporal = archivo.as_os_str().to_owned();
    temporal.push(".tmp");
    let texto = serde_json::to_string_pretty(contenido).map_err(std::io::Error::other)?;
    fs::write(&temporal, texto)?;
    fs::rename(&temporal, archivo)
}

fn agregar_sufijo(archivo: &Path, sufijo: &str) -> PathBuf {
    let mut ruta = archivo.as_os_str().to_owned();
    ruta.push(sufijo);
    PathBuf::from(ruta)
}

/// Lo que se recuerda de cada archivo de audio: sus puntos de mezcla, su
/// tipo, las etiquetas leidas del archivo (o corregidas por el usuario), la duracion,
/// el ranking (0 a 5), el BPM fijado a mano y el momento de la ultima
/// reproduccion.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FichaDeCancion {
    pub puntos: PuntosDeMezcla,
    /// Comercial, Efecto, etc.: decide como se trata. None: sin clasificar
    /// (se trata como un Tema).
    pub tipo: Option<TipoDeElemento>,
    pub titulo: Option<String>,
    pub artista: Option<String>,
    pub genero: Option<String>,
    pub album: Option<String>,
    pub anio: Option<u16>,
    /// Nota u observacion libre del usuario sobre el tema o elemento; se ve
    /// en el tooltip de la lista y de los botones de efectos.
    pub nota: Option<String>,
    /// Efecto que suena solo cada vez que sale el tema, dentro de su ventana
    /// de overlay (o enseguida si no tiene).
    pub efecto_fijado: Option<PathBuf>,
    pub duracion: Option<f64>,
    pub ranking: u8,
    /// Tempo fijado por el usuario; sin el, vale el que estima el analisis.
    pub bpm: Option<f64>,
    /// Fecha y hora UTC en formato ISO 8601, por ejemplo 2026-10-22T19:40:12Z.
    pub ultima_reproduccion: Option<String>,
    /// Overlay de versiones anteriores (segundos restantes) que todavia no se
    /// pudo convertir a ventana porque falta la duracion del archivo.
    overlay_por_convertir: Option<f64>,
    /// Ficha de una version anterior, que no distinguia los puntos manuales
    /// de los sugeridos; se decide al conocer el analisis del tema.
    pub origen_por_resolver: bool,
    /// Datos descriptivos que el usuario corrigio a mano o eligio de
    /// internet: al releer las etiquetas no se proponen pisar.
    pub corregidos: CamposCorregidos,
}

impl FichaDeCancion {
    fn esta_vacia(&self) -> bool {
        *self == Self::default()
    }

    fn a_json(&self) -> Value {
        let mut valores = Map::new();
        valores.insert("intro".into(), json!(self.puntos.intro()));
        valores.insert("outro".into(), json!(self.puntos.outro()));
        valores.insert("recorteInicial".into(), json!(self.puntos.recorte_inicial()));
        valores.insert("recorteFinal".into(), json!(self.puntos.recorte_final()));
        if !self.origen_por_resolver {
            valores.insert("puntosManuales".into(), json!(self.puntos.manuales()));
        }
        valores.insert("overlayManual".into(), json!(self.puntos.overlay_manual()));
        // Siempre se escribe ("": sin clasificar, a proposito): una ficha sin
        // "tipo" es de una version anterior y se clasifica por su genero al
        // leerla; con null (sin clasificar por defecto, de versiones
        // anteriores) pasa a ser un Tema.
        valores.insert("tipo".into(), json!(self.tipo.map_or("", TipoDeElemento::nombre)));
        if let Some(ventana) = self.puntos.overlay() {
            valores.insert("overlayInicio".into(), json!(ventana.inicio()));
            valores.insert("overlayFin".into(), json!(ventana.fin()));
        } else if let Some(restante) = self.overlay_por_convertir {
            valores.insert("overlay".into(), json!(restante));
        }
        for (campo, valor) in
            [("titulo", &self.titulo), ("artista", &self.artista), ("genero", &self.genero), ("album", &self.album), ("nota", &self.nota)]
        {
            if let Some(valor) = valor {
                valores.insert(campo.into(), json!(valor));
            }
        }
        if let Some(anio) = self.anio {
            valores.insert("anio".into(), json!(anio));
        }
        if let Some(efecto) = &self.efecto_fijado {
            valores.insert("efectoFijado".into(), json!(efecto.to_string_lossy()));
        }
        if let Some(duracion) = self.duracion {
            valores.insert("duracion".into(), json!(duracion));
        }
        valores.insert("ranking".into(), json!(self.ranking));
        if let Some(bpm) = self.bpm {
            valores.insert("bpm".into(), json!(bpm));
        }
        if let Some(momento) = &self.ultima_reproduccion {
            valores.insert("ultimaReproduccion".into(), json!(momento));
        }
        // Siempre se escribe (aunque este vacio): sin la clave, la ficha es de
        // una version anterior que no registraba las correcciones. Si en una
        // de esas ya se registro alguna, se marca que el registro es parcial.
        let claves = self.corregidos.claves();
        if !self.corregidos.es_anterior() || !claves.is_empty() {
            valores.insert("corregidos".into(), json!(claves));
        }
        if self.corregidos.es_anterior() && !claves.is_empty() {
            valores.insert("registroParcial".into(), json!(true));
        }
        Value::Object(valores)
    }

    /// Lee una ficha y dice si su tipo vino de una version anterior y se
    /// migro (hay que reescribir el archivo).
    fn desde_json(audio: &str, valores: &Value) -> Result<(Self, Migracion), String> {
        let numero = |campo: &str| valores.get(campo).and_then(Value::as_f64);
        let texto = |campo: &str| valores.get(campo).and_then(Value::as_str).map(str::to_string);
        let intro = numero("intro").ok_or_else(|| tr!("falta la intro de {audio}", audio = audio))?;
        let outro = numero("outro").ok_or_else(|| tr!("falta la outro de {audio}", audio = audio))?;
        let ventana = match (numero("overlayInicio"), numero("overlayFin")) {
            (Some(inicio), Some(fin)) => Some(VentanaDeOverlay::nueva(inicio, fin)?),
            _ => None,
        };
        let manuales = valores.get("puntosManuales").and_then(Value::as_bool);
        // Antes no se distinguia: un overlay guardado lo habia puesto el usuario.
        let overlay_manual = valores.get("overlayManual").and_then(Value::as_bool).unwrap_or(ventana.is_some());
        let puntos = PuntosDeMezcla::nuevos(intro, outro, ventana)?
            .con_recortes(numero("recorteInicial").unwrap_or(0.0), numero("recorteFinal").unwrap_or(0.0))?
            .como_manuales(manuales.unwrap_or(false))
            .con_overlay_manual(overlay_manual);
        let genero = texto("genero");
        let es_marcador = Path::new(audio).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("marca"));
        let (tipo, migrada) = match valores.get("tipo") {
            // Sin clasificar por defecto (versiones anteriores, que dejaban
            // asi lo que no traia genero): suena como un Tema y lo es. Un
            // marcador no es un tema.
            Some(Value::Null) if !es_marcador => (Some(TipoDeElemento::Tema), Migracion::SinClasificarATema),
            Some(tipo) => (tipo.as_str().and_then(TipoDeElemento::desde_nombre), Migracion::Ninguna),
            None => (TipoDeElemento::deducido_del_genero(genero.as_deref()), Migracion::TipoDelGenero),
        };
        let mut ficha = Self {
            puntos,
            origen_por_resolver: manuales.is_none(),
            tipo,
            titulo: texto("titulo"),
            artista: texto("artista"),
            genero,
            album: texto("album"),
            nota: texto("nota"),
            anio: numero("anio").map(|anio| anio as u16).filter(|anio| anio_valido(*anio)),
            efecto_fijado: texto("efectoFijado").map(PathBuf::from),
            duracion: numero("duracion"),
            ranking: numero("ranking").map_or(0, |ranking| ranking.clamp(0.0, RANKING_MAXIMO as f64) as u8),
            bpm: numero("bpm").filter(|bpm| (BPM_MINIMO..=BPM_MAXIMO).contains(bpm)),
            ultima_reproduccion: texto("ultimaReproduccion"),
            overlay_por_convertir: None,
            corregidos: match valores.get("corregidos").and_then(Value::as_array) {
                Some(claves) => {
                    let corregidos = CamposCorregidos::desde_claves(claves.iter().filter_map(Value::as_str));
                    if valores.get("registroParcial").and_then(Value::as_bool).unwrap_or(false) {
                        corregidos.como_anterior()
                    } else {
                        corregidos
                    }
                }
                None => CamposCorregidos::sin_registro(),
            },
        };
        if ventana.is_none() {
            ficha.overlay_por_convertir = numero("overlay").filter(|restante| *restante > 0.0);
            ficha.convertir_overlay_anterior();
        }
        Ok((ficha, migrada))
    }

    /// Titulo, artista, album, anio y genero de la ficha.
    pub fn datos_descriptivos(&self) -> DatosDescriptivos {
        DatosDescriptivos {
            titulo: self.titulo.clone(),
            artista: self.artista.clone(),
            album: self.album.clone(),
            anio: self.anio,
            genero: self.genero.clone(),
        }
    }

    /// Pasa el overlay de versiones anteriores a ventana en cuanto se conoce
    /// la duracion: X segundos restantes equivalen a [duracion - X, duracion].
    fn convertir_overlay_anterior(&mut self) {
        let (Some(restante), Some(duracion)) = (self.overlay_por_convertir, self.duracion) else {
            return;
        };
        self.overlay_por_convertir = None;
        if self.puntos.overlay().is_none() {
            let ventana = VentanaDeOverlay::desde_restante(restante, duracion);
            self.puntos = self.puntos.con_overlay(ventana).con_overlay_manual(ventana.is_some());
        }
    }
}

/// Cambios de datos descriptivos pedidos por el usuario; None deja el valor
/// como estaba y un texto vacio lo borra.
#[derive(Default, PartialEq)]
pub struct CambiosDeDatos {
    /// Some(None) deja el elemento sin clasificar.
    pub tipo: Option<Option<TipoDeElemento>>,
    pub titulo: Option<String>,
    pub artista: Option<String>,
    pub genero: Option<String>,
    pub album: Option<String>,
    /// Texto vacio borra la nota.
    pub nota: Option<String>,
    /// Some(None) borra el anio.
    pub anio: Option<Option<u16>>,
    /// Some(None) quita el efecto fijado.
    pub efecto_fijado: Option<Option<PathBuf>>,
    pub ranking: Option<u8>,
    /// Some(None) borra el BPM manual (vuelve al del analisis).
    pub bpm: Option<Option<f64>>,
    /// Momento en que empezo a sonar por ultima vez, en ISO 8601 UTC
    /// ("2026-10-22T19:40:12Z"); Some(None) lo deja como si nunca hubiera sonado.
    pub ultima_reproduccion: Option<Option<String>>,
}

/// Guarda una ficha por archivo de audio, para que al volver a agregar un
/// tema se recuperen sus puntos de mezcla y sus datos.
///
/// Formato: { "C:\\musica\\tema.mp3": { "intro": 6.0, "outro": 10.0, "recorteInicial": 0.4, "recorteFinal": 1.2,
///   "puntosManuales": false, "overlayInicio": 80.0, "overlayFin": 105.0, "overlayManual": true,
///   "tipo": "Tema", "titulo": "Tema", "artista": "Banda", "genero": "Rock", "duracion": 215.3,
///   "ranking": 4, "bpm": 124.0, "ultimaReproduccion": "2026-10-22T19:40:12Z" }, ... }
/// Solo intro y outro son obligatorios al leer. El "overlay" de versiones
/// anteriores (segundos restantes) se convierte a ventana al conocer la
/// duracion. Sin "puntosManuales" (versiones anteriores), si son manuales se
/// decide al conocer el analisis del tema. Sin "tipo" (versiones
/// anteriores, que usaban el genero para eso) el tipo se deduce del genero
/// una sola vez; antes de reescribir el archivo se deja una copia con el
/// sufijo `.antes-del-tipo`. El genero no se toca. "tipo": "" es sin
/// clasificar (elegido asi); null, de versiones que dejaban sin clasificar
/// lo que no traia genero, pasa a Tema una sola vez (copia `.antes-del-tema`).
///
/// Junto a ese archivo, `reubicaciones.json` recuerda los archivos que se
/// reubicaron en el mantenimiento del catalogo ({ "ruta anterior": "ruta
/// nueva" }), para que las listas guardadas con la ruta anterior se carguen
/// con la nueva.
pub struct RepositorioDeCanciones {
    archivo: PathBuf,
    fichas: HashMap<PathBuf, FichaDeCancion>,
    reubicaciones: BTreeMap<PathBuf, PathBuf>,
}

// Saltos que se siguen como maximo al traducir una ruta reubicada (un archivo
// movido varias veces); evita un ciclo si el registro quedara mal.
const SALTOS_DE_REUBICACION: usize = 16;

impl RepositorioDeCanciones {
    pub fn new(archivo: PathBuf) -> Self {
        let mut repositorio = Self { archivo, fichas: HashMap::new(), reubicaciones: BTreeMap::new() };
        repositorio.cargar();
        repositorio.reubicaciones = leer_reubicaciones(&repositorio.archivo_de_reubicaciones());
        repositorio
    }

    fn archivo_de_reubicaciones(&self) -> PathBuf {
        self.archivo.with_file_name("reubicaciones.json")
    }

    /// Cantidad de fichas del catalogo.
    pub fn cantidad(&self) -> usize {
        self.fichas.len()
    }

    /// Archivos del catalogo.
    pub fn archivos(&self) -> impl Iterator<Item = &Path> {
        self.fichas.keys().map(PathBuf::as_path)
    }

    /// Cada archivo del catalogo con su ficha, sin orden.
    pub fn fichas(&self) -> impl Iterator<Item = (&Path, &FichaDeCancion)> {
        self.fichas.iter().map(|(audio, ficha)| (audio.as_path(), ficha))
    }

    /// Fichas cuyo archivo ya no existe (se movio, se renombro o se borro),
    /// ordenadas por ruta.
    pub fn sin_archivo(&self) -> Vec<(PathBuf, FichaDeCancion)> {
        let mut faltantes: Vec<(PathBuf, FichaDeCancion)> = self
            .fichas
            .iter()
            .filter(|(audio, _)| !audio.is_file())
            .map(|(audio, ficha)| (audio.clone(), ficha.clone()))
            .collect();
        faltantes.sort_by(|(una, _), (otra, _)| una.cmp(otra));
        faltantes
    }

    /// Ruta con que hay que abrir un archivo: la misma si existe o, si no
    /// existe y se reubico en el mantenimiento, la nueva.
    pub fn ruta_actual(&self, ruta: &Path) -> PathBuf {
        if ruta.is_file() || self.reubicaciones.is_empty() {
            return ruta.to_path_buf();
        }
        let mut actual = normalizar(ruta);
        for _ in 0..SALTOS_DE_REUBICACION {
            match self.reubicaciones.get(&actual) {
                Some(nueva) => actual = nueva.clone(),
                None => break,
            }
        }
        actual
    }

    /// Pasa la ficha de cada ruta anterior a su ruta nueva sin perder datos,
    /// actualiza los efectos fijados que apuntaban a la anterior y recuerda
    /// el cambio. Si la ruta nueva ya tenia ficha (por haberse agregado desde
    /// su nueva ubicacion), lo que el usuario fijo en la anterior manda y la
    /// nueva aporta lo que falte. Devuelve cuantas fichas se reubicaron.
    pub fn reubicar(&mut self, cambios: &[(PathBuf, PathBuf)]) -> usize {
        let mut reubicadas = 0;
        for (anterior, nueva) in cambios {
            let (anterior, nueva) = (normalizar(anterior), normalizar(nueva));
            if anterior == nueva {
                continue;
            }
            let Some(ficha) = self.fichas.remove(&anterior) else {
                continue;
            };
            let combinada = match self.fichas.remove(&nueva) {
                Some(existente) => combinar_fichas(ficha, existente),
                None => ficha,
            };
            self.fichas.insert(nueva.clone(), combinada);
            for ficha in self.fichas.values_mut() {
                if ficha.efecto_fijado.as_deref().is_some_and(|efecto| misma_ruta(efecto, &anterior)) {
                    ficha.efecto_fijado = Some(nueva.clone());
                }
            }
            // Una reubicacion anterior que llevaba a `anterior` ahora lleva a `nueva`.
            for destino in self.reubicaciones.values_mut() {
                if misma_ruta(destino, &anterior) {
                    *destino = nueva.clone();
                }
            }
            self.reubicaciones.remove(&nueva);
            self.reubicaciones.insert(anterior, nueva);
            reubicadas += 1;
        }
        if reubicadas > 0 {
            self.escribir();
            self.escribir_reubicaciones();
        }
        reubicadas
    }

    /// Borra las fichas de estos archivos (por ejemplo, las de archivos que
    /// ya no existen) con una sola escritura. Devuelve cuantas se borraron.
    pub fn olvidar_varios(&mut self, archivos: &[PathBuf]) -> usize {
        let borradas = archivos.iter().filter(|audio| self.fichas.remove(&normalizar(audio)).is_some()).count();
        if borradas > 0 {
            self.escribir();
        }
        borradas
    }

    fn escribir_reubicaciones(&self) {
        let contenido: Map<String, Value> = self
            .reubicaciones
            .iter()
            .map(|(anterior, nueva)| (anterior.to_string_lossy().into_owned(), Value::String(nueva.to_string_lossy().into_owned())))
            .collect();
        let archivo = self.archivo_de_reubicaciones();
        if let Err(error) = escribir_json(&archivo, &Value::Object(contenido)) {
            log::warn!("No se pudieron guardar las reubicaciones en {}: {error}", archivo.display());
        }
    }

    /// Repositorio en ~/.mcp-raptor-player/mezcla.json.
    pub fn en_carpeta_del_usuario() -> Self {
        Self::new(carpeta_del_usuario().join("mezcla.json"))
    }

    pub fn buscar(&self, audio: &Path) -> Option<&FichaDeCancion> {
        // Las pistas de la lista ya tienen la ruta absoluta y limpia: se
        // buscan tal cual, sin normalizar (se consulta varias veces por ciclo).
        if ruta_ya_normalizada(audio) {
            return self.fichas.get(audio);
        }
        self.fichas.get(&normalizar(audio))
    }

    /// Tipo del archivo; None si no tiene ficha o esta sin clasificar.
    pub fn tipo_de(&self, audio: &Path) -> Option<TipoDeElemento> {
        self.buscar(audio).and_then(|ficha| ficha.tipo)
    }

    pub fn puntos(&self, audio: &Path) -> Option<PuntosDeMezcla> {
        self.buscar(audio).map(|ficha| ficha.puntos)
    }

    pub fn guardar_puntos(&mut self, audio: &Path, puntos: PuntosDeMezcla) {
        self.modificar(audio, |ficha| {
            ficha.puntos = puntos;
            ficha.overlay_por_convertir = None;
            ficha.origen_por_resolver = false;
        });
    }

    /// Completa las fichas con lo leido de cada archivo, con una sola
    /// escritura del JSON. La duracion se actualiza siempre; las etiquetas
    /// solo si no habia un valor, para no pisar lo que corrigio el usuario.
    pub fn completar_con_analisis(&mut self, resultados: &[(PathBuf, Analisis)]) {
        let mut hubo_cambios = false;
        for (audio, analisis) in resultados {
            hubo_cambios |= self.aplicar(audio, |ficha| {
                if analisis.duracion.is_some() {
                    ficha.duracion = analisis.duracion;
                    ficha.convertir_overlay_anterior();
                }
                // Un archivo nuevo entra clasificado: con el tipo que nombra
                // el genero de sus etiquetas ("Comercial") o, si no, Tema.
                if ficha.tipo.is_none() && ficha.genero.is_none() {
                    ficha.tipo = TipoDeElemento::deducido_del_genero(analisis.genero.as_deref());
                }
                for (destino, leido) in [
                    (&mut ficha.titulo, &analisis.titulo),
                    (&mut ficha.artista, &analisis.artista),
                    (&mut ficha.genero, &analisis.genero),
                    (&mut ficha.album, &analisis.album),
                ] {
                    if destino.is_none() {
                        destino.clone_from(leido);
                    }
                }
                if ficha.anio.is_none() {
                    ficha.anio = analisis.anio;
                }
            });
        }
        if hubo_cambios {
            self.escribir();
        }
    }

    /// Borra la ficha del archivo (descatalogar). Devuelve si habia una.
    pub fn olvidar(&mut self, audio: &Path) -> bool {
        let habia = self.fichas.remove(&normalizar(audio)).is_some();
        if habia {
            self.escribir();
        }
        habia
    }

    pub fn registrar_reproduccion(&mut self, audio: &Path, momento: String) {
        self.modificar(audio, |ficha| ficha.ultima_reproduccion = Some(momento));
    }

    pub fn cambiar_datos(&mut self, audio: &Path, cambios: CambiosDeDatos) -> FichaDeCancion {
        let limpiar = |texto: String| Some(texto.trim().to_string()).filter(|texto| !texto.is_empty());
        self.modificar(audio, |ficha| {
            let antes = ficha.datos_descriptivos();
            if let Some(tipo) = cambios.tipo {
                ficha.tipo = tipo;
            }
            if let Some(titulo) = cambios.titulo {
                ficha.titulo = limpiar(titulo);
            }
            if let Some(artista) = cambios.artista {
                ficha.artista = limpiar(artista);
            }
            if let Some(genero) = cambios.genero {
                ficha.genero = limpiar(genero);
            }
            if let Some(album) = cambios.album {
                ficha.album = limpiar(album);
            }
            if let Some(nota) = cambios.nota {
                ficha.nota = limpiar(nota);
            }
            if let Some(anio) = cambios.anio {
                ficha.anio = anio.filter(|anio| anio_valido(*anio));
            }
            if let Some(efecto) = cambios.efecto_fijado {
                ficha.efecto_fijado = efecto;
            }
            if let Some(ranking) = cambios.ranking {
                ficha.ranking = ranking.min(RANKING_MAXIMO);
            }
            if let Some(bpm) = cambios.bpm {
                ficha.bpm = bpm.map(|bpm| (bpm.clamp(BPM_MINIMO, BPM_MAXIMO) * 10.0).round() / 10.0);
            }
            if let Some(momento) = cambios.ultima_reproduccion {
                ficha.ultima_reproduccion = momento;
            }
            // Lo que el usuario cambio queda como corregido a mano.
            let despues = ficha.datos_descriptivos();
            for campo in CampoDescriptivo::TODOS {
                if antes.valor(campo) != despues.valor(campo) {
                    ficha.corregidos.marcar(campo);
                }
            }
        });
        self.buscar(audio).cloned().unwrap_or_default()
    }

    /// Aplica los cambios elegidos de una fuente, con una sola escritura.
    /// Lo que viene de las etiquetas deja de contar como corregido; lo que
    /// viene de internet pasa a contar como corregido. Devuelve cuantos
    /// datos cambiaron.
    pub fn aplicar_propuestas(&mut self, propuestas: &[Propuesta], origen: OrigenDeLosDatos) -> usize {
        let mut cambiados = 0;
        for propuesta in propuestas.iter().filter(|propuesta| propuesta.elegida) {
            let cambio = self.aplicar(&propuesta.archivo, |ficha| {
                let nuevo = propuesta.nuevo.clone();
                match propuesta.campo {
                    CampoDescriptivo::Titulo => ficha.titulo = Some(nuevo),
                    CampoDescriptivo::Artista => ficha.artista = Some(nuevo),
                    CampoDescriptivo::Album => ficha.album = Some(nuevo),
                    CampoDescriptivo::Genero => ficha.genero = Some(nuevo),
                    CampoDescriptivo::Anio => {
                        if let Some(anio) = nuevo.parse::<u16>().ok().filter(|anio| anio_valido(*anio)) {
                            ficha.anio = Some(anio);
                        }
                    }
                }
                match origen {
                    OrigenDeLosDatos::Etiquetas => ficha.corregidos.desmarcar(propuesta.campo),
                    OrigenDeLosDatos::Internet => ficha.corregidos.marcar(propuesta.campo),
                }
            });
            cambiados += usize::from(cambio);
        }
        if cambiados > 0 {
            self.escribir();
        }
        cambiados
    }

    /// Aplica un cambio a la ficha del archivo y reescribe el JSON solo si
    /// algo cambio.
    fn modificar(&mut self, audio: &Path, cambio: impl FnOnce(&mut FichaDeCancion)) {
        if self.aplicar(audio, cambio) {
            self.escribir();
        }
    }

    /// Aplica un cambio en memoria y devuelve si algo cambio. Una ficha que
    /// queda vacia se borra.
    fn aplicar(&mut self, audio: &Path, cambio: impl FnOnce(&mut FichaDeCancion)) -> bool {
        let clave = normalizar(audio);
        let anterior = self.fichas.get(&clave).cloned().unwrap_or_default();
        let mut ficha = anterior.clone();
        cambio(&mut ficha);
        if ficha == anterior {
            return false;
        }
        if ficha.esta_vacia() {
            self.fichas.remove(&clave);
        } else {
            self.fichas.insert(clave, ficha);
        }
        true
    }

    fn cargar(&mut self) {
        if !self.archivo.is_file() {
            return;
        }
        match leer_fichas(&self.archivo) {
            Ok(leidas) => {
                self.fichas = leidas.fichas;
                if leidas.del_genero > 0 {
                    self.guardar_migracion_del_tipo(leidas.del_genero);
                } else if leidas.a_tema > 0 {
                    self.guardar_migracion_a_tema(leidas.a_tema);
                }
            }
            Err(error) => {
                // Un archivo danado no debe impedir usar el reproductor; se
                // respalda para no perderlo al guardar de nuevo.
                log::warn!("No se pudo leer {}: {error}; se respalda y se empieza vacio", self.archivo.display());
                if let Err(error) = fs::rename(&self.archivo, agregar_sufijo(&self.archivo, ".danado")) {
                    log::warn!("No se pudo respaldar {}: {error}", self.archivo.display());
                }
            }
        }
    }

    /// Respalda el archivo de una version anterior (sin tipos) y lo reescribe
    /// con el tipo que se dedujo del genero de cada ficha.
    fn guardar_migracion_del_tipo(&self, migradas: usize) {
        let respaldo = agregar_sufijo(&self.archivo, ".antes-del-tipo");
        if !respaldo.exists() {
            if let Err(error) = fs::copy(&self.archivo, &respaldo) {
                log::warn!("No se pudo respaldar {}: {error}; no se migra el tipo", self.archivo.display());
                return;
            }
        }
        let clasificadas = self.fichas.values().filter(|ficha| ficha.tipo.is_some()).count();
        log::info!(
            "Catalogo: {migradas} ficha(s) de una version anterior; {clasificadas} quedaron con tipo segun su genero (copia en {})",
            respaldo.display()
        );
        self.escribir();
    }

    /// Respalda el catalogo y lo reescribe con los elementos que estaban sin
    /// clasificar por defecto convertidos en Tema (una sola vez: desde ahora
    /// "sin clasificar" se guarda distinto y solo si se elige).
    fn guardar_migracion_a_tema(&self, migradas: usize) {
        let respaldo = agregar_sufijo(&self.archivo, ".antes-del-tema");
        if !respaldo.exists()
            && let Err(error) = fs::copy(&self.archivo, &respaldo)
        {
            log::warn!("No se pudo respaldar {}: {error}; no se pasan a Tema los elementos sin clasificar", self.archivo.display());
            return;
        }
        log::info!("Catalogo: {migradas} elemento(s) sin clasificar pasaron a Tema (copia en {})", respaldo.display());
        self.escribir();
    }

    fn escribir(&self) {
        let contenido: BTreeMap<String, Value> = self
            .fichas
            .iter()
            .map(|(audio, ficha)| (audio.to_string_lossy().into_owned(), ficha.a_json()))
            .collect();
        let contenido = Value::Object(contenido.into_iter().collect::<Map<_, _>>());
        if let Err(error) = escribir_json(&self.archivo, &contenido) {
            log::warn!("No se pudieron guardar las canciones en {}: {error}", self.archivo.display());
        }
    }
}

/// Une la ficha de un archivo reubicado con la que ya tenia su ruta nueva:
/// manda lo que se fijo en la anterior y la nueva completa lo que falte.
fn combinar_fichas(anterior: FichaDeCancion, nueva: FichaDeCancion) -> FichaDeCancion {
    let usar_puntos_nuevos = !anterior.puntos.manuales() && !anterior.puntos.overlay_manual() && nueva.puntos != PuntosDeMezcla::default();
    FichaDeCancion {
        puntos: if usar_puntos_nuevos { nueva.puntos } else { anterior.puntos },
        tipo: anterior.tipo.or(nueva.tipo),
        titulo: anterior.titulo.or(nueva.titulo),
        artista: anterior.artista.or(nueva.artista),
        genero: anterior.genero.or(nueva.genero),
        album: anterior.album.or(nueva.album),
        nota: anterior.nota.or(nueva.nota),
        anio: anterior.anio.or(nueva.anio),
        efecto_fijado: anterior.efecto_fijado.or(nueva.efecto_fijado),
        duracion: nueva.duracion.or(anterior.duracion),
        ranking: if anterior.ranking > 0 { anterior.ranking } else { nueva.ranking },
        bpm: anterior.bpm.or(nueva.bpm),
        // Las dos son ISO 8601 en UTC: la mayor es la mas reciente.
        ultima_reproduccion: anterior.ultima_reproduccion.max(nueva.ultima_reproduccion),
        overlay_por_convertir: anterior.overlay_por_convertir.or(nueva.overlay_por_convertir),
        origen_por_resolver: anterior.origen_por_resolver && !usar_puntos_nuevos,
        corregidos: anterior.corregidos,
    }
}

/// Lee el registro de reubicaciones; si no existe o esta danado, vacio.
fn leer_reubicaciones(archivo: &Path) -> BTreeMap<PathBuf, PathBuf> {
    let Ok(texto) = fs::read_to_string(archivo) else {
        return BTreeMap::new();
    };
    let Ok(Value::Object(entradas)) = serde_json::from_str::<Value>(&texto) else {
        log::warn!("No se pudo leer {}; se ignora", archivo.display());
        return BTreeMap::new();
    };
    entradas
        .iter()
        .filter_map(|(anterior, nueva)| Some((normalizar(Path::new(anterior)), normalizar(Path::new(nueva.as_str()?)))))
        .collect()
}

/// Migracion del tipo de una ficha al leerla.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Migracion {
    Ninguna,
    /// Sin "tipo" (versiones que usaban el genero): se dedujo del genero.
    TipoDelGenero,
    /// Sin clasificar por defecto (null): paso a ser un Tema.
    SinClasificarATema,
}

/// Las fichas del archivo y cuantas se migraron de cada forma.
struct FichasLeidas {
    fichas: HashMap<PathBuf, FichaDeCancion>,
    del_genero: usize,
    a_tema: usize,
}

fn leer_fichas(archivo: &Path) -> Result<FichasLeidas, String> {
    let texto = fs::read_to_string(archivo).map_err(|error| error.to_string())?;
    let raiz: Value = serde_json::from_str(&texto).map_err(|error| error.to_string())?;
    let entradas = raiz.as_object().ok_or_else(|| tr!("se esperaba un objeto JSON"))?;
    let mut leidas = FichasLeidas { fichas: HashMap::new(), del_genero: 0, a_tema: 0 };
    for (audio, valores) in entradas {
        let (ficha, migracion) = FichaDeCancion::desde_json(audio, valores)?;
        match migracion {
            Migracion::Ninguna => {}
            Migracion::TipoDelGenero => leidas.del_genero += 1,
            Migracion::SinClasificarATema => leidas.a_tema += 1,
        }
        leidas.fichas.insert(normalizar(Path::new(audio)), ficha);
    }
    Ok(leidas)
}

/// Fecha y hora actual en UTC, en formato ISO 8601.
pub fn momento_actual() -> String {
    let segundos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |transcurrido| transcurrido.as_secs() as i64);
    formatear_iso8601(segundos)
}

/// Convierte segundos desde 1970 a fecha civil (algoritmo de Howard Hinnant).
fn formatear_iso8601(segundos: i64) -> String {
    let (dias, resto) = (segundos.div_euclid(86_400), segundos.rem_euclid(86_400));
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let dia_de_era = z - era * 146_097;
    let anio_de_era = (dia_de_era - dia_de_era / 1_460 + dia_de_era / 36_524 - dia_de_era / 146_096) / 365;
    let dia_del_anio = dia_de_era - (365 * anio_de_era + anio_de_era / 4 - anio_de_era / 100);
    let mes_desde_marzo = (5 * dia_del_anio + 2) / 153;
    let dia = dia_del_anio - (153 * mes_desde_marzo + 2) / 5 + 1;
    let mes = if mes_desde_marzo < 10 { mes_desde_marzo + 3 } else { mes_desde_marzo - 9 };
    let anio = anio_de_era + era * 400 + i64::from(mes <= 2);
    format!("{anio:04}-{mes:02}-{dia:02}T{:02}:{:02}:{:02}Z", resto / 3_600, resto % 3_600 / 60, resto % 60)
}

/// Preferencias del reproductor que no pertenecen a una cancion ni a la
/// botonera.
///
/// Formato: { "fader": 5, "crossfadeInteligente": false, "nivelarVolumen": false,
/// "objetivoLufs": -11, "desplazar": false, "ultimaLista": "C:\\listas\\tarde.json",
/// "ultimoTema": { "posicion": 3, "archivo": "C:\\musica\\a.mp3" },
/// "ventanaDelEditor": { "x": 300, "y": 120, "ancho": 1180, "alto": 640 },
/// "editorCentrado": false, "caratulasDeInternet": false,
/// "salidaDePreescucha": { "id": "wasapi:{...}", "nombre": "Auriculares" },
/// "volumenDePreescucha": 0.8, "carpetaDeLocuciones": "D:\locuciones",
/// "localidadDelClima": { "nombre": "Buenos Aires", "latitud": -34.6037, "longitud": -58.3816 },
/// "idioma": "es", "formatoDeFecha": "dma" }
pub struct RepositorioDePreferencias {
    archivo: PathBuf,
    fader: u8,
    crossfade_inteligente: bool,
    nivelar_volumen: bool,
    objetivo_de_sonoridad: ObjetivoDeSonoridad,
    desplazar: bool,
    // Modo AL AIRE: protege la secuencia que sale al aire.
    al_aire: bool,
    ultima_lista: Option<PathBuf>,
    ultimo_tema: Option<UltimoTema>,
    ventana_del_editor: Option<GeometriaDeVentana>,
    editor_centrado: bool,
    ocultar_textos: bool,
    ajustes_de_efectos_visibles: bool,
    controles_visibles: bool,
    ver_onda: bool,
    caratulas_de_internet: bool,
    salida_de_preescucha: Option<SalidaDeAudio>,
    volumen_de_preescucha: f64,
    carpeta_de_locuciones: Option<PathBuf>,
    localidad_del_clima: Localidad,
    // None: todavia no se eligio; se usa el del sistema.
    idioma: Option<Idioma>,
    formato_de_fecha: FormatoDeFecha,
}

/// Volumen de la preescucha si no se eligio otro: el pleno.
pub const VOLUMEN_DE_PREESCUCHA_POR_DEFECTO: f64 = 1.0;

/// Posicion y tamano de una ventana, en pixeles fisicos de la pantalla.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometriaDeVentana {
    pub x: i32,
    pub y: i32,
    pub ancho: u32,
    pub alto: u32,
}

/// Tema que sonaba (o estaba marcado como actual) al cerrar: la posicion
/// distingue las copias de un mismo comercial y el archivo confirma que la
/// lista no cambio.
#[derive(Clone, Debug, PartialEq)]
pub struct UltimoTema {
    pub posicion: usize,
    pub archivo: PathBuf,
}

impl RepositorioDePreferencias {
    pub fn new(archivo: PathBuf) -> Self {
        let raiz = fs::read_to_string(&archivo).ok().and_then(|texto| serde_json::from_str::<Value>(&texto).ok());
        let valor = |clave: &str| raiz.as_ref().and_then(|raiz| raiz.get(clave));
        let fader = valor("fader")
            .and_then(Value::as_u64)
            .map_or(FADER_POR_DEFECTO, |fader| fader.min(FADER_MAXIMO as u64) as u8);
        let crossfade_inteligente = valor("crossfadeInteligente").and_then(Value::as_bool).unwrap_or(false);
        let nivelar_volumen = valor("nivelarVolumen").and_then(Value::as_bool).unwrap_or(false);
        let objetivo_de_sonoridad =
            valor("objetivoLufs").and_then(Value::as_f64).and_then(ObjetivoDeSonoridad::de_lufs).unwrap_or_default();
        let desplazar = valor("desplazar").and_then(Value::as_bool).unwrap_or(false);
        let al_aire = valor("alAire").and_then(Value::as_bool).unwrap_or(false);
        let ultima_lista = valor("ultimaLista").and_then(Value::as_str).map(PathBuf::from);
        let ultimo_tema = valor("ultimoTema").and_then(|tema| {
            Some(UltimoTema {
                posicion: tema.get("posicion")?.as_u64()? as usize,
                archivo: PathBuf::from(tema.get("archivo")?.as_str()?),
            })
        });
        let ventana_del_editor = valor("ventanaDelEditor").and_then(|geometria| {
            let entero = |clave: &str| geometria.get(clave)?.as_i64();
            let geometria = GeometriaDeVentana {
                x: i32::try_from(entero("x")?).ok()?,
                y: i32::try_from(entero("y")?).ok()?,
                ancho: u32::try_from(entero("ancho")?).ok()?,
                alto: u32::try_from(entero("alto")?).ok()?,
            };
            (geometria.ancho > 0 && geometria.alto > 0).then_some(geometria)
        });
        let editor_centrado = valor("editorCentrado").and_then(Value::as_bool).unwrap_or(false);
        let ocultar_textos = valor("ocultarTextos").and_then(Value::as_bool).unwrap_or(false);
        let ver_onda = valor("verOnda").and_then(Value::as_bool).unwrap_or(false);
        let caratulas_de_internet = valor("caratulasDeInternet").and_then(Value::as_bool).unwrap_or(false);
        let controles_visibles = valor("controlesVisibles").and_then(Value::as_bool).unwrap_or(true);
        let ajustes_de_efectos_visibles = valor("ajustesDeEfectosVisibles").and_then(Value::as_bool).unwrap_or(false);
        let salida_de_preescucha = valor("salidaDePreescucha").and_then(|salida| {
            let texto = |clave: &str| salida.get(clave)?.as_str().map(str::to_string);
            Some(SalidaDeAudio { id: texto("id").filter(|id| !id.is_empty())?, nombre: texto("nombre").unwrap_or_default() })
        });
        let volumen_de_preescucha = valor("volumenDePreescucha")
            .and_then(Value::as_f64)
            .filter(|volumen| volumen.is_finite())
            .map_or(VOLUMEN_DE_PREESCUCHA_POR_DEFECTO, |volumen| volumen.clamp(0.0, 1.0));
        let carpeta_de_locuciones =
            valor("carpetaDeLocuciones").and_then(Value::as_str).filter(|carpeta| !carpeta.is_empty()).map(PathBuf::from);
        let localidad_del_clima = valor("localidadDelClima")
            .and_then(|localidad| {
                let numero = |clave: &str| localidad.get(clave)?.as_f64().filter(|valor| valor.is_finite());
                let (latitud, longitud) = (numero("latitud")?, numero("longitud")?);
                ((-90.0..=90.0).contains(&latitud) && (-180.0..=180.0).contains(&longitud)).then(|| Localidad {
                    nombre: localidad.get("nombre").and_then(Value::as_str).unwrap_or_default().to_string(),
                    latitud,
                    longitud,
                    zona_horaria: localidad.get("zonaHoraria").and_then(Value::as_str).map(str::to_string),
                })
            })
            .unwrap_or_default();
        let idioma = valor("idioma").and_then(Value::as_str).and_then(Idioma::desde_codigo);
        let formato_de_fecha =
            valor("formatoDeFecha").and_then(Value::as_str).and_then(FormatoDeFecha::desde_codigo).unwrap_or_default();
        Self {
            archivo,
            fader,
            crossfade_inteligente,
            nivelar_volumen,
            objetivo_de_sonoridad,
            desplazar,
            al_aire,
            ultima_lista,
            ultimo_tema,
            ventana_del_editor,
            editor_centrado,
            ocultar_textos,
            ajustes_de_efectos_visibles,
            controles_visibles,
            ver_onda,
            caratulas_de_internet,
            salida_de_preescucha,
            volumen_de_preescucha,
            carpeta_de_locuciones,
            localidad_del_clima,
            idioma,
            formato_de_fecha,
        }
    }

    /// Repositorio en ~/.mcp-raptor-player/preferencias.json.
    pub fn en_carpeta_del_usuario() -> Self {
        Self::new(carpeta_del_usuario().join("preferencias.json"))
    }

    /// Segundos de mezcla o fundido al usar Siguiente o Detener (0 a 15).
    pub fn fader(&self) -> u8 {
        self.fader
    }

    pub fn guardar_fader(&mut self, segundos: u8) {
        let segundos = segundos.min(FADER_MAXIMO);
        if segundos == self.fader {
            return;
        }
        self.fader = segundos;
        self.escribir();
    }

    /// Mezclar segun el analisis de los temas (silencios, fundidos y ritmo).
    pub fn crossfade_inteligente(&self) -> bool {
        self.crossfade_inteligente
    }

    pub fn guardar_crossfade_inteligente(&mut self, activo: bool) {
        if activo != self.crossfade_inteligente {
            self.crossfade_inteligente = activo;
            self.escribir();
        }
    }

    /// Llevar todos los temas a una misma sonoridad.
    pub fn nivelar_volumen(&self) -> bool {
        self.nivelar_volumen
    }

    pub fn guardar_nivelar_volumen(&mut self, activo: bool) {
        if activo != self.nivelar_volumen {
            self.nivelar_volumen = activo;
            self.escribir();
        }
    }

    /// Sonoridad a la que se llevan los temas al nivelar.
    pub fn objetivo_de_sonoridad(&self) -> ObjetivoDeSonoridad {
        self.objetivo_de_sonoridad
    }

    pub fn guardar_objetivo_de_sonoridad(&mut self, objetivo: ObjetivoDeSonoridad) {
        if objetivo != self.objetivo_de_sonoridad {
            self.objetivo_de_sonoridad = objetivo;
            self.escribir();
        }
    }

    /// Ultima lista con nombre que se cargo o guardo.
    pub fn ultima_lista(&self) -> Option<&Path> {
        self.ultima_lista.as_deref()
    }

    pub fn guardar_ultima_lista(&mut self, lista: Option<&Path>) {
        if lista != self.ultima_lista.as_deref() {
            self.ultima_lista = lista.map(Path::to_path_buf);
            self.escribir();
        }
    }

    pub fn ultimo_tema(&self) -> Option<&UltimoTema> {
        self.ultimo_tema.as_ref()
    }

    pub fn guardar_ultimo_tema(&mut self, tema: Option<UltimoTema>) {
        if tema != self.ultimo_tema {
            self.ultimo_tema = tema;
            self.escribir();
        }
    }

    /// Ultima posicion y tamano del editor de pistas.
    pub fn ventana_del_editor(&self) -> Option<GeometriaDeVentana> {
        self.ventana_del_editor
    }

    pub fn guardar_ventana_del_editor(&mut self, geometria: GeometriaDeVentana) {
        if Some(geometria) != self.ventana_del_editor {
            self.ventana_del_editor = Some(geometria);
            self.escribir();
        }
    }

    /// Salida por la que suena la preescucha del editor (None: la misma de
    /// la musica, con la musica detenida).
    pub fn salida_de_preescucha(&self) -> Option<&SalidaDeAudio> {
        self.salida_de_preescucha.as_ref()
    }

    pub fn guardar_salida_de_preescucha(&mut self, salida: Option<SalidaDeAudio>) {
        if salida != self.salida_de_preescucha {
            self.salida_de_preescucha = salida;
            self.escribir();
        }
    }

    /// Volumen de la preescucha (0 a 1), aparte del de la musica.
    pub fn volumen_de_preescucha(&self) -> f64 {
        self.volumen_de_preescucha
    }

    pub fn guardar_volumen_de_preescucha(&mut self, volumen: f64) {
        let volumen = (volumen.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        if (volumen - self.volumen_de_preescucha).abs() > f64::EPSILON {
            self.volumen_de_preescucha = volumen;
            self.escribir();
        }
    }

    /// Abrir el editor siempre centrado sobre la ventana principal, en vez
    /// de donde quedo la ultima vez.
    pub fn editor_centrado(&self) -> bool {
        self.editor_centrado
    }

    pub fn guardar_editor_centrado(&mut self, activo: bool) {
        if activo != self.editor_centrado {
            self.editor_centrado = activo;
            self.escribir();
        }
    }

    /// Idioma elegido para la interfaz; None si nunca se eligio.
    pub fn idioma(&self) -> Option<Idioma> {
        self.idioma
    }

    /// Orden de dia, mes y año con que se muestran las fechas.
    pub fn formato_de_fecha(&self) -> FormatoDeFecha {
        self.formato_de_fecha
    }

    pub fn guardar_formato_de_fecha(&mut self, formato: FormatoDeFecha) {
        if self.formato_de_fecha != formato {
            self.formato_de_fecha = formato;
            self.escribir();
        }
    }

    pub fn guardar_idioma(&mut self, idioma: Idioma) {
        if self.idioma != Some(idioma) {
            self.idioma = Some(idioma);
            self.escribir();
        }
    }

    /// Volumen, fader y opciones de reproduccion a la vista en la ventana
    /// principal (si no, ocultos tras el engranaje).
    pub fn controles_visibles(&self) -> bool {
        self.controles_visibles
    }

    pub fn guardar_controles_visibles(&mut self, visibles: bool) {
        if visibles != self.controles_visibles {
            self.controles_visibles = visibles;
            self.escribir();
        }
    }

    /// El panel de detalles muestra la forma de onda del tema en lugar de sus
    /// datos.
    pub fn ver_onda(&self) -> bool {
        self.ver_onda
    }

    pub fn guardar_ver_onda(&mut self, activo: bool) {
        if activo != self.ver_onda {
            self.ver_onda = activo;
            self.escribir();
        }
    }

    /// Buscar en internet (Deezer, iTunes, MusicBrainz) la carátula de los
    /// temas que no la traen ni la tienen en su carpeta. Apagado de entrada:
    /// manda el artista y el album a esos servicios.
    pub fn caratulas_de_internet(&self) -> bool {
        self.caratulas_de_internet
    }

    pub fn guardar_caratulas_de_internet(&mut self, activo: bool) {
        if activo != self.caratulas_de_internet {
            self.caratulas_de_internet = activo;
            self.escribir();
        }
    }

    /// Ocultar los textos de los botones de la ventana principal: quedan los
    /// iconos y el nombre pasa al globo de ayuda.
    pub fn ocultar_textos(&self) -> bool {
        self.ocultar_textos
    }

    pub fn guardar_ocultar_textos(&mut self, activo: bool) {
        if activo != self.ocultar_textos {
            self.ocultar_textos = activo;
            self.escribir();
        }
    }

    /// Ajustes de los efectos a la vista en la botonera (si no, plegados
    /// tras el engranaje y todos los botones entran sin desplazar).
    pub fn ajustes_de_efectos_visibles(&self) -> bool {
        self.ajustes_de_efectos_visibles
    }

    pub fn guardar_ajustes_de_efectos_visibles(&mut self, visibles: bool) {
        if visibles != self.ajustes_de_efectos_visibles {
            self.ajustes_de_efectos_visibles = visibles;
            self.escribir();
        }
    }

    /// Carpeta de las locuciones pregrabadas del anuncio de hora,
    /// temperatura y humedad.
    pub fn carpeta_de_locuciones(&self) -> Option<&Path> {
        self.carpeta_de_locuciones.as_deref()
    }

    pub fn guardar_carpeta_de_locuciones(&mut self, carpeta: &Path) {
        if Some(carpeta) != self.carpeta_de_locuciones.as_deref() {
            self.carpeta_de_locuciones = Some(carpeta.to_path_buf());
            self.escribir();
        }
    }

    /// Localidad de la que se anuncia el clima (Buenos Aires si no se eligio
    /// otra en el archivo de preferencias).
    pub fn localidad_del_clima(&self) -> &Localidad {
        &self.localidad_del_clima
    }

    pub fn guardar_localidad_del_clima(&mut self, localidad: Localidad) {
        if localidad != self.localidad_del_clima {
            self.localidad_del_clima = localidad;
            self.escribir();
        }
    }

    /// Mantener la pista que suena en el centro de la lista.
    pub fn desplazar(&self) -> bool {
        self.desplazar
    }

    pub fn guardar_desplazar(&mut self, activo: bool) {
        if activo != self.desplazar {
            self.desplazar = activo;
            self.escribir();
        }
    }

    /// Modo de operacion elegido (AL AIRE o EDITOR).
    pub fn modo_de_operacion(&self) -> ModoDeOperacion {
        if self.al_aire { ModoDeOperacion::AlAire } else { ModoDeOperacion::Editor }
    }

    pub fn guardar_modo_de_operacion(&mut self, modo: ModoDeOperacion) {
        if modo.al_aire() != self.al_aire {
            self.al_aire = modo.al_aire();
            self.escribir();
        }
    }

    fn escribir(&self) {
        let datos = json!({
            "fader": self.fader,
            "crossfadeInteligente": self.crossfade_inteligente,
            "nivelarVolumen": self.nivelar_volumen,
            "objetivoLufs": self.objetivo_de_sonoridad.lufs(),
            "desplazar": self.desplazar,
            "alAire": self.al_aire,
            "ultimaLista": self.ultima_lista.as_ref().map(|lista| lista.to_string_lossy().into_owned()),
            "ultimoTema": self.ultimo_tema.as_ref().map(|tema| json!({
                "posicion": tema.posicion,
                "archivo": tema.archivo.to_string_lossy(),
            })),
            "ventanaDelEditor": self.ventana_del_editor.map(|geometria| json!({
                "x": geometria.x,
                "y": geometria.y,
                "ancho": geometria.ancho,
                "alto": geometria.alto,
            })),
            "editorCentrado": self.editor_centrado,
            "ocultarTextos": self.ocultar_textos,
            "verOnda": self.ver_onda,
            "caratulasDeInternet": self.caratulas_de_internet,
            "controlesVisibles": self.controles_visibles,
            "ajustesDeEfectosVisibles": self.ajustes_de_efectos_visibles,
            "salidaDePreescucha": self.salida_de_preescucha.as_ref().map(|salida| json!({
                "id": salida.id,
                "nombre": salida.nombre,
            })),
            "volumenDePreescucha": self.volumen_de_preescucha,
            "carpetaDeLocuciones": self.carpeta_de_locuciones.as_ref().map(|carpeta| carpeta.to_string_lossy().into_owned()),
            "localidadDelClima": {
                "nombre": self.localidad_del_clima.nombre,
                "latitud": self.localidad_del_clima.latitud,
                "longitud": self.localidad_del_clima.longitud,
                "zonaHoraria": self.localidad_del_clima.zona_horaria,
            },
            "idioma": self.idioma.map(Idioma::codigo),
            "formatoDeFecha": self.formato_de_fecha.codigo(),
        });
        if let Err(error) = escribir_json(&self.archivo, &datos) {
            log::warn!("No se pudieron guardar las preferencias en {}: {error}", self.archivo.display());
        }
    }
}

/// Fecha y hora local de los registros en ISO 8601: "2026-11-02 14:05:33".
/// Va de lo mayor a lo menor (anio, mes, dia), asi que no se confunde el dia
/// con el mes y las planillas la reconocen como fecha.
pub const FORMATO_DE_FECHA_Y_HORA: &str = "%Y-%m-%d %H:%M:%S";

/// Una salida al aire de un comercial, para el control de la publicidad.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EmisionDeComercial {
    /// Fecha y hora local de inicio y de fin (`FORMATO_DE_FECHA_Y_HORA`).
    pub inicio: String,
    pub fin: String,
    /// Segundos que llego a sonar (sin contar las pausas).
    pub segundos: f64,
    pub duracion: Option<f64>,
    /// Sono hasta el final o se corto antes.
    pub completa: bool,
    pub titulo: String,
    pub artista: String,
    pub genero: String,
    /// Comercial o Publicidad (vacio en las lineas de versiones anteriores).
    pub tipo: String,
    pub archivo: PathBuf,
    /// Tanda programada que lo emitio ("Tanda A 10:30"); vacio si se puso a mano.
    pub tanda: String,
    /// Orden de publicidad a la que pertenece; vacio si no es de una orden.
    pub orden: String,
}

/// Donde se guardan las salidas al aire de la propaganda. Hoy son archivos
/// CSV por mes (`RegistroDeComerciales`); para pasar a una base de datos
/// alcanza con otra implementacion de este contrato.
pub trait AlmacenDeEmisiones {
    /// Anota una salida al aire. Un error de escritura se informa en el log:
    /// no debe frenar la emision.
    fn anotar(&self, emision: &EmisionDeComercial);
    /// Las salidas que empezaron entre esas fechas (inclusive), en orden.
    fn emisiones_entre(&self, desde: Date, hasta: Date) -> Result<Vec<EmisionDeComercial>, String>;
}

/// Meses de registro que se conservan: el actual y los dos anteriores, asi
/// el mes pasado siempre esta completo para facturarlo.
pub const MESES_DE_REGISTRO: u32 = 3;

/// Registro de los comerciales emitidos en un CSV por mes
/// (comerciales/2026-11.csv), separado por punto y coma, que se abre con una
/// planilla de calculo. Solo se agregan lineas. Al cambiar de mes se borran
/// los archivos que quedaron fuera de los meses a conservar.
pub struct RegistroDeComerciales {
    carpeta: PathBuf,
    meses_a_conservar: u32,
    // Mes ("2026-11") en que se roto por ultima vez.
    mes_rotado: RefCell<String>,
}

// El registro unico de las versiones anteriores queda con este sufijo una
// vez repartido en los archivos de cada mes.
const SUFIJO_DEL_REGISTRO_MIGRADO: &str = ".migrado";

/// Fecha "02/11/2026 14:05:33" (dia/mes/anio, como anotaban las versiones
/// anteriores) en ISO 8601: "2026-11-02 14:05:33". None si no tiene esa forma.
fn fecha_dia_mes_anio_a_iso(campo: &str) -> Option<String> {
    let (fecha, hora) = campo.split_once(' ')?;
    let partes: Vec<&str> = fecha.split('/').collect();
    let [dia, mes, anio] = partes.as_slice() else {
        return None;
    };
    let son_digitos = |texto: &str, largo: usize| texto.len() == largo && texto.bytes().all(|byte| byte.is_ascii_digit());
    (son_digitos(dia, 2) && son_digitos(mes, 2) && son_digitos(anio, 4)).then(|| format!("{anio}-{mes}-{dia} {hora}"))
}

/// Linea del registro con inicio y fin pasados a ISO 8601, o None si no
/// tiene fechas del formato anterior (el encabezado, o ya convertida).
fn linea_con_fechas_iso(linea: &str) -> Option<String> {
    let mut campos = linea.splitn(3, ';');
    let (inicio, fin, resto) = (campos.next()?, campos.next()?, campos.next()?);
    Some(format!("{};{};{resto}", fecha_dia_mes_anio_a_iso(inicio)?, fecha_dia_mes_anio_a_iso(fin)?))
}

/// Mes "2026-11" de una fecha.
fn mes_de(fecha: Date) -> String {
    format!("{:04}-{:02}", fecha.year(), fecha.month())
}

/// Mes "2026-11" con que empieza un texto ISO 8601 ("2026-11-02 14:05:33"),
/// o None si no empieza asi.
fn mes_al_principio(texto: &str) -> Option<&str> {
    let mes = texto.get(..7)?;
    let bytes = mes.as_bytes();
    let es_mes = bytes[4] == b'-' && bytes.iter().enumerate().all(|(posicion, byte)| posicion == 4 || byte.is_ascii_digit());
    es_mes.then_some(mes)
}

/// Separa los campos de una linea CSV con punto y coma; un campo entre
/// comillas puede tener punto y coma y comillas dobladas.
fn campos_csv(linea: &str) -> Vec<String> {
    let mut campos = vec![String::new()];
    let mut entre_comillas = false;
    let mut letras = linea.chars().peekable();
    while let Some(letra) = letras.next() {
        let campo = campos.last_mut().expect("siempre hay un campo");
        match letra {
            '"' if entre_comillas && letras.peek() == Some(&'"') => {
                letras.next();
                campo.push('"');
            }
            '"' => entre_comillas = !entre_comillas,
            ';' if !entre_comillas => campos.push(String::new()),
            otra => campo.push(otra),
        }
    }
    campos
}

impl RegistroDeComerciales {
    const ENCABEZADO: &'static str = "Inicio;Fin;Segundos emitidos;Duracion;Completo;Titulo;Artista;Genero;Archivo;Tanda;Orden;Tipo";

    /// Registro en esa carpeta, conservando esa cantidad de meses. Si existe
    /// `anterior` (el registro unico de las versiones previas), lo reparte
    /// primero en los archivos de cada mes.
    pub fn new(carpeta: PathBuf, anterior: Option<&Path>, meses_a_conservar: u32) -> Self {
        let registro = Self { carpeta, meses_a_conservar: meses_a_conservar.max(1), mes_rotado: RefCell::default() };
        if let Some(anterior) = anterior.filter(|anterior| anterior.is_file())
            && let Err(error) = registro.migrar(anterior)
        {
            log::warn!("No se pudo repartir por mes el registro {}: {error}", anterior.display());
        }
        registro.rotar(jiff::Zoned::now().date());
        registro
    }

    /// Registro en ~/.mcp-raptor-player/comerciales, con los ultimos tres meses.
    pub fn en_carpeta_del_usuario() -> Self {
        let carpeta = carpeta_del_usuario();
        Self::new(carpeta.join("comerciales"), Some(&carpeta.join("comerciales.csv")), MESES_DE_REGISTRO)
    }

    fn archivo_del_mes(&self, mes: &str) -> PathBuf {
        self.carpeta.join(format!("{mes}.csv"))
    }

    /// Reparte el registro unico de las versiones anteriores en los archivos
    /// de cada mes, pasando a ISO 8601 las fechas dia/mes/anio. El original
    /// queda intacto con el sufijo ".migrado".
    fn migrar(&self, anterior: &Path) -> std::io::Result<()> {
        let texto = fs::read_to_string(anterior)?;
        let mes_actual = mes_de(jiff::Zoned::now().date());
        let mut por_mes: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for linea in texto.trim_start_matches('\u{feff}').lines() {
            if linea.trim().is_empty() || linea.starts_with("Inicio;") {
                continue;
            }
            let linea = linea_con_fechas_iso(linea).unwrap_or_else(|| linea.to_string());
            let mes = mes_al_principio(&linea).map_or_else(|| mes_actual.clone(), str::to_string);
            por_mes.entry(mes).or_default().push(linea);
        }
        let mut cantidad = 0;
        for (mes, lineas) in &por_mes {
            for linea in lineas {
                self.agregar_linea(mes, linea)?;
                cantidad += 1;
            }
        }
        let copia = agregar_sufijo(anterior, SUFIJO_DEL_REGISTRO_MIGRADO);
        fs::rename(anterior, &copia)?;
        log::info!(
            "Registro de comerciales: {cantidad} linea(s) repartidas en {} archivo(s) por mes; el original quedo en {}",
            por_mes.len(),
            copia.display()
        );
        Ok(())
    }

    /// Borra los archivos de los meses que quedaron afuera de los que se
    /// conservan (contando el de `hoy`).
    pub(crate) fn rotar(&self, hoy: Date) {
        let primero = hoy.first_of_month();
        let limite = primero.checked_sub(Span::new().months(i64::from(self.meses_a_conservar - 1))).unwrap_or(primero);
        let limite = mes_de(limite);
        *self.mes_rotado.borrow_mut() = mes_de(hoy);
        let Ok(entradas) = fs::read_dir(&self.carpeta) else {
            return;
        };
        for entrada in entradas.flatten() {
            let nombre = entrada.file_name().to_string_lossy().into_owned();
            let Some(mes) = nombre.strip_suffix(".csv").filter(|mes| mes.len() == 7 && mes_al_principio(mes).is_some()) else {
                continue;
            };
            if mes < limite.as_str() {
                match fs::remove_file(entrada.path()) {
                    Ok(()) => log::info!("Registro de comerciales: se borro {nombre} (se conservan {} meses)", self.meses_a_conservar),
                    Err(error) => log::warn!("No se pudo borrar {}: {error}", entrada.path().display()),
                }
            }
        }
    }

    fn agregar_linea(&self, mes: &str, linea: &str) -> std::io::Result<()> {
        use std::io::Write;
        fs::create_dir_all(&self.carpeta)?;
        let archivo = self.archivo_del_mes(mes);
        let nuevo = !archivo.is_file();
        let mut archivo = fs::OpenOptions::new().create(true).append(true).open(&archivo)?;
        if nuevo {
            // La marca BOM hace que Excel lea bien los acentos.
            write!(archivo, "\u{feff}{}\r\n", Self::ENCABEZADO)?;
        }
        write!(archivo, "{linea}\r\n")
    }

    fn linea(emision: &EmisionDeComercial) -> String {
        let campo = |texto: &str| {
            if texto.contains([';', '"', '\n', '\r']) {
                format!("\"{}\"", texto.replace('"', "\"\"").replace(['\r', '\n'], " "))
            } else {
                texto.to_string()
            }
        };
        let segundos = |valor: f64| format!("{valor:.1}").replace('.', ",");
        [
            campo(&emision.inicio),
            campo(&emision.fin),
            segundos(emision.segundos),
            emision.duracion.map(segundos).unwrap_or_default(),
            (if emision.completa { "Si" } else { "No" }).to_string(),
            campo(&emision.titulo),
            campo(&emision.artista),
            campo(&emision.genero),
            campo(&emision.archivo.display().to_string()),
            campo(&emision.tanda),
            campo(&emision.orden),
            campo(&emision.tipo),
        ]
        .join(";")
    }

    /// Lee una linea del registro; None si no tiene los campos minimos. Las
    /// lineas de las versiones anteriores no tienen las columnas Tanda, Orden
    /// ni Tipo.
    fn leer_linea(linea: &str) -> Option<EmisionDeComercial> {
        let campos = campos_csv(linea);
        if campos.len() < 9 {
            return None;
        }
        let segundos = |texto: &str| texto.trim().replace(',', ".").parse::<f64>().ok();
        Some(EmisionDeComercial {
            inicio: campos[0].clone(),
            fin: campos[1].clone(),
            segundos: segundos(&campos[2])?,
            duracion: segundos(&campos[3]),
            completa: campos[4].eq_ignore_ascii_case("si"),
            titulo: campos[5].clone(),
            artista: campos[6].clone(),
            genero: campos[7].clone(),
            archivo: PathBuf::from(&campos[8]),
            tanda: campos.get(9).cloned().unwrap_or_default(),
            orden: campos.get(10).cloned().unwrap_or_default(),
            tipo: campos.get(11).cloned().unwrap_or_default(),
        })
    }
}

impl AlmacenDeEmisiones for RegistroDeComerciales {
    fn anotar(&self, emision: &EmisionDeComercial) {
        let mes = mes_al_principio(&emision.inicio).map_or_else(|| mes_de(jiff::Zoned::now().date()), str::to_string);
        // El primer comercial de un mes nuevo borra el mes que queda afuera.
        if mes.as_str() > self.mes_rotado.borrow().as_str() {
            self.rotar(jiff::Zoned::now().date());
        }
        if let Err(error) = self.agregar_linea(&mes, &Self::linea(emision)) {
            log::warn!("No se pudo anotar el comercial en {}: {error}", self.archivo_del_mes(&mes).display());
        }
    }

    fn emisiones_entre(&self, desde: Date, hasta: Date) -> Result<Vec<EmisionDeComercial>, String> {
        if desde > hasta {
            return Err(tr!("La fecha inicial es posterior a la final.").into());
        }
        let (primer_dia, ultimo_dia) = (desde.to_string(), hasta.to_string());
        let mut emisiones = Vec::new();
        let mut mes = desde.first_of_month();
        while mes <= hasta {
            let archivo = self.archivo_del_mes(&mes_de(mes));
            match fs::read_to_string(&archivo) {
                Ok(texto) => emisiones.extend(
                    texto
                        .trim_start_matches('\u{feff}')
                        .lines()
                        .skip(1)
                        .filter_map(Self::leer_linea)
                        .filter(|emision| emision.inicio.get(..10).is_some_and(|dia| dia >= primer_dia.as_str() && dia <= ultimo_dia.as_str())),
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(tr!("No se pudo leer {}: {error}", archivo.display(), error = error)),
            }
            let Ok(siguiente) = mes.checked_add(Span::new().months(1)) else { break };
            mes = siguiente;
        }
        emisiones.sort_by(|una, otra| una.inicio.cmp(&otra.inicio));
        Ok(emisiones)
    }
}

/// Donde se guardan las tandas programadas. Hoy es un JSON
/// (`RepositorioDeTandas`); se puede reemplazar por una base de datos.
pub trait AlmacenDeTandas {
    fn tandas(&self) -> &[TandaProgramada];
    /// Agrega la tanda o reemplaza la que tenga el mismo nombre.
    fn guardar_tanda(&mut self, tanda: TandaProgramada);
    /// Quita la tanda de ese nombre y la devuelve.
    fn quitar_tanda(&mut self, nombre: &str) -> Option<TandaProgramada>;
    /// Anota la ultima hora pautada atendida (encolada o perdida).
    fn anotar_salida(&mut self, nombre: &str, salida: DateTime);
}

/// Tandas programadas en ~/.mcp-raptor-player/tandas.json.
///
/// Formato: { "tandas": [ { "nombre": "Tanda A", "archivos": ["D:\\spots\\a.mp3"],
/// "inicio": "10:30", "cadaMinutos": 30, "hasta": "20:00", "dias": ["lun", "mar"],
/// "antesSegundos": 180, "despuesSegundos": 120, "modo": "tolerancia", "prioridad": 5, "activa": true,
/// "ultimaSalida": "2026-11-07T10:30:00" } ] }
pub struct RepositorioDeTandas {
    archivo: PathBuf,
    tandas: Vec<TandaProgramada>,
}

impl RepositorioDeTandas {
    pub fn new(archivo: PathBuf) -> Self {
        let tandas = match fs::read_to_string(&archivo) {
            Ok(texto) => match leer_tandas(&texto) {
                Ok(tandas) => tandas,
                Err(error) => {
                    log::warn!("{} esta danado ({error}); se respalda y se empieza sin tandas", archivo.display());
                    if let Err(error) = fs::rename(&archivo, agregar_sufijo(&archivo, ".danado")) {
                        log::warn!("No se pudo respaldar {}: {error}", archivo.display());
                    }
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Self { archivo, tandas }
    }

    pub fn en_carpeta_del_usuario() -> Self {
        Self::new(carpeta_del_usuario().join("tandas.json"))
    }

    fn escribir(&self) {
        let tandas: Vec<Value> = self.tandas.iter().map(tanda_a_json).collect();
        if let Err(error) = escribir_json(&self.archivo, &json!({ "tandas": tandas })) {
            log::warn!("No se pudieron guardar las tandas en {}: {error}", self.archivo.display());
        }
    }
}

impl AlmacenDeTandas for RepositorioDeTandas {
    fn tandas(&self) -> &[TandaProgramada] {
        &self.tandas
    }

    fn guardar_tanda(&mut self, tanda: TandaProgramada) {
        match self.tandas.iter_mut().find(|guardada| guardada.se_llama(&tanda.nombre)) {
            Some(guardada) => *guardada = tanda,
            None => self.tandas.push(tanda),
        }
        self.escribir();
    }

    fn quitar_tanda(&mut self, nombre: &str) -> Option<TandaProgramada> {
        let posicion = self.tandas.iter().position(|tanda| tanda.se_llama(nombre))?;
        let quitada = self.tandas.remove(posicion);
        self.escribir();
        Some(quitada)
    }

    fn anotar_salida(&mut self, nombre: &str, salida: DateTime) {
        if let Some(tanda) = self.tandas.iter_mut().find(|tanda| tanda.se_llama(nombre)) {
            tanda.ultima_salida = Some(salida);
            self.escribir();
        }
    }
}

fn tanda_a_json(tanda: &TandaProgramada) -> Value {
    let regla = &tanda.recurrencia;
    let dias: Vec<&str> =
        DIAS_DE_LA_SEMANA.iter().zip(regla.dias()).filter(|(_, elegido)| *elegido).map(|(dia, _)| *dia).collect();
    let mut datos = json!({
        "nombre": tanda.nombre,
        "archivos": tanda.archivos.iter().map(|archivo| archivo.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "inicio": formatear_hora(regla.inicio()),
        "dias": dias,
        "antesSegundos": tanda.antes,
        "despuesSegundos": tanda.despues,
        "modo": tanda.modo.clave(),
        "prioridad": tanda.prioridad,
        "activa": tanda.activa,
    });
    if let Some(minutos) = regla.cada_minutos() {
        datos["cadaMinutos"] = json!(minutos);
    }
    if let Some(hasta) = regla.hasta() {
        datos["hasta"] = json!(formatear_hora(hasta));
    }
    if let Some(salida) = tanda.ultima_salida {
        datos["ultimaSalida"] = json!(salida.to_string());
    }
    if let Some(segundos) = tanda.duracion_maxima {
        datos["duracionMaximaSegundos"] = json!(segundos);
    }
    datos
}

fn leer_tandas(texto: &str) -> Result<Vec<TandaProgramada>, String> {
    let datos: Value = serde_json::from_str(texto).map_err(|error| error.to_string())?;
    let tandas = datos.get("tandas").and_then(Value::as_array).ok_or_else(|| tr!("falta la lista 'tandas'"))?;
    tandas.iter().map(tanda_desde_json).collect()
}

fn tanda_desde_json(datos: &Value) -> Result<TandaProgramada, String> {
    let texto = |clave: &str| datos.get(clave).and_then(Value::as_str);
    let segundos = |clave: &str, por_defecto: u32| {
        datos.get(clave).and_then(Value::as_u64).map_or(por_defecto, |valor| valor.min(u64::from(TOLERANCIA_MAXIMA)) as u32)
    };
    let nombre = texto("nombre").filter(|nombre| !nombre.trim().is_empty()).ok_or_else(|| tr!("una tanda no tiene nombre"))?.to_string();
    let archivos = datos
        .get("archivos")
        .and_then(Value::as_array)
        .map(|archivos| archivos.iter().filter_map(Value::as_str).map(PathBuf::from).collect())
        .unwrap_or_default();
    let inicio = parsear_hora(texto("inicio").ok_or_else(|| tr!("la tanda {nombre} no tiene 'inicio'", nombre = nombre))?)?;
    let hasta = texto("hasta").map(parsear_hora).transpose()?;
    let cada = datos.get("cadaMinutos").and_then(Value::as_u64).map(|minutos| minutos.min(u64::from(u32::MAX)) as u32);
    let dias: Vec<String> = datos
        .get("dias")
        .and_then(Value::as_array)
        .map(|dias| dias.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_else(|| vec!["todos".into()]);
    let recurrencia = Recurrencia::nueva(inicio, cada, hasta, parsear_dias(&dias)?).map_err(|error| tr!("tanda {nombre}: {error}", nombre = nombre, error = error))?;
    let ultima_salida = texto("ultimaSalida").and_then(|salida| salida.parse::<DateTime>().ok());
    Ok(TandaProgramada {
        nombre,
        archivos,
        recurrencia,
        antes: segundos("antesSegundos", TOLERANCIA_PREVIA_POR_DEFECTO),
        despues: segundos("despuesSegundos", TOLERANCIA_POSTERIOR_POR_DEFECTO),
        modo: texto("modo").map(ModoDeEntrada::desde_clave).transpose()?.unwrap_or_default(),
        prioridad: datos
            .get("prioridad")
            .and_then(Value::as_u64)
            .map_or(PRIORIDAD_POR_DEFECTO, |prioridad| prioridad.clamp(u64::from(PRIORIDAD_MINIMA), u64::from(PRIORIDAD_MAXIMA)) as u8),
        duracion_maxima: datos.get("duracionMaximaSegundos").and_then(Value::as_u64).filter(|segundos| *segundos > 0).map(|segundos| segundos.min(3_600) as u32),
        activa: datos.get("activa").and_then(Value::as_bool).unwrap_or(true),
        ultima_salida,
    })
}

/// Donde se guardan las ordenes de publicidad. Hoy es un JSON
/// (`RepositorioDeOrdenes`); se puede reemplazar por una base de datos.
pub trait AlmacenDeOrdenes {
    fn ordenes(&self) -> &[OrdenDePublicidad];
    /// Agrega la orden o reemplaza la que tenga el mismo nombre.
    fn guardar_orden(&mut self, orden: OrdenDePublicidad);
    fn quitar_orden(&mut self, nombre: &str) -> Option<OrdenDePublicidad>;
}

/// Ordenes de publicidad en ~/.mcp-raptor-player/ordenes.json.
///
/// Formato: { "ordenes": [ { "nombre": "Sol - Noviembre", "anunciante": "Supermercado Sol",
/// "rubro": "Supermercados", "desde": "2026-11-01", "hasta": "2026-11-30", "dias": ["lun"],
/// "franjaDesde": "08:00", "franjaHasta": "20:00", "frecuencia": { "cadaMinutos": 120 }
/// (o { "vecesPorDia": 3 } o { "horarios": ["08:00", "12:30"] }), "avisos": ["D:\\a.mp3"],
/// "tandas": ["Tanda A"], "prioridad": 5, "activa": true } ] }
pub struct RepositorioDeOrdenes {
    archivo: PathBuf,
    ordenes: Vec<OrdenDePublicidad>,
}

impl RepositorioDeOrdenes {
    pub fn new(archivo: PathBuf) -> Self {
        let ordenes = match fs::read_to_string(&archivo) {
            Ok(texto) => match leer_ordenes(&texto) {
                Ok(ordenes) => ordenes,
                Err(error) => {
                    log::warn!("{} esta danado ({error}); se respalda y se empieza sin ordenes", archivo.display());
                    if let Err(error) = fs::rename(&archivo, agregar_sufijo(&archivo, ".danado")) {
                        log::warn!("No se pudo respaldar {}: {error}", archivo.display());
                    }
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Self { archivo, ordenes }
    }

    pub fn en_carpeta_del_usuario() -> Self {
        Self::new(carpeta_del_usuario().join("ordenes.json"))
    }

    fn escribir(&self) {
        let ordenes: Vec<Value> = self.ordenes.iter().map(orden_a_json).collect();
        if let Err(error) = escribir_json(&self.archivo, &json!({ "ordenes": ordenes })) {
            log::warn!("No se pudieron guardar las ordenes en {}: {error}", self.archivo.display());
        }
    }
}

impl AlmacenDeOrdenes for RepositorioDeOrdenes {
    fn ordenes(&self) -> &[OrdenDePublicidad] {
        &self.ordenes
    }

    fn guardar_orden(&mut self, orden: OrdenDePublicidad) {
        match self.ordenes.iter_mut().find(|guardada| guardada.se_llama(&orden.nombre)) {
            Some(guardada) => *guardada = orden,
            None => self.ordenes.push(orden),
        }
        self.escribir();
    }

    fn quitar_orden(&mut self, nombre: &str) -> Option<OrdenDePublicidad> {
        let posicion = self.ordenes.iter().position(|orden| orden.se_llama(nombre))?;
        let quitada = self.ordenes.remove(posicion);
        self.escribir();
        Some(quitada)
    }
}

/// Donde se guardan las fichas de los anunciantes. Hoy es un JSON
/// (`RepositorioDeAnunciantes`); se puede reemplazar por una base de datos.
pub trait AlmacenDeAnunciantes {
    fn anunciantes(&self) -> &[Anunciante];
    /// Agrega la ficha o reemplaza la que tenga el mismo nombre.
    fn guardar_anunciante(&mut self, anunciante: Anunciante);
    fn quitar_anunciante(&mut self, nombre: &str) -> Option<Anunciante>;
}

/// Fichas de los anunciantes en ~/.mcp-raptor-player/anunciantes.json.
///
/// Formato: { "anunciantes": [ { "nombre": "Supermercado Sol", "razonSocial": "Sol S.A.",
/// "cuit": "30-71234567-1", "rubro": "Supermercados", "contacto": "Ana Perez",
/// "telefono": "011 4555-1234", "correo": "ventas@sol.com.ar", "direccion": "...",
/// "agencia": "", "notas": "", "activo": true } ] }
pub struct RepositorioDeAnunciantes {
    archivo: PathBuf,
    anunciantes: Vec<Anunciante>,
}

impl RepositorioDeAnunciantes {
    pub fn new(archivo: PathBuf) -> Self {
        let anunciantes = match fs::read_to_string(&archivo) {
            Ok(texto) => match leer_anunciantes(&texto) {
                Ok(anunciantes) => anunciantes,
                Err(error) => {
                    log::warn!("{} esta danado ({error}); se respalda y se empieza sin anunciantes", archivo.display());
                    if let Err(error) = fs::rename(&archivo, agregar_sufijo(&archivo, ".danado")) {
                        log::warn!("No se pudo respaldar {}: {error}", archivo.display());
                    }
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Self { archivo, anunciantes }
    }

    pub fn en_carpeta_del_usuario() -> Self {
        Self::new(carpeta_del_usuario().join("anunciantes.json"))
    }

    fn escribir(&self) {
        let anunciantes: Vec<Value> = self.anunciantes.iter().map(anunciante_a_json).collect();
        if let Err(error) = escribir_json(&self.archivo, &json!({ "anunciantes": anunciantes })) {
            log::warn!("No se pudieron guardar los anunciantes en {}: {error}", self.archivo.display());
        }
    }
}

impl AlmacenDeAnunciantes for RepositorioDeAnunciantes {
    fn anunciantes(&self) -> &[Anunciante] {
        &self.anunciantes
    }

    fn guardar_anunciante(&mut self, anunciante: Anunciante) {
        match self.anunciantes.iter_mut().find(|guardado| guardado.se_llama(&anunciante.nombre)) {
            Some(guardado) => *guardado = anunciante,
            None => self.anunciantes.push(anunciante),
        }
        self.anunciantes.sort_by_key(|anunciante| anunciante.nombre.to_lowercase());
        self.escribir();
    }

    fn quitar_anunciante(&mut self, nombre: &str) -> Option<Anunciante> {
        let posicion = self.anunciantes.iter().position(|anunciante| anunciante.se_llama(nombre))?;
        let quitado = self.anunciantes.remove(posicion);
        self.escribir();
        Some(quitado)
    }
}

fn anunciante_a_json(anunciante: &Anunciante) -> Value {
    json!({
        "nombre": anunciante.nombre,
        "razonSocial": anunciante.razon_social,
        "cuit": anunciante.cuit,
        "rubro": anunciante.rubro,
        "contacto": anunciante.contacto,
        "telefono": anunciante.telefono,
        "correo": anunciante.correo,
        "direccion": anunciante.direccion,
        "agencia": anunciante.agencia,
        "notas": anunciante.notas,
        "activo": anunciante.activo,
    })
}

fn leer_anunciantes(texto: &str) -> Result<Vec<Anunciante>, String> {
    let datos: Value = serde_json::from_str(texto).map_err(|error| error.to_string())?;
    let anunciantes = datos.get("anunciantes").and_then(Value::as_array).ok_or_else(|| tr!("falta la lista 'anunciantes'"))?;
    Ok(anunciantes
        .iter()
        .map(|datos| {
            let texto = |clave: &str| datos.get(clave).and_then(Value::as_str).unwrap_or_default().to_string();
            Anunciante {
                nombre: texto("nombre"),
                razon_social: texto("razonSocial"),
                cuit: texto("cuit"),
                rubro: texto("rubro"),
                contacto: texto("contacto"),
                telefono: texto("telefono"),
                correo: texto("correo"),
                direccion: texto("direccion"),
                agencia: texto("agencia"),
                notas: texto("notas"),
                activo: datos.get("activo").and_then(Value::as_bool).unwrap_or(true),
            }
        })
        .filter(|anunciante| !anunciante.nombre.trim().is_empty())
        .collect())
}

/// Los almacenes de la publicidad (emisiones, tandas, ordenes, pautas y
/// anunciantes). Hoy son archivos; cada uno puede reemplazarse por una base
/// de datos implementando su trait.
pub struct AlmacenesDePublicidad {
    pub emisiones: Box<dyn AlmacenDeEmisiones>,
    pub tandas: Box<dyn AlmacenDeTandas>,
    pub ordenes: Box<dyn AlmacenDeOrdenes>,
    pub pautas: Box<dyn AlmacenDePautas>,
    pub anunciantes: Box<dyn AlmacenDeAnunciantes>,
}

impl AlmacenesDePublicidad {
    /// Los archivos de ~/.mcp-raptor-player.
    pub fn en_carpeta_del_usuario() -> Self {
        Self {
            emisiones: Box::new(RegistroDeComerciales::en_carpeta_del_usuario()),
            tandas: Box::new(RepositorioDeTandas::en_carpeta_del_usuario()),
            ordenes: Box::new(RepositorioDeOrdenes::en_carpeta_del_usuario()),
            pautas: Box::new(RepositorioDePautas::en_carpeta_del_usuario()),
            anunciantes: Box::new(RepositorioDeAnunciantes::en_carpeta_del_usuario()),
        }
    }
}

fn dias_a_json(dias: [bool; 7]) -> Vec<&'static str> {
    DIAS_DE_LA_SEMANA.iter().zip(dias).filter(|(_, elegido)| *elegido).map(|(dia, _)| *dia).collect()
}

fn textos_de(datos: &Value, clave: &str) -> Vec<String> {
    datos.get(clave).and_then(Value::as_array).map(|valores| valores.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn orden_a_json(orden: &OrdenDePublicidad) -> Value {
    let frecuencia = match &orden.frecuencia {
        Frecuencia::CadaMinutos(minutos) => json!({ "cadaMinutos": minutos }),
        Frecuencia::VecesPorDia(veces) => json!({ "vecesPorDia": veces }),
        Frecuencia::EnHorarios(horarios) => json!({ "horarios": horarios.iter().map(|horario| formatear_hora(*horario)).collect::<Vec<_>>() }),
    };
    json!({
        "nombre": orden.nombre,
        "anunciante": orden.anunciante,
        "rubro": orden.rubro,
        "desde": orden.desde.to_string(),
        "hasta": orden.hasta.to_string(),
        "dias": dias_a_json(orden.dias),
        "franjaDesde": formatear_hora(orden.franja.0),
        "franjaHasta": formatear_hora(orden.franja.1),
        "frecuencia": frecuencia,
        "avisos": orden.avisos.iter().map(|aviso| aviso.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "tandas": orden.tandas,
        "prioridad": orden.prioridad,
        "activa": orden.activa,
    })
}

fn leer_ordenes(texto: &str) -> Result<Vec<OrdenDePublicidad>, String> {
    let datos: Value = serde_json::from_str(texto).map_err(|error| error.to_string())?;
    let ordenes = datos.get("ordenes").and_then(Value::as_array).ok_or_else(|| tr!("falta la lista 'ordenes'"))?;
    ordenes.iter().map(orden_desde_json).collect()
}

fn orden_desde_json(datos: &Value) -> Result<OrdenDePublicidad, String> {
    let texto = |clave: &str| datos.get(clave).and_then(Value::as_str).unwrap_or_default().to_string();
    let nombre = texto("nombre");
    let fecha = |clave: &str| texto(clave).parse::<Date>().map_err(|_| tr!("orden {nombre}: '{clave}' no es una fecha AAAA-MM-DD", nombre = nombre, clave = clave));
    let hora = |clave: &str, por_defecto: &str| {
        let valor = texto(clave);
        parsear_hora(if valor.is_empty() { por_defecto } else { &valor })
    };
    let frecuencia = datos.get("frecuencia").ok_or_else(|| tr!("orden {nombre}: falta 'frecuencia'", nombre = nombre))?;
    let frecuencia = if let Some(minutos) = frecuencia.get("cadaMinutos").and_then(Value::as_u64) {
        Frecuencia::CadaMinutos(minutos.min(1_440) as u32)
    } else if let Some(veces) = frecuencia.get("vecesPorDia").and_then(Value::as_u64) {
        Frecuencia::VecesPorDia(veces.min(96) as u32)
    } else {
        Frecuencia::EnHorarios(textos_de(frecuencia, "horarios").iter().map(|horario| parsear_hora(horario)).collect::<Result<_, _>>()?)
    };
    let dias = textos_de(datos, "dias");
    let orden = OrdenDePublicidad {
        anunciante: texto("anunciante"),
        rubro: texto("rubro"),
        desde: fecha("desde")?,
        hasta: fecha("hasta")?,
        dias: if dias.is_empty() { [true; 7] } else { parsear_dias(&dias)? },
        franja: (hora("franjaDesde", "00:00")?, hora("franjaHasta", "23:59")?),
        frecuencia,
        avisos: textos_de(datos, "avisos").into_iter().map(PathBuf::from).collect(),
        tandas: textos_de(datos, "tandas"),
        prioridad: datos
            .get("prioridad")
            .and_then(Value::as_u64)
            .map_or(PRIORIDAD_POR_DEFECTO, |prioridad| prioridad.clamp(u64::from(PRIORIDAD_MINIMA), u64::from(PRIORIDAD_MAXIMA)) as u8),
        activa: datos.get("activa").and_then(Value::as_bool).unwrap_or(true),
        nombre: nombre.clone(),
    };
    orden.validar().map_err(|error| tr!("orden {nombre}: {error}", nombre = nombre, error = error))?;
    Ok(orden)
}

/// Donde se guarda la pauta de cada dia (para revisarla despues) y las
/// reposiciones pendientes, que sobreviven a un reinicio.
pub trait AlmacenDePautas {
    fn guardar_pauta(&self, fecha: Date, pauta: &PautaDelDia, reposiciones: &[Reposicion]);
    fn reposiciones(&self, fecha: Date) -> Vec<Reposicion>;
}

/// Pautas del dia en ~/.mcp-raptor-player/pauta/AAAA-MM-DD.json; se
/// conservan los mismos meses que el registro de comerciales.
pub struct RepositorioDePautas {
    carpeta: PathBuf,
    meses_a_conservar: u32,
}

impl RepositorioDePautas {
    pub fn new(carpeta: PathBuf, meses_a_conservar: u32) -> Self {
        let repositorio = Self { carpeta, meses_a_conservar: meses_a_conservar.max(1) };
        repositorio.rotar(jiff::Zoned::now().date());
        repositorio
    }

    pub fn en_carpeta_del_usuario() -> Self {
        Self::new(carpeta_del_usuario().join("pauta"), MESES_DE_REGISTRO)
    }

    fn archivo(&self, fecha: Date) -> PathBuf {
        self.carpeta.join(format!("{fecha}.json"))
    }

    /// Borra las pautas de los meses que ya no se conservan.
    pub(crate) fn rotar(&self, hoy: Date) {
        let primero = hoy.first_of_month();
        let limite = mes_de(primero.checked_sub(Span::new().months(i64::from(self.meses_a_conservar - 1))).unwrap_or(primero));
        let Ok(entradas) = fs::read_dir(&self.carpeta) else { return };
        for entrada in entradas.flatten() {
            let nombre = entrada.file_name().to_string_lossy().into_owned();
            let Some(fecha) = nombre.strip_suffix(".json").filter(|fecha| fecha.len() == 10) else { continue };
            if mes_al_principio(fecha).is_some_and(|mes| mes < limite.as_str())
                && let Err(error) = fs::remove_file(entrada.path())
            {
                log::warn!("No se pudo borrar {}: {error}", entrada.path().display());
            }
        }
    }
}

impl AlmacenDePautas for RepositorioDePautas {
    fn guardar_pauta(&self, fecha: Date, pauta: &PautaDelDia, reposiciones: &[Reposicion]) {
        let asignaciones: Vec<Value> = pauta
            .asignaciones
            .iter()
            .map(|asignacion| {
                json!({
                    "orden": asignacion.orden,
                    "aviso": asignacion.aviso.to_string_lossy(),
                    "tanda": asignacion.tanda,
                    "salida": asignacion.salida.to_string(),
                    "reposicion": asignacion.reposicion,
                })
            })
            .collect();
        let sin_ubicar: Vec<Value> = pauta
            .sin_ubicar
            .iter()
            .map(|sin_ubicar| json!({ "orden": sin_ubicar.orden, "objetivo": sin_ubicar.objetivo.to_string(), "motivo": sin_ubicar.motivo }))
            .collect();
        let reposiciones: Vec<Value> = reposiciones
            .iter()
            .map(|reposicion| json!({ "orden": reposicion.orden, "aviso": reposicion.aviso.to_string_lossy(), "desde": reposicion.desde.to_string() }))
            .collect();
        let datos = json!({
            "fecha": fecha.to_string(),
            "asignaciones": asignaciones,
            "sinUbicar": sin_ubicar,
            "reposiciones": reposiciones,
        });
        if let Err(error) = escribir_json(&self.archivo(fecha), &datos) {
            log::warn!("No se pudo guardar la pauta del {fecha}: {error}");
        }
    }

    fn reposiciones(&self, fecha: Date) -> Vec<Reposicion> {
        let Ok(texto) = fs::read_to_string(self.archivo(fecha)) else { return Vec::new() };
        let Ok(datos) = serde_json::from_str::<Value>(&texto) else { return Vec::new() };
        datos
            .get("reposiciones")
            .and_then(Value::as_array)
            .map(|reposiciones| {
                reposiciones
                    .iter()
                    .filter_map(|reposicion| {
                        Some(Reposicion {
                            orden: reposicion.get("orden")?.as_str()?.to_string(),
                            aviso: PathBuf::from(reposicion.get("aviso")?.as_str()?),
                            desde: reposicion.get("desde")?.as_str()?.parse().ok()?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Escribe un certificado de emision en ~/.mcp-raptor-player/certificados
/// (CSV con punto y coma, para una planilla) y devuelve su ruta.
pub fn escribir_certificado(nombre: &str, lineas: &[String]) -> Result<PathBuf, String> {
    let limpio: String = nombre.chars().map(|letra| if letra.is_alphanumeric() || "-_ ".contains(letra) { letra } else { '_' }).collect();
    let archivo = carpeta_del_usuario().join("certificados").join(format!("{}.csv", limpio.trim()));
    let mut texto = String::from("\u{feff}");
    for linea in lineas {
        texto.push_str(linea);
        texto.push_str("\r\n");
    }
    fs::create_dir_all(archivo.parent().expect("tiene carpeta"))
        .and_then(|_| fs::write(&archivo, texto))
        .map_err(|error| tr!("No se pudo escribir el certificado {}: {error}", archivo.display(), error = error))?;
    Ok(archivo)
}

/// Ruta absoluta sin "." ni "..": normalizarla no la cambia.
fn ruta_ya_normalizada(ruta: &Path) -> bool {
    ruta.is_absolute()
        && ruta.components().all(|componente| !matches!(componente, std::path::Component::CurDir | std::path::Component::ParentDir))
}

/// En Windows las rutas no distinguen mayusculas de minusculas.
fn misma_ruta(una: &Path, otra: &Path) -> bool {
    let (una, otra) = (normalizar(una), normalizar(otra));
    if cfg!(windows) {
        una.to_string_lossy().to_lowercase() == otra.to_string_lossy().to_lowercase()
    } else {
        una == otra
    }
}

/// Botones de una hoja de la botonera.
type Hoja = [Option<PathBuf>; CANTIDAD_DE_BOTONES];

/// Largo maximo del nombre de una hoja de la botonera, en caracteres.
pub const LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA: usize = 40;

/// Guarda la botonera de efectos: hojas de 10 botones con el archivo de cada
/// uno, la hoja a la vista, el modo de disparo, el volumen de los efectos y
/// la atenuacion de la musica. Siempre hay al menos una hoja.
///
/// Formato: { "modo": "TRABADA", "atenuarMusica": true, "nivelDeMusica": 0.3, "atenuacionInteligente": true,
///   "volumenDeEfecto": -2.0, "anticipacion": 0.04, "recuperacion": 0.8, "separacionDb": 12, "hojaActual": 0,
///   "hojas": [["C:\\efectos\\aplauso.wav", null, ...], [...]], "nombresDeHojas": ["Offs", null] }
/// La lista "botones" de versiones anteriores se lee como la primera hoja.
/// Una hoja sin nombre (null) se muestra como "Hoja N".
pub struct RepositorioDeBotonera {
    archivo: PathBuf,
    hojas: Vec<Hoja>,
    /// Nombre de cada hoja, a la par de `hojas`; None es el de siempre.
    nombres: Vec<Option<String>>,
    hoja_actual: usize,
    modo: ModoDeOverlay,
    efecto: AjustesDeEfecto,
}

impl RepositorioDeBotonera {
    pub fn new(archivo: PathBuf) -> Self {
        let mut repositorio = Self {
            archivo,
            hojas: vec![Hoja::default()],
            nombres: vec![None],
            hoja_actual: 0,
            modo: ModoDeOverlay::Libre,
            efecto: ajustes_de_efecto_por_defecto(),
        };
        repositorio.cargar();
        repositorio
    }

    /// Repositorio en ~/.mcp-raptor-player/botonera.json.
    pub fn en_carpeta_del_usuario() -> Self {
        Self::new(carpeta_del_usuario().join("botonera.json"))
    }

    pub fn cantidad_de_hojas(&self) -> usize {
        self.hojas.len()
    }

    /// Hoja a la vista, desde 0.
    pub fn hoja_actual(&self) -> usize {
        self.hoja_actual
    }

    pub fn mostrar_hoja(&mut self, hoja: usize) {
        if hoja < self.hojas.len() && hoja != self.hoja_actual {
            self.hoja_actual = hoja;
            self.escribir();
        }
    }

    /// Agrega una hoja vacia al final, la pone a la vista y devuelve su numero.
    pub fn agregar_hoja(&mut self) -> usize {
        self.hojas.push(Hoja::default());
        self.nombres.push(None);
        self.hoja_actual = self.hojas.len() - 1;
        self.escribir();
        self.hoja_actual
    }

    /// Quita una hoja con sus asignaciones (los archivos no se tocan).
    pub fn quitar_hoja(&mut self, hoja: usize) -> Result<(), String> {
        if self.hojas.len() <= 1 {
            return Err(tr!("Tiene que quedar al menos una hoja de efectos.").into());
        }
        if hoja >= self.hojas.len() {
            return Err(tr!("No existe la hoja {} (hay {}).", hoja + 1, self.hojas.len()));
        }
        self.hojas.remove(hoja);
        self.nombres.remove(hoja);
        if self.hoja_actual >= hoja && self.hoja_actual > 0 {
            self.hoja_actual -= 1;
        }
        self.escribir();
        Ok(())
    }

    /// Nombre que se le puso a la hoja (desde 0); None si tiene el de siempre.
    pub fn nombre_de_hoja(&self, hoja: usize) -> Option<&str> {
        self.nombres.get(hoja).and_then(|nombre| nombre.as_deref())
    }

    /// Le pone nombre a una hoja (desde 0). Se recortan los espacios de los
    /// extremos; vacio, vuelve al de siempre ("Hoja N").
    pub fn renombrar_hoja(&mut self, hoja: usize, nombre: &str) -> Result<(), String> {
        let Some(lugar) = self.nombres.get_mut(hoja) else {
            return Err(tr!("No existe la hoja {} (hay {}).", hoja + 1, self.hojas.len()));
        };
        let nombre = nombre.trim();
        if nombre.chars().count() > LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA {
            return Err(tr!("El nombre de la hoja puede tener hasta {} caracteres.", LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA));
        }
        let nuevo = (!nombre.is_empty()).then(|| nombre.to_string());
        if *lugar != nuevo {
            *lugar = nuevo;
            self.escribir();
        }
        Ok(())
    }

    /// Cuantos botones de la hoja tienen archivo.
    pub fn asignados_en(&self, hoja: usize) -> usize {
        self.hojas.get(hoja).map_or(0, |botones| botones.iter().flatten().count())
    }

    pub fn archivo(&self, hoja: usize, boton: usize) -> Option<&Path> {
        self.hojas.get(hoja).and_then(|botones| botones.get(boton)).and_then(|archivo| archivo.as_deref())
    }

    /// Archivo de un boton de la hoja a la vista.
    pub fn archivo_del_boton(&self, boton: usize) -> Option<&Path> {
        self.archivo(self.hoja_actual, boton)
    }

    /// Todos los efectos asignados: (hoja, boton, archivo).
    pub fn efectos(&self) -> impl Iterator<Item = (usize, usize, &Path)> {
        self.hojas.iter().enumerate().flat_map(|(hoja, botones)| {
            botones.iter().enumerate().filter_map(move |(boton, archivo)| archivo.as_deref().map(|archivo| (hoja, boton, archivo)))
        })
    }

    /// Cambia la ruta de los botones que tenian un archivo reubicado.
    /// Devuelve cuantos botones cambiaron.
    pub fn reubicar(&mut self, cambios: &[(PathBuf, PathBuf)]) -> usize {
        let mut cambiados = 0;
        for boton in self.hojas.iter_mut().flat_map(|botones| botones.iter_mut()) {
            let Some(actual) = boton.as_ref() else {
                continue;
            };
            if let Some((_, nueva)) = cambios.iter().find(|(anterior, _)| misma_ruta(anterior, actual)) {
                *boton = Some(normalizar(nueva));
                cambiados += 1;
            }
        }
        if cambiados > 0 {
            self.escribir();
        }
        cambiados
    }

    /// Asigna el archivo al boton. Dentro de una hoja cada efecto va en un
    /// solo boton: si ya esta en otro, no se asigna y se informa cual.
    pub fn asignar(&mut self, hoja: usize, boton: usize, audio: &Path) -> Result<(), String> {
        let audio = normalizar(audio);
        let Some(botones) = self.hojas.get_mut(hoja) else {
            return Err(tr!("No existe la hoja {}.", hoja + 1));
        };
        if let Some(repetido) = botones
            .iter()
            .enumerate()
            .position(|(otro, archivo)| otro != boton && archivo.as_deref().is_some_and(|archivo| misma_ruta(archivo, &audio)))
        {
            return Err(tr!("{} ya esta asignado al boton {repetido} de la hoja {}.", nombre_de_archivo(&audio), hoja + 1, repetido = repetido));
        }
        let Some(lugar) = botones.get_mut(boton) else {
            return Err(tr!("No existe el boton {boton}.", boton = boton));
        };
        *lugar = Some(audio);
        self.escribir();
        Ok(())
    }

    pub fn quitar(&mut self, hoja: usize, boton: usize) {
        if let Some(lugar) = self.hojas.get_mut(hoja).and_then(|botones| botones.get_mut(boton)) {
            *lugar = None;
            self.escribir();
        }
    }

    pub fn modo(&self) -> ModoDeOverlay {
        self.modo
    }

    pub fn guardar_modo(&mut self, modo: ModoDeOverlay) {
        self.modo = modo;
        self.escribir();
    }

    /// Volumen de los efectos y atenuacion de la musica.
    pub fn ajustes_de_efecto(&self) -> AjustesDeEfecto {
        self.efecto
    }

    pub fn guardar_ajustes_de_efecto(&mut self, ajustes: AjustesDeEfecto) {
        self.efecto = acotar_ajustes(ajustes);
        self.escribir();
    }

    fn cargar(&mut self) {
        if !self.archivo.is_file() {
            return;
        }
        if let Err(error) = self.leer() {
            log::warn!("No se pudo leer {}: {error}; se empieza con la botonera vacia", self.archivo.display());
            self.hojas = vec![Hoja::default()];
            self.nombres = vec![None];
            self.hoja_actual = 0;
            self.modo = ModoDeOverlay::Libre;
            self.efecto = ajustes_de_efecto_por_defecto();
        }
    }

    fn leer(&mut self) -> Result<(), String> {
        let texto = fs::read_to_string(&self.archivo).map_err(|error| error.to_string())?;
        let raiz: Value = serde_json::from_str(&texto).map_err(|error| error.to_string())?;
        let raiz = raiz.as_object().ok_or_else(|| tr!("se esperaba un objeto JSON"))?;
        if let Some(modo) = raiz.get("modo") {
            self.modo = match modo.as_str() {
                Some("LIBRE") => ModoDeOverlay::Libre,
                Some("TRABADA") => ModoDeOverlay::Trabada,
                _ => return Err(tr!("modo desconocido: {modo}", modo = modo)),
            };
        }
        let booleano = |clave: &str| -> Result<Option<bool>, String> {
            raiz.get(clave).map(|valor| valor.as_bool().ok_or(tr!("{clave} debe ser booleano", clave = clave))).transpose()
        };
        let numero = |clave: &str| -> Result<Option<f64>, String> {
            raiz.get(clave).map(|valor| valor.as_f64().ok_or(tr!("{clave} debe ser numerico", clave = clave))).transpose()
        };
        let por_defecto = ajustes_de_efecto_por_defecto();
        self.efecto = acotar_ajustes(AjustesDeEfecto {
            atenuar: booleano("atenuarMusica")?.unwrap_or(por_defecto.atenuar),
            nivel_de_musica: numero("nivelDeMusica")?.unwrap_or(por_defecto.nivel_de_musica),
            inteligente: booleano("atenuacionInteligente")?.unwrap_or(por_defecto.inteligente),
            volumen_db: numero("volumenDeEfecto")?.unwrap_or(por_defecto.volumen_db),
            anticipacion: numero("anticipacion")?.unwrap_or(por_defecto.anticipacion),
            recuperacion: numero("recuperacion")?.unwrap_or(por_defecto.recuperacion),
            separacion_db: numero("separacionDb")?.unwrap_or(por_defecto.separacion_db),
        });
        let hojas: Vec<&Value> = match (raiz.get("hojas"), raiz.get("botones")) {
            (Some(Value::Array(hojas)), _) => hojas.iter().collect(),
            (None, Some(botones)) => vec![botones],
            (None, None) => Vec::new(),
            (Some(otro), _) => return Err(tr!("hojas invalidas: {otro}", otro = otro)),
        };
        let mut leidas = Vec::new();
        for hoja in hojas {
            let botones = hoja.as_array().ok_or_else(|| tr!("cada hoja debe ser una lista"))?;
            let mut leida = Hoja::default();
            for (indice, valor) in botones.iter().take(CANTIDAD_DE_BOTONES).enumerate() {
                leida[indice] = match valor {
                    Value::Null => None,
                    Value::String(ruta) => Some(PathBuf::from(ruta)),
                    otro => return Err(tr!("boton invalido: {otro}", otro = otro)),
                };
            }
            leidas.push(leida);
        }
        if leidas.is_empty() {
            leidas.push(Hoja::default());
        }
        // Los nombres faltan en las botoneras de versiones anteriores; uno que
        // no es texto (o vacio) deja el nombre de siempre.
        let nombres_leidos = raiz.get("nombresDeHojas").and_then(Value::as_array);
        let nombres = (0..leidas.len())
            .map(|hoja| {
                nombres_leidos
                    .and_then(|nombres| nombres.get(hoja))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|nombre| !nombre.is_empty())
                    .map(|nombre| nombre.chars().take(LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA).collect())
            })
            .collect();
        self.hoja_actual =
            raiz.get("hojaActual").and_then(Value::as_u64).map_or(0, |hoja| hoja as usize).min(leidas.len() - 1);
        self.hojas = leidas;
        self.nombres = nombres;
        Ok(())
    }

    fn escribir(&self) {
        let hojas: Vec<Value> = self
            .hojas
            .iter()
            .map(|botones| {
                Value::Array(
                    botones
                        .iter()
                        .map(|boton| {
                            boton.as_ref().map_or(Value::Null, |ruta| Value::String(ruta.to_string_lossy().into_owned()))
                        })
                        .collect(),
                )
            })
            .collect();
        let modo = match self.modo {
            ModoDeOverlay::Libre => "LIBRE",
            ModoDeOverlay::Trabada => "TRABADA",
        };
        let contenido = json!({
            "modo": modo,
            "atenuarMusica": self.efecto.atenuar,
            "nivelDeMusica": self.efecto.nivel_de_musica,
            "atenuacionInteligente": self.efecto.inteligente,
            "volumenDeEfecto": self.efecto.volumen_db,
            "anticipacion": self.efecto.anticipacion,
            "recuperacion": self.efecto.recuperacion,
            "separacionDb": self.efecto.separacion_db,
            "hojaActual": self.hoja_actual,
            "hojas": hojas,
            "nombresDeHojas": self.nombres,
        });
        if let Err(error) = escribir_json(&self.archivo, &contenido) {
            log::warn!("No se pudo guardar la botonera en {}: {error}", self.archivo.display());
        }
    }
}

/// Guarda la lista de reproduccion para recuperarla al volver a abrir el
/// programa: las pistas en orden, la actual y si se repite.
///
/// Formato: { "pistas": ["C:\\musica\\a.mp3", ...], "actual": 3, "repetir": false }
pub struct RepositorioDeLista {
    archivo: PathBuf,
}

/// Lista guardada; `actual` es una posicion desde 0 dentro de `pistas`.
#[derive(Debug, Default, PartialEq)]
pub struct ListaGuardada {
    pub pistas: Vec<PathBuf>,
    pub actual: Option<usize>,
    pub repetir: bool,
}

impl RepositorioDeLista {
    pub fn new(archivo: PathBuf) -> Self {
        Self { archivo }
    }

    /// Repositorio de la sesion en ~/.mcp-raptor-player/listas/lista.json: la
    /// lista tal como quedo, guardada en cada cambio. La de versiones
    /// anteriores (en la raiz de la carpeta) se mueve ahi.
    pub fn en_carpeta_del_usuario() -> Self {
        let archivo = carpeta_de_listas().join("lista.json");
        mudar_si_falta(&carpeta_del_usuario().join("lista.json"), &archivo);
        Self::new(archivo)
    }

    pub fn cargar(&self) -> ListaGuardada {
        leer_lista(&self.archivo).unwrap_or_default()
    }

    pub fn guardar(&self, lista: &ListaGuardada) {
        if let Err(error) = escribir_lista(&self.archivo, lista) {
            log::warn!("{error}");
        }
    }
}

/// Carpeta de las listas: la de la sesion y, por defecto, las que se guardan
/// con nombre.
pub fn carpeta_de_listas() -> PathBuf {
    carpeta_del_usuario().join("listas")
}

/// Si el archivo es una lista M3U (.m3u o .m3u8) y no una del reproductor.
fn es_m3u(archivo: &Path) -> bool {
    archivo
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("m3u") || extension.eq_ignore_ascii_case("m3u8"))
}

/// Caracteres de Windows-1252 entre 0x80 y 0x9F (el resto coincide con
/// Latin-1); los cinco que no estan definidos quedan como el caracter de
/// reemplazo.
const WINDOWS_1252_ALTOS: [char; 32] = [
    '€', '\u{fffd}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{fffd}', 'Ž', '\u{fffd}', '\u{fffd}', '‘', '’', '“',
    '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{fffd}', 'ž', 'Ÿ',
];

/// Texto de una M3U: UTF-8 si lo es (con o sin la marca BOM) y, si no,
/// Windows-1252, como guardan las .m3u Winamp y el Reproductor de Windows.
fn texto_de_m3u(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(texto) => texto,
        Err(error) => error
            .into_bytes()
            .into_iter()
            .map(|byte| match byte {
                0x80..=0x9F => WINDOWS_1252_ALTOS[usize::from(byte - 0x80)],
                otro => char::from(otro),
            })
            .collect(),
    }
}

/// Ruta de una linea de M3U: una direccion file:// (como las guarda VLC, con
/// los caracteres codificados) o una ruta, absoluta o relativa a la lista.
fn ruta_de_linea_m3u(linea: &str, carpeta: &Path) -> PathBuf {
    let Some(direccion) = linea.get(..7).filter(|inicio| inicio.eq_ignore_ascii_case("file://")).map(|_| &linea[7..]) else {
        return carpeta.join(linea);
    };
    let ruta = decodificar_porcentajes(direccion);
    // file:///D:/... en Windows es D:/...; file://servidor/... es //servidor/...
    let ruta = match ruta.strip_prefix('/') {
        Some(local) if cfg!(windows) && local.as_bytes().get(1) == Some(&b':') => local.to_string(),
        Some(_) => ruta,
        None => format!("//{ruta}"),
    };
    PathBuf::from(ruta)
}

/// Deshace la codificacion %XX de una direccion (los bytes forman UTF-8).
fn decodificar_porcentajes(texto: &str) -> String {
    let bytes = texto.as_bytes();
    let mut resultado = Vec::with_capacity(bytes.len());
    let mut indice = 0;
    while indice < bytes.len() {
        let par = bytes.get(indice + 1..indice + 3).and_then(|par| std::str::from_utf8(par).ok()).and_then(|par| u8::from_str_radix(par, 16).ok());
        match (bytes[indice], par) {
            (b'%', Some(byte)) => {
                resultado.push(byte);
                indice += 3;
            }
            (byte, _) => {
                resultado.push(byte);
                indice += 1;
            }
        }
    }
    String::from_utf8_lossy(&resultado).into_owned()
}

/// Lee una lista: del reproductor (JSON) o M3U, que usan otros programas. En
/// una M3U las rutas relativas son relativas a la carpeta de la lista.
pub fn leer_lista(archivo: &Path) -> Result<ListaGuardada, String> {
    let error_de_lectura = |error: std::io::Error| tr!("No se pudo leer {}: {error}", archivo.display(), error = error);
    if es_m3u(archivo) {
        let texto = texto_de_m3u(fs::read(archivo).map_err(error_de_lectura)?);
        let carpeta = archivo.parent().unwrap_or(Path::new(""));
        let pistas = texto
            .lines()
            .map(|linea| linea.trim().trim_start_matches('\u{feff}'))
            .filter(|linea| !linea.is_empty() && !linea.starts_with('#'))
            .map(|linea| ruta_de_linea_m3u(linea, carpeta))
            .collect();
        return Ok(ListaGuardada { pistas, actual: None, repetir: false });
    }
    let texto = fs::read_to_string(archivo).map_err(error_de_lectura)?;
    let raiz: Value =
        serde_json::from_str(&texto).map_err(|error| tr!("{} no es una lista valida: {error}", archivo.display(), error = error))?;
    let pistas: Vec<PathBuf> = raiz
        .get("pistas")
        .and_then(Value::as_array)
        .ok_or_else(|| tr!("{} no es una lista del reproductor (falta \"pistas\")", archivo.display()))?
        .iter()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect();
    let actual = raiz.get("actual").and_then(Value::as_u64).map(|actual| actual as usize).filter(|actual| *actual < pistas.len());
    Ok(ListaGuardada { pistas, actual, repetir: raiz.get("repetir").and_then(Value::as_bool).unwrap_or(false) })
}

/// Escribe una lista; con extension .m3u o .m3u8, en ese formato (solo las
/// rutas, sin la actual ni la repeticion).
pub fn escribir_lista(archivo: &Path, lista: &ListaGuardada) -> Result<(), String> {
    let escrito = if es_m3u(archivo) {
        let mut texto = String::from("#EXTM3U\n");
        for pista in &lista.pistas {
            texto.push_str(&pista.to_string_lossy());
            texto.push('\n');
        }
        archivo.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::write(archivo, texto))
    } else {
        let pistas: Vec<Value> = lista.pistas.iter().map(|ruta| Value::String(ruta.to_string_lossy().into_owned())).collect();
        escribir_json(archivo, &json!({ "pistas": pistas, "actual": lista.actual, "repetir": lista.repetir }))
    };
    escrito.map_err(|error| tr!("No se pudo guardar la lista en {}: {error}", archivo.display(), error = error))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn carpeta_temporal(nombre: &str) -> PathBuf {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-persistencia-{nombre}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&carpeta);
        carpeta
    }

    #[test]
    fn la_ficha_sobrevive_a_un_reinicio() {
        let archivo = carpeta_temporal("mezcla").join("mezcla.json");
        let audio = std::env::temp_dir().join("tema.mp3");
        let puntos = PuntosDeMezcla::nuevos(6.0, 10.0, Some(VentanaDeOverlay::nueva(80.0, 105.0).unwrap()))
            .unwrap()
            .con_recortes(0.5, 1.25)
            .unwrap()
            .como_manuales(true)
            .con_overlay_manual(true);
        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        repositorio.guardar_puntos(&audio, puntos);
        let analisis = Analisis {
            duracion: Some(215.5),
            titulo: Some("Tema".into()),
            artista: Some("Banda".into()),
            genero: Some("Rock".into()),
            album: Some("Primero".into()),
            anio: Some(1999),
        };
        repositorio.completar_con_analisis(&[(audio.clone(), analisis)]);
        let cambios = CambiosDeDatos { ranking: Some(9), titulo: Some("Otro".into()), bpm: Some(Some(123.456)), ..Default::default() };
        repositorio.cambiar_datos(&audio, cambios);
        repositorio.registrar_reproduccion(&audio, "2026-10-22T19:40:12Z".into());

        let leida = RepositorioDeCanciones::new(archivo).buscar(&audio).cloned().unwrap();
        assert_eq!(leida.puntos, puntos);
        assert_eq!(leida.titulo.as_deref(), Some("Otro"));
        assert_eq!(leida.bpm, Some(123.5));
        assert_eq!(leida.artista.as_deref(), Some("Banda"));
        assert_eq!(leida.genero.as_deref(), Some("Rock"));
        assert_eq!((leida.album.as_deref(), leida.anio), (Some("Primero"), Some(1999)), "album y anio de las etiquetas");
        assert_eq!(leida.duracion, Some(215.5));
        assert_eq!(leida.ranking, RANKING_MAXIMO);
        assert_eq!(leida.ultima_reproduccion.as_deref(), Some("2026-10-22T19:40:12Z"));
    }

    #[test]
    fn migra_el_tipo_desde_el_genero_una_sola_vez() {
        let carpeta = carpeta_temporal("migrar-tipo");
        fs::create_dir_all(&carpeta).unwrap();
        let archivo = carpeta.join("mezcla.json");
        let [aviso, seco, tema] = ["aviso.mp3", "seco.mp3", "tema.mp3"].map(|nombre| std::env::temp_dir().join(nombre));
        let mut contenido = Map::new();
        for (audio, genero) in [(&aviso, "Publicidad radial"), (&seco, "Tema seco"), (&tema, "Rock")] {
            contenido.insert(
                normalizar(audio).to_string_lossy().into_owned(),
                json!({ "intro": 0.0, "outro": 0.0, "puntosManuales": false, "genero": genero }),
            );
        }
        fs::write(&archivo, Value::Object(contenido).to_string()).unwrap();
        let repositorio = RepositorioDeCanciones::new(archivo.clone());
        let tipo = |audio: &Path| repositorio.buscar(audio).unwrap().tipo;
        assert_eq!(
            (tipo(&aviso), tipo(&seco), tipo(&tema)),
            (Some(TipoDeElemento::Publicidad), Some(TipoDeElemento::Seco), Some(TipoDeElemento::Tema))
        );
        // El genero queda como estaba y la version anterior se respalda.
        assert_eq!(repositorio.buscar(&aviso).unwrap().genero.as_deref(), Some("Publicidad radial"));
        assert!(carpeta.join("mezcla.json.antes-del-tipo").is_file());
        // Ya migrado, el tipo elegido por el usuario manda sobre el genero.
        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        repositorio.cambiar_datos(&aviso, CambiosDeDatos { tipo: Some(None), ..Default::default() });
        repositorio.cambiar_datos(&tema, CambiosDeDatos { tipo: Some(Some(TipoDeElemento::Seco)), ..Default::default() });
        let releido = RepositorioDeCanciones::new(archivo);
        assert_eq!(releido.buscar(&aviso).unwrap().tipo, None);
        assert_eq!(releido.buscar(&tema).unwrap().tipo, Some(TipoDeElemento::Seco));
        assert_eq!(releido.buscar(&tema).unwrap().genero.as_deref(), Some("Rock"));
    }

    #[test]
    fn lo_sin_clasificar_por_defecto_pasa_a_tema_una_sola_vez() {
        let carpeta = carpeta_temporal("sin-clasificar-a-tema");
        fs::create_dir_all(&carpeta).unwrap();
        let archivo = carpeta.join("mezcla.json");
        let [viejo, elegido, marcador] = ["viejo.mp3", "elegido.mp3", "Inicio de bloque fijo.marca"].map(|nombre| std::env::temp_dir().join(nombre));
        let mut contenido = Map::new();
        for (audio, tipo) in [(&viejo, Value::Null), (&elegido, json!("")), (&marcador, Value::Null)] {
            contenido.insert(normalizar(audio).to_string_lossy().into_owned(), json!({ "intro": 0.0, "outro": 0.0, "tipo": tipo }));
        }
        fs::write(&archivo, Value::Object(contenido).to_string()).unwrap();
        let repositorio = RepositorioDeCanciones::new(archivo.clone());
        let tipo = |repositorio: &RepositorioDeCanciones, audio: &Path| repositorio.buscar(audio).unwrap().tipo;
        assert_eq!(tipo(&repositorio, &viejo), Some(TipoDeElemento::Tema), "null de una version anterior");
        assert_eq!(tipo(&repositorio, &elegido), None, "sin clasificar elegido a proposito");
        assert_eq!(tipo(&repositorio, &marcador), None, "un marcador no es un tema");
        assert!(carpeta.join("mezcla.json.antes-del-tema").is_file());
        // Lo sin clasificar se guarda como "" y no vuelve a convertirse.
        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        repositorio.cambiar_datos(&viejo, CambiosDeDatos { tipo: Some(None), ..Default::default() });
        let releido = RepositorioDeCanciones::new(archivo.clone());
        assert_eq!(tipo(&releido, &viejo), None);
        let guardado: Value = serde_json::from_str(&fs::read_to_string(&archivo).unwrap()).unwrap();
        assert_eq!(guardado[normalizar(&viejo).to_string_lossy().as_ref()]["tipo"], json!(""));
        // Un archivo nuevo sin genero entra como Tema.
        let nuevo = std::env::temp_dir().join("sin-genero.mp3");
        let mut repositorio = releido;
        repositorio.completar_con_analisis(&[(nuevo.clone(), Analisis::default())]);
        assert_eq!(tipo(&repositorio, &nuevo), Some(TipoDeElemento::Tema));
    }

    #[test]
    fn un_archivo_nuevo_se_clasifica_por_sus_etiquetas() {
        let archivo = carpeta_temporal("tipo-de-etiquetas").join("mezcla.json");
        let [aviso, tema] = ["aviso-nuevo.mp3", "tema-nuevo.mp3"].map(|nombre| std::env::temp_dir().join(nombre));
        let mut repositorio = RepositorioDeCanciones::new(archivo);
        let con_genero = |genero: &str| Analisis { genero: Some(genero.into()), ..Default::default() };
        repositorio.completar_con_analisis(&[(aviso.clone(), con_genero("Comerciales")), (tema.clone(), con_genero("Rock"))]);
        assert_eq!(repositorio.buscar(&aviso).unwrap().tipo, Some(TipoDeElemento::Comercial));
        assert_eq!(repositorio.buscar(&tema).unwrap().tipo, Some(TipoDeElemento::Tema));
        // Sin genero en las etiquetas, por defecto un Tema.
        let sin_genero = std::env::temp_dir().join("sin-genero-nuevo.mp3");
        repositorio.completar_con_analisis(&[(sin_genero.clone(), Analisis { titulo: Some("Algo".into()), ..Default::default() })]);
        assert_eq!(repositorio.buscar(&sin_genero).unwrap().tipo, Some(TipoDeElemento::Tema));
    }

    #[test]
    fn el_analisis_no_pisa_datos_corregidos() {
        let archivo = carpeta_temporal("analisis").join("mezcla.json");
        let audio = std::env::temp_dir().join("tema.flac");
        let mut repositorio = RepositorioDeCanciones::new(archivo);
        repositorio.cambiar_datos(&audio, CambiosDeDatos { titulo: Some("Corregido".into()), ..Default::default() });
        repositorio.completar_con_analisis(&[(audio.clone(), Analisis { titulo: Some("Etiqueta".into()), ..Default::default() })]);
        assert_eq!(repositorio.buscar(&audio).unwrap().titulo.as_deref(), Some("Corregido"));
    }

    #[test]
    fn lee_el_formato_de_la_version_java() {
        let carpeta = carpeta_temporal("java");
        fs::create_dir_all(&carpeta).unwrap();
        let archivo = carpeta.join("mezcla.json");
        let audio = std::env::temp_dir().join("viejo.mp3");
        let mut contenido = Map::new();
        contenido.insert(normalizar(&audio).to_string_lossy().into_owned(), json!({ "intro": 6.0, "outro": 10.0 }));
        fs::write(&archivo, Value::Object(contenido).to_string()).unwrap();
        let ficha = RepositorioDeCanciones::new(archivo).buscar(&audio).cloned().unwrap();
        assert_eq!(ficha.puntos, PuntosDeMezcla::nuevos(6.0, 10.0, None).unwrap());
        assert!(ficha.origen_por_resolver, "no se sabe si los puso el usuario");
        assert_eq!(ficha.ranking, 0);
    }

    #[test]
    fn convierte_el_overlay_anterior_cuando_conoce_la_duracion() {
        let carpeta = carpeta_temporal("overlay-anterior");
        fs::create_dir_all(&carpeta).unwrap();
        let archivo = carpeta.join("mezcla.json");
        let con_duracion = std::env::temp_dir().join("con_duracion.mp3");
        let sin_duracion = std::env::temp_dir().join("sin_duracion.mp3");
        let mut contenido = Map::new();
        let clave = |audio: &Path| normalizar(audio).to_string_lossy().into_owned();
        contenido.insert(clave(&con_duracion), json!({ "intro": 0.0, "outro": 0.0, "overlay": 20.0, "duracion": 180.0 }));
        contenido.insert(clave(&sin_duracion), json!({ "intro": 0.0, "outro": 0.0, "overlay": 15.0 }));
        fs::write(&archivo, Value::Object(contenido).to_string()).unwrap();

        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        let esperada = VentanaDeOverlay::nueva(160.0, 180.0).unwrap();
        assert_eq!(repositorio.puntos(&con_duracion).unwrap().overlay(), Some(esperada));
        assert_eq!(repositorio.puntos(&sin_duracion).unwrap().overlay(), None);

        let analisis = Analisis { duracion: Some(100.0), ..Default::default() };
        repositorio.completar_con_analisis(&[(sin_duracion.clone(), analisis)]);
        let convertida = VentanaDeOverlay::nueva(85.0, 100.0).unwrap();
        assert_eq!(repositorio.puntos(&sin_duracion).unwrap().overlay(), Some(convertida));
        let releida = RepositorioDeCanciones::new(archivo);
        assert_eq!(releida.puntos(&sin_duracion).unwrap().overlay(), Some(convertida));
    }

    #[test]
    fn descatalogar_borra_la_ficha_del_json() {
        let archivo = carpeta_temporal("descatalogar").join("mezcla.json");
        let audio = PathBuf::from("musica/descatalogado.mp3");
        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        repositorio.cambiar_datos(&audio, CambiosDeDatos { titulo: Some("Tema".into()), ..Default::default() });
        assert!(RepositorioDeCanciones::new(archivo.clone()).buscar(&audio).is_some());
        assert!(repositorio.olvidar(&audio));
        assert!(!repositorio.olvidar(&audio));
        assert!(RepositorioDeCanciones::new(archivo).buscar(&audio).is_none());
    }

    fn emision_de_prueba() -> EmisionDeComercial {
        EmisionDeComercial {
            inicio: "2026-11-02 14:05:00".into(),
            fin: "2026-11-02 14:05:30".into(),
            segundos: 29.96,
            duracion: Some(30.0),
            completa: true,
            titulo: "Promo; verano".into(),
            artista: "Tienda \"Sol\"".into(),
            genero: "Comercial".into(),
            archivo: PathBuf::from("tandas/promo.mp3"),
            tanda: "Tanda A 14:00".into(),
            orden: "Sol - Noviembre".into(),
            tipo: "Comercial".into(),
        }
    }

    #[test]
    fn el_registro_de_comerciales_anota_por_mes_y_se_relee() {
        let carpeta = carpeta_temporal("comerciales").join("comerciales");
        let registro = RegistroDeComerciales::new(carpeta.clone(), None, MESES_DE_REGISTRO);
        let emision = emision_de_prueba();
        registro.anotar(&emision);
        let cortada = EmisionDeComercial { completa: false, segundos: 12.0, tanda: String::new(), orden: String::new(), ..emision.clone() };
        registro.anotar(&cortada);
        let texto = fs::read_to_string(carpeta.join("2026-11.csv")).unwrap();
        let lineas: Vec<&str> = texto.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lineas.len(), 3);
        assert_eq!(lineas[0], RegistroDeComerciales::ENCABEZADO);
        assert_eq!(
            lineas[1],
            format!(
                "2026-11-02 14:05:00;2026-11-02 14:05:30;30,0;30,0;Si;\"Promo; verano\";\"Tienda \"\"Sol\"\"\";Comercial;{};Tanda A 14:00;Sol - Noviembre;Comercial",
                PathBuf::from("tandas/promo.mp3").display()
            )
        );
        assert!(lineas[2].contains(";12,0;30,0;No;"));
        let leidas = registro.emisiones_entre(jiff::civil::date(2026, 11, 1), jiff::civil::date(2026, 11, 2)).unwrap();
        let redondeada = EmisionDeComercial { segundos: 30.0, ..emision };
        assert_eq!(leidas, [redondeada, cortada]);
        assert!(registro.emisiones_entre(jiff::civil::date(2026, 11, 3), jiff::civil::date(2026, 11, 30)).unwrap().is_empty());
    }

    #[test]
    fn el_registro_conserva_solo_los_ultimos_meses() {
        let carpeta = carpeta_temporal("comerciales-rotacion").join("comerciales");
        fs::create_dir_all(&carpeta).unwrap();
        for mes in ["2026-07", "2026-08", "2026-09", "2026-10", "2026-11"] {
            fs::write(carpeta.join(format!("{mes}.csv")), "x").unwrap();
        }
        fs::write(carpeta.join("notas.csv"), "no es de un mes").unwrap();
        let registro = RegistroDeComerciales::new(carpeta.clone(), None, 3);
        registro.rotar(jiff::civil::date(2026, 11, 7));
        let mut quedan: Vec<String> = fs::read_dir(&carpeta).unwrap().flatten().map(|entrada| entrada.file_name().to_string_lossy().into_owned()).collect();
        quedan.sort();
        assert_eq!(quedan, ["2026-09.csv", "2026-10.csv", "2026-11.csv", "notas.csv"]);
        // A fin de anio cuenta el anio anterior.
        registro.rotar(jiff::civil::date(2027, 1, 15));
        assert!(!carpeta.join("2026-10.csv").exists() && carpeta.join("2026-11.csv").exists());
    }

    #[test]
    fn el_registro_anterior_se_reparte_por_mes_con_fechas_iso() {
        let carpeta = carpeta_temporal("comerciales-iso");
        fs::create_dir_all(&carpeta).unwrap();
        let anterior = carpeta.join("comerciales.csv");
        let original = "\u{feff}Inicio;Fin;Segundos emitidos;Duracion;Completo;Titulo;Artista;Genero;Archivo\r\n\
            02/11/2026 22:56:43;02/11/2026 22:57:29;46,5;47,5;Si;Diferente;Corto;Comercial;D:\\a.mp3\r\n\
            2026-10-30 10:00:00;2026-10-30 10:00:30;30,0;30,0;Si;Ya nueva;;Publicidad;D:\\b.mp3\r\n";
        fs::write(&anterior, original).unwrap();
        let registro = RegistroDeComerciales::new(carpeta.join("comerciales"), Some(&anterior), 1_000);
        let noviembre = fs::read_to_string(carpeta.join("comerciales").join("2026-11.csv")).unwrap();
        let lineas: Vec<&str> = noviembre.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lineas, [RegistroDeComerciales::ENCABEZADO, "2026-11-02 22:56:43;2026-11-02 22:57:29;46,5;47,5;Si;Diferente;Corto;Comercial;D:\\a.mp3"]);
        let octubre = fs::read_to_string(carpeta.join("comerciales").join("2026-10.csv")).unwrap();
        assert!(octubre.ends_with("2026-10-30 10:00:00;2026-10-30 10:00:30;30,0;30,0;Si;Ya nueva;;Publicidad;D:\\b.mp3\r\n"));
        // El original queda intacto aparte, y la segunda vez no hay nada que hacer.
        assert!(!anterior.exists());
        assert_eq!(fs::read_to_string(agregar_sufijo(&anterior, SUFIJO_DEL_REGISTRO_MIGRADO)).unwrap(), original);
        let leidas = registro.emisiones_entre(jiff::civil::date(2026, 10, 1), jiff::civil::date(2026, 11, 30)).unwrap();
        assert_eq!(leidas.len(), 2);
        assert_eq!(leidas[0].tanda, "", "las lineas anteriores no tienen tanda");
        RegistroDeComerciales::new(carpeta.join("comerciales"), Some(&anterior), 1_000);
        assert_eq!(fs::read_to_string(carpeta.join("comerciales").join("2026-11.csv")).unwrap(), noviembre);
    }

    #[test]
    fn las_tandas_se_guardan_y_se_releen() {
        let archivo = carpeta_temporal("tandas").join("tandas.json");
        let mut repositorio = RepositorioDeTandas::new(archivo.clone());
        let recurrencia = Recurrencia::nueva(parsear_hora("10:30").unwrap(), Some(30), Some(parsear_hora("20:00").unwrap()), [true, true, true, true, true, false, false]).unwrap();
        let tanda = TandaProgramada {
            nombre: "Tanda Mañana".into(),
            archivos: vec![PathBuf::from("spots/a.mp3"), PathBuf::from("spots/b.mp3")],
            recurrencia,
            antes: 180,
            despues: 120,
            modo: ModoDeEntrada::Exacta,
            prioridad: 8,
            duracion_maxima: Some(180),
            activa: true,
            ultima_salida: None,
        };
        repositorio.guardar_tanda(tanda.clone());
        let salida = jiff::civil::date(2026, 11, 6).at(10, 30, 0, 0);
        repositorio.anotar_salida("tanda manana", salida);
        let releido = RepositorioDeTandas::new(archivo.clone());
        assert_eq!(releido.tandas(), [TandaProgramada { ultima_salida: Some(salida), ..tanda.clone() }]);
        // Mismo nombre: reemplaza.
        let mut releido = releido;
        releido.guardar_tanda(TandaProgramada { activa: false, ..tanda });
        assert_eq!(releido.tandas().len(), 1);
        assert!(!RepositorioDeTandas::new(archivo.clone()).tandas()[0].activa);
        assert!(releido.quitar_tanda("TANDA MAÑANA").is_some());
        assert!(RepositorioDeTandas::new(archivo).tandas().is_empty());
    }

    #[test]
    fn los_anunciantes_se_guardan_ordenados_y_se_releen() {
        let carpeta = carpeta_temporal("anunciantes");
        let archivo = carpeta.join("anunciantes.json");
        let mut repositorio = RepositorioDeAnunciantes::new(archivo.clone());
        let sol = Anunciante {
            nombre: "Supermercado Sol".into(),
            razon_social: "Sol S.A.".into(),
            cuit: "30-71234567-1".into(),
            rubro: "Supermercados".into(),
            contacto: "Ana".into(),
            telefono: "011 4555-1234".into(),
            correo: "ventas@sol.com.ar".into(),
            direccion: "Calle 1".into(),
            agencia: "Agencia X".into(),
            notas: "Paga a 30 dias".into(),
            activo: true,
        };
        let almacen = Anunciante { nombre: "Almacen Luna".into(), activo: false, ..Anunciante::default() };
        repositorio.guardar_anunciante(sol.clone());
        repositorio.guardar_anunciante(almacen.clone());
        assert_eq!(RepositorioDeAnunciantes::new(archivo.clone()).anunciantes(), [almacen.clone(), sol.clone()]);
        let renovado = Anunciante { telefono: "011 4000-0000".into(), ..sol.clone() };
        repositorio.guardar_anunciante(Anunciante { nombre: "SUPERMERCADO SOL".into(), ..renovado.clone() });
        assert_eq!(repositorio.anunciantes().len(), 2, "mismo nombre: reemplaza");
        assert!(repositorio.quitar_anunciante("almacen luna").is_some());
        assert_eq!(RepositorioDeAnunciantes::new(archivo).anunciantes()[0].telefono, "011 4000-0000");
    }

    #[test]
    fn las_ordenes_y_las_reposiciones_se_guardan_y_se_releen() {
        let carpeta = carpeta_temporal("ordenes");
        let archivo = carpeta.join("ordenes.json");
        let mut repositorio = RepositorioDeOrdenes::new(archivo.clone());
        let orden = OrdenDePublicidad {
            nombre: "Sol - Noviembre".into(),
            anunciante: "Supermercado Sol".into(),
            rubro: "Supermercados".into(),
            desde: jiff::civil::date(2026, 11, 1),
            hasta: jiff::civil::date(2026, 11, 30),
            dias: [true, true, true, true, true, false, false],
            franja: (parsear_hora("08:00").unwrap(), parsear_hora("20:00").unwrap()),
            frecuencia: Frecuencia::EnHorarios(vec![parsear_hora("09:00").unwrap(), parsear_hora("13:30").unwrap()]),
            avisos: vec![PathBuf::from("spots/sol-1.mp3")],
            tandas: vec!["Tanda A".into()],
            prioridad: 7,
            activa: true,
        };
        repositorio.guardar_orden(orden.clone());
        let otra = OrdenDePublicidad { nombre: "Ruta".into(), frecuencia: Frecuencia::VecesPorDia(3), tandas: Vec::new(), ..orden.clone() };
        repositorio.guardar_orden(otra.clone());
        assert_eq!(RepositorioDeOrdenes::new(archivo.clone()).ordenes(), [orden.clone(), otra]);
        assert!(repositorio.quitar_orden("ruta").is_some());
        assert_eq!(RepositorioDeOrdenes::new(archivo).ordenes(), [orden]);

        let pautas = RepositorioDePautas::new(carpeta.join("pauta"), MESES_DE_REGISTRO);
        let fecha = jiff::civil::date(2026, 11, 6);
        let reponer = Reposicion { orden: "Sol - Noviembre".into(), aviso: PathBuf::from("spots/sol-1.mp3"), desde: fecha.at(12, 0, 0, 0) };
        pautas.guardar_pauta(fecha, &PautaDelDia::default(), std::slice::from_ref(&reponer));
        assert_eq!(pautas.reposiciones(fecha), [reponer]);
        assert!(pautas.reposiciones(jiff::civil::date(2026, 11, 7)).is_empty());
    }

    #[test]
    fn los_campos_csv_respetan_las_comillas() {
        assert_eq!(campos_csv("a;\"b;c\";\"d \"\"e\"\"\";"), ["a", "b;c", "d \"e\"", ""]);
    }

    #[test]
    fn reubicar_conserva_la_ficha_los_efectos_fijados_y_recuerda_el_cambio() {
        let carpeta = carpeta_temporal("reubicar");
        fs::create_dir_all(&carpeta).unwrap();
        let archivo = carpeta.join("mezcla.json");
        let (anterior, nueva) = (carpeta.join("viejo/tema.mp3"), carpeta.join("nuevo/tema.mp3"));
        let (efecto_anterior, efecto_nuevo) = (carpeta.join("viejo/aplauso.wav"), carpeta.join("nuevo/aplauso.wav"));
        let otro = carpeta.join("otro.mp3");
        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        repositorio.cambiar_datos(&anterior, CambiosDeDatos { genero: Some("Rock".into()), ranking: Some(4), ..Default::default() });
        repositorio.registrar_reproduccion(&anterior, "2026-11-01T10:00:00Z".into());
        repositorio.cambiar_datos(&otro, CambiosDeDatos { efecto_fijado: Some(Some(efecto_anterior.clone())), ..Default::default() });
        repositorio.cambiar_datos(&efecto_anterior, CambiosDeDatos { genero: Some("Efecto".into()), ..Default::default() });
        // La ruta nueva ya tenia ficha: se agrego desde su nueva ubicacion.
        repositorio.cambiar_datos(&nueva, CambiosDeDatos { titulo: Some("Tema".into()), ..Default::default() });
        repositorio.registrar_reproduccion(&nueva, "2026-11-03T10:00:00Z".into());
        assert_eq!(repositorio.sin_archivo().len(), 4, "ningun archivo existe");

        let cambios = vec![(anterior.clone(), nueva.clone()), (efecto_anterior.clone(), efecto_nuevo.clone())];
        assert_eq!(repositorio.reubicar(&cambios), 2);
        let leido = RepositorioDeCanciones::new(archivo);
        assert!(leido.buscar(&anterior).is_none());
        let ficha = leido.buscar(&nueva).unwrap();
        assert_eq!(ficha.genero.as_deref(), Some("Rock"));
        assert_eq!(ficha.ranking, 4);
        assert_eq!(ficha.titulo.as_deref(), Some("Tema"), "la nueva completa lo que falta");
        assert_eq!(ficha.ultima_reproduccion.as_deref(), Some("2026-11-03T10:00:00Z"), "vale la mas reciente");
        assert_eq!(leido.buscar(&otro).unwrap().efecto_fijado.as_deref(), Some(normalizar(&efecto_nuevo).as_path()));
        // Las listas guardadas con la ruta anterior se abren con la nueva.
        assert_eq!(leido.ruta_actual(&anterior), normalizar(&nueva));
        assert_eq!(leido.ruta_actual(&otro), otro, "sin reubicar queda igual");

        let mut leido = leido;
        assert_eq!(leido.olvidar_varios(&[otro.clone(), carpeta.join("no-estaba.mp3")]), 1);
        assert!(leido.buscar(&otro).is_none());
    }

    #[test]
    fn reubicar_dos_veces_sigue_la_cadena() {
        let carpeta = carpeta_temporal("reubicar-dos-veces");
        fs::create_dir_all(&carpeta).unwrap();
        let mut repositorio = RepositorioDeCanciones::new(carpeta.join("mezcla.json"));
        let (a, b, c) = (carpeta.join("a/tema.mp3"), carpeta.join("b/tema.mp3"), carpeta.join("c/tema.mp3"));
        repositorio.cambiar_datos(&a, CambiosDeDatos { ranking: Some(3), ..Default::default() });
        repositorio.reubicar(&[(a.clone(), b.clone())]);
        repositorio.reubicar(&[(b.clone(), c.clone())]);
        assert_eq!(repositorio.ruta_actual(&a), normalizar(&c));
        assert_eq!(repositorio.ruta_actual(&b), normalizar(&c));
        assert_eq!(repositorio.buscar(&c).unwrap().ranking, 3);
    }

    #[test]
    fn la_botonera_reubica_sus_botones() {
        let carpeta = carpeta_temporal("botonera-reubicar");
        let mut botonera = RepositorioDeBotonera::new(carpeta.join("botonera.json"));
        let (anterior, nueva) = (carpeta.join("viejo/bocina.wav"), carpeta.join("nuevo/bocina.wav"));
        botonera.asignar(0, 3, &anterior).unwrap();
        assert_eq!(botonera.reubicar(&[(anterior, nueva.clone())]), 1);
        assert_eq!(botonera.archivo(0, 3), Some(normalizar(&nueva).as_path()));
    }

    #[test]
    fn solo_se_convierten_fechas_dia_mes_anio() {
        assert_eq!(fecha_dia_mes_anio_a_iso("02/11/2026 14:05:33").as_deref(), Some("2026-11-02 14:05:33"));
        for campo in ["2026-11-02 14:05:33", "Inicio", "2/11/2026 14:05:33", "02/11/26 14:05:33", "aa/bb/cccc 14:05:33"] {
            assert_eq!(fecha_dia_mes_anio_a_iso(campo), None, "{campo}");
        }
    }

    #[test]
    fn un_json_danado_se_respalda() {
        let carpeta = carpeta_temporal("danado");
        fs::create_dir_all(&carpeta).unwrap();
        let archivo = carpeta.join("mezcla.json");
        fs::write(&archivo, "{ no es json").unwrap();
        let repositorio = RepositorioDeCanciones::new(archivo.clone());
        assert!(repositorio.fichas.is_empty());
        assert!(carpeta.join("mezcla.json.danado").is_file());
    }

    #[test]
    fn formatea_fechas_iso8601() {
        assert_eq!(formatear_iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(formatear_iso8601(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(formatear_iso8601(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn el_fader_se_recuerda_y_se_acota() {
        let archivo = carpeta_temporal("preferencias").join("preferencias.json");
        assert_eq!(RepositorioDePreferencias::new(archivo.clone()).fader(), FADER_POR_DEFECTO);
        RepositorioDePreferencias::new(archivo.clone()).guardar_fader(42);
        assert_eq!(RepositorioDePreferencias::new(archivo).fader(), FADER_MAXIMO);
    }

    #[test]
    fn las_opciones_de_mezcla_se_recuerdan_sin_perder_el_fader() {
        let archivo = carpeta_temporal("opciones-de-mezcla").join("preferencias.json");
        let mut preferencias = RepositorioDePreferencias::new(archivo.clone());
        assert!(!preferencias.crossfade_inteligente() && !preferencias.nivelar_volumen());
        preferencias.guardar_fader(3);
        preferencias.guardar_crossfade_inteligente(true);
        preferencias.guardar_nivelar_volumen(true);
        preferencias.guardar_desplazar(true);
        preferencias.guardar_modo_de_operacion(ModoDeOperacion::AlAire);
        assert_eq!(preferencias.objetivo_de_sonoridad(), ObjetivoDeSonoridad::Menos11, "por defecto");
        preferencias.guardar_objetivo_de_sonoridad(ObjetivoDeSonoridad::Menos8);
        let leidas = RepositorioDePreferencias::new(archivo);
        assert!(leidas.crossfade_inteligente() && leidas.nivelar_volumen() && leidas.desplazar());
        assert_eq!(leidas.modo_de_operacion(), ModoDeOperacion::AlAire);
        assert_eq!(leidas.objetivo_de_sonoridad(), ObjetivoDeSonoridad::Menos8);
        assert_eq!(leidas.fader(), 3);
    }

    #[test]
    fn la_botonera_sobrevive_a_un_reinicio() {
        let archivo = carpeta_temporal("botonera").join("botonera.json");
        let efecto = std::env::temp_dir().join("aplauso.wav");
        let otro = std::env::temp_dir().join("bocina.wav");
        let mut repositorio = RepositorioDeBotonera::new(archivo.clone());
        repositorio.asignar(0, 3, &efecto).unwrap();
        assert_eq!(repositorio.agregar_hoja(), 1);
        repositorio.asignar(1, 9, &otro).unwrap();
        repositorio.guardar_modo(ModoDeOverlay::Trabada);
        let ajustes = AjustesDeEfecto {
            atenuar: true,
            nivel_de_musica: 0.4,
            inteligente: false,
            volumen_db: -2.5,
            anticipacion: 0.3,
            recuperacion: 1.2,
            separacion_db: 18.0,
        };
        repositorio.guardar_ajustes_de_efecto(ajustes);

        let leido = RepositorioDeBotonera::new(archivo);
        assert_eq!(leido.cantidad_de_hojas(), 2);
        assert_eq!(leido.hoja_actual(), 1);
        assert_eq!(leido.archivo(0, 3), Some(normalizar(&efecto).as_path()));
        assert_eq!(leido.archivo_del_boton(9), Some(normalizar(&otro).as_path()));
        assert_eq!(leido.archivo(0, 0), None);
        assert_eq!(leido.efectos().count(), 2);
        assert_eq!(leido.modo(), ModoDeOverlay::Trabada);
        assert_eq!(leido.ajustes_de_efecto(), ajustes);
    }

    #[test]
    fn un_efecto_no_se_repite_dentro_de_una_hoja() {
        let archivo = carpeta_temporal("repetidos").join("botonera.json");
        let efecto = std::env::temp_dir().join("aplauso.wav");
        let mut repositorio = RepositorioDeBotonera::new(archivo);
        repositorio.asignar(0, 2, &efecto).unwrap();
        let error = repositorio.asignar(0, 5, &efecto).unwrap_err();
        assert!(error.contains("boton 2"), "{error}");
        assert_eq!(repositorio.archivo(0, 5), None);
        // Reasignarlo al mismo boton no es repetirlo.
        repositorio.asignar(0, 2, &efecto).unwrap();
        // En otra hoja si puede estar.
        repositorio.agregar_hoja();
        repositorio.asignar(1, 5, &efecto).unwrap();
    }

    #[test]
    fn siempre_queda_una_hoja() {
        let archivo = carpeta_temporal("hojas").join("botonera.json");
        let mut repositorio = RepositorioDeBotonera::new(archivo);
        assert!(repositorio.quitar_hoja(0).is_err());
        repositorio.agregar_hoja();
        repositorio.agregar_hoja();
        repositorio.mostrar_hoja(2);
        repositorio.quitar_hoja(2).unwrap();
        assert_eq!((repositorio.cantidad_de_hojas(), repositorio.hoja_actual()), (2, 1));
        repositorio.quitar_hoja(0).unwrap();
        assert_eq!((repositorio.cantidad_de_hojas(), repositorio.hoja_actual()), (1, 0));
        assert!(repositorio.quitar_hoja(0).is_err());
    }

    #[test]
    fn las_hojas_recuerdan_su_nombre_y_lo_acompanan_al_quitar_otras() {
        let archivo = carpeta_temporal("nombres-de-hojas").join("botonera.json");
        let mut repositorio = RepositorioDeBotonera::new(archivo.clone());
        assert_eq!(repositorio.nombre_de_hoja(0), None);
        repositorio.agregar_hoja();
        repositorio.agregar_hoja();
        repositorio.renombrar_hoja(1, "  Offs en español ").unwrap();
        repositorio.renombrar_hoja(2, "Offs in English").unwrap();
        assert!(repositorio.renombrar_hoja(3, "No existe").is_err());
        assert!(repositorio.renombrar_hoja(0, &"x".repeat(LARGO_MAXIMO_DEL_NOMBRE_DE_HOJA + 1)).is_err());
        // Se guardan y se leen al volver a abrir.
        let leido = RepositorioDeBotonera::new(archivo.clone());
        assert_eq!((leido.nombre_de_hoja(0), leido.nombre_de_hoja(1)), (None, Some("Offs en español")));
        // Al quitar una hoja, los nombres siguen a sus hojas.
        repositorio.quitar_hoja(1).unwrap();
        assert_eq!(repositorio.nombre_de_hoja(1), Some("Offs in English"));
        // Vacio vuelve al nombre de siempre.
        repositorio.renombrar_hoja(1, "   ").unwrap();
        assert_eq!(RepositorioDeBotonera::new(archivo).nombre_de_hoja(1), None);
    }

    #[test]
    fn lee_la_botonera_de_una_sola_hoja() {
        let carpeta = carpeta_temporal("botonera-anterior");
        fs::create_dir_all(&carpeta).unwrap();
        let archivo = carpeta.join("botonera.json");
        fs::write(&archivo, r#"{"modo":"LIBRE","botones":["C:/a.wav",null,"C:/b.wav"]}"#).unwrap();
        let repositorio = RepositorioDeBotonera::new(archivo);
        assert_eq!(repositorio.cantidad_de_hojas(), 1);
        assert_eq!(repositorio.archivo(0, 2), Some(Path::new("C:/b.wav")));
    }

    #[test]
    fn la_lista_de_versiones_anteriores_se_muda_a_su_carpeta() {
        let carpeta = carpeta_temporal("mudanza");
        fs::create_dir_all(&carpeta).unwrap();
        let anterior = carpeta.join("lista.json");
        let nuevo = carpeta.join("listas").join("lista.json");
        fs::write(&anterior, "{}").unwrap();
        mudar_si_falta(&anterior, &nuevo);
        assert!(!anterior.exists() && nuevo.is_file());
        // Si ya hay una en el lugar nuevo, la anterior no la pisa.
        fs::write(&anterior, "viejo").unwrap();
        mudar_si_falta(&anterior, &nuevo);
        assert_eq!(fs::read_to_string(&nuevo).unwrap(), "{}");
        let _ = fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn las_listas_con_nombre_se_leen_y_escriben_en_json_y_m3u() {
        let carpeta = carpeta_temporal("listas-con-nombre");
        let lista = ListaGuardada {
            pistas: vec![carpeta.join("a.mp3"), carpeta.join("spot.mp3"), carpeta.join("spot.mp3")],
            actual: Some(2),
            repetir: true,
        };
        let json = carpeta.join("tarde.json");
        escribir_lista(&json, &lista).unwrap();
        assert_eq!(leer_lista(&json).unwrap(), lista, "conserva las copias, la actual y la repeticion");
        let m3u = carpeta.join("tarde.m3u8");
        escribir_lista(&m3u, &lista).unwrap();
        assert_eq!(leer_lista(&m3u).unwrap().pistas, lista.pistas);
        // Rutas relativas y comentarios de otros programas.
        fs::write(carpeta.join("otra.m3u"), "#EXTM3U\n#EXTINF:1,Tema\nsub/b.flac\n\n").unwrap();
        assert_eq!(leer_lista(&carpeta.join("otra.m3u")).unwrap().pistas, vec![carpeta.join("sub/b.flac")]);
        fs::write(carpeta.join("mala.json"), "{}").unwrap();
        assert!(leer_lista(&carpeta.join("mala.json")).is_err());
        assert!(leer_lista(&carpeta.join("no-existe.json")).is_err());
        let _ = fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn una_m3u_de_otro_programa_se_lee_aunque_no_sea_utf8() {
        let carpeta = carpeta_temporal("m3u-ansi");
        fs::create_dir_all(&carpeta).unwrap();
        // Winamp y el Reproductor de Windows guardan las .m3u en Windows-1252.
        let mut ansi = b"#EXTM3U\r\n#EXTINF:215,Canci".to_vec();
        ansi.extend([0xF3]);
        ansi.extend(b"n\r\nD:\\M\xfasica\\Canci\xf3n \x96 en vivo.mp3\r\n");
        fs::write(carpeta.join("winamp.m3u"), ansi).unwrap();
        // En Windows la ruta es absoluta; en Linux, relativa a la lista: join
        // da lo mismo que la lectura en los dos sistemas.
        assert_eq!(leer_lista(&carpeta.join("winamp.m3u")).unwrap().pistas, vec![carpeta.join("D:\\Música\\Canción – en vivo.mp3")]);
        // Con la marca BOM, es UTF-8 aunque la extension sea .m3u.
        fs::write(carpeta.join("bom.m3u"), "\u{feff}#EXTM3U\nMúsica/Canción.mp3\n").unwrap();
        assert_eq!(leer_lista(&carpeta.join("bom.m3u")).unwrap().pistas, vec![carpeta.join("Música/Canción.mp3")]);
        let _ = fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn una_m3u_con_direcciones_file_se_lee_como_rutas() {
        let carpeta = carpeta_temporal("m3u-uri");
        fs::create_dir_all(&carpeta).unwrap();
        // Asi guarda VLC: direcciones file:// con los caracteres codificados.
        let texto = "#EXTM3U\nfile:///D:/M%C3%BAsica/Tema%20uno%20%28remix%29.mp3\nfile:///home/radio/b%C3%A1sico.flac\nfile://servidor/compartida/tema.mp3\n";
        fs::write(carpeta.join("vlc.m3u8"), texto).unwrap();
        let pistas = leer_lista(&carpeta.join("vlc.m3u8")).unwrap().pistas;
        let esperadas = if cfg!(windows) {
            ["D:/Música/Tema uno (remix).mp3", "/home/radio/básico.flac", "//servidor/compartida/tema.mp3"]
        } else {
            ["/D:/Música/Tema uno (remix).mp3", "/home/radio/básico.flac", "//servidor/compartida/tema.mp3"]
        };
        assert_eq!(pistas, esperadas.map(PathBuf::from));
        let _ = fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn la_lista_conserva_rutas_con_acentos_espacios_y_comillas() {
        let carpeta = carpeta_temporal("lista-rara");
        let lista = ListaGuardada {
            pistas: vec![carpeta.join("Canción \"en vivo\" #1.mp3"), carpeta.join("a b").join("ñandú;tema.flac"), carpeta.join("日本.ogg")],
            actual: Some(1),
            repetir: false,
        };
        for nombre in ["rara.json", "rara.m3u8", "rara.m3u"] {
            escribir_lista(&carpeta.join(nombre), &lista).unwrap();
            assert_eq!(leer_lista(&carpeta.join(nombre)).unwrap().pistas, lista.pistas, "{nombre}");
        }
        let _ = fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn la_ultima_reproduccion_se_corrige_o_se_borra_a_mano() {
        let archivo = carpeta_temporal("sono").join("mezcla.json");
        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        let audio = Path::new("C:/musica/a.mp3");
        repositorio.registrar_reproduccion(audio, "2026-11-23T11:13:05Z".into());
        // Sin pedirlo, no cambia.
        repositorio.cambiar_datos(audio, CambiosDeDatos { titulo: Some("A".into()), ..Default::default() });
        assert_eq!(repositorio.buscar(audio).unwrap().ultima_reproduccion.as_deref(), Some("2026-11-23T11:13:05Z"));
        repositorio.cambiar_datos(audio, CambiosDeDatos { ultima_reproduccion: Some(Some("2026-11-20T09:00:00Z".into())), ..Default::default() });
        let leida = RepositorioDeCanciones::new(archivo.clone());
        assert_eq!(leida.buscar(audio).unwrap().ultima_reproduccion.as_deref(), Some("2026-11-20T09:00:00Z"));
        repositorio.cambiar_datos(audio, CambiosDeDatos { ultima_reproduccion: Some(None), ..Default::default() });
        assert_eq!(RepositorioDeCanciones::new(archivo).buscar(audio).unwrap().ultima_reproduccion, None);
    }

    #[test]
    fn la_ficha_guarda_album_anio_y_efecto_fijado() {
        let archivo = carpeta_temporal("album").join("mezcla.json");
        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        let cambios = CambiosDeDatos {
            album: Some(" Clasicos ".into()),
            anio: Some(Some(1987)),
            efecto_fijado: Some(Some(PathBuf::from("C:/efectos/id.wav"))),
            ..Default::default()
        };
        repositorio.cambiar_datos(Path::new("C:/musica/a.mp3"), cambios);
        let leida = RepositorioDeCanciones::new(archivo);
        let ficha = leida.buscar(Path::new("C:/musica/a.mp3")).unwrap();
        assert_eq!(ficha.album.as_deref(), Some("Clasicos"));
        assert_eq!(ficha.anio, Some(1987));
        assert_eq!(ficha.efecto_fijado.as_deref(), Some(Path::new("C:/efectos/id.wav")));
    }

    #[test]
    fn la_ficha_guarda_la_nota_y_un_texto_vacio_la_borra() {
        let archivo = carpeta_temporal("nota").join("mezcla.json");
        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        let audio = Path::new("C:/efectos/risas.wav");
        let nota = |texto: &str| CambiosDeDatos { nota: Some(texto.into()), ..Default::default() };
        repositorio.cambiar_datos(audio, nota("  Usar solo en el programa de la tarde  "));
        let leida = RepositorioDeCanciones::new(archivo.clone());
        assert_eq!(leida.buscar(audio).unwrap().nota.as_deref(), Some("Usar solo en el programa de la tarde"));
        repositorio.cambiar_datos(audio, nota("   "));
        let leida = RepositorioDeCanciones::new(archivo);
        assert_eq!(leida.buscar(audio).and_then(|ficha| ficha.nota.clone()), None, "un texto vacio borra la nota");
    }

    #[test]
    fn el_catalogo_recuerda_lo_corregido_a_mano_y_lo_aplicado_de_una_fuente() {
        let archivo = carpeta_temporal("corregidos").join("mezcla.json");
        let tema = PathBuf::from("C:/musica/amiga.mp3");
        let mut repositorio = RepositorioDeCanciones::new(archivo.clone());
        repositorio.completar_con_analisis(&[(tema.clone(), Analisis { titulo: Some("amiga mia".into()), ..Default::default() })]);
        assert!(repositorio.buscar(&tema).unwrap().corregidos.claves().is_empty(), "lo leido de las etiquetas no es corregido");
        // Corregir a mano marca solo lo que cambio.
        repositorio.cambiar_datos(&tema, CambiosDeDatos { titulo: Some("Amiga mía".into()), artista: Some(String::new()), ..Default::default() });
        assert_eq!(repositorio.buscar(&tema).unwrap().corregidos.claves(), ["titulo"]);
        // Lo que se aplica de internet cuenta como corregido; de las etiquetas, no.
        let propuesta = |campo, nuevo: &str| Propuesta {
            archivo: tema.clone(),
            campo,
            actual: None,
            nuevo: nuevo.into(),
            corregido: false,
            sin_registro: false,
            elegida: true,
        };
        let aplicados = repositorio.aplicar_propuestas(
            &[propuesta(CampoDescriptivo::Album, "Más"), propuesta(CampoDescriptivo::Anio, "1997")],
            OrigenDeLosDatos::Internet,
        );
        assert_eq!(aplicados, 2);
        repositorio.aplicar_propuestas(&[propuesta(CampoDescriptivo::Titulo, "Amiga Mia")], OrigenDeLosDatos::Etiquetas);
        let releido = RepositorioDeCanciones::new(archivo.clone());
        let ficha = releido.buscar(&tema).unwrap();
        assert_eq!((ficha.titulo.as_deref(), ficha.album.as_deref(), ficha.anio), (Some("Amiga Mia"), Some("Más"), Some(1997)));
        assert_eq!(ficha.corregidos.claves(), ["album", "anio"]);
        // Una ficha de una version anterior no registra las correcciones.
        fs::write(&archivo, r#"{"C:/musica/viejo.mp3": {"intro": 0, "outro": 0, "tipo": "Tema", "titulo": "Viejo"}}"#).unwrap();
        let viejo = Path::new("C:/musica/viejo.mp3");
        let mut anterior = RepositorioDeCanciones::new(archivo.clone());
        assert!(anterior.buscar(viejo).unwrap().corregidos.es_anterior());
        // Lo que se corrige despues se recuerda, sin olvidar que el resto no se sabe.
        anterior.cambiar_datos(viejo, CambiosDeDatos { album: Some("Disco".into()), ..Default::default() });
        let corregidos = RepositorioDeCanciones::new(archivo).buscar(viejo).unwrap().corregidos;
        assert!(corregidos.es_anterior() && corregidos.claves() == ["album"]);
    }

    #[test]
    fn las_preferencias_recuerdan_la_ultima_lista_y_el_ultimo_tema() {
        let archivo = carpeta_temporal("ultima-lista").join("preferencias.json");
        let mut preferencias = RepositorioDePreferencias::new(archivo.clone());
        assert!(preferencias.ultima_lista().is_none() && preferencias.ultimo_tema().is_none());
        preferencias.guardar_ultima_lista(Some(Path::new("C:/listas/tarde.json")));
        let tema = UltimoTema { posicion: 4, archivo: PathBuf::from("C:/musica/a.mp3") };
        preferencias.guardar_ultimo_tema(Some(tema.clone()));
        let leidas = RepositorioDePreferencias::new(archivo.clone());
        assert_eq!(leidas.ultima_lista(), Some(Path::new("C:/listas/tarde.json")));
        assert_eq!(leidas.ultimo_tema(), Some(&tema));
        assert!(leidas.ventana_del_editor().is_none());
        let mut preferencias = leidas;
        let geometria = GeometriaDeVentana { x: -1200, y: 40, ancho: 1180, alto: 640 };
        preferencias.guardar_ventana_del_editor(geometria);
        assert!(!RepositorioDePreferencias::new(archivo.clone()).editor_centrado());
        let mut preferencias = RepositorioDePreferencias::new(archivo.clone());
        preferencias.guardar_editor_centrado(true);
        assert!(RepositorioDePreferencias::new(archivo.clone()).editor_centrado());
        assert!(!RepositorioDePreferencias::new(archivo.clone()).ocultar_textos());
        RepositorioDePreferencias::new(archivo.clone()).guardar_ocultar_textos(true);
        let leidas = RepositorioDePreferencias::new(archivo.clone());
        assert!(leidas.ocultar_textos() && leidas.editor_centrado());
        assert!(!leidas.ajustes_de_efectos_visibles());
        assert!(!leidas.caratulas_de_internet(), "no sale a internet sin que se pida");
        RepositorioDePreferencias::new(archivo.clone()).guardar_caratulas_de_internet(true);
        assert!(RepositorioDePreferencias::new(archivo.clone()).caratulas_de_internet());
        RepositorioDePreferencias::new(archivo.clone()).guardar_ajustes_de_efectos_visibles(true);
        assert!(RepositorioDePreferencias::new(archivo.clone()).ajustes_de_efectos_visibles());
        assert_eq!(RepositorioDePreferencias::new(archivo.clone()).ventana_del_editor(), Some(geometria));
    }

    #[test]
    fn las_preferencias_recuerdan_las_locuciones_y_la_localidad_del_clima() {
        let archivo = carpeta_temporal("locuciones").join("preferencias.json");
        let preferencias = RepositorioDePreferencias::new(archivo.clone());
        assert_eq!(preferencias.carpeta_de_locuciones(), None);
        assert_eq!(preferencias.localidad_del_clima(), &Localidad::default());
        let mut preferencias = RepositorioDePreferencias::new(archivo.clone());
        preferencias.guardar_carpeta_de_locuciones(Path::new("D:/locuciones"));
        let leidas = RepositorioDePreferencias::new(archivo.clone());
        assert_eq!(leidas.carpeta_de_locuciones(), Some(Path::new("D:/locuciones")));
        // La localidad se cambia a mano en el archivo; una invalida vuelve a la de siempre.
        fs::write(&archivo, r#"{ "localidadDelClima": { "nombre": "Cordoba", "latitud": -31.42, "longitud": -64.18 } }"#).unwrap();
        let cordoba = RepositorioDePreferencias::new(archivo.clone());
        assert_eq!(cordoba.localidad_del_clima(), &Localidad { nombre: "Cordoba".into(), latitud: -31.42, longitud: -64.18, zona_horaria: None });
        // La zona horaria se guarda y se lee con la localidad.
        let mut con_zona = RepositorioDePreferencias::new(archivo.clone());
        let bogota = Localidad { nombre: "Bogota".into(), latitud: 4.61, longitud: -74.08, zona_horaria: Some("America/Bogota".into()) };
        con_zona.guardar_localidad_del_clima(bogota.clone());
        assert_eq!(RepositorioDePreferencias::new(archivo.clone()).localidad_del_clima(), &bogota);
        fs::write(&archivo, r#"{ "localidadDelClima": { "latitud": 120, "longitud": 0 } }"#).unwrap();
        assert_eq!(RepositorioDePreferencias::new(archivo).localidad_del_clima(), &Localidad::default());
    }

    #[test]
    fn las_preferencias_recuerdan_la_salida_y_el_volumen_de_preescucha() {
        let archivo = carpeta_temporal("preescucha").join("preferencias.json");
        let mut preferencias = RepositorioDePreferencias::new(archivo.clone());
        assert!(preferencias.salida_de_preescucha().is_none());
        assert_eq!(preferencias.volumen_de_preescucha(), VOLUMEN_DE_PREESCUCHA_POR_DEFECTO);
        let auriculares = SalidaDeAudio { id: "wasapi:{0.0.0.00000000}.{abc}".into(), nombre: "Auriculares".into() };
        preferencias.guardar_salida_de_preescucha(Some(auriculares.clone()));
        preferencias.guardar_volumen_de_preescucha(0.654);
        let leidas = RepositorioDePreferencias::new(archivo.clone());
        assert_eq!(leidas.salida_de_preescucha(), Some(&auriculares));
        assert_eq!(leidas.volumen_de_preescucha(), 0.65);
        let mut preferencias = leidas;
        preferencias.guardar_salida_de_preescucha(None);
        preferencias.guardar_volumen_de_preescucha(7.0);
        let leidas = RepositorioDePreferencias::new(archivo);
        assert!(leidas.salida_de_preescucha().is_none());
        assert_eq!(leidas.volumen_de_preescucha(), 1.0);
    }

    #[test]
    fn la_lista_se_guarda_y_se_recupera() {
        let archivo = carpeta_temporal("lista").join("lista.json");
        let repositorio = RepositorioDeLista::new(archivo);
        assert_eq!(repositorio.cargar(), ListaGuardada::default());
        let lista = ListaGuardada {
            pistas: vec![PathBuf::from("C:/a.mp3"), PathBuf::from("C:/b.flac")],
            actual: Some(1),
            repetir: true,
        };
        repositorio.guardar(&lista);
        assert_eq!(repositorio.cargar(), lista);
    }
}
