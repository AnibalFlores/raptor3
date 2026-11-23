//! Buscador del catalogo: encuentra temas ya conocidos por titulo, artista,
//! tipo, genero, album o nombre de archivo (todas las palabras, sin importar
//! mayusculas ni acentos), o con prefijos por un dato en particular
//! (`artista:`, `ranking:>=4`, `sonado:>30d`...), y los inserta en la lista.
//!
//! Solo busca en las fichas del catalogo (los temas que alguna vez entraron
//! a la lista); no recorre carpetas. Los de tipo Efecto no aparecen, porque
//! solo van en la botonera.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{CloseRequestResponse, ComponentHandle, Image, Model, ModelRc, VecModel};

use super::caratula::{CaratulaVista, DestinoDeCaratula};
use super::{Controlador, con_controlador, formatear_minutos, reproduccion_para_la_lista};
use crate::dominio::{ConsultaDeTemas, DatosDelTema, Pista, ReglasDelTipo, TipoDeElemento, nombre_de_archivo};
use crate::persistencia::{FichaDeCancion, RepositorioDeCanciones, carpeta_del_usuario};
use crate::servicio::bpm_de_la_cache;
use crate::{BuscadorDelCatalogo, FichaDelCatalogo};

/// Resultados que se muestran como maximo en la ventana.
const RESULTADOS_A_MOSTRAR: usize = 500;

/// Tareas del catalogo que pueden tardar con muchos temas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TareaDelCatalogo {
    ReleerEtiquetas,
    CompletarDesdeInternet,
    DescargarCaratulas,
}

impl TareaDelCatalogo {
    /// Origen del menu Actualizar datos: 1 internet, otro releer.
    fn de_origen(origen: i32) -> Self {
        if origen == 1 { Self::CompletarDesdeInternet } else { Self::ReleerEtiquetas }
    }

    /// Desde cuantos temas conviene avisar antes de empezar.
    fn umbral(self) -> usize {
        match self {
            // Lee cada archivo del disco: rapido, pero miles llevan minutos.
            Self::ReleerEtiquetas => 300,
            // Una consulta por segundo (lo que piden los servicios).
            Self::CompletarDesdeInternet | Self::DescargarCaratulas => 30,
        }
    }

    /// Segundos aproximados por tema (en el peor caso).
    fn segundos_por_tema(self) -> f64 {
        match self {
            Self::ReleerEtiquetas => 0.05,
            Self::CompletarDesdeInternet | Self::DescargarCaratulas => crate::servicio::PAUSA_ENTRE_CONSULTAS.as_secs_f64(),
        }
    }

    fn titulo(self) -> String {
        match self {
            Self::ReleerEtiquetas => tr!("Releer etiquetas"),
            Self::CompletarDesdeInternet => tr!("Completar desde internet"),
            Self::DescargarCaratulas => tr!("Carátulas"),
        }
    }
}

/// Minutos (redondeados hacia arriba, al menos 1) que lleva la tarea.
fn minutos_estimados(tarea: TareaDelCatalogo, cantidad: usize) -> u64 {
    ((cantidad as f64 * tarea.segundos_por_tema()) / 60.0).ceil().max(1.0) as u64
}

/// Aviso antes de una tarea larga sobre muchos temas, o None si es corta.
fn aviso_de_demora(tarea: TareaDelCatalogo, cantidad: usize) -> Option<String> {
    if cantidad < tarea.umbral() {
        return None;
    }
    let minutos = minutos_estimados(tarea, cantidad);
    Some(match tarea {
        TareaDelCatalogo::ReleerEtiquetas => tr!(
            "Se van a releer las etiquetas de {} archivo(s): en un catálogo grande puede tardar unos {minutos} minuto(s), según el disco. Mientras tanto se puede seguir usando el reproductor.\n\nPara hacerlo con menos temas, búsquelos o elíjalos antes.\n\n¿Empezar?",
            cantidad,
            minutos = minutos
        ),
        TareaDelCatalogo::CompletarDesdeInternet => tr!(
            "Se van a buscar {} tema(s) en internet, uno por segundo: puede tardar unos {minutos} minuto(s). Se puede detener y aplicar lo encontrado hasta ese momento.\n\nPara hacerlo con menos temas, búsquelos o elíjalos antes.\n\n¿Empezar?",
            cantidad,
            minutos = minutos
        ),
        TareaDelCatalogo::DescargarCaratulas => tr!(
            "Se van a buscar las carátulas de {} tema(s): las que faltan se descargan de a una por segundo y puede tardar hasta unos {minutos} minuto(s). Se detiene al cerrar el catálogo.\n\nPara hacerlo con menos temas, búsquelos o elíjalos antes.\n\n¿Empezar?",
            cantidad,
            minutos = minutos
        ),
    })
}

