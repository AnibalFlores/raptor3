//! Actualizar los datos del catalogo desde una fuente externa: releer las
//! etiquetas de los archivos (despues de corregirlas con Picard u otro
//! programa) o completar desde MusicBrainz. La lectura y las consultas van en
//! un hilo; cada resultado se compara con la ficha y los cambios propuestos
//! se muestran para elegirlos antes de aplicarlos al catalogo.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, VecModel};

use super::buscador::titulo_de_la_ficha;
use super::{Controlador, con_controlador};
use crate::audio::Decodificador;
use crate::dominio::{DatosDescriptivos, OrigenDeLosDatos, Propuesta, ReglasDelTipo, proponer};
use crate::servicio::{PAUSA_ENTRE_CONSULTAS, buscar_grabacion};
use crate::{ActualizacionDeDatos, CambioPropuesto};

/// Errores seguidos de MusicBrainz tras los que se deja de consultar (sin
/// conexion, servicio caido).
const ERRORES_PARA_DETENER: usize = 3;
/// Cada cuantos archivos leidos se muestran los resultados al releer.
const LECTURAS_POR_ENTREGA: usize = 25;

/// La ventana de revision y el trabajo en curso.
pub(super) struct ActualizacionDelCatalogo {
    ventana: ActualizacionDeDatos,
    modelo: Rc<VecModel<CambioPropuesto>>,
    propuestas: Vec<Propuesta>,
    origen: OrigenDeLosDatos,
    // Sube con cada trabajo: los resultados de uno anterior se descartan.
    generacion: u64,
    cancelar: Arc<AtomicBool>,
    total: usize,
    hechos: usize,
    sin_coincidencia: usize,
    errores: Vec<String>,
}

impl ActualizacionDelCatalogo {
    pub(super) fn nueva() -> Result<Self, String> {
        let ventana = ActualizacionDeDatos::new()
            .map_err(|error| tr!("No se pudo crear la ventana de actualizacion de datos: {error}", error = error))?;
        let modelo = Rc::new(VecModel::default());
        ventana.set_cambios(ModelRc::from(modelo.clone()));
        ventana.on_elegir(|indice, elegido| con_controlador(|c| c.elegir_cambio(indice, elegido)));
        ventana.on_aplicar(|| con_controlador(Controlador::aplicar_cambios_elegidos));
        ventana.on_detener(|| con_controlador(Controlador::detener_actualizacion));
        ventana.on_cerrar(|| con_controlador(Controlador::cerrar_actualizacion));
        ventana.window().on_close_requested(|| {
            con_controlador(Controlador::detener_actualizacion);
            CloseRequestResponse::HideWindow
        });
        Ok(Self {
            ventana,
            modelo,
            propuestas: Vec::new(),
            origen: OrigenDeLosDatos::Etiquetas,
            generacion: 0,
            cancelar: Arc::new(AtomicBool::new(false)),
            total: 0,
            hechos: 0,
            sin_coincidencia: 0,
            errores: Vec::new(),
        })
    }

    pub(super) fn ventana(&self) -> &slint::Window {
        self.ventana.window()
    }
}

/// Lo que se manda del hilo de trabajo a la interfaz.
struct Entrega {
    generacion: u64,
    lecturas: Vec<(PathBuf, DatosDescriptivos)>,
    hechos: usize,
    sin_coincidencia: usize,
    errores: Vec<String>,
    terminado: bool,
}

fn entregar(entrega: Entrega) {
    let _ = slint::invoke_from_event_loop(move || con_controlador(|c| c.recibir_entrega(entrega)));
}

/// Fila de la tabla para un cambio propuesto.
fn fila(propuesta: &Propuesta, tema: String) -> CambioPropuesto {
    let nota = if propuesta.corregido {
        tr!("corregido a mano")
    } else if propuesta.sin_registro && propuesta.actual.is_some() {
        tr!("ficha anterior: no registra correcciones")
    } else {
        String::new()
    };
    CambioPropuesto {
        elegido: propuesta.elegida,
        tema: tema.into(),
        ruta: propuesta.archivo.to_string_lossy().into_owned().into(),
        campo: propuesta.campo.clave().into(),
        actual: propuesta.actual.clone().unwrap_or_default().into(),
        nuevo: propuesta.nuevo.clone().into(),
        nota: nota.into(),
    }
}

