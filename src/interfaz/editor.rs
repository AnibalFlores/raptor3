//! Editor de parametros de una pista: forma de onda, ventana de overlay,
//! puntos de mezcla (inicio, fin, intro y outro) y datos de la cancion, con
//! preescucha.
//!
//! Los puntos de mezcla y la ventana salen solos del analisis del tema; el
//! editor los muestra como automaticos y, si el usuario los cambia, quedan
//! como manuales. Volver al automatico (o usar la ventana sugerida) los
//! devuelve a lo que propone el analisis.
//!
//! La onda se calcula en otro hilo (o se lee de la cache) y se dibuja en un
//! buffer de pixeles; las marcas se dibujan y arrastran en Slint, asi que
//! moverlas no obliga a redibujar la onda. La preescucha usa una voz propia
//! del mezclador y solo se permite con la musica detenida o en pausa, salvo
//! que se elija otra salida de audio para ella (ver `salida_de_preescucha`).
//! Tiene su propio volumen, que se aplica al tema y al efecto de prueba.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::winit_030::WinitWindowAccessor;
use slint::{CloseRequestResponse, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use super::{Controlador, con_controlador, etiqueta, programar_en_interfaz, recorte_de_efecto};
use crate::audio::{FormaDeOnda, anio_valido};
use crate::dominio::{
    Pista, PuntosDeMezcla, ReglasDelTipo, TipoDeElemento, VentanaDeOverlay, formatear_tiempo, nombre_de_archivo,
    parsear_tiempo,
};
use crate::persistencia::{
    BPM_MAXIMO, BPM_MINIMO, CambiosDeDatos, GeometriaDeVentana, RANKING_MAXIMO, carpeta_del_usuario,
};
use crate::servicio::{AjustesDeEfecto, DatosDeMezcla, GestorDeOverlay, SugerenciaDeOverlay};
use crate::{EditorDePista, MarcaDeEdicion};

// Largo de la ventana de overlay que se crea al fijar uno de sus extremos.
const VENTANA_INICIAL: f64 = 10.0;
// Lo minimo que se puede acercar la vista, en segundos.
const VISTA_MINIMA: f64 = 1.0;
// Tamano del editor la primera vez que se abre (pixeles logicos).
const ANCHO_INICIAL_DEL_EDITOR: f32 = 1180.0;
const ALTO_INICIAL_DEL_EDITOR: f32 = 620.0;
const COLOR_CLARO: [u8; 3] = [40, 105, 160];
const COLOR_OSCURO: [u8; 3] = [110, 180, 225];

/// Pista abierta en el editor.
pub(super) struct Edicion {
    pista: Pista,
    onda: Option<Rc<FormaDeOnda>>,
    /// Distingue la carga de onda vigente de una de otra pista anterior.
    generacion: u64,
    /// Archivos de efectos de la lista desplegable de prueba.
    efectos: Vec<PathBuf>,
    /// Efectos que se pueden fijar al tema: los de la botonera y, si el
    /// fijado no esta en ella, ese archivo. La posicion 0 del combo es
    /// "(ninguno)" y la i corresponde a `fijables[i - 1]`.
    fijables: Vec<PathBuf>,
    /// Parametros con los que se dibujo la onda por ultima vez: ancho y alto
    /// en pixeles, vista y tema. Si no cambian, no se vuelve a dibujar.
    dibujada: Option<(u32, u32, f32, f32, bool)>,
    /// Segundos que se saltean al final; el fin se muestra al conocer la
    /// duracion.
    recorte_final: f64,
    /// Lo que propone el analisis, cuando ya esta la onda.
    automaticos: Option<PuntosDeMezcla>,
    sugerencia: Option<SugerenciaDeOverlay>,
    /// Tempo que estima el analisis, cuando ya esta la onda.
    bpm_automatico: Option<f64>,
}

// El efecto de prueba se programa en su momento exacto con esta antelacion.
const PROGRAMAR_EFECTO_CON: f64 = 0.3;

/// Preescucha en curso.
pub(super) struct Preescucha {
    hasta: f64,
    /// Ganancia de la nivelacion del tema, para aplicar el volumen de la
    /// preescucha si se mueve mientras suena.
    nivelacion: f64,
    efecto: Option<(f64, PathBuf)>,
    /// Ya sono el efecto de prueba: se corta con la preescucha.
    efecto_lanzado: bool,
}

pub(super) fn cablear(editor: &EditorDePista) {
    editor.on_pedir_onda(|_, _| con_controlador(|c| c.redibujar_onda()));
    editor.on_campo_editado(|marca, texto| con_controlador(|c| c.editar_campo(marca, &texto)));
    editor.on_guardar(|| con_controlador(|c| c.guardar_edicion()));
    editor.on_buscar_en_musicbrainz(|| con_controlador(|c| {
        if let Some(archivo) = c.edicion.as_ref().map(|edicion| edicion.pista.archivo().to_path_buf()) {
            c.completar_desde_musicbrainz(vec![archivo]);
        }
    }));
    editor.on_cancelar(|| con_controlador(|c| c.cerrar_editor()));
    editor.on_zoom(|factor, centro| con_controlador(|c| c.zoom_de_onda(f64::from(factor), f64::from(centro))));
    editor.on_preescuchar(|desde, hasta, efecto_en| {
        con_controlador(|c| c.preescuchar(f64::from(desde), f64::from(hasta), f64::from(efecto_en)))
    });
    editor.on_detener_preescucha(|| con_controlador(|c| c.detener_preescucha()));
    editor.on_salida_de_preescucha_elegida(|posicion| {
        con_controlador(|c| c.elegir_salida_de_preescucha(usize::try_from(posicion).unwrap_or(0)))
    });
    editor.on_buscar_salidas(|| con_controlador(|c| c.buscar_salidas_de_preescucha()));
    editor.on_volumen_de_preescucha_ajustado(|volumen, guardar| {
        con_controlador(|c| c.ajustar_volumen_de_preescucha(f64::from(volumen), guardar))
    });
    editor.on_volver_al_automatico(|| con_controlador(|c| c.volver_al_automatico()));
    editor.on_usar_overlay_sugerido(|| con_controlador(|c| c.usar_overlay_sugerido()));
    editor.on_calcular_bpm(|| con_controlador(|c| c.calcular_bpm()));
    editor.on_multiplicar_bpm(|factor| con_controlador(|c| c.multiplicar_bpm(f64::from(factor))));
    let debil = editor.as_weak();
    editor.on_tipo_elegido(move |tipo, anterior| {
        // La confirmacion se pide cuando ya se cerro la lista del combo: con
        // la lista abierta, Windows crea el dialogo oculto y el programa queda
        // esperandolo. Y antes de tomar el controlador: el dialogo no debe
        // frenar la reproduccion.
        let debil = debil.clone();
        slint::Timer::single_shot(ESPERA_TRAS_EL_COMBO, move || {
            let Some(editor) = debil.upgrade() else {
                return;
            };
            let tipo = tipo_del_combo(tipo);
            if confirmar_tipo_sin_mezcla(&editor, tipo) == Some(false) {
                editor.set_tipo(anterior);
                editor.set_tipo_anterior(anterior);
                return;
            }
            con_controlador(|c| c.al_elegir_tipo(tipo));
        });
    });
    editor.on_efecto_ajustado(|| {
        con_controlador(|c| {
            let editor = &c.editor;
            let ajustes = AjustesDeEfecto {
                atenuar: editor.get_atenuar(),
                inteligente: editor.get_efecto_inteligente(),
                nivel_de_musica: f64::from(editor.get_nivel_de_musica()),
                volumen_db: (f64::from(editor.get_volumen_de_efecto()) * 2.0).round() / 2.0,
                anticipacion: (f64::from(editor.get_anticipacion()) * 100.0).round() / 100.0,
                recuperacion: (f64::from(editor.get_recuperacion()) * 10.0).round() / 10.0,
                separacion_db: f64::from(editor.get_separacion()).round(),
            };
            c.cambiar_ajustes_de_efecto(ajustes);
        })
    });
    editor.window().on_close_requested(|| {
        con_controlador(|c| c.al_cerrarse_el_editor());
        CloseRequestResponse::HideWindow
    });
}

/// Generos que el editor sugiere: todos los que ya existen en el catalogo,
/// en orden alfabetico. Asi la lista crece con los que va usando el usuario.
/// Los que solo difieren en mayusculas o acentos se muestran una sola vez,
/// con la forma mas usada.
pub(super) fn generos_del_catalogo(canciones: &crate::persistencia::RepositorioDeCanciones) -> Vec<String> {
    use std::collections::HashMap;
    let mut formas_por_genero: HashMap<String, HashMap<String, usize>> = HashMap::new();
    for (_, ficha) in canciones.fichas() {
        let Some(genero) = ficha.genero.as_deref().map(str::trim).filter(|genero| !genero.is_empty()) else {
            continue;
        };
        *formas_por_genero
            .entry(crate::dominio::simplificar_nombre(genero))
            .or_default()
            .entry(genero.to_string())
            .or_insert(0) += 1;
    }
    let mut generos: Vec<(String, String)> = formas_por_genero
        .into_iter()
        .filter_map(|(clave, formas)| {
            formas
                .into_iter()
                .max_by(|(una_forma, unos), (otra_forma, otros)| unos.cmp(otros).then_with(|| otra_forma.cmp(una_forma)))
                .map(|(forma, _)| (clave, forma))
        })
        .collect();
    generos.sort();
    generos.into_iter().map(|(_, forma)| forma).collect()
}

/// Espera entre elegir un tipo en el combo y pedir la confirmacion, para que
/// la lista del combo ya este cerrada.
const ESPERA_TRAS_EL_COMBO: std::time::Duration = std::time::Duration::from_millis(80);

/// Tipo que corresponde a una posicion del combo (0: sin clasificar).
fn tipo_del_combo(posicion: i32) -> Option<TipoDeElemento> {
    usize::try_from(posicion - 1).ok().and_then(|posicion| TipoDeElemento::TODOS.get(posicion).copied())
}

/// Posicion del combo de un tipo.
fn posicion_en_el_combo(tipo: Option<TipoDeElemento>) -> i32 {
    tipo.map_or(0, |tipo| tipo.posicion() as i32 + 1)
}

/// Si el tipo suena sin mezcla y el tema tiene efecto fijado o ventana de
/// overlay, avisa que se van a quitar y pregunta si seguir: Some(true) si
/// acepta, Some(false) si cancela. Sin nada que perder no pregunta (None).
fn confirmar_tipo_sin_mezcla(editor: &EditorDePista, tipo: Option<TipoDeElemento>) -> Option<bool> {
    let tipo = tipo.filter(|tipo| tipo.sin_mezcla())?;
    let efecto = usize::try_from(editor.get_efecto_fijado())
        .ok()
        .filter(|posicion| *posicion > 0)
        .and_then(|posicion| editor.get_efectos_fijables().row_data(posicion));
    let tiene_overlay = editor.get_tiene_overlay();
    if efecto.is_none() && !tiene_overlay {
        return None;
    }
    let mut consecuencias = Vec::new();
    if let Some(efecto) = efecto {
        consecuencias.push(tr!("- Se quitara el efecto fijado: {efecto}.", efecto = efecto));
    }
    let ventana = if tiene_overlay {
        tr!(" Se quita la ventana actual ({} a {}).", formatear_tiempo(f64::from(editor.get_overlay_inicio())), formatear_tiempo(f64::from(editor.get_overlay_fin())))
    } else {
        String::new()
    };
    consecuencias.push(tr!("- No estara disponible la ventana de overlay.{ventana}", ventana = ventana));
    consecuencias.push(tr!("- Tambien se quitan la intro y la outro: el tema suena sin mezcla.").into());
    let descripcion = tr!("Un elemento de tipo {} no es un Tema: suena sin mezcla.\n\n{}\n\n¿Desea cambiar el tipo?", tipo.nombre_visible(), consecuencias.join("\n"));
    let respuesta = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(&tr!("Tipo sin mezcla"))
        .set_description(descripcion)
        .set_buttons(rfd::MessageButtons::OkCancel)
        .set_parent(&editor.window().window_handle())
        .show();
    Some(respuesta == rfd::MessageDialogResult::Ok)
}

/// Area de una pantalla, en pixeles fisicos del escritorio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AreaDePantalla {
    pub x: i32,
    pub y: i32,
    pub ancho: u32,
    pub alto: u32,
}

