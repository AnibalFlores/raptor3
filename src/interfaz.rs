//! Controlador de la ventana principal: conecta el dominio (la lista), los
//! servicios de reproduccion y la persistencia con la interfaz Slint.
//!
//! Todo corre en el hilo de la interfaz. El estado vive en un controlador
//! compartido por los callbacks, el temporizador y el servidor MCP (que llega
//! a traves de `ejecutar_en_interfaz`). Los dialogos se abren siempre con el
//! controlador libre, porque sus bucles modales pueden despachar otros eventos.

use crate::idioma::Idioma;
use crate::{BarraDeNivel, IdiomaDeLaInterfaz};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

#[cfg(test)]
mod capturas;
mod actualizacion;
mod buscador;
mod caratula;
mod localidad;
mod editor;
mod hth;
mod mantenimiento;
mod marcadores;
mod modo;
mod ordenes;
mod anunciantes;
mod avisos;
mod pauta;
mod salida_de_preescucha;
mod tandas;
mod ubicacion;

#[cfg(test)]
mod pruebas {
    use super::*;

    fn momento(texto: &str) -> jiff::Zoned {
        texto.parse::<jiff::Timestamp>().unwrap().to_zoned(jiff::tz::TimeZone::UTC)
    }

    #[test]
    fn el_manual_del_idioma_elegido_va_primero() {
        assert_eq!(manuales_en_orden(Idioma::Ingles), ["Raptor_Manual_en.pdf", "Raptor_Manual_es.pdf"]);
        assert_eq!(manuales_en_orden(Idioma::Espanol), ["Raptor_Manual_es.pdf", "Raptor_Manual_en.pdf"]);
    }

    #[test]
    fn el_reloj_muestra_dia_fecha_y_hora() {
        let lunes = momento("2026-11-02T14:05:03Z");
        assert_eq!(fecha_y_hora(&lunes), ("Lunes 02/11/2026".to_string(), "14:05:03".to_string()));
    }

    #[test]
    fn los_comerciales_no_se_mezclan_ni_con_el_anterior_ni_con_el_siguiente() {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-interfaz-sin-mezcla-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&carpeta);
        let mut canciones = RepositorioDeCanciones::new(carpeta.join("mezcla.json"));
        let mut lista = ListaDeReproduccion::new();
        let pistas = lista.insertar(0, ["tema.mp3", "anuncio.mp3", "otro.mp3", "ultimo.mp3"].map(PathBuf::from));
        let puntos = PuntosDeMezcla::nuevos(6.0, 8.0, None).unwrap();
        for pista in &pistas {
            lista.definir_puntos(pista, puntos).unwrap();
        }
        canciones.cambiar_datos(pistas[1].archivo(), CambiosDeDatos { tipo: Some(Some(TipoDeElemento::Anuncio)), ..Default::default() });
        let segundos = |indice| {
            transicion_tras_indice(&lista, &HashMap::new(), &canciones, false, indice, &|_| true).map(|transicion| transicion.segundos)
        };
        assert_eq!(segundos(0), Some(0.0));
        assert_eq!(segundos(1), Some(0.0));
        assert_eq!(segundos(2), Some(6.0));
        let _ = std::fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn efecto_va_solo_en_la_botonera_y_seco_no_se_anota_como_comercial() {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-tipos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&carpeta);
        let mut canciones = RepositorioDeCanciones::new(carpeta.join("mezcla.json"));
        let tipo = |tipo: TipoDeElemento| CambiosDeDatos { tipo: Some(Some(tipo)), ..Default::default() };
        let (aplauso, cortina, tanda) = (Path::new("aplauso.wav"), Path::new("cortina.mp3"), Path::new("tanda.mp3"));
        canciones.cambiar_datos(aplauso, tipo(TipoDeElemento::Efecto));
        canciones.cambiar_datos(cortina, tipo(TipoDeElemento::Seco));
        canciones.cambiar_datos(tanda, tipo(TipoDeElemento::Publicidad));
        // El genero ya no decide: un tema de genero "Comercial" sigue siendo un Tema.
        let rock = Path::new("rock.mp3");
        canciones.cambiar_datos(rock, CambiosDeDatos { genero: Some("Comercial".into()), tipo: Some(Some(TipoDeElemento::Tema)), ..Default::default() });
        assert!(!es_facturable(&canciones, rock) && !suena_sin_mezcla(&canciones, &Pista::new(rock)));
        assert!(es_solo_para_la_botonera(&canciones, aplauso));
        assert!(!es_solo_para_la_botonera(&canciones, cortina));
        assert!(!es_solo_para_la_botonera(&canciones, Path::new("sin-ficha.mp3")));
        assert!(es_facturable(&canciones, tanda));
        assert!(!es_facturable(&canciones, cortina), "un tema seco no va al registro de comerciales");
        assert!(!es_facturable(&canciones, aplauso));
        let promocion = Path::new("promocion.mp3");
        canciones.cambiar_datos(promocion, tipo(TipoDeElemento::Anuncio));
        assert!(!es_facturable(&canciones, promocion), "un anuncio de la emisora no se factura");
        assert!(suena_sin_mezcla(&canciones, &Pista::new(promocion)));
        // Seco tambien suena sin mezcla.
        assert!(suena_sin_mezcla(&canciones, &Pista::new(cortina)));
        let _ = std::fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn el_efecto_suena_solo_el_tramo_fijado_a_mano() {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-recorte-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&carpeta);
        let mut canciones = RepositorioDeCanciones::new(carpeta.join("mezcla.json"));
        // Como un efecto que nunca estuvo en la lista: su ficha no tiene
        // duracion, solo los puntos que se guardaron en el editor.
        let archivo = PathBuf::from("bocina.wav");
        assert_eq!(recorte_de_efecto(&canciones, &archivo), RecorteDeEfecto::default(), "sin ficha de puntos");
        let recortes = PuntosDeMezcla::default().con_recortes(0.5, 1.5).unwrap();
        // Los recortes automaticos (del analisis) no se aplican a los efectos.
        canciones.guardar_puntos(&archivo, recortes);
        assert_eq!(recorte_de_efecto(&canciones, &archivo), RecorteDeEfecto::default());
        canciones.guardar_puntos(&archivo, recortes.como_manuales(true));
        assert!(canciones.buscar(&archivo).unwrap().duracion.is_none());
        let recorte = recorte_de_efecto(&canciones, &archivo);
        assert_eq!(recorte, RecorteDeEfecto { desde: 0.5, antes_del_final: 1.5 });
        assert_eq!(recorte.duracion(Some(6.0)), Some(4.0));
        let _ = std::fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn dice_cuanto_hace_omitiendo_ceros_y_en_singular_cuando_es_uno() {
        let ahora = momento("2026-10-23T12:34:15Z");
        assert_eq!(hace(&momento("2026-10-23T09:14:00Z"), &ahora), "Hace 3 horas 20 minutos y 15 segundos");
        assert_eq!(hace(&momento("2026-10-23T11:19:14Z"), &ahora), "Hace 1 hora 15 minutos y 1 segundo");
        assert_eq!(hace(&momento("2025-07-11T16:04:15Z"), &ahora), "Hace 1 año 3 meses 11 días 20 horas y 30 minutos");
        assert_eq!(hace(&momento("2024-09-22T12:34:15Z"), &ahora), "Hace 2 años 1 mes y 1 día");
        assert_eq!(hace(&momento("2026-10-23T12:34:05Z"), &ahora), "Hace 10 segundos");
        assert_eq!(hace(&momento("2026-10-23T12:34:15Z"), &ahora), "Recien");
    }

    #[test]
    fn marca_el_overlay_y_la_outro_en_el_progreso() {
        let ventana = crate::dominio::VentanaDeOverlay::nueva(10.0, 25.0).unwrap();
        let puntos = PuntosDeMezcla::nuevos(8.0, 12.0, Some(ventana)).unwrap().con_recortes(0.5, 2.0).unwrap();
        let marcas = MarcasDeProgreso::de(&puntos, 200.0);
        assert_eq!(marcas.overlay, (10.0, 25.0));
        assert_eq!(marcas.outro, (188.0, 198.0));
        assert!(!marcas.overlay_manual && !marcas.outro_manual);
        let manuales = puntos.como_manuales(true).con_overlay_manual(true);
        let marcas = MarcasDeProgreso::de(&manuales, 200.0);
        assert!(marcas.overlay_manual && marcas.outro_manual);
        let sin_nada = PuntosDeMezcla::nuevos(0.0, 0.0, None).unwrap();
        assert_eq!(MarcasDeProgreso::de(&sin_nada, 200.0).overlay, MarcasDeProgreso::SIN_TRAMO);
        assert_eq!(MarcasDeProgreso::de(&sin_nada, 200.0).outro, MarcasDeProgreso::SIN_TRAMO);
    }

    #[test]
    fn los_minutos_se_muestran_con_decimas() {
        assert_eq!(formatear_minutos(270.66), "4:30.6");
        assert_eq!(formatear_minutos(5.0), "0:05.0");
        assert_eq!(formatear_minutos(59.99), "0:59.9");
        assert_eq!(formatear_minutos(3725.4), "62:05.4");
    }

    #[test]
    fn la_fecha_se_muestra_en_dia_mes_anio() {
        assert!(fecha_local("2026-10-23T12:34:00Z").contains("/10/2026"));
        assert_eq!(fecha_local("no es fecha"), "no es fecha");
    }
}

use crate::audio::{Analisis, Decodificador, FormaDeOnda, PrioridadDeHilo, fijar_prioridad_del_hilo_actual};
pub use salida_de_preescucha::SalidaDePreescucha;
use hth::{
    FRANJA_DEL_ANUNCIO_HTH, TIPO_DEL_ANUNCIO_HTH, titulo_del_anuncio_hth, elegir_carpeta_de_locuciones_con_dialogo, es_anuncio_hth,
    es_base_hth, es_base_hth_en, insertar_anuncio_hth_con_dialogo,
};
use marcadores::{TIPO_DEL_MARCADOR, ayuda_del_marcador, es_marcador, marcador_de, titulo_del_marcador};
use crate::dominio::{
    AudiosDeCarpeta, audios_de_carpeta,
    Duplicado, EXTENSIONES_SOPORTADAS, ListaDeReproduccion, MarcadorDeBloque, Pista, PuntosDeMezcla, buscar_duplicados, clave_de_tema,
    formatear_duracion, formatear_tiempo, formatear_ventana, nombre_de_archivo, ReglasDelTipo, TipoDeElemento,
    ConsultaDeTemas, DatosDelTema, saltar_a_coincidencia, ModoDeOperacion, OperacionSobreLaSecuencia,
};
use crate::persistencia::{
    FORMATO_DE_FECHA_Y_HORA,
    CANTIDAD_DE_BOTONES, CambiosDeDatos, EmisionDeComercial, FADER_MAXIMO, FADER_POR_DEFECTO, FichaDeCancion,
    RANKING_MAXIMO, ListaGuardada, AlmacenDeAnunciantes, AlmacenDeEmisiones, AlmacenDeOrdenes, AlmacenesDePublicidad, AlmacenDePautas, AlmacenDeTandas, RepositorioDeBotonera, RepositorioDeCanciones,
    RepositorioDeLista, RepositorioDePreferencias, UltimoTema,
    carpeta_de_listas, carpeta_del_usuario, escribir_lista, leer_lista, momento_actual,
};
use crate::servicio::{
    AjustesDeEfecto, Azar, Curva, DatosDeMezcla, Entorno, EstadoDeEspera, EventoDeReproduccion, GestorDeOverlay,
    GestorDeReproduccion, ModoDeOverlay, MusicaAlrededor, ObjetivoDeSonoridad, RecorteDeEfecto,
    ServicioDeClima, Transicion, transicion_inteligente,
};
use crate::{AcercaDe, BotonDeEfecto, EditorDePista, FichaSinArchivo, FilaDePista, MantenimientoDelCatalogo, VentanaPrincipal};

const INTERVALO_DEL_CICLO: Duration = Duration::from_millis(50);
// Mientras se arrastra la barra de progreso solo se busca cuando el usuario
// se detiene un instante; un clic busca enseguida en el ciclo siguiente.
const ESPERA_PARA_BUSCAR: Duration = Duration::from_millis(150);
// La atenuacion se guarda al terminar de moverla, no en cada paso.
const ESPERA_PARA_GUARDAR_ATENUACION: Duration = Duration::from_millis(500);
const DURACION_DE_AVISO_DE_BOTONERA: Duration = Duration::from_secs(3);
const ESPERA_DE_LA_INTERFAZ: Duration = Duration::from_secs(10);
// Diferencia (s) hasta la que intro y outro de una version anterior se
// consideran sugeridas y no puestas a mano.
const TOLERANCIA_DE_PUNTOS_SUGERIDOS: f64 = 0.15;
pub const VOLUMEN_POR_DEFECTO: f64 = 0.8;

/// Formatos de la etiqueta de tiempo; un clic pasa al siguiente. Empieza en
/// segundos restantes, que sirve para leer el tiempo que falta y fijar el
/// punto de overlay.
#[derive(Clone, Copy)]
enum FormatoDeTiempo {
    Transcurrido,
    Restante,
    SegundosRestantes,
    SegundosTranscurridos,
}

impl FormatoDeTiempo {
    fn siguiente(self) -> Self {
        match self {
            Self::Transcurrido => Self::Restante,
            Self::Restante => Self::SegundosRestantes,
            Self::SegundosRestantes => Self::SegundosTranscurridos,
            Self::SegundosTranscurridos => Self::Transcurrido,
        }
    }
}

/// Botones de transporte, tal como los pide un cliente externo.
#[derive(Clone, Copy, Debug)]
pub enum AccionDeTransporte {
    Reproducir,
    Pausar,
    Alternar,
    /// Al aire, `salir_del_aire` pasa a modo EDITOR al detener; si no, sigue
    /// al aire para cuando se vuelva a reproducir.
    Detener { salir_del_aire: bool },
    Siguiente,
    Anterior,
}

/// Valores de deslizantes y opciones a cambiar; None deja el actual.
#[derive(Default)]
pub struct Ajustes {
    pub volumen: Option<f64>,
    pub fader: Option<u8>,
    pub posicion: Option<f64>,
    pub repetir: Option<bool>,
    pub botonera_visible: Option<bool>,
    pub modo_libre: Option<bool>,
    pub bajar_musica: Option<bool>,
    pub nivel_de_musica: Option<f64>,
    pub atenuacion_inteligente: Option<bool>,
    /// Volumen de los efectos en dB.
    pub volumen_de_efecto: Option<f64>,
    /// Segundos que tarda la musica en bajar antes de un efecto.
    pub anticipacion: Option<f64>,
    /// Segundos que tarda la musica en volver despues de un efecto.
    pub recuperacion: Option<f64>,
    /// Con ajuste inteligente, dB que la musica queda debajo del efecto.
    pub separacion: Option<f64>,
    pub crossfade_inteligente: Option<bool>,
    pub nivelar_volumen: Option<bool>,
    pub objetivo_de_sonoridad: Option<ObjetivoDeSonoridad>,
}

pub struct Controlador {
    ventana: slint::Weak<VentanaPrincipal>,
    lista: ListaDeReproduccion,
    gestor: GestorDeReproduccion,
    overlay: GestorDeOverlay,
    repositorio_de_canciones: RepositorioDeCanciones,
    repositorio_de_botonera: RepositorioDeBotonera,
    repositorio_de_preferencias: RepositorioDePreferencias,
    filas: Rc<VecModel<FilaDePista>>,
    botones: Rc<VecModel<BotonDeEfecto>>,
    seleccion: BTreeSet<usize>,
    ancla_de_seleccion: Option<usize>,
    volumen_seleccionado: f64,
    formato_de_tiempo: FormatoDeTiempo,
    busqueda_pendiente: Option<(f64, Instant)>,
    atenuacion_por_guardar: Option<Instant>,
    aviso_de_botonera: Option<(String, Instant)>,
    estado_anterior: (bool, bool),
    // Errores a mostrar en un dialogo cuando se libere el controlador.
    errores: Vec<String>,
    // Editor de pistas y su preescucha, con una voz propia del mezclador o
    // por otra salida de audio.
    editor: EditorDePista,
    // Mantenimiento del catalogo: su ventana, las fichas sin archivo que
    // muestra y la busqueda vigente (una vieja se descarta).
    mantenimiento: MantenimientoDelCatalogo,
    // Buscador del catalogo: su ventana, los resultados y lo elegido.
    buscador: buscador::BuscadorDeCatalogo,
    // Selector de la localidad del clima del anuncio HTH.
    selector_de_localidad: localidad::SelectorDeLocalidadDelClima,
    actualizacion: actualizacion::ActualizacionDelCatalogo,
    modelo_de_mantenimiento: Rc<VecModel<FichaSinArchivo>>,
    fichas_perdidas: Vec<mantenimiento::FichaPerdida>,
    generacion_de_busqueda: u64,
    edicion: Option<editor::Edicion>,
    generacion_de_onda: u64,
    preescucha: SalidaDePreescucha,
    preescucha_en_curso: Option<editor::Preescucha>,
    // Duracion conocida de cada pista de la lista, en el mismo orden; se
    // recalcula al cambiar la lista y sirve para los totales de cada ciclo.
    duraciones: Vec<Option<f64>>,
    // Segundos que se ahorra la mezcla tras cada pista y, desde cada posicion
    // hasta el final, lo que falta sonar (uno mas que pistas). Se calculan al
    // refrescar la lista: el ciclo solo suma lo que va de la pista actual.
    ahorros_de_mezcla: Vec<f64>,
    netos_desde: Vec<f64>,
    // Hay un crossfade iniciado con el boton Siguiente (no por la outro).
    mezcla_manual: bool,
    // Pista (y su posicion) que arranca cuando termine el fundido de salida:
    // Siguiente hacia o desde un tipo sin mezcla no mezcla.
    tras_el_fundido: Option<(usize, Pista)>,
    // Lista con nombre abierta (None: sin nombre) y su contenido tal como se
    // guardo, para saber si hay cambios sin guardar.
    archivo_de_lista: Option<PathBuf>,
    contenido_guardado: Vec<Pista>,
    repositorio_de_lista: RepositorioDeLista,
    // Cuenta los ciclos, para tareas que alcanza con hacer una vez por segundo.
    ciclos: u64,
    // Hoja y boton del ultimo efecto pulsado, para resaltar ese boton (y no
    // otros que tengan el mismo archivo).
    boton_del_efecto: Option<(usize, usize)>,
    // Duracion de cada archivo de efecto, para mostrarla en su boton.
    duraciones_de_efectos: RefCell<std::collections::HashMap<PathBuf, Option<f64>>>,
    // Pista que esta sonando (o en pausa), para registrar su ultima
    // reproduccion cuando deje de sonar.
    pista_en_curso: Option<Pista>,
    // Cuando empezo a sonar (hora local) y hasta donde llego, para anotar
    // los comerciales en su registro.
    inicio_en_curso: String,
    posicion_en_curso: f64,
    registro_de_comerciales: Box<dyn AlmacenDeEmisiones>,
    // Tandas programadas y, de los avisos que encolaron, a que tanda
    // pertenecen ("Tanda A 10:30"), para anotarlo en el registro.
    tandas: Box<dyn AlmacenDeTandas>,
    pistas_de_tanda: HashMap<Pista, std::collections::VecDeque<ordenes::OrigenDelAviso>>,
    // Ordenes de publicidad, las pautas de cada dia y lo que hay que reponer hoy.
    ordenes: Box<dyn AlmacenDeOrdenes>,
    pautas: Box<dyn AlmacenDePautas>,
    // Fichas de los anunciantes.
    anunciantes: Box<dyn AlmacenDeAnunciantes>,
    pauta_calculada: Option<ordenes::PautaCalculada>,
    reposiciones: Vec<crate::dominio::Reposicion>,
    fecha_de_reposiciones: Option<jiff::civil::Date>,
    ventana_de_ordenes: ordenes::VentanaDeOrdenes,
    // Selector de avisos del catalogo para las tandas y las ordenes.
    avisos_del_catalogo: avisos::VentanaDeAvisos,
    // Ventana de las fichas de los anunciantes.
    anunciantes_de_la_ventana: anunciantes::VentanaDeAnunciantes,
    // Acerca de, para traerlo con las demas ventanas.
    acerca: Option<slint::Weak<AcercaDe>>,
    // Tanda a hora exacta que espera su momento para bajar lo que suena.
    corte_de_tanda: Option<tandas::CorteDeTanda>,
    pauta: pauta::VentanaDePauta,
    // Busqueda en la lista y posiciones que coinciden, en orden.
    busqueda: Option<ConsultaDeTemas>,
    // Problema de la busqueda escrita (un prefijo con un valor que no se entiende).
    problema_de_busqueda: Option<String>,
    coincidencias: Vec<usize>,
    // Analisis de cada tema para el crossfade inteligente y la nivelacion, y
    // los que se estan calculando.
    datos_de_mezcla: HashMap<PathBuf, DatosDeMezcla>,
    analisis_pedidos: HashSet<PathBuf>,
    // Llegaron analisis nuevos: hay que refrescar la lista en el proximo ciclo.
    filas_por_refrescar: bool,
    // Fila que se centro por ultima vez con "Desplazar", para centrar solo
    // al cambiar de tema.
    fila_centrada: Option<usize>,
    // Al aire, se detuvo con el boton: al quedar detenida se pregunta si
    // salir del modo AL AIRE (la pregunta queda pendiente para el ciclo).
    detenida_al_aire: bool,
    pregunta_de_salir_del_aire: bool,
    // Forma de onda del ultimo tema sobre el que sono un efecto, para medir
    // como suena la musica en ese tramo.
    onda_de_musica: RefCell<Option<(PathBuf, Rc<FormaDeOnda>)>>,
    // Tema cuyas barras de nivel muestra el panel de detalles (o se piden).
    onda_del_detalle: RefCell<Option<PathBuf>>,
    // Sube con cada pedido de barras: la respuesta de uno anterior se descarta.
    generacion_de_barras: Cell<u64>,
    // Tema cuya carátula muestra el panel de detalles (o se busca), las ya
    // vistas por archivo (None: no tiene) y el pedido vigente, que tambien lee
    // el hilo que la busca para no salir a internet por un tema que ya paso.
    caratula_del_detalle: RefCell<Option<PathBuf>>,
    caratulas_vistas: RefCell<HashMap<PathBuf, caratula::CaratulaVista>>,
    generacion_de_caratula: std::sync::Arc<std::sync::atomic::AtomicU64>,
    // Lo mismo para el tema elegido en el catalogo.
    caratula_del_catalogo: RefCell<Option<PathBuf>>,
    generacion_de_caratula_del_catalogo: std::sync::Arc<std::sync::atomic::AtomicU64>,
    // Sube al empezar o detener una descarga de carátulas desde el catalogo.
    descarga_de_caratulas: std::sync::Arc<std::sync::atomic::AtomicU64>,
    // Clima para el anuncio HTH, la duracion del ultimo armado y la eleccion
    // de tomas de las locuciones.
    clima: ServicioDeClima,
    duracion_del_anuncio_hth: Option<f64>,
    azar_de_locuciones: Azar,
    // Ultima base musical insertada (para no repetirla) y lo que duro la
    // ultima voz dicha sobre una base.
    ultima_base_hth: Option<PathBuf>,
    duracion_de_la_voz_hth: Option<f64>,
}

/// Accion sobre las hojas de la botonera, tal como la pide un cliente externo.
#[derive(Clone, Debug)]
pub enum AccionDeHoja {
    /// Mostrar la hoja indicada (desde 0).
    Ir(usize),
    Agregar,
    /// Quitar la hoja indicada (desde 0).
    Quitar(usize),
    /// Ponerle nombre a la hoja indicada (desde 0; None, la que esta a la
    /// vista); vacio vuelve al de siempre.
    Renombrar(Option<usize>, String),
}

thread_local! {
    static CONTROLADOR: RefCell<Option<Rc<RefCell<Controlador>>>> = const { RefCell::new(None) };
}

/// Crea el controlador, cablea los callbacks de la ventana y arranca el ciclo
/// periodico. El temporizador devuelto debe mantenerse vivo.
/// Fuente de ancho fijo de los relojes, una que trae cada sistema.
const FUENTE_MONOESPACIADA: &str = if cfg!(windows) {
    "Consolas"
} else if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "DejaVu Sans Mono"
};

pub fn conectar(
    ventana: &VentanaPrincipal,
    gestor: GestorDeReproduccion,
    overlay: GestorDeOverlay,
    repositorio_de_canciones: RepositorioDeCanciones,
    repositorio_de_botonera: RepositorioDeBotonera,
    repositorio_de_preferencias: RepositorioDePreferencias,
    preescucha: SalidaDePreescucha,
    repositorio_de_lista: RepositorioDeLista,
    almacenes: AlmacenesDePublicidad,
) -> Result<Timer, String> {
    let AlmacenesDePublicidad { emisiones: registro_de_comerciales, tandas, ordenes, pautas, anunciantes } = almacenes;
    ventana.set_fuente_monoespaciada(FUENTE_MONOESPACIADA.into());
    aplicar_idioma_en_slint(crate::idioma::actual());
    let editor = EditorDePista::new().map_err(|error| tr!("No se pudo crear el editor de pistas: {error}", error = error))?;
    editor::cablear(&editor);
    let ventana_de_mantenimiento = MantenimientoDelCatalogo::new()
        .map_err(|error| tr!("No se pudo crear la ventana de mantenimiento del catalogo: {error}", error = error))?;
    mantenimiento::cablear(&ventana_de_mantenimiento);
    let modelo_de_mantenimiento = Rc::new(VecModel::default());
    ventana_de_mantenimiento.set_fichas(ModelRc::from(modelo_de_mantenimiento.clone()));
    let filas = Rc::new(VecModel::default());
    let botones = Rc::new(VecModel::default());
    ventana.set_filas(ModelRc::from(filas.clone()));
    ventana.set_botones(ModelRc::from(botones.clone()));

    let mut controlador = Controlador {
        ventana: ventana.as_weak(),
        lista: ListaDeReproduccion::new(),
        gestor,
        overlay,
        repositorio_de_canciones,
        repositorio_de_botonera,
        repositorio_de_preferencias,
        filas,
        botones,
        seleccion: BTreeSet::new(),
        ancla_de_seleccion: None,
        volumen_seleccionado: VOLUMEN_POR_DEFECTO,
        formato_de_tiempo: FormatoDeTiempo::SegundosRestantes,
        busqueda_pendiente: None,
        atenuacion_por_guardar: None,
        aviso_de_botonera: None,
        estado_anterior: (false, false),
        errores: Vec::new(),
        editor,
        mantenimiento: ventana_de_mantenimiento,
        buscador: buscador::BuscadorDeCatalogo::nuevo()?,
        selector_de_localidad: localidad::SelectorDeLocalidadDelClima::nuevo()?,
        actualizacion: actualizacion::ActualizacionDelCatalogo::nueva()?,
        modelo_de_mantenimiento,
        fichas_perdidas: Vec::new(),
        generacion_de_busqueda: 0,
        edicion: None,
        generacion_de_onda: 0,
        preescucha,
        preescucha_en_curso: None,
        duraciones: Vec::new(),
        ahorros_de_mezcla: Vec::new(),
        netos_desde: vec![0.0],
        mezcla_manual: false,
        tras_el_fundido: None,
        archivo_de_lista: None,
        contenido_guardado: Vec::new(),
        repositorio_de_lista,
        ciclos: 0,
        boton_del_efecto: None,
        duraciones_de_efectos: RefCell::default(),
        pista_en_curso: None,
        inicio_en_curso: String::new(),
        posicion_en_curso: 0.0,
        registro_de_comerciales,
        tandas,
        pistas_de_tanda: ordenes::sin_origenes(),
        ordenes,
        pautas,
        anunciantes,
        pauta_calculada: None,
        reposiciones: Vec::new(),
        fecha_de_reposiciones: None,
        ventana_de_ordenes: ordenes::VentanaDeOrdenes::nueva()?,
        avisos_del_catalogo: avisos::VentanaDeAvisos::nueva()?,
        anunciantes_de_la_ventana: anunciantes::VentanaDeAnunciantes::nueva()?,
        acerca: None,
        corte_de_tanda: None,
        pauta: pauta::VentanaDePauta::nueva()?,
        busqueda: None,
        problema_de_busqueda: None,
        coincidencias: Vec::new(),
        datos_de_mezcla: HashMap::new(),
        analisis_pedidos: HashSet::new(),
        filas_por_refrescar: false,
        fila_centrada: None,
        detenida_al_aire: false,
        pregunta_de_salir_del_aire: false,
        onda_de_musica: RefCell::default(),
        onda_del_detalle: RefCell::default(),
        generacion_de_barras: Cell::new(0),
        caratula_del_detalle: RefCell::default(),
        caratulas_vistas: RefCell::default(),
        generacion_de_caratula: std::sync::Arc::default(),
        caratula_del_catalogo: RefCell::default(),
        generacion_de_caratula_del_catalogo: std::sync::Arc::default(),
        descarga_de_caratulas: std::sync::Arc::default(),
        clima: ServicioDeClima::default(),
        duracion_del_anuncio_hth: None,
        azar_de_locuciones: Azar::nuevo(),
        ultima_base_hth: None,
        duracion_de_la_voz_hth: None,
    };
    controlador.inicializar(ventana);
    controlador.aplicar_idioma_en_las_ventanas();
    controlador.recuperar_lista();
    // El clima se consulta siempre: lo muestra el reloj (y lo dice el anuncio HTH).
    controlador.iniciar_clima();
    controlador.refrescar_clima();
    CONTROLADOR.with(|celda| *celda.borrow_mut() = Some(Rc::new(RefCell::new(controlador))));

    cablear_callbacks(ventana)?;

    let temporizador = Timer::default();
    let debil = ventana.as_weak();
    temporizador.start(TimerMode::Repeated, INTERVALO_DEL_CICLO, move || {
        let mut preguntar = false;
        con_controlador(|c| {
            c.ciclo();
            preguntar = std::mem::take(&mut c.pregunta_de_salir_del_aire);
        });
        // El dialogo va fuera de este ciclo: mientras esta abierto, el
        // temporizador sigue andando.
        if preguntar {
            let debil = debil.clone();
            Timer::single_shot(Duration::ZERO, move || preguntar_si_salir_del_aire(&debil, true));
        }
    });
    Ok(temporizador)
}

/// Ejecuta una accion sobre el controlador y, ya liberado, muestra los
/// errores que haya dejado. Si el controlador esta ocupado (un dialogo
/// abierto), la accion se descarta: solo lo usan eventos repetibles.
fn con_controlador(accion: impl FnOnce(&mut Controlador)) {
    let errores = CONTROLADOR.with(|celda| {
        let controlador = celda.borrow().clone()?;
        let mut controlador = controlador.try_borrow_mut().ok()?;
        accion(&mut controlador);
        Some(std::mem::take(&mut controlador.errores))
    });
    for error in errores.into_iter().flatten() {
        mostrar_error(&error);
    }
}

/// Elige la traduccion de los textos de Slint (@tr) en todas las ventanas.
fn aplicar_idioma_en_slint(idioma: Idioma) {
    if let Err(error) = slint::select_bundled_translation(idioma.codigo_para_slint()) {
        log::warn!("No se pudo elegir el idioma {} en la interfaz: {error}", idioma.codigo());
    }
}

