//! Dominio del reproductor: pistas, puntos de mezcla y la lista de reproduccion.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

mod actualizacion;
mod anunciantes;
mod bloques;
mod busqueda;
mod consulta;
mod carpetas;
mod duplicados;
mod modo;
mod tipos;
mod ordenes;
mod tandas;

pub use actualizacion::{CampoDescriptivo, CamposCorregidos, DatosDescriptivos, OrigenDeLosDatos, Propuesta, proponer};
pub use anunciantes::Anunciante;
pub use ordenes::simplificar as simplificar_nombre;
pub use bloques::{MarcadorDeBloque, bloque_de, posiciones_fijas};
pub use busqueda::{Busqueda, saltar_a_coincidencia};
pub use consulta::{ConsultaDeTemas, DatosDelTema, simplificar as simplificar_texto};
pub use duplicados::{Duplicado, buscar_duplicados, clave_de_tema};
pub use modo::{ModoDeOperacion, OperacionSobreLaSecuencia};
pub use carpetas::{AudiosDeCarpeta, audios_de_carpeta};
pub use ordenes::{
    Asignacion, AvisoEntregado, Frecuencia, Hueco, OrdenDePublicidad, PautaDelDia, Reposicion, avisos_sin_entregar,
    etiqueta_de_salida, repartir, salidas_contratadas,
};
pub use tandas::{
    DIAS_DE_LA_SEMANA, EntradaDeTanda, ModoDeEntrada, PRIORIDAD_MAXIMA, PRIORIDAD_MINIMA, PRIORIDAD_POR_DEFECTO,
    Recurrencia, TOLERANCIA_MAXIMA, TOLERANCIA_POSTERIOR_POR_DEFECTO,
    TOLERANCIA_PREVIA_POR_DEFECTO, TandaProgramada, decidir_entrada, describir_dias, formatear_hora, parsear_dias, parsear_hora,
};
pub use tipos::{ReglasDelTipo, TipoDeElemento};

/// Extensiones de archivo que el reproductor acepta en la lista. Son las que
/// decodifica symphonia; opus y wma no tienen decodificador nativo.
pub const EXTENSIONES_SOPORTADAS: &[&str] =
    &["mp3", "wav", "aif", "aiff", "aac", "m4a", "flac", "ogg", "oga"];

/// Error de negocio cuando una pista no puede cargarse o reproducirse.
#[derive(Debug, Clone)]
pub struct PistaInvalida(pub String);

impl fmt::Display for PistaInvalida {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PistaInvalida {}

/// Elemento de la lista de reproduccion: un archivo de audio del disco local.
///
/// Dos pistas son iguales si apuntan al mismo archivo; en Windows la
/// comparacion no distingue mayusculas, igual que el sistema de archivos.
///
/// La ruta y la clave de comparacion se comparten: clonar una pista no copia
/// texto y compararla o buscarla en un mapa no crea cadenas nuevas, algo que
/// se hace cientos de veces por segundo con listas grandes.
#[derive(Clone, Debug)]
pub struct Pista {
    archivo: Arc<Path>,
    clave: Arc<str>,
}

impl Pista {
    pub fn new(archivo: impl Into<PathBuf>) -> Self {
        let archivo: PathBuf = archivo.into();
        let texto = archivo.to_string_lossy();
        let clave: Arc<str> = if cfg!(windows) { texto.to_lowercase().into() } else { texto.into() };
        Self { archivo: archivo.into(), clave }
    }

    pub fn archivo(&self) -> &Path {
        &self.archivo
    }

    /// Nombre visible de la pista, obtenido del nombre del archivo.
    pub fn nombre(&self) -> String {
        nombre_de_archivo(&self.archivo)
    }

    /// Indica si el nombre del archivo tiene una extension de audio soportada.
    pub fn es_formato_soportado(archivo: &Path) -> bool {
        archivo
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| EXTENSIONES_SOPORTADAS.contains(&extension.to_lowercase().as_str()))
            .unwrap_or(false)
    }

    fn clave(&self) -> &str {
        &self.clave
    }
}

impl PartialEq for Pista {
    fn eq(&self, otra: &Self) -> bool {
        self.clave() == otra.clave()
    }
}

impl Eq for Pista {}

impl Hash for Pista {
    fn hash<H: Hasher>(&self, estado: &mut H) {
        self.clave().hash(estado);
    }
}

pub fn nombre_de_archivo(archivo: &Path) -> String {
    archivo
        .file_name()
        .map(|nombre| nombre.to_string_lossy().into_owned())
        .unwrap_or_else(|| archivo.to_string_lossy().into_owned())
}

/// Ruta absoluta sin componentes "." ni "..", sin tocar el disco.
pub fn normalizar(ruta: &Path) -> PathBuf {
    let absoluta = std::path::absolute(ruta).unwrap_or_else(|_| ruta.to_path_buf());
    let mut resultado = PathBuf::new();
    for componente in absoluta.components() {
        match componente {
            Component::CurDir => {}
            Component::ParentDir => {
                resultado.pop();
            }
            otro => resultado.push(otro),
        }
    }
    resultado
}

/// Tramo de una pista, en segundos desde su comienzo, dentro del cual un
/// efecto de la botonera en modo trabado puede sonar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VentanaDeOverlay {
    inicio: f64,
    fin: f64,
}

impl VentanaDeOverlay {
    pub fn nueva(inicio: f64, fin: f64) -> Result<Self, String> {
        let inicio = validar_segundos(inicio, &tr!("inicio del overlay"))?;
        let fin = validar_segundos(fin, &tr!("fin del overlay"))?;
        if fin <= inicio {
            return Err(tr!("El fin del overlay debe ser posterior a su inicio").into());
        }
        Ok(Self { inicio, fin })
    }

    /// Ventana equivalente al overlay de versiones anteriores, que era la
    /// cantidad de segundos restantes desde la que se permitia el efecto.
    pub fn desde_restante(restante: f64, duracion: f64) -> Option<Self> {
        if restante <= 0.0 || duracion <= 0.0 {
            return None;
        }
        Self::nueva((duracion - restante).max(0.0), duracion).ok()
    }