// Parte de la barra de titulo que tiene que verse en una pantalla para poder
// agarrar la ventana y moverla.
const ANCHO_VISIBLE_DEL_TITULO: i64 = 120;
const ALTO_DE_LA_BARRA_DE_TITULO: i64 = 32;
const ALTO_VISIBLE_DEL_TITULO: i64 = 16;

/// Si la barra de titulo de una ventana con esa geometria queda, al menos en
/// parte, dentro de alguna pantalla: lo justo para verla y moverla.
pub(super) fn titulo_visible(geometria: GeometriaDeVentana, pantallas: &[AreaDePantalla]) -> bool {
    let (izquierda, arriba) = (i64::from(geometria.x), i64::from(geometria.y));
    let derecha = izquierda + i64::from(geometria.ancho);
    let abajo = arriba + ALTO_DE_LA_BARRA_DE_TITULO;
    pantallas.iter().any(|pantalla| {
        let (px, py) = (i64::from(pantalla.x), i64::from(pantalla.y));
        let ancho = derecha.min(px + i64::from(pantalla.ancho)) - izquierda.max(px);
        let alto = abajo.min(py + i64::from(pantalla.alto)) - arriba.max(py);
        ancho >= ANCHO_VISIBLE_DEL_TITULO.min(i64::from(geometria.ancho)) && alto >= ALTO_VISIBLE_DEL_TITULO
    })
}

/// Opciones del combo de efecto fijado: "(ninguno)", los de la botonera con
/// su nombre y los demas archivos por su nombre.
fn modelo_de_fijables(fijables: &[PathBuf], nombres_de_la_botonera: &[slint::SharedString]) -> ModelRc<slint::SharedString> {
    let mut opciones: Vec<slint::SharedString> = vec!["(ninguno)".into()];
    opciones.extend(fijables.iter().enumerate().map(|(posicion, archivo)| {
        nombres_de_la_botonera.get(posicion).cloned().unwrap_or_else(|| format!("Archivo: {}", nombre_de_archivo(archivo)).into())
    }));
    ModelRc::new(VecModel::from(opciones))
}