impl Controlador {
    /// Marca el idioma en las ventanas que tienen textos con formato escritos
    /// en los dos idiomas, y en el menu.
    fn aplicar_idioma_en_las_ventanas(&self) {
        let ingles = crate::idioma::en_ingles();
        self.con_ventana(|ventana| {
            ventana.global::<IdiomaDeLaInterfaz>().set_ingles(ingles);
            ventana.set_idioma(i32::from(ingles));
        });
        self.editor.global::<IdiomaDeLaInterfaz>().set_ingles(ingles);
        self.buscador.ventana_slint().global::<IdiomaDeLaInterfaz>().set_ingles(ingles);
        self.mostrar_formato_de_fecha();
    }

    /// El formato de fecha elegido en el menu y en el editor (que lo muestra
    /// en el globo del campo Sono).
    fn mostrar_formato_de_fecha(&self) {
        use crate::formato_de_fecha::{FormatoDeFecha, actual};
        let nombres: Vec<slint::SharedString> = FormatoDeFecha::TODOS.iter().map(|formato| formato.nombre().into()).collect();
        let elegido = FormatoDeFecha::TODOS.iter().position(|formato| *formato == actual()).unwrap_or(0) as i32;
        self.con_ventana(|ventana| {
            ventana.set_formatos_de_fecha(ModelRc::new(VecModel::from(nombres)));
            ventana.set_formato_de_fecha(elegido);
        });
        self.editor.set_patron_de_fecha(actual().para_escribir().into());
        let ejemplo: slint::SharedString = actual().ejemplo().into();
        self.con_ventana(|ventana| ventana.global::<IdiomaDeLaInterfaz>().set_ejemplo_de_fecha(ejemplo.clone()));
        self.buscador.ventana_slint().global::<IdiomaDeLaInterfaz>().set_ejemplo_de_fecha(ejemplo);
    }

    /// Menu Ventana > Formato de fecha: cambia como se ven (y se escriben)
    /// las fechas en toda la interfaz y lo recuerda.
    fn elegir_formato_de_fecha(&mut self, indice: i32) {
        use crate::formato_de_fecha::FormatoDeFecha;
        let formato = usize::try_from(indice).ok().and_then(|indice| FormatoDeFecha::TODOS.get(indice).copied()).unwrap_or_default();
        crate::formato_de_fecha::establecer(formato);
        self.repositorio_de_preferencias.guardar_formato_de_fecha(formato);
        self.mostrar_formato_de_fecha();
        self.refrescar_reloj();
        self.refrescar_filas();
        self.refrescar_detalles();
        self.refrescar_buscador_si_esta_abierto();
        self.refrescar_tabla_de_la_pauta();
        self.refrescar_sono_del_editor();
        self.mostrar_estado(&tr!("Formato de fecha: {}", formato.nombre()));
    }

    /// Menu Ventana > Idioma: cambia el idioma de toda la interfaz al momento
    /// y lo recuerda para la proxima vez.
    fn elegir_idioma(&mut self, indice: i32) {
        let idioma = usize::try_from(indice).ok().and_then(|indice| Idioma::TODOS.get(indice).copied()).unwrap_or_default();
        crate::idioma::establecer(idioma);
        self.repositorio_de_preferencias.guardar_idioma(idioma);
        aplicar_idioma_en_slint(idioma);
        self.aplicar_idioma_en_las_ventanas();
        // Los textos que arma Rust se rehacen en el idioma nuevo (tambien el
        // origen de la carátula, que se vuelve a mostrar).
        *self.caratula_del_detalle.borrow_mut() = None;
        self.refrescar_filas();
        self.refrescar_interfaz();
        self.refrescar_detalles();
        self.refrescar_titulo();
        self.refrescar_clima();
        self.refrescar_tabla_de_la_pauta();
        self.refrescar_ordenes_si_estan_abiertas();
        self.refrescar_tabla_de_anunciantes();
        self.mostrar_estado(&tr!("Idioma de la interfaz: {}", idioma.nombre()));
    }
}

pub fn mostrar_error(mensaje: &str) {
    log::error!("{mensaje}");
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(&tr!("Error de la aplicacion"))
        .set_description(tr!("Ocurrio un problema inesperado\n\n{mensaje}", mensaje = mensaje))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Ejecuta una tarea en el hilo de la interfaz y espera su resultado. La usan
/// los clientes externos (servidor MCP), que corren en otros hilos.
pub fn ejecutar_en_interfaz<T: Send + 'static>(
    tarea: impl FnOnce(&mut Controlador) -> T + Send + 'static,
) -> Result<T, String> {
    let (emisor, receptor) = mpsc::channel();
    slint::invoke_from_event_loop(move || intentar_en_interfaz(tarea, emisor))
        .map_err(|error| tr!("La interfaz no esta disponible: {error}", error = error))?;
    receptor.recv_timeout(ESPERA_DE_LA_INTERFAZ).map_err(|_| tr!("La interfaz no respondio a tiempo"))
}

/// Como `ejecutar_en_interfaz`, pero sin esperar el resultado.
fn programar_en_interfaz(tarea: impl FnOnce(&mut Controlador) + Send + 'static) {
    let (emisor, _receptor) = mpsc::channel();
    let _ = slint::invoke_from_event_loop(move || intentar_en_interfaz(tarea, emisor));
}

fn intentar_en_interfaz<T: Send + 'static>(
    tarea: impl FnOnce(&mut Controlador) -> T + Send + 'static,
    emisor: mpsc::Sender<T>,
) {
    let Some(controlador) = CONTROLADOR.with(|celda| celda.borrow().clone()) else {
        return;
    };
    let Ok(mut controlador) = controlador.try_borrow_mut() else {
        // Ocupado con un dialogo: se reintenta enseguida sin bloquear.
        Timer::single_shot(INTERVALO_DEL_CICLO, move || intentar_en_interfaz(tarea, emisor));
        return;
    };
    let resultado = tarea(&mut controlador);
    let errores = std::mem::take(&mut controlador.errores);
    drop(controlador);
    let _ = emisor.send(resultado);
    for error in errores {
        mostrar_error(&error);
    }
}

/// Lee duracion y etiquetas de los archivos en otro hilo, para no frenar la
/// interfaz con carpetas grandes, y las guarda en la ficha de cada cancion.
/// La forma de onda (y con ella los puntos automaticos) se calcula aparte.
fn analizar_en_segundo_plano(archivos: Vec<PathBuf>) {
    // El anuncio HTH cambia cada vez que se arma: no tiene ficha.
    let archivos: Vec<PathBuf> =
        archivos.into_iter().filter(|archivo| !es_anuncio_hth(archivo) && !es_marcador(archivo)).collect();
    if archivos.is_empty() {
        return;
    }
    let lanzado = std::thread::Builder::new().name("analisis-de-archivos".into()).spawn(move || {
        fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Fondo);
        let mut resultados = Vec::new();
        for archivo in archivos {
            match Decodificador::analizar(&archivo) {
                Ok(analisis) => resultados.push((archivo, analisis)),
                Err(error) => log::warn!("No se pudo analizar {}: {error}", archivo.display()),
            }
        }
        programar_en_interfaz(move |controlador| {
            controlador.repositorio_de_canciones.completar_con_analisis(&resultados);
            // Un tipo sin mezcla leido de las etiquetas deja solo inicio y fin.
            for (archivo, _) in &resultados {
                controlador.reaplicar_puntos_automaticos(archivo);
            }
            controlador.sincronizar_puntos(resultados.iter().map(|(archivo, _)| archivo.as_path()));
            controlador.refrescar_filas();
            controlador.sacar_de_la_lista_los_efectos();
        });
    });
    if let Err(error) = lanzado {
        log::warn!("No se pudo iniciar el analisis de archivos: {error}");
    }
}

/// Calcula (o lee de la cache) la forma de onda de estos archivos en otro
/// hilo y entrega sus datos de mezcla de a uno, a medida que estan.
fn analizar_mezcla_en_segundo_plano(archivos: Vec<PathBuf>) {
    if archivos.is_empty() {
        return;
    }
    let cache = carpeta_del_usuario().join("ondas");
    let lanzado = std::thread::Builder::new().name("analisis-de-mezcla".into()).spawn(move || {
        fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Fondo);
        for archivo in archivos {
            let datos = match FormaDeOnda::cargar_o_calcular(&archivo, &cache) {
                Ok(onda) => Some(DatosDeMezcla::de_la_onda(&onda)),
                Err(error) => {
                    log::warn!("No se pudo analizar la mezcla de {}: {error}", archivo.display());
                    None
                }
            };
            programar_en_interfaz(move |controlador| controlador.recibir_datos_de_mezcla(archivo, datos));
        }
    });
    if let Err(error) = lanzado {
        log::warn!("No se pudo iniciar el analisis de mezcla: {error}");
    }
}

fn elegir_audios(ventana: &slint::Weak<VentanaPrincipal>, titulo: &str, filtro: &str) -> rfd::FileDialog {
    let mut dialogo = rfd::FileDialog::new().set_title(titulo).add_filter(filtro, EXTENSIONES_SOPORTADAS);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    dialogo
}

/// Pregunta antes de cerrar si hay musica sonando.
fn confirmar_salida(ventana: &slint::Weak<VentanaPrincipal>) -> bool {
    let mut dialogo = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(&tr!("Salir de MCP Raptor Player"))
        .set_description(&tr!("Hay musica sonando. ¿Desea salir del programa?"))
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    dialogo.show() == rfd::MessageDialogResult::Yes
}

fn cablear_callbacks(ventana: &VentanaPrincipal) -> Result<(), String> {
    let debil = ventana.as_weak();
    ventana.window().on_close_requested(move || {
        let mut sonando = false;
        con_controlador(|c| sonando = c.gestor.esta_reproduciendo());
        if sonando && !confirmar_salida(&debil) {
            return CloseRequestResponse::KeepWindowShown;
        }
        con_controlador(|c| {
            c.registrar_fin_de_reproduccion();
            c.guardar_lista();
            // Ninguna ventana hija queda abierta mientras el programa termina.
            let cerradas = c.cerrar_ventanas_hijas();
            log::info!("Al salir se cerraron {cerradas} ventana(s) hija(s)");
        });
        let _ = slint::quit_event_loop();
        CloseRequestResponse::HideWindow
    });

    let acerca = AcercaDe::new().map_err(|error| tr!("No se pudo crear la ventana Acerca de: {error}", error = error))?;
    acerca.set_version(env!("CARGO_PKG_VERSION").into());
    acerca.set_correo(CORREO_DE_CONTACTO.into());
    acerca.on_escribir_al_autor(|| {
        if let Err(error) = abrir_enlace(&format!("mailto:{CORREO_DE_CONTACTO}")) {
            log::warn!("No se pudo abrir el programa de correo: {error}");
        }
    });
    acerca.set_repositorio(REPOSITORIO.trim_start_matches("https://").into());
    acerca.on_donar(|| {
        if let Err(error) = abrir_enlace(DONACIONES) {
            log::warn!("No se pudo abrir el navegador en {DONACIONES}: {error}");
        }
    });
    acerca.on_abrir_repositorio(|| {
        if let Err(error) = abrir_enlace(REPOSITORIO) {
            log::warn!("No se pudo abrir el navegador en {REPOSITORIO}: {error}");
        }
    });
    let acerca_debil = acerca.as_weak();
    acerca.on_cerrar(move || {
        if let Some(acerca) = acerca_debil.upgrade() {
            let _ = acerca.hide();
        }
    });
    ventana.on_acerca_de(move || con_controlador(|c| c.abrir_acerca_de(&acerca)));
    let debil = ventana.as_weak();
    ventana.on_abrir_manual(move || abrir_manual(&debil));
    ventana.on_traer_editor(|| con_controlador(|c| c.traer_ventanas()));
    ventana.on_alternar_editor_centrado(|| con_controlador(|c| c.alternar_editor_centrado()));
    ventana.on_alternar_ocultar_textos(|| con_controlador(|c| c.alternar_ocultar_textos()));
    ventana.on_alternar_caratulas_de_internet(|| con_controlador(|c| c.alternar_caratulas_de_internet()));
    ventana.on_alternar_vista_del_detalle(|| con_controlador(Controlador::alternar_vista_del_detalle));
    ventana.on_elegir_idioma(|indice| con_controlador(|c| c.elegir_idioma(indice)));
    ventana.on_elegir_formato_de_fecha(|indice| con_controlador(|c| c.elegir_formato_de_fecha(indice)));
    ventana.on_mantenimiento_del_catalogo(|| {
        con_controlador(|c| {
            if let Err(error) = c.abrir_mantenimiento() {
                c.errores.push(error);
            }
        })
    });
    ventana.on_abrir_anunciantes(|| {
        con_controlador(|c| {
            if let Err(error) = c.abrir_anunciantes(None) {
                c.mostrar_estado(&error);
            }
        })
    });
    ventana.on_abrir_ordenes(|| {
        con_controlador(|c| {
            if let Err(error) = c.abrir_ordenes() {
                c.mostrar_estado(&error);
            }
        })
    });
    ventana.on_abrir_pauta(|| {
        con_controlador(|c| {
            if let Err(error) = c.abrir_pauta() {
                c.mostrar_estado(&error);
            }
        })
    });
    ventana.on_buscar_en_el_catalogo(|| {
        con_controlador(|c| {
            if let Err(error) = c.abrir_buscador_del_catalogo() {
                c.errores.push(error);
            }
        })
    });
    let debil = ventana.as_weak();
    ventana.on_encolar_como_siguiente(move |indice| {
        let indice = indice as usize;
        // Si el tema sale de un bloque fijo, se pregunta antes (sin tomar el
        // controlador mientras el dialogo esta abierto).
        let mut aviso = None;
        con_controlador(|c| aviso = c.aviso_de_salida_de_bloque(indice));
        if let Some(aviso) = aviso {
            let mut dialogo = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title(&tr!("Sacar del bloque fijo"))
                .set_description(aviso)
                .set_buttons(rfd::MessageButtons::OkCancel);
            if let Some(ventana) = debil.upgrade() {
                dialogo = dialogo.set_parent(&ventana.window().window_handle());
            }
            if dialogo.show() != rfd::MessageDialogResult::Ok {
                con_controlador(|c| c.mostrar_estado(&tr!("Encolar cancelado: el tema sigue en su bloque fijo")));
                return;
            }
        }
        con_controlador(|c| {
            if let Err(error) = c.encolar_como_siguiente(indice) {
                c.mostrar_estado(&error);
            }
        })
    });
    ventana.on_subir_seleccionada(|| con_controlador(|c| c.mover_seleccionada(-1)));
    ventana.on_bajar_seleccionada(|| con_controlador(|c| c.mover_seleccionada(1)));

    let debil = ventana.as_weak();
    ventana.on_agregar_archivos(move || {
        let filtro = tr!("Audio ({})", EXTENSIONES_SOPORTADAS.join(", "));
        let Some(archivos) = elegir_audios(&debil, &tr!("Seleccionar archivos de audio"), &filtro).pick_files() else {
            return;
        };
        let mut nuevas = Vec::new();
        con_controlador(|c| nuevas = c.agregar_y_resumir(archivos, ""));
        revisar_duplicados_agregados(debil.clone(), nuevas);
    });
    let debil = ventana.as_weak();
    ventana.on_agregar_carpeta(move || agregar_carpeta_con_dialogo(&debil));
    let debil = ventana.as_weak();
    ventana.on_validar_lista(move || validar_lista_con_dialogos(&debil));
    let debil = ventana.as_weak();
    ventana.on_cargar_lista(move || cargar_lista_con_dialogo(&debil));
    let debil = ventana.as_weak();
    ventana.on_guardar_lista(move || {
        guardar_lista_con_dialogo(&debil, false);
    });
    let debil = ventana.as_weak();
    ventana.on_guardar_lista_como(move || {
        guardar_lista_con_dialogo(&debil, true);
    });
    let debil = ventana.as_weak();
    ventana.on_quitar_seleccionadas(move || {
        let mut posiciones = Vec::new();
        con_controlador(|c| posiciones = c.seleccion.iter().copied().collect());
        if posiciones.is_empty() {
            con_controlador(|c| c.mostrar_estado(&tr!("Seleccione una pista para quitarla")));
            return;
        }
        quitar_con_confirmacion(&debil, posiciones);
    });
    let debil = ventana.as_weak();
    ventana.on_limpiar_lista(move || limpiar_con_confirmacion(&debil));
    ventana.on_editar_pista(|indice| con_controlador(|c| c.editar_pista_desde_la_lista(indice)));
    let debil = ventana.as_weak();
    ventana.on_quitar_fila(move |indice| {
        // Sobre una fila de la seleccion se quita toda la seleccion; sobre
        // otra, solo esa fila.
        let indice = indice as usize;
        let mut posiciones = vec![indice];
        con_controlador(|c| {
            if c.seleccion.contains(&indice) {
                posiciones = c.seleccion.iter().copied().collect();
            }
        });
        quitar_con_confirmacion(&debil, posiciones);
    });
    let debil = ventana.as_weak();
    ventana.on_descatalogar_fila(move |indice| descatalogar_con_dialogo(&debil, indice as usize));
    ventana.on_actualizar_datos_de_fila(|indice, origen| {
        con_controlador(|c| {
            let archivos = c.archivos_para_actualizar(indice as usize);
            actualizacion::actualizar_desde(c, archivos, origen);
        })
    });
    ventana.on_insertar_marcador(|tipo| {
        con_controlador(|c| {
            let marcador = if tipo == 1 { MarcadorDeBloque::Inicio } else { MarcadorDeBloque::Fin };
            if let Err(error) = c.insertar_marcador(marcador, None) {
                c.mostrar_estado(&error);
            }
        });
    });
    ventana.on_fijar_seleccion_como_bloque(|| {
        con_controlador(|c| {
            if let Err(error) = c.fijar_seleccion_como_bloque() {
                c.mostrar_estado(&error);
            }
        });
    });
    ventana.on_cambiar_base_hth(|indice, base| {
        con_controlador(|c| {
            let base = usize::try_from(base).ok();
            match c.cambiar_base_hth(indice as usize, base) {
                Ok(mensaje) | Err(mensaje) => c.mostrar_estado(&mensaje),
            }
        });
    });
    ventana.on_buscar(|texto| con_controlador(|c| c.buscar(&texto)));
    ventana.on_ir_a_coincidencia(|avanzar| con_controlador(|c| c.ir_a_coincidencia(avanzar)));
    ventana.on_clic_en_fila(|indice, control, mayusculas| {
        con_controlador(|c| c.clic_en_fila(indice as usize, control, mayusculas))
    });
    // Doble clic en un marcador: selecciona su bloque completo.
    ventana.on_doble_clic_en_fila(|indice| {
        con_controlador(|c| {
            if !c.seleccionar_bloque(indice as usize) {
                c.reproducir_desde_la_lista(indice as usize);
            }
        })
    });
    ventana.on_alternar_reproduccion(|| con_controlador(Controlador::alternar_reproduccion));
    ventana.on_detener(|| con_controlador(Controlador::detener_desde_la_ventana));
    ventana.on_anterior(|| con_controlador(Controlador::reproducir_anterior));
    ventana.on_siguiente(|| con_controlador(Controlador::reproducir_siguiente));
    ventana.on_repetir_cambiado(|repetir| con_controlador(|c| c.definir_repetir(repetir)));
    ventana.on_crossfade_inteligente_cambiado(|activo| con_controlador(|c| c.cambiar_crossfade_inteligente(activo)));
    ventana.on_nivelar_volumen_cambiado(|activo| con_controlador(|c| c.cambiar_nivelar_volumen(activo)));
    ventana.on_objetivo_lufs_cambiado(|indice| {
        if let Some(&objetivo) = ObjetivoDeSonoridad::TODOS.get(indice as usize) {
            con_controlador(|c| c.cambiar_objetivo_de_sonoridad(objetivo));
        }
    });
    ventana.on_desplazar_cambiado(|activo| con_controlador(|c| c.cambiar_desplazar(activo)));
    // Entrar al aire es inmediato; salir pide confirmacion (como al Detener).
    let debil = ventana.as_weak();
    ventana.on_al_aire_cambiado(move |al_aire| {
        if al_aire {
            con_controlador(|c| {
                c.cambiar_modo_de_operacion(ModoDeOperacion::AlAire);
            });
        } else {
            preguntar_si_salir_del_aire(&debil, false);
        }
    });
    ventana.on_volumen_cambiado(|volumen| con_controlador(|c| c.ajustar_volumen(volumen as f64)));
    ventana.on_volumen_por_defecto(|| con_controlador(|c| c.cambiar_volumen(VOLUMEN_POR_DEFECTO)));
    ventana.on_fader_cambiado(|segundos| con_controlador(|c| c.guardar_fader(segundos.round() as u8)));
    ventana.on_fader_por_defecto(|| con_controlador(|c| c.cambiar_fader(FADER_POR_DEFECTO)));
    ventana.on_progreso_movido(|segundos| {
        con_controlador(|c| {
            if c.admitida_o_avisar(OperacionSobreLaSecuencia::Adelantar) {
                c.busqueda_pendiente = Some((segundos as f64, Instant::now()));
            }
        })
    });
    ventana.on_mezclar_lista(|| {
        con_controlador(|c| {
            if let Err(motivo) = c.mezclar_seleccion_o_lista() {
                c.mostrar_estado(&motivo);
            }
        })
    });
    ventana.on_ordenar_por_bpm(|| {
        con_controlador(|c| {
            if let Err(motivo) = c.ordenar_seleccion_o_lista_por_bpm() {
                c.mostrar_estado(&motivo);
            }
        })
    });
    ventana.on_ciclar_formato_de_tiempo(|| {
        con_controlador(|c| {
            c.formato_de_tiempo = c.formato_de_tiempo.siguiente();
            c.refrescar_etiqueta_de_tiempo();
        })
    });

    let debil = ventana.as_weak();
    ventana.on_pulsar_efecto(move |indice| {
        let indice = indice as usize;
        let mut sin_archivo = false;
        con_controlador(|c| sin_archivo = c.pulsar_efecto(None, indice).is_err());
        if sin_archivo {
            asignar_efecto_con_dialogo(&debil, indice);
        }
    });
    ventana.on_tecla_de_efecto(|indice| {
        con_controlador(|c| {
            if let Err(motivo) = c.pulsar_efecto(None, indice as usize) {
                c.mostrar_estado(&motivo);
            }
        })
    });
    let debil = ventana.as_weak();
    ventana.on_asignar_efecto(move |indice| asignar_efecto_con_dialogo(&debil, indice as usize));
    ventana.on_quitar_efecto(|indice| {
        con_controlador(|c| {
            let hoja = c.repositorio_de_botonera.hoja_actual();
            c.quitar_efecto(hoja, indice as usize);
        })
    });
    ventana.on_hoja_anterior(|| con_controlador(|c| c.mover_hoja(-1)));
    ventana.on_hoja_siguiente(|| con_controlador(|c| c.mover_hoja(1)));
    ventana.on_agregar_hoja(|| {
        con_controlador(|c| {
            let _ = c.hojas_de_efectos(AccionDeHoja::Agregar);
        })
    });
    ventana.on_renombrar_hoja(|nombre| {
        con_controlador(|c| match c.hojas_de_efectos(AccionDeHoja::Renombrar(None, nombre.to_string())) {
            Ok(mensaje) | Err(mensaje) => c.mostrar_estado(&mensaje),
        })
    });
    let debil = ventana.as_weak();
    ventana.on_quitar_hoja(move || {
        let mut hoja_y_asignados = None;
        con_controlador(|c| {
            let hoja = c.repositorio_de_botonera.hoja_actual();
            hoja_y_asignados = Some((hoja, c.repositorio_de_botonera.asignados_en(hoja)));
        });
        let Some((hoja, asignados)) = hoja_y_asignados else {
            return;
        };
        if asignados > 0 && !confirmar_quitar_hoja(&debil, hoja, asignados) {
            return;
        }
        con_controlador(|c| {
            if let Err(error) = c.hojas_de_efectos(AccionDeHoja::Quitar(hoja)) {
                c.aviso_de_botonera = Some((error, Instant::now()));
            }
        });
    });
    ventana.on_insertar_efecto(|indice| con_controlador(|c| c.insertar_efecto_en_la_lista(indice as usize)));
    let para_hth = ventana.as_weak();
    ventana.on_insertar_hth(move || insertar_anuncio_hth_con_dialogo(&para_hth));
    let para_locuciones = ventana.as_weak();
    ventana.on_elegir_locuciones(move || {
        elegir_carpeta_de_locuciones_con_dialogo(&para_locuciones);
    });
    ventana.on_cambiar_localidad_del_clima(|| con_controlador(Controlador::abrir_selector_de_localidad));
    ventana.on_actualizar_clima(|| con_controlador(Controlador::actualizar_clima_ahora));
    ventana.on_editar_efecto(|indice| {
        con_controlador(|c| {
            if let Err(error) = c.abrir_editor_de_efecto(indice as usize) {
                c.mostrar_estado(&error);
            }
        })
    });
    ventana.on_nivel_de_musica_por_defecto(|| con_controlador(|c| c.nivel_de_musica_por_defecto()));
    ventana.on_ir_a_pista_actual(|| con_controlador(Controlador::ir_a_pista_actual));
    ventana.on_modo_libre_cambiado(|libre| con_controlador(|c| c.guardar_modo_libre(libre)));
    ventana.on_controles_alternados(|visibles| {
        con_controlador(|c| c.repositorio_de_preferencias.guardar_controles_visibles(visibles));
    });
    ventana.on_ajustes_de_efectos_alternados(|visibles| {
        con_controlador(|c| c.repositorio_de_preferencias.guardar_ajustes_de_efectos_visibles(visibles));
    });
    ventana.on_efecto_ajustado(|| {
        con_controlador(|c| {
            let Some(ventana) = c.ventana.upgrade() else {
                return;
            };
            c.cambiar_ajustes_de_efecto(AjustesDeEfecto {
                atenuar: ventana.get_atenuar(),
                inteligente: ventana.get_efecto_inteligente(),
                nivel_de_musica: f64::from(ventana.get_nivel_de_musica()),
                volumen_db: (f64::from(ventana.get_volumen_de_efecto()) * 2.0).round() / 2.0,
                anticipacion: (f64::from(ventana.get_anticipacion()) * 100.0).round() / 100.0,
                recuperacion: (f64::from(ventana.get_recuperacion()) * 10.0).round() / 10.0,
                separacion_db: f64::from(ventana.get_separacion()).round(),
            });
        })
    });
    ventana.on_detener_efecto(|| con_controlador(Controlador::detener_efecto));
    Ok(())
}


/// Correo de contacto del autor, en la ventana Acerca de. Es un reenvio
/// (relay): no expone la casilla personal.
const CORREO_DE_CONTACTO: &str = "jxlfybcrk@mozmail.com";

/// Repositorio del proyecto en GitHub (el campo `repository` de Cargo.toml).
const REPOSITORIO: &str = env!("CARGO_PKG_REPOSITORY");

/// Pagina de donaciones del autor en PayPal (boton Acerca de > Donar).
const DONACIONES: &str = "https://www.paypal.com/donate/?hosted_button_id=SYCZZFULZ6TKE";

/// Archivo del manual en ese idioma: Raptor_Manual_es.pdf, Raptor_Manual_en.pdf.
fn nombre_del_manual(idioma: Idioma) -> String {
    format!("Raptor_Manual_{}.pdf", idioma.codigo())
}

/// Manuales a buscar, en orden: primero el del idioma de la interfaz y
/// despues los demas, para abrir alguno aunque falte el preferido.
fn manuales_en_orden(idioma: Idioma) -> Vec<String> {
    std::iter::once(idioma)
        .chain(Idioma::TODOS.into_iter().filter(|otro| *otro != idioma))
        .map(nombre_del_manual)
        .collect()
}

/// Lugares donde se busca un manual: junto al ejecutable, en los recursos de
/// la app de macOS (el ejecutable esta en Contents/MacOS y los PDF en
/// Contents/Resources), en la carpeta de trabajo y en la carpeta del proyecto
/// o su docs/manual (versiones de desarrollo).
fn ubicaciones_del_manual(nombre: &str) -> Vec<PathBuf> {
    let mut ubicaciones = Vec::new();
    if let Some(carpeta) = std::env::current_exe().ok().and_then(|ejecutable| ejecutable.parent().map(Path::to_path_buf)) {
        ubicaciones.push(carpeta.join(nombre));
        if carpeta.ends_with("Contents/MacOS") {
            ubicaciones.push(carpeta.with_file_name("Resources").join(nombre));
        }
    }
    if let Ok(carpeta) = std::env::current_dir() {
        ubicaciones.push(carpeta.join(nombre));
    }
    let proyecto = Path::new(env!("CARGO_MANIFEST_DIR"));
    ubicaciones.push(proyecto.join(nombre));
    ubicaciones.push(proyecto.join("docs").join("manual").join(nombre));
    ubicaciones
}

/// Abre el manual en PDF del idioma de la interfaz (o, si falta, el de otro
/// idioma) con el visor del sistema, o avisa donde ponerlo.
fn abrir_manual(ventana: &slint::Weak<VentanaPrincipal>) {
    let encontrado = manuales_en_orden(crate::idioma::actual())
        .iter()
        .flat_map(|nombre| ubicaciones_del_manual(nombre))
        .find(|ubicacion| ubicacion.is_file());
    let Some(manual) = encontrado.as_deref() else {
        let preferido = nombre_del_manual(crate::idioma::actual());
        let buscado: Vec<String> = ubicaciones_del_manual(&preferido).iter().map(|ubicacion| format!("- {}", ubicacion.display())).collect();
        let mut dialogo = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title("Manual")
            .set_description(tr!("No se encontro {MANUAL}. Copielo junto al programa. Se busco en:\n{}", buscado.join("\n"), MANUAL = preferido))
            .set_buttons(rfd::MessageButtons::Ok);
        if let Some(ventana) = ventana.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        dialogo.show();
        return;
    };
    if let Err(error) = abrir_con_el_sistema(manual) {
        mostrar_error(&tr!("No se pudo abrir {}: {error}", manual.display(), error = error));
    }
}

/// Abre un archivo con el programa que el sistema tiene asociado.
fn abrir_con_el_sistema(archivo: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut comando = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut comando = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut comando = std::process::Command::new("xdg-open");
    comando.arg(archivo).spawn().map(|_| ())
}

/// Abre una direccion (https:, mailto:) con el programa predeterminado. En
/// Windows no se usa explorer: interpreta los "=" y "," de la linea de
/// comandos y no abre direcciones con parametros (como la de PayPal).
fn abrir_enlace(direccion: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", direccion]).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        abrir_con_el_sistema(Path::new(direccion))
    }
}

/// Pide confirmacion antes de quitar una hoja que tiene efectos asignados.
fn confirmar_quitar_hoja(ventana: &slint::Weak<VentanaPrincipal>, hoja: usize, asignados: usize) -> bool {
    let mut dialogo = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(&tr!("Quitar hoja de efectos"))
        .set_description(tr!("La hoja {} tiene {asignados} efecto(s) asignado(s). ¿Quitarla? Los archivos no se borran.", hoja + 1, asignados = asignados))
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    dialogo.show() == rfd::MessageDialogResult::Yes
}