/// Si la tarea es larga, avisa cuanto puede tardar y pregunta si empezar.
fn confirmar_demora(ventana: &slint::Weak<BuscadorDelCatalogo>, tarea: TareaDelCatalogo, cantidad: usize) -> bool {
    let Some(aviso) = aviso_de_demora(tarea, cantidad) else {
        return true;
    };
    let mut dialogo = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(tarea.titulo())
        .set_description(aviso)
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    dialogo.show() == rfd::MessageDialogResult::Yes
}

/// Los archivos de la tarea, si es corta o el usuario acepta la demora.
fn archivos_confirmados(
    ventana: &slint::Weak<BuscadorDelCatalogo>,
    tarea: TareaDelCatalogo,
    archivos: impl FnOnce(&Controlador) -> Vec<PathBuf>,
) -> Option<Vec<PathBuf>> {
    let mut elegidos = Vec::new();
    con_controlador(|c| elegidos = archivos(c));
    (!elegidos.is_empty() && confirmar_demora(ventana, tarea, elegidos.len())).then_some(elegidos)
}

/// Lo que encontro una busqueda en el catalogo.
pub(crate) struct ResultadoDelCatalogo {
    /// Los primeros archivos que coinciden, por artista, titulo y ruta.
    pub archivos: Vec<PathBuf>,
    /// Cuantos coinciden en total.
    pub total: usize,
}

/// Titulo con que se muestra la ficha: el suyo o el nombre del archivo.
pub(crate) fn titulo_de_la_ficha(audio: &Path, ficha: &FichaDeCancion) -> String {
    ficha.titulo.clone().filter(|titulo| !titulo.trim().is_empty()).unwrap_or_else(|| nombre_de_archivo(audio))
}

/// Datos de la ficha que mira la consulta (sin posicion: no es de la lista).
/// Sin prefijo se busca en el nombre del archivo, no en sus carpetas.
pub(crate) fn datos_de_la_ficha<'a>(nombre: &'a str, ruta: &'a str, ficha: &'a FichaDeCancion, bpm: Option<f64>) -> DatosDelTema<'a> {
    let texto = |dato: &'a Option<String>| dato.as_deref().unwrap_or_default();
    DatosDelTema {
        titulo: texto(&ficha.titulo),
        artista: texto(&ficha.artista),
        genero: texto(&ficha.genero),
        album: texto(&ficha.album),
        tipo: ficha.tipo.map(TipoDeElemento::para_buscar).unwrap_or_default(),
        nota: texto(&ficha.nota),
        archivo: nombre,
        ruta,
        anio: ficha.anio,
        ranking: Some(ficha.ranking),
        bpm: ficha.bpm.or(bpm),
        duracion: ficha.duracion,
        ultima_reproduccion: ficha.ultima_reproduccion.as_deref(),
        posicion: None,
    }
}

/// Si la ficha cumple la consulta (sin consulta, si). `bpm` es el tempo
/// analizado, para los temas sin uno fijado.
pub(crate) fn ficha_coincide(consulta: Option<&ConsultaDeTemas>, audio: &Path, ficha: &FichaDeCancion, bpm: Option<f64>) -> bool {
    consulta.is_none_or(|consulta| {
        let (nombre, ruta) = (nombre_de_archivo(audio), audio.to_string_lossy());
        consulta.coincide(&datos_de_la_ficha(&nombre, &ruta, ficha, bpm))
    })
}

/// Busca en el catalogo los temas que cumplen la consulta del texto (con el
/// texto vacio, todos). Devuelve los primeros `limite`, o el problema si la
/// consulta no se entiende. `bpm_de` da el tempo analizado de un tema.
pub(crate) fn buscar_en_el_catalogo(
    canciones: &RepositorioDeCanciones,
    texto: &str,
    limite: usize,
    ahora: &jiff::Zoned,
    bpm_de: impl Fn(&Path) -> Option<f64>,
) -> Result<ResultadoDelCatalogo, String> {
    let consulta = ConsultaDeTemas::nueva_en(texto, ahora)?;
    if consulta.as_ref().is_some_and(ConsultaDeTemas::usa_la_posicion) {
        return Err(tr!("«#» busca por posición en la lista; en el catálogo no hay posiciones").into());
    }
    let mut encontrados: Vec<(String, String, &Path)> = canciones
        .fichas()
        .filter(|(_, ficha)| !ficha.tipo.solo_para_la_botonera())
        .filter(|(audio, ficha)| ficha_coincide(consulta.as_ref(), audio, ficha, bpm_de(audio)))
        .map(|(audio, ficha)| {
            let artista = ficha.artista.as_deref().unwrap_or_default().to_lowercase();
            (artista, titulo_de_la_ficha(audio, ficha).to_lowercase(), audio)
        })
        .collect();
    let total = encontrados.len();
    encontrados.sort_unstable();
    let archivos = encontrados.into_iter().take(limite).map(|(_, _, audio)| audio.to_path_buf()).collect();
    Ok(ResultadoDelCatalogo { archivos, total })
}


