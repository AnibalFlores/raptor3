//! Selector de avisos: busca en el catalogo de la emisora los avisos para la
//! tanda o la orden que se edita, en lugar de abrir directamente el
//! explorador de archivos.
//!
//! Por defecto muestra los elementos con tipo de tanda (Comercial,
//! Publicidad, Anuncio, Podcast o Seco). Advierte cuantos elementos del catalogo no
//! tienen tipo y permite asignarlo a varios a la vez, para que el operador
//! mantenga clasificado todo lo almacenado. Solo se agregan los que tienen
//! archivo y un tipo valido; lo demas queda marcado con su motivo.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, VecModel};

use super::buscador::{ficha_coincide, titulo_de_la_ficha};
use super::{Controlador, con_controlador, formatear_minutos};
use crate::dominio::{ConsultaDeTemas, EXTENSIONES_SOPORTADAS, Pista, ReglasDelTipo, TipoDeElemento, nombre_de_archivo};
use crate::persistencia::{CambiosDeDatos, FichaDeCancion, RepositorioDeCanciones};
use crate::{AvisoDelCatalogo, SelectorDeAvisos};

/// Resultados que se muestran como maximo en la ventana.
const RESULTADOS_A_MOSTRAR: usize = 500;
/// Tipos que se pueden asignar desde el selector, en el orden de la ventana.
const TIPOS_ASIGNABLES: [TipoDeElemento; 5] = TipoDeElemento::DE_TANDA;

/// A que se agregan los avisos elegidos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DestinoDeAvisos {
    /// La tanda del formulario de la pauta.
    Tanda,
    /// La orden del formulario de ordenes de publicidad.
    Orden,
}

/// Que temas del catalogo muestra el selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FiltroDeAvisos {
    /// Los de tipo de tanda (Comercial, Publicidad, Anuncio, Podcast o Seco).
    Avisos,
    /// Los que no tienen tipo: hay que clasificarlos.
    SinTipo,
    /// Todo el catalogo menos los efectos (que solo van en la botonera).
    Todo,
}

impl FiltroDeAvisos {
    fn desde_indice(indice: i32) -> Self {
        match indice {
            1 => Self::SinTipo,
            2 => Self::Todo,
            _ => Self::Avisos,
        }
    }

    fn admite(self, ficha: &FichaDeCancion) -> bool {
        match self {
            Self::Avisos => ficha.tipo.va_en_tanda(),
            Self::SinTipo => ficha.tipo.is_none(),
            Self::Todo => !ficha.tipo.solo_para_la_botonera(),
        }
    }
}

/// Lo que encontro una busqueda de avisos en el catalogo.
#[derive(Debug, PartialEq)]
pub(crate) struct AvisosDelCatalogo {
    /// Los primeros que coinciden, por anunciante (artista) y titulo.
    pub archivos: Vec<PathBuf>,
    /// Cuantos coinciden en total.
    pub total: usize,
    /// Elementos del catalogo con tipo de tanda (sin importar la busqueda).
    pub de_tanda: usize,
    /// Elementos del catalogo sin tipo (sin importar la busqueda).
    pub sin_tipo: usize,
}