fn asignar_efecto_con_dialogo(ventana: &slint::Weak<VentanaPrincipal>, indice: usize) {
    let titulo = tr!("Archivo de sonido para el boton {indice}", indice = indice);
    let Some(archivo) = elegir_audios(ventana, &titulo, "Audio").pick_file() else {
        return;
    };
    let mut rechazo = None;
    con_controlador(|c| {
        let hoja = c.repositorio_de_botonera.hoja_actual();
        rechazo = c.asignar_efecto(hoja, indice, &archivo).err();
    });
    if let Some(motivo) = rechazo {
        let mut dialogo = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&tr!("Efecto repetido"))
            .set_description(tr!("{motivo}\nCada efecto puede estar en un solo boton de la hoja.", motivo = motivo))
            .set_buttons(rfd::MessageButtons::Ok);
        if let Some(ventana) = ventana.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        dialogo.show();
    }
}

/// Dialogo de listas: arranca en la carpeta de listas y acepta las del
/// reproductor y las M3U.
fn dialogo_de_listas(ventana: &slint::Weak<VentanaPrincipal>, titulo: &str) -> rfd::FileDialog {
    let carpeta = carpeta_de_listas();
    let _ = std::fs::create_dir_all(&carpeta);
    let mut dialogo = rfd::FileDialog::new()
        .set_title(titulo)
        .set_directory(&carpeta)
        .add_filter(&tr!("Lista de MCP Raptor Player"), &["json"])
        .add_filter(&tr!("Lista M3U"), &["m3u8", "m3u"]);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    dialogo
}

fn avisar_error(ventana: &slint::Weak<VentanaPrincipal>, titulo: &str, mensaje: &str) {
    let mut dialogo = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(titulo)
        .set_description(mensaje)
        .set_buttons(rfd::MessageButtons::Ok);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    dialogo.show();
}

/// Guarda la lista con su nombre o, si no tiene (o `como`), pide uno.
/// Devuelve si quedo guardada.
fn guardar_lista_con_dialogo(ventana: &slint::Weak<VentanaPrincipal>, como: bool) -> bool {
    let mut destino = None;
    con_controlador(|c| destino = c.archivo_de_lista.clone().filter(|_| !como));
    let destino = match destino {
        Some(destino) => destino,
        None => {
            let mut nombre = tr!("Nueva lista.json");
            con_controlador(|c| {
                if let Some(actual) = &c.archivo_de_lista {
                    nombre = nombre_de_archivo(actual);
                }
            });
            match dialogo_de_listas(ventana, &tr!("Guardar lista como")).set_file_name(&nombre).save_file() {
                Some(destino) if destino.extension().is_none() => destino.with_extension("json"),
                Some(destino) => destino,
                None => return false,
            }
        }
    };
    let mut resultado = Ok(String::new());
    con_controlador(|c| resultado = c.guardar_lista_en(&destino));
    match resultado {
        Ok(_) => true,
        Err(error) => {
            avisar_error(ventana, &tr!("Guardar lista"), &error);
            false
        }
    }
}

/// Carga una lista; si la actual tiene cambios sin guardar, primero pregunta.
fn cargar_lista_con_dialogo(ventana: &slint::Weak<VentanaPrincipal>) {
    let mut admitida = false;
    con_controlador(|c| admitida = c.admitida_o_avisar(OperacionSobreLaSecuencia::CargarLista));
    if !admitida {
        return;
    }
    let mut modificada = false;
    con_controlador(|c| modificada = c.lista_modificada() && !c.lista.esta_vacia());
    if modificada {
        let mut dialogo = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&tr!("Cargar lista"))
            .set_description(&tr!("La lista actual tiene cambios sin guardar. ¿Guardarla antes de cargar otra?"))
            .set_buttons(rfd::MessageButtons::YesNoCancel);
        if let Some(ventana) = ventana.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        match dialogo.show() {
            rfd::MessageDialogResult::Yes => {
                if !guardar_lista_con_dialogo(ventana, false) {
                    return;
                }
            }
            rfd::MessageDialogResult::No => {}
            _ => return,
        }
    }
    let Some(archivo) = dialogo_de_listas(ventana, &tr!("Cargar lista")).pick_file() else {
        return;
    };
    let mut resultado = Ok(String::new());
    con_controlador(|c| resultado = c.cargar_lista_desde(&archivo));
    if let Err(error) = resultado {
        avisar_error(ventana, &tr!("Cargar lista"), &error);
    }
}

/// Pregunta si salir del modo AL AIRE y pasar a EDITOR: despues de Detener
/// al aire (`detenida`) o al pulsar el boton rojo AL AIRE. Si no, sigue al
/// aire.
fn preguntar_si_salir_del_aire(ventana: &slint::Weak<VentanaPrincipal>, detenida: bool) {
    let (titulo, pregunta) = if detenida {
        (
            tr!("Reproduccion detenida"),
            tr!("La reproducción se detuvo en modo AL AIRE. ¿Salir del modo AL AIRE y pasar a modo EDITOR?\n\nSí: todos los comandos quedan habilitados.\nNo: sigue AL AIRE para cuando se vuelva a reproducir."),
        )
    } else {
        (
            tr!("Modo AL AIRE"),
            tr!("¿Salir del modo AL AIRE y pasar a modo EDITOR?\n\nSí: todos los comandos quedan habilitados.\nNo: sigue AL AIRE."),
        )
    };
    let mut dialogo = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(&titulo)
        .set_description(pregunta)
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    let salir = dialogo.show() == rfd::MessageDialogResult::Yes;
    con_controlador(|c| {
        // Si mientras tanto volvio a sonar o ya se cambio el modo, no se toca.
        if !c.modo_de_operacion().al_aire() {
            return;
        }
        if salir {
            c.cambiar_modo_de_operacion(ModoDeOperacion::Editor);
        } else {
            c.mostrar_estado(&tr!("Sigue en modo AL AIRE"));
        }
    });
}

/// Vacia la lista despues de confirmar: no se puede deshacer.
fn limpiar_con_confirmacion(ventana: &slint::Weak<VentanaPrincipal>) {
    let mut cuantas = None;
    con_controlador(|c| {
        if c.lista.esta_vacia() {
            c.mostrar_estado(&tr!("La lista esta vacia"));
        } else if c.admitida_o_avisar(OperacionSobreLaSecuencia::Limpiar) {
            cuantas = Some(c.lista.tamano());
        }
    });
    let Some(cuantas) = cuantas else { return };
    let mut dialogo = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(&tr!("Limpiar la lista"))
        .set_description(tr!("¿Quitar las {} pistas de la lista y detener la reproducción? No se puede deshacer (los archivos y sus datos guardados no se borran).", cuantas))
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    if dialogo.show() == rfd::MessageDialogResult::Yes {
        con_controlador(Controlador::limpiar_lista);
    }
}

/// Quita las pistas de esas posiciones; si son mas de una, pide confirmacion.
/// La que suena no se quita y, si hay marcadores de bloque entre temas, la
/// primera vez solo se quitan los marcadores.
fn quitar_con_confirmacion(ventana: &slint::Weak<VentanaPrincipal>, posiciones: Vec<usize>) {
    let mut por_quitar = (Vec::new(), false);
    con_controlador(|c| por_quitar = c.preparar_quitar(&posiciones));
    let (posiciones, quedo_la_que_suena) = por_quitar;
    if posiciones.is_empty() {
        return;
    }
    let mut admitida = false;
    con_controlador(|c| admitida = c.admitida_o_avisar(OperacionSobreLaSecuencia::Quitar(posiciones.len())));
    if !admitida {
        return;
    }
    if posiciones.len() > 1 {
        let mut dialogo = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&tr!("Quitar de la lista"))
            .set_description(tr!("¿Quitar {} pistas de la lista? Los archivos y sus datos guardados no se borran.", posiciones.len()))
            .set_buttons(rfd::MessageButtons::YesNo);
        if let Some(ventana) = ventana.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        if dialogo.show() != rfd::MessageDialogResult::Yes {
            return;
        }
    }
    con_controlador(|c| {
        c.quitar_posiciones(&posiciones);
        if quedo_la_que_suena {
            c.mostrar_estado(&tr!("Se quitaron {} pista(s); la que suena quedo en la lista ({} pista(s))", posiciones.len(), c.lista.tamano()));
        }
    });
}

/// Pregunta si descatalogar la pista de esa fila y, si se confirma, la quita
/// de la lista y borra sus datos guardados.
fn descatalogar_con_dialogo(ventana: &slint::Weak<VentanaPrincipal>, indice: usize) {
    let mut elegida = None;
    con_controlador(|c| {
        elegida = c.lista.pista_en(indice).cloned().map(|pista| {
            let tema = c.titulo_destacado(&pista);
            (pista, tema)
        })
    });
    let Some((pista, tema)) = elegida else { return };
    let mut motivo = None;
    con_controlador(|c| motivo = c.motivo_para_no_descatalogar(&pista));
    if let Some(motivo) = motivo {
        con_controlador(|c| c.mostrar_estado(&motivo));
        return;
    }
    let mut dialogo = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Descatalogar")
        .set_description(format!(
            "{tema}\n{}\n\nSe quitara de la lista y se borraran sus datos guardados: puntos de mezcla, \
             overlay, titulo, artista, genero, ranking, BPM y ultima reproduccion. El archivo de audio no se borra.\n\n\
             ¿Descatalogar este tema?",
            pista.archivo().display()
        ))
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    if dialogo.show() == rfd::MessageDialogResult::Yes {
        con_controlador(|c| c.descatalogar(&pista));
    }
}

/// Valida la lista: ofrece quitar los archivos que ya no existen y despues
/// pregunta, uno por uno, si quitar cada tema repetido.
fn validar_lista_con_dialogos(ventana: &slint::Weak<VentanaPrincipal>) {
    let mut faltantes = Vec::new();
    con_controlador(|c| faltantes = c.pistas_faltantes());
    let mut quitadas_por_faltar = 0;
    if !faltantes.is_empty() && confirmar_quitar_faltantes(ventana, &faltantes) {
        con_controlador(|c| quitadas_por_faltar = c.quitar_pistas(&faltantes));
    }
    let mut duplicados = Vec::new();
    con_controlador(|c| duplicados = c.temas_repetidos(&HashSet::new(), false, &HashMap::new()));
    let revision = preguntar_por_duplicados(ventana, &duplicados, false);
    let archivos = match faltantes.len() {
        0 => tr!("todos los archivos existen"),
        cantidad => tr!("{cantidad} archivo(s) inexistente(s), {quitadas_por_faltar} quitado(s)", cantidad = cantidad, quitadas_por_faltar = quitadas_por_faltar),
    };
    let repetidos = match revision.encontrados {
        0 => tr!("sin temas repetidos"),
        cantidad => tr!("{cantidad} tema(s) repetido(s), {} quitado(s)", revision.quitados, cantidad = cantidad),
    };
    let cancelada = if revision.cancelada { tr!(" (revision cancelada)") } else { String::new() };
    let mensaje = tr!("Lista validada: {archivos}; {repetidos}{cancelada}", archivos = archivos, repetidos = repetidos, cancelada = cancelada);
    con_controlador(|c| c.mostrar_estado(&mensaje));
}

/// Lee en otro hilo las etiquetas de las pistas recien agregadas y despues
/// pregunta, una por una, si quitar las que repiten un tema de la lista.
/// Agregar una carpeta con sus subcarpetas: se elige la carpeta, se recorre
/// en otro hilo (puede ser grande o estar en la red) y se pide confirmacion
/// con lo que se encontro antes de agregar.
fn agregar_carpeta_con_dialogo(ventana: &slint::Weak<VentanaPrincipal>) {
    let mut dialogo = rfd::FileDialog::new().set_title(&tr!("Carpeta para agregar a la lista (con sus subcarpetas)"));
    if let Some(principal) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&principal.window().window_handle());
    }
    let Some(carpeta) = dialogo.pick_folder() else {
        return;
    };
    con_controlador(|c| c.mostrar_estado(&tr!("Buscando archivos de audio en {} y sus subcarpetas...", carpeta.display())));
    let ventana = ventana.clone();
    let lanzado = std::thread::Builder::new().name("recorrido-de-carpeta".into()).spawn(move || {
        fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Fondo);
        let encontrados = audios_de_carpeta(&carpeta, true);
        let _ = slint::invoke_from_event_loop(move || confirmar_y_agregar_carpeta(&ventana, &carpeta, encontrados));
    });
    if let Err(error) = lanzado {
        mostrar_error(&tr!("No se pudo recorrer la carpeta: {error}", error = error));
    }
}

/// Muestra lo que se encontro en la carpeta y, si se confirma, lo agrega.
fn confirmar_y_agregar_carpeta(
    ventana: &slint::Weak<VentanaPrincipal>,
    carpeta: &Path,
    encontrados: std::io::Result<AudiosDeCarpeta>,
) {
    let encontrados = match encontrados {
        Ok(encontrados) => encontrados,
        Err(error) => {
            con_controlador(|c| c.mostrar_estado(&tr!("No se pudo leer la carpeta")));
            mostrar_error(&tr!("No se pudo leer la carpeta {}: {error}", carpeta.display(), error = error));
            return;
        }
    };
    let subcarpetas = encontrados.carpetas.saturating_sub(1);
    let ilegibles = match encontrados.ilegibles {
        0 => String::new(),
        cantidad => tr!("\n\n{cantidad} subcarpeta(s) no se pudieron leer y se saltearon.", cantidad = cantidad),
    };
    let mut dialogo = rfd::MessageDialog::new().set_title(&tr!("Agregar carpeta"));
    if let Some(principal) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&principal.window().window_handle());
    }
    if encontrados.archivos.is_empty() {
        con_controlador(|c| c.mostrar_estado(&tr!("No hay archivos de audio en {}", carpeta.display())));
        dialogo
            .set_level(rfd::MessageLevel::Info)
            .set_description(tr!("No se encontraron archivos de audio en {} ni en sus {subcarpetas} subcarpeta(s).{ilegibles}", carpeta.display(), subcarpetas = subcarpetas, ilegibles = ilegibles))
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        return;
    }
    let confirmado = dialogo
        .set_level(rfd::MessageLevel::Info)
        .set_description(format!(
            "Se encontraron {} archivo(s) de audio en {} y sus {subcarpetas} subcarpeta(s).{ilegibles}\n\n\
             Se insertan despues de la pista seleccionada (o al final de la lista), carpeta por carpeta y por \
             nombre. Los que ya estan en la lista se omiten y los de tipo Efecto quedan afuera.\n\n¿Agregarlos?",
            encontrados.archivos.len(),
            carpeta.display()
        ))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes;
    if !confirmado {
        con_controlador(|c| c.mostrar_estado(&tr!("No se agrego la carpeta")));
        return;
    }
    let origen = format!(" de {}", carpeta.display());
    let mut nuevas = Vec::new();
    con_controlador(|c| nuevas = c.agregar_y_resumir(encontrados.archivos, &origen));
    revisar_duplicados_agregados(ventana.clone(), nuevas);
}

fn revisar_duplicados_agregados(ventana: slint::Weak<VentanaPrincipal>, nuevas: Vec<Pista>) {
    if nuevas.is_empty() {
        return;
    }
    let lanzado = std::thread::Builder::new().name("revision-de-duplicados".into()).spawn(move || {
        fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Fondo);
        let etiquetas: HashMap<PathBuf, Analisis> = nuevas
            .iter()
            .filter_map(|pista| Some((pista.archivo().to_path_buf(), Decodificador::etiquetas(pista.archivo()).ok()?)))
            .collect();
        let _ = slint::invoke_from_event_loop(move || {
            let nuevas: HashSet<Pista> = nuevas.into_iter().collect();
            let mut duplicados = Vec::new();
            con_controlador(|c| duplicados = c.temas_repetidos(&nuevas, true, &etiquetas));
            let revision = preguntar_por_duplicados(&ventana, &duplicados, true);
            if revision.encontrados > 0 {
                let mensaje = tr!("{} tema(s) agregado(s) ya estaban en la lista; {} quitado(s)", revision.encontrados, revision.quitados);
                con_controlador(|c| c.mostrar_estado(&mensaje));
            }
        });
    });
    if let Err(error) = lanzado {
        log::warn!("No se pudo iniciar la revision de duplicados: {error}");
    }
}

/// Resultado de preguntar por los temas repetidos.
struct RevisionDeDuplicados {
    encontrados: usize,
    quitados: usize,
    cancelada: bool,
}

/// Pregunta por cada tema repetido si quitar la copia; Cancelar termina.
fn preguntar_por_duplicados(
    ventana: &slint::Weak<VentanaPrincipal>,
    duplicados: &[Duplicado],
    al_agregar: bool,
) -> RevisionDeDuplicados {
    let mut revision = RevisionDeDuplicados { encontrados: 0, quitados: 0, cancelada: false };
    for duplicado in duplicados {
        let mut descripcion = None;
        con_controlador(|c| descripcion = c.describir_duplicado(duplicado, al_agregar));
        // Una de las dos ya se quito al responder por otra copia.
        let Some(descripcion) = descripcion else { continue };
        revision.encontrados += 1;
        let mut dialogo = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&tr!("Tema repetido"))
            .set_description(descripcion)
            .set_buttons(rfd::MessageButtons::YesNoCancel);
        if let Some(ventana) = ventana.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        match dialogo.show() {
            rfd::MessageDialogResult::Yes => {
                let mut quitadas = 0;
                con_controlador(|c| quitadas = c.quitar_pistas(std::slice::from_ref(&duplicado.quitar)));
                revision.quitados += quitadas;
            }
            rfd::MessageDialogResult::No => {}
            _ => {
                revision.cancelada = true;
                break;
            }
        }
    }
    revision
}

/// Pregunta si quitar de la lista los archivos que ya no existen.
fn confirmar_quitar_faltantes(ventana: &slint::Weak<VentanaPrincipal>, faltantes: &[Pista]) -> bool {
    const MOSTRADOS: usize = 15;
    let mut detalle: Vec<String> =
        faltantes.iter().take(MOSTRADOS).map(|pista| pista.archivo().display().to_string()).collect();
    if faltantes.len() > MOSTRADOS {
        detalle.push(tr!("... y {} mas", faltantes.len() - MOSTRADOS));
    }
    let mut dialogo = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(&tr!("Archivos inexistentes"))
        .set_description(tr!("{} archivo(s) de la lista ya no existe(n):\n\n{}\n\n¿Quitarlos de la lista?", faltantes.len(), detalle.join("\n")))
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(ventana) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&ventana.window().window_handle());
    }
    dialogo.show() == rfd::MessageDialogResult::Yes
}

/// Pistas que cambian de orden al mezclar u ordenar: toda la lista o el
/// bloque seleccionado (cuyas pistas solo se reacomodan entre ellas).
struct Reordenamiento {
    posiciones: Vec<usize>,
    es_un_bloque: bool,
}

impl Reordenamiento {
    fn de_toda_la_lista(tamano: usize) -> Self {
        Self { posiciones: (0..tamano).collect(), es_un_bloque: false }
    }
}

/// Tramos que se marcan en el deslizante de progreso, en segundos desde el
/// comienzo del archivo; (-1, -1) si no hay.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MarcasDeProgreso {
    overlay: (f32, f32),
    outro: (f32, f32),
    overlay_manual: bool,
    outro_manual: bool,
    /// Superposicion real con la siguiente.
    mezcla: (f32, f32),
}

impl MarcasDeProgreso {
    const SIN_TRAMO: (f32, f32) = (-1.0, -1.0);
    const NINGUNA: Self =
        Self { overlay: Self::SIN_TRAMO, outro: Self::SIN_TRAMO, overlay_manual: false, outro_manual: false, mezcla: Self::SIN_TRAMO };

    /// Tramo de la pista en que se superpone con la siguiente: termina en su
    /// ultimo sonido y dura lo que la mezcla, corrido por la alineacion de
    /// los golpes (como lo programa el gestor de reproduccion).
    fn tramo_de_mezcla(transicion: &Transicion, duracion: f64) -> (f32, f32) {
        let fin = (duracion - transicion.recorte_final).clamp(0.0, duracion);
        let inicio = (fin - transicion.segundos + transicion.desfase).clamp(0.0, fin);
        if fin > inicio { (inicio as f32, fin as f32) } else { Self::SIN_TRAMO }
    }

    /// La outro va desde que el tema empieza a apagarse hasta el recorte final.
    fn de(puntos: &PuntosDeMezcla, duracion: f64) -> Self {
        let tramo = |inicio: f64, fin: f64| {
            let (inicio, fin) = (inicio.clamp(0.0, duracion), fin.clamp(0.0, duracion));
            if fin > inicio { (inicio as f32, fin as f32) } else { Self::SIN_TRAMO }
        };
        let overlay = puntos.overlay().map_or(Self::SIN_TRAMO, |ventana| tramo(ventana.inicio(), ventana.fin()));
        let outro = if puntos.outro_audible() > 0.0 {
            tramo(duracion - puntos.outro(), duracion - puntos.recorte_final())
        } else {
            Self::SIN_TRAMO
        };
        Self { overlay, outro, overlay_manual: puntos.overlay_manual(), outro_manual: puntos.manuales(), mezcla: Self::SIN_TRAMO }
    }
}