/// Texto de la sugerencia de overlay para el editor.
fn texto_de_sugerencia(sugerencia: Option<SugerenciaDeOverlay>) -> String {
    match sugerencia {
        Some(sugerencia) => tr!("Sugerida: {} a {} sobre la {} (confianza {:.0} %).", formatear_tiempo(sugerencia.ventana.inicio()), formatear_tiempo(sugerencia.ventana.fin()), sugerencia.zona.nombre(), sugerencia.confianza * 100.0),
        None => tr!("Sin sugerencia: no se encontro un tramo instrumental claro de al menos 4 s.").into(),
    }
}

/// BPM con un decimal solo si hace falta: "124" o "123.5".
fn formatear_bpm(bpm: f64) -> String {
    let texto = format!("{bpm:.1}");
    texto.strip_suffix(".0").map_or(texto.clone(), str::to_string)
}

/// Lee el BPM escrito en el editor: vacio es None (el del analisis).
fn leer_bpm(texto: &str) -> Result<Option<f64>, String> {
    let texto = texto.trim().replace(',', ".");
    if texto.is_empty() {
        return Ok(None);
    }
    match texto.parse::<f64>() {
        Ok(bpm) if (BPM_MINIMO..=BPM_MAXIMO).contains(&bpm) => Ok(Some(bpm)),
        _ => Err(tr!("El BPM debe ser un numero entre {BPM_MINIMO} y {BPM_MAXIMO}, o quedar vacio.", BPM_MINIMO = BPM_MINIMO, BPM_MAXIMO = BPM_MAXIMO)),
    }
}

/// Redondea a centesimas lo que llega de Slint (f32), para no guardar
/// valores como 80.30000305.
fn centesimas(valor: f32) -> f64 {
    (f64::from(valor) * 100.0).round() / 100.0
}

impl Controlador {
    /// Abre el editor con una pista de la lista (posicion desde 0).
    pub fn abrir_editor(&mut self, indice: usize) -> Result<String, String> {
        let pista = self
            .lista
            .pista_en(indice)
            .cloned()
            .ok_or_else(|| tr!("Posicion {} fuera de la lista (1 a {}).", indice + 1, self.lista.tamano()))?;
        self.validar_que_no_sea_el_anuncio_hth(&pista)?;
        self.abrir_editor_con(pista)
    }

    /// Abre el editor con el archivo de un boton de la hoja de efectos a la
    /// vista (desde 0), para fijar sus datos (por ejemplo el tipo Efecto) y
    /// el tramo que suena (inicio y fin).
    pub fn abrir_editor_de_efecto(&mut self, boton: usize) -> Result<String, String> {
        let archivo = self
            .repositorio_de_botonera
            .archivo_del_boton(boton)
            .map(Path::to_path_buf)
            .ok_or_else(|| tr!("El boton {boton} no tiene un archivo asignado.", boton = boton))?;
        if !archivo.is_file() {
            return Err(tr!("No existe el archivo del boton {boton}: {}", archivo.display(), boton = boton));
        }
        self.abrir_editor_con(Pista::new(archivo))
    }

    /// Abre el editor con una pista, este o no en la lista. Fuera de la
    /// lista se muestran los puntos guardados en su ficha.
    fn abrir_editor_con(&mut self, pista: Pista) -> Result<String, String> {
        self.detener_preescucha();
        let puntos = if self.lista.indice_de(&pista).is_some() {
            self.lista.puntos_de(&pista)
        } else {
            self.repositorio_de_canciones.puntos(pista.archivo()).unwrap_or_default()
        };
        let ficha = self.ficha_de(&pista);
        let duracion = self.duracion_de(&pista).unwrap_or(0.0);
        // Para fijar o probar sobre el tema solo se ofrecen los efectos de la
        // botonera de tipo Efecto o Seco, en el mismo orden en los dos
        // combos (el fijado i es el de prueba i - 1).
        let canciones = &self.repositorio_de_canciones;
        let (efectos, nombres_de_efectos): (Vec<PathBuf>, Vec<slint::SharedString>) = self
            .repositorio_de_botonera
            .efectos()
            .filter(|(_, _, archivo)| {
                canciones.tipo_de(archivo).efecto_o_seco()
            })
            .map(|(hoja, boton, archivo)| {
                (archivo.to_path_buf(), tr!("Hoja {} · {boton}: {}", hoja + 1, nombre_de_archivo(archivo), boton = boton).into())
            })
            .unzip();

        let editor = &self.editor;
        editor.set_pista(etiqueta(&self.lista, &pista).into());
        editor.set_ruta(pista.archivo().display().to_string().into());
        editor.set_duracion(duracion as f32);
        editor.set_aviso(tr!("Calculando la forma de onda...").into());
        editor.set_onda(Image::default());
        editor.set_vista_desde(0.0);
        editor.set_vista_hasta(duracion.max(1.0) as f32);
        editor.set_cursor(0.0);
        editor.set_inicio(puntos.recorte_inicial() as f32);
        editor.set_fin((duracion - puntos.recorte_final()).max(0.0) as f32);
        editor.set_intro(puntos.intro() as f32);
        editor.set_outro(puntos.outro() as f32);
        editor.set_puntos_manuales(puntos.manuales());
        editor.set_overlay_manual(puntos.overlay_manual());
        editor.set_hay_automaticos(false);
        editor.set_sugerencia_de_overlay(tr!("Buscando un tramo instrumental para el overlay...").into());
        editor.set_hay_sugerencia_de_overlay(false);
        editor.set_tiene_overlay(puntos.overlay().is_some());
        let ventana = puntos.overlay();
        editor.set_overlay_inicio(ventana.map_or(0.0, |ventana| ventana.inicio()) as f32);
        editor.set_overlay_fin(ventana.map_or(0.0, |ventana| ventana.fin()) as f32);
        editor.set_titulo(ficha.titulo.clone().unwrap_or_default().into());
        editor.set_artista(ficha.artista.clone().unwrap_or_default().into());
        editor.set_genero(ficha.genero.clone().unwrap_or_default().into());
        editor.set_tipo(posicion_en_el_combo(ficha.tipo));
        editor.set_tipo_anterior(posicion_en_el_combo(ficha.tipo));
        editor.set_sin_overlay(ficha.tipo.sin_mezcla());
        editor.set_album(ficha.album.clone().unwrap_or_default().into());
        editor.set_nota(ficha.nota.clone().unwrap_or_default().into());
        editor.set_anio(ficha.anio.map(|anio| anio.to_string()).unwrap_or_default().into());
        let mut fijables = efectos.clone();
        if let Some(fijado) = ficha.efecto_fijado.as_ref().filter(|fijado| !fijables.contains(fijado)) {
            fijables.push(fijado.clone());
        }
        editor.set_efectos_fijables(modelo_de_fijables(&fijables, &nombres_de_efectos));
        let posicion_fijada = ficha
            .efecto_fijado
            .as_ref()
            .and_then(|fijado| fijables.iter().position(|archivo| archivo == fijado))
            .map_or(0, |posicion| posicion + 1);
        editor.set_efecto_fijado(posicion_fijada as i32);
        editor.invoke_olvidar_previos();
        let generos: Vec<slint::SharedString> = generos_del_catalogo(&self.repositorio_de_canciones).into_iter().map(Into::into).collect();
        editor.set_generos_sugeridos(ModelRc::new(VecModel::from(generos)));
        editor.set_ranking(i32::from(ficha.ranking));
        let bpm = ficha.bpm.or_else(|| self.datos_de_mezcla(&pista).and_then(|datos| datos.bpm()));
        editor.set_bpm(bpm.map(formatear_bpm).unwrap_or_default().into());
        editor.set_efectos(ModelRc::new(VecModel::from(nombres_de_efectos)));
        editor.set_efecto_de_prueba(0);
        editor.set_posicion_de_preescucha(-1.0);
        editor.set_plan_del_efecto("".into());
        editor.set_volumen_de_preescucha(self.repositorio_de_preferencias.volumen_de_preescucha() as f32);

        self.generacion_de_onda += 1;
        let generacion = self.generacion_de_onda;
        self.edicion = Some(Edicion {
            pista: pista.clone(),
            onda: None,
            generacion,
            efectos,
            fijables,
            dibujada: None,
            recorte_final: puntos.recorte_final(),
            automaticos: None,
            sugerencia: None,
            bpm_automatico: None,
        });
        self.calcular_onda(pista.archivo().to_path_buf(), generacion);
        let estaba_abierto = self.editor.window().is_visible();
        if !estaba_abierto {
            // Cada vez que se abre se vuelven a buscar las salidas: pudieron
            // conectarse o desconectarse placas mientras estaba cerrado.
            self.actualizar_salidas_de_preescucha();
        }
        self.ciclo_del_editor();
        if !estaba_abierto {
            self.ubicar_editor();
        }
        self.editor.show().map_err(|error| tr!("No se pudo abrir el editor: {error}", error = error))?;
        if !estaba_abierto {
            // Algunos sistemas solo respetan la posicion con la ventana ya visible.
            self.ubicar_editor();
        }
        Ok(tr!("Editor abierto con {}.", etiqueta(&self.lista, &pista)))
    }

