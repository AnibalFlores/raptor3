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

use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, VecModel};

use super::{Controlador, con_controlador, formatear_minutos};
use crate::dominio::{ConsultaDeTemas, DatosDelTema, Pista, ReglasDelTipo, TipoDeElemento, nombre_de_archivo};
use crate::persistencia::{FichaDeCancion, RepositorioDeCanciones};
use crate::{BuscadorDelCatalogo, FichaDelCatalogo};

/// Resultados que se muestran como maximo en la ventana.
const RESULTADOS_A_MOSTRAR: usize = 500;

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
        tipo: ficha.tipo.map(TipoDeElemento::nombre).unwrap_or_default(),
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
    bpm_de: impl Fn(&Path) -> Option<f64>,
) -> Result<ResultadoDelCatalogo, String> {
    let consulta = ConsultaDeTemas::nueva(texto)?;
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

/// Dia local de una fecha ISO 8601 (UTC) del catalogo: "06/11/2026".
fn fecha_local_corta(momento: &str) -> String {
    momento
        .parse::<jiff::Timestamp>()
        .map(|instante| instante.to_zoned(jiff::tz::TimeZone::system()).strftime("%d/%m/%Y").to_string())
        .unwrap_or_default()
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
    ventana.on_releer_etiquetas(|| con_controlador(|c| {
        let archivos = c.archivos_elegidos_del_catalogo();
        c.releer_etiquetas(archivos);
    }));
    ventana.on_completar_desde_musicbrainz(|| con_controlador(|c| {
        let archivos = c.archivos_elegidos_del_catalogo();
        c.completar_desde_musicbrainz(archivos);
    }));
    ventana.window().on_close_requested(|| CloseRequestResponse::HideWindow);
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
        let resultado = buscar_en_el_catalogo(&self.repositorio_de_canciones, texto, limite, |audio| self.bpm_de(audio))?;
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
        let _ = self.buscador.ventana.hide();
    }

    /// Vuelve a buscar con el texto de la ventana y muestra los resultados.
    fn mostrar_resultados_del_catalogo(&mut self) {
        let texto = self.buscador.ventana.get_texto();
        let en_catalogo = self.repositorio_de_canciones.cantidad();
        let resultado = match buscar_en_el_catalogo(&self.repositorio_de_canciones, &texto, RESULTADOS_A_MOSTRAR, |audio| self.bpm_de(audio)) {
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
        let en_lista = self.archivos_en_la_lista();
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
                    anio: ficha.anio.map(|anio| anio.to_string()).unwrap_or_default().into(),
                    duracion: ficha.duracion.map(formatear_minutos).unwrap_or_default().into(),
                    sonado: ficha.ultima_reproduccion.as_deref().map(fecha_local_corta).unwrap_or_default().into(),
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

        let todos = buscar_en_el_catalogo(&canciones, "", 10, |_| None).unwrap();
        assert_eq!(todos.total, 3, "los efectos no aparecen");
        assert_eq!(todos.archivos, vec![audio("a.mp3"), audio("b.mp3"), audio("c.mp3")]);

        let sanz = buscar_en_el_catalogo(&canciones, "corazon SANZ", 10, |_| None).unwrap();
        assert_eq!(sanz.archivos, vec![audio("b.mp3")]);
        assert_eq!(buscar_en_el_catalogo(&canciones, "rock", 10, |_| None).unwrap().archivos, vec![audio("c.mp3")]);
        assert_eq!(buscar_en_el_catalogo(&canciones, "c.mp3", 10, |_| None).unwrap().archivos, vec![audio("c.mp3")]);
        // El tipo tambien se busca.
        assert_eq!(buscar_en_el_catalogo(&canciones, "tema persiana", 10, |_| None).unwrap().archivos, vec![audio("c.mp3")]);

        let limitado = buscar_en_el_catalogo(&canciones, "", 2, |_| None).unwrap();
        assert_eq!((limitado.total, limitado.archivos.len()), (3, 2));

        // Con prefijos: por un dato, por ranking, por tempo y por la ultima
        // reproduccion.
        canciones.cambiar_datos(&audio("a.mp3"), CambiosDeDatos { ranking: Some(5), bpm: Some(Some(96.0)), ..CambiosDeDatos::default() });
        canciones.registrar_reproduccion(&audio("c.mp3"), "2020-01-15T12:00:00Z".into());
        let buscar = |texto: &str| buscar_en_el_catalogo(&canciones, texto, 10, |_| None).map(|resultado| resultado.archivos);
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