fn formatear_segundos(segundos: f64) -> String {
    let texto = format!("{segundos:.2}");
    texto.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Minutos, segundos y decimas: "4:30.6". Las decimas se truncan, para no
/// mostrar un segundo que todavia no paso.
pub fn formatear_minutos(segundos: f64) -> String {
    let decimas = (segundos.max(0.0) * 10.0).floor() as u64;
    format!("{}:{:02}.{}", decimas / 600, decimas / 10 % 60, decimas % 10)
}

/// Si la pista se puede reproducir: su archivo existe y no es un marcador de
/// bloque fijo. Las que faltan (movidas o borradas) y los marcadores se
/// saltean al avanzar, para no dejar la emision en silencio.
fn archivo_existe(pista: &Pista) -> bool {
    pista.archivo().is_file() && !es_marcador(pista.archivo())
}

/// Mezcla a aplicar al final de la pista que suena: hacia la siguiente de la
/// lista cuyo archivo exista.
fn transicion_desde(
    lista: &ListaDeReproduccion,
    datos: &HashMap<PathBuf, DatosDeMezcla>,
    canciones: &RepositorioDeCanciones,
    inteligente: bool,
    actual: &Pista,
) -> Option<Transicion> {
    let indice = lista.indice_actual().filter(|&indice| lista.pista_en(indice) == Some(actual))?;
    transicion_tras_indice(lista, datos, canciones, inteligente, indice, &archivo_existe)
}

/// Si el tipo del tema pide que suene sin mezcla (todos menos el Tema:
/// tandas comerciales, efectos y secos). El anuncio HTH tampoco se mezcla.
fn suena_sin_mezcla(canciones: &RepositorioDeCanciones, pista: &Pista) -> bool {
    es_anuncio_hth(pista.archivo()) || canciones.tipo_de(pista.archivo()).sin_mezcla()
}

/// Nombre de una hoja de la botonera (desde 0) cuando no se le puso otro.
fn nombre_de_siempre_de_la_hoja(hoja: usize) -> String {
    tr!("Hoja {}", hoja + 1)
}

/// Si sobre la pista puede salir un efecto de la botonera en modo trabado:
/// no sobre los tipos sin mezcla (Comercial, Publicidad, Anuncio...) ni sobre
/// una base del anuncio HTH, cuya ventana es para la voz del anuncio.
fn admite_efectos_de_la_botonera(canciones: &RepositorioDeCanciones, carpeta_de_bases: Option<&Path>, pista: &Pista) -> bool {
    !suena_sin_mezcla(canciones, pista) && !es_base_hth_en(pista.archivo(), carpeta_de_bases)
}

/// Si el archivo es propaganda facturable (tipo Comercial o Publicidad): sus
/// salidas al aire se anotan en el registro de comerciales.
fn es_facturable(canciones: &RepositorioDeCanciones, archivo: &Path) -> bool {
    canciones.tipo_de(archivo).facturable()
}

/// Si el archivo es de tipo Efecto: solo puede estar en la botonera.
fn es_solo_para_la_botonera(canciones: &RepositorioDeCanciones, archivo: &Path) -> bool {
    canciones.tipo_de(archivo).solo_para_la_botonera()
}

/// Tramo que suena de un efecto: el inicio y el fin fijados a mano en el
/// editor; sin ellos, el archivo entero (los recortes automaticos del
/// analisis no se aplican a los efectos).
fn recorte_de_efecto(canciones: &RepositorioDeCanciones, archivo: &Path) -> RecorteDeEfecto {
    let Some(ficha) = canciones.buscar(archivo).filter(|ficha| ficha.puntos.manuales()) else {
        return RecorteDeEfecto::default();
    };
    RecorteDeEfecto { desde: ficha.puntos.recorte_inicial(), antes_del_final: ficha.puntos.recorte_final() }
}

/// Mezcla entre la pista de la posicion indicada y la que le sigue. Si una de
/// las dos es comercial, anuncio o publicidad, no hay mezcla: la siguiente
/// arranca cuando termina la anterior. Con el crossfade inteligente y el
/// analisis de las dos, la decide el analisis; si no, se superponen los
/// segundos que fija la regla de mezcla.
fn transicion_tras_indice(
    lista: &ListaDeReproduccion,
    datos: &HashMap<PathBuf, DatosDeMezcla>,
    canciones: &RepositorioDeCanciones,
    inteligente: bool,
    indice: usize,
    existe: &dyn Fn(&Pista) -> bool,
) -> Option<Transicion> {
    let saliente = lista.pista_en(indice).filter(|pista| !es_marcador(pista.archivo()))?;
    // La entrante es la proxima que se puede reproducir: las que faltan se saltean.
    let entrante = lista.proxima_disponible(Some(indice), existe).0.and_then(|siguiente| lista.pista_en(siguiente))?;
    let (puntos_a, puntos_b) = (lista.puntos_de(saliente), lista.puntos_de(entrante));
    if suena_sin_mezcla(canciones, saliente) || suena_sin_mezcla(canciones, entrante) {
        return Some(Transicion { inicio: puntos_b.recorte_inicial(), ..Transicion::simple(entrante.clone(), 0.0) });
    }
    if inteligente {
        if let (Some(a), Some(b)) = (datos.get(saliente.archivo()), datos.get(entrante.archivo())) {
            return Some(transicion_inteligente(entrante.clone(), (&puntos_a, a), (&puntos_b, b)));
        }
    }
    Some(Transicion {
        inicio: puntos_b.recorte_inicial(),
        recorte_final: puntos_a.recorte_final(),
        outro_saliente: puntos_a.outro(),
        ..Transicion::simple(entrante.clone(), PuntosDeMezcla::segundos_de_mezcla(&puntos_a, &puntos_b))
    })
}

/// Segundos que una mezcla le quita a la duracion de la lista: la
/// superposicion, corrida por la alineacion de los golpes. Los recortes de
/// cada tema se descuentan aparte.
fn segundos_ahorrados(transicion: &Transicion) -> f64 {
    (transicion.segundos - transicion.desfase).max(0.0)
}

/// Fecha ISO 8601 (UTC) en la hora local: "22/10/2026 16:40".
#[cfg(test)]
fn fecha_local(momento: &str) -> String {
    momento
        .parse::<jiff::Timestamp>()
        .map(|instante| instante.to_zoned(jiff::tz::TimeZone::system()).strftime("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_else(|_| momento.to_string())
}

/// Tiempo transcurrido en palabras: "Hace 3 horas 20 minutos y 15 segundos".
/// Las unidades en cero se omiten y las que valen 1 van en singular.
fn hace(pasado: &jiff::Zoned, ahora: &jiff::Zoned) -> String {
    let Ok(lapso) = ahora.since((jiff::Unit::Year, pasado)) else {
        return String::new();
    };
    let unidades = [
        (i64::from(lapso.get_years()), "año", "años"),
        (i64::from(lapso.get_months()), "mes", "meses"),
        (i64::from(lapso.get_days()), "día", "días"),
        (i64::from(lapso.get_hours()), "hora", "horas"),
        (lapso.get_minutes(), "minuto", "minutos"),
        (lapso.get_seconds(), "segundo", "segundos"),
    ];
    let partes: Vec<String> = unidades
        .iter()
        .filter(|(valor, _, _)| *valor > 0)
        .map(|(valor, singular, plural)| format!("{valor} {}", if *valor == 1 { singular } else { plural }))
        .collect();
    match partes.as_slice() {
        [] => "Recien".into(),
        [unica] => format!("Hace {unica}"),
        [anteriores @ .., ultima] => tr!("Hace {} y {ultima}", anteriores.join(" "), ultima = ultima),
    }
}

/// Formato de la columna Sono (lista y catalogo), con el año corto y en el
/// orden elegido: "23/10/26 12:34:56".
fn formato_de_la_columna_sono() -> String {
    format!("{} %H:%M:%S", crate::formato_de_fecha::actual().corta())
}

/// Formato completo de la ultima reproduccion (panel de detalle y editor):
/// "23/10/2026 12:34:56".
pub(super) fn formato_de_la_reproduccion() -> String {
    format!("{} %H:%M:%S", crate::formato_de_fecha::actual().completa())
}

/// Un momento guardado (ISO 8601 UTC) con ese formato en esa zona; vacio si
/// no se entiende.
pub(super) fn texto_de_reproduccion(momento: &str, zona: &jiff::tz::TimeZone, formato: &str) -> String {
    momento.parse::<jiff::Timestamp>().map(|instante| instante.to_zoned(zona.clone()).strftime(formato).to_string()).unwrap_or_default()
}

/// Lee la ultima reproduccion escrita a mano en el formato elegido
/// ("23/10/2026 12:34:56", o sin los segundos) en la hora de la localidad y la devuelve como se guarda
/// (ISO 8601 UTC). Vacio: nunca sono. No admite fechas futuras.
pub(super) fn leer_reproduccion(texto: &str, zona: &jiff::tz::TimeZone, ahora: jiff::Timestamp) -> Result<Option<String>, String> {
    let texto = texto.trim();
    if texto.is_empty() {
        return Ok(None);
    }
    let formato = crate::formato_de_fecha::actual();
    let civil = jiff::civil::DateTime::strptime(formato_de_la_reproduccion(), texto)
        .or_else(|_| jiff::civil::DateTime::strptime(format!("{} %H:%M", formato.completa()), texto))
        .map_err(|_| tr!("Sonó debe tener la forma {} hh:mm:ss (o quedar vacío).", formato.para_escribir()))?;
    let instante = civil.to_zoned(zona.clone()).map_err(|error| error.to_string())?.timestamp();
    if instante > ahora {
        return Err(tr!("Sonó no puede ser una fecha futura."));
    }
    Ok(Some(instante.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()))
}

/// Lo que muestra la columna Sono de una fila: el texto, el globo (cuanto
/// hace o cuanto falta) y si es una estimacion (va en cursiva).
#[derive(Default)]
struct HorarioDeInicio {
    texto: String,
    ayuda: String,
    estimado: bool,
}

/// Fecha y hora de la ultima reproduccion para la columna de la lista;
/// vacio si no se entiende.
pub(super) fn reproduccion_para_la_lista(momento: &str, zona: &jiff::tz::TimeZone) -> String {
    texto_de_reproduccion(momento, zona, &formato_de_la_columna_sono())
}

/// Fecha de la ultima reproduccion en hora local ("23/10/2026 12:34:56") y el
/// tiempo transcurrido desde entonces, para el tooltip.
fn ultima_reproduccion(momento: &str, zona: &jiff::tz::TimeZone) -> (String, String) {
    let Ok(instante) = momento.parse::<jiff::Timestamp>() else {
        return (momento.to_string(), String::new());
    };
    let zona = zona.clone();
    let pasado = instante.to_zoned(zona.clone());
    let ahora = jiff::Zoned::now().with_time_zone(zona);
    (pasado.strftime(&formato_de_la_reproduccion()).to_string(), hace(&pasado, &ahora))
}

/// Fecha ("Lunes 02/11/2026") y hora ("14:05:33") locales, para el reloj.
/// Tipo (clave fija: Tema, Comercial... HTH, Bloque fijo), numero (vacio si
/// no esta en la lista), titulo y artista (vacio si no tiene) de una pista,
/// para el renglon grande.
#[derive(Default)]
struct RenglonDestacado {
    tipo: String,
    numero: String,
    titulo: String,
    artista: String,
}

impl RenglonDestacado {
    /// En una linea: "# 3 Titulo - Artista".
    fn texto(&self) -> String {
        let tema = if self.artista.is_empty() { self.titulo.clone() } else { format!("{} - {}", self.titulo, self.artista) };
        if self.numero.is_empty() { tema } else { format!("# {} {tema}", self.numero) }
    }
}

/// Nombre del archivo sin la extension ("Aplausos.wav" -> "Aplausos").
fn nombre_sin_extension(archivo: &Path) -> String {
    archivo.file_stem().map_or_else(|| nombre_de_archivo(archivo), |nombre| nombre.to_string_lossy().into_owned())
}

fn fecha_y_hora(momento: &jiff::Zoned) -> (String, String) {
    let dia = match momento.weekday() {
        jiff::civil::Weekday::Monday => tr!("Lunes"),
        jiff::civil::Weekday::Tuesday => tr!("Martes"),
        jiff::civil::Weekday::Wednesday => tr!("Miércoles"),
        jiff::civil::Weekday::Thursday => tr!("Jueves"),
        jiff::civil::Weekday::Friday => tr!("Viernes"),
        jiff::civil::Weekday::Saturday => tr!("Sábado"),
        jiff::civil::Weekday::Sunday => tr!("Domingo"),
    };
    (format!("{dia} {}", momento.strftime(crate::formato_de_fecha::actual().completa())), momento.strftime("%H:%M:%S").to_string())
}

/// Fecha y hora local para los registros, en ISO 8601: "2026-11-02 14:05:33".
fn hora_local_actual() -> String {
    jiff::Zoned::now().strftime(FORMATO_DE_FECHA_Y_HORA).to_string()
}

/// Ranking como estrellas: "★★★★☆ (4/5)".
fn estrellas(ranking: u8) -> String {
    format!("{} ({}/{RANKING_MAXIMO})", estrellas_de_la_lista(ranking), ranking.min(RANKING_MAXIMO))
}

/// Ranking como estrellas, sin el numero, para la columna de la lista: "★★★★☆".
fn estrellas_de_la_lista(ranking: u8) -> String {
    let llenas = ranking.min(RANKING_MAXIMO);
    format!("{}{}", "★".repeat(llenas as usize), "☆".repeat((RANKING_MAXIMO - llenas) as usize))
}

/// Columna Overlay de la lista: solo los segundos que dura la ventana ("27.5").
fn segundos_de_la_ventana(ventana: Option<crate::dominio::VentanaDeOverlay>) -> String {
    ventana.map_or_else(|| "—".into(), |ventana| formatear_segundos(ventana.fin() - ventana.inicio()))
}

/// Nombre de una pista con su numero de orden en la lista: "# 12 tema.flac".
fn etiqueta(lista: &ListaDeReproduccion, pista: &Pista) -> String {
    match lista.indice_de(pista) {
        Some(indice) => format!("# {} {}", indice + 1, pista.nombre()),
        None => pista.nombre(),
    }
}

impl Controlador {
    fn inicializar(&mut self, ventana: &VentanaPrincipal) {
        let repositorio = &self.repositorio_de_botonera;
        ventana.set_volumen(self.volumen_seleccionado as f32);
        ventana.set_editor_centrado(self.repositorio_de_preferencias.editor_centrado());
        ventana.set_ocultar_textos(self.repositorio_de_preferencias.ocultar_textos());
        ventana.set_caratulas_de_internet(self.repositorio_de_preferencias.caratulas_de_internet());
        ventana.set_ver_onda(self.repositorio_de_preferencias.ver_onda());
        ventana.set_controles_visibles(self.repositorio_de_preferencias.controles_visibles());
        ventana.set_ajustes_de_efectos_visibles(self.repositorio_de_preferencias.ajustes_de_efectos_visibles());
        ventana.set_fader(f32::from(self.repositorio_de_preferencias.fader()));
        ventana.set_crossfade_inteligente(self.repositorio_de_preferencias.crossfade_inteligente());
        ventana.set_nivelar_volumen(self.repositorio_de_preferencias.nivelar_volumen());
        ventana.set_objetivo_lufs(self.repositorio_de_preferencias.objetivo_de_sonoridad().indice() as i32);
        ventana.set_desplazar(self.repositorio_de_preferencias.desplazar());
        ventana.set_al_aire(self.repositorio_de_preferencias.modo_de_operacion().al_aire());
        ventana.set_modo_libre(repositorio.modo() == ModoDeOverlay::Libre);
        let ajustes = repositorio.ajustes_de_efecto();
        self.overlay.definir_ajustes(ajustes);
        self.mostrar_ajustes_de_efecto(ajustes);
        preparar_efectos(repositorio.efectos().map(|(_, _, archivo)| archivo.to_path_buf()).collect());
        self.gestor.ajustar_volumen(self.volumen_seleccionado);
        self.refrescar_botones();
        self.refrescar_interfaz();
    }

    fn con_ventana(&self, accion: impl FnOnce(&VentanaPrincipal)) {
        if let Some(ventana) = self.ventana.upgrade() {
            accion(&ventana);
        }
    }

    fn mostrar_estado(&self, texto: &str) {
        self.con_ventana(|ventana| ventana.set_estado(texto.into()));
    }

    fn estado_visible(&self) -> String {
        self.ventana.upgrade().map(|ventana| ventana.get_estado().to_string()).unwrap_or_default()
    }

    fn fader(&self) -> u8 {
        self.repositorio_de_preferencias.fader()
    }

    /// Ciclo periodico: atiende el fin de pistas y las mezclas, libera los
    /// efectos en espera, aplica busquedas pendientes y actualiza la vista.
    fn ciclo(&mut self) {
        let (lista, datos, canciones, inteligente) =
            (&self.lista, &self.datos_de_mezcla, &self.repositorio_de_canciones, self.crossfade_inteligente());
        let eventos = self.gestor.ciclo(|actual| transicion_desde(lista, datos, canciones, inteligente, actual));
        for evento in eventos {
            match evento {
                EventoDeReproduccion::PistaFinalizada(pista) => self.avanzar_secuencia(&pista),
                EventoDeReproduccion::TransicionIniciada(pista) => self.al_iniciar_transicion(&pista),
                EventoDeReproduccion::Detenida => match self.tras_el_fundido.take() {
                    Some((indice, pista)) => self.reproducir_tras_el_fundido(indice, &pista),
                    None => self.al_quedar_detenida(),
                },
                // Sin dialogo: la secuencia sigue sola con la pista siguiente.
                EventoDeReproduccion::Fallo(motivo) => self.mostrar_estado(&motivo),
            }
        }
        self.atender_corte_de_tanda();

        // Solo con un efecto de la botonera en espera hace falta saber cuales
        // son las bases del HTH (sobre ellas no sale: va la voz del anuncio).
        let carpeta_de_bases = self.overlay.en_espera().and_then(|_| self.carpeta_de_bases_en_la_lista());
        let musica = MedidorDeMusica {
            datos: &self.datos_de_mezcla,
            nivelar: self.repositorio_de_preferencias.nivelar_volumen(),
            objetivo: self.repositorio_de_preferencias.objetivo_de_sonoridad(),
            volumen: self.volumen_seleccionado,
            onda: &self.onda_de_musica,
        };
        let lista = &self.lista;
        let ventana_de = |pista: &Pista| lista.puntos_de(pista).overlay();
        let canciones = &self.repositorio_de_canciones;
        let admite_efectos = |pista: &Pista| admite_efectos_de_la_botonera(canciones, carpeta_de_bases.as_deref(), pista);
        let musica_de = |pista: &Pista, desde: f64, segundos: f64| musica.medir(pista, desde, segundos);
        let canciones_de_efectos = &self.repositorio_de_canciones;
        let recorte_de = |archivo: &Path| recorte_de_efecto(canciones_de_efectos, archivo);
        let entorno = Entorno {
            ventana_de: &ventana_de,
            admite_efectos: &admite_efectos,
            musica_de: &musica_de,
            recorte_de: &recorte_de,
        };
        if let Err(error) = self.overlay.ciclo(&self.gestor, &entorno) {
            self.errores.push(error.to_string());
        }

        if let Some((segundos, momento)) = self.busqueda_pendiente {
            if momento.elapsed() >= ESPERA_PARA_BUSCAR {
                self.busqueda_pendiente = None;
                if let Err(error) = self.gestor.buscar(segundos) {
                    self.errores.push(error.to_string());
                }
            }
        }
        if self.atenuacion_por_guardar.is_some_and(|momento| momento.elapsed() >= ESPERA_PARA_GUARDAR_ATENUACION) {
            self.atenuacion_por_guardar = None;
            self.repositorio_de_botonera.guardar_ajustes_de_efecto(self.overlay.ajustes());
        }

        let estado = (self.gestor.esta_reproduciendo(), self.gestor.esta_pausado());
        if estado != self.estado_anterior {
            self.refrescar_interfaz();
        }
        self.actualizar_progreso();
        self.refrescar_botones();
        self.refrescar_estado_de_botonera();
        self.ciclo_del_editor();
        self.seguir_reproduccion();
        self.refrescar_aviso_de_fader();
        if std::mem::take(&mut self.filas_por_refrescar) {
            self.refrescar_filas();
            self.refrescar_analisis_de_mezcla();
        }
        self.refrescar_totales();
        self.completar_zona_de_la_localidad();
        self.refrescar_reloj();
        self.ciclos += 1;
        // Una vez por segundo: el tiempo transcurrido de la ultima reproduccion.
        if self.ciclos % 20 == 0 {
            self.refrescar_detalles();
            self.refrescar_horarios();
            self.refrescar_clima();
            self.ciclo_de_tandas();
            self.refrescar_pauta();
            self.refrescar_ordenes_si_estan_abiertas();
        }
    }

    /// La hora de la localidad elegida para el anuncio HTH: la de su zona
    /// horaria (guardada con la localidad o, si es una localidad de antes,
    /// la que informo la consulta del clima); si no se conoce, la del equipo.
    pub(super) fn ahora_en_la_localidad(&self) -> jiff::Zoned {
        let ahora = jiff::Zoned::now();
        let nombre = self.repositorio_de_preferencias.localidad_del_clima().zona_horaria.clone().or_else(|| self.clima.zona_informada());
        match nombre.as_deref().and_then(crate::servicio::zona_horaria) {
            Some(zona) => ahora.with_time_zone(zona),
            None => ahora,
        }
    }

    /// Zona horaria de la localidad del HTH (la del sistema si no se sabe).
    pub(super) fn zona_de_la_localidad(&self) -> jiff::tz::TimeZone {
        self.ahora_en_la_localidad().time_zone().clone()
    }

    /// Una localidad guardada sin zona horaria (de una version anterior) se
    /// completa con la que informo la consulta del clima.
    fn completar_zona_de_la_localidad(&mut self) {
        let localidad = self.repositorio_de_preferencias.localidad_del_clima();
        if localidad.zona_horaria.is_some() {
            return;
        }
        if let Some(zona) = self.clima.zona_informada() {
            let completa = crate::servicio::Localidad { zona_horaria: Some(zona), ..localidad.clone() };
            self.repositorio_de_preferencias.guardar_localidad_del_clima(completa);
            self.recalcular_por_la_localidad();
        }
    }

    fn refrescar_reloj(&self) {
        let (fecha, hora) = fecha_y_hora(&self.ahora_en_la_localidad());
        self.con_ventana(|ventana| {
            if ventana.get_hora_actual() != hora.as_str() || ventana.get_fecha_actual() != fecha.as_str() {
                ventana.set_hora_actual(hora.into());
                ventana.set_fecha_actual(fecha.into());
            }
        });
    }

    /// Registra la ultima reproduccion de una pista cuando empieza a sonar:
    /// la columna Sono de la lista muestra la fecha y hora de ese comienzo.
    fn seguir_reproduccion(&mut self) {
        let sonando = (self.gestor.esta_reproduciendo() || self.gestor.esta_pausado()) && self.gestor.hay_pista_cargada();
        let actual = self.gestor.pista_actual().filter(|_| sonando).cloned();
        if actual != self.pista_en_curso {
            self.registrar_fin_de_reproduccion();
            if let Some(pista) = actual.as_ref().filter(|pista| !es_anuncio_hth(pista.archivo())) {
                self.repositorio_de_canciones.registrar_reproduccion(pista.archivo(), momento_actual());
            }
            self.pista_en_curso = actual;
            self.inicio_en_curso = hora_local_actual();
            self.posicion_en_curso = 0.0;
            // La columna Sono muestra el comienzo y las nuevas estimaciones.
            self.refrescar_filas();
        } else if actual.is_some() {
            self.posicion_en_curso = self.gestor.tiempo_transcurrido();
        }
    }

    fn registrar_fin_de_reproduccion(&mut self) {
        if let Some(pista) = self.pista_en_curso.take() {
            if es_facturable(&self.repositorio_de_canciones, pista.archivo()) {
                self.anotar_comercial(&pista);
            }
            self.refrescar_detalles();
        }
    }

    /// Anota en el registro de comerciales una salida al aire que termino
    /// (porque se acabo, se paso a otra pista o se detuvo).
    fn anotar_comercial(&mut self, pista: &Pista) {
        let ficha = self.ficha_de(pista);
        let duracion = self.repositorio_de_canciones.buscar(pista.archivo()).and_then(|ficha| ficha.duracion);
        let recorte_final = self.lista.puntos_de(pista).recorte_final();
        // Margen por el ciclo de refresco y el silencio final del archivo.
        let completa = duracion.is_some_and(|duracion| self.posicion_en_curso >= duracion - recorte_final - 1.0);
        let origen = self.pistas_de_tanda.get_mut(pista).and_then(std::collections::VecDeque::pop_front).unwrap_or_default();
        if self.pistas_de_tanda.get(pista).is_some_and(std::collections::VecDeque::is_empty) {
            self.pistas_de_tanda.remove(pista);
        }
        self.registro_de_comerciales.anotar(&EmisionDeComercial {
            inicio: self.inicio_en_curso.clone(),
            fin: hora_local_actual(),
            segundos: self.posicion_en_curso,
            duracion,
            completa,
            titulo: ficha.titulo.unwrap_or_else(|| pista.nombre()),
            artista: ficha.artista.unwrap_or_default(),
            genero: ficha.genero.unwrap_or_default(),
            tipo: ficha.tipo.map(TipoDeElemento::nombre).unwrap_or_default().into(),
            archivo: pista.archivo().to_path_buf(),
            tanda: origen.tanda,
            orden: origen.orden.clone().unwrap_or_default(),
        });
        // Un aviso de una orden que salio cortado se repone hoy.
        if let Some(orden) = origen.orden.filter(|_| !completa) {
            self.agregar_reposicion(&orden, pista.archivo().to_path_buf());
            self.mostrar_estado(&tr!("El aviso de la orden «{orden}» salio cortado: se repone en la proxima tanda posible", orden = orden));
        }
    }

    // Busqueda en la lista

    /// Marca las pistas que coinciden con el texto (palabras o condiciones
    /// con prefijo, ver `ConsultaDeTemas`) y lleva el selector a la primera
    /// desde la seleccion; con `#508`, a esa posicion. Con el texto vacio, la
    /// lista vuelve a la normalidad.
    fn buscar(&mut self, texto: &str) {
        (self.busqueda, self.problema_de_busqueda) = match ConsultaDeTemas::nueva_en(texto, &self.ahora_en_la_localidad()) {
            Ok(consulta) => (consulta, None),
            Err(problema) => (None, Some(problema)),
        };
        self.refrescar_filas();
        if let Some(problema) = self.problema_de_busqueda.clone() {
            self.mostrar_estado(&problema);
            return;
        }
        let ya_esta_en_una = self.ancla_de_seleccion.is_some_and(|ancla| self.coincidencias.contains(&ancla));
        if self.busqueda.is_some() && !ya_esta_en_una {
            let desde = self.ancla_de_seleccion.and_then(|ancla| ancla.checked_sub(1));
            if let Some(indice) = saltar_a_coincidencia(&self.coincidencias, desde, true) {
                self.ir_a_fila(indice);
            }
        }
    }

    /// Salta a la coincidencia siguiente o anterior, dando la vuelta.
    fn ir_a_coincidencia(&mut self, avanzar: bool) {
        if let Some(indice) = saltar_a_coincidencia(&self.coincidencias, self.ancla_de_seleccion, avanzar) {
            self.ir_a_fila(indice);
        }
    }

    fn ir_a_fila(&mut self, indice: usize) {
        self.seleccionar_solo(indice);
        self.refrescar_filas();
        self.con_ventana(|ventana| ventana.invoke_mostrar_fila(indice as i32));
        self.informar_marcador();
    }

    /// "3 de 12", "12 resultados" o "Sin resultados"; vacio sin busqueda.
    fn resultado_de_busqueda(&self) -> String {
        if self.problema_de_busqueda.is_some() {
            return tr!("Búsqueda no válida").into();
        }
        if self.busqueda.is_none() {
            return String::new();
        }
        let total = self.coincidencias.len();
        match self.ancla_de_seleccion.and_then(|ancla| self.coincidencias.iter().position(|&indice| indice == ancla)) {
            _ if total == 0 => tr!("Sin resultados").into(),
            Some(orden) => format!("{} de {total}", orden + 1),
            None => format!("{total} resultado(s)"),
        }
    }

    /// Recupera la lista guardada al cerrar la ultima vez. Los archivos que
    /// ya no existen se omiten.
    /// Recupera la lista de la sesion anterior (con los cambios que no se
    /// hayan guardado) o, si no hay, la ultima lista con nombre. El tema
    /// actual es el ultimo que recuerdan las preferencias.
    fn recuperar_lista(&mut self) {
        let ultima = self.repositorio_de_preferencias.ultima_lista().map(Path::to_path_buf).filter(|lista| lista.is_file());
        let de_la_ultima = ultima.as_deref().and_then(|lista| leer_lista(lista).ok());
        self.contenido_guardado =
            de_la_ultima.as_ref().map(|lista| lista.pistas.iter().map(Pista::new).collect()).unwrap_or_default();
        self.archivo_de_lista = ultima;
        let mut guardada = self.repositorio_de_lista.cargar();
        if guardada.pistas.is_empty() {
            guardada = de_la_ultima.unwrap_or_default();
        }
        guardada.pistas = guardada.pistas.iter().map(|pista| self.ruta_para_cargar(pista)).collect();
        if guardada.pistas.is_empty() {
            self.refrescar_titulo();
            return;
        }
        self.preparar_lista_con_anuncio_hth(&guardada.pistas);
        self.preparar_lista_con_marcadores(&guardada.pistas);
        let total = guardada.pistas.len();
        // La actual se ubica por posicion (puede haber copias del mismo
        // comercial), descontando los archivos que faltan antes que ella.
        let actual = guardada.actual.filter(|&indice| guardada.pistas[indice].is_file()).map(|indice| {
            indice - guardada.pistas[..indice].iter().filter(|archivo| !archivo.is_file()).count()
        });
        let existentes: Vec<PathBuf> = guardada.pistas.into_iter().filter(|archivo| archivo.is_file()).collect();
        let faltan = total - existentes.len();
        // La lista guardada ya estaba validada: se recupera tal cual, con sus copias.
        self.insertar_pistas(existentes, Some(0), true);
        self.lista.definir_repetir(guardada.repetir);
        self.con_ventana(|ventana| ventana.set_repetir(guardada.repetir));
        let ultimo_tema = self.repositorio_de_preferencias.ultimo_tema().cloned().and_then(|tema| {
            let pista = Pista::new(&tema.archivo);
            if self.lista.pista_en(tema.posicion) == Some(&pista) { Some(tema.posicion) } else { self.lista.indice_de(&pista) }
        });
        if let Some(indice) = ultimo_tema.or(actual).filter(|&indice| indice < self.lista.tamano()) {
            self.lista.seleccionar_indice(indice);
            self.seleccionar_solo(indice);
            self.con_ventana(|ventana| ventana.invoke_mostrar_fila(indice as i32));
        }
        self.refrescar_titulo();
        self.refrescar_interfaz();
        let nombre = self.archivo_de_lista.as_deref().map_or_else(String::new, |lista| format!(" ({})", nombre_de_archivo(lista)));
        let mut mensaje = tr!("Se recupero la lista anterior{nombre}: {} pista(s)", self.lista.tamano(), nombre = nombre);
        if faltan > 0 {
            mensaje.push_str(&tr!(" ({faltan} archivo(s) ya no existen)", faltan = faltan));
        }
        self.mostrar_estado(&mensaje);
    }

    /// Guarda la lista de la sesion para recuperarla al volver a abrir el
    /// programa, y el tema actual en las preferencias.
    fn guardar_lista(&mut self) {
        self.repositorio_de_lista.guardar(&self.lista_para_guardar());
        let ultimo = self.lista.indice_actual().and_then(|posicion| {
            Some(UltimoTema { posicion, archivo: self.lista.pista_en(posicion)?.archivo().to_path_buf() })
        });
        self.repositorio_de_preferencias.guardar_ultimo_tema(ultimo);
        self.refrescar_titulo();
    }

    fn lista_para_guardar(&self) -> ListaGuardada {
        ListaGuardada {
            pistas: self.lista.pistas().iter().map(|pista| pista.archivo().to_path_buf()).collect(),
            actual: self.lista.indice_actual(),
            repetir: self.lista.repite(),
        }
    }

    /// Si la lista cambio desde que se cargo o se guardo con nombre (una
    /// lista sin nombre y no vacia cuenta como cambiada).
    pub fn lista_modificada(&self) -> bool {
        self.lista.pistas() != self.contenido_guardado.as_slice()
    }

    /// Nombre de la lista en el titulo de la ventana, con * si hay cambios.
    fn refrescar_titulo(&self) {
        let nombre = self.archivo_de_lista.as_deref().map_or_else(|| tr!("Lista sin nombre"), nombre_de_archivo);
        let marca = if self.lista_modificada() && !self.lista.esta_vacia() { " *" } else { "" };
        self.con_ventana(|ventana| ventana.set_nombre_de_lista(format!("{nombre}{marca}").into()));
    }

    /// Ruta con que entra a la lista una pista guardada: los marcadores de
    /// bloque fijo y el anuncio HTH de otra carpeta de datos (otro equipo u
    /// otro usuario) pasan a los de esta, y los archivos reubicados en el
    /// mantenimiento, a su ruta nueva.
    fn ruta_para_cargar(&self, ruta: &Path) -> PathBuf {
        marcadores::marcador_en_esta_carpeta(ruta)
            .or_else(|| hth::anuncio_hth_en_esta_carpeta(ruta))
            .unwrap_or_else(|| self.repositorio_de_canciones.ruta_actual(ruta))
    }

    /// Reemplaza la lista por la de un archivo (del reproductor o M3U). Si
    /// hay un tema sonando, sigue sonando: si esta en la lista nueva queda
    /// como actual y, si no, al terminar empieza la lista nueva.
    pub fn cargar_lista_desde(&mut self, archivo: &Path) -> Result<String, String> {
        self.admitida(OperacionSobreLaSecuencia::CargarLista)?;
        let mut leida = leer_lista(archivo)?;
        // Lo que se guarda como contenido de la lista es lo del archivo: si
        // hubo reubicaciones, la lista queda con cambios para guardarla al dia.
        let en_el_archivo: Vec<Pista> = leida.pistas.iter().map(Pista::new).collect();
        leida.pistas = leida.pistas.iter().map(|pista| self.ruta_para_cargar(pista)).collect();
        self.preparar_lista_con_anuncio_hth(&leida.pistas);
        self.preparar_lista_con_marcadores(&leida.pistas);
        let total = leida.pistas.len();
        let existentes: Vec<PathBuf> = leida.pistas.iter().filter(|pista| pista.is_file()).cloned().collect();
        let faltan = total - existentes.len();
        let actual = leida.actual.filter(|&indice| leida.pistas[indice].is_file()).map(|indice| {
            indice - leida.pistas[..indice].iter().filter(|pista| !pista.is_file()).count()
        });
        self.tras_el_fundido = None;
        self.lista.limpiar();
        self.limpiar_seleccion();
        self.insertar_pistas(existentes, Some(0), true);
        self.lista.definir_repetir(leida.repetir);
        self.con_ventana(|ventana| ventana.set_repetir(leida.repetir));
        let sonando = self.gestor.pista_actual().filter(|_| self.gestor.hay_pista_cargada()).cloned();
        let nueva_actual = match sonando {
            Some(pista) => self.lista.indice_de(&pista),
            None => actual,
        };
        if let Some(indice) = nueva_actual.filter(|&indice| indice < self.lista.tamano()) {
            self.lista.seleccionar_indice(indice);
            self.seleccionar_solo(indice);
            self.con_ventana(|ventana| ventana.invoke_mostrar_fila(indice as i32));
        }
        self.contenido_guardado = en_el_archivo;
        self.archivo_de_lista = Some(archivo.to_path_buf());
        self.repositorio_de_preferencias.guardar_ultima_lista(Some(archivo));
        self.guardar_lista();
        self.refrescar_interfaz();
        let mut mensaje = tr!("Lista {} cargada: {} pista(s)", nombre_de_archivo(archivo), self.lista.tamano());
        if faltan > 0 {
            mensaje.push_str(&tr!(" ({faltan} archivo(s) ya no existen)", faltan = faltan));
        }
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Archivo de la lista abierta, si se cargo o se guardo con nombre.
    pub fn archivo_de_lista_abierta(&self) -> Option<PathBuf> {
        self.archivo_de_lista.clone()
    }

    /// Guarda la lista con ese nombre; queda como la lista abierta.
    pub fn guardar_lista_en(&mut self, archivo: &Path) -> Result<String, String> {
        escribir_lista(archivo, &self.lista_para_guardar())?;
        self.contenido_guardado = self.lista.pistas().to_vec();
        self.archivo_de_lista = Some(archivo.to_path_buf());
        self.repositorio_de_preferencias.guardar_ultima_lista(Some(archivo));
        self.refrescar_titulo();
        let mensaje = tr!("Lista guardada en {}", archivo.display());
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Lleva el selector a la pista que suena y la deja a la vista.
    fn ir_a_pista_actual(&mut self) {
        let Some(indice) = self.lista.indice_actual().filter(|_| self.gestor.hay_pista_cargada()) else {
            self.mostrar_estado(&tr!("No hay una pista en reproduccion"));
            return;
        };
        self.seleccionar_solo(indice);
        self.refrescar_filas();
        self.con_ventana(|ventana| ventana.invoke_mostrar_fila(indice as i32));
        let pista = self.lista.pista_en(indice).expect("indice valido");
        self.mostrar_estado(&tr!("Selector en {}", etiqueta(&self.lista, pista)));
    }

    fn nivel_de_musica_por_defecto(&mut self) {
        self.cambiar_ajustes_de_efecto(AjustesDeEfecto { nivel_de_musica: 0.5, ..self.overlay.ajustes() });
    }

    /// Muestra la hoja de efectos anterior (-1) o la siguiente (+1).
    fn mover_hoja(&mut self, desplazamiento: i64) {
        let destino = self.repositorio_de_botonera.hoja_actual() as i64 + desplazamiento;
        if let Ok(destino) = usize::try_from(destino) {
            let _ = self.hojas_de_efectos(AccionDeHoja::Ir(destino));
        }
    }

    /// Cambia de hoja de efectos, agrega una o quita una (siempre queda una).
    pub fn hojas_de_efectos(&mut self, accion: AccionDeHoja) -> Result<String, String> {
        let repositorio = &mut self.repositorio_de_botonera;
        let mensaje = match accion {
            AccionDeHoja::Ir(hoja) if hoja < repositorio.cantidad_de_hojas() => {
                repositorio.mostrar_hoja(hoja);
                tr!("Hoja de efectos {} de {}.", hoja + 1, repositorio.cantidad_de_hojas())
            }
            AccionDeHoja::Ir(hoja) => {
                return Err(tr!("No existe la hoja {} (hay {}).", hoja + 1, repositorio.cantidad_de_hojas()));
            }
            AccionDeHoja::Agregar => {
                let hoja = repositorio.agregar_hoja();
                tr!("Se agrego la hoja de efectos {}.", hoja + 1)
            }
            AccionDeHoja::Quitar(hoja) => {
                repositorio.quitar_hoja(hoja)?;
                tr!("Se quito la hoja {}; quedan {}.", hoja + 1, repositorio.cantidad_de_hojas())
            }
            AccionDeHoja::Renombrar(hoja, nombre) => {
                let hoja = hoja.unwrap_or(repositorio.hoja_actual());
                repositorio.renombrar_hoja(hoja, &nombre)?;
                match repositorio.nombre_de_hoja(hoja) {
                    Some(nombre) => tr!("La hoja {} se llama «{nombre}».", hoja + 1, nombre = nombre),
                    None => tr!("La hoja {} volvio a llamarse «{}».", hoja + 1, nombre_de_siempre_de_la_hoja(hoja)),
                }
            }
        };
        self.refrescar_botones();
        Ok(mensaje)
    }

    /// Resalta Detener o Siguiente mientras dura el fundido que iniciaron.
    fn refrescar_aviso_de_fader(&mut self) {
        if !self.gestor.hay_transicion_en_curso() {
            self.mezcla_manual = false;
        }
        let deteniendo = self.gestor.esta_deteniendo();
        let mezclando = self.mezcla_manual;
        // Cualquier mezcla, automatica o pedida con Siguiente, pinta el tiempo.
        let en_mezcla = self.gestor.hay_transicion_en_curso() && !deteniendo;
        self.con_ventana(|ventana| {
            ventana.set_deteniendo(deteniendo);
            ventana.set_mezclando(mezclando);
            ventana.set_en_mezcla(en_mezcla);
        });
    }

    /// Duracion total de la lista y lo que falta para terminarla, ambas
    /// descontando los segundos que se superponen en cada mezcla.
    fn totales(&self) -> (f64, Option<f64>, usize) {
        let sin_duracion = self.duraciones.iter().filter(|duracion| duracion.is_none()).count();
        let mezcla_tras = |indice: usize| self.ahorros_de_mezcla.get(indice).copied().unwrap_or(0.0);
        let puntos = |indice: usize| self.lista.pista_en(indice).map(|pista| self.lista.puntos_de(pista)).unwrap_or_default();
        let neto = |desde: usize| self.netos_desde.get(desde).copied().unwrap_or(0.0);
        let sonando = self.gestor.esta_reproduciendo() || self.gestor.esta_pausado();
        let restante = self.lista.indice_actual().filter(|_| sonando && self.gestor.hay_pista_cargada()).map(|actual| {
            let duracion = self.gestor.duracion_total().or(self.duraciones.get(actual).copied().flatten()).unwrap_or(0.0);
            let fin = duracion - puntos(actual).recorte_final();
            (fin - self.gestor.tiempo_transcurrido()).max(0.0) - mezcla_tras(actual) + neto(actual + 1)
        });
        (neto(0), restante.map(|segundos| segundos.max(0.0)), sin_duracion)
    }

    fn refrescar_totales(&self) {
        let (total, restante, sin_duracion) = self.totales();
        let mut texto = format!("Total {}", formatear_duracion(total));
        if let Some(restante) = restante {
            texto.push_str(&tr!("  ·  Restante {}", formatear_duracion(restante)));
        }
        if sin_duracion > 0 {
            texto.push_str(&tr!("  (+{sin_duracion} sin duracion)", sin_duracion = sin_duracion));
        }
        self.con_ventana(|ventana| ventana.set_total_de_lista(texto.into()));
    }

    /// Muestra en el panel de detalles la pista marcada con el selector o,
    /// si no hay ninguna, la que suena.
    fn refrescar_detalles(&self) {
        let pista = self
            .ancla_de_seleccion
            .filter(|indice| self.seleccion.contains(indice))
            .or(self.lista.indice_actual())
            .and_then(|indice| self.lista.pista_en(indice));
        let titulo_de_la_onda = pista.map(|pista| self.titulo_y_artista(pista)).unwrap_or_default();
        let (carpeta, archivo, ficha) = match pista {
            Some(pista) => (
                pista.archivo().parent().map_or_else(|| "—".to_string(), |carpeta| carpeta.display().to_string()),
                nombre_de_archivo(pista.archivo()),
                self.ficha_de(pista),
            ),
            None => ("—".to_string(), tr!("Sin pista seleccionada"), FichaDeCancion::default()),
        };
        let dato = |valor: &Option<String>| valor.clone().unwrap_or_else(|| "—".into());
        let (ultima, transcurrido) = match (pista, ficha.ultima_reproduccion.as_deref()) {
            (None, _) => ("—".to_string(), String::new()),
            (Some(_), None) => (tr!("nunca"), String::new()),
            (Some(_), Some(momento)) => ultima_reproduccion(momento, &self.zona_de_la_localidad()),
        };
        let ranking = if pista.is_some() { estrellas(ficha.ranking) } else { "—".into() };
        let anio = ficha.anio.map_or_else(|| "—".to_string(), |anio| anio.to_string());
        let efecto = ficha.efecto_fijado.as_deref().map_or_else(|| "—".to_string(), nombre_de_archivo);
        // Clave fija del tipo: el color de la carátula por defecto y el icono del tipo.
        let tipo_clave = match pista {
            Some(pista) if es_marcador(pista.archivo()) => "Bloque fijo",
            Some(pista) if es_anuncio_hth(pista.archivo()) => "HTH",
            Some(_) => ficha.tipo.map_or("", TipoDeElemento::nombre),
            None => "",
        };
        self.con_ventana(|ventana| {
            ventana.set_detalle_tipo_clave(tipo_clave.into());
            ventana.set_detalle_carpeta(carpeta.into());
            ventana.set_titulo_de_la_onda(titulo_de_la_onda.into());
            ventana.set_detalle_archivo(archivo.into());
            ventana.set_detalle_genero(dato(&ficha.genero).into());
            ventana.set_detalle_tipo(ficha.tipo.map_or_else(|| tr!("Sin clasificar"), TipoDeElemento::nombre_visible).into());
            ventana.set_detalle_album(dato(&ficha.album).into());
            ventana.set_detalle_anio(anio.into());
            ventana.set_detalle_efecto(efecto.into());
            ventana.set_detalle_ranking(ranking.into());
            ventana.set_detalle_ultima(ultima.into());
            ventana.set_detalle_hace(transcurrido.into());
        });
        self.refrescar_marcas_de_la_onda(pista);
        self.refrescar_onda_del_detalle(pista.map(Pista::archivo));
        self.refrescar_caratula_del_detalle(pista);
    }

    /// Mueve las pistas seleccionadas (una, un bloque o varias salteadas)
    /// una posicion arriba (-1) o abajo (+1); la seleccion las sigue.
    fn mover_seleccionada(&mut self, desplazamiento: i32) {
        if self.seleccion.is_empty() {
            self.mostrar_estado(&tr!("Seleccione una o mas pistas para moverlas"));
            return;
        }
        if !self.admitida_o_avisar(OperacionSobreLaSecuencia::Mover(self.seleccion.len())) {
            return;
        }
        let abajo = desplazamiento > 0;
        let posiciones: Vec<usize> = self.seleccion.iter().copied().collect();
        let Some(nuevas) = self.lista.desplazar(&posiciones, abajo) else {
            self.mostrar_estado(&if abajo { tr!("Ya esta al final de la lista") } else { tr!("Ya esta al principio de la lista") });
            return;
        };
        let correr = |posicion: usize| if abajo { posicion + 1 } else { posicion - 1 };
        self.ancla_de_seleccion = self.ancla_de_seleccion.filter(|ancla| self.seleccion.contains(ancla)).map(correr);
        self.seleccion = nuevas.iter().copied().collect();
        self.guardar_lista();
        self.refrescar_interfaz();
        let visible = if abajo { nuevas.last() } else { nuevas.first() };
        if let Some(&visible) = visible {
            self.con_ventana(|ventana| ventana.invoke_mostrar_fila(visible as i32));
        }
        let mensaje = match nuevas.as_slice() {
            [unica] => {
                let pista = self.lista.pista_en(*unica).expect("posicion valida");
                tr!("{} (antes # {})", etiqueta(&self.lista, pista), posiciones[0] + 1)
            }
            _ => tr!("{} pistas movidas {}", nuevas.len(), if abajo { "abajo" } else { "arriba" }),
        };
        self.mostrar_estado(&mensaje);
    }

    /// Encola una pista (posicion desde 0) como la proxima: la lleva justo
    /// despues de la que suena (o al principio, si no suena ninguna), asi el
    /// final de la actual se mezcla con ella. Lo demas sigue en su orden.
    pub fn encolar_como_siguiente(&mut self, indice: usize) -> Result<String, String> {
        let pista = self.lista.pista_en(indice).cloned().ok_or_else(|| {
            tr!("Posicion fuera de la lista (1 a {}).", self.lista.tamano())
        })?;
        if es_marcador(pista.archivo()) {
            return Err(tr!("Los marcadores de bloque fijo no se encolan.").into());
        }
        let actual = self.lista.indice_actual();
        if actual == Some(indice) {
            return Err(tr!("{} es la pista actual.", etiqueta(&self.lista, &pista)));
        }
        // Quitarla de antes de la actual corre la actual un lugar hacia arriba.
        let destino = match actual {
            Some(actual) if indice > actual => actual + 1,
            Some(actual) => actual,
            None => 0,
        };
        if destino != indice {
            self.lista.mover(indice, destino);
            self.guardar_lista();
        }
        let nombre = etiqueta(&self.lista, &pista);
        self.seleccionar_solo(destino);
        self.refrescar_interfaz();
        self.con_ventana(|ventana| ventana.invoke_mostrar_fila(destino as i32));
        let como = match self.lista.pista_actual() {
            Some(saliente) if suena_sin_mezcla(&self.repositorio_de_canciones, saliente)
                || suena_sin_mezcla(&self.repositorio_de_canciones, &pista) => tr!("sin mezcla (no es un Tema)"),
            Some(_) if self.gestor.hay_pista_cargada() => tr!("mezclada con el final de la actual"),
            _ => tr!("a continuacion"),
        };
        let mensaje = if destino == indice {
            tr!("{nombre} ya es la siguiente: suena {como}", nombre = nombre, como = como)
        } else {
            tr!("{nombre} encolada (antes # {}): suena {como}", indice + 1, nombre = nombre, como = como)
        };
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Lleva una pista (posicion desde 0) a otra posicion; el selector la sigue.
    pub fn mover_pista(&mut self, desde: usize, hasta: usize) -> Result<String, String> {
        let tamano = self.lista.tamano();
        if !self.lista.mover(desde, hasta) {
            return Err(tr!("Posicion fuera de la lista (1 a {tamano}).", tamano = tamano));
        }
        self.seleccionar_solo(hasta);
        self.guardar_lista();
        self.refrescar_interfaz();
        self.con_ventana(|ventana| ventana.invoke_mostrar_fila(hasta as i32));
        let pista = self.lista.pista_en(hasta).expect("posicion valida");
        let mensaje = tr!("{} (antes # {})", etiqueta(&self.lista, pista), desde + 1);
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Mezcla la lista al azar; la pista que suena queda primera.
    pub fn mezclar_lista(&mut self) -> Result<String, String> {
        self.mezclar(Reordenamiento::de_toda_la_lista(self.lista.tamano()))
    }

    /// Mezcla al azar el bloque seleccionado si hay dos o mas pistas
    /// seleccionadas; si no, toda la lista.
    pub fn mezclar_seleccion_o_lista(&mut self) -> Result<String, String> {
        self.mezclar(self.reordenamiento_segun_la_seleccion())
    }

    fn mezclar(&mut self, reordenamiento: Reordenamiento) -> Result<String, String> {
        self.admitida(OperacionSobreLaSecuencia::Reordenar)?;
        // Lo que esta en un bloque fijo no se mueve.
        let (posiciones, fijas) = self.sin_bloques_fijos(&reordenamiento.posiciones);
        if posiciones.len() < 2 {
            return Err(if fijas > 0 {
                tr!("No hay al menos dos pistas para mezclar fuera de los bloques fijos.").into()
            } else {
                tr!("Hacen falta al menos dos pistas para mezclar la lista.").into()
            });
        }
        let reordenamiento = Reordenamiento { posiciones, ..reordenamiento };
        let semilla = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |momento| momento.as_nanos() as u64);
        self.lista.mezclar(&reordenamiento.posiciones, semilla);
        let mut mensaje = if reordenamiento.es_un_bloque {
            tr!("{} pistas seleccionadas mezcladas al azar", reordenamiento.posiciones.len())
        } else {
            tr!("Lista mezclada al azar")
        };
        if fijas > 0 {
            mensaje.push_str(&tr!(" ({fijas} tema(s) en bloques fijos, sin cambios)", fijas = fijas));
        }
        Ok(self.lista_reordenada(&reordenamiento, &mensaje))
    }

    /// Ordena la lista por BPM, de menor a mayor y, a igual tempo, por
    /// ranking de mayor a menor; los temas sin tempo (o todavia sin analizar)
    /// van al final.
    pub fn ordenar_lista_por_bpm(&mut self) -> Result<String, String> {
        self.ordenar_por_bpm(Reordenamiento::de_toda_la_lista(self.lista.tamano()))
    }

    /// Ordena por BPM el bloque seleccionado si hay dos o mas pistas
    /// seleccionadas; si no, toda la lista.
    pub fn ordenar_seleccion_o_lista_por_bpm(&mut self) -> Result<String, String> {
        self.ordenar_por_bpm(self.reordenamiento_segun_la_seleccion())
    }

    fn ordenar_por_bpm(&mut self, reordenamiento: Reordenamiento) -> Result<String, String> {
        self.admitida(OperacionSobreLaSecuencia::Reordenar)?;
        // Lo que esta en un bloque fijo no se mueve.
        let (posiciones, fijas) = self.sin_bloques_fijos(&reordenamiento.posiciones);
        if posiciones.len() < 2 {
            return Err(if fijas > 0 {
                tr!("No hay al menos dos pistas para ordenar fuera de los bloques fijos.").into()
            } else {
                tr!("Hacen falta al menos dos pistas para ordenar la lista.").into()
            });
        }
        let reordenamiento = Reordenamiento { posiciones, ..reordenamiento };
        let pistas: Vec<Pista> =
            reordenamiento.posiciones.iter().filter_map(|&posicion| self.lista.pista_en(posicion).cloned()).collect();
        let tempos: HashMap<Pista, Option<f64>> = pistas.iter().map(|pista| (pista.clone(), self.bpm_de(pista.archivo()))).collect();
        let rankings: HashMap<Pista, u8> = pistas
            .iter()
            .map(|pista| (pista.clone(), self.repositorio_de_canciones.buscar(pista.archivo()).map_or(0, |ficha| ficha.ranking)))
            .collect();
        let sin_tempo = tempos.values().filter(|bpm| bpm.is_none()).count();
        self.lista.ordenar_por_bpm(&reordenamiento.posiciones, |pista| tempos[pista], |pista| rankings[pista]);
        let mut mensaje = if reordenamiento.es_un_bloque {
            tr!("{} pistas seleccionadas ordenadas por BPM, de menor a mayor (a igual tempo, por ranking)", pistas.len())
        } else {
            tr!("Lista ordenada por BPM, de menor a mayor (a igual tempo, por ranking)")
        };
        if sin_tempo > 0 {
            mensaje.push_str(&tr!(" ({sin_tempo} sin tempo, al final)", sin_tempo = sin_tempo));
        }
        if fijas > 0 {
            mensaje.push_str(&tr!(" ({fijas} tema(s) en bloques fijos, sin cambios)", fijas = fijas));
        }
        Ok(self.lista_reordenada(&reordenamiento, &mensaje))
    }

    /// Las posiciones seleccionadas si son dos o mas; si no, toda la lista.
    fn reordenamiento_segun_la_seleccion(&self) -> Reordenamiento {
        match self.seleccion.len() {
            0 | 1 => Reordenamiento::de_toda_la_lista(self.lista.tamano()),
            _ => Reordenamiento { posiciones: self.seleccion.iter().copied().collect(), es_un_bloque: true },
        }
    }

    /// Guarda y muestra la lista despues de cambiarle el orden. Un bloque
    /// queda seleccionado y a la vista; si fue toda la lista, se muestra
    /// desde el principio.
    fn lista_reordenada(&mut self, reordenamiento: &Reordenamiento, mensaje: &str) -> String {
        if !reordenamiento.es_un_bloque {
            self.limpiar_seleccion();
        }
        self.guardar_lista();
        self.refrescar_interfaz();
        let visible = if reordenamiento.es_un_bloque { reordenamiento.posiciones.first().copied().unwrap_or(0) } else { 0 };
        self.con_ventana(|ventana| ventana.invoke_mostrar_fila(visible as i32));
        self.mostrar_estado(mensaje);
        mensaje.to_string()
    }

    /// Abre el editor con la pista indicada o, con -1, con la marcada con el
    /// selector (o la que suena, o la primera).
    fn editar_pista_desde_la_lista(&mut self, indice: i32) {
        // Doble clic en un marcador: selecciona su bloque completo.
        if usize::try_from(indice).is_ok_and(|indice| self.seleccionar_bloque(indice)) {
            return;
        }
        let indice = usize::try_from(indice)
            .ok()
            .or(self.ancla_de_seleccion.filter(|indice| self.seleccion.contains(indice)))
            .or(self.lista.indice_actual())
            .unwrap_or(0);
        if let Err(error) = self.abrir_editor(indice) {
            self.mostrar_estado(&error);
        }
    }

    // Operaciones de lista compartidas por la interfaz y los clientes externos
    // (servidor MCP).

    pub fn pistas_en_lista(&self) -> Vec<Pista> {
        self.lista.pistas().to_vec()
    }

    pub fn pista_en_reproduccion(&self) -> Option<Pista> {
        let sonando = self.gestor.esta_reproduciendo() || self.gestor.esta_pausado();
        self.gestor.pista_actual().filter(|_| sonando).cloned()
    }

    pub fn puntos_de(&self, pista: &Pista) -> PuntosDeMezcla {
        self.lista.puntos_de(pista)
    }

    /// Ficha guardada del archivo (vacia si todavia no tiene datos).
    pub fn ficha_de(&self, pista: &Pista) -> FichaDeCancion {
        self.repositorio_de_canciones.buscar(pista.archivo()).cloned().unwrap_or_default()
    }

    /// Segundos que la pista se mezcla con la siguiente, o None si es la
    /// ultima y no se repite la lista.
    pub fn segundos_de_mezcla_tras(&self, pista: &Pista) -> Option<f64> {
        let indice = self.lista.indice_de(pista)?;
        self.transicion_tras(indice).map(|transicion| transicion.segundos)
    }

    /// Analisis de mezcla de una pista, si ya esta calculado.
    pub fn datos_de_mezcla(&self, pista: &Pista) -> Option<DatosDeMezcla> {
        self.datos_de_mezcla.get(pista.archivo()).copied()
    }

    fn crossfade_inteligente(&self) -> bool {
        self.repositorio_de_preferencias.crossfade_inteligente()
    }

    fn nivelar_volumen(&self) -> bool {
        self.repositorio_de_preferencias.nivelar_volumen()
    }

    pub fn objetivo_de_sonoridad(&self) -> ObjetivoDeSonoridad {
        self.repositorio_de_preferencias.objetivo_de_sonoridad()
    }

    fn transicion_tras(&self, indice: usize) -> Option<Transicion> {
        transicion_tras_indice(
            &self.lista,
            &self.datos_de_mezcla,
            &self.repositorio_de_canciones,
            self.crossfade_inteligente(),
            indice,
            &archivo_existe,
        )
    }

    /// La proxima pista reproducible despues de la actual y las que se
    /// saltearon porque su archivo no existe (los marcadores se saltean sin
    /// contarlos).
    fn proxima_reproducible(&self) -> (Option<usize>, Vec<usize>) {
        let (proxima, salteadas) = self.lista.proxima_disponible(self.lista.indice_actual(), archivo_existe);
        let salteadas = salteadas
            .into_iter()
            .filter(|&indice| self.lista.pista_en(indice).is_some_and(|pista| !es_marcador(pista.archivo())))
            .collect();
        (proxima, salteadas)
    }

    /// Avisa que se saltearon pistas porque su archivo no existe.
    fn avisar_salteadas(&mut self, salteadas: &[usize]) {
        let nombres: Vec<String> = salteadas
            .iter()
            .filter_map(|&indice| self.lista.pista_en(indice))
            .filter(|pista| !es_marcador(pista.archivo()))
            .map(|pista| etiqueta(&self.lista, pista))
            .collect();
        if nombres.is_empty() {
            return;
        }
        log::warn!("Se saltearon pistas cuyo archivo no existe: {}", nombres.join(", "));
        self.mostrar_estado(&tr!("Se salteo {} porque el archivo no existe (Validar ofrece quitarlo de la lista)", nombres.join(", ")));
    }

    /// Precalcula lo que ahorra cada mezcla y lo que falta sonar desde cada
    /// posicion, con las transiciones ya calculadas para la columna Mezcla.
    fn precalcular_totales(&mut self, transiciones: &[Option<Transicion>]) {
        let ultima = self.duraciones.len().saturating_sub(1);
        self.ahorros_de_mezcla = transiciones
            .iter()
            .enumerate()
            .map(|(indice, transicion)| match transicion {
                Some(transicion) if indice < ultima => segundos_ahorrados(transicion),
                _ => 0.0,
            })
            .collect();
        let mut netos = vec![0.0; self.duraciones.len() + 1];
        for (indice, pista) in self.lista.pistas().iter().enumerate().rev() {
            let puntos = self.lista.puntos_de(pista);
            // Lo que suena del tema, entre su inicio y su fin.
            let audible = self.duraciones[indice]
                .map_or(0.0, |duracion| (duracion - puntos.recorte_inicial() - puntos.recorte_final()).max(0.0));
            netos[indice] = netos[indice + 1] + audible - self.ahorros_de_mezcla[indice];
        }
        self.netos_desde = netos;
    }

    /// Activa o desactiva el crossfade inteligente.
    pub fn cambiar_crossfade_inteligente(&mut self, activo: bool) {
        self.repositorio_de_preferencias.guardar_crossfade_inteligente(activo);
        self.con_ventana(|ventana| ventana.set_crossfade_inteligente(activo));
        self.pedir_datos_de_mezcla();
        self.refrescar_filas();
        self.mostrar_estado(&if activo {
            tr!("Crossfade inteligente: saltea silencios, adapta la duracion, usa curvas de potencia y alinea los golpes")
        } else {
            tr!("Crossfade inteligente desactivado: se mezclan los segundos de intro y outro")
        });
    }

    /// Activa o desactiva mantener la pista que suena en el centro de la lista.
    fn cambiar_desplazar(&mut self, activo: bool) {
        self.repositorio_de_preferencias.guardar_desplazar(activo);
        self.con_ventana(|ventana| ventana.set_desplazar(activo));
        self.fila_centrada = None;
        self.centrar_pista_actual();
    }

    /// Con "Desplazar" activo, centra en la lista la pista que suena cuando
    /// cambia de tema (o de posicion).
    fn centrar_pista_actual(&mut self) {
        if !self.repositorio_de_preferencias.desplazar() || !self.gestor.hay_pista_cargada() {
            return;
        }
        let Some(indice) = self.lista.indice_actual() else { return };
        if self.fila_centrada != Some(indice) {
            self.fila_centrada = Some(indice);
            self.con_ventana(|ventana| ventana.invoke_centrar_fila(indice as i32));
        }
    }

    /// Activa o desactiva la nivelacion de volumen.
    pub fn cambiar_nivelar_volumen(&mut self, activo: bool) {
        self.repositorio_de_preferencias.guardar_nivelar_volumen(activo);
        self.con_ventana(|ventana| ventana.set_nivelar_volumen(activo));
        self.pedir_datos_de_mezcla();
        self.aplicar_ganancias();
        let mensaje = if activo {
            tr!("Nivelar volumen: cada tema se lleva a {}", self.objetivo_de_sonoridad().descripcion())
        } else {
            tr!("Nivelar volumen desactivado")
        };
        self.mostrar_estado(&mensaje);
    }

    /// Cambia la sonoridad a la que se nivela; rige al instante si la
    /// nivelacion esta activa.
    pub fn cambiar_objetivo_de_sonoridad(&mut self, objetivo: ObjetivoDeSonoridad) {
        self.repositorio_de_preferencias.guardar_objetivo_de_sonoridad(objetivo);
        self.con_ventana(|ventana| ventana.set_objetivo_lufs(objetivo.indice() as i32));
        self.aplicar_ganancias();
        let mensaje = if self.nivelar_volumen() {
            tr!("Nivelar volumen: cada tema se lleva a {}", objetivo.descripcion())
        } else {
            tr!("Objetivo de nivelacion: {} LUFS (active Nivelar volumen para aplicarlo)", objetivo.lufs())
        };
        self.mostrar_estado(&mensaje);
    }

    /// Pide el analisis de los temas de la lista que todavia no lo tienen: de
    /// el salen sus puntos automaticos, el crossfade inteligente y la
    /// nivelacion.
    fn pedir_datos_de_mezcla(&mut self) {
        let faltantes: Vec<PathBuf> = self
            .lista
            .pistas()
            .iter()
            .map(|pista| pista.archivo().to_path_buf())
            .filter(|archivo| !es_anuncio_hth(archivo) && !es_marcador(archivo))
            .filter(|archivo| !self.datos_de_mezcla.contains_key(archivo) && !self.analisis_pedidos.contains(archivo))
            .collect();
        self.analisis_pedidos.extend(faltantes.iter().cloned());
        analizar_mezcla_en_segundo_plano(faltantes);
        self.refrescar_analisis_de_mezcla();
    }

    /// Guarda el analisis de un tema (None si no se pudo hacer) y le aplica
    /// los puntos automaticos.
    fn recibir_datos_de_mezcla(&mut self, archivo: PathBuf, datos: Option<DatosDeMezcla>) {
        self.analisis_pedidos.remove(&archivo);
        // Si el panel esperaba la onda de este tema, ya esta en la cache.
        if self.onda_del_detalle.borrow().as_deref() == Some(archivo.as_path()) && self.sin_barras() {
            *self.onda_del_detalle.borrow_mut() = None;
            self.refrescar_detalles();
        }
        if let Some(datos) = datos {
            self.aplicar_puntos_automaticos(&archivo, &datos);
            self.datos_de_mezcla.insert(archivo, datos);
            if self.nivelar_volumen() {
                self.aplicar_ganancias();
            }
        }
        self.filas_por_refrescar = true;
    }

    /// Completa los puntos de un tema con los que propone su analisis, sin
    /// tocar los manuales, y los guarda si cambiaron.
    /// Vuelve a aplicar los puntos automaticos de un tema ya analizado (por
    /// ejemplo, al cambiar su genero). Devuelve si estaba analizado.
    fn reaplicar_puntos_automaticos(&mut self, archivo: &Path) -> bool {
        let Some(datos) = self.datos_de_mezcla.get(archivo).copied() else {
            return false;
        };
        self.aplicar_puntos_automaticos(archivo, &datos);
        self.filas_por_refrescar = true;
        true
    }

    fn aplicar_puntos_automaticos(&mut self, archivo: &Path, datos: &DatosDeMezcla) {
        // Los tipos sin mezcla suenan sin mezcla: del analisis solo se
        // toman el inicio y el fin (la intro, la outro y la ventana quedan
        // en lo que fije el usuario).
        let reservado = suena_sin_mezcla(&self.repositorio_de_canciones, &Pista::new(archivo));
        let automaticos =
            if reservado { datos.puntos_automaticos().solo_recortes() } else { datos.puntos_automaticos() };
        let ficha = self.repositorio_de_canciones.buscar(archivo).cloned();
        let mut actuales = ficha.as_ref().map(|ficha| ficha.puntos).unwrap_or_default();
        if ficha.as_ref().is_some_and(|ficha| ficha.origen_por_resolver) {
            // Ficha de una version anterior: los puntos eran sugeridos si
            // coinciden con los automaticos (o si no habia ninguno).
            let parecido = |a: f64, b: f64| (a - b).abs() <= TOLERANCIA_DE_PUNTOS_SUGERIDOS;
            let sugeridos = (actuales.intro() == 0.0 && actuales.outro() == 0.0)
                || (parecido(actuales.intro(), automaticos.intro()) && parecido(actuales.outro(), automaticos.outro()));
            actuales = actuales.como_manuales(!sugeridos);
        }
        let puntos = actuales.completar_con(&automaticos);
        let duracion = Some(datos.duracion).filter(|duracion| *duracion > 0.0);
        if puntos.validar_en(duracion).is_err() {
            return;
        }
        if ficha.is_none_or(|ficha| ficha.puntos != puntos || ficha.origen_por_resolver) {
            self.repositorio_de_canciones.guardar_puntos(archivo, puntos);
        }
        let pista = Pista::new(archivo);
        if self.lista.indice_de(&pista).is_some() && self.lista.puntos_de(&pista) != puntos {
            let _ = self.lista.definir_puntos(&pista, puntos);
        }
    }

    /// Pasa al gestor el inicio y el fin de cada tema de la lista.
    fn aplicar_recortes(&mut self) {
        let recortes: HashMap<Pista, (f64, f64)> = self
            .lista
            .pistas()
            .iter()
            .map(|pista| {
                let puntos = self.lista.puntos_de(pista);
                (pista.clone(), (puntos.recorte_inicial(), puntos.recorte_final()))
            })
            .filter(|(_, (inicial, final_))| *inicial > 0.0 || *final_ > 0.0)
            .collect();
        self.gestor.definir_recortes(recortes);
    }

    /// Pasa al gestor la duracion guardada de cada tema, para que al cargarlo
    /// no tenga que sondear el archivo en el hilo de la interfaz.
    fn aplicar_duraciones(&mut self) {
        let duraciones: HashMap<Pista, f64> = self
            .lista
            .pistas()
            .iter()
            .filter_map(|pista| {
                let duracion = self.repositorio_de_canciones.buscar(pista.archivo())?.duracion?;
                Some((pista.clone(), duracion))
            })
            .collect();
        self.gestor.definir_duraciones(duraciones);
    }

    /// Pasa al gestor la ganancia de cada elemento de la lista: la de la
    /// nivelacion, si esta activa, y la del maximo volumen para los avisos
    /// (Comercial, Publicidad y Anuncio), siempre.
    fn aplicar_ganancias(&mut self) {
        let (objetivo, nivelar) = (self.objetivo_de_sonoridad(), self.nivelar_volumen());
        let ganancias: HashMap<Pista, f64> = self
            .lista
            .pistas()
            .iter()
            .filter_map(|pista| {
                let a_volumen_maximo = self.repositorio_de_canciones.tipo_de(pista.archivo()).a_volumen_maximo();
                if !nivelar && !a_volumen_maximo {
                    return None;
                }
                let datos = self.datos_de_mezcla.get(pista.archivo())?;
                Some((pista.clone(), datos.ganancia(objetivo, nivelar, a_volumen_maximo)))
            })
            .collect();
        self.gestor.definir_ganancias(ganancias);
    }

    /// Muestra cuantos temas faltan analizar.
    fn refrescar_analisis_de_mezcla(&self) {
        let pendientes = self.analisis_pedidos.len();
        let texto = if pendientes > 0 { tr!("Analizando {pendientes} tema(s)...", pendientes = pendientes) } else { String::new() };
        self.con_ventana(|ventana| ventana.set_analisis_de_mezcla(texto.into()));
    }

    /// Posicion donde entran las pistas nuevas: despues de la seleccionada.
    fn punto_de_insercion(&self) -> usize {
        self.ancla_de_seleccion
            .filter(|indice| self.seleccion.contains(indice))
            .map_or(self.lista.tamano(), |indice| indice + 1)
    }

    /// Inserta archivos en una posicion (desde 0) o, con None, en el punto
    /// de insercion. Los temas recuperan los puntos guardados de usos
    /// anteriores; etiquetas, duracion y puntos automaticos se calculan en
    /// segundo plano. El selector queda en la ultima pista insertada.
    pub fn agregar_pistas_en(&mut self, archivos: Vec<PathBuf>, posicion: Option<usize>) -> usize {
        self.insertar_pistas(archivos, posicion, false).len()
    }

    /// Como `agregar_pistas_en`, pero devuelve las pistas que eran nuevas.
    /// Con `copias`, tambien entran los archivos que ya estan en la lista.
    fn insertar_pistas(&mut self, archivos: Vec<PathBuf>, posicion: Option<usize>, copias: bool) -> Vec<Pista> {
        // Los de tipo Efecto solo pueden estar en la botonera.
        let (archivos, efectos): (Vec<PathBuf>, Vec<PathBuf>) =
            archivos.into_iter().partition(|archivo| !es_solo_para_la_botonera(&self.repositorio_de_canciones, archivo));
        let posicion = posicion.unwrap_or_else(|| self.punto_de_insercion());
        let nuevas =
            if copias { self.lista.insertar_copias(posicion, archivos) } else { self.lista.insertar(posicion, archivos) };
        let mut a_analizar = Vec::with_capacity(nuevas.len());
        for pista in &nuevas {
            if let Some(ficha) = self.repositorio_de_canciones.buscar(pista.archivo()) {
                let _ = self.lista.definir_puntos(pista, ficha.puntos);
            }
            a_analizar.push(pista.archivo().to_path_buf());
        }
        analizar_en_segundo_plano(a_analizar);
        self.pedir_datos_de_mezcla();
        self.aplicar_ganancias();
        if let Some(ultima) = nuevas.last().and_then(|pista| self.lista.indice_de(pista)) {
            self.seleccionar_solo(ultima);
            self.con_ventana(|ventana| ventana.invoke_mostrar_fila(ultima as i32));
        }
        self.guardar_lista();
        self.refrescar_interfaz();
        let mut estado = tr!("{} pista(s) en la lista", self.lista.tamano());
        if !efectos.is_empty() {
            estado.push_str(&tr!("; {} archivo(s) de tipo Efecto quedaron afuera (solo van en la botonera)", efectos.len()));
        }
        self.mostrar_estado(&estado);
        nuevas
    }

    /// Agrega archivos en el punto de insercion y resume en la barra de
    /// estado cuantos entraron y por que no entraron los demas. `origen`
    /// completa el mensaje (por ejemplo, " de D:\\musica").
    fn agregar_y_resumir(&mut self, archivos: Vec<PathBuf>, origen: &str) -> Vec<Pista> {
        let total = archivos.len();
        let efectos = archivos.iter().filter(|archivo| es_solo_para_la_botonera(&self.repositorio_de_canciones, archivo)).count();
        let nuevas = self.insertar_pistas(archivos, None, false);
        let ya_estaban = total.saturating_sub(efectos + nuevas.len());
        let mut resumen = tr!("Se agregaron {} de {total} archivo(s){origen}", nuevas.len(), total = total, origen = origen);
        let mut motivos = Vec::new();
        if ya_estaban > 0 {
            motivos.push(tr!("{ya_estaban} ya estaban en la lista", ya_estaban = ya_estaban));
        }
        if efectos > 0 {
            motivos.push(tr!("{efectos} de tipo Efecto (solo van en la botonera)", efectos = efectos));
        }
        if !motivos.is_empty() {
            resumen.push_str(&format!(": {}", motivos.join(", ")));
        }
        log::info!("{resumen}");
        self.mostrar_estado(&resumen);
        nuevas
    }

    /// Quita de la lista los temas de tipo Efecto (por ejemplo, si el
    /// genero llego de las etiquetas despues de agregarlos). El que esta
    /// cargado para sonar no se toca.
    fn sacar_de_la_lista_los_efectos(&mut self) {
        let cargada = self.gestor.pista_actual().filter(|_| self.gestor.hay_pista_cargada()).cloned();
        let posiciones: Vec<usize> = (0..self.lista.tamano())
            .filter(|&indice| {
                self.lista.pista_en(indice).is_some_and(|pista| {
                    Some(pista) != cargada.as_ref() && es_solo_para_la_botonera(&self.repositorio_de_canciones, pista.archivo())
                })
            })
            .collect();
        if posiciones.is_empty() {
            return;
        }
        let quitadas = self.quitar_posiciones(&posiciones);
        self.mostrar_estado(&tr!("Se quitaron {quitadas} pista(s) de tipo Efecto: solo pueden estar en la botonera.", quitadas = quitadas));
    }

    /// Si el archivo es de tipo Efecto: solo puede estar en la botonera.
    pub fn es_solo_para_la_botonera(&self, archivo: &Path) -> bool {
        es_solo_para_la_botonera(&self.repositorio_de_canciones, archivo)
    }

    /// El anuncio HTH se arma solo: no tiene datos que editar.
    pub fn validar_que_no_sea_el_anuncio_hth(&self, pista: &Pista) -> Result<(), String> {
        if es_anuncio_hth(pista.archivo()) {
            return Err(tr!("El anuncio de hora, temperatura y humedad se arma solo al salir al aire: no tiene datos ni puntos que editar.").into());
        }
        Ok(())
    }

    /// Un tema que esta en la lista no puede pasar a tipo Efecto.
    pub fn validar_tipo(&self, pista: &Pista, tipo: Option<TipoDeElemento>) -> Result<(), String> {
        self.validar_que_no_sea_el_anuncio_hth(pista)?;
        if tipo.solo_para_la_botonera() && self.lista.indice_de(pista).is_some() {
            return Err(tr!("{} esta en la lista y el tipo Efecto es solo para la botonera: quitelo de la lista antes de darle ese tipo.", pista.nombre()));
        }
        Ok(())
    }

    // Validacion de la lista

    /// Pistas de la lista cuyo archivo ya no existe.
    fn pistas_faltantes(&self) -> Vec<Pista> {
        self.lista.pistas().iter().filter(|pista| !pista.archivo().is_file()).cloned().collect()
    }

    /// Temas repetidos de la lista (ver `buscar_duplicados`). Los titulos y
    /// artistas salen de la ficha de cada tema; si no la tiene, de
    /// `etiquetas` (leidas al agregarlo) o del nombre del archivo. Los
    /// archivos que no existen no se comparan y la pista que suena nunca se
    /// propone para quitar. Lo que no es un Tema (comercial, anuncio,
    /// publicidad, seco) puede repetirse: no se compara.
    fn temas_repetidos(
        &self,
        nuevas: &HashSet<Pista>,
        solo_nuevas: bool,
        etiquetas: &HashMap<PathBuf, Analisis>,
    ) -> Vec<Duplicado> {
        // Las bases del anuncio HTH pueden repetirse, como los reservados.
        let carpeta_de_bases = self.carpeta_de_bases_en_la_lista();
        let temas: Vec<(Pista, String)> = self
            .lista
            .pistas()
            .iter()
            .filter(|pista| !es_base_hth(pista.archivo(), carpeta_de_bases.as_deref()))
            .filter(|pista| pista.archivo().is_file() && !self.puede_repetirse(pista, etiquetas))
            .filter_map(|pista| Some((pista.clone(), self.clave_de(pista, etiquetas)?)))
            .collect();
        buscar_duplicados(&temas, nuevas, self.gestor.pista_actual(), solo_nuevas)
    }

    /// Si la pista puede estar repetida en la lista: un marcador, el anuncio
    /// HTH o lo que no es un Tema (segun su ficha o, si todavia no la tiene,
    /// el genero de las etiquetas leidas al agregarlo).
    fn puede_repetirse(&self, pista: &Pista, etiquetas: &HashMap<PathBuf, Analisis>) -> bool {
        if es_anuncio_hth(pista.archivo()) || es_marcador(pista.archivo()) {
            return true;
        }
        match self.repositorio_de_canciones.buscar(pista.archivo()) {
            Some(ficha) => ficha.tipo.sin_mezcla(),
            None => etiquetas
                .get(pista.archivo())
                .and_then(|analisis| TipoDeElemento::deducido_del_genero(analisis.genero.as_deref()))
                .sin_mezcla(),
        }
    }

    fn clave_de(&self, pista: &Pista, etiquetas: &HashMap<PathBuf, Analisis>) -> Option<String> {
        let ficha = self.repositorio_de_canciones.buscar(pista.archivo());
        let leidas = etiquetas.get(pista.archivo());
        let titulo = ficha
            .and_then(|ficha| ficha.titulo.clone())
            .or_else(|| leidas.and_then(|analisis| analisis.titulo.clone()))
            .or_else(|| pista.archivo().file_stem().map(|nombre| nombre.to_string_lossy().into_owned()))?;
        let artista = ficha
            .and_then(|ficha| ficha.artista.clone())
            .or_else(|| leidas.and_then(|analisis| analisis.artista.clone()))
            .unwrap_or_default();
        clave_de_tema(&titulo, &artista)
    }

    /// Texto de la pregunta por un tema repetido, o None si alguna de las
    /// dos pistas ya no esta en la lista.
    fn describir_duplicado(&self, duplicado: &Duplicado, al_agregar: bool) -> Option<String> {
        let describir = |pista: &Pista| -> Option<String> {
            let posicion = self.lista.indice_de(pista)? + 1;
            let duracion = self.duracion_de(pista).map_or_else(|| tr!("duracion desconocida").into(), formatear_duracion);
            Some(format!("# {posicion}  {}  ({duracion})", pista.archivo().display()))
        };
        let (conservar, quitar) = (describir(&duplicado.conservar)?, describir(&duplicado.quitar)?);
        let ficha = self.repositorio_de_canciones.buscar(duplicado.conservar.archivo());
        let titulo = ficha.and_then(|ficha| ficha.titulo.clone()).unwrap_or_else(|| duplicado.conservar.nombre());
        let artista = ficha.and_then(|ficha| ficha.artista.clone()).map_or_else(String::new, |artista| format!(" de {artista}"));
        let (rotulo_conservar, rotulo_quitar, pregunta) = if al_agregar {
            (tr!("Ya estaba"), tr!("Recien agregada"), tr!("¿Quitar de la lista la pista recien agregada?"))
        } else {
            (tr!("Se conserva"), tr!("Repetida"), tr!("¿Quitar de la lista la pista repetida?"))
        };
        Some(tr!("\"{titulo}\"{artista} esta mas de una vez en la lista.\n\n{rotulo_conservar}: {conservar}\n{rotulo_quitar}: {quitar}\n\n{pregunta}\n(Cancelar termina la revision.)", titulo = titulo, artista = artista, rotulo_conservar = rotulo_conservar, conservar = conservar, rotulo_quitar = rotulo_quitar, quitar = quitar, pregunta = pregunta))
    }

    /// Inserta en la lista el archivo de un boton de la hoja a la vista. Un
    /// comercial, anuncio o publicidad entra aunque ya este en la lista: puede
    /// salir en mas de una tanda.
    fn insertar_efecto_en_la_lista(&mut self, boton: usize) {
        let Some(archivo) = self.repositorio_de_botonera.archivo_del_boton(boton).map(Path::to_path_buf) else {
            return;
        };
        if es_solo_para_la_botonera(&self.repositorio_de_canciones, &archivo) {
            self.mostrar_estado(&tr!("{} es de tipo Efecto: solo puede estar en la botonera", nombre_de_archivo(&archivo)));
            return;
        }
        let reservado = self.puede_repetirse(&Pista::new(&archivo), &HashMap::new());
        if self.insertar_pistas(vec![archivo.clone()], None, reservado).is_empty() {
            self.mostrar_estado(&tr!("{} ya esta en la lista", nombre_de_archivo(&archivo)));
        }
    }

    /// Duracion de un efecto, leida una sola vez por archivo.
    fn duracion_de_efecto(&self, archivo: &Path) -> Option<f64> {
        if let Some(duracion) = self.duraciones_de_efectos.borrow().get(archivo) {
            return *duracion;
        }
        let completa = self
            .repositorio_de_canciones
            .buscar(archivo)
            .and_then(|ficha| ficha.duracion)
            .or_else(|| Decodificador::sondear_duracion(archivo).ok().flatten());
        // Si se fijo un tramo en el editor, el boton muestra lo que dura ese tramo.
        let duracion = recorte_de_efecto(&self.repositorio_de_canciones, archivo).duracion(completa);
        self.duraciones_de_efectos.borrow_mut().insert(archivo.to_path_buf(), duracion);
        duracion
    }

    /// Quita de la lista las pistas indicadas, con todas sus copias. Devuelve
    /// cuantas filas se quitaron.
    pub fn quitar_pistas(&mut self, pistas: &[Pista]) -> usize {
        let posiciones: Vec<usize> = (0..self.lista.tamano())
            .filter(|&indice| self.lista.pista_en(indice).is_some_and(|pista| pistas.contains(pista)))
            .collect();
        self.quitar_posiciones(&posiciones)
    }

    /// Lo que hace Quitar (de la interfaz) con esas posiciones: la pista que
    /// suena (o esta en pausa) no se quita, y si entre ellas hay marcadores
    /// de bloque y temas, solo se quitan los marcadores. Devuelve lo que queda
    /// por quitar y si se dejo afuera la que suena.
    fn preparar_quitar(&mut self, posiciones: &[usize]) -> (Vec<usize>, bool) {
        let suena = self.lista.indice_actual().filter(|_| self.gestor.hay_pista_cargada());
        let quitables: Vec<usize> =
            posiciones.iter().copied().filter(|&indice| Some(indice) != suena && indice < self.lista.tamano()).collect();
        let quedo_la_que_suena = suena.is_some_and(|actual| posiciones.contains(&actual));
        if quitables.is_empty() {
            if quedo_la_que_suena {
                self.mostrar_estado(&tr!("La pista que suena no se quita: detengala o pase a otra antes"));
            }
            return (Vec::new(), false);
        }
        if self.quitar_solo_los_marcadores(&quitables) {
            return (Vec::new(), false);
        }
        (quitables, quedo_la_que_suena)
    }

    /// Quita las pistas de esas posiciones (desde 0); si una de ellas es la
    /// que suena, detiene la reproduccion. Devuelve cuantas se quitaron.
    pub fn quitar_posiciones(&mut self, posiciones: &[usize]) -> usize {
        let mut posiciones: Vec<usize> = posiciones.iter().copied().filter(|&indice| indice < self.lista.tamano()).collect();
        posiciones.sort_unstable();
        posiciones.dedup();
        let suena_una = self.gestor.hay_pista_cargada() && self.lista.indice_actual().is_some_and(|actual| posiciones.contains(&actual));
        if suena_una {
            self.gestor.detener();
        }
        // De atras hacia adelante, para que las posiciones no se corran.
        for &indice in posiciones.iter().rev() {
            self.lista.quitar_en(indice);
        }
        let quitadas = posiciones.len();
        // El selector queda en el lugar de la primera quitada (ahora la
        // siguiente; o la ultima, si se quito el final) para no perderse en
        // listas largas.
        let lugar = posiciones.first().copied().zip(self.lista.tamano().checked_sub(1)).map(|(primera, ultima)| primera.min(ultima));
        match lugar {
            Some(indice) => self.seleccionar_solo(indice),
            None => self.limpiar_seleccion(),
        }
        // Lo que suena solo cambio de numero: "Desplazar" no debe llevarse la
        // lista hacia el y perder el lugar donde se estaba quitando.
        self.fila_centrada = self.lista.indice_actual();
        self.guardar_lista();
        self.refrescar_interfaz();
        if let Some(indice) = lugar {
            let centrar = self.repositorio_de_preferencias.desplazar();
            self.con_ventana(|ventana| {
                if centrar {
                    ventana.invoke_centrar_fila(indice as i32);
                } else {
                    ventana.invoke_mostrar_fila(indice as i32);
                }
            });
        }
        self.mostrar_estado(&tr!("{} pista(s) en la lista", self.lista.tamano()));
        quitadas
    }

    /// Cambia la intro/outro/overlay de una pista de la lista y la recuerda
    /// para la proxima vez que se agregue ese archivo.
    pub fn definir_puntos_de_mezcla(&mut self, pista: &Pista, puntos: PuntosDeMezcla) -> Result<(), String> {
        puntos.validar_en(self.duracion_de(pista))?;
        self.lista.definir_puntos(pista, puntos)?;
        self.repositorio_de_canciones.guardar_puntos(pista.archivo(), puntos);
        self.refrescar_filas();
        self.mostrar_estado(&tr!("{}: intro {}, outro {}, overlay {}", etiqueta(&self.lista, pista), formatear_tiempo(puntos.intro()), formatear_tiempo(puntos.outro()), formatear_ventana(puntos.overlay())));
        Ok(())
    }

    /// Trae a la lista los puntos guardados de estos archivos, por ejemplo
    /// cuando el analisis convirtio un overlay de una version anterior.
    fn sincronizar_puntos<'a>(&mut self, archivos: impl Iterator<Item = &'a Path>) {
        let mut hubo_cambios = false;
        for archivo in archivos {
            let pista = Pista::new(archivo);
            let Some(guardados) = self.repositorio_de_canciones.puntos(archivo) else {
                continue;
            };
            if self.lista.indice_de(&pista).is_some() && self.lista.puntos_de(&pista) != guardados {
                hubo_cambios |= self.lista.definir_puntos(&pista, guardados).is_ok();
            }
        }
        if hubo_cambios {
            self.refrescar_filas();
        }
    }

    /// Corrige el tipo, titulo, artista, genero o ranking de una pista.
    pub fn cambiar_datos(&mut self, pista: &Pista, cambios: CambiosDeDatos) -> FichaDeCancion {
        let cambia_el_tipo = cambios.tipo.is_some();
        let mut ficha = self.repositorio_de_canciones.cambiar_datos(pista.archivo(), cambios);
        // Lo que no es un Tema no admite efecto fijado.
        if ficha.efecto_fijado.is_some() && ficha.tipo.sin_mezcla() {
            let sin_efecto = CambiosDeDatos { efecto_fijado: Some(None), ..CambiosDeDatos::default() };
            ficha = self.repositorio_de_canciones.cambiar_datos(pista.archivo(), sin_efecto);
        }
        if cambia_el_tipo && self.reaplicar_puntos_automaticos(pista.archivo()) {
            ficha = self.ficha_de(pista);
        }
        // Un aviso suena al maximo volumen: el tipo cambia su ganancia.
        if cambia_el_tipo {
            self.aplicar_ganancias();
        }
        // Los datos se ven en la lista, y la nota tambien en los botones de efectos.
        self.refrescar_filas();
        self.refrescar_botones();
        // Con otro artista o album puede corresponder otra carátula.
        self.olvidar_caratula(pista.archivo());
        self.refrescar_detalles();
        ficha
    }

    /// Pone el selector de la lista en una posicion (desde 0). Esa pista
    /// queda marcada como la proxima del boton Siguiente.
    pub fn seleccionar_posicion(&mut self, indice: usize) -> Result<String, String> {
        if indice >= self.lista.tamano() {
            return Err(tr!("Posicion {} fuera de la lista (1 a {}).", indice + 1, self.lista.tamano()));
        }
        self.seleccionar_solo(indice);
        self.refrescar_filas();
        self.con_ventana(|ventana| ventana.invoke_mostrar_fila(indice as i32));
        if self.marcador().is_none() {
            let pista = self.lista.pista_en(indice).expect("indice validado");
            return Ok(tr!("{} es la pista actual; Siguiente sigue el orden de la lista.", etiqueta(&self.lista, pista)));
        }
        if self.marcada_para_siguiente().is_none() {
            let pista = self.lista.pista_en(indice).expect("indice validado");
            let mensaje = tr!("{} queda elegida; al aire Siguiente sigue el orden de la lista.", etiqueta(&self.lista, pista));
            self.mostrar_estado(&mensaje);
            return Ok(mensaje);
        }
        self.informar_marcador();
        Ok(self.estado_visible())
    }

    /// Ejecuta un boton de transporte y devuelve el estado que queda a la vista.
    pub fn transporte(&mut self, accion: AccionDeTransporte) -> String {
        match accion {
            AccionDeTransporte::Reproducir if self.gestor.esta_reproduciendo() => {}
            AccionDeTransporte::Pausar if !self.gestor.esta_reproduciendo() => {}
            AccionDeTransporte::Reproducir | AccionDeTransporte::Pausar | AccionDeTransporte::Alternar => {
                self.alternar_reproduccion()
            }
            AccionDeTransporte::Detener { salir_del_aire } => {
                self.detener_todo();
                if self.modo_de_operacion().al_aire() {
                    if salir_del_aire {
                        self.cambiar_modo_de_operacion(ModoDeOperacion::Editor);
                    } else {
                        self.mostrar_estado(&tr!("{} Sigue en modo AL AIRE.", self.estado_visible()));
                    }
                }
            }
            AccionDeTransporte::Siguiente => self.reproducir_siguiente(),
            AccionDeTransporte::Anterior => self.reproducir_anterior(),
        }
        self.estado_visible()
    }

    /// Cambia deslizantes y opciones como si los moviera el usuario.
    pub fn ajustar(&mut self, ajustes: Ajustes) -> Result<(), String> {
        if let Some(volumen) = ajustes.volumen {
            self.cambiar_volumen(volumen);
        }
        if let Some(fader) = ajustes.fader {
            self.cambiar_fader(fader);
        }
        if let Some(posicion) = ajustes.posicion {
            self.admitida(OperacionSobreLaSecuencia::Adelantar)?;
            self.gestor.buscar(posicion).map_err(|error| error.to_string())?;
        }
        if let Some(repetir) = ajustes.repetir {
            self.con_ventana(|ventana| ventana.set_repetir(repetir));
            self.definir_repetir(repetir);
        }
        if let Some(visible) = ajustes.botonera_visible {
            self.con_ventana(|ventana| ventana.set_botonera_visible(visible));
        }
        if let Some(libre) = ajustes.modo_libre {
            self.con_ventana(|ventana| ventana.set_modo_libre(libre));
            self.guardar_modo_libre(libre);
        }
        let de_efecto = [ajustes.bajar_musica.is_some(), ajustes.atenuacion_inteligente.is_some()];
        let valores_de_efecto = [ajustes.nivel_de_musica, ajustes.volumen_de_efecto, ajustes.anticipacion, ajustes.recuperacion, ajustes.separacion];
        if de_efecto.iter().any(|cambia| *cambia) || valores_de_efecto.iter().any(Option::is_some) {
            let actuales = self.overlay.ajustes();
            self.cambiar_ajustes_de_efecto(AjustesDeEfecto {
                atenuar: ajustes.bajar_musica.unwrap_or(actuales.atenuar),
                inteligente: ajustes.atenuacion_inteligente.unwrap_or(actuales.inteligente),
                nivel_de_musica: ajustes.nivel_de_musica.unwrap_or(actuales.nivel_de_musica),
                volumen_db: ajustes.volumen_de_efecto.unwrap_or(actuales.volumen_db),
                anticipacion: ajustes.anticipacion.unwrap_or(actuales.anticipacion),
                recuperacion: ajustes.recuperacion.unwrap_or(actuales.recuperacion),
                separacion_db: ajustes.separacion.unwrap_or(actuales.separacion_db),
            });
            self.atenuacion_por_guardar = None;
            self.repositorio_de_botonera.guardar_ajustes_de_efecto(self.overlay.ajustes());
        }
        if let Some(activo) = ajustes.crossfade_inteligente {
            self.cambiar_crossfade_inteligente(activo);
        }
        if let Some(objetivo) = ajustes.objetivo_de_sonoridad {
            self.cambiar_objetivo_de_sonoridad(objetivo);
        }
        if let Some(activo) = ajustes.nivelar_volumen {
            self.cambiar_nivelar_volumen(activo);
        }
        Ok(())
    }

    /// Resumen del estado de la ventana, para clientes externos.
    pub fn resumen(&self) -> String {
        let estado = if self.gestor.esta_deteniendo() {
            tr!("deteniendo con fundido")
        } else if self.gestor.esta_reproduciendo() {
            tr!("reproduciendo")
        } else if self.gestor.esta_pausado() {
            tr!("pausado")
        } else {
            tr!("detenido")
        };
        let mut texto = tr!("Estado: {estado}\n", estado = estado);
        let sonando = self.gestor.esta_reproduciendo() || self.gestor.esta_pausado();
        match self.gestor.pista_actual().filter(|_| self.gestor.hay_pista_cargada()) {
            Some(pista) if !sonando => {
                texto.push_str(&tr!("Ultima pista: {} (detenida)\n", etiqueta(&self.lista, pista)));
            }
            Some(pista) => {
                let total = self.gestor.duracion_total().unwrap_or(0.0);
                texto.push_str(&tr!("Pista actual: {} ({} / {})\n", etiqueta(&self.lista, pista), formatear_minutos(self.gestor.tiempo_transcurrido()), formatear_minutos(total)));
            }
            None => texto.push_str(&tr!("Pista actual: ninguna\n")),
        }
        if self.gestor.hay_transicion_en_curso() {
            match self.gestor.detalle_de_transicion() {
                "" => texto.push_str(&tr!("Mezcla en curso: si\n")),
                detalle => texto.push_str(&tr!("Mezcla en curso: {detalle}\n", detalle = detalle)),
            }
        }
        match self.marcador().and_then(|indice| self.lista.pista_en(indice)) {
            Some(pista) => texto.push_str(&tr!("Proxima con Siguiente (selector): {}\n", etiqueta(&self.lista, pista))),
            None => {
                let natural = self.lista.indice_siguiente(self.lista.indice_actual()).and_then(|i| self.lista.pista_en(i));
                let natural = natural.map_or_else(|| "ninguna".to_string(), |pista| etiqueta(&self.lista, pista));
                texto.push_str(&tr!("Proxima con Siguiente: {natural}\n", natural = natural));
            }
        }
        let visible = self.ventana.upgrade().is_some_and(|ventana| ventana.get_botonera_visible());
        let efecto = self.overlay.ajustes();
        let (total, restante, sin_duracion) = self.totales();
        texto.push_str(&tr!("Duracion total de la lista: {} | Restante: {}{}\n", formatear_duracion(total), restante.map_or_else(|| "-".into(), formatear_duracion), if sin_duracion > 0 { format!(" ({sin_duracion} pista(s) sin duracion)") } else { String::new() }));
        texto.push_str(&tr!("Modo: {}\n", if self.modo_de_operacion().al_aire() { tr!("AL AIRE") } else { tr!("EDITOR") }));
        texto.push_str(&tr!("Volumen: {} % | Fader: {} s | Repetir lista: {} | Pistas en la lista: {}\n", (self.volumen_seleccionado * 100.0).round(), self.fader(), si_no(self.lista.repite()), self.lista.tamano()));
        texto.push_str(&tr!("Crossfade inteligente: {} | Nivelar volumen: {} (a {} LUFS){}\n", si_no(self.crossfade_inteligente()), si_no(self.nivelar_volumen()), self.objetivo_de_sonoridad().lufs(), match self.analisis_pedidos.len() {
                0 => String::new(),
                pendientes => tr!(" | Analizando {pendientes} tema(s)", pendientes = pendientes),
            }));
        texto.push_str(&tr!("Botonera visible: {} | Modo libre: {} | Volumen de los efectos: {:+.1} dB | Bajar la musica durante el efecto: {} | Ajuste inteligente: {} | Musica al {} % (sin ajuste inteligente) | Atenua en: {:.2} s | Recuperacion: {:.1} s\n", si_no(visible), si_no(self.repositorio_de_botonera.modo() == ModoDeOverlay::Libre), efecto.volumen_db, si_no(efecto.atenuar), si_no(efecto.inteligente), (efecto.nivel_de_musica * 100.0).round(), efecto.anticipacion, efecto.recuperacion));
        if let Some(plan) = self.overlay.ultimo_plan() {
            texto.push_str(&tr!("Ultimo efecto: {}\n", plan.descripcion()));
        }
        let repositorio = &self.repositorio_de_botonera;
        // Numero de la hoja (desde 1) seguido de su nombre propio, si tiene.
        let hoja_con_nombre = |hoja: usize| match repositorio.nombre_de_hoja(hoja) {
            Some(nombre) => format!("{} «{nombre}»", hoja + 1),
            None => (hoja + 1).to_string(),
        };
        let actual = repositorio.hoja_actual();
        texto.push_str(&tr!("Hoja de efectos a la vista: {} de {}\n", hoja_con_nombre(actual), repositorio.cantidad_de_hojas()));
        for hoja in (0..repositorio.cantidad_de_hojas()).filter(|hoja| *hoja != repositorio.hoja_actual()) {
            let asignados: Vec<String> = repositorio
                .efectos()
                .filter(|(de_la_hoja, _, _)| *de_la_hoja == hoja)
                .map(|(_, boton, archivo)| format!("{boton}: {}", nombre_de_archivo(archivo)))
                .collect();
            let detalle = if asignados.is_empty() { "vacia".to_string() } else { asignados.join(", ") };
            texto.push_str(&tr!("  Hoja {}: {detalle}\n", hoja_con_nombre(hoja), detalle = detalle));
        }
        texto.push_str(&tr!("Botones de la hoja {}:\n", hoja_con_nombre(actual)));
        for indice in 0..CANTIDAD_DE_BOTONES {
            let archivo = self.repositorio_de_botonera.archivo_del_boton(indice);
            let marca = match archivo {
                Some(archivo) if self.overlay.en_espera() == Some(archivo) => "  [en espera]",
                Some(archivo) if self.overlay.sonando() == Some(archivo) => "  [sonando]",
                _ => "",
            };
            let descripcion = archivo.map_or_else(|| "(vacio)".to_string(), |ruta| ruta.display().to_string());
            texto.push_str(&format!("  {indice}: {descripcion}{marca}\n"));
        }
        if let Some(pista) = self.pista_en_edicion() {
            texto.push_str(&tr!("Editor abierto con: {}\n", etiqueta(&self.lista, pista)));
        }
        texto.push_str(&tr!("Barra de estado: {}", self.estado_visible()));
        texto
    }

    /// Quita la pista de la lista y borra su ficha (puntos de mezcla, datos
    /// y ranking) del catalogo.
    /// Por que esa pista no se puede descatalogar: los marcadores de bloque
    /// fijo y los anuncios y bases HTH son piezas del programa, no temas del
    /// catalogo.
    fn motivo_para_no_descatalogar(&self, pista: &Pista) -> Option<String> {
        if marcador_de(pista.archivo()).is_some() {
            return Some(tr!("Los marcadores de bloque fijo no se descatalogan: use Quitar de la lista.").into());
        }
        let carpeta_de_bases = self.carpeta_de_bases_en_la_lista();
        if es_anuncio_hth(pista.archivo()) || es_base_hth(pista.archivo(), carpeta_de_bases.as_deref()) {
            return Some(tr!("Los anuncios y bases HTH no se descatalogan: use Quitar de la lista.").into());
        }
        None
    }

    fn descatalogar(&mut self, pista: &Pista) {
        let nombre = etiqueta(&self.lista, pista);
        self.quitar_pistas(std::slice::from_ref(pista));
        let mensaje = if self.repositorio_de_canciones.olvidar(pista.archivo()) {
            tr!("{nombre} descatalogada: se quito de la lista y se borraron sus datos", nombre = nombre)
        } else {
            tr!("{nombre} se quito de la lista (no tenia datos guardados)", nombre = nombre)
        };
        self.mostrar_estado(&mensaje);
    }

    fn limpiar_lista(&mut self) {
        if !self.admitida_o_avisar(OperacionSobreLaSecuencia::Limpiar) {
            return;
        }
        self.gestor.detener();
        self.lista.limpiar();
        self.limpiar_seleccion();
        self.guardar_lista();
        self.refrescar_interfaz();
        self.mostrar_estado(&tr!("Lista vacia"));
    }

    fn clic_en_fila(&mut self, indice: usize, control: bool, mayusculas: bool) {
        if indice >= self.lista.tamano() {
            return;
        }
        match self.ancla_de_seleccion {
            Some(ancla) if mayusculas => {
                self.seleccion = (ancla.min(indice)..=ancla.max(indice)).collect();
            }
            _ if control => {
                if !self.seleccion.remove(&indice) {
                    self.seleccion.insert(indice);
                }
                self.ancla_de_seleccion = Some(indice);
            }
            _ => self.seleccionar_solo(indice),
        }
        self.refrescar_filas();
        self.informar_marcador();
    }

    /// Pista marcada con el selector como proxima del boton Siguiente: la
    /// ultima fila elegida, si sigue seleccionada y no es la que suena.
    fn marcador(&self) -> Option<usize> {
        self.ancla_de_seleccion.filter(|indice| {
            self.seleccion.contains(indice) && *indice < self.lista.tamano() && Some(*indice) != self.lista.indice_actual()
        })
    }

    fn informar_marcador(&self) {
        let Some(pista) = self.marcada_para_siguiente().and_then(|indice| self.lista.pista_en(indice)) else {
            return;
        };
        let como = match self.fader() {
            0 => tr!("sin mezcla"),
            segundos => tr!("mezcla de {segundos} s", segundos = segundos),
        };
        self.mostrar_estado(&tr!("Proxima con Siguiente: {} ({como})", etiqueta(&self.lista, pista), como = como));
    }

    fn seleccionar_solo(&mut self, indice: usize) {
        self.seleccion = BTreeSet::from([indice]);
        self.ancla_de_seleccion = Some(indice);
    }

    fn limpiar_seleccion(&mut self) {
        self.seleccion.clear();
        self.ancla_de_seleccion = None;
    }

    /// Duracion conocida de una pista: la de la que suena o la de su ficha.
    pub fn duracion_de(&self, pista: &Pista) -> Option<f64> {
        if self.gestor.pista_actual() == Some(pista) {
            if let Some(duracion) = self.gestor.duracion_total() {
                return Some(duracion);
            }
        }
        if es_anuncio_hth(pista.archivo()) {
            return self.duracion_del_anuncio_hth;
        }
        // Los marcadores no suenan: no suman a la duracion de la lista.
        if es_marcador(pista.archivo()) {
            return Some(0.0);
        }
        self.repositorio_de_canciones.buscar(pista.archivo()).and_then(|ficha| ficha.duracion)
    }

    /// Reproducir (doble clic o menu) una fila: al aire, mientras suena
    /// una pista no se salta a otra.
    fn reproducir_desde_la_lista(&mut self, indice: usize) {
        if self.gestor.hay_pista_cargada() && !self.admitida_o_avisar(OperacionSobreLaSecuencia::SaltarA) {
            return;
        }
        self.reproducir_pista(indice);
    }

    /// Pista marcada con el selector a la que pasa Siguiente: al aire no hay
    /// (Siguiente sigue el orden de la lista).
    fn marcada_para_siguiente(&self) -> Option<usize> {
        self.marcador().filter(|_| self.modo_de_operacion().admite(OperacionSobreLaSecuencia::SiguienteMarcada))
    }

    fn reproducir_pista(&mut self, indice: usize) {
        if indice < self.lista.tamano() {
            self.lista.seleccionar_indice(indice);
            self.reproducir_pista_actual();
        }
    }

    /// Pausa al instante (sin fader); reanuda o empieza segun corresponda.
    fn alternar_reproduccion(&mut self) {
        if self.gestor.esta_reproduciendo() {
            self.gestor.pausar();
        } else if self.gestor.esta_pausado() {
            if let Err(error) = self.gestor.reanudar() {
                self.errores.push(error.to_string());
            }
        } else if let Some(indice) = self.marcador() {
            self.reproducir_pista(indice);
        } else if self.lista.pista_actual().is_some() {
            self.reproducir_pista_actual();
        } else if !self.lista.esta_vacia() {
            self.reproducir_pista(0);
        } else {
            self.mostrar_estado(&tr!("La lista esta vacia"));
        }
        self.refrescar_interfaz();
    }

    /// Pasa a la pista marcada con el selector o, si no hay, a la que sigue en
    /// la lista. Con musica sonando y fader mayor que 0, la mezcla durante
    /// esos segundos; si no, la siguiente arranca enseguida.
    fn reproducir_siguiente(&mut self) {
        // La marcada con el selector, o la proxima cuyo archivo exista.
        let (proxima, salteadas) = self.proxima_reproducible();
        let (destino, salteadas) = match self.marcada_para_siguiente() {
            Some(marcada) => (Some(marcada), Vec::new()),
            None => (proxima, salteadas),
        };
        let Some(destino) = destino else {
            self.mostrar_estado(&if salteadas.is_empty() {
                tr!("No hay pista siguiente")
            } else {
                tr!("No hay pista siguiente: los archivos que siguen en la lista no existen")
            });
            return;
        };
        self.pasar_a(destino);
    }

    /// Pasa a la pista de esa posicion como Siguiente: con musica sonando y
    /// fader mayor que 0, la mezcla durante esos segundos (o, hacia o desde
    /// un tipo sin mezcla, baja lo que suena y despues arranca); si no,
    /// arranca enseguida.
    fn pasar_a(&mut self, destino: usize) {
        // La marcada a mano que ya no existe no corta lo que suena.
        if let Some(pista) = self.lista.pista_en(destino).filter(|pista| !archivo_existe(pista)) {
            let aviso = if es_marcador(pista.archivo()) {
                tr!("No se puede pasar a un marcador de bloque fijo: no suena. Marque un tema.")
            } else {
                tr!("No se puede pasar a {}: el archivo no existe", etiqueta(&self.lista, pista))
            };
            self.mostrar_estado(&aviso);
            return;
        }
        let fader = f64::from(self.fader());
        if !self.gestor.esta_reproduciendo() || fader <= 0.0 {
            self.reproducir_pista(destino);
            return;
        }
        // Hacia o desde un comercial no se mezcla: la que suena se apaga con
        // el fader y despues arranca la otra.
        let canciones = &self.repositorio_de_canciones;
        let reservada = |pista: Option<&Pista>| pista.is_some_and(|pista| suena_sin_mezcla(canciones, pista));
        if reservada(self.gestor.pista_actual()) || reservada(self.lista.pista_en(destino)) {
            let pista = self.lista.pista_en(destino).cloned().expect("el destino esta en la lista");
            self.busqueda_pendiente = None;
            if self.gestor.detener_con_fundido(fader) {
                self.tras_el_fundido = Some((destino, pista.clone()));
                self.refrescar_interfaz();
                self.mostrar_estado(&tr!("Fundido de salida de {} s y despues {} (tipo sin mezcla: sin mezcla)", self.fader(), etiqueta(&self.lista, &pista)));
            } else {
                self.reproducir_pista(destino);
            }
            return;
        }
        self.lista.seleccionar_indice(destino);
        let pista = self.lista.pista_actual().cloned().expect("el destino esta en la lista");
        self.busqueda_pendiente = None;
        match self.gestor.mezclar_hacia(self.transicion_con_fader(&pista, fader)) {
            Ok(()) => {
                self.mezcla_manual = true;
                self.al_empezar_pista(&pista);
                self.refrescar_interfaz();
                self.refrescar_etiqueta_de_tiempo();
                self.mostrar_estado(&tr!("Mezclando hacia {} ({} s)", etiqueta(&self.lista, &pista), self.fader()));
            }
            Err(error) => {
                self.mostrar_estado(&tr!("Pista invalida: {error}", error = error));
                self.errores.push(error.to_string());
            }
        }
    }

    /// Mezcla del boton Siguiente: dura lo que el fader. Con el crossfade
    /// inteligente usa curvas de potencia y la entrante saltea su silencio.
    fn transicion_con_fader(&self, pista: &Pista, fader: f64) -> Transicion {
        let simple = Transicion { inicio: self.lista.puntos_de(pista).recorte_inicial(), ..Transicion::simple(pista.clone(), fader) };
        if !self.crossfade_inteligente() {
            return simple;
        }
        Transicion { curva: Curva::Potencia { saliente_natural: false, entrante_natural: false }, ..simple }
    }

    fn reproducir_anterior(&mut self) {
        if !self.admitida_o_avisar(OperacionSobreLaSecuencia::Anterior) {
            return;
        }
        if self.lista.anterior().is_some() {
            self.reproducir_pista_actual();
        } else {
            self.mostrar_estado(&tr!("No hay pista anterior"));
        }
    }

    /// Detiene bajando la musica durante los segundos del fader; un segundo
    /// Detener durante el fundido, o el fader en 0, detiene al instante.
    fn detener_todo(&mut self) {
        self.busqueda_pendiente = None;
        self.tras_el_fundido = None;
        if self.gestor.detener_con_fundido(f64::from(self.fader())) {
            self.mostrar_estado(&tr!("Deteniendo con fundido de {} s (Detener otra vez corta ya)", self.fader()));
            return;
        }
        self.al_quedar_detenida();
    }

    /// Arranca la pista que esperaba el fin del fundido, si sigue en la lista
    /// (en su posicion o, si la lista cambio, en otra).
    fn reproducir_tras_el_fundido(&mut self, indice: usize, pista: &Pista) {
        let indice = if self.lista.pista_en(indice) == Some(pista) { Some(indice) } else { self.lista.indice_de(pista) };
        match indice {
            Some(indice) => self.reproducir_pista(indice),
            None => self.al_quedar_detenida(),
        }
    }

    fn al_quedar_detenida(&mut self) {
        self.con_ventana(|ventana| ventana.set_progreso(0.0));
        self.refrescar_etiqueta_de_tiempo();
        self.refrescar_interfaz();
        self.mostrar_estado(&tr!("Reproduccion detenida"));
        // Detenida con el boton estando al aire: se pregunta si salir.
        if std::mem::take(&mut self.detenida_al_aire) && self.modo_de_operacion().al_aire() {
            self.pregunta_de_salir_del_aire = true;
        }
    }

    /// Detener (el boton): al aire, cuando termina el fundido se pregunta si
    /// pasar a modo EDITOR o seguir al aire.
    fn detener_desde_la_ventana(&mut self) {
        if self.modo_de_operacion().al_aire() && self.gestor.hay_pista_cargada() {
            self.detenida_al_aire = true;
        }
        self.detener_todo();
    }

    fn definir_repetir(&mut self, repetir: bool) {
        self.lista.definir_repetir(repetir);
        self.guardar_lista();
        self.refrescar_interfaz();
    }

    fn ajustar_volumen(&mut self, volumen: f64) {
        self.volumen_seleccionado = volumen.clamp(0.0, 1.0);
        self.gestor.ajustar_volumen(self.volumen_seleccionado);
    }

    /// Cambia el volumen tambien en el deslizante.
    fn cambiar_volumen(&mut self, volumen: f64) {
        self.ajustar_volumen(volumen);
        let volumen = self.volumen_seleccionado as f32;
        self.con_ventana(|ventana| ventana.set_volumen(volumen));
    }

    fn guardar_fader(&mut self, segundos: u8) {
        self.repositorio_de_preferencias.guardar_fader(segundos.min(FADER_MAXIMO));
    }

    /// Cambia el fader tambien en el deslizante.
    fn cambiar_fader(&mut self, segundos: u8) {
        self.guardar_fader(segundos);
        let fader = f32::from(self.fader());
        self.con_ventana(|ventana| ventana.set_fader(fader));
    }

    /// Logica de secuencia: al terminar una pista se reproduce la siguiente;
    /// si no queda ninguna, la secuencia finaliza. Solo se usa cuando no hubo
    /// mezcla; con mezcla, la siguiente ya arranco en al_iniciar_transicion.
    fn avanzar_secuencia(&mut self, finalizada: &Pista) {
        if self.gestor.pista_actual() != Some(finalizada) {
            return;
        }
        // Se cargo otra lista mientras sonaba un tema que no esta en ella:
        // sigue la lista nueva desde el principio.
        if self.lista.indice_actual().is_none() && !self.lista.esta_vacia() {
            self.reproducir_pista(0);
            return;
        }
        let (destino, salteadas) = self.proxima_reproducible();
        match destino {
            Some(indice) => {
                self.lista.seleccionar_indice(indice);
                self.reproducir_pista_actual();
                self.avisar_salteadas(&salteadas);
            }
            None => {
                self.refrescar_interfaz();
                if salteadas.is_empty() {
                    self.mostrar_estado(&tr!("Secuencia finalizada"));
                } else {
                    self.mostrar_estado(&tr!("Secuencia finalizada: los archivos que seguian en la lista no existen"));
                }
            }
        }
    }

    fn al_iniciar_transicion(&mut self, entrante: &Pista) {
        let (destino, salteadas) = self.proxima_reproducible();
        let salteadas = match destino.filter(|&indice| self.lista.pista_en(indice) == Some(entrante)) {
            Some(indice) => {
                self.lista.seleccionar_indice(indice);
                salteadas
            }
            None => Vec::new(),
        };
        self.al_empezar_pista(entrante);
        self.refrescar_interfaz();
        self.refrescar_etiqueta_de_tiempo();
        let detalle = match self.gestor.detalle_de_transicion() {
            "" => String::new(),
            detalle => format!(" ({detalle})"),
        };
        self.mostrar_estado(&tr!("Mezclando hacia {}{detalle}", etiqueta(&self.lista, entrante), detalle = detalle));
        self.avisar_salteadas(&salteadas);
    }

    /// Registra el inicio de una pista y mueve el selector a ella.
    fn al_empezar_pista(&mut self, pista: &Pista) {
        // Si volvio a sonar antes de quedar detenida, ya no se pregunta.
        self.detenida_al_aire = false;
        self.disparar_efecto_fijado(pista);
        self.guardar_lista();
        if let Some(indice) = self.lista.indice_actual() {
            self.seleccionar_solo(indice);
        }
    }

    fn reproducir_pista_actual(&mut self) {
        let Some(pista) = self.lista.pista_actual().cloned() else {
            return;
        };
        // Un marcador no suena: se pasa a la proxima pista reproducible.
        if es_marcador(pista.archivo()) {
            match self.proxima_reproducible().0 {
                Some(indice) => {
                    self.lista.seleccionar_indice(indice);
                    self.reproducir_pista_actual();
                }
                None => self.mostrar_estado(&tr!("Los marcadores de bloque fijo no suenan y no hay temas despues")),
            }
            return;
        }
        // El anuncio HTH se arma con la hora de este momento; si no se puede,
        // se saltea (diria una hora vieja) y sigue la proxima pista.
        if es_anuncio_hth(pista.archivo()) && !self.preparar_anuncio_hth() {
            if let Some(indice) = self.proxima_sin_anuncio_hth() {
                self.lista.seleccionar_indice(indice);
                self.reproducir_pista_actual();
            } else {
                self.refrescar_interfaz();
            }
            return;
        }
        // Un archivo movido o borrado se avisa en la barra de estado, sin
        // dialogo: una ventana modal frenaria el ciclo de la reproduccion.
        if !archivo_existe(&pista) {
            log::warn!("No existe el archivo de {}", pista.archivo().display());
            self.refrescar_interfaz();
            self.mostrar_estado(&tr!("No se puede reproducir {}: el archivo no existe (Validar ofrece quitarlo de la lista)", etiqueta(&self.lista, &pista)));
            return;
        }
        // Lo que esperaba un fundido queda sin efecto: ya arranca otra pista.
        self.tras_el_fundido = None;
        // Si se vuelve a empezar la misma pista, la anterior pasada ya termino.
        self.registrar_fin_de_reproduccion();
        self.busqueda_pendiente = None;
        match self.gestor.reproducir(&pista) {
            Ok(()) => {
                self.gestor.ajustar_volumen(self.volumen_seleccionado);
                self.al_empezar_pista(&pista);
            }
            Err(error) => {
                self.mostrar_estado(&tr!("Pista invalida: {error}", error = error));
                self.errores.push(error.to_string());
            }
        }
        self.refrescar_interfaz();
        self.refrescar_etiqueta_de_tiempo();
    }

    fn actualizar_progreso(&mut self) {
        if !self.gestor.hay_pista_cargada() {
            return;
        }
        let duracion = self.gestor.duracion_total().filter(|duracion| *duracion > 0.0);
        let moviendo = self.busqueda_pendiente.is_some();
        let transcurrido = self.gestor.tiempo_transcurrido();
        let marcas = match (duracion, self.gestor.pista_actual()) {
            (Some(duracion), Some(pista)) => {
                let mut marcas = MarcasDeProgreso::de(&self.lista.puntos_de(pista), duracion);
                // La mezcla que viene, mientras no haya una en curso.
                if !self.gestor.hay_transicion_en_curso() {
                    let inteligente = self.crossfade_inteligente();
                    let transicion =
                        transicion_desde(&self.lista, &self.datos_de_mezcla, &self.repositorio_de_canciones, inteligente, pista);
                    if let Some(transicion) = transicion.filter(|transicion| transicion.segundos > 0.0) {
                        marcas.mezcla = MarcasDeProgreso::tramo_de_mezcla(&transicion, duracion);
                    }
                }
                marcas
            }
            _ => MarcasDeProgreso::NINGUNA,
        };
        self.con_ventana(|ventana| {
            ventana.set_progreso_habilitado(duracion.is_some());
            let del_detalle = self.onda_del_detalle.borrow();
            ventana.set_onda_de_la_que_suena(
                del_detalle.is_some() && del_detalle.as_deref() == self.gestor.pista_actual().map(Pista::archivo),
            );
            if let Some(duracion) = duracion {
                ventana.set_progreso_maximo(duracion as f32);
                if !moviendo {
                    ventana.set_progreso(transcurrido as f32);
                }
            }
            let ((overlay_inicio, overlay_fin), (outro_inicio, outro_fin)) = (marcas.overlay, marcas.outro);
            ventana.set_progreso_overlay_inicio(overlay_inicio);
            ventana.set_progreso_overlay_fin(overlay_fin);
            ventana.set_progreso_outro_inicio(outro_inicio);
            ventana.set_progreso_outro_fin(outro_fin);
            ventana.set_progreso_overlay_manual(marcas.overlay_manual);
            ventana.set_progreso_outro_manual(marcas.outro_manual);
            ventana.set_progreso_mezcla_inicio(marcas.mezcla.0);
            ventana.set_progreso_mezcla_fin(marcas.mezcla.1);
        });
        self.refrescar_etiqueta_de_tiempo();
    }

    fn refrescar_etiqueta_de_tiempo(&self) {
        let duracion = self.gestor.duracion_total().filter(|duracion| duracion.is_finite() && *duracion > 0.0);
        let total = duracion.unwrap_or(0.0);
        let transcurrido = match duracion {
            Some(total) => self.gestor.tiempo_transcurrido().clamp(0.0, total),
            None => 0.0,
        };
        let restante = (total - transcurrido).max(0.0);
        let texto = match self.formato_de_tiempo {
            FormatoDeTiempo::Transcurrido => format!("{} / {}", formatear_minutos(transcurrido), formatear_minutos(total)),
            FormatoDeTiempo::Restante => format!("-{} / {}", formatear_minutos(restante), formatear_minutos(total)),
            FormatoDeTiempo::SegundosRestantes => format!("-{restante:.1} s / {total:.1} s"),
            FormatoDeTiempo::SegundosTranscurridos => format!("{transcurrido:.1} s / {total:.1} s"),
        };
        self.con_ventana(|ventana| ventana.set_tiempo(texto.into()));
    }

    fn refrescar_interfaz(&mut self) {
        let reproduciendo = self.gestor.esta_reproduciendo();
        let pausado = self.gestor.esta_pausado();
        self.estado_anterior = (reproduciendo, pausado);
        let actual = self
            .gestor
            .pista_actual()
            .map(|pista| self.titulo_destacado(pista))
            .unwrap_or_else(|| tr!("Sin pista cargada").into());
        let indice_actual = self.lista.indice_actual();
        let hay_anterior = indice_actual.is_some_and(|indice| indice > 0);
        let hay_siguiente = self.marcada_para_siguiente().is_some() || self.lista.indice_siguiente(indice_actual).is_some();
        self.refrescar_renglon_destacado();
        self.con_ventana(|ventana| {
            ventana.set_pista_actual(actual.into());
            if reproduciendo {
                ventana.set_texto_reproducir(tr!("Pausa").into());
                ventana.set_estado(tr!("Reproduciendo en secuencia").into());
            } else if pausado {
                ventana.set_texto_reproducir(tr!("Reanudar").into());
                ventana.set_estado("Pausado".into());
            } else {
                ventana.set_texto_reproducir(tr!("Reproducir").into());
            }
            ventana.set_anterior_habilitado(hay_anterior);
            ventana.set_siguiente_habilitado(hay_siguiente);
        });
        self.refrescar_filas();
        self.centrar_pista_actual();
    }

    /// Titulo grande de la pista que suena: "# 84 Titulo - Artista". Sin
    /// titulo en la ficha se usa el nombre del archivo sin la extension.
    fn titulo_destacado(&self, pista: &Pista) -> String {
        self.renglon_destacado(pista, self.lista.indice_de(pista)).texto()
    }

    /// Numero, titulo y artista de una pista para el renglon grande.
    fn renglon_destacado(&self, pista: &Pista, indice: Option<usize>) -> RenglonDestacado {
        let ficha = self.repositorio_de_canciones.buscar(pista.archivo());
        let titulo = ficha.and_then(|ficha| ficha.titulo.clone()).unwrap_or_else(|| {
            pista.archivo().file_stem().map_or_else(|| pista.nombre(), |nombre| nombre.to_string_lossy().into_owned())
        });
        let artista = ficha.and_then(|ficha| ficha.artista.clone()).filter(|artista| !artista.trim().is_empty()).unwrap_or_default();
        // Lo que no tiene tipo suena como un tema: lleva su icono.
        let tipo = if es_marcador(pista.archivo()) {
            TIPO_DEL_MARCADOR.to_string()
        } else if es_anuncio_hth(pista.archivo()) || es_base_hth(pista.archivo(), self.carpeta_de_bases_en_la_lista().as_deref()) {
            TIPO_DEL_ANUNCIO_HTH.to_string()
        } else {
            ficha.and_then(|ficha| ficha.tipo).map_or(TipoDeElemento::Tema.nombre(), TipoDeElemento::nombre).to_string()
        };
        RenglonDestacado { tipo, numero: indice.map(|indice| (indice + 1).to_string()).unwrap_or_default(), titulo, artista }
    }

    /// El renglon grande: la pista que suena y la que sigue en la secuencia
    /// (al aire se alternan).
    fn refrescar_renglon_destacado(&self) {
        let actual = self.gestor.pista_actual().filter(|_| self.gestor.hay_pista_cargada());
        let en_curso = actual.map(|pista| {
            let indice = self.lista.indice_actual().filter(|&indice| self.lista.pista_en(indice) == Some(pista)).or_else(|| self.lista.indice_de(pista));
            self.renglon_destacado(pista, indice)
        });
        let siguiente = en_curso.as_ref().and_then(|_| {
            let (proxima, _) = self.proxima_reproducible();
            proxima.and_then(|indice| self.lista.pista_en(indice).map(|pista| self.renglon_destacado(pista, Some(indice))))
        });
        let en_curso = en_curso.unwrap_or_default();
        let siguiente = siguiente.unwrap_or_default();
        self.con_ventana(|ventana| {
            ventana.set_tipo_en_curso(en_curso.tipo.as_str().into());
            ventana.set_numero_en_curso(en_curso.numero.as_str().into());
            ventana.set_titulo_en_curso(en_curso.titulo.as_str().into());
            ventana.set_artista_en_curso(en_curso.artista.as_str().into());
            ventana.set_tipo_siguiente(siguiente.tipo.as_str().into());
            ventana.set_numero_siguiente(siguiente.numero.as_str().into());
            ventana.set_titulo_siguiente(siguiente.titulo.as_str().into());
            ventana.set_artista_siguiente(siguiente.artista.as_str().into());
            ventana.set_siguiente_en_una_linea(siguiente.texto().into());
        });
    }

    /// BPM del tema: el fijado a mano o, si no hay, el del analisis.
    pub fn bpm_de(&self, archivo: &Path) -> Option<f64> {
        let manual = self.repositorio_de_canciones.buscar(archivo).and_then(|ficha| ficha.bpm);
        manual.or_else(|| self.datos_de_mezcla.get(archivo).and_then(DatosDeMezcla::bpm))
    }

    /// BPM del tema para la lista: "…" mientras se analiza y "—" si no tiene
    /// un ritmo claro.
    fn texto_de_bpm(&self, archivo: &Path) -> String {
        match self.bpm_de(archivo) {
            Some(bpm) => format!("{bpm:.0}"),
            None if self.analisis_pedidos.contains(archivo) => "…".into(),
            None => "—".into(),
        }
    }

    /// La columna Sono de cada fila: cuando empezo a sonar por ultima vez o,
    /// en las que siguen a la que suena, cuando se estima que va a empezar
    /// (descontando lo que se superpone en cada mezcla). La estimacion se
    /// corta en la primera pista sin duracion conocida.
    fn horarios_de_inicio(&self) -> Vec<HorarioDeInicio> {
        // Todo en la hora de la localidad del HTH, como el reloj.
        let zona = self.zona_de_la_localidad();
        let mut horarios: Vec<HorarioDeInicio> = self
            .lista
            .pistas()
            .iter()
            .map(|pista| {
                if es_marcador(pista.archivo()) {
                    return HorarioDeInicio::default();
                }
                self.repositorio_de_canciones
                    .buscar(pista.archivo())
                    .and_then(|ficha| ficha.ultima_reproduccion.as_deref())
                    .map(|momento| HorarioDeInicio {
                        texto: reproduccion_para_la_lista(momento, &zona),
                        ayuda: ultima_reproduccion(momento, &zona).1,
                        estimado: false,
                    })
                    .unwrap_or_default()
            })
            .collect();
        let sonando = (self.gestor.esta_reproduciendo() || self.gestor.esta_pausado()) && self.gestor.hay_pista_cargada();
        let Some(actual) = self.lista.indice_actual().filter(|_| sonando) else {
            return horarios;
        };
        let ahorro = |indice: usize| self.ahorros_de_mezcla.get(indice).copied().unwrap_or(0.0);
        let puntos_actuales = self.lista.pista_en(actual).map(|pista| self.lista.puntos_de(pista)).unwrap_or_default();
        let duracion_actual = self.gestor.duracion_total().or(self.duraciones.get(actual).copied().flatten()).unwrap_or(0.0);
        // Segundos hasta que empiece la siguiente.
        let mut faltan = (duracion_actual - puntos_actuales.recorte_final() - self.gestor.tiempo_transcurrido()).max(0.0) - ahorro(actual);
        let ahora = jiff::Timestamp::now();
        for (indice, pista) in self.lista.pistas().iter().enumerate().skip(actual + 1) {
            if es_marcador(pista.archivo()) {
                continue;
            }
            let espera = faltan.max(0.0);
            let inicio = ahora.checked_add(jiff::SignedDuration::from_secs_f64(espera)).unwrap_or(ahora);
            horarios[indice] = HorarioDeInicio {
                texto: inicio.to_zoned(zona.clone()).strftime(&formato_de_la_columna_sono()).to_string(),
                ayuda: tr!("Hora estimada: empieza dentro de {}", formatear_duracion(espera)),
                estimado: true,
            };
            let Some(duracion) = self.duraciones.get(indice).copied().flatten() else {
                break;
            };
            let puntos = self.lista.puntos_de(pista);
            faltan += (duracion - puntos.recorte_inicial() - puntos.recorte_final()).max(0.0) - ahorro(indice);
        }
        horarios
    }

    /// Actualiza solo la columna Sono (las estimaciones corren con el reloj),
    /// sin rearmar el resto de las filas.
    fn refrescar_horarios(&self) {
        let horarios = self.horarios_de_inicio();
        if horarios.len() != self.filas.row_count() {
            return;
        }
        for (indice, horario) in horarios.into_iter().enumerate() {
            let Some(mut fila) = self.filas.row_data(indice) else { continue };
            if fila.marcador > 0 {
                continue;
            }
            if fila.sono.as_str() != horario.texto || fila.ayuda_de_sono.as_str() != horario.ayuda || fila.sono_estimada != horario.estimado {
                fila.sono = horario.texto.into();
                fila.ayuda_de_sono = horario.ayuda.into();
                fila.sono_estimada = horario.estimado;
                self.filas.set_row_data(indice, fila);
            }
        }
    }

    /// Actualiza solo las filas que cambiaron, para no interrumpir una
    /// edicion en curso en otra fila.
    fn refrescar_filas(&mut self) {
        // La siguiente del renglon grande cambia con la lista.
        self.refrescar_renglon_destacado();
        // Los recortes de cada tema cambian con sus puntos, que siempre
        // terminan refrescando la lista.
        self.aplicar_recortes();
        self.aplicar_duraciones();
        self.duraciones = self.lista.pistas().iter().map(|pista| self.duracion_de(pista)).collect();
        let transiciones: Vec<Option<Transicion>> = (0..self.lista.tamano()).map(|indice| self.transicion_tras(indice)).collect();
        self.precalcular_totales(&transiciones);
        let hay_pista = self.gestor.hay_pista_cargada();
        let carpeta_de_bases = self.carpeta_de_bases_en_la_lista();
        let en_bloque_fijo = self.posiciones_en_bloques_fijos();
        let horarios = self.horarios_de_inicio();
        let filas: Vec<FilaDePista> = self
            .lista
            .pistas()
            .iter()
            .enumerate()
            .map(|(indice, pista)| {
                let puntos = self.lista.puntos_de(pista);
                let ficha = self.repositorio_de_canciones.buscar(pista.archivo());
                let dato = |valor: Option<&String>| valor.cloned().unwrap_or_default();
                let es_actual = hay_pista && Some(indice) == self.lista.indice_actual();
                let mezcla = transiciones[indice]
                    .as_ref()
                    .map_or_else(|| "—".into(), |transicion| formatear_segundos(transicion.segundos));
                let anuncio = es_anuncio_hth(pista.archivo());
                let base = es_base_hth(pista.archivo(), carpeta_de_bases.as_deref());
                let titulo = ficha.and_then(|ficha| ficha.titulo.clone()).unwrap_or_else(|| pista.nombre());
                let marcador = marcador_de(pista.archivo());
                let mut fila = FilaDePista {
                    posicion: (indice + 1).to_string().into(),
                    titulo: if anuncio {
                        titulo_del_anuncio_hth().into()
                    } else if base {
                        tr!("HTH · {titulo}", titulo = titulo).into()
                    } else {
                        dato(ficha.and_then(|ficha| ficha.titulo.as_ref())).into()
                    },
                    artista: dato(ficha.and_then(|ficha| ficha.artista.as_ref())).into(),
                    ruta: pista.archivo().display().to_string().into(),
                    overlay: segundos_de_la_ventana(puntos.overlay()).into(),
                    ventana_de_overlay: puntos
                        .overlay()
                        .map(|ventana| format!("{} - {}", formatear_tiempo(ventana.inicio()), formatear_tiempo(ventana.fin())))
                        .unwrap_or_default()
                        .into(),
                    mezcla: mezcla.into(),
                    duracion: self.duraciones[indice].map_or_else(|| "…".into(), formatear_duracion).into(),
                    tipo: if anuncio || base {
                        TIPO_DEL_ANUNCIO_HTH.into()
                    } else {
                        ficha.and_then(|ficha| ficha.tipo).map(TipoDeElemento::nombre).unwrap_or_default().into()
                    },
                    nota: dato(ficha.and_then(|ficha| ficha.nota.as_ref())).into(),
                    ranking: if anuncio { "".into() } else { estrellas_de_la_lista(ficha.map_or(0, |ficha| ficha.ranking)).into() },
                    sono: horarios[indice].texto.as_str().into(),
                    ayuda_de_sono: horarios[indice].ayuda.as_str().into(),
                    sono_estimada: horarios[indice].estimado,
                    // Una base HTH conserva su genero; el anuncio no tiene.
                    genero: if anuncio { "".into() } else { dato(ficha.and_then(|ficha| ficha.genero.as_ref())).into() },
                    anio: if anuncio { "".into() } else { ficha.and_then(|ficha| ficha.anio).map(|anio| anio.to_string()).unwrap_or_default().into() },
                    bpm: self.texto_de_bpm(pista.archivo()).into(),
                    seleccionada: self.seleccion.contains(&indice),
                    overlay_manual: puntos.overlay_manual(),
                    fijado: ficha.is_some_and(|ficha| ficha.efecto_fijado.is_some()),
                    reservado: if anuncio || base {
                        FRANJA_DEL_ANUNCIO_HTH
                    } else {
                        ficha.and_then(|ficha| ficha.tipo).map_or(0, TipoDeElemento::franja)
                    },
                    sonando: es_actual,
                    hth: if base { 2 } else { i32::from(anuncio) },
                    coincide: self.busqueda.as_ref().is_some_and(|consulta| {
                        let ruta = pista.archivo().to_string_lossy();
                        consulta.coincide(&DatosDelTema {
                            titulo: ficha.and_then(|ficha| ficha.titulo.as_deref()).unwrap_or_default(),
                            artista: ficha.and_then(|ficha| ficha.artista.as_deref()).unwrap_or_default(),
                            genero: ficha.and_then(|ficha| ficha.genero.as_deref()).unwrap_or_default(),
                            album: ficha.and_then(|ficha| ficha.album.as_deref()).unwrap_or_default(),
                            tipo: ficha.and_then(|ficha| ficha.tipo).map(TipoDeElemento::para_buscar).unwrap_or_default(),
                            nota: ficha.and_then(|ficha| ficha.nota.as_deref()).unwrap_or_default(),
                            archivo: &ruta,
                            ruta: &ruta,
                            anio: ficha.and_then(|ficha| ficha.anio),
                            ranking: ficha.map(|ficha| ficha.ranking),
                            bpm: self.bpm_de(pista.archivo()),
                            duracion: self.duraciones[indice],
                            ultima_reproduccion: ficha.and_then(|ficha| ficha.ultima_reproduccion.as_deref()),
                            posicion: Some(indice + 1),
                        })
                    }),
                    marcador: match marcador {
                        Some(MarcadorDeBloque::Inicio) => 1,
                        Some(MarcadorDeBloque::Fin) => 2,
                        None => 0,
                    },
                    en_bloque_fijo: en_bloque_fijo.get(indice).copied().unwrap_or(false),
                };
                if let Some(marcador) = marcador {
                    fila.titulo = titulo_del_marcador(marcador).into();
                    fila.artista = ayuda_del_marcador().into();
                    fila.tipo = TIPO_DEL_MARCADOR.into();
                    fila.genero = "".into();
                    fila.ranking = "".into();
                    fila.sono = "".into();
                    fila.ayuda_de_sono = "".into();
                    fila.sono_estimada = false;
                    fila.nota = "".into();
                    fila.duracion = "".into();
                    fila.mezcla = "".into();
                    fila.bpm = "".into();
                    fila.overlay = "".into();
                    fila.ventana_de_overlay = "".into();
                    fila.reservado = 0;
                    fila.fijado = false;
                }
                fila
            })
            .collect();
        self.coincidencias = filas.iter().enumerate().filter(|(_, fila)| fila.coincide).map(|(indice, _)| indice).collect();
        self.refrescar_bases_hth(filas.iter().any(|fila| fila.hth > 0));
        let resultado = self.resultado_de_busqueda();
        let buscando = self.busqueda.is_some() || self.problema_de_busqueda.is_some();
        self.con_ventana(|ventana| {
            ventana.set_buscando(buscando);
            ventana.set_resultado_de_busqueda(resultado.into());
        });
        actualizar_modelo(&self.filas, filas);
        // El boton Siguiente tambien depende del selector.
        let hay_siguiente = self.marcada_para_siguiente().is_some() || self.lista.indice_siguiente(self.lista.indice_actual()).is_some();
        let hay_seleccion = !self.seleccion.is_empty();
        self.con_ventana(|ventana| {
            ventana.set_siguiente_habilitado(hay_siguiente);
            ventana.set_hay_seleccion(hay_seleccion);
        });
        self.refrescar_detalles();
        self.refrescar_totales();
    }

    // Botonera

    /// Dispara el efecto de un boton (0 a 9) de una hoja (desde 0; None es la
    /// que esta a la vista). Si el que se pulsa es el que esta en espera, lo
    /// cancela. Falla si el boton esta vacio.
    pub fn pulsar_efecto(&mut self, hoja: Option<usize>, indice: usize) -> Result<String, String> {
        let hoja = hoja.unwrap_or(self.repositorio_de_botonera.hoja_actual());
        if hoja >= self.repositorio_de_botonera.cantidad_de_hojas() {
            return Err(tr!("No existe la hoja {} (hay {}).", hoja + 1, self.repositorio_de_botonera.cantidad_de_hojas()));
        }
        let Some(archivo) = self.repositorio_de_botonera.archivo(hoja, indice).map(Path::to_path_buf) else {
            return Err(tr!("El boton {indice} de la hoja {} esta vacio.", hoja + 1, indice = indice));
        };
        if self.overlay.en_espera() == Some(archivo.as_path()) {
            self.overlay.cancelar_espera();
            self.refrescar_botones();
            return Ok(tr!("Se cancelo el efecto en espera del boton {indice}.", indice = indice));
        }
        if !archivo.is_file() {
            let aviso = tr!("No se encuentra {}", nombre_de_archivo(&archivo));
            self.aviso_de_botonera = Some((aviso.clone(), Instant::now()));
            return Ok(aviso);
        }
        let modo = self.repositorio_de_botonera.modo();
        let carpeta_de_bases = self.carpeta_de_bases_en_la_lista();
        // Los tipos sin mezcla no tienen ventana de overlay y la de una base
        // del HTH es para la voz del anuncio: en modo trabado no aceptan
        // efectos (en modo libre si).
        let en_curso_reservada = self
            .gestor
            .pista_actual()
            .filter(|_| self.gestor.esta_reproduciendo() || self.gestor.esta_pausado())
            .filter(|pista| !admite_efectos_de_la_botonera(&self.repositorio_de_canciones, carpeta_de_bases.as_deref(), pista))
            .cloned();
        if let (ModoDeOverlay::Trabada, Some(pista)) = (modo, en_curso_reservada) {
            let aviso = if es_base_hth_en(pista.archivo(), carpeta_de_bases.as_deref()) {
                tr!("{} es la base de un anuncio HTH y no admite efectos (solo en modo libre).", etiqueta(&self.lista, &pista))
            } else {
                tr!("{} no es un Tema y no admite efectos (solo en modo libre).", etiqueta(&self.lista, &pista))
            };
            self.aviso_de_botonera = Some((aviso.clone(), Instant::now()));
            return Ok(aviso);
        }
        self.boton_del_efecto = Some((hoja, indice));
        let musica = MedidorDeMusica {
            datos: &self.datos_de_mezcla,
            nivelar: self.repositorio_de_preferencias.nivelar_volumen(),
            objetivo: self.repositorio_de_preferencias.objetivo_de_sonoridad(),
            volumen: self.volumen_seleccionado,
            onda: &self.onda_de_musica,
        };
        let lista = &self.lista;
        let ventana_de = |pista: &Pista| lista.puntos_de(pista).overlay();
        let canciones = &self.repositorio_de_canciones;
        let admite_efectos = |pista: &Pista| admite_efectos_de_la_botonera(canciones, carpeta_de_bases.as_deref(), pista);
        let musica_de = |pista: &Pista, desde: f64, segundos: f64| musica.medir(pista, desde, segundos);
        let canciones_de_efectos = &self.repositorio_de_canciones;
        let recorte_de = |archivo: &Path| recorte_de_efecto(canciones_de_efectos, archivo);
        let entorno = Entorno {
            ventana_de: &ventana_de,
            admite_efectos: &admite_efectos,
            musica_de: &musica_de,
            recorte_de: &recorte_de,
        };
        if let Err(error) = self.overlay.disparar(&archivo, modo, &self.gestor, &entorno) {
            self.errores.push(error.to_string());
        }
        self.refrescar_botones();
        let nombre = tr!("hoja {}: {}", hoja + 1, nombre_de_archivo(&archivo));
        let lista = &self.lista;
        let ventana_de = |pista: &Pista| lista.puntos_de(pista).overlay();
        Ok(match self.overlay.estado_de_espera(&self.gestor, &ventana_de) {
            Some(EstadoDeEspera::Faltan(segundos)) => {
                tr!("Boton {indice}: {nombre} espera la ventana de overlay (faltan {:.1} s).", segundos, indice = indice, nombre = nombre)
            }
            Some(EstadoDeEspera::ProximaPista) => {
                tr!("Boton {indice}: {nombre} ya no cabe en la ventana; espera la de la proxima pista.", indice = indice, nombre = nombre)
            }
            None => {
                let plan = self.overlay.ultimo_plan().map(|plan| format!(" ({})", plan.descripcion())).unwrap_or_default();
                tr!("Boton {indice}: sonando {nombre}{plan}.", indice = indice, nombre = nombre, plan = plan)
            }
        })
    }

    pub fn asignar_efecto(&mut self, hoja: usize, indice: usize, archivo: &Path) -> Result<(), String> {
        self.repositorio_de_botonera.asignar(hoja, indice, archivo)?;
        preparar_efectos(vec![archivo.to_path_buf()]);
        self.refrescar_botones();
        Ok(())
    }

    pub fn quitar_efecto(&mut self, hoja: usize, indice: usize) {
        self.repositorio_de_botonera.quitar(hoja, indice);
        self.refrescar_botones();
    }

    pub fn cantidad_de_hojas(&self) -> usize {
        self.repositorio_de_botonera.cantidad_de_hojas()
    }

    pub fn hoja_actual(&self) -> usize {
        self.repositorio_de_botonera.hoja_actual()
    }

    pub fn detener_efecto(&mut self) {
        self.overlay.detener();
        self.refrescar_botones();
    }

    fn guardar_modo_libre(&mut self, libre: bool) {
        let modo = if libre { ModoDeOverlay::Libre } else { ModoDeOverlay::Trabada };
        self.repositorio_de_botonera.guardar_modo(modo);
    }

    /// Cambia el volumen de los efectos y la atenuacion de la musica, los
    /// muestra en la ventana y en el editor, y los guarda en un momento.
    pub(crate) fn cambiar_ajustes_de_efecto(&mut self, ajustes: AjustesDeEfecto) {
        self.overlay.definir_ajustes(ajustes);
        self.mostrar_ajustes_de_efecto(self.overlay.ajustes());
        self.atenuacion_por_guardar = Some(Instant::now());
    }

    fn mostrar_ajustes_de_efecto(&self, ajustes: AjustesDeEfecto) {
        self.con_ventana(|ventana| {
            ventana.set_atenuar(ajustes.atenuar);
            ventana.set_nivel_de_musica(ajustes.nivel_de_musica as f32);
            ventana.set_efecto_inteligente(ajustes.inteligente);
            ventana.set_volumen_de_efecto(ajustes.volumen_db as f32);
            ventana.set_anticipacion(ajustes.anticipacion as f32);
            ventana.set_recuperacion(ajustes.recuperacion as f32);
            ventana.set_separacion(ajustes.separacion_db as f32);
        });
        let editor = &self.editor;
        editor.set_atenuar(ajustes.atenuar);
        editor.set_nivel_de_musica(ajustes.nivel_de_musica as f32);
        editor.set_efecto_inteligente(ajustes.inteligente);
        editor.set_volumen_de_efecto(ajustes.volumen_db as f32);
        editor.set_anticipacion(ajustes.anticipacion as f32);
        editor.set_recuperacion(ajustes.recuperacion as f32);
        editor.set_separacion(ajustes.separacion_db as f32);
    }

    /// Si el tema tiene un efecto fijado, lo deja programado en modo trabado:
    /// suena entero dentro de su ventana de overlay (o enseguida si no
    /// tiene). Va antes que el efecto de la botonera que este esperando, que
    /// no se pierde: sale despues o en la pista siguiente que lo admita. Los
    /// tipos sin mezcla no admiten efecto fijado.
    fn disparar_efecto_fijado(&mut self, pista: &Pista) {
        // Sobre una base del anuncio HTH, el efecto es la voz recien armada.
        let efecto = match self.voz_para_la_base_hth(pista) {
            Some(Some(voz)) => voz,
            Some(None) => return,
            None => {
                if suena_sin_mezcla(&self.repositorio_de_canciones, pista) {
                    return;
                }
                let Some(efecto) =
                    self.repositorio_de_canciones.buscar(pista.archivo()).and_then(|ficha| ficha.efecto_fijado.clone())
                else {
                    return;
                };
                efecto
            }
        };
        if !efecto.is_file() {
            self.mostrar_estado(&tr!("No se encuentra el efecto fijado {}", nombre_de_archivo(&efecto)));
            return;
        }
        let musica = MedidorDeMusica {
            datos: &self.datos_de_mezcla,
            nivelar: self.repositorio_de_preferencias.nivelar_volumen(),
            objetivo: self.repositorio_de_preferencias.objetivo_de_sonoridad(),
            volumen: self.volumen_seleccionado,
            onda: &self.onda_de_musica,
        };
        let lista = &self.lista;
        let ventana_de = |pista: &Pista| lista.puntos_de(pista).overlay();
        let admite_efectos = |_: &Pista| true;
        let musica_de = |pista: &Pista, desde: f64, segundos: f64| musica.medir(pista, desde, segundos);
        let canciones_de_efectos = &self.repositorio_de_canciones;
        let recorte_de = |archivo: &Path| recorte_de_efecto(canciones_de_efectos, archivo);
        let entorno = Entorno {
            ventana_de: &ventana_de,
            admite_efectos: &admite_efectos,
            musica_de: &musica_de,
            recorte_de: &recorte_de,
        };
        if let Err(error) = self.overlay.disparar_fijado(&efecto, &self.gestor, &entorno) {
            self.errores.push(error.to_string());
        }
        self.refrescar_botones();
    }

    /// Lo necesario para medir como suena la musica alrededor de un efecto,
    /// sin tomar prestado todo el controlador.
    fn medidor_de_musica(&self) -> MedidorDeMusica<'_> {
        MedidorDeMusica {
            datos: &self.datos_de_mezcla,
            nivelar: self.nivelar_volumen(),
            objetivo: self.objetivo_de_sonoridad(),
            volumen: self.volumen_seleccionado,
            onda: &self.onda_de_musica,
        }
    }

    fn refrescar_botones(&self) {
        let en_espera = self.overlay.en_espera();
        let sonando = self.overlay.sonando();
        let hoja = self.repositorio_de_botonera.hoja_actual();
        // Si se sabe que boton se pulso, solo ese se resalta; si no (un efecto
        // de prueba del editor), se resaltan los que tienen ese archivo.
        let es_el_pulsado = |indice: usize| self.boton_del_efecto.is_none_or(|pulsado| pulsado == (hoja, indice));
        let botones: Vec<BotonDeEfecto> = (0..CANTIDAD_DE_BOTONES)
            .map(|indice| {
                let archivo = self.repositorio_de_botonera.archivo_del_boton(indice);
                let estado = match archivo {
                    Some(archivo) if Some(archivo) == en_espera && es_el_pulsado(indice) => 1,
                    Some(archivo) if Some(archivo) == sonando && es_el_pulsado(indice) => 2,
                    _ => 0,
                };
                let ficha = archivo.and_then(|archivo| self.repositorio_de_canciones.buscar(archivo));
                let nota = ficha.and_then(|ficha| ficha.nota.as_deref());
                // El titulo del catalogo si lo tiene; si no, el nombre sin extension.
                let titulo = ficha.and_then(|ficha| ficha.titulo.as_deref()).map(str::trim).filter(|titulo| !titulo.is_empty());
                BotonDeEfecto {
                    duracion: archivo
                        .and_then(|archivo| self.duracion_de_efecto(archivo))
                        .map_or_else(String::new, formatear_duracion)
                        .into(),
                    vacio: archivo.is_none(),
                    insertable: archivo.is_some_and(|archivo| !es_solo_para_la_botonera(&self.repositorio_de_canciones, archivo)),
                    texto: match (archivo, titulo) {
                        (None, _) => "(vacio)".into(),
                        (Some(_), Some(titulo)) => titulo.to_owned(),
                        (Some(archivo), None) => archivo
                            .file_stem()
                            .map_or_else(|| nombre_de_archivo(archivo), |nombre| nombre.to_string_lossy().into_owned()),
                    }
                    .into(),
                    con_nota: nota.is_some(),
                    ayuda: match (archivo, nota) {
                        (None, _) => tr!("Clic para asignar un archivo de sonido").into(),
                        (Some(ruta), None) => ruta.display().to_string(),
                        (Some(ruta), Some(nota)) => tr!("Nota: {nota}\n\n{}", ruta.display(), nota = nota),
                    }
                    .into(),
                    estado,
                }
            })
            .collect();
        actualizar_modelo(&self.botones, botones);
        let repositorio = &self.repositorio_de_botonera;
        let (hoja, hojas) = (repositorio.hoja_actual(), repositorio.cantidad_de_hojas());
        let de_siempre = nombre_de_siempre_de_la_hoja(hoja);
        let propio = repositorio.nombre_de_hoja(hoja).map(str::to_string);
        self.con_ventana(|ventana| {
            ventana.set_hoja_de_efectos(tr!("Hoja {} de {hojas}", hoja + 1, hojas = hojas).into());
            ventana.set_nombre_de_hoja_propio(propio.is_some());
            ventana.set_nombre_de_hoja(propio.as_deref().unwrap_or(&de_siempre).into());
            ventana.set_nombre_de_hoja_de_siempre(de_siempre.as_str().into());
            ventana.set_hay_hoja_anterior(hoja > 0);
            ventana.set_hay_hoja_siguiente(hoja + 1 < hojas);
            ventana.set_puede_quitar_hoja(hojas > 1);
            ventana.set_efecto_en_espera(en_espera.is_some());
        });
    }

    fn refrescar_estado_de_botonera(&mut self) {
        if self.aviso_de_botonera.as_ref().is_some_and(|(_, momento)| momento.elapsed() > DURACION_DE_AVISO_DE_BOTONERA) {
            self.aviso_de_botonera = None;
        }
        let lista = &self.lista;
        let ventana_de = |pista: &Pista| lista.puntos_de(pista).overlay();
        let actividad = if let Some((aviso, _)) = &self.aviso_de_botonera {
            aviso.clone()
        } else if let Some(en_espera) = self.overlay.en_espera() {
            let detalle = match self.overlay.estado_de_espera(&self.gestor, &ventana_de) {
                Some(EstadoDeEspera::Faltan(segundos)) => tr!(" (faltan {:.1} s)", segundos.max(0.0)),
                Some(EstadoDeEspera::ProximaPista) => tr!(" (espera la ventana de la proxima pista)").into(),
                None => String::new(),
            };
            tr!("En espera: {}{detalle}", nombre_sin_extension(en_espera), detalle = detalle)
        } else if let Some(sonando) = self.overlay.sonando() {
            tr!("Sonando: {}", nombre_sin_extension(sonando))
        } else {
            String::new()
        };
        // La actividad tambien se ve junto al boton Efectos, por si el panel
        // esta oculto; sin actividad, el panel muestra una ayuda.
        let texto = if actividad.is_empty() { tr!("Clic derecho en un boton para cambiar su archivo").into() } else { actividad.clone() };
        self.con_ventana(|ventana| {
            ventana.set_estado_de_botonera(texto.into());
            ventana.set_actividad_de_efectos(actividad.into());
        });
    }
}

/// Mide como suena la musica en un tramo de un tema, para ajustar el volumen
/// de un efecto y cuanto baja la musica.
struct MedidorDeMusica<'a> {
    datos: &'a HashMap<PathBuf, DatosDeMezcla>,
    nivelar: bool,
    objetivo: ObjetivoDeSonoridad,
    volumen: f64,
    onda: &'a RefCell<Option<(PathBuf, Rc<FormaDeOnda>)>>,
}

impl MedidorDeMusica<'_> {
    /// Sonoridad del tema tal como se escucha (con la nivelacion y el volumen
    /// elegido) y cuanto se aparta de lo tipico el tramo [desde, desde + segundos].
    fn medir(&self, pista: &Pista, desde: f64, segundos: f64) -> Option<MusicaAlrededor> {
        let datos = self.datos.get(pista.archivo());
        let nivelacion = datos.filter(|_| self.nivelar).map_or(0.0, |datos| 20.0 * datos.ganancia_de_nivelacion(self.objetivo).log10());
        let sonoridad = datos.and_then(|datos| datos.sonoridad).map(|sonoridad| sonoridad + nivelacion + 20.0 * self.volumen.max(0.01).log10());
        let desvio_local_db = self.onda_de(pista).and_then(|onda| onda.desvio_local_db(desde, desde + segundos.max(0.5))).unwrap_or(0.0);
        Some(MusicaAlrededor { sonoridad, desvio_local_db })
    }

    /// Forma de onda del tema, solo de la cache (se recuerda la ultima): se
    /// pide en el hilo de la interfaz al pulsar un efecto, y calcularla ahi
    /// congelaria la ventana. El analisis de la lista ya la deja guardada.
    fn onda_de(&self, pista: &Pista) -> Option<Rc<FormaDeOnda>> {
        if let Some((archivo, onda)) = self.onda.borrow().as_ref() {
            if archivo.as_path() == pista.archivo() {
                return Some(onda.clone());
            }
        }
        let onda = Rc::new(FormaDeOnda::de_la_cache(pista.archivo(), &carpeta_del_usuario().join("ondas"))?);
        *self.onda.borrow_mut() = Some((pista.archivo().to_path_buf(), onda.clone()));
        Some(onda)
    }
}

/// Analiza en otro hilo los efectos de la botonera que todavia no tienen su
/// forma de onda en la cache, para que al pulsarlos no haya demora.
fn preparar_efectos(archivos: Vec<PathBuf>) {
    if archivos.is_empty() {
        return;
    }
    let cache = carpeta_del_usuario().join("ondas");
    let lanzado = std::thread::Builder::new().name("analisis-de-efectos".into()).spawn(move || {
        fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Fondo);
        for archivo in archivos.iter().filter(|archivo| archivo.is_file()) {
            if let Err(error) = FormaDeOnda::cargar_o_calcular(archivo, &cache) {
                log::warn!("No se pudo analizar el efecto {}: {error}", archivo.display());
            }
        }
    });
    if let Err(error) = lanzado {
        log::warn!("No se pudo iniciar el analisis de los efectos: {error}");
    }
}

fn si_no(valor: bool) -> &'static str {
    if valor { "si" } else { "no" }
}