    pub fn inicio(&self) -> f64 {
        self.inicio
    }

    pub fn fin(&self) -> f64 {
        self.fin
    }

    /// Verifica que la ventana entre en una pista de la duracion indicada.
    pub fn validar_en(&self, duracion: Option<f64>) -> Result<(), String> {
        match duracion {
            Some(duracion) if self.fin > duracion + 0.001 => Err(tr!("El overlay termina en {} pero la pista dura {}", formatear_tiempo(self.fin), formatear_tiempo(duracion))),
            _ => Ok(()),
        }
    }
}

/// Puntos de una pista para mezclarla con sus vecinas y con los efectos de la
/// botonera, todos en segundos.
///
/// - Recortes: lo que se saltea al principio (el tema empieza en
///   `recorte_inicial`) y al final (termina `recorte_final` segundos antes
///   del final del archivo), normalmente silencio.
/// - Intro: desde el comienzo del archivo hasta donde el tema llega a su
///   nivel; lo que queda despues del recorte inicial es el tramo sobre el
///   que puede entrar mientras la anterior todavia suena.
/// - Outro: los ultimos segundos del archivo desde que el tema empieza a
///   apagarse; lo que queda antes del recorte final es el tramo sobre el que
///   puede empezar la siguiente.
/// - Overlay: ventana de la pista dentro de la cual un efecto de la botonera
///   en modo trabado puede sonar encima; sin ventana, el efecto suena ya.
///
/// Recortes, intro y outro salen solos del analisis del tema mientras no
/// sean manuales; lo mismo la ventana de overlay. Cambiarlos a mano los
/// marca como manuales y el analisis ya no los toca.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PuntosDeMezcla {
    intro: f64,
    outro: f64,
    recorte_inicial: f64,
    recorte_final: f64,
    overlay: Option<VentanaDeOverlay>,
    manuales: bool,
    overlay_manual: bool,
}

impl PuntosDeMezcla {
    pub const NINGUNO: PuntosDeMezcla = PuntosDeMezcla {
        intro: 0.0,
        outro: 0.0,
        recorte_inicial: 0.0,
        recorte_final: 0.0,
        overlay: None,
        manuales: false,
        overlay_manual: false,
    };

    /// Puntos sin recortes. Quedan como automaticos; los que fija el
    /// usuario se marcan con `como_manuales`.
    pub fn nuevos(intro: f64, outro: f64, overlay: Option<VentanaDeOverlay>) -> Result<Self, String> {
        Ok(Self { intro: validar_segundos(intro, "intro")?, outro: validar_segundos(outro, "outro")?, overlay, ..Self::NINGUNO })
    }

    pub fn con_recortes(&self, inicial: f64, final_: f64) -> Result<Self, String> {
        Ok(Self {
            recorte_inicial: validar_segundos(inicial, "inicio")?,
            recorte_final: validar_segundos(final_, &tr!("recorte final"))?,
            ..*self
        })
    }

    pub fn intro(&self) -> f64 {
        self.intro
    }

    pub fn outro(&self) -> f64 {
        self.outro
    }

    pub fn recorte_inicial(&self) -> f64 {
        self.recorte_inicial
    }

    pub fn recorte_final(&self) -> f64 {
        self.recorte_final
    }

    /// Segundos de intro que suenan, despues del recorte inicial.
    pub fn intro_audible(&self) -> f64 {
        (self.intro - self.recorte_inicial).max(0.0)
    }

    /// Segundos de outro que suenan, antes del recorte final.
    pub fn outro_audible(&self) -> f64 {
        (self.outro - self.recorte_final).max(0.0)
    }

    pub fn overlay(&self) -> Option<VentanaDeOverlay> {
        self.overlay
    }

    /// Recortes, intro y outro fijados por el usuario.
    pub fn manuales(&self) -> bool {
        self.manuales
    }

    /// Ventana de overlay fijada (o quitada) por el usuario.
    pub fn overlay_manual(&self) -> bool {
        self.overlay_manual
    }

    /// Regla de mezcla: se superponen tantos segundos como el menor entre la
    /// outro audible de la pista que termina y la intro audible de la que
    /// entra.
    pub fn segundos_de_mezcla(saliente: &PuntosDeMezcla, entrante: &PuntosDeMezcla) -> f64 {
        saliente.outro_audible().min(entrante.intro_audible())
    }

    /// Solo el inicio y el fin: sin intro, sin outro y sin ventana de
    /// overlay. Asi quedan los temas de tipo sin mezcla, que suenan sin
    /// mezcla.
    pub fn solo_recortes(&self) -> Self {
        Self { intro: self.recorte_inicial, outro: self.recorte_final, overlay: None, ..*self }
    }

    pub fn con_overlay(&self, overlay: Option<VentanaDeOverlay>) -> Self {
        Self { overlay, ..*self }
    }

    pub fn como_manuales(&self, manuales: bool) -> Self {
        Self { manuales, ..*self }
    }

    pub fn con_overlay_manual(&self, overlay_manual: bool) -> Self {
        Self { overlay_manual, ..*self }
    }

    /// Estos puntos con los recortes, la intro y la outro de `automaticos`
    /// (salvo que sean manuales) y con su ventana de overlay (salvo que la
    /// del usuario sea manual).
    pub fn completar_con(&self, automaticos: &PuntosDeMezcla) -> Self {
        let mut resultado = if self.manuales {
            *self
        } else {
            Self { overlay: self.overlay, overlay_manual: self.overlay_manual, manuales: false, ..*automaticos }
        };
        if !self.overlay_manual {
            resultado.overlay = automaticos.overlay;
        }
        resultado
    }