/// Busca en el catalogo los temas del filtro con todas las palabras del
/// texto. Cuenta ademas los avisos y los elementos sin tipo de todo el
/// catalogo, para advertir lo que falta clasificar.
pub(crate) fn buscar_avisos(canciones: &RepositorioDeCanciones, texto: &str, filtro: FiltroDeAvisos, limite: usize) -> AvisosDelCatalogo {
    // Con una consulta que no se entiende no coincide ninguno.
    let (consulta, entendida) = match ConsultaDeTemas::nueva(texto) {
        Ok(consulta) => (consulta, true),
        Err(_) => (None, false),
    };
    let (mut de_tanda, mut sin_tipo) = (0, 0);
    let mut encontrados: Vec<(String, String, &Path)> = Vec::new();
    for (audio, ficha) in canciones.fichas() {
        if FiltroDeAvisos::Avisos.admite(ficha) {
            de_tanda += 1;
        }
        if FiltroDeAvisos::SinTipo.admite(ficha) {
            sin_tipo += 1;
        }
        if entendida && filtro.admite(ficha) && ficha_coincide(consulta.as_ref(), audio, ficha, None) {
            let artista = ficha.artista.as_deref().unwrap_or_default().to_lowercase();
            encontrados.push((artista, titulo_de_la_ficha(audio, ficha).to_lowercase(), audio));
        }
    }
    let total = encontrados.len();
    encontrados.sort_unstable();
    let archivos = encontrados.into_iter().take(limite).map(|(_, _, audio)| audio.to_path_buf()).collect();
    AvisosDelCatalogo { archivos, total, de_tanda, sin_tipo }
}

/// La ventana del selector y lo que muestra.
pub(super) struct VentanaDeAvisos {
    ventana: SelectorDeAvisos,
    modelo: Rc<VecModel<AvisoDelCatalogo>>,
    resultados: Vec<PathBuf>,
    seleccion: BTreeSet<usize>,
    ancla: Option<usize>,
    destino: DestinoDeAvisos,
}

impl VentanaDeAvisos {
    pub(super) fn nueva() -> Result<Self, String> {
        let ventana = SelectorDeAvisos::new().map_err(|error| tr!("No se pudo crear el selector de avisos: {error}", error = error))?;
        let modelo = Rc::new(VecModel::default());
        ventana.set_avisos(ModelRc::from(modelo.clone()));
        cablear(&ventana);
        Ok(Self { ventana, modelo, resultados: Vec::new(), seleccion: BTreeSet::new(), ancla: None, destino: DestinoDeAvisos::Tanda })
    }

    pub(super) fn ventana(&self) -> &slint::Window {
        self.ventana.window()
    }
}

fn cablear(ventana: &SelectorDeAvisos) {
    ventana.on_buscar(|_| con_controlador(Controlador::mostrar_avisos_del_catalogo));
    ventana.on_cambiar_filtro(|_| con_controlador(Controlador::mostrar_avisos_del_catalogo));
    ventana.on_clic(|indice, control, mayusculas| con_controlador(|c| c.clic_en_los_avisos(indice as usize, control, mayusculas)));
    ventana.on_agregar(|indice| con_controlador(|c| c.agregar_avisos_elegidos(usize::try_from(indice).ok())));
    ventana.on_asignar_tipo(|| con_controlador(Controlador::asignar_tipo_a_los_elegidos));
    let debil = ventana.as_weak();
    ventana.on_desde_archivo(move || {
        let mut dialogo =
            rfd::FileDialog::new().set_title(&tr!("Avisos fuera del catalogo")).add_filter(&tr!("Archivos de audio"), EXTENSIONES_SOPORTADAS);
        if let Some(ventana) = debil.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        if let Some(archivos) = dialogo.pick_files() {
            con_controlador(|c| c.agregar_archivos_de_aviso(archivos));
        }
    });
    ventana.on_cerrar(|| con_controlador(|c| {
        let _ = c.avisos_del_catalogo.ventana.hide();
    }));
    ventana.window().on_close_requested(|| CloseRequestResponse::HideWindow);
}

impl Controlador {
    /// Abre (o trae al frente) el selector para agregar avisos a la tanda o
    /// a la orden del formulario.
    pub(super) fn abrir_selector_de_avisos(&mut self, destino: DestinoDeAvisos) {
        self.avisos_del_catalogo.destino = destino;
        self.avisos_del_catalogo.ventana.set_aviso("".into());
        self.avisos_del_catalogo.ventana.set_aviso_es_error(false);
        self.mostrar_avisos_del_catalogo();
        let ventana = self.avisos_del_catalogo.ventana.clone_strong();
        match self.mostrar_sobre_la_principal(ventana.window(), || ventana.show()) {
            Ok(()) => ventana.invoke_enfocar(),
            Err(error) => self.avisar_en_el_destino(&error, true),
        }
    }