/// Reemplaza el contenido de un modelo tocando solo las filas distintas.
fn actualizar_modelo<T: Clone + PartialEq + 'static>(modelo: &VecModel<T>, nuevas: Vec<T>) {
    if modelo.row_count() != nuevas.len() {
        modelo.set_vec(nuevas);
        return;
    }
    for (indice, nueva) in nuevas.into_iter().enumerate() {
        if modelo.row_data(indice).as_ref() != Some(&nueva) {
            modelo.set_row_data(indice, nueva);
        }
    }
}

/// Barras de nivel de la vista de onda del panel de detalles.
const BARRAS_DE_LA_ONDA: usize = 200;

impl Controlador {
    /// Boton del panel de detalles: alterna entre los datos y la onda.
    fn alternar_vista_del_detalle(&mut self) {
        let activo = !self.repositorio_de_preferencias.ver_onda();
        self.repositorio_de_preferencias.guardar_ver_onda(activo);
        self.con_ventana(|ventana| ventana.set_ver_onda(activo));
        *self.onda_del_detalle.borrow_mut() = None;
        self.refrescar_detalles();
    }

    fn sin_barras(&self) -> bool {
        self.ventana.upgrade().is_none_or(|ventana| ventana.get_barras().row_count() == 0)
    }

    /// Pide las barras de nivel del tema del panel, si cambio. Se leen de la
    /// cache de formas de onda (la que deja el analisis de la lista) en otro
    /// hilo: no se decodifica el audio ni se traba la interfaz.
    fn refrescar_onda_del_detalle(&self, archivo: Option<&Path>) {
        if !self.repositorio_de_preferencias.ver_onda() {
            return;
        }
        if self.onda_del_detalle.borrow().as_deref() == archivo {
            return;
        }
        *self.onda_del_detalle.borrow_mut() = archivo.map(Path::to_path_buf);
        let generacion = self.generacion_de_barras.get() + 1;
        self.generacion_de_barras.set(generacion);
        let sin_onda = archivo.is_none_or(|archivo| es_marcador(archivo) || es_anuncio_hth(archivo) || !archivo.is_file());
        let estado = match archivo {
            None => tr!("Sin pista seleccionada"),
            Some(_) if sin_onda => tr!("Sin forma de onda"),
            Some(_) => tr!("Cargando la forma de onda..."),
        };
        let suena = archivo.is_some() && archivo == self.gestor.pista_actual().map(Pista::archivo);
        self.con_ventana(|ventana| {
            ventana.set_barras(ModelRc::new(VecModel::<BarraDeNivel>::default()));
            ventana.set_datos_tecnicos("".into());
            ventana.set_estado_de_la_onda(estado.into());
            ventana.set_onda_de_la_que_suena(suena);
        });
        let Some(archivo) = archivo.filter(|_| !sin_onda).map(Path::to_path_buf) else {
            return;
        };
        let cache = carpeta_del_usuario().join("ondas");
        let lanzado = std::thread::Builder::new().name("barras-de-nivel".into()).spawn(move || {
            fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Fondo);
            let onda = FormaDeOnda::de_la_cache(&archivo, &cache);
            let tecnicos = Decodificador::datos_tecnicos(&archivo, onda.as_ref().map(FormaDeOnda::duracion))
                .map(|datos| datos.describir())
                .unwrap_or_default();
            let barras = onda.map(|onda| (onda.barras_de_nivel(BARRAS_DE_LA_ONDA), onda.duracion()));
            programar_en_interfaz(move |controlador| controlador.recibir_barras(generacion, barras, tecnicos));
        });
        if let Err(error) = lanzado {
            log::warn!("No se pudo iniciar la lectura de la onda: {error}");
        }
    }

    fn recibir_barras(&mut self, generacion: u64, barras: Option<(Vec<(f32, f32)>, f64)>, tecnicos: String) {
        if generacion != self.generacion_de_barras.get() {
            return;
        }
        self.con_ventana(|ventana| ventana.set_datos_tecnicos(tecnicos.into()));
        self.con_ventana(|ventana| match barras {
            Some((barras, duracion)) => {
                let modelo: Vec<BarraDeNivel> = barras.into_iter().map(|(pico, rms)| BarraDeNivel { pico, rms }).collect();
                ventana.set_duracion_de_la_onda(duracion as f32);
                ventana.set_barras(ModelRc::new(VecModel::from(modelo)));
            }
            // Todavia no esta en la cache: aparece al terminar el analisis.
            None => ventana.set_estado_de_la_onda(tr!("Analizando el tema: la forma de onda aparece al terminar.").into()),
        });
    }
}