impl Controlador {
    /// Relee las etiquetas de los archivos y propone actualizar las fichas.
    pub(super) fn releer_etiquetas(&mut self, archivos: Vec<PathBuf>) {
        let archivos: Vec<PathBuf> = archivos.into_iter().filter(|archivo| archivo.is_file()).collect();
        if archivos.is_empty() {
            self.mostrar_estado(&tr!("No hay archivos para releer (los marcadores, los anuncios HTH y los archivos que faltan no tienen etiquetas)."));
            return;
        }
        let (generacion, cancelar) = self.comenzar_actualizacion(
            OrigenDeLosDatos::Etiquetas,
            archivos.len(),
            tr!("Etiquetas de los archivos: lo que escribio en ellos otro programa (Picard, Mp3tag...). Los datos que usted corrigio a mano no se marcan solos."),
        );
        let lanzado = std::thread::Builder::new().name("releer-etiquetas".into()).spawn(move || {
            let mut lecturas = Vec::new();
            let mut errores = Vec::new();
            for (numero, archivo) in archivos.iter().enumerate() {
                if cancelar.load(Ordering::Relaxed) {
                    break;
                }
                match Decodificador::solo_etiquetas(archivo) {
                    Ok(analisis) => lecturas.push((
                        archivo.clone(),
                        DatosDescriptivos {
                            titulo: analisis.titulo,
                            artista: analisis.artista,
                            album: analisis.album,
                            anio: analisis.anio,
                            genero: analisis.genero,
                        },
                    )),
                    Err(error) => errores.push(error),
                }
                if lecturas.len() >= LECTURAS_POR_ENTREGA {
                    entregar(Entrega {
                        generacion,
                        lecturas: std::mem::take(&mut lecturas),
                        hechos: numero + 1,
                        sin_coincidencia: 0,
                        errores: std::mem::take(&mut errores),
                        terminado: false,
                    });
                }
            }
            entregar(Entrega { generacion, lecturas, hechos: archivos.len(), sin_coincidencia: 0, errores, terminado: true });
        });
        if let Err(error) = lanzado {
            self.terminar_actualizacion_con_error(&tr!("No se pudo iniciar la lectura: {error}", error = error));
        }
    }

    /// Busca cada tema en MusicBrainz (uno por segundo) y propone completar
    /// sus datos. Solo los temas (no los avisos ni los efectos).
    pub(super) fn completar_desde_musicbrainz(&mut self, archivos: Vec<PathBuf>) {
        let temas: Vec<(PathBuf, String, Option<String>, Option<f64>)> = archivos
            .into_iter()
            .filter(|archivo| !self.repositorio_de_canciones.tipo_de(archivo).sin_mezcla())
            .filter_map(|archivo| {
                let ficha = self.repositorio_de_canciones.buscar(&archivo);
                let titulo = ficha
                    .and_then(|ficha| ficha.titulo.clone())
                    .or_else(|| archivo.file_stem().map(|nombre| nombre.to_string_lossy().into_owned()))?;
                let artista = ficha.and_then(|ficha| ficha.artista.clone());
                let duracion = ficha.and_then(|ficha| ficha.duracion);
                Some((archivo, titulo, artista, duracion))
            })
            .collect();
        if temas.is_empty() {
            self.mostrar_estado(&tr!("No hay temas para buscar en MusicBrainz (los avisos, efectos, marcadores y anuncios HTH no se buscan)."));
            return;
        }
        let (generacion, cancelar) = self.comenzar_actualizacion(
            OrigenDeLosDatos::MusicBrainz,
            temas.len(),
            tr!("MusicBrainz (musicbrainz.org), la base libre de datos musicales que usa Picard: se busca cada tema por titulo y artista, de a uno por segundo. Solo se marcan los datos que faltan; los demas, revíselos."),
        );
        let lanzado = std::thread::Builder::new().name("musicbrainz".into()).spawn(move || {
            let mut errores_seguidos = 0;
            for (numero, (archivo, titulo, artista, duracion)) in temas.iter().enumerate() {
                if cancelar.load(Ordering::Relaxed) {
                    break;
                }
                if numero > 0 {
                    std::thread::sleep(PAUSA_ENTRE_CONSULTAS);
                }
                let mut entrega = Entrega {
                    generacion,
                    lecturas: Vec::new(),
                    hechos: numero + 1,
                    sin_coincidencia: 0,
                    errores: Vec::new(),
                    terminado: false,
                };
                match buscar_grabacion(titulo, artista.as_deref(), *duracion) {
                    Ok(Some(coincidencia)) => {
                        errores_seguidos = 0;
                        entrega.lecturas.push((archivo.clone(), coincidencia.datos));
                    }
                    Ok(None) => {
                        errores_seguidos = 0;
                        entrega.sin_coincidencia = 1;
                    }
                    Err(error) => {
                        errores_seguidos += 1;
                        entrega.errores.push(error);
                        if errores_seguidos >= ERRORES_PARA_DETENER {
                            entrega.terminado = true;
                            entregar(entrega);
                            return;
                        }
                    }
                }
                entregar(entrega);
            }
            entregar(Entrega {
                generacion,
                lecturas: Vec::new(),
                hechos: temas.len(),
                sin_coincidencia: 0,
                errores: Vec::new(),
                terminado: true,
            });
        });
        if let Err(error) = lanzado {
            self.terminar_actualizacion_con_error(&tr!("No se pudo iniciar la busqueda: {error}", error = error));
        }
    }