    /// Verifica que los puntos entren en una pista de la duracion indicada.
    pub fn validar_en(&self, duracion: Option<f64>) -> Result<(), String> {
        if let Some(duracion) = duracion {
            if self.intro > duracion + 0.001 {
                return Err(tr!("La intro supera la duracion de la pista ({})", formatear_tiempo(duracion)));
            }
            if self.outro > duracion + 0.001 {
                return Err(tr!("La outro supera la duracion de la pista ({})", formatear_tiempo(duracion)));
            }
            if duracion > 0.0 && self.recorte_inicial + self.recorte_final >= duracion {
                return Err(tr!("El fin del tema debe ser posterior a su inicio").into());
            }
        }
        self.overlay.map_or(Ok(()), |ventana| ventana.validar_en(duracion))
    }
}

fn validar_segundos(segundos: f64, campo: &str) -> Result<f64, String> {
    if !segundos.is_finite() || segundos < 0.0 {
        return Err(tr!("El {campo} debe ser un numero de segundos mayor o igual a 0", campo = campo));
    }
    Ok(segundos)
}

/// Escribe un tiempo como segundos ("45.5") o, desde el minuto, como
/// minutos y segundos ("1:20.5"), con hasta dos decimales.
pub fn formatear_tiempo(segundos: f64) -> String {
    let centesimas = (segundos.max(0.0) * 100.0).round() as u64;
    let (minutos, resto) = (centesimas / 6_000, centesimas % 6_000);
    let segundos = format!("{}.{:02}", resto / 100, resto % 100);
    let segundos = segundos.trim_end_matches('0').trim_end_matches('.');
    if minutos == 0 {
        segundos.to_string()
    } else if resto < 1_000 {
        format!("{minutos}:0{segundos}")
    } else {
        format!("{minutos}:{segundos}")
    }
}

/// Escribe una duracion con decimas, siempre con minutos: "0:08.0",
/// "3:25.4" o, desde la hora, "1:02:33.4".
pub fn formatear_duracion(segundos: f64) -> String {
    let decimas = (segundos.max(0.0) * 10.0).round() as u64;
    let (horas, minutos, segundos, decima) = (decimas / 36_000, decimas / 600 % 60, decimas / 10 % 60, decimas % 10);
    if horas > 0 {
        format!("{horas}:{minutos:02}:{segundos:02}.{decima}")
    } else {
        format!("{minutos}:{segundos:02}.{decima}")
    }
}

/// Lee un tiempo escrito como segundos ("80", "80,5") o como minutos y
/// segundos ("1:20", "1:20.5").
pub fn parsear_tiempo(texto: &str) -> Result<f64, String> {
    let texto = texto.trim().replace(',', ".");
    let invalido = || tr!("'{texto}' no es un tiempo valido (use segundos o m:ss)", texto = texto);
    let segundos = match texto.split_once(':') {
        Some((minutos, segundos)) => {
            let minutos: u64 = minutos.trim().parse().map_err(|_| invalido())?;
            let segundos: f64 = segundos.trim().parse().map_err(|_| invalido())?;
            if !(0.0..60.0).contains(&segundos) {
                return Err(invalido());
            }
            minutos as f64 * 60.0 + segundos
        }
        None => texto.parse().map_err(|_| invalido())?,
    };
    validar_segundos(segundos, "tiempo")
}

/// Texto de una ventana con su duracion: "1:50.25 - 2:00.3 | 10.05", o "—"
/// sin ventana.
pub fn formatear_ventana(ventana: Option<VentanaDeOverlay>) -> String {
    match ventana {
        Some(ventana) => format!(
            "{} - {} | {}",
            formatear_tiempo(ventana.inicio()),
            formatear_tiempo(ventana.fin()),
            formatear_tiempo(ventana.fin() - ventana.inicio())
        ),
        None => "—".into(),
    }
}

/// Lista ordenada de pistas con nocion de pista actual.
///
/// Reglas de negocio:
/// - No se permiten pistas duplicadas (misma ruta de archivo), salvo las que
///   se insertan a proposito como copias (`insertar_copias`): los comerciales
///   pueden salir mas de una vez. Las copias comparten los puntos de mezcla.
/// - La navegacion "siguiente/anterior" recorre la lista sin salir de sus limites.
/// - Cada pista tiene puntos de mezcla (intro/outro); entre dos pistas
///   consecutivas se mezclan los segundos que indica PuntosDeMezcla.
/// - Con la repeticion activada, despues de la ultima pista sigue la primera
///   (y se mezclan como cualquier otro par consecutivo).
#[derive(Default)]
pub struct ListaDeReproduccion {
    pistas: Vec<Pista>,
    puntos_de_mezcla: HashMap<Pista, PuntosDeMezcla>,
    indice_actual: Option<usize>,
    repetir: bool,
}