    /// Nombre de lo que se edita, para el titulo: «la tanda «Tanda A»».
    fn nombre_del_destino(&self) -> String {
        let (articulo, nombre) = match self.avisos_del_catalogo.destino {
            DestinoDeAvisos::Tanda => (tr!("la tanda"), self.pauta.nombre_en_el_formulario()),
            DestinoDeAvisos::Orden => (tr!("la orden"), self.ventana_de_ordenes.nombre_en_el_formulario()),
        };
        if nombre.is_empty() { format!("{articulo} nueva") } else { format!("{articulo} «{nombre}»") }
    }

    fn avisos_del_destino(&self) -> &[PathBuf] {
        match self.avisos_del_catalogo.destino {
            DestinoDeAvisos::Tanda => self.pauta.avisos_en_el_formulario(),
            DestinoDeAvisos::Orden => self.ventana_de_ordenes.avisos_en_el_formulario(),
        }
    }

    /// Agrega los archivos al borrador del destino (sin repetir) y devuelve
    /// cuantos entraron.
    fn sumar_al_destino(&mut self, archivos: Vec<PathBuf>) -> usize {
        let agregados = match self.avisos_del_catalogo.destino {
            DestinoDeAvisos::Tanda => self.pauta.sumar_avisos(archivos),
            DestinoDeAvisos::Orden => self.ventana_de_ordenes.sumar_avisos(archivos),
        };
        self.refrescar_avisos_del_destino();
        agregados
    }

    /// Vuelve a mostrar los avisos del formulario del destino (y la tabla de
    /// la pauta, que marca las tandas con problemas).
    fn refrescar_avisos_del_destino(&mut self) {
        match self.avisos_del_catalogo.destino {
            DestinoDeAvisos::Tanda => {
                self.refrescar_avisos_de_la_pauta();
                self.refrescar_tabla_de_la_pauta();
            }
            DestinoDeAvisos::Orden => self.refrescar_avisos_de_la_orden(),
        }
    }

    fn avisar_en_el_destino(&self, aviso: &str, es_error: bool) {
        match self.avisos_del_catalogo.destino {
            DestinoDeAvisos::Tanda => self.avisar_en_la_pauta(aviso, es_error),
            DestinoDeAvisos::Orden => self.avisar_en_ordenes(aviso, es_error),
        }
    }

    fn avisar_en_el_selector(&self, aviso: &str, es_error: bool) {
        self.avisos_del_catalogo.ventana.set_aviso(aviso.into());
        self.avisos_del_catalogo.ventana.set_aviso_es_error(es_error);
    }

    /// Por que ese archivo no puede agregarse como aviso, si no puede.
    fn motivo_para_no_agregar(&self, archivo: &Path) -> Option<String> {
        if !archivo.is_file() {
            return Some(tr!("sin archivo").into());
        }
        match self.repositorio_de_canciones.tipo_de(archivo) {
            None => Some(tr!("sin tipo: asignele uno").into()),
            Some(tipo) if !tipo.va_en_tanda() => Some(tr!("{}: no va en tandas", tipo.nombre_visible())),
            Some(_) => None,
        }
    }