/// La ventana del buscador y lo que muestra.
pub(super) struct BuscadorDeCatalogo {
    ventana: BuscadorDelCatalogo,
    modelo: Rc<VecModel<FichaDelCatalogo>>,
    resultados: Vec<PathBuf>,
    seleccion: BTreeSet<usize>,
    ancla: Option<usize>,
}

impl BuscadorDeCatalogo {
    /// La ventana de Slint, para fijarle el idioma.
    pub(super) fn ventana_slint(&self) -> &BuscadorDelCatalogo {
        &self.ventana
    }

    pub(super) fn nuevo() -> Result<Self, String> {
        let ventana =
            BuscadorDelCatalogo::new().map_err(|error| tr!("No se pudo crear el buscador del catalogo: {error}", error = error))?;
        let modelo = Rc::new(VecModel::default());
        ventana.set_fichas(ModelRc::from(modelo.clone()));
        cablear(&ventana);
        Ok(Self { ventana, modelo, resultados: Vec::new(), seleccion: BTreeSet::new(), ancla: None })
    }

    pub(super) fn ventana(&self) -> &slint::Window {
        self.ventana.window()
    }
}

fn cablear(ventana: &BuscadorDelCatalogo) {
    ventana.on_buscar(|_| con_controlador(Controlador::mostrar_resultados_del_catalogo));
    ventana.on_clic(|indice, control, mayusculas| {
        con_controlador(|c| c.clic_en_el_catalogo(indice as usize, control, mayusculas))
    });
    ventana.on_insertar(|indice| con_controlador(|c| c.insertar_del_catalogo(usize::try_from(indice).ok(), false)));
    ventana.on_agregar_al_final(|| con_controlador(|c| c.insertar_del_catalogo(None, true)));
    ventana.on_cerrar(|| con_controlador(|c| c.cerrar_buscador_del_catalogo()));
    // Con muchos temas, antes de empezar se avisa cuanto puede tardar.
    let debil = ventana.as_weak();
    ventana.on_releer_etiquetas(move || {
        let tarea = TareaDelCatalogo::ReleerEtiquetas;
        if let Some(archivos) = archivos_confirmados(&debil, tarea, Controlador::archivos_elegidos_del_catalogo) {
            con_controlador(|c| c.releer_etiquetas(archivos));
        }
    });
    let debil = ventana.as_weak();
    ventana.on_completar_desde_internet(move || {
        let tarea = TareaDelCatalogo::CompletarDesdeInternet;
        if let Some(archivos) = archivos_confirmados(&debil, tarea, Controlador::archivos_elegidos_del_catalogo) {
            con_controlador(|c| c.completar_desde_internet(archivos));
        }
    });
    let debil = ventana.as_weak();
    ventana.on_descargar_caratulas(move || {
        let tarea = TareaDelCatalogo::DescargarCaratulas;
        if let Some(archivos) = archivos_confirmados(&debil, tarea, Controlador::archivos_elegidos_del_catalogo) {
            con_controlador(|c| {
                c.buscador.ventana.set_aviso(tr!("Buscando las carátulas de {} tema(s)...", archivos.len()).into());
                c.descargar_caratulas(archivos, Controlador::avisar_en_el_catalogo);
            });
        }
    });
    ventana.on_editar(|indice| con_controlador(|c| c.editar_del_catalogo(indice as usize)));
    let debil = ventana.as_weak();
    ventana.on_actualizar(move |indice, origen| {
        let tarea = TareaDelCatalogo::de_origen(origen);
        let del_menu = |c: &Controlador| c.archivos_del_menu_del_catalogo(indice as usize);
        if let Some(archivos) = archivos_confirmados(&debil, tarea, del_menu) {
            con_controlador(|c| super::actualizacion::actualizar_desde(c, archivos, origen));
        }
    });
    let debil = ventana.as_weak();
    ventana.on_descatalogar(move |indice| descatalogar_con_dialogo(&debil, indice as usize));
    ventana.window().on_close_requested(|| {
        con_controlador(|c| c.detener_descarga_de_caratulas());
        CloseRequestResponse::HideWindow
    });
}