impl Controlador {
    /// Marcas del tema de la onda del panel (overlay, outro y la mezcla con
    /// la siguiente), las mismas que pinta la barra de progreso.
    fn refrescar_marcas_de_la_onda(&self, pista: Option<&Pista>) {
        if !self.repositorio_de_preferencias.ver_onda() {
            return;
        }
        let es_la_que_suena = pista.is_some() && pista.map(Pista::archivo) == self.gestor.pista_actual().map(Pista::archivo);
        let duracion = pista.and_then(|pista| {
            if es_la_que_suena { self.gestor.duracion_total() } else { self.ficha_de(pista).duracion }
        });
        let marcas = match (pista, duracion.filter(|duracion| *duracion > 0.0)) {
            (Some(pista), Some(duracion)) => {
                let mut marcas = MarcasDeProgreso::de(&self.lista.puntos_de(pista), duracion);
                if !(es_la_que_suena && self.gestor.hay_transicion_en_curso()) {
                    let inteligente = self.crossfade_inteligente();
                    let transicion =
                        transicion_desde(&self.lista, &self.datos_de_mezcla, &self.repositorio_de_canciones, inteligente, pista);
                    if let Some(transicion) = transicion.filter(|transicion| transicion.segundos > 0.0) {
                        marcas.mezcla = MarcasDeProgreso::tramo_de_mezcla(&transicion, duracion);
                    }
                }
                marcas
            }
            _ => MarcasDeProgreso::NINGUNA,
        };
        self.con_ventana(|ventana| {
            ventana.set_onda_overlay_inicio(marcas.overlay.0);
            ventana.set_onda_overlay_fin(marcas.overlay.1);
            ventana.set_onda_outro_inicio(marcas.outro.0);
            ventana.set_onda_outro_fin(marcas.outro.1);
            ventana.set_onda_overlay_manual(marcas.overlay_manual);
            ventana.set_onda_outro_manual(marcas.outro_manual);
            ventana.set_onda_mezcla_inicio(marcas.mezcla.0);
            ventana.set_onda_mezcla_fin(marcas.mezcla.1);
        });
    }
}