    /// Pista abierta en el editor, si lo hay.
    pub fn pista_en_edicion(&self) -> Option<&Pista> {
        self.edicion.as_ref().map(|edicion| &edicion.pista)
    }

    fn calcular_onda(&self, archivo: PathBuf, generacion: u64) {
        let cache = carpeta_del_usuario().join("ondas");
        let lanzado = std::thread::Builder::new().name("forma-de-onda".into()).spawn(move || {
            let inicio = std::time::Instant::now();
            let resultado = FormaDeOnda::cargar_o_calcular(&archivo, &cache);
            log::info!("Forma de onda de {} en {:?}", archivo.display(), inicio.elapsed());
            programar_en_interfaz(move |controlador| controlador.onda_calculada(generacion, resultado));
        });
        if let Err(error) = lanzado {
            self.editor.set_aviso(tr!("No se pudo calcular la forma de onda: {error}", error = error).into());
        }
    }

    fn onda_calculada(&mut self, generacion: u64, resultado: Result<FormaDeOnda, String>) {
        let Some(edicion) = self.edicion.as_mut().filter(|edicion| edicion.generacion == generacion) else {
            return;
        };
        match resultado {
            Ok(onda) => {
                if self.editor.get_duracion() <= 0.0 {
                    let duracion = onda.duracion();
                    self.editor.set_duracion(duracion as f32);
                    self.editor.set_vista_hasta(duracion.max(1.0) as f32);
                    self.editor.set_fin((duracion - edicion.recorte_final).max(0.0) as f32);
                }
                let datos = DatosDeMezcla::de_la_onda(&onda);
                edicion.automaticos = Some(datos.puntos_automaticos());
                edicion.sugerencia = datos.overlay_sugerido;
                edicion.bpm_automatico = datos.bpm();
                if self.editor.get_bpm().trim().is_empty() {
                    self.editor.set_bpm(datos.bpm().map(formatear_bpm).unwrap_or_default().into());
                }
                edicion.onda = Some(Rc::new(onda));
                self.editor.set_hay_automaticos(true);
                self.editor.set_hay_sugerencia_de_overlay(datos.overlay_sugerido.is_some());
                self.editor.set_sugerencia_de_overlay(texto_de_sugerencia(datos.overlay_sugerido).into());
                self.editor.set_aviso(
                    "Arrastre las marcas sobre la onda o escriba los tiempos (segundos o m:ss). \
                     Clic: mover el cursor. Rueda: zoom."
                        .into(),
                );
                self.redibujar_onda();
            }
            Err(error) => self.editor.set_aviso(tr!("No se pudo leer la forma de onda: {error}", error = error).into()),
        }
    }