/// Pregunta si descatalogar los temas del menu (el de la fila o, si esta
/// elegida, todos los elegidos) y, si se confirma, los quita de la lista y
/// borra sus datos guardados.
fn descatalogar_con_dialogo(ventana: &slint::Weak<BuscadorDelCatalogo>, indice: usize) {
    let mut elegidos = Vec::new();
    con_controlador(|c| {
        elegidos = c
            .archivos_del_menu_del_catalogo(indice)
            .into_iter()
            .map(|archivo| {
                let ficha = c.repositorio_de_canciones.buscar(&archivo).cloned().unwrap_or_default();
                let tema = titulo_de_la_ficha(&archivo, &ficha);
                let tema = match ficha.artista.filter(|artista| !artista.trim().is_empty()) {
                    Some(artista) => format!("{tema} - {artista}"),
                    None => tema,
                };
                (archivo, tema)
            })
            .collect();
    });
    if elegidos.is_empty() {
        return;
    }
    const A_LA_VISTA: usize = 12;
    let mut temas: Vec<String> = elegidos.iter().take(A_LA_VISTA).map(|(_, tema)| format!("• {tema}")).collect();
    if elegidos.len() > A_LA_VISTA {
        temas.push(tr!("... y {} mas", elegidos.len() - A_LA_VISTA));
    }
    let mut dialogo = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(tr!("Descatalogar"))
        .set_description(tr!(
            "{}\n\nSe quitaran de la lista y se borraran sus datos guardados: puntos de mezcla, overlay, titulo, artista, genero, ranking, BPM y ultima reproduccion. Los archivos de audio no se borran.\n\n¿Descatalogar {} tema(s)?",
            temas.join("\n"),
            elegidos.len()
        ))
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    if dialogo.show() == rfd::MessageDialogResult::Yes {
        let archivos: Vec<PathBuf> = elegidos.into_iter().map(|(archivo, _)| archivo).collect();
        con_controlador(|c| c.descatalogar_del_catalogo(&archivos));
    }
}

/// Tema del catalogo encontrado, para los clientes externos (servidor MCP).
pub struct TemaDelCatalogo {
    pub archivo: PathBuf,
    pub titulo: String,
    pub ficha: FichaDeCancion,
    pub en_lista: bool,
    pub existe: bool,
}

impl Controlador {
    /// Busca en el catalogo (como el buscador de la ventana) y devuelve los
    /// primeros `limite` temas y cuantos coinciden en total.
    pub fn temas_del_catalogo(&self, texto: &str, limite: usize) -> Result<(Vec<TemaDelCatalogo>, usize), String> {
        let resultado = buscar_en_el_catalogo(&self.repositorio_de_canciones, texto, limite, &self.ahora_en_la_localidad(), |audio| self.bpm_de(audio))?;
        let en_lista = self.archivos_en_la_lista();
        let temas = resultado
            .archivos
            .into_iter()
            .map(|archivo| {
                let ficha = self.repositorio_de_canciones.buscar(&archivo).cloned().unwrap_or_default();
                TemaDelCatalogo {
                    titulo: titulo_de_la_ficha(&archivo, &ficha),
                    en_lista: en_lista.contains(&Pista::new(&archivo)),
                    existe: archivo.is_file(),
                    ficha,
                    archivo,
                }
            })
            .collect();
        Ok((temas, resultado.total))
    }

    /// Abre (o trae al frente) el buscador, con el foco en el texto.
    pub fn abrir_buscador_del_catalogo(&mut self) -> Result<(), String> {
        self.buscador.ventana.set_aviso("".into());
        self.mostrar_resultados_del_catalogo();
        let ventana = self.buscador.ventana.clone_strong();
        self.mostrar_sobre_la_principal(ventana.window(), || ventana.show())?;
        self.buscador.ventana.invoke_enfocar();
        Ok(())
    }

    /// Los resultados elegidos o, si no hay, todos los que se muestran.
    fn archivos_elegidos_del_catalogo(&self) -> Vec<PathBuf> {
        let buscador = &self.buscador;
        if buscador.seleccion.is_empty() {
            return buscador.resultados.clone();
        }
        buscador.seleccion.iter().filter_map(|indice| buscador.resultados.get(*indice).cloned()).collect()
    }

