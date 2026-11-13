//! Capturas de las ventanas dibujadas fuera de pantalla, con el renderizador
//! por software, para revisar a ojo que no se corten ni se solapen controles
//! en las resoluciones que se soportan (Full HD al 100 % como minimo).
//!
//! No corre con las demas pruebas: se pide a mano y deja los PNG en
//! target/capturas.
//!
//! ```text
//! cargo test capturas -- --ignored --test-threads=1
//! ```

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, Rgb8Pixel, SharedString, VecModel};

use crate::{
    AcercaDe, AnunciantesDeLaEmisora, AvisoDeTanda, FilaDeAnunciante, AvisoDelCatalogo, BotonDeEfecto, BuscadorDelCatalogo, EditorDePista, FichaDelCatalogo, FichaSinArchivo, FilaDePista,
    FilaDeOrden, FilaDeTanda, LineaDePauta, MantenimientoDelCatalogo, OrdenesDePublicidad, PautaDeTandas, SelectorDeAvisos, SelectorDeLocalidad,
    VentanaPrincipal,
};

thread_local! {
    // Ventanas que crea la plataforma, en orden de creacion.
    static VENTANAS: RefCell<Vec<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(Vec::new()) };
}

struct PlataformaDeCaptura;

impl Platform for PlataformaDeCaptura {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let ventana = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        VENTANAS.with(|ventanas| ventanas.borrow_mut().push(ventana.clone()));
        Ok(ventana)
    }
}

/// Resolucion de pantalla a probar: area de la ventana en pixeles fisicos y
/// escala de Windows. El alto descuenta la barra de tareas y el titulo de la
/// ventana maximizada.
struct Pantalla {
    nombre: &'static str,
    ancho: u32,
    alto: u32,
    escala: f32,
}

const PANTALLAS: [Pantalla; 7] = [
    // Tamano minimo de la ventana principal (sin maximizar).
    Pantalla { nombre: "minima", ancho: 1180, alto: 600, escala: 1.0 },
    Pantalla { nombre: "fhd-100", ancho: 1920, alto: 1000, escala: 1.0 },
    Pantalla { nombre: "fhd-125", ancho: 1920, alto: 980, escala: 1.25 },
    Pantalla { nombre: "fhd-150", ancho: 1920, alto: 960, escala: 1.5 },
    Pantalla { nombre: "qhd-100", ancho: 2560, alto: 1360, escala: 1.0 },
    Pantalla { nombre: "qhd-125", ancho: 2560, alto: 1350, escala: 1.25 },
    Pantalla { nombre: "uhd-150", ancho: 3840, alto: 2040, escala: 1.5 },
];

fn carpeta_de_capturas() -> PathBuf {
    let carpeta = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("capturas");
    std::fs::create_dir_all(&carpeta).expect("carpeta de capturas");
    carpeta
}

/// Dibuja la ventana con ese tamano y escala, y guarda la imagen completa.
fn capturar(adaptador: &MinimalSoftwareWindow, ancho: u32, alto: u32, escala: f32, nombre: &str) {
    adaptador.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: escala });
    adaptador.set_size(slint::PhysicalSize::new(ancho, alto));
    let mut pixeles = vec![Rgb8Pixel::default(); (ancho * alto) as usize];
    // Dos pasadas: la primera acomoda los layouts y las listas.
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        adaptador.request_redraw();
        adaptador.draw_if_needed(|renderizador| {
            renderizador.render(&mut pixeles, ancho as usize);
        });
    }
    let archivo = carpeta_de_capturas().join(format!("{nombre}.png"));
    let mut codificador = png::Encoder::new(std::fs::File::create(&archivo).expect("archivo de captura"), ancho, alto);
    codificador.set_color(png::ColorType::Rgb);
    codificador.set_depth(png::BitDepth::Eight);
    let datos: Vec<u8> = pixeles.iter().flat_map(|pixel| [pixel.r, pixel.g, pixel.b]).collect();
    codificador.write_header().expect("encabezado PNG").write_image_data(&datos).expect("imagen PNG");
}

fn texto(valor: &str) -> SharedString {
    valor.into()
}