    /// Dibuja la onda al tamano actual de su zona y con la vista actual, salvo
    /// que ya este dibujada asi.
    fn redibujar_onda(&mut self) {
        let editor = &self.editor;
        let Some(edicion) = &mut self.edicion else {
            return;
        };
        let Some(onda) = &edicion.onda else {
            return;
        };
        let escala = editor.window().scale_factor();
        let ancho = (editor.get_ancho_de_onda() * escala).round() as u32;
        let alto = (editor.get_alto_de_onda() * escala).round() as u32;
        if ancho == 0 || alto == 0 {
            return;
        }
        let parametros = (ancho, alto, editor.get_vista_desde(), editor.get_vista_hasta(), editor.get_oscuro());
        if edicion.dibujada == Some(parametros) {
            return;
        }
        let color = if parametros.4 { COLOR_OSCURO } else { COLOR_CLARO };
        let pixeles = onda.dibujar(ancho, alto, f64::from(parametros.2), f64::from(parametros.3), color);
        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&pixeles, ancho, alto);
        editor.set_onda(Image::from_rgba8(buffer));
        edicion.dibujada = Some(parametros);
    }

    /// Acerca (factor < 1) o aleja (factor > 1) la vista dejando `centro` en
    /// el mismo lugar de la pantalla; factor 0 muestra la pista entera.
    fn zoom_de_onda(&mut self, factor: f64, centro: f64) {
        let duracion = f64::from(self.editor.get_duracion());
        if duracion <= 0.0 {
            return;
        }
        let (desde, hasta) = (f64::from(self.editor.get_vista_desde()), f64::from(self.editor.get_vista_hasta()));
        let (nuevo_desde, nuevo_hasta) = if factor <= 0.0 {
            (0.0, duracion)
        } else {
            let rango = ((hasta - desde) * factor).clamp(VISTA_MINIMA.min(duracion), duracion);
            let relativo = ((centro - desde) / (hasta - desde)).clamp(0.0, 1.0);
            let nuevo_desde = (centro - relativo * rango).clamp(0.0, duracion - rango);
            (nuevo_desde, nuevo_desde + rango)
        };
        self.editor.set_vista_desde(nuevo_desde as f32);
        self.editor.set_vista_hasta(nuevo_hasta as f32);
        self.redibujar_onda();
    }

    /// Vuelve inicio, fin, intro y outro a lo que propone el analisis.
    fn volver_al_automatico(&mut self) {
        self.editor.invoke_olvidar_previos();
        let Some(automaticos) = self.edicion.as_ref().and_then(|edicion| edicion.automaticos) else {
            return;
        };
        let editor = &self.editor;
        let duracion = f64::from(editor.get_duracion());
        editor.set_inicio(automaticos.recorte_inicial() as f32);
        editor.set_fin((duracion - automaticos.recorte_final()).max(0.0) as f32);
        editor.set_intro(automaticos.intro() as f32);
        editor.set_outro(automaticos.outro() as f32);
        editor.set_puntos_manuales(false);
        editor.set_aviso(
            tr!("Automatico: empieza en {}, termina en {}, intro de {} s y outro de {} s. Guarde para aplicarlo.", formatear_tiempo(automaticos.recorte_inicial()), formatear_tiempo(duracion - automaticos.recorte_final()), formatear_tiempo(automaticos.intro_audible()), formatear_tiempo(automaticos.outro_audible()))
            .into(),
        );
    }

    /// Pone la ventana de overlay que sugiere el analisis (o ninguna, si no
    /// hay sugerencia) y la deja como automatica.
    fn usar_overlay_sugerido(&mut self) {
        self.editor.invoke_olvidar_previos();
        let Some(edicion) = &self.edicion else {
            return;
        };
        let editor = &self.editor;
        match edicion.sugerencia {
            Some(sugerencia) => {
                editor.set_overlay_inicio(sugerencia.ventana.inicio() as f32);
                editor.set_overlay_fin(sugerencia.ventana.fin() as f32);
                editor.set_tiene_overlay(true);
                editor.set_aviso(tr!("Ventana sugerida. Pruebela con \"Ventana con efecto\" y guarde.").into());
            }
            None => {
                editor.set_tiene_overlay(false);
                editor.set_aviso(tr!("No hay sugerencia: sin ventana, los efectos suenan enseguida.").into());
            }
        }
        editor.set_overlay_manual(false);
    }

    /// Valor escrito en un campo de tiempo del editor.
    fn editar_campo(&mut self, marca: MarcaDeEdicion, texto: &str) {
        let duracion = f64::from(self.editor.get_duracion());
        let segundos = match parsear_tiempo(texto) {
            Ok(segundos) if segundos <= duracion => segundos,
            Ok(_) => {
                self.editor.set_aviso(tr!("El tiempo supera la duracion de la pista ({texto}).", texto = texto).into());
                return;
            }
            Err(error) => {
                self.editor.set_aviso(error.into());
                return;
            }
        };
        let editor = &self.editor;
        let tiene_overlay = editor.get_tiene_overlay();
        let (inicio, fin) = (f64::from(editor.get_overlay_inicio()), f64::from(editor.get_overlay_fin()));
        let (comienzo, final_) = (f64::from(editor.get_inicio()), f64::from(editor.get_fin()));
        match marca {
            MarcaDeEdicion::Cursor => editor.set_cursor(segundos as f32),
            MarcaDeEdicion::Intro | MarcaDeEdicion::Outro | MarcaDeEdicion::Inicio | MarcaDeEdicion::Fin => {
                match marca {
                    MarcaDeEdicion::Intro => editor.set_intro(segundos as f32),
                    MarcaDeEdicion::Outro => editor.set_outro(segundos as f32),
                    MarcaDeEdicion::Inicio if segundos < final_ => editor.set_inicio(segundos as f32),
                    MarcaDeEdicion::Fin if segundos > comienzo => editor.set_fin(segundos as f32),
                    _ => {
                        editor.set_aviso(tr!("El fin del tema debe ser posterior a su inicio.").into());
                        return;
                    }
                }
                editor.set_puntos_manuales(true);
            }
            MarcaDeEdicion::OverlayInicio if !tiene_overlay || segundos < fin => {
                let fin = if tiene_overlay { fin } else { (segundos + VENTANA_INICIAL).min(duracion) };
                editor.set_overlay_inicio(segundos as f32);
                editor.set_overlay_fin(fin as f32);
                editor.set_tiene_overlay(fin > segundos);
                editor.set_overlay_manual(true);
            }
            MarcaDeEdicion::OverlayFin if !tiene_overlay || segundos > inicio => {
                let inicio = if tiene_overlay { inicio } else { (segundos - VENTANA_INICIAL).max(0.0) };
                editor.set_overlay_inicio(inicio as f32);
                editor.set_overlay_fin(segundos as f32);
                editor.set_tiene_overlay(segundos > inicio);
                editor.set_overlay_manual(true);
            }
            MarcaDeEdicion::OverlayInicio | MarcaDeEdicion::OverlayFin => {
                editor.set_aviso(tr!("El overlay debe terminar despues de empezar.").into());
                return;
            }
        }
        editor.set_aviso("".into());
    }

    /// Guarda lo editado en la lista y en la ficha de la cancion.
    fn guardar_edicion(&mut self) {
        let Some(pista) = self.edicion.as_ref().map(|edicion| edicion.pista.clone()) else {
            return;
        };
        let editor = &self.editor;
        let ventana = if editor.get_tiene_overlay() {
            match VentanaDeOverlay::nueva(centesimas(editor.get_overlay_inicio()), centesimas(editor.get_overlay_fin())) {
                Ok(ventana) => Some(ventana),
                Err(error) => {
                    editor.set_aviso(error.into());
                    return;
                }
            }
        } else {
            None
        };
        let duracion = f64::from(editor.get_duracion());
        let recorte_final = ((duracion - f64::from(editor.get_fin())) * 100.0).round().max(0.0) / 100.0;
        let puntos = PuntosDeMezcla::nuevos(centesimas(editor.get_intro()), centesimas(editor.get_outro()), ventana)
            .and_then(|puntos| puntos.con_recortes(centesimas(editor.get_inicio()), recorte_final))
            .map(|puntos| puntos.como_manuales(editor.get_puntos_manuales()).con_overlay_manual(editor.get_overlay_manual()));
        let puntos = match puntos {
            Ok(puntos) => puntos,
            Err(error) => {
                editor.set_aviso(error.into());
                return;
            }
        };
        let bpm = match leer_bpm(&editor.get_bpm()) {
            Ok(bpm) => bpm,
            Err(error) => {
                editor.set_aviso(error.into());
                return;
            }
        };
        // Si coincide con el del analisis no se fija a mano: sigue al analisis.
        let automatico = self.edicion.as_ref().and_then(|edicion| edicion.bpm_automatico).or_else(|| {
            self.datos_de_mezcla(&pista).and_then(|datos| datos.bpm())
        });
        let bpm = bpm.filter(|bpm| automatico.is_none_or(|automatico| formatear_bpm(automatico) != formatear_bpm(*bpm)));
        let editor = &self.editor;
        let anio = match editor.get_anio().trim() {
            "" => None,
            texto => match texto.parse::<u16>().ok().filter(|anio| anio_valido(*anio)) {
                Some(anio) => Some(anio),
                None => {
                    editor.set_aviso(tr!("El anio debe ser un numero entre 1000 y 2999 (o quedar vacio).").into());
                    return;
                }
            },
        };
        let tipo = tipo_del_combo(editor.get_tipo());
        if let Err(error) = self.validar_tipo(&pista, tipo) {
            editor.set_aviso(error.into());
            return;
        }
        let efecto_fijado = usize::try_from(editor.get_efecto_fijado() - 1)
            .ok()
            .and_then(|posicion| self.edicion.as_ref()?.fijables.get(posicion).cloned());
        let cambios = CambiosDeDatos {
            tipo: Some(tipo),
            titulo: Some(editor.get_titulo().to_string()),
            artista: Some(editor.get_artista().to_string()),
            genero: Some(editor.get_genero().to_string()),
            album: Some(editor.get_album().to_string()),
            nota: Some(editor.get_nota().to_string()),
            anio: Some(anio),
            efecto_fijado: Some(efecto_fijado),
            ranking: Some(editor.get_ranking().clamp(0, i32::from(RANKING_MAXIMO)) as u8),
            bpm: Some(bpm),
        };
        let guardado = if self.lista.indice_de(&pista).is_some() {
            self.definir_puntos_de_mezcla(&pista, puntos)
        } else {
            let duracion = Some(f64::from(self.editor.get_duracion())).filter(|duracion| *duracion > 0.0);
            puntos.validar_en(duracion).map(|_| self.repositorio_de_canciones.guardar_puntos(pista.archivo(), puntos))
        };
        if let Err(error) = guardado {
            self.editor.set_aviso(error.into());
            return;
        }
        self.cambiar_datos(&pista, cambios);
        // Si es un efecto de la botonera, su boton muestra la duracion del
        // tramo y si se puede insertar en la lista.
        self.duraciones_de_efectos.borrow_mut().remove(pista.archivo());
        self.refrescar_botones();
        self.cerrar_editor();
        let mut estado = format!("Guardado: {}", etiqueta(&self.lista, &pista));
        if let Some(aviso) = self.aviso_de_ventana_de_la_base(&pista) {
            estado.push_str(&tr!(". Base del anuncio HTH: {aviso}", aviso = aviso));
        }
        self.mostrar_estado(&estado);
    }

    /// Al elegir un tipo que no es Tema (comercial, anuncio, publicidad,
    /// efecto o seco) se quitan la intro, la outro y la ventana de overlay:
    /// suenan sin mezcla y solo cuentan su inicio y su fin. El usuario puede
    /// volver a fijarlos a mano.
    fn al_elegir_tipo(&mut self, tipo: Option<TipoDeElemento>) {
        self.editor.set_sin_overlay(tipo.sin_mezcla());
        let Some(tipo) = tipo.filter(|tipo| tipo.sin_mezcla()) else {
            self.editor.set_aviso("".into());
            return;
        };
        self.editor.invoke_olvidar_previos();
        let editor = &self.editor;
        let recorte_final = (editor.get_duracion() - editor.get_fin()).max(0.0);
        editor.set_intro(editor.get_inicio());
        editor.set_outro(recorte_final);
        editor.set_tiene_overlay(false);
        editor.set_overlay_manual(false);
        // Inicio y fin fijados a mano siguen siendo manuales (de ellos
        // depende, por ejemplo, el tramo que suena de un efecto).
        editor.set_efecto_fijado(0);
        editor.set_aviso(
            tr!("{}: sin intro, outro, ventana de overlay ni efecto fijado; solo cuentan el inicio y el fin. Guarde para aplicarlo.", tipo.nombre_visible())
            .into(),
        );
    }

    /// Pone en el campo BPM el tempo que estima el analisis de todo el tema.
    fn calcular_bpm(&mut self) {
        let Some(edicion) = &self.edicion else {
            return;
        };
        let aviso = if edicion.onda.is_none() {
            tr!("Todavia se esta calculando la forma de onda; pruebe en un momento.")
        } else if let Some(bpm) = edicion.bpm_automatico {
            self.editor.set_bpm(formatear_bpm(bpm).into());
            tr!("BPM calculado: {} (estimado sobre todo el tema). Guarde para aplicarlo.", formatear_bpm(bpm))
        } else {
            tr!("No se encontro un ritmo claro en el tema; puede escribir el BPM a mano.")
        };
        self.editor.set_aviso(aviso.into());
    }

    /// Duplica o divide a la mitad el BPM escrito, sin salir del rango.
    fn multiplicar_bpm(&mut self, factor: f64) {
        let aviso = match leer_bpm(&self.editor.get_bpm()) {
            Ok(Some(bpm)) if (BPM_MINIMO..=BPM_MAXIMO).contains(&(bpm * factor)) => {
                self.editor.set_bpm(formatear_bpm(bpm * factor).into());
                tr!("BPM {} → {}. Guarde para aplicarlo.", formatear_bpm(bpm), formatear_bpm(bpm * factor))
            }
            Ok(Some(bpm)) => tr!("{} BPM queda fuera del rango ({BPM_MINIMO} a {BPM_MAXIMO}).", formatear_bpm(bpm * factor), BPM_MINIMO = BPM_MINIMO, BPM_MAXIMO = BPM_MAXIMO),
            Ok(None) => tr!("Escriba o calcule un BPM primero."),
            Err(error) => error,
        };
        self.editor.set_aviso(aviso.into());
    }

    pub fn cerrar_editor(&mut self) {
        self.al_cerrarse_el_editor();
        let _ = self.editor.hide();
    }

    fn al_cerrarse_el_editor(&mut self) {
        self.detener_preescucha();
        // No se deja ocupada la otra salida (ni despierto un auricular
        // inalambrico) con el editor cerrado.
        self.preescucha.cerrar();
        self.edicion = None;
        self.recordar_ubicacion_del_editor();
    }

    /// Pone el editor donde quedo la ultima vez, si esa posicion se ve en
    /// alguna pantalla conectada (pudo quedar en un monitor que ya no esta).
    /// Si no se ve, la primera vez o con "Editor siempre centrado", lo centra
    /// sobre la ventana principal.
    fn ubicar_editor(&self) {
        if !self.repositorio_de_preferencias.editor_centrado()
            && let Some(geometria) = self.repositorio_de_preferencias.ventana_del_editor()
        {
            let pantallas = self.pantallas();
            if titulo_visible(geometria, &pantallas) {
                let ventana_del_editor = self.editor.window();
                ventana_del_editor.set_size(slint::PhysicalSize::new(geometria.ancho, geometria.alto));
                ventana_del_editor.set_position(slint::PhysicalPosition::new(geometria.x, geometria.y));
                return;
            }
            log::info!("El editor estaba fuera de las pantallas conectadas ({geometria:?}); se centra en la ventana principal");
        }
        let inicial = self.ventana.upgrade().map(|principal| {
            slint::LogicalSize::new(ANCHO_INICIAL_DEL_EDITOR, ALTO_INICIAL_DEL_EDITOR).to_physical(principal.window().scale_factor())
        });
        if let Some(tamano) = inicial {
            self.centrar_editor(tamano);
        }
    }

    /// Centra el editor sobre la ventana principal con ese tamano (sin pasar
    /// el de la ventana principal).
    fn centrar_editor(&self, tamano: slint::PhysicalSize) {
        let Some(principal) = self.ventana.upgrade() else {
            return;
        };
        let principal = principal.window();
        let (origen, area) = (principal.position(), principal.size());
        let tamano = slint::PhysicalSize::new(tamano.width.min(area.width.max(1)), tamano.height.min(area.height.max(1)));
        let ventana_del_editor = self.editor.window();
        ventana_del_editor.set_size(tamano);
        let centrar = |desde: i32, disponible: u32, ocupado: u32| desde + ((disponible as i32 - ocupado as i32) / 2).max(0);
        ventana_del_editor.set_position(slint::PhysicalPosition::new(
            centrar(origen.x, area.width, tamano.width),
            centrar(origen.y, area.height, tamano.height),
        ));
    }

    /// Pantallas conectadas (area en pixeles fisicos). Si no se pueden
    /// consultar, vale el area de la ventana principal.
    pub(super) fn pantallas(&self) -> Vec<AreaDePantalla> {
        let Some(principal) = self.ventana.upgrade() else {
            return Vec::new();
        };
        let monitores = principal.window().with_winit_window(|ventana| {
            ventana
                .available_monitors()
                .map(|monitor| {
                    let (posicion, tamano) = (monitor.position(), monitor.size());
                    AreaDePantalla { x: posicion.x, y: posicion.y, ancho: tamano.width, alto: tamano.height }
                })
                .collect::<Vec<_>>()
        });
        match monitores {
            Some(monitores) if !monitores.is_empty() => monitores,
            _ => {
                let (posicion, tamano) = (principal.window().position(), principal.window().size());
                vec![AreaDePantalla { x: posicion.x, y: posicion.y, ancho: tamano.width, alto: tamano.height }]
            }
        }
    }

    /// Menu Ventana: trae el editor (abierto) sobre la ventana principal.
    pub(super) fn traer_editor(&mut self) {
        let ventana_del_editor = self.editor.window();
        if !ventana_del_editor.is_visible() {
            self.mostrar_estado(&tr!("El editor no esta abierto: al abrirlo aparece sobre la ventana principal."));
            return;
        }
        let tamano = ventana_del_editor.size();
        self.centrar_editor(tamano);
        self.recordar_ubicacion_del_editor();
        self.mostrar_estado(&tr!("El editor se centro sobre la ventana principal."));
    }

    /// Menu Ventana: abrir el editor siempre centrado o donde quedo.
    pub(super) fn alternar_editor_centrado(&mut self) {
        let activo = !self.repositorio_de_preferencias.editor_centrado();
        self.repositorio_de_preferencias.guardar_editor_centrado(activo);
        self.con_ventana(|ventana| ventana.set_editor_centrado(activo));
        self.mostrar_estado(&if activo {
            tr!("El editor se abrira siempre centrado sobre la ventana principal.")
        } else {
            tr!("El editor se abrira donde quedo la ultima vez (si esa pantalla esta conectada).")
        });
    }

    /// Muestra u oculta los textos de los botones de la ventana principal.
    pub(super) fn alternar_ocultar_textos(&mut self) {
        let activo = !self.repositorio_de_preferencias.ocultar_textos();
        self.repositorio_de_preferencias.guardar_ocultar_textos(activo);
        self.con_ventana(|ventana| ventana.set_ocultar_textos(activo));
        self.mostrar_estado(&if activo {
            tr!("Los botones muestran solo el icono; el nombre aparece al pasar el puntero.")
        } else {
            tr!("Los botones muestran el icono y el texto (en ventanas angostas quedan solo los iconos).")
        });
    }

    /// Guarda la posicion y el tamano del editor para la proxima vez.
    fn recordar_ubicacion_del_editor(&mut self) {
        let ventana_del_editor = self.editor.window();
        if !ventana_del_editor.is_visible() || ventana_del_editor.is_minimized() {
            return;
        }
        let (posicion, tamano) = (ventana_del_editor.position(), ventana_del_editor.size());
        if tamano.width == 0 || tamano.height == 0 {
            return;
        }
        self.repositorio_de_preferencias.guardar_ventana_del_editor(GeometriaDeVentana {
            x: posicion.x,
            y: posicion.y,
            ancho: tamano.width,
            alto: tamano.height,
        });
    }

    // Preescucha

    /// Por otra salida se puede preescuchar siempre; compartiendo la de la
    /// musica, solo con la musica detenida o en pausa.
    fn preescucha_permitida(&self) -> bool {
        self.preescucha.es_aparte() || !self.gestor.esta_reproduciendo()
    }

    /// Busca las salidas de audio, abre la elegida si esta disponible y
    /// actualiza el combo.
    fn actualizar_salidas_de_preescucha(&mut self) {
        let elegida = self.repositorio_de_preferencias.salida_de_preescucha().cloned();
        let aviso = self.preescucha.actualizar(elegida.as_ref());
        let textos: Vec<slint::SharedString> = self.preescucha.textos().into_iter().map(Into::into).collect();
        self.editor.set_salidas_de_preescucha(ModelRc::new(VecModel::from(textos)));
        self.editor.set_salida_de_preescucha(self.preescucha.elegida_en_el_combo() as i32);
        self.editor.set_preescucha_aparte(self.preescucha.es_aparte());
        if let Some(aviso) = aviso {
            self.editor.set_aviso(aviso.into());
        }
    }

    fn buscar_salidas_de_preescucha(&mut self) {
        self.detener_preescucha();
        self.actualizar_salidas_de_preescucha();
        if self.preescucha.es_aparte() || self.repositorio_de_preferencias.salida_de_preescucha().is_none() {
            let cantidad = self.preescucha.textos().len() - 1;
            self.editor.set_aviso(tr!("Salidas de audio actualizadas: {cantidad} distinta(s) de la de la musica.", cantidad = cantidad).into());
        }
    }

    /// El usuario eligio una salida en el combo (0: la de la musica).
    fn elegir_salida_de_preescucha(&mut self, posicion: usize) {
        self.detener_preescucha();
        let salida = self.preescucha.salida_en(posicion);
        self.repositorio_de_preferencias.guardar_salida_de_preescucha(salida.clone());
        self.actualizar_salidas_de_preescucha();
        if let Some(salida) = salida.as_ref().filter(|_| self.preescucha.es_aparte()) {
            self.editor.set_aviso(tr!("La preescucha suena por {}, aun con la musica sonando.", salida.nombre).into());
        } else if salida.is_none() {
            self.editor.set_aviso(tr!("La preescucha comparte la salida de la musica: se usa con la musica detenida.").into());
        }
    }

    /// Mientras se mueve el deslizante cambia el volumen de lo que suena; al
    /// soltarlo (`guardar`) se recuerda.
    fn ajustar_volumen_de_preescucha(&mut self, volumen: f64, guardar: bool) {
        let volumen = volumen.clamp(0.0, 1.0);
        if let Some(preescucha) = &self.preescucha_en_curso {
            self.preescucha.motor_ref().definir_volumen(volumen * preescucha.nivelacion);
        }
        if guardar {
            self.repositorio_de_preferencias.guardar_volumen_de_preescucha(volumen);
        }
    }

    /// Volumen de la preescucha que se esta ajustando en el editor.
    fn volumen_de_preescucha(&self) -> f64 {
        f64::from(self.editor.get_volumen_de_preescucha()).clamp(0.0, 1.0)
    }

    /// Gestor que hace sonar el efecto de prueba: el de la salida aparte (con
    /// los mismos ajustes que los efectos de la lista) o el de la lista.
    fn efectos_de_prueba(&mut self) -> &mut GestorDeOverlay {
        let ajustes = self.overlay.ajustes();
        match self.preescucha.aparte_mut() {
            Some(aparte) => {
                aparte.efectos.definir_ajustes(ajustes);
                &mut aparte.efectos
            }
            None => &mut self.overlay,
        }
    }

    fn preescuchar(&mut self, desde: f64, hasta: f64, efecto_en: f64) {
        let Some(edicion) = &self.edicion else {
            return;
        };
        if !self.preescucha_permitida() {
            self.editor.set_aviso(tr!("Detene o pausa la musica para preescuchar.").into());
            return;
        }
        // Si el tema tiene efecto fijado se prueba ese, que es el que va a
        // sonar en la lista; si no, el elegido en el combo de prueba.
        let fijado = usize::try_from(self.editor.get_efecto_fijado() - 1).ok().and_then(|posicion| edicion.fijables.get(posicion));
        let de_prueba = || edicion.efectos.get(self.editor.get_efecto_de_prueba().max(0) as usize);
        let efecto = (efecto_en >= 0.0).then(|| fijado.or_else(de_prueba).cloned()).flatten().map(|archivo| (efecto_en, archivo));
        let pista = edicion.pista.clone();
        // Suena con la nivelacion de la lista y el volumen propio de la
        // preescucha.
        let nivelacion = self
            .datos_de_mezcla(&pista)
            .filter(|_| self.nivelar_volumen())
            .map_or(1.0, |datos| datos.ganancia_de_nivelacion(self.objetivo_de_sonoridad()));
        let volumen = self.volumen_de_preescucha() * nivelacion;
        let duracion = self.duracion_de(&pista);
        let motor = self.preescucha.motor();
        let arranque = motor.cargar(&pista, duracion).and_then(|_| {
            motor.buscar(desde)?;
            motor.definir_volumen(volumen);
            motor.reproducir()
        });
        match arranque {
            Ok(()) => {
                self.preescucha_en_curso = Some(Preescucha { hasta, nivelacion, efecto, efecto_lanzado: false });
                self.editor.set_preescuchando(true);
            }
            Err(error) => {
                self.preescucha.motor().detener();
                self.editor.set_aviso(tr!("No se pudo preescuchar: {error}", error = error).into());
            }
        }
    }

    pub(super) fn detener_preescucha(&mut self) {
        self.preescucha.motor().detener();
        if self.preescucha_en_curso.take().is_some_and(|preescucha| preescucha.efecto_lanzado) {
            self.efectos_de_prueba().detener();
        }
        self.editor.set_preescuchando(false);
        self.editor.set_posicion_de_preescucha(-1.0);
    }

    /// Parte del ciclo periodico: redibuja la onda si cambio su zona o la
    /// vista, avanza la preescucha, hace sonar el efecto de prueba en su
    /// momento y la corta al llegar al final del tramo o si empieza la musica.
    pub(super) fn ciclo_del_editor(&mut self) {
        if self.edicion.is_none() {
            return;
        }
        self.redibujar_onda();
        if let Some(motivo) = self.preescucha.revisar() {
            self.detener_preescucha();
            self.preescucha.cerrar();
            self.actualizar_salidas_de_preescucha();
            self.editor.set_aviso(motivo.into());
        }
        if let Some(Err(error)) = self.preescucha.aparte_mut().map(|aparte| aparte.efectos.registrar_fin()) {
            self.editor.set_aviso(tr!("No se pudo hacer sonar el efecto: {error}", error = error).into());
        }
        let permitida = self.preescucha_permitida();
        self.editor.set_puede_preescuchar(permitida);
        let Some(preescucha) = &mut self.preescucha_en_curso else {
            return;
        };
        let motor = self.preescucha.motor();
        if let Some(motivo) = motor.tomar_error() {
            self.editor.set_aviso(tr!("No se pudo preescuchar: {motivo}", motivo = motivo).into());
        }
        let termino = motor.tomar_fin();
        let posicion = motor.posicion_actual();
        if termino || posicion >= preescucha.hasta || !permitida {
            self.detener_preescucha();
            return;
        }
        self.editor.set_posicion_de_preescucha(posicion as f32);
        // El efecto se programa en el frame exacto en que la preescucha llega
        // a su momento, con el mismo volumen y la misma atenuacion que en la
        // lista.
        if !preescucha.efecto.as_ref().is_some_and(|(momento, _)| posicion >= *momento - PROGRAMAR_EFECTO_CON) {
            return;
        }
        let (momento, archivo) = preescucha.efecto.take().expect("hay efecto");
        preescucha.efecto_lanzado = true;
        self.boton_del_efecto = None;
        let arranque = self.preescucha.motor_ref().frame_de_salida_en(momento);
        let Some(pista) = self.pista_en_edicion().cloned() else {
            return;
        };
        let recorte = recorte_de_efecto(&self.repositorio_de_canciones, &archivo);
        let segundos = recorte.duracion(self.efectos_de_prueba().duracion_del_efecto(&archivo)).unwrap_or(0.0);
        let musica = self.medidor_de_musica().medir(&pista, momento, segundos);
        let escala = self.volumen_de_preescucha();
        match self.efectos_de_prueba().sonar_ya(&archivo, arranque, musica, escala, recorte) {
            Ok(()) => {
                let plan = self.efectos_de_prueba().ultimo_plan().map(|plan| tr!("Ultima prueba: {}", plan.descripcion())).unwrap_or_default();
                self.editor.set_plan_del_efecto(plan.into());
            }
            Err(error) => self.editor.set_aviso(tr!("No se pudo hacer sonar el efecto: {error}", error = error).into()),
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const PRIMARIA: AreaDePantalla = AreaDePantalla { x: 0, y: 0, ancho: 1920, alto: 1080 };
    // Segundo monitor a la derecha del primero.
    const SECUNDARIA: AreaDePantalla = AreaDePantalla { x: 1920, y: 0, ancho: 1920, alto: 1080 };

    fn editor_en(x: i32, y: i32) -> GeometriaDeVentana {
        GeometriaDeVentana { x, y, ancho: 1180, alto: 620 }
    }

    #[test]
    fn el_editor_en_un_monitor_desconectado_no_se_ve() {
        let en_el_segundo = editor_en(2300, 200);
        assert!(titulo_visible(en_el_segundo, &[PRIMARIA, SECUNDARIA]));
        assert!(!titulo_visible(en_el_segundo, &[PRIMARIA]), "con un solo monitor quedaria afuera");
    }

    #[test]
    fn alcanza_con_que_se_vea_parte_de_la_barra_de_titulo() {
        assert!(titulo_visible(editor_en(300, 120), &[PRIMARIA]));
        // Corrido casi todo a la derecha: quedan 200 px de titulo a la vista.
        assert!(titulo_visible(editor_en(1720, 100), &[PRIMARIA]));
        // Solo 50 px a la vista: no alcanza para agarrarla.
        assert!(!titulo_visible(editor_en(1870, 100), &[PRIMARIA]));
        // La barra de titulo por encima de la pantalla.
        assert!(!titulo_visible(editor_en(300, -40), &[PRIMARIA]));
        // Debajo de todo.
        assert!(!titulo_visible(editor_en(300, 1075), &[PRIMARIA]));
        // Un monitor a la izquierda (coordenadas negativas).
        let izquierda = AreaDePantalla { x: -1920, y: 0, ancho: 1920, alto: 1080 };
        assert!(titulo_visible(editor_en(-1500, 100), &[izquierda, PRIMARIA]));
        assert!(!titulo_visible(editor_en(-1500, 100), &[PRIMARIA]));
        assert!(!titulo_visible(editor_en(300, 120), &[]), "sin pantallas no se ve");
    }
}

impl Controlador {
    /// Si el editor muestra ese archivo, pone en la solapa Datos los datos
    /// descriptivos que acaban de cambiar en el catalogo (lo demas que se
    /// este editando no se toca).
    pub(super) fn refrescar_datos_del_editor(&mut self, archivo: &std::path::Path) {
        let Some(edicion) = self.edicion.as_ref() else { return };
        if edicion.pista.archivo() != archivo {
            return;
        }
        let Some(ficha) = self.repositorio_de_canciones.buscar(archivo) else { return };
        let editor = &self.editor;
        editor.set_titulo(ficha.titulo.clone().unwrap_or_default().into());
        editor.set_artista(ficha.artista.clone().unwrap_or_default().into());
        editor.set_genero(ficha.genero.clone().unwrap_or_default().into());
        editor.set_album(ficha.album.clone().unwrap_or_default().into());
        editor.set_anio(ficha.anio.map(|anio| anio.to_string()).unwrap_or_default().into());
    }
}