    /// Vuelve a buscar, si la ventana esta abierta, para mostrar los datos
    /// actualizados.
    pub(super) fn refrescar_buscador_si_esta_abierto(&mut self) {
        if self.buscador.ventana.window().is_visible() {
            self.mostrar_resultados_del_catalogo();
        }
    }

    fn cerrar_buscador_del_catalogo(&mut self) {
        self.detener_descarga_de_caratulas();
        let _ = self.buscador.ventana.hide();
    }

    /// Los temas a los que se aplica el menu de una fila: los elegidos si la
    /// fila es uno de ellos; si no, esa fila.
    fn archivos_del_menu_del_catalogo(&self, indice: usize) -> Vec<PathBuf> {
        if self.buscador.seleccion.contains(&indice) {
            return self.archivos_elegidos_del_catalogo();
        }
        self.buscador.resultados.get(indice).cloned().into_iter().collect()
    }

    /// Abre el editor con ese tema del catalogo, este o no en la lista.
    fn editar_del_catalogo(&mut self, indice: usize) {
        let Some(archivo) = self.buscador.resultados.get(indice).cloned() else { return };
        if !archivo.is_file() {
            self.buscador.ventana.set_aviso(tr!("No existe el archivo: {}", archivo.display()).into());
            return;
        }
        if let Err(error) = self.abrir_editor_con(Pista::new(archivo)) {
            self.buscador.ventana.set_aviso(error.into());
        }
    }

    /// Quita los temas de la lista y borra sus fichas del catalogo.
    fn descatalogar_del_catalogo(&mut self, archivos: &[PathBuf]) {
        let pistas: Vec<Pista> = archivos.iter().map(Pista::new).collect();
        let de_la_lista = self.quitar_pistas(&pistas);
        let borradas = self.repositorio_de_canciones.olvidar_varios(archivos);
        for archivo in archivos {
            self.olvidar_caratula(archivo);
        }
        let aviso = tr!(
            "Se descatalogaron {borradas} tema(s): se borraron sus datos y se quitaron {de_la_lista} pista(s) de la lista",
            borradas = borradas,
            de_la_lista = de_la_lista
        );
        log::info!("{aviso}");
        self.mostrar_estado(&aviso);
        self.mostrar_resultados_del_catalogo();
        self.buscador.ventana.set_aviso(aviso.into());
    }

    /// Avance y resultado de la descarga de carátulas, en el aviso de abajo.
    fn avisar_en_el_catalogo(&mut self, texto: String, terminado: bool) {
        if terminado {
            self.mostrar_estado(&texto);
        }
        self.buscador.ventana.set_aviso(texto.into());
    }

    /// Muestra arriba el tema del ultimo clic (el ancla): sus datos y su
    /// carátula. Sin tema elegido, nada.
    fn mostrar_elegida_del_catalogo(&mut self) {
        let elegida = self.buscador.ancla.filter(|ancla| self.buscador.seleccion.contains(ancla)).and_then(|ancla| self.buscador.resultados.get(ancla).cloned());
        let ventana = &self.buscador.ventana;
        let Some(archivo) = elegida else {
            ventana.set_hay_elegida(false);
            self.pedir_caratula(DestinoDeCaratula::Catalogo, None);
            return;
        };
        let ficha = self.repositorio_de_canciones.buscar(&archivo).cloned().unwrap_or_default();
        let titulo = titulo_de_la_ficha(&archivo, &ficha);
        let encabezado = match ficha.artista.as_deref().filter(|artista| !artista.trim().is_empty()) {
            Some(artista) => format!("{titulo} — {artista}"),
            None => titulo,
        };
        let datos: Vec<String> = [ficha.album.clone(), ficha.anio.map(|anio| anio.to_string()), ficha.genero.clone()]
            .into_iter()
            .flatten()
            .filter(|dato| !dato.trim().is_empty())
            .collect();
        ventana.set_elegida_titulo(encabezado.into());
        ventana.set_elegida_datos(if datos.is_empty() { archivo.display().to_string() } else { datos.join(" · ") }.into());
        ventana.set_elegida_tipo(ficha.tipo.map(TipoDeElemento::nombre).unwrap_or_default().into());
        ventana.set_hay_elegida(true);
        self.pedir_caratula(DestinoDeCaratula::Catalogo, Some(&Pista::new(archivo)));
    }