    /// Limpia la ventana, la muestra y prepara un trabajo nuevo.
    fn comenzar_actualizacion(&mut self, origen: OrigenDeLosDatos, total: usize, explicacion: String) -> (u64, Arc<AtomicBool>) {
        let actualizacion = &mut self.actualizacion;
        actualizacion.cancelar.store(true, Ordering::Relaxed);
        actualizacion.cancelar = Arc::new(AtomicBool::new(false));
        actualizacion.generacion += 1;
        actualizacion.origen = origen;
        actualizacion.propuestas.clear();
        actualizacion.modelo.set_vec(Vec::new());
        actualizacion.total = total;
        actualizacion.hechos = 0;
        actualizacion.sin_coincidencia = 0;
        actualizacion.errores.clear();
        let ventana = actualizacion.ventana.clone_strong();
        ventana.set_origen(explicacion.into());
        ventana.set_trabajando(true);
        ventana.set_progreso(0.0);
        ventana.set_elegidos(0);
        ventana.set_aviso("".into());
        let resultado = (actualizacion.generacion, actualizacion.cancelar.clone());
        self.refrescar_resumen_de_actualizacion();
        if let Err(error) = self.mostrar_sobre_la_principal(ventana.window(), || ventana.show()) {
            self.mostrar_estado(&error);
        }
        resultado
    }

    fn terminar_actualizacion_con_error(&mut self, error: &str) {
        let ventana = &self.actualizacion.ventana;
        ventana.set_trabajando(false);
        ventana.set_aviso(error.into());
    }

    fn recibir_entrega(&mut self, entrega: Entrega) {
        if entrega.generacion != self.actualizacion.generacion {
            return;
        }
        let origen = self.actualizacion.origen;
        let mut filas = Vec::new();
        for (archivo, leidos) in entrega.lecturas {
            let ficha = self.repositorio_de_canciones.buscar(&archivo).cloned().unwrap_or_default();
            let tema = titulo_de_la_ficha(&archivo, &ficha);
            for propuesta in proponer(archivo.clone(), &ficha.datos_descriptivos(), ficha.corregidos, &leidos, origen) {
                filas.push(fila(&propuesta, tema.clone()));
                self.actualizacion.propuestas.push(propuesta);
            }
        }
        let actualizacion = &mut self.actualizacion;
        for nueva in filas {
            actualizacion.modelo.push(nueva);
        }
        actualizacion.hechos = actualizacion.hechos.max(entrega.hechos);
        actualizacion.sin_coincidencia += entrega.sin_coincidencia;
        actualizacion.errores.extend(entrega.errores);
        if entrega.terminado {
            actualizacion.ventana.set_trabajando(false);
            if let Some(error) = actualizacion.errores.last() {
                let aviso = tr!("{} error(es); el ultimo: {error}", actualizacion.errores.len(), error = error);
                actualizacion.ventana.set_aviso(aviso.into());
            }
        }
        self.refrescar_resumen_de_actualizacion();
    }

    fn refrescar_resumen_de_actualizacion(&self) {
        let actualizacion = &self.actualizacion;
        let elegidos = actualizacion.propuestas.iter().filter(|propuesta| propuesta.elegida).count();
        let temas: std::collections::BTreeSet<&Path> = actualizacion.propuestas.iter().map(|propuesta| propuesta.archivo.as_path()).collect();
        let mut resumen = tr!(
            "{} de {} tema(s) revisados: {} dato(s) distintos en {} tema(s)",
            actualizacion.hechos,
            actualizacion.total,
            actualizacion.propuestas.len(),
            temas.len()
        );
        if actualizacion.sin_coincidencia > 0 {
            resumen.push_str(&tr!("; {} sin una coincidencia segura en MusicBrainz", actualizacion.sin_coincidencia));
        }
        let ventana = &actualizacion.ventana;
        ventana.set_resumen(resumen.into());
        ventana.set_elegidos(elegidos as i32);
        ventana.set_progreso(if actualizacion.total == 0 { 1.0 } else { actualizacion.hechos as f32 / actualizacion.total as f32 });
    }