impl Controlador {
    /// "Titulo - Artista" de una pista (o el nombre del archivo, si no tiene
    /// titulo), sin la posicion en la lista.
    fn titulo_y_artista(&self, pista: &Pista) -> String {
        let ficha = self.ficha_de(pista);
        let titulo = ficha.titulo.clone().unwrap_or_else(|| pista.nombre());
        match ficha.artista.as_deref().filter(|artista| !artista.trim().is_empty()) {
            Some(artista) => format!("{titulo} - {artista}"),
            None => titulo,
        }
    }
}

#[cfg(test)]
mod pruebas_de_la_reproduccion {
    use super::{formato_de_la_reproduccion, leer_reproduccion, reproduccion_para_la_lista, texto_de_reproduccion};

    fn zona(nombre: &str) -> jiff::tz::TimeZone {
        jiff::tz::TimeZone::get(nombre).unwrap()
    }

    #[test]
    fn la_reproduccion_se_muestra_con_segundos_en_la_hora_de_la_localidad() {
        let momento = "2026-11-23T11:13:05Z";
        assert_eq!(reproduccion_para_la_lista(momento, &zona("America/Argentina/Buenos_Aires")), "23/11/26 08:13:05");
        assert_eq!(reproduccion_para_la_lista(momento, &zona("Asia/Tokyo")), "23/11/26 20:13:05");
        assert_eq!(texto_de_reproduccion(momento, &zona("Asia/Tokyo"), &formato_de_la_reproduccion()), "23/11/2026 20:13:05");
        assert_eq!(reproduccion_para_la_lista("cualquier cosa", &zona("Asia/Tokyo")), "");
    }