    /// La carátula del tema elegido en el catalogo.
    pub(super) fn mostrar_caratula_del_catalogo(&self, vista: CaratulaVista) {
        let ventana = &self.buscador.ventana;
        match vista {
            Some(vista) => {
                ventana.set_caratula(vista.chica);
                ventana.set_caratula_ampliada(vista.ampliada);
                ventana.set_origen_de_la_caratula(vista.origen.describir().into());
                ventana.set_hay_caratula(true);
            }
            None => {
                ventana.set_hay_caratula(false);
                ventana.set_caratula(Image::default());
                ventana.set_caratula_ampliada(Image::default());
                ventana.set_origen_de_la_caratula("".into());
            }
        }
    }

    /// Vuelve a buscar con el texto de la ventana y muestra los resultados.
    fn mostrar_resultados_del_catalogo(&mut self) {
        let texto = self.buscador.ventana.get_texto();
        let en_catalogo = self.repositorio_de_canciones.cantidad();
        let resultado = match buscar_en_el_catalogo(&self.repositorio_de_canciones, &texto, RESULTADOS_A_MOSTRAR, &self.ahora_en_la_localidad(), |audio| self.bpm_de(audio)) {
            Ok(resultado) => resultado,
            Err(problema) => {
                self.buscador.ventana.set_resumen(problema.into());
                self.buscador.resultados.clear();
                self.buscador.seleccion.clear();
                self.buscador.ancla = None;
                self.buscador.modelo.set_vec(Vec::new());
                self.buscador.ventana.set_seleccionadas(0);
                return;
            }
        };
        let resumen = match resultado.total {
            _ if texto.trim().is_empty() => tr!("{en_catalogo} tema(s) en el catalogo", en_catalogo = en_catalogo),
            0 => tr!("Ningun tema del catalogo coincide"),
            total if total > resultado.archivos.len() => {
                tr!("{total} tema(s) coinciden; se muestran los primeros {}: afine la busqueda", resultado.archivos.len(), total = total)
            }
            total => tr!("{total} tema(s) coinciden", total = total),
        };
        self.buscador.ventana.set_resumen(resumen.into());
        self.buscador.resultados = resultado.archivos;
        self.buscador.seleccion.clear();
        self.buscador.ancla = None;
        self.mostrar_elegida_del_catalogo();
        let en_lista = self.archivos_en_la_lista();
        let ondas = carpeta_del_usuario().join("ondas");
        let zona = self.zona_de_la_localidad();
        let fichas: Vec<FichaDelCatalogo> = self
            .buscador
            .resultados
            .iter()
            .map(|audio| {
                let ficha = self.repositorio_de_canciones.buscar(audio).cloned().unwrap_or_default();
                FichaDelCatalogo {
                    titulo: titulo_de_la_ficha(audio, &ficha).into(),
                    artista: ficha.artista.unwrap_or_default().into(),
                    tipo: ficha.tipo.map(TipoDeElemento::nombre).unwrap_or_default().into(),
                    genero: ficha.genero.unwrap_or_default().into(),
                    album: ficha.album.unwrap_or_default().into(),
                    anio: ficha.anio.map(|anio| anio.to_string()).unwrap_or_default().into(),
                    duracion: ficha.duracion.map(formatear_minutos).unwrap_or_default().into(),
                    // El fijado, el del analisis de la lista o el que guarda
                    // la cache de la onda (si el tema se analizo alguna vez).
                    bpm: self
                        .bpm_de(audio)
                        .or_else(|| bpm_de_la_cache(audio, &ondas))
                        .map(|bpm| format!("{bpm:.0}"))
                        .unwrap_or_default()
                        .into(),
                    // En la hora de la localidad del HTH, como la lista.
                    sonado: ficha
                        .ultima_reproduccion
                        .as_deref()
                        .map(|momento| reproduccion_para_la_lista(momento, &zona))
                        .unwrap_or_default()
                        .into(),
                    ruta: audio.display().to_string().into(),
                    en_lista: en_lista.contains(&Pista::new(audio)),
                    sin_archivo: !audio.is_file(),
                    seleccionada: false,
                }
            })
            .collect();
        self.buscador.modelo.set_vec(fichas);
        self.buscador.ventana.set_seleccionadas(0);
    }

    fn archivos_en_la_lista(&self) -> HashSet<Pista> {
        self.lista.pistas().iter().cloned().collect()
    }

    /// Clic, Ctrl+clic o Mayus+clic en un resultado, como en la lista.
    fn clic_en_el_catalogo(&mut self, indice: usize, control: bool, mayusculas: bool) {
        if indice >= self.buscador.resultados.len() {
            return;
        }
        let buscador = &mut self.buscador;
        match buscador.ancla {
            Some(ancla) if mayusculas => buscador.seleccion = (ancla.min(indice)..=ancla.max(indice)).collect(),
            _ if control => {
                if !buscador.seleccion.remove(&indice) {
                    buscador.seleccion.insert(indice);
                }
                buscador.ancla = Some(indice);
            }
            _ => {
                buscador.seleccion = BTreeSet::from([indice]);
                buscador.ancla = Some(indice);
            }
        }
        self.refrescar_marcas_del_catalogo();
        self.mostrar_elegida_del_catalogo();
    }