    /// Vuelve a buscar con el texto y el filtro de la ventana.
    fn mostrar_avisos_del_catalogo(&mut self) {
        let ventana = &self.avisos_del_catalogo.ventana;
        let (texto, filtro) = (ventana.get_texto(), FiltroDeAvisos::desde_indice(ventana.get_filtro()));
        let encontrados = buscar_avisos(&self.repositorio_de_canciones, &texto, filtro, RESULTADOS_A_MOSTRAR);
        let mut resumen = match encontrados.total {
            0 if texto.trim().is_empty() => tr!("Ningun tema del catalogo con ese filtro"),
            0 => tr!("Ningun tema coincide"),
            total if total > encontrados.archivos.len() => {
                tr!("{total} tema(s) coinciden; se muestran los primeros {}: afine la busqueda", encontrados.archivos.len(), total = total)
            }
            total => format!("{total} tema(s)"),
        };
        resumen.push_str(&tr!(". En el catalogo hay {} aviso(s) con tipo de tanda", encontrados.de_tanda));
        if encontrados.sin_tipo > 0 {
            resumen.push_str(&tr!(" y {} elemento(s) SIN TIPO: reviselos con el filtro «Sin tipo» y asigneles uno (un aviso sin tipo no puede ir en tandas ni ordenes)", encontrados.sin_tipo));
        }
        resumen.push('.');
        ventana.set_resumen(resumen.into());
        ventana.set_faltan_tipos(encontrados.sin_tipo > 0);
        ventana.set_destino(self.nombre_del_destino().into());
        self.avisos_del_catalogo.resultados = encontrados.archivos;
        self.avisos_del_catalogo.seleccion.clear();
        self.avisos_del_catalogo.ancla = None;
        self.refrescar_filas_de_avisos(true);
    }

    /// Arma (o, sin `completo`, solo corrige marcas y estados) las filas.
    fn refrescar_filas_de_avisos(&self, completo: bool) {
        let ya_estan: Vec<Pista> = self.avisos_del_destino().iter().map(Pista::new).collect();
        let selector = &self.avisos_del_catalogo;
        let filas: Vec<AvisoDelCatalogo> = selector
            .resultados
            .iter()
            .enumerate()
            .map(|(indice, audio)| {
                let ficha = self.repositorio_de_canciones.buscar(audio).cloned().unwrap_or_default();
                let problema = self.motivo_para_no_agregar(audio);
                let ya_agregado = ya_estan.contains(&Pista::new(audio));
                let estado = match (&problema, ya_agregado) {
                    (Some(problema), _) => problema.clone(),
                    (None, true) => tr!("ya esta"),
                    (None, false) => String::new(),
                };
                AvisoDelCatalogo {
                    titulo: titulo_de_la_ficha(audio, &ficha).into(),
                    artista: ficha.artista.unwrap_or_default().into(),
                    tipo: ficha.tipo.map_or("sin tipo", TipoDeElemento::nombre).into(),
                    genero: ficha.genero.unwrap_or_default().into(),
                    duracion: self.duracion_de_efecto(audio).map(formatear_minutos).unwrap_or_default().into(),
                    ruta: audio.display().to_string().into(),
                    estado: estado.into(),
                    con_problema: problema.is_some(),
                    ya_agregado,
                    seleccionado: selector.seleccion.contains(&indice),
                }
            })
            .collect();
        if completo || selector.modelo.row_count() != filas.len() {
            selector.modelo.set_vec(filas);
        } else {
            for (indice, fila) in filas.into_iter().enumerate() {
                if selector.modelo.row_data(indice).as_ref() != Some(&fila) {
                    selector.modelo.set_row_data(indice, fila);
                }
            }
        }
        selector.ventana.set_seleccionados(selector.seleccion.len() as i32);
    }

    /// Clic, Ctrl+clic o Mayus+clic en un resultado, como en la lista.
    fn clic_en_los_avisos(&mut self, indice: usize, control: bool, mayusculas: bool) {
        let selector = &mut self.avisos_del_catalogo;
        if indice >= selector.resultados.len() {
            return;
        }
        match selector.ancla {
            Some(ancla) if mayusculas => selector.seleccion = (ancla.min(indice)..=ancla.max(indice)).collect(),
            _ if control => {
                if !selector.seleccion.remove(&indice) {
                    selector.seleccion.insert(indice);
                }
                selector.ancla = Some(indice);
            }
            _ => {
                selector.seleccion = BTreeSet::from([indice]);
                selector.ancla = Some(indice);
            }
        }
        self.refrescar_filas_de_avisos(false);
    }