/// Datos de muestra largos, para que se note lo que no entra.
fn llenar_ventana(ventana: &VentanaPrincipal) {
    let filas: Vec<FilaDePista> = (0..40)
        .map(|indice| FilaDePista {
            posicion: texto(&(indice + 1).to_string()),
            titulo: texto(&format!("Tema numero {} con un titulo bastante largo (Remix extendido)", indice + 1)),
            artista: texto("Orquesta Sinfonica y Coro de la Ciudad"),
            ruta: texto("D:\\musica\\tema.mp3"),
            overlay: texto("27.5"),
            ventana_de_overlay: texto("1:05.3 - 1:32.8"),
            mezcla: texto("12.5"),
            duracion: texto("12:34.5"),
            tipo: texto(match indice % 7 {
                3 => "Comercial",
                5 => "Seco",
                _ => "Tema",
            }),
            nota: texto(if indice % 4 == 0 { "Version editada para radio" } else { "" }),
            ranking: texto(match indice % 7 {
                3 | 5 => "",
                _ => "★★★★☆",
            }),
            genero: texto(match indice % 7 {
                3 | 5 => "",
                _ => "Rock alternativo latino",
            }),
            anio: texto(if indice % 7 == 3 { "" } else { "1999" }),
            bpm: texto("128"),
            seleccionada: indice == 2,
            overlay_manual: indice % 2 == 0,
            fijado: indice % 5 == 1,
            reservado: match indice % 7 {
                3 => 1,
                5 => 5,
                _ => 0,
            },
            coincide: false,
            sonando: indice == 2,
            hth: 0,
            marcador: 0,
            en_bloque_fijo: false,
        })
        .collect();
    let mut filas = filas;
    // Un anuncio HTH sobre una base y otro con la voz sola.
    for (posicion, hth, titulo) in [(6, 2, "HTH · Base 1"), (8, 1, "Hora, temperatura y humedad")] {
        let fila = &mut filas[posicion];
        fila.titulo = texto(titulo);
        fila.artista = texto(if hth == 2 { "Radio Planet" } else { "" });
        fila.tipo = texto("HTH");
        fila.genero = texto(if hth == 2 { "Pop" } else { "" });
        fila.anio = texto("");
        fila.nota = texto("");
        if hth == 1 {
            fila.ranking = texto("");
        }
        fila.duracion = texto(if hth == 2 { "0:31.9" } else { "0:09.0" });
        fila.reservado = 6;
        fila.fijado = false;
        fila.hth = hth;
    }
    // Un bloque fijo con dos temas.
    for (posicion, marcador, titulo) in [(13, 1, "Inicio de bloque fijo"), (16, 2, "Fin de bloque fijo")] {
        let fila = &mut filas[posicion];
        fila.titulo = texto(titulo);
        fila.artista = texto("Mezclar y BPM no cambian lo que queda dentro");
        fila.tipo = texto("Bloque fijo");
        fila.genero = texto("");
        for campo in [&mut fila.duracion, &mut fila.mezcla, &mut fila.bpm, &mut fila.overlay, &mut fila.ventana_de_overlay, &mut fila.ranking, &mut fila.nota] {
            *campo = texto("");
        }
        fila.reservado = 0;
        fila.fijado = false;
        fila.marcador = marcador;
    }
    for fila in &mut filas[13..=16] {
        fila.en_bloque_fijo = true;
    }
    ventana.set_filas(ModelRc::new(VecModel::from(filas)));
    let botones: Vec<BotonDeEfecto> = (0..10)
        .map(|indice| BotonDeEfecto {
            // Largo a proposito: tiene que cortarse en dos renglones.
            texto: texto(&format!("{indice}  Efecto con un nombre muy largo de prueba para la cortina de la tarde")),
            ayuda: texto("Efecto"),
            estado: i32::from(indice == 1) + 2 * i32::from(indice == 4),
            duracion: texto("12.3 s"),
            vacio: indice == 9,
            insertable: indice != 9,
            con_nota: indice == 2,
        })
        .collect();
    ventana.set_botones(ModelRc::new(VecModel::from(botones)));
    ventana.set_nombre_de_lista(texto("Programa de la tarde *"));
    ventana.set_pista_actual(texto("Orquesta Sinfonica y Coro de la Ciudad - Tema numero 3 con un titulo bastante largo"));
    // El formato mas largo del tiempo del tema.
    ventana.set_tiempo(texto("-1:02:33.4 / 1:02:33.4"));
    ventana.set_progreso_maximo(300.0);
    ventana.set_progreso(120.0);
    ventana.set_progreso_habilitado(true);
    ventana.set_progreso_overlay_inicio(60.0);
    ventana.set_progreso_overlay_fin(90.0);
    ventana.set_progreso_outro_inicio(270.0);
    ventana.set_progreso_outro_fin(298.0);
    // Mezcla efectiva de 8 s, al final de la outro.
    ventana.set_progreso_mezcla_inicio(290.0);
    ventana.set_progreso_mezcla_fin(298.0);
    ventana.set_anterior_habilitado(true);
    ventana.set_siguiente_habilitado(true);
    ventana.set_mezclando(true);
    ventana.set_proxima_tanda(texto("Proxima tanda: «Tanda A» 10:30 (en 4:12)"));
    ventana.set_clima_hth(texto("Buenos Aires: 13,9 °C, humedad 84 %
Ultima lectura hace 4 min"));
    ventana.set_clima_del_reloj(texto("13,9 °C · humedad 84 % · Buenos Aires"));
    ventana.set_deteniendo(true);
    ventana.set_texto_reproducir(texto("Reproducir"));
    ventana.set_analisis_de_mezcla(texto("Analizando 12 tema(s) para la mezcla y la nivelacion (a -11 LUFS)..."));
    ventana.set_total_de_lista(texto("Total 12:34:56  ·  Restante 11:22:33  (+3 sin duracion)"));
    ventana.set_detalle_archivo(texto("Tema numero 3 (Remix extendido).flac"));
    ventana.set_detalle_carpeta(texto("D:\\musica\\coleccion\\rock alternativo\\Orquesta Sinfonica"));
    ventana.set_detalle_genero(texto("Rock alternativo latino"));
    ventana.set_detalle_tipo(texto("Tema"));
    ventana.set_detalle_album(texto("Grandes exitos de la orquesta, volumen tres (edicion de lujo)"));
    ventana.set_detalle_anio(texto("1999"));
    ventana.set_detalle_efecto(texto("Hoja 1 · 4: Aplausos del publico en vivo.mp3"));
    ventana.set_detalle_ranking(texto("★★★★☆"));
    ventana.set_detalle_ultima(texto("02/11/2026 14:05:33"));
    ventana.set_detalle_hace(texto("Hace 3 horas"));
    ventana.set_fecha_actual(texto("Miercoles 30/09/2026"));
    ventana.set_hora_actual(texto("23:59:59"));
    ventana.set_hay_seleccion(true);
    ventana.set_hoja_de_efectos(texto("Hoja 12 de 12"));
    ventana.set_hay_hoja_anterior(true);
    ventana.set_puede_quitar_hoja(true);
    ventana.set_estado(texto(
        "Mezclando hacia Orquesta Sinfonica y Coro de la Ciudad - Tema numero 4 con un titulo bastante largo (5 s)",
    ));
    ventana.set_estado_de_botonera(texto("Boton 4: Aplausos del publico en vivo espera la ventana de overlay (faltan 12.5 s)."));
    ventana.set_buscando(true);
    ventana.set_resultado_de_busqueda(texto("12 de 340"));
    ventana.set_actividad_de_efectos(texto("En espera: Aplausos del publico en vivo (faltan 12.5 s)"));
    ventana.set_efecto_en_espera(true);
}

fn llenar_editor(editor: &EditorDePista) {
    editor.set_pista(texto("Tema numero 3 con un titulo bastante largo (Remix extendido) - Orquesta Sinfonica"));
    editor.set_ruta(texto("D:\\musica\\coleccion\\rock alternativo\\Orquesta Sinfonica\\Tema numero 3 (Remix extendido).flac"));
    editor.set_duracion(245.0);
    editor.set_vista_hasta(245.0);
    editor.set_cursor(80.0);
    editor.set_inicio(0.4);
    editor.set_fin(243.0);
    editor.set_intro(12.0);
    editor.set_outro(20.0);
    editor.set_tiene_overlay(true);
    editor.set_overlay_inicio(65.0);
    editor.set_overlay_fin(92.5);
    editor.set_overlay_manual(true);
    editor.set_hay_automaticos(true);
    editor.set_puntos_manuales(true);
    editor.set_sugerencia_de_overlay(texto(
        "Sugerida: 1:05.0 a 1:32.5 (27.5 s), en el tramo instrumental mas parejo despues del primer estribillo.",
    ));
    editor.set_titulo(texto("Tema numero 3 con un titulo bastante largo (Remix extendido)"));
    editor.set_artista(texto("Orquesta Sinfonica y Coro de la Ciudad"));
    editor.set_genero(texto("Rock alternativo latino"));
    // Tipo Tema (0 es sin clasificar).
    editor.set_tipo(1);
    editor.set_tipo_anterior(1);
    editor.set_album(texto("Grandes exitos de la orquesta, volumen tres"));
    editor.set_nota(texto("Version editada para radio: sin el solo de guitarra del final"));
    editor.set_anio(texto("1999"));
    editor.set_bpm(texto("128.5"));
    let generos = ["Balada", "Cumbia", "Pop", "Rock", "Rock alternativo latino"].map(texto);
    editor.set_generos_sugeridos(ModelRc::new(VecModel::from(generos.to_vec())));
    let fijables = ["(ninguno)", "Hoja 1 · 4: Aplausos del publico en vivo.mp3"].map(texto);
    editor.set_efectos_fijables(ModelRc::new(VecModel::from(fijables.to_vec())));
    editor.set_efecto_fijado(1);
    editor.set_efectos(ModelRc::new(VecModel::from(vec![texto("Hoja 1 · 4: Aplausos del publico en vivo.mp3")])));
    editor.set_puede_preescuchar(true);
    editor.set_salidas_de_preescucha(ModelRc::new(VecModel::from(vec![
        texto("Misma salida de la musica (con la musica detenida)"),
        texto("Audifono de los auriculares con microfono (4- Microsoft LifeChat LX-3000 )"),
        texto("Altavoces (FxSound Audio Enhancer)"),
    ])));
    editor.set_salida_de_preescucha(1);
    editor.set_preescucha_aparte(true);
    editor.set_volumen_de_preescucha(0.65);
    editor.set_plan_del_efecto(texto("Efecto a -3.5 dB, musica al 42 % en 40 ms y vuelve en 0.8 s"));
    editor.set_aviso(texto("Los cambios se aplican al guardar."));
}

#[test]
#[ignore = "genera capturas en target/capturas; se pide a mano"]
fn capturas_de_las_ventanas() {
    slint::platform::set_platform(Box::new(PlataformaDeCaptura)).expect("plataforma de captura");
    let ventana = VentanaPrincipal::new().expect("ventana principal");
    llenar_ventana(&ventana);
    ventana.show().expect("mostrar ventana");
    let editor = EditorDePista::new().expect("editor");
    llenar_editor(&editor);
    editor.show().expect("mostrar editor");
    let mantenimiento = MantenimientoDelCatalogo::new().expect("mantenimiento");
    llenar_mantenimiento(&mantenimiento);
    mantenimiento.show().expect("mostrar mantenimiento");
    let acerca = AcercaDe::new().expect("acerca de");
    acerca.set_version(texto(env!("CARGO_PKG_VERSION")));
    acerca.set_correo(texto("jxlfybcrk@mozmail.com"));
    acerca.show().expect("mostrar acerca de");
    let buscador = BuscadorDelCatalogo::new().expect("buscador");
    llenar_buscador(&buscador);
    buscador.show().expect("mostrar buscador");
    let pauta = PautaDeTandas::new().expect("pauta");
    llenar_pauta(&pauta);
    pauta.show().expect("mostrar pauta");
    let ordenes = OrdenesDePublicidad::new().expect("ordenes");
    llenar_ordenes(&ordenes);
    ordenes.show().expect("mostrar ordenes");
    let selector = SelectorDeAvisos::new().expect("selector de avisos");
    llenar_selector_de_avisos(&selector);
    selector.show().expect("mostrar selector de avisos");
    let anunciantes = AnunciantesDeLaEmisora::new().expect("anunciantes");
    llenar_anunciantes(&anunciantes);
    anunciantes.show().expect("mostrar anunciantes");
    let (principal, del_editor, del_mantenimiento, del_acerca, del_buscador, de_la_pauta, de_las_ordenes, del_selector) = VENTANAS.with(|ventanas| {
        let ventanas = ventanas.borrow();
        (
            ventanas[0].clone(),
            ventanas[1].clone(),
            ventanas[2].clone(),
            ventanas[3].clone(),
            ventanas[4].clone(),
            ventanas[5].clone(),
            ventanas[6].clone(),
            ventanas[7].clone(),
        )
    });
    let de_los_anunciantes = VENTANAS.with(|ventanas| ventanas.borrow()[8].clone());
    let localidad = SelectorDeLocalidad::new().expect("selector de localidad");
    llenar_selector_de_localidad(&localidad);
    localidad.show().expect("mostrar selector de localidad");
    let del_selector_de_localidad = VENTANAS.with(|ventanas| ventanas.borrow()[9].clone());
    for (ancho, alto, tamano) in [(680, 480, "inicial"), (520, 360, "minimo")] {
        capturar(&del_selector_de_localidad, ancho, alto, 1.0, &format!("localidad-{tamano}"));
    }
    for (ancho, alto, tamano) in [(1180, 780, "inicial"), (980, 640, "minimo")] {
        capturar(&de_los_anunciantes, ancho, alto, 1.0, &format!("anunciantes-{tamano}"));
    }
    for (ancho, alto, tamano) in [(980, 620, "inicial"), (780, 420, "minimo")] {
        capturar(&del_selector, ancho, alto, 1.0, &format!("avisos-{tamano}"));
    }
    for (ancho, alto, tamano) in [(1240, 860, "inicial"), (1000, 720, "minimo")] {
        capturar(&de_las_ordenes, ancho, alto, 1.0, &format!("ordenes-{tamano}"));
    }
    for (ancho, alto, tamano) in [(1180, 760, "inicial"), (960, 640, "minimo")] {
        capturar(&de_la_pauta, ancho, alto, 1.0, &format!("pauta-{tamano}"));
    }
    for (ancho, alto, tamano) in [(1000, 620, "inicial"), (900, 380, "minimo")] {
        capturar(&del_buscador, ancho, alto, 1.0, &format!("buscador-{tamano}"));
    }
    capturar(&del_acerca, 560, 600, 1.0, "acerca-de");
    for pantalla in &PANTALLAS {
        for botonera in [false, true] {
            ventana.set_botonera_visible(botonera);
            let sufijo = if botonera { "-botonera" } else { "" };
            capturar(&principal, pantalla.ancho, pantalla.alto, pantalla.escala, &format!("principal-{}{sufijo}", pantalla.nombre));
        }
    }
    // Con la preferencia de ocultar los textos de los botones.
    ventana.set_ocultar_textos(true);
    for pantalla in PANTALLAS.iter().filter(|pantalla| ["minima", "fhd-100"].contains(&pantalla.nombre)) {
        capturar(&principal, pantalla.ancho, pantalla.alto, pantalla.escala, &format!("principal-{}-sin-textos", pantalla.nombre));
    }
    ventana.set_ocultar_textos(false);
    // Con los ajustes de los efectos desplegados (con la ventana baja, la
    // botonera se desplaza).
    ventana.set_botonera_visible(true);
    ventana.set_ajustes_de_efectos_visibles(true);
    for pantalla in PANTALLAS.iter().filter(|pantalla| ["fhd-100", "fhd-150"].contains(&pantalla.nombre)) {
        capturar(&principal, pantalla.ancho, pantalla.alto, pantalla.escala, &format!("principal-{}-ajustes", pantalla.nombre));
    }
    ventana.set_ajustes_de_efectos_visibles(false);
    // El editor: tamano inicial y minimo, en cada solapa.
    for (ancho, alto, tamano) in [(1180, 620, "inicial"), (1080, 560, "minimo")] {
        for solapa in 0..3 {
            editor.set_solapa(solapa);
            capturar(&del_editor, ancho, alto, 1.0, &format!("editor-{tamano}-solapa{solapa}"));
        }
    }
    for (ancho, alto, tamano) in [(1100, 600, "inicial"), (900, 420, "minimo")] {
        capturar(&del_mantenimiento, ancho, alto, 1.0, &format!("mantenimiento-{tamano}"));
    }
}

fn llenar_buscador(ventana: &BuscadorDelCatalogo) {
    let ficha = |titulo: &str, artista: &str, genero: &str, anio: &str, duracion: &str, en_lista: bool, seleccionada: bool| {
        FichaDelCatalogo {
            titulo: texto(titulo),
            artista: texto(artista),
            tipo: texto("Tema"),
            genero: texto(genero),
            anio: texto(anio),
            duracion: texto(duracion),
            sonado: texto(if en_lista { "20/09/2026" } else if seleccionada { "14/08/2026" } else { "02/09/2026" }),
            ruta: texto("D:\\musica\\tema.mp3"),
            en_lista,
            sin_archivo: false,
            seleccionada,
        }
    };
    let mut fichas = vec![
        ficha("Amiga mia", "Alejandro Sanz", "Pop latino", "1997", "4:21.3", true, false),
        ficha("Corazon partio", "Alejandro Sanz", "Pop latino", "1997", "5:47.8", false, true),
        ficha("Y si fuera ella", "Alejandro Sanz", "Pop latino", "1997", "5:08.0", false, true),
        ficha("Mi soledad y yo", "Alejandro Sanz", "Pop latino", "1997", "4:36.1", false, false),
        ficha("El alma al aire", "Alejandro Sanz", "Pop latino", "2000", "4:41.5", false, false),
    ];
    let mut perdida = ficha("Quisiera ser", "Alejandro Sanz", "Pop latino", "2000", "4:52.6", false, false);
    perdida.sin_archivo = true;
    fichas.push(perdida);
    ventana.set_fichas(ModelRc::new(VecModel::from(fichas)));
    ventana.set_texto(texto("artista:sanz sonado:>30d"));
    ventana.set_resumen(texto("6 tema(s) coinciden"));
    ventana.set_seleccionadas(2);
}

fn llenar_anunciantes(ventana: &AnunciantesDeLaEmisora) {
    let fila = |nombre: &str, razon: &str, cuit: &str, rubro: &str, contacto: &str, ordenes: &str, estado: &str, sin_ficha: bool, elegida: bool| {
        FilaDeAnunciante {
            nombre: texto(nombre),
            razon_social: texto(razon),
            cuit: texto(cuit),
            rubro: texto(rubro),
            contacto: texto(contacto),
            ordenes: texto(ordenes),
            estado: texto(estado),
            sin_ficha,
            activo: true,
            seleccionada: elegida,
        }
    };
    ventana.set_anunciantes(ModelRc::new(VecModel::from(vec![
        fila("Concesionaria Ruta 3", "Ruta 3 Automotores S.R.L.", "30-71234567-1", "Autos", "Marcos Diaz", "1 activa(s) de 1", "", false, false),
        fila("Supermercado Sol", "Sol S.A.", "30-71234567-1", "Supermercados", "Ana Perez", "2 activa(s) de 3", "", false, true),
        fila("Farmacia Centro", "", "", "Farmacias", "", "ninguna", "falta razon social, CUIT, telefono o correo", false, false),
        fila("Almacen Luna", "", "", "", "", "1 activa(s) de 1", "sin ficha: elijalo y carguela", true, false),
    ])));
    ventana.set_ordenes_del_anunciante(ModelRc::new(VecModel::from(vec![
        LineaDePauta { texto: texto("Sol - Noviembre: cada 2 h de 08:00 a 20:00, lun a vie, del 2026-11-01 al 2026-11-30"), problema: false },
        LineaDePauta { texto: texto("Sol - Fin de semana: 3 por dia de 09:00 a 13:00, sab y dom, del 2026-11-01 al 2026-11-30"), problema: false },
        LineaDePauta { texto: texto("Sol - Octubre (inactiva): cada 1 h de 08:00 a 20:00, lun a vie, del 2026-10-01 al 2026-10-31"), problema: false },
    ])));
    ventana.set_resumen(texto("3 anunciante(s) con ficha; 1 nombrado(s) en ordenes SIN FICHA; 1 con datos de facturacion incompletos."));
    ventana.set_editando(true);
    ventana.set_nombre(texto("Supermercado Sol"));
    ventana.set_razon_social(texto("Sol S.A."));
    ventana.set_cuit(texto("30-71234567-1"));
    ventana.set_rubro(texto("Supermercados"));
    ventana.set_contacto(texto("Ana Perez"));
    ventana.set_telefono(texto("011 4555-1234"));
    ventana.set_correo(texto("ventas@sol.example"));
    ventana.set_direccion(texto("Av. Siempreviva 742, Springfield"));
    ventana.set_agencia(texto("Agencia Norte"));
    ventana.set_notas(texto("Factura a 30 dias"));
    ventana.set_activo(true);
    ventana.set_aviso(texto("Anunciante «Supermercado Sol» actualizado."));
}

fn llenar_selector_de_localidad(ventana: &SelectorDeLocalidad) {
    ventana.set_actual(texto("Buenos Aires: 13,9 °C, humedad 84 % · Ultima lectura hace 4 min"));
    ventana.set_texto(texto("Cordoba"));
    ventana.set_resumen(texto("3 localidad(es) para «Cordoba»; elija la suya por la region."));
    let localidad = |nombre: &str, region: &str, coordenadas: &str| crate::LocalidadEncontrada {
        nombre: texto(nombre),
        region: texto(region),
        coordenadas: texto(coordenadas),
    };
    ventana.set_localidades(ModelRc::new(VecModel::from(vec![
        localidad("Córdoba", "Capital, Córdoba, Argentina", "-31.41, -64.18"),
        localidad("Córdoba", "Andalucía, España", "37.89, -4.77"),
        localidad("Córdoba", "Veracruz, México", "18.88, -96.93"),
    ])));
    ventana.set_elegida(0);
}

fn llenar_selector_de_avisos(ventana: &SelectorDeAvisos) {
    let aviso = |titulo: &str, artista: &str, tipo: &str, duracion: &str, estado: &str, con_problema: bool, ya: bool, elegido: bool| {
        AvisoDelCatalogo {
            titulo: texto(titulo),
            artista: texto(artista),
            tipo: texto(tipo),
            genero: texto(if tipo == "Seco" { "Institucional" } else { "" }),
            duracion: texto(duracion),
            ruta: texto("D:\\spots\\aviso.mp3"),
            estado: texto(estado),
            con_problema,
            ya_agregado: ya,
            seleccionado: elegido,
        }
    };
    ventana.set_avisos(ModelRc::new(VecModel::from(vec![
        aviso("Ofertas de noviembre", "Almacen Luna", "Comercial", "0:20.0", "", false, false, true),
        aviso("Campana de vacunacion", "Municipalidad", "Anuncio", "0:30.0", "ya esta", false, true, false),
        aviso("Vuelve el 0 km", "Concesionaria Ruta 3", "Comercial", "0:25.0", "", false, false, true),
        aviso("La hora", "Emisora", "Seco", "0:05.0", "", false, false, false),
        aviso("Ofertas de fin de semana", "Supermercado Sol", "Publicidad", "0:30.0", "sin archivo", true, false, false),
    ])));
    ventana.set_destino(texto("la tanda «Tanda A»"));
    ventana.set_resumen(texto(
        "5 tema(s). En el catalogo hay 18 aviso(s) con tipo de tanda y 3 elemento(s) SIN TIPO: reviselos con el filtro «Sin tipo» y asigneles uno (un aviso sin tipo no puede ir en tandas ni ordenes).",
    ));
    ventana.set_faltan_tipos(true);
    ventana.set_seleccionados(2);
}

fn llenar_pauta(ventana: &PautaDeTandas) {
    let fila = |nombre: &str, regla: &str, entrada: &str, avisos: &str, proxima: &str, activa: bool, seleccionada: bool| FilaDeTanda {
        nombre: texto(nombre),
        regla: texto(regla),
        entrada: texto(entrada),
        avisos: texto(avisos),
        proxima: texto(proxima),
        activa,
        problema: false,
        seleccionada,
    };
    let mut sin_archivo = fila("Promo fin de semana", "18:00, sab y dom", "-3 min / +2 min", "2 aviso(s), 0:58.0", "sab 07/11 18:00", true, false);
    sin_archivo.problema = true;
    let filas = vec![
        fila("Tanda A", "10:30 y cada 30 min hasta 20:00, lun a vie", "-3 min / +2 min", "4 aviso(s), 1:45.0", "vie 06/11 10:30 (en 4:12)", true, true),
        fila("Hora en punto", "07:00 y cada 60 min hasta 22:00, todos los dias", "Exacta (-1 min / +0 min)", "1 aviso(s), 0:12.0", "vie 06/11 11:00 (en 34:12)", true, false),
        sin_archivo,
        fila("Navidad", "12:00, todos los dias", "-3 min / +2 min", "3 aviso(s), 1:20.0", "inactiva", false, false),
    ];
    ventana.set_tandas(ModelRc::new(VecModel::from(filas)));
    let aviso = |nombre: &str, tipo: &str, duracion: &str, problema: &str, seleccionado: bool| AvisoDeTanda {
        nombre: texto(nombre),
        tipo: texto(tipo),
        duracion: texto(duracion),
        ruta: texto("D:\\spots\\aviso.mp3"),
        problema: texto(problema),
        seleccionado,
    };
    let avisos = vec![
        aviso("Supermercado El Sol - Anunciante A", "Comercial", "0:30.0", "", false),
        aviso("Concesionaria Ruta 3", "Comercial", "0:25.0", "", true),
        aviso("Campana de vacunacion", "Anuncio", "0:20.0", "", false),
        aviso("promo-nueva.mp3", "sin genero", "0:30.0", "promo-nueva.mp3: no tiene genero; asignele Comercial, Publicidad o Anuncio", false),
    ];
    ventana.set_avisos(ModelRc::new(VecModel::from(avisos)));
    ventana.set_editando(true);
    ventana.set_hay_aviso_elegido(true);
    ventana.set_nombre(texto("Tanda A"));
    ventana.set_inicio(texto("10:30"));
    ventana.set_cada_minutos(30);
    ventana.set_hasta(texto("20:00"));
    for (dia, elegido) in [true, true, true, true, true, false, false].into_iter().enumerate() {
        match dia {
            0 => ventana.set_lun(elegido),
            1 => ventana.set_mar(elegido),
            2 => ventana.set_mie(elegido),
            3 => ventana.set_jue(elegido),
            4 => ventana.set_vie(elegido),
            5 => ventana.set_sab(elegido),
            _ => ventana.set_dom(elegido),
        }
    }
    ventana.set_antes_minutos(3);
    ventana.set_despues_minutos(2);
    ventana.set_prioridad(5);
    ventana.set_activa(true);
    ventana.set_aviso(texto("1 aviso(s) agregado(s); se guardan con el boton de abajo. 1 sin un genero valido para una tanda: elijalos y use Asignar genero."));
    ventana.set_aviso_es_error(true);
}

fn llenar_ordenes(ventana: &OrdenesDePublicidad) {
    let fila = |nombre: &str, anunciante: &str, rubro: &str, regla: &str, hoy: &str, problema: bool, seleccionada: bool| FilaDeOrden {
        nombre: texto(nombre),
        anunciante: texto(anunciante),
        rubro: texto(rubro),
        regla: texto(regla),
        vigencia: texto("2026-11-01 al 2026-11-30"),
        hoy: texto(hoy),
        activa: true,
        problema,
        seleccionada,
    };
    ventana.set_ordenes(ModelRc::new(VecModel::from(vec![
        fila("Sol - Noviembre", "Supermercado Sol", "Supermercados", "cada 2 h de 08:00 a 20:00, lun a vie", "7 de 7", false, true),
        fila("Ruta 3 - Noviembre", "Concesionaria Ruta 3", "Autos", "3 por dia de 08:00 a 20:00, todos los dias", "3 de 3", false, false),
        fila("Luna - Promo", "Almacen Luna", "Supermercados", "a las 10:00, 18:00 de 08:00 a 20:00, lun a vie", "1 de 2", true, false),
    ])));
    let aviso = |nombre: &str, duracion: &str, seleccionado: bool| AvisoDeTanda {
        nombre: texto(nombre),
        tipo: texto("Comercial"),
        duracion: texto(duracion),
        ruta: texto("D:\\spots\\sol.mp3"),
        problema: texto(""),
        seleccionado,
    };
    ventana.set_avisos(ModelRc::new(VecModel::from(vec![aviso("Sol - ofertas de la semana", "0:30.0", true), aviso("Sol - carniceria", "0:20.0", false)])));
    let linea = |texto_de_la_linea: &str, problema: bool| LineaDePauta { texto: texto(texto_de_la_linea), problema };
    ventana.set_pauta_de_hoy(ModelRc::new(VecModel::from(vec![
        linea("08:00 Tanda A: Sol - Noviembre (sol-1.mp3), Ruta 3 - Noviembre (ruta-2.mp3) | 1:35.0 de 3:00.0", false),
        linea("10:00 Tanda A: Sol - Noviembre (sol-2.mp3), Luna - Promo (luna.mp3) (reposicion) | 1:50.0 de 3:00.0", false),
        linea("SIN UBICAR 18:00 Luna - Promo: otro aviso de su rubro ocupa las tandas posibles", true),
    ])));
    ventana.set_resumen_de_hoy(texto("1 salida(s) sin ubicar"));
    ventana.set_editando(true);
    ventana.set_hay_aviso_elegido(true);
    ventana.set_nombre(texto("Sol - Noviembre"));
    ventana.set_anunciante(texto("Supermercado Sol"));
    ventana.set_rubro(texto("Supermercados"));
    ventana.set_estado_del_anunciante(texto("Sol S.A. · CUIT 30-71234567-1"));
    ventana.set_desde(texto("2026-11-01"));
    ventana.set_hasta(texto("2026-11-30"));
    ventana.set_lun(true);
    ventana.set_mar(true);
    ventana.set_mie(true);
    ventana.set_jue(true);
    ventana.set_vie(true);
    ventana.set_franja_desde(texto("08:00"));
    ventana.set_franja_hasta(texto("20:00"));
    ventana.set_tipo_de_frecuencia(0);
    ventana.set_cantidad(120);
    ventana.set_prioridad(5);
    ventana.set_activa(true);
    ventana.set_aviso(texto("Orden actualizada: «Sol - Noviembre» de Supermercado Sol (Supermercados) | hoy: 7 de 7"));
}

fn llenar_mantenimiento(ventana: &MantenimientoDelCatalogo) {
    let ficha = |tema: &str, ruta: &str, hallazgo: &str, encontrado: bool| FichaSinArchivo {
        elegida: true,
        tema: texto(tema),
        ruta: texto(ruta),
        hallazgo: texto(hallazgo),
        encontrado,
    };
    let fichas = vec![
        ficha("We Are The Champions - Queen", "D:\\musica\\viejo\\Queen\\We Are The Champions.mp3", "E:\\musica\\Rock\\Queen\\We Are The Champions.mp3", true),
        ficha("Careless Whisper - George Michael", "D:\\musica\\viejo\\George Michael\\Careless Whisper.mp3", "E:\\musica\\Pop\\George Michael\\Careless Whisper.mp3", true),
        ficha("Off - Dejate Llevar.mp3", "D:\\AHF\\Music\\Dias de Radio\\Radio Uno\\Offs\\Off - Dejate Llevar.mp3", "2 archivos con ese nombre: no se puede elegir uno", false),
        ficha("Tema numero 3 con un titulo bastante largo (Remix extendido) - Orquesta Sinfonica", "D:\\musica\\viejo\\tema3.flac", "No esta en la carpeta", false),
    ];
    ventana.set_fichas(ModelRc::new(VecModel::from(fichas)));
    ventana.set_resumen(texto("4 de 1250 fichas del catalogo tienen un archivo que ya no existe"));
    ventana.set_encontradas(2);
    ventana.set_elegidas(4);
    ventana.set_aviso(texto("En E:\\musica se encontraron 2 de 4; 1 con varios archivos del mismo nombre"));
}