impl ListaDeReproduccion {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserta archivos a partir de `posicion` (desde 0; mas alla del final
    /// agrega al final), ignorando los que ya estaban. La pista actual sigue
    /// siendo la misma cancion. Devuelve las pistas insertadas.
    pub fn insertar<I: IntoIterator<Item = PathBuf>>(&mut self, posicion: usize, archivos: I) -> Vec<Pista> {
        self.insertar_con(posicion, archivos, false)
    }

    /// Como `insertar`, pero admite archivos que ya estan en la lista: quedan
    /// como copias.
    pub fn insertar_copias<I: IntoIterator<Item = PathBuf>>(&mut self, posicion: usize, archivos: I) -> Vec<Pista> {
        self.insertar_con(posicion, archivos, true)
    }

    fn insertar_con<I: IntoIterator<Item = PathBuf>>(&mut self, posicion: usize, archivos: I, copias: bool) -> Vec<Pista> {
        let mut destino = posicion.min(self.pistas.len());
        let mut insertadas = Vec::new();
        for archivo in archivos {
            let pista = Pista::new(archivo);
            if !copias && self.pistas.contains(&pista) {
                continue;
            }
            self.pistas.insert(destino, pista.clone());
            if let Some(actual) = self.indice_actual.filter(|actual| *actual >= destino) {
                self.indice_actual = Some(actual + 1);
            }
            destino += 1;
            insertadas.push(pista);
        }
        insertadas
    }

    pub fn tamano(&self) -> usize {
        self.pistas.len()
    }

    pub fn esta_vacia(&self) -> bool {
        self.pistas.is_empty()
    }

    pub fn pistas(&self) -> &[Pista] {
        &self.pistas
    }

    pub fn pista_en(&self, indice: usize) -> Option<&Pista> {
        self.pistas.get(indice)
    }

    pub fn indice_actual(&self) -> Option<usize> {
        self.indice_actual
    }

    pub fn pista_actual(&self) -> Option<&Pista> {
        self.indice_actual.and_then(|indice| self.pista_en(indice))
    }

    /// Marca una pista como actual sin reproducirla.
    pub fn seleccionar_indice(&mut self, indice: usize) {
        assert!(indice < self.pistas.len(), "Indice fuera de la lista: {indice}");
        self.indice_actual = Some(indice);
    }

    pub fn repite(&self) -> bool {
        self.repetir
    }

    pub fn definir_repetir(&mut self, repetir: bool) {
        self.repetir = repetir;
    }

    /// Posicion de la pista que sigue a la indicada: la proxima, o la primera
    /// si la indicada es la ultima y la repeticion esta activada.
    pub fn indice_siguiente(&self, indice: Option<usize>) -> Option<usize> {
        let indice = indice.filter(|&indice| indice < self.pistas.len())?;
        if indice + 1 < self.pistas.len() {
            return Some(indice + 1);
        }
        self.repetir.then_some(0)
    }

    /// Retrocede a la pista anterior si existe.
    pub fn anterior(&mut self) -> Option<&Pista> {
        match self.indice_actual {
            Some(indice) if indice > 0 => {
                self.indice_actual = Some(indice - 1);
                self.pista_actual()
            }
            _ => None,
        }
    }

    /// Quita la pista de una posicion y ajusta el indice actual para que siga
    /// apuntando a la misma cancion cuando sea posible.
    pub fn quitar_en(&mut self, indice: usize) -> Option<Pista> {
        if indice >= self.pistas.len() {
            return None;
        }
        let eliminada = self.pistas.remove(indice);
        // Los puntos se comparten con las copias que quedan.
        if !self.pistas.contains(&eliminada) {
            self.puntos_de_mezcla.remove(&eliminada);
        }
        if let Some(actual) = self.indice_actual {
            if indice < actual {
                self.indice_actual = Some(actual - 1);
            } else if indice == actual {
                self.indice_actual = self.pistas.len().checked_sub(1).map(|ultima| actual.min(ultima));
            }
        }
        Some(eliminada)
    }

    /// Mueve la pista de la posicion `desde` a la posicion `hasta`; la pista
    /// actual sigue siendo la misma cancion. Devuelve false si alguna
    /// posicion esta fuera de la lista.
    pub fn mover(&mut self, desde: usize, hasta: usize) -> bool {
        if desde >= self.pistas.len() || hasta >= self.pistas.len() {
            return false;
        }
        let pista = self.pistas.remove(desde);
        self.pistas.insert(hasta, pista);
        if let Some(actual) = self.indice_actual {
            self.indice_actual = Some(if actual == desde {
                hasta
            } else if desde < actual && hasta >= actual {
                actual - 1
            } else if desde > actual && hasta <= actual {
                actual + 1
            } else {
                actual
            });
        }
        true
    }

    /// Mueve un lugar hacia arriba (o hacia abajo) las pistas de esas
    /// posiciones, juntas o salteadas; las demas les ceden el lugar. Devuelve
    /// las posiciones nuevas, o None si no hay posiciones validas o alguna
    /// ya esta en el borde hacia el que se mueve.
    pub fn desplazar(&mut self, posiciones: &[usize], abajo: bool) -> Option<Vec<usize>> {
        let mut posiciones = posiciones.to_vec();
        posiciones.sort_unstable();
        posiciones.dedup();
        let (&primera, &ultima) = (posiciones.first()?, posiciones.last()?);
        if ultima >= self.pistas.len() || (!abajo && primera == 0) || (abajo && ultima + 1 == self.pistas.len()) {
            return None;
        }
        if abajo {
            for &posicion in posiciones.iter().rev() {
                self.mover(posicion, posicion + 1);
            }
        } else {
            for &posicion in &posiciones {
                self.mover(posicion, posicion - 1);
            }
        }
        Some(posiciones.iter().map(|&posicion| if abajo { posicion + 1 } else { posicion - 1 }).collect())
    }

    /// Mezcla al azar las pistas de esas posiciones (`semilla` elige el
    /// orden); cada una toma el lugar de otra del mismo grupo y el resto de
    /// la lista no se mueve (asi quedan quietos los bloques fijos, que no se
    /// incluyen). Si la pista actual esta entre ellas, queda en la primera de
    /// esas posiciones, para que siga sonando y despues vengan las demas en
    /// el orden nuevo.
    pub fn mezclar(&mut self, posiciones: &[usize], semilla: u64) {
        let posiciones = self.posiciones_validas(posiciones);
        let mut pistas: Vec<Pista> = posiciones.iter().map(|&posicion| self.pistas[posicion].clone()).collect();
        let mut generador = semilla | 1;
        let mut azar = |hasta: usize| {
            // xorshift64: alcanza para barajar, sin depender de otra biblioteca.
            generador ^= generador << 13;
            generador ^= generador >> 7;
            generador ^= generador << 17;
            (generador % hasta as u64) as usize
        };
        let actual = self.pista_actual().cloned();
        let desde = match actual.and_then(|actual| pistas.iter().position(|pista| *pista == actual)) {
            Some(indice) => {
                pistas[..=indice].rotate_right(1);
                1
            }
            None => 0,
        };
        for indice in (desde + 1..pistas.len()).rev() {
            let otro = desde + azar(indice - desde + 1);
            pistas.swap(indice, otro);
        }
        let mezcladas = pistas;
        self.reordenar_en(&posiciones, |pistas| pistas.clone_from_slice(&mezcladas));
    }

    /// Ordena por tempo, de menor a mayor, las pistas de esas posiciones,
    /// para que queden juntos los temas de BPM parecido; los que no tienen
    /// tempo van detras. A igual BPM (redondeado) va primero el de mayor
    /// ranking, y a igual ranking se conserva el orden. El resto de la lista
    /// no se mueve y la pista actual sigue siendo la misma cancion.
    pub fn ordenar_por_bpm(
        &mut self,
        posiciones: &[usize],
        bpm_de: impl Fn(&Pista) -> Option<f64>,
        ranking_de: impl Fn(&Pista) -> u8,
    ) {
        self.reordenar_en(posiciones, |pistas| {
            pistas.sort_by_cached_key(|pista| {
                let bpm = bpm_de(pista).map_or(u64::MAX, |bpm| bpm.round().max(0.0) as u64);
                (bpm, std::cmp::Reverse(ranking_de(pista)))
            });
        });
    }

    /// Saca las pistas de esas posiciones (las validas, en orden), deja que
    /// `reordenar` las cambie de orden y las vuelve a poner en esas mismas
    /// posiciones. La pista actual sigue siendo la misma cancion.
    /// Posiciones dentro de la lista, ordenadas y sin repetir.
    fn posiciones_validas(&self, posiciones: &[usize]) -> Vec<usize> {
        let mut posiciones: Vec<usize> = posiciones.iter().copied().filter(|&posicion| posicion < self.pistas.len()).collect();
        posiciones.sort_unstable();
        posiciones.dedup();
        posiciones
    }

    fn reordenar_en(&mut self, posiciones: &[usize], reordenar: impl FnOnce(&mut [Pista])) {
        let posiciones = self.posiciones_validas(posiciones);
        let actual = self.pista_actual().cloned();
        let mut pistas: Vec<Pista> = posiciones.iter().map(|&posicion| self.pistas[posicion].clone()).collect();
        reordenar(&mut pistas);
        for (posicion, pista) in posiciones.into_iter().zip(pistas) {
            self.pistas[posicion] = pista;
        }
        self.seguir_a(actual);
    }

    /// Vuelve a marcar como actual la pista indicada; si tiene copias, la
    /// mas cercana a donde estaba.
    fn seguir_a(&mut self, pista: Option<Pista>) {
        let antes = self.indice_actual.unwrap_or(0);
        self.indice_actual = pista.and_then(|pista| {
            (0..self.pistas.len()).filter(|&indice| self.pistas[indice] == pista).min_by_key(|&indice| indice.abs_diff(antes))
        });
    }

    pub fn limpiar(&mut self) {
        self.pistas.clear();
        self.puntos_de_mezcla.clear();
        self.indice_actual = None;
    }

    pub fn indice_de(&self, pista: &Pista) -> Option<usize> {
        self.pistas.iter().position(|candidata| candidata == pista)
    }

    pub fn puntos_de(&self, pista: &Pista) -> PuntosDeMezcla {
        self.puntos_de_mezcla.get(pista).copied().unwrap_or(PuntosDeMezcla::NINGUNO)
    }

    pub fn definir_puntos(&mut self, pista: &Pista, puntos: PuntosDeMezcla) -> Result<(), String> {
        if !self.pistas.contains(pista) {
            return Err(tr!("La pista no esta en la lista: {}", pista.nombre()));
        }
        self.puntos_de_mezcla.insert(pista.clone(), puntos);
        Ok(())
    }

    /// Cambia el archivo de una pista (por ejemplo, porque se movio) en todas
    /// sus posiciones, conservando sus puntos de mezcla. Devuelve cuantas
    /// posiciones cambiaron.
    pub fn reubicar(&mut self, anterior: &Pista, nueva: &Pista) -> usize {
        let mut cambiadas = 0;
        for pista in self.pistas.iter_mut().filter(|pista| *pista == anterior) {
            *pista = nueva.clone();
            cambiadas += 1;
        }
        if let Some(puntos) = self.puntos_de_mezcla.remove(anterior) {
            self.puntos_de_mezcla.insert(nueva.clone(), puntos);
        }
        cambiadas
    }

    /// Proxima pista despues de la indicada que cumple `disponible` (por
    /// ejemplo, que su archivo exista), y las posiciones que se saltean en el
    /// camino porque no la cumplen. Con la repeticion activada da la vuelta,
    /// pero recorre la lista una sola vez: si ninguna cumple, devuelve None.
    pub fn proxima_disponible(&self, indice: Option<usize>, disponible: impl Fn(&Pista) -> bool) -> (Option<usize>, Vec<usize>) {
        let mut salteadas = Vec::new();
        let mut desde = indice;
        for _ in 0..self.pistas.len() {
            let Some(candidata) = self.indice_siguiente(desde) else {
                break;
            };
            if disponible(&self.pistas[candidata]) {
                return (Some(candidata), salteadas);
            }
            salteadas.push(candidata);
            desde = Some(candidata);
        }
        (None, salteadas)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn lista_con(nombres: &[&str]) -> ListaDeReproduccion {
        let mut lista = ListaDeReproduccion::new();
        lista.insertar(usize::MAX, nombres.iter().map(PathBuf::from));
        lista
    }

    fn nombres(lista: &ListaDeReproduccion) -> Vec<String> {
        lista.pistas().iter().map(Pista::nombre).collect()
    }

    #[test]
    fn mezclar_deja_primera_la_actual_y_cambia_el_orden() {
        let archivos: Vec<String> = (0..20).map(|numero| format!("{numero:02}.mp3")).collect();
        let referencias: Vec<&str> = archivos.iter().map(String::as_str).collect();
        let mut lista = lista_con(&referencias);
        lista.seleccionar_indice(7);
        lista.mezclar(&(0..20).collect::<Vec<_>>(), 12345);
        assert_eq!(lista.indice_actual(), Some(0));
        assert_eq!(nombres(&lista)[0], "07.mp3");
        let mut ordenados = nombres(&lista);
        ordenados.sort();
        assert_eq!(ordenados, archivos, "son las mismas pistas");
        assert_ne!(nombres(&lista)[1..], {
            let mut sin_actual = archivos.clone();
            sin_actual.remove(7);
            sin_actual
        }[..]);
        // Otra semilla da otro orden.
        let mut otra = lista_con(&referencias);
        otra.mezclar(&(0..20).collect::<Vec<_>>(), 999);
        assert_ne!(nombres(&otra), nombres(&lista));
    }

    #[test]
    fn ordena_por_bpm_y_deja_al_final_los_que_no_tienen() {
        let mut lista = lista_con(&["lento.mp3", "sin.mp3", "rapido.mp3", "medio.mp3", "medio2.mp3"]);
        lista.seleccionar_indice(2);
        let bpm_de = |pista: &Pista| match pista.nombre().as_str() {
            "lento.mp3" => Some(90.2),
            "rapido.mp3" => Some(140.0),
            "medio.mp3" => Some(124.4),
            "medio2.mp3" => Some(123.6),
            _ => None,
        };
        lista.ordenar_por_bpm(&[0, 1, 2, 3, 4], bpm_de, |_| 0);
        assert_eq!(nombres(&lista), ["lento.mp3", "medio.mp3", "medio2.mp3", "rapido.mp3", "sin.mp3"]);
        // A igual tempo, primero el de mayor ranking.
        lista.ordenar_por_bpm(&[0, 1, 2, 3, 4], bpm_de, |pista| if pista.nombre() == "medio2.mp3" { 5 } else { 3 });
        assert_eq!(nombres(&lista), ["lento.mp3", "medio2.mp3", "medio.mp3", "rapido.mp3", "sin.mp3"]);
        assert_eq!(lista.pista_actual().map(Pista::nombre).as_deref(), Some("rapido.mp3"));
    }

    #[test]
    fn mezcla_y_ordena_solo_el_bloque_indicado() {
        let archivos: Vec<String> = (0..12).map(|numero| format!("{numero:02}")).collect();
        let referencias: Vec<&str> = archivos.iter().map(String::as_str).collect();
        let mut lista = lista_con(&referencias);
        lista.seleccionar_indice(5);
        let bloque: Vec<usize> = (3..9).collect();
        lista.mezclar(&bloque, 4242);
        let resultado = nombres(&lista);
        // Fuera del bloque nada se mueve; adentro estan las mismas pistas.
        assert_eq!(resultado[..3], archivos[..3]);
        assert_eq!(resultado[9..], archivos[9..]);
        let mut adentro = resultado[3..9].to_vec();
        adentro.sort();
        assert_eq!(adentro, archivos[3..9]);
        // La que suena queda primera del bloque.
        assert_eq!(resultado[3], "05");
        assert_eq!(lista.indice_actual(), Some(3));

        let mut lista = lista_con(&["d", "c", "b", "a", "e"]);
        let bpm_de = |pista: &Pista| Some(f64::from(pista.nombre().as_bytes()[0]));
        lista.ordenar_por_bpm(&[1, 2, 3], bpm_de, |_| 0);
        assert_eq!(nombres(&lista), ["d", "a", "b", "c", "e"]);
    }

    #[test]
    fn mueve_bloques_de_pistas_seleccionadas() {
        let mut lista = lista_con(&["a", "b", "c", "d", "e", "f"]);
        lista.seleccionar_indice(4);
        // Un bloque (b, c) y una suelta (e) suben juntos.
        assert_eq!(lista.desplazar(&[1, 2, 4], false), Some(vec![0, 1, 3]));
        assert_eq!(nombres(&lista), ["b", "c", "a", "e", "d", "f"]);
        assert_eq!(lista.pista_actual().map(Pista::nombre).as_deref(), Some("e"));
        // Ya en el borde: no se mueve nada.
        assert_eq!(lista.desplazar(&[0, 1], false), None);
        assert_eq!(lista.desplazar(&[0, 1], true), Some(vec![1, 2]));
        assert_eq!(nombres(&lista), ["a", "b", "c", "e", "d", "f"]);
        assert_eq!(lista.desplazar(&[5], true), None);
        assert_eq!(lista.desplazar(&[], true), None);
    }

    #[test]
    fn no_agrega_duplicados() {
        let mut lista = lista_con(&["a.mp3", "b.mp3"]);
        lista.insertar(usize::MAX, [PathBuf::from("a.mp3"), PathBuf::from("c.mp3")]);
        assert_eq!(lista.tamano(), 3);
    }

    #[test]
    fn solo_recortes_deja_el_inicio_y_el_fin_sin_mezcla() {
        let ventana = VentanaDeOverlay::nueva(10.0, 20.0).ok();
        let puntos = PuntosDeMezcla::nuevos(8.0, 12.0, ventana).unwrap().con_recortes(0.5, 1.5).unwrap();
        let reservado = puntos.solo_recortes();
        assert_eq!((reservado.recorte_inicial(), reservado.recorte_final()), (0.5, 1.5));
        assert_eq!((reservado.intro_audible(), reservado.outro_audible()), (0.0, 0.0));
        assert_eq!(reservado.overlay(), None);
    }

    #[test]
    fn las_copias_comparten_puntos_y_se_quitan_de_a_una() {
        let mut lista = lista_con(&["spot.mp3", "b.mp3"]);
        let spot = lista.pista_en(0).cloned().unwrap();
        let puntos = PuntosDeMezcla::nuevos(1.0, 2.0, None).unwrap();
        lista.definir_puntos(&spot, puntos).unwrap();
        lista.insertar_copias(usize::MAX, [PathBuf::from("spot.mp3")]);
        assert_eq!(nombres(&lista), ["spot.mp3", "b.mp3", "spot.mp3"]);
        // Al mezclar, la actual sigue siendo el mismo comercial.
        lista.seleccionar_indice(2);
        lista.mezclar(&[0, 1, 2], 7);
        assert_eq!(lista.pista_actual(), Some(&spot));
        lista.quitar_en(0);
        assert_eq!(lista.puntos_de(&spot), puntos, "la otra copia conserva los puntos");
        let restante = lista.indice_de(&spot).unwrap();
        lista.quitar_en(restante);
        assert_eq!(lista.puntos_de(&spot), PuntosDeMezcla::NINGUNO);
    }

    #[test]
    fn navega_sin_salir_de_los_limites() {
        let mut lista = lista_con(&["a.mp3", "b.mp3"]);
        assert!(lista.indice_siguiente(lista.indice_actual()).is_none(), "sin pista actual no hay siguiente");
        lista.seleccionar_indice(0);
        assert!(lista.anterior().is_none());
        assert_eq!(lista.indice_siguiente(lista.indice_actual()), Some(1));
        lista.seleccionar_indice(1);
        assert!(lista.indice_siguiente(lista.indice_actual()).is_none());
        assert_eq!(lista.indice_actual(), Some(1));
    }

    #[test]
    fn con_repeticion_la_ultima_sigue_con_la_primera() {
        let mut lista = lista_con(&["a.mp3", "b.mp3"]);
        lista.definir_repetir(true);
        assert_eq!(lista.indice_siguiente(Some(1)), Some(0));
    }

    #[test]
    fn quitar_ajusta_el_indice_actual() {
        let mut lista = lista_con(&["a.mp3", "b.mp3", "c.mp3"]);
        lista.seleccionar_indice(2);
        lista.quitar_en(0);
        assert_eq!(lista.indice_actual(), Some(1));
        lista.quitar_en(1);
        assert_eq!(lista.indice_actual(), Some(0));
        lista.quitar_en(0);
        assert_eq!(lista.indice_actual(), None);
    }

    #[test]
    fn insertar_respeta_la_posicion_y_la_pista_actual() {
        let nombres = |lista: &ListaDeReproduccion| lista.pistas().iter().map(Pista::nombre).collect::<Vec<_>>();
        let mut lista = lista_con(&["a.mp3", "b.mp3", "c.mp3"]);
        lista.seleccionar_indice(1);
        let insertadas = lista.insertar(1, [PathBuf::from("x.mp3"), PathBuf::from("a.mp3"), PathBuf::from("y.mp3")]);
        assert_eq!(insertadas.len(), 2, "a.mp3 ya estaba");
        assert_eq!(nombres(&lista), ["a.mp3", "x.mp3", "y.mp3", "b.mp3", "c.mp3"]);
        assert_eq!(lista.pista_actual().map(Pista::nombre), Some("b.mp3".into()));
        lista.insertar(99, [PathBuf::from("z.mp3")]);
        assert_eq!(nombres(&lista).last().map(String::as_str), Some("z.mp3"));
        assert_eq!(lista.pista_actual().map(Pista::nombre), Some("b.mp3".into()));
    }

    #[test]
    fn mover_conserva_la_pista_actual() {
        let nombres = |lista: &ListaDeReproduccion| lista.pistas().iter().map(Pista::nombre).collect::<Vec<_>>();
        let mut lista = lista_con(&["a.mp3", "b.mp3", "c.mp3", "d.mp3"]);
        lista.seleccionar_indice(1);
        assert!(lista.mover(1, 2));
        assert_eq!(nombres(&lista), ["a.mp3", "c.mp3", "b.mp3", "d.mp3"]);
        assert_eq!(lista.pista_actual().map(Pista::nombre), Some("b.mp3".into()));
        assert!(lista.mover(3, 0));
        assert_eq!(nombres(&lista), ["d.mp3", "a.mp3", "c.mp3", "b.mp3"]);
        assert_eq!(lista.pista_actual().map(Pista::nombre), Some("b.mp3".into()));
        assert!(lista.mover(0, 3));
        assert_eq!(lista.pista_actual().map(Pista::nombre), Some("b.mp3".into()));
        assert!(!lista.mover(0, 4));
    }

    #[test]
    fn formatea_duraciones_con_decimas() {
        assert_eq!(formatear_duracion(8.0), "0:08.0");
        assert_eq!(formatear_duracion(205.44), "3:25.4");
        assert_eq!(formatear_duracion(3_753.36), "1:02:33.4");
        assert_eq!(formatear_duracion(59.96), "1:00.0");
    }

    #[test]
    fn la_mezcla_es_el_menor_entre_outro_e_intro() {
        let mut lista = lista_con(&["a.mp3", "b.mp3"]);
        let a = lista.pista_en(0).unwrap().clone();
        let b = lista.pista_en(1).unwrap().clone();
        lista.definir_puntos(&a, PuntosDeMezcla::nuevos(0.0, 8.0, None).unwrap()).unwrap();
        lista.definir_puntos(&b, PuntosDeMezcla::nuevos(5.0, 0.0, None).unwrap()).unwrap();
        assert_eq!(PuntosDeMezcla::segundos_de_mezcla(&lista.puntos_de(&a), &lista.puntos_de(&b)), 5.0);
        assert_eq!(PuntosDeMezcla::segundos_de_mezcla(&lista.puntos_de(&b), &lista.puntos_de(&a)), 0.0);
    }

    #[test]
    fn reubicar_cambia_todas_las_copias_y_conserva_los_puntos() {
        let mut lista = lista_con(&["a.mp3", "spot.mp3"]);
        let spot = lista.pista_en(1).unwrap().clone();
        lista.insertar_copias(2, vec![spot.archivo().to_path_buf()]);
        let puntos = PuntosDeMezcla::nuevos(2.0, 3.0, None).unwrap();
        lista.definir_puntos(&spot, puntos).unwrap();
        let movido = Pista::new(PathBuf::from("tandas").join("spot.mp3"));
        assert_eq!(lista.reubicar(&spot, &movido), 2);
        assert_eq!(lista.pista_en(1), Some(&movido));
        assert_eq!(lista.pista_en(2), Some(&movido));
        assert_eq!(lista.puntos_de(&movido), puntos);
    }

    #[test]
    fn la_proxima_disponible_saltea_las_que_faltan() {
        let mut lista = lista_con(&["a.mp3", "b.mp3", "c.mp3", "d.mp3"]);
        let faltan = |pista: &Pista| !["b.mp3", "c.mp3"].contains(&pista.nombre().as_str());
        assert_eq!(lista.proxima_disponible(Some(0), faltan), (Some(3), vec![1, 2]));
        assert_eq!(lista.proxima_disponible(Some(2), faltan), (Some(3), vec![]));
        // Sin repeticion no hay nada despues de la ultima.
        assert_eq!(lista.proxima_disponible(Some(3), faltan), (None, vec![]));
        // Con repeticion da la vuelta.
        lista.definir_repetir(true);
        assert_eq!(lista.proxima_disponible(Some(3), faltan), (Some(0), vec![]));
        // Si no hay ninguna disponible, recorre una sola vez y termina.
        let (destino, salteadas) = lista.proxima_disponible(Some(0), |_| false);
        assert_eq!(destino, None);
        assert_eq!(salteadas.len(), 4);
    }

    #[test]
    fn los_recortes_descuentan_intro_y_outro() {
        let puntos = PuntosDeMezcla::nuevos(5.0, 8.0, None).unwrap().con_recortes(2.0, 3.0).unwrap();
        assert_eq!((puntos.intro_audible(), puntos.outro_audible()), (3.0, 5.0));
        let otra = PuntosDeMezcla::nuevos(10.0, 0.0, None).unwrap();
        assert_eq!(PuntosDeMezcla::segundos_de_mezcla(&puntos, &otra), 5.0);
        assert!(puntos.validar_en(Some(100.0)).is_ok());
        assert!(puntos.con_recortes(60.0, 40.0).unwrap().validar_en(Some(100.0)).is_err());
    }

    #[test]
    fn los_puntos_automaticos_no_pisan_los_manuales() {
        let ventana = |inicio: f64| Some(VentanaDeOverlay::nueva(inicio, inicio + 8.0).unwrap());
        let automaticos = PuntosDeMezcla::nuevos(4.0, 6.0, ventana(1.0)).unwrap().con_recortes(1.0, 2.0).unwrap();
        assert_eq!(PuntosDeMezcla::NINGUNO.completar_con(&automaticos), automaticos);

        let manuales = PuntosDeMezcla::nuevos(9.0, 9.0, None).unwrap().como_manuales(true);
        let completados = manuales.completar_con(&automaticos);
        assert_eq!((completados.intro(), completados.outro(), completados.recorte_inicial()), (9.0, 9.0, 0.0));
        assert_eq!(completados.overlay(), ventana(1.0), "el overlay sigue siendo automatico");

        let sin_overlay = PuntosDeMezcla::NINGUNO.con_overlay_manual(true);
        let completados = sin_overlay.completar_con(&automaticos);
        assert_eq!(completados.overlay(), None, "el usuario quito el overlay");
        assert_eq!(completados.intro(), 4.0);
        assert!(completados.overlay_manual() && !completados.manuales());
    }

    #[test]
    fn rechaza_segundos_invalidos() {
        assert!(PuntosDeMezcla::nuevos(-1.0, 0.0, None).is_err());
        assert!(PuntosDeMezcla::nuevos(f64::NAN, 0.0, None).is_err());
        assert!(VentanaDeOverlay::nueva(10.0, 10.0).is_err());
        assert!(VentanaDeOverlay::nueva(12.0, 10.0).is_err());
    }

    #[test]
    fn valida_los_puntos_contra_la_duracion() {
        let ventana = VentanaDeOverlay::nueva(80.0, 105.0).unwrap();
        let puntos = PuntosDeMezcla::nuevos(6.0, 10.0, Some(ventana)).unwrap();
        assert!(puntos.validar_en(Some(180.0)).is_ok());
        assert!(puntos.validar_en(None).is_ok());
        assert!(puntos.validar_en(Some(100.0)).is_err());
        assert!(PuntosDeMezcla::nuevos(200.0, 0.0, None).unwrap().validar_en(Some(180.0)).is_err());
    }

    #[test]
    fn convierte_el_overlay_anterior_a_ventana() {
        let ventana = VentanaDeOverlay::desde_restante(20.0, 180.0).unwrap();
        assert_eq!((ventana.inicio(), ventana.fin()), (160.0, 180.0));
        let mayor_que_la_pista = VentanaDeOverlay::desde_restante(300.0, 180.0).unwrap();
        assert_eq!((mayor_que_la_pista.inicio(), mayor_que_la_pista.fin()), (0.0, 180.0));
        assert!(VentanaDeOverlay::desde_restante(0.0, 180.0).is_none());
    }

    #[test]
    fn escribe_y_lee_tiempos() {
        assert_eq!(formatear_tiempo(45.5), "45.5");
        assert_eq!(formatear_tiempo(80.0), "1:20");
        assert_eq!(formatear_tiempo(65.25), "1:05.25");
        assert_eq!(formatear_tiempo(3_600.0), "60:00");
        assert_eq!(parsear_tiempo("1:20.5").unwrap(), 80.5);
        assert_eq!(parsear_tiempo(" 80,5 ").unwrap(), 80.5);
        assert!(parsear_tiempo("1:75").is_err());
        assert!(parsear_tiempo("-3").is_err());
        assert!(parsear_tiempo("abc").is_err());
        for segundos in [0.0, 9.99, 59.5, 60.0, 61.04, 754.25] {
            assert!((parsear_tiempo(&formatear_tiempo(segundos)).unwrap() - segundos).abs() < 1e-9);
        }
    }

    #[test]
    fn escribe_ventanas_con_su_duracion() {
        let ventana = VentanaDeOverlay::nueva(80.0, 105.0).unwrap();
        assert_eq!(formatear_ventana(Some(ventana)), "1:20 - 1:45 | 25");
        let tabla = VentanaDeOverlay::nueva(110.25, 120.3).unwrap();
        assert_eq!(formatear_ventana(Some(tabla)), "1:50.25 - 2:00.3 | 10.05");
        assert_eq!(formatear_ventana(None), "—");
    }

    #[test]
    fn reconoce_formatos_soportados() {
        assert!(Pista::es_formato_soportado(Path::new("tema.FLAC")));
        assert!(!Pista::es_formato_soportado(Path::new("tema.wma")));
        assert!(!Pista::es_formato_soportado(Path::new("sin_extension")));
    }
}