    /// Los resultados elegidos: ese indice o, sin indice, la seleccion.
    fn avisos_elegidos(&self, indice: Option<usize>) -> Vec<PathBuf> {
        let selector = &self.avisos_del_catalogo;
        match indice {
            Some(indice) => selector.resultados.get(indice).cloned().into_iter().collect(),
            None => selector.seleccion.iter().filter_map(|&indice| selector.resultados.get(indice).cloned()).collect(),
        }
    }

    /// Agrega al destino los avisos elegidos que tienen archivo y tipo de
    /// tanda; los demas quedan afuera con su motivo.
    fn agregar_avisos_elegidos(&mut self, indice: Option<usize>) {
        let elegidos = self.avisos_elegidos(indice);
        if elegidos.is_empty() {
            return self.avisar_en_el_selector(&tr!("Elija uno o mas avisos de la lista."), true);
        }
        let total = elegidos.len();
        let (validos, rechazados): (Vec<PathBuf>, Vec<PathBuf>) =
            elegidos.into_iter().partition(|archivo| self.motivo_para_no_agregar(archivo).is_none());
        let cantidad_validos = validos.len();
        let agregados = self.sumar_al_destino(validos);
        let mut mensaje = tr!("Se agregaron {agregados} de {total} aviso(s) a {}", self.nombre_del_destino(), agregados = agregados, total = total);
        let mut motivos = Vec::new();
        if cantidad_validos > agregados {
            motivos.push(tr!("{} ya estaban", cantidad_validos - agregados));
        }
        let detalle: Vec<String> = rechazados
            .iter()
            .take(3)
            .map(|archivo| format!("{} ({})", nombre_de_archivo(archivo), self.motivo_para_no_agregar(archivo).unwrap_or_default()))
            .collect();
        if !rechazados.is_empty() {
            let resto = if rechazados.len() > detalle.len() { tr!(" y {} mas", rechazados.len() - detalle.len()) } else { String::new() };
            motivos.push(tr!("{} no se agregaron: {}{resto}", rechazados.len(), detalle.join(", "), resto = resto));
        }
        if !motivos.is_empty() {
            mensaje.push_str(&format!("; {}", motivos.join("; ")));
        }
        mensaje.push('.');
        let hubo_rechazos = !rechazados.is_empty();
        if hubo_rechazos {
            mensaje.push_str(&tr!(" Asigneles un tipo con el selector de abajo y vuelva a agregarlos."));
        }
        self.avisar_en_el_selector(&mensaje, hubo_rechazos);
        if agregados > 0 {
            self.avisar_en_el_destino(&tr!("{agregados} aviso(s) agregado(s) del catalogo; se guardan con el boton de abajo.", agregados = agregados), false);
        }
        self.refrescar_filas_de_avisos(false);
    }

    /// Asigna en el catalogo el tipo del selector a los avisos elegidos.
    fn asignar_tipo_a_los_elegidos(&mut self) {
        let elegidos = self.avisos_elegidos(None);
        if elegidos.is_empty() {
            return;
        }
        let opcion = self.avisos_del_catalogo.ventana.get_tipo_a_asignar().clamp(0, TIPOS_ASIGNABLES.len() as i32 - 1);
        let tipo = TIPOS_ASIGNABLES[opcion as usize];
        for archivo in &elegidos {
            self.cambiar_datos(&Pista::new(archivo), CambiosDeDatos { tipo: Some(Some(tipo)), ..CambiosDeDatos::default() });
        }
        self.invalidar_pauta();
        log::info!("Tipo {} asignado a {} aviso(s) del catalogo", tipo.nombre(), elegidos.len());
        // La busqueda se repite: con el filtro «Sin tipo», los asignados salen de la lista.
        self.mostrar_avisos_del_catalogo();
        self.refrescar_avisos_del_destino();
        self.avisar_en_el_selector(&tr!("Tipo {} asignado a {} aviso(s) en el catalogo.", tipo.nombre_visible(), elegidos.len()), false);
    }