    #[test]
    fn la_reproduccion_escrita_a_mano_se_guarda_en_utc() {
        let ahora: jiff::Timestamp = "2026-11-23T12:00:00Z".parse().unwrap();
        let buenos_aires = zona("America/Argentina/Buenos_Aires");
        assert_eq!(leer_reproduccion("23/11/2026 08:13:05", &buenos_aires, ahora), Ok(Some("2026-11-23T11:13:05Z".into())));
        // Sin los segundos tambien vale; vacio es "nunca sono".
        assert_eq!(leer_reproduccion(" 23/11/2026 08:13 ", &buenos_aires, ahora), Ok(Some("2026-11-23T11:13:00Z".into())));
        assert_eq!(leer_reproduccion("  ", &buenos_aires, ahora), Ok(None));
        // Lo que se escribe es hora de la localidad.
        assert_eq!(leer_reproduccion("23/11/2026 20:13:05", &zona("Asia/Tokyo"), ahora), Ok(Some("2026-11-23T11:13:05Z".into())));
        assert!(leer_reproduccion("ayer", &buenos_aires, ahora).is_err());
        assert!(leer_reproduccion("31/02/2026 10:00:00", &buenos_aires, ahora).is_err());
        assert!(leer_reproduccion("24/11/2026 10:00:00", &buenos_aires, ahora).is_err(), "futura");
    }
}