    /// Actualiza en los resultados lo elegido y lo que ya esta en la lista.
    fn refrescar_marcas_del_catalogo(&self) {
        let en_lista = self.archivos_en_la_lista();
        for (indice, audio) in self.buscador.resultados.iter().enumerate() {
            let Some(mut ficha) = self.buscador.modelo.row_data(indice) else { continue };
            let (seleccionada, esta) = (self.buscador.seleccion.contains(&indice), en_lista.contains(&Pista::new(audio)));
            if ficha.seleccionada != seleccionada || ficha.en_lista != esta {
                ficha.seleccionada = seleccionada;
                ficha.en_lista = esta;
                self.buscador.modelo.set_row_data(indice, ficha);
            }
        }
        self.buscador.ventana.set_seleccionadas(self.buscador.seleccion.len() as i32);
    }

    /// Inserta en la lista ese resultado o, sin indice, los elegidos: despues
    /// de la pista seleccionada en la ventana principal o, con `al_final`, al
    /// final. Los que ya estan en la lista no se repiten.
    fn insertar_del_catalogo(&mut self, indice: Option<usize>, al_final: bool) {
        let elegidos: Vec<PathBuf> = match indice {
            Some(indice) => self.buscador.resultados.get(indice).cloned().into_iter().collect(),
            None => self.buscador.seleccion.iter().filter_map(|&indice| self.buscador.resultados.get(indice).cloned()).collect(),
        };
        if elegidos.is_empty() {
            self.buscador.ventana.set_aviso(tr!("Elija uno o mas temas de la lista de resultados.").into());
            return;
        }
        let total = elegidos.len();
        let (existentes, sin_archivo): (Vec<PathBuf>, Vec<PathBuf>) = elegidos.into_iter().partition(|audio| audio.is_file());
        let posicion = al_final.then(|| self.lista.tamano());
        let nuevas = if existentes.is_empty() { Vec::new() } else { self.insertar_pistas(existentes.clone(), posicion, false) };
        let ya_estaban = existentes.len() - nuevas.len();
        let mut aviso = tr!("Se insertaron {} de {total} tema(s) en la lista", nuevas.len(), total = total);
        let mut motivos = Vec::new();
        if ya_estaban > 0 {
            motivos.push(tr!("{ya_estaban} ya estaban", ya_estaban = ya_estaban));
        }
        if !sin_archivo.is_empty() {
            motivos.push(tr!("{} sin archivo (ver el mantenimiento del catalogo)", sin_archivo.len()));
        }
        if !motivos.is_empty() {
            aviso.push_str(&format!(": {}", motivos.join(", ")));
        }
        log::info!("{aviso}");
        self.mostrar_estado(&aviso);
        self.buscador.ventana.set_aviso(aviso.into());
        self.refrescar_marcas_del_catalogo();
    }
}

#[cfg(test)]
mod pruebas_de_demora {
    use super::*;

    #[test]
    fn avisa_solo_las_tareas_largas() {
        assert_eq!(aviso_de_demora(TareaDelCatalogo::CompletarDesdeInternet, 29), None);
        assert_eq!(aviso_de_demora(TareaDelCatalogo::ReleerEtiquetas, 299), None);
        assert!(aviso_de_demora(TareaDelCatalogo::CompletarDesdeInternet, 30).is_some());
        assert!(aviso_de_demora(TareaDelCatalogo::DescargarCaratulas, 30).is_some());
        assert!(aviso_de_demora(TareaDelCatalogo::ReleerEtiquetas, 300).is_some());
    }