    /// Agrega avisos elegidos con el explorador (todavia fuera del
    /// catalogo). Entran aunque no tengan tipo, pero se advierte: la tanda
    /// o la orden no se guarda hasta asignarselo.
    fn agregar_archivos_de_aviso(&mut self, archivos: Vec<PathBuf>) {
        let sin_tipo: Vec<String> = archivos
            .iter()
            .filter(|archivo| self.motivo_para_no_agregar(archivo).is_some())
            .map(|archivo| nombre_de_archivo(archivo))
            .collect();
        let agregados = self.sumar_al_destino(archivos);
        let mut mensaje = tr!("{agregados} aviso(s) agregado(s) desde archivos a {}.", self.nombre_del_destino(), agregados = agregados);
        if !sin_tipo.is_empty() {
            mensaje.push_str(&tr!(" {} sin un tipo valido ({}): eligalos aqui con el filtro «Sin tipo» y use Asignar tipo; asi tambien quedan clasificados en el catalogo.", sin_tipo.len(), sin_tipo.join(", ")));
        }
        self.avisar_en_el_selector(&mensaje, !sin_tipo.is_empty());
        self.avisar_en_el_destino(&mensaje, !sin_tipo.is_empty());
        self.refrescar_filas_de_avisos(false);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn filtra_los_avisos_y_cuenta_los_elementos_sin_tipo() {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-avisos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&carpeta);
        std::fs::create_dir_all(&carpeta).unwrap();
        let mut canciones = RepositorioDeCanciones::new(carpeta.join("canciones.json"));
        let datos = |titulo: &str, artista: &str, tipo: Option<TipoDeElemento>| CambiosDeDatos {
            titulo: Some(titulo.into()),
            artista: Some(artista.into()),
            // El genero ya no decide nada: puede ser cualquiera.
            genero: Some("Rock".into()),
            tipo: Some(tipo),
            ..CambiosDeDatos::default()
        };
        let audio = |nombre: &str| carpeta.join(nombre);
        use TipoDeElemento::*;
        canciones.cambiar_datos(&audio("sol.mp3"), datos("Ofertas", "Supermercado Sol", Some(Comercial)));
        canciones.cambiar_datos(&audio("luna.mp3"), datos("Promo", "Supermercado Luna", Some(Publicidad)));
        canciones.cambiar_datos(&audio("voz.mp3"), datos("Hora", "Emisora", Some(Seco)));
        canciones.cambiar_datos(&audio("risas.mp3"), datos("Risas", "", Some(Efecto)));
        canciones.cambiar_datos(&audio("tema.mp3"), datos("Persiana americana", "Soda Stereo", Some(Tema)));
        canciones.cambiar_datos(&audio("nuevo.mp3"), datos("Aviso nuevo", "Farmacia", None));

        let avisos = buscar_avisos(&canciones, "", FiltroDeAvisos::Avisos, 10);
        assert_eq!(avisos.archivos, vec![audio("voz.mp3"), audio("luna.mp3"), audio("sol.mp3")], "por anunciante");
        assert_eq!((avisos.de_tanda, avisos.sin_tipo), (3, 1));

        let sin_tipo = buscar_avisos(&canciones, "", FiltroDeAvisos::SinTipo, 10);
        assert_eq!(sin_tipo.archivos, vec![audio("nuevo.mp3")]);

        let todo = buscar_avisos(&canciones, "", FiltroDeAvisos::Todo, 10);
        assert_eq!(todo.total, 5, "los efectos no aparecen");

        let sol = buscar_avisos(&canciones, "supermercado sol", FiltroDeAvisos::Avisos, 10);
        assert_eq!(sol.archivos, vec![audio("sol.mp3")]);
        assert_eq!(sol.sin_tipo, 1, "los conteos son de todo el catalogo");
        let _ = std::fs::remove_dir_all(&carpeta);
    }
}