    fn elegir_cambio(&mut self, indice: i32, elegido: bool) {
        let actualizacion = &mut self.actualizacion;
        let indices: Vec<usize> = match usize::try_from(indice) {
            Ok(indice) if indice < actualizacion.propuestas.len() => vec![indice],
            Ok(_) => return,
            Err(_) => (0..actualizacion.propuestas.len()).collect(),
        };
        for indice in indices {
            actualizacion.propuestas[indice].elegida = elegido;
            if let Some(mut fila) = actualizacion.modelo.row_data(indice) {
                fila.elegido = elegido;
                actualizacion.modelo.set_row_data(indice, fila);
            }
        }
        self.refrescar_resumen_de_actualizacion();
    }

    /// Aplica al catalogo los cambios marcados y los saca de la tabla.
    fn aplicar_cambios_elegidos(&mut self) {
        let origen = self.actualizacion.origen;
        let (elegidas, restantes): (Vec<Propuesta>, Vec<Propuesta>) =
            std::mem::take(&mut self.actualizacion.propuestas).into_iter().partition(|propuesta| propuesta.elegida);
        let cambiados = self.repositorio_de_canciones.aplicar_propuestas(&elegidas, origen);
        let temas: std::collections::BTreeSet<PathBuf> = elegidas.iter().map(|propuesta| propuesta.archivo.clone()).collect();
        let filas: Vec<CambioPropuesto> = {
            let modelo = &self.actualizacion.modelo;
            (0..modelo.row_count()).filter_map(|indice| modelo.row_data(indice)).filter(|fila| !fila.elegido).collect()
        };
        self.actualizacion.propuestas = restantes;
        self.actualizacion.modelo.set_vec(filas);
        let aviso = tr!("Se actualizaron {cambiados} dato(s) en {} tema(s) del catalogo.", temas.len(), cambiados = cambiados);
        self.actualizacion.ventana.set_aviso(aviso.clone().into());
        self.refrescar_resumen_de_actualizacion();
        self.refrescar_filas();
        self.refrescar_buscador_si_esta_abierto();
        for archivo in &temas {
            self.refrescar_datos_del_editor(archivo);
        }
        log::info!("Actualizacion de datos ({origen:?}): {cambiados} dato(s) en {} tema(s)", temas.len());
        self.mostrar_estado(&aviso);
    }

    fn detener_actualizacion(&mut self) {
        let actualizacion = &mut self.actualizacion;
        actualizacion.cancelar.store(true, Ordering::Relaxed);
        if actualizacion.ventana.get_trabajando() {
            actualizacion.ventana.set_trabajando(false);
            actualizacion.ventana.set_aviso(tr!("Detenido: puede aplicar lo encontrado hasta ahora.").into());
        }
    }

    fn cerrar_actualizacion(&mut self) {
        self.detener_actualizacion();
        let _ = self.actualizacion.ventana.hide();
    }

    /// Archivos de la lista para actualizar desde el menu de una fila: la
    /// seleccion si la fila esta en ella; si no, esa fila. Sin marcadores ni
    /// anuncios HTH.
    pub(super) fn archivos_para_actualizar(&self, indice: usize) -> Vec<PathBuf> {
        let posiciones: Vec<usize> =
            if self.seleccion.contains(&indice) { self.seleccion.iter().copied().collect() } else { vec![indice] };
        let mut archivos: Vec<PathBuf> = Vec::new();
        for posicion in posiciones {
            let Some(pista) = self.lista.pista_en(posicion) else { continue };
            if super::es_marcador(pista.archivo()) || super::es_anuncio_hth(pista.archivo()) {
                continue;
            }
            if !archivos.iter().any(|archivo| archivo == pista.archivo()) {
                archivos.push(pista.archivo().to_path_buf());
            }
        }
        archivos
    }
}

/// Origen elegido en un menu: 0 etiquetas, 1 MusicBrainz.
pub(super) fn actualizar_desde(c: &mut Controlador, archivos: Vec<PathBuf>, origen: i32) {
    if origen == 1 {
        c.completar_desde_musicbrainz(archivos);
    } else {
        c.releer_etiquetas(archivos);
    }
}