    #[test]
    fn estima_los_minutos_hacia_arriba() {
        // 1,1 s por tema: 30 temas son 33 s, que se muestran como 1 minuto.
        assert_eq!(minutos_estimados(TareaDelCatalogo::CompletarDesdeInternet, 30), 1);
        // 1000 temas: 1100 s, 18,3 minutos.
        assert_eq!(minutos_estimados(TareaDelCatalogo::CompletarDesdeInternet, 1000), 19);
        assert_eq!(minutos_estimados(TareaDelCatalogo::ReleerEtiquetas, 5000), 5);
        assert_eq!(TareaDelCatalogo::de_origen(1), TareaDelCatalogo::CompletarDesdeInternet);
        assert_eq!(TareaDelCatalogo::de_origen(0), TareaDelCatalogo::ReleerEtiquetas);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::persistencia::CambiosDeDatos;

    #[test]
    fn busca_por_cualquier_dato_y_ordena_por_artista_y_titulo() {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-buscador-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&carpeta);
        std::fs::create_dir_all(&carpeta).unwrap();
        let mut canciones = RepositorioDeCanciones::new(carpeta.join("canciones.json"));
        let datos = |titulo: &str, artista: &str, genero: &str| CambiosDeDatos {
            titulo: Some(titulo.into()),
            artista: Some(artista.into()),
            genero: Some(genero.into()),
            tipo: Some(TipoDeElemento::deducido_del_genero(Some(genero))),
            ..CambiosDeDatos::default()
        };
        let audio = |nombre: &str| carpeta.join(nombre);
        canciones.cambiar_datos(&audio("b.mp3"), datos("Corazón partío", "Alejandro Sanz", "Pop"));
        canciones.cambiar_datos(&audio("a.mp3"), datos("Amiga mía", "Alejandro Sanz", "Pop"));
        canciones.cambiar_datos(&audio("c.mp3"), datos("Persiana americana", "Soda Stereo", "Rock"));
        canciones.cambiar_datos(&audio("risas.mp3"), datos("Risas", "", "Efecto"));

        let todos = buscar_en_el_catalogo(&canciones, "", 10, &jiff::Zoned::now(), |_| None).unwrap();
        assert_eq!(todos.total, 3, "los efectos no aparecen");
        assert_eq!(todos.archivos, vec![audio("a.mp3"), audio("b.mp3"), audio("c.mp3")]);

        let sanz = buscar_en_el_catalogo(&canciones, "corazon SANZ", 10, &jiff::Zoned::now(), |_| None).unwrap();
        assert_eq!(sanz.archivos, vec![audio("b.mp3")]);
        assert_eq!(buscar_en_el_catalogo(&canciones, "rock", 10, &jiff::Zoned::now(), |_| None).unwrap().archivos, vec![audio("c.mp3")]);
        assert_eq!(buscar_en_el_catalogo(&canciones, "c.mp3", 10, &jiff::Zoned::now(), |_| None).unwrap().archivos, vec![audio("c.mp3")]);
        // El tipo tambien se busca.
        assert_eq!(buscar_en_el_catalogo(&canciones, "tema persiana", 10, &jiff::Zoned::now(), |_| None).unwrap().archivos, vec![audio("c.mp3")]);

        let limitado = buscar_en_el_catalogo(&canciones, "", 2, &jiff::Zoned::now(), |_| None).unwrap();
        assert_eq!((limitado.total, limitado.archivos.len()), (3, 2));

        // Con prefijos: por un dato, por ranking, por tempo y por la ultima
        // reproduccion.
        canciones.cambiar_datos(&audio("a.mp3"), CambiosDeDatos { ranking: Some(5), bpm: Some(Some(96.0)), ..CambiosDeDatos::default() });
        canciones.registrar_reproduccion(&audio("c.mp3"), "2020-01-15T12:00:00Z".into());
        let buscar = |texto: &str| buscar_en_el_catalogo(&canciones, texto, 10, &jiff::Zoned::now(), |_| None).map(|resultado| resultado.archivos);
        assert_eq!(buscar("titulo:amiga"), Ok(vec![audio("a.mp3")]));
        assert_eq!(buscar("artista:sanz -titulo:amiga"), Ok(vec![audio("b.mp3")]));
        assert_eq!(buscar("ranking:5"), Ok(vec![audio("a.mp3")]));
        assert_eq!(buscar("bpm:90..100"), Ok(vec![audio("a.mp3")]));
        assert_eq!(buscar("sonado:2020-01"), Ok(vec![audio("c.mp3")]));
        assert_eq!(buscar("sonado:nunca"), Ok(vec![audio("a.mp3"), audio("b.mp3")]));
        assert_eq!(buscar("sonado:>1a"), Ok(vec![audio("c.mp3")]));
        // Sin prefijo no se busca en las carpetas; con archivo:, si.
        assert_eq!(buscar("raptor-pruebas-buscador"), Ok(vec![]));
        assert_eq!(buscar("archivo:raptor-pruebas-buscador").map(|archivos| archivos.len()), Ok(3));
        assert!(buscar("#5").is_err(), "en el catalogo no hay posiciones");
        assert!(buscar("ranking:muchas").is_err());
        let _ = std::fs::remove_dir_all(&carpeta);
    }
}
