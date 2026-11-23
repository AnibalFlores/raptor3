//! Carátulas de los temas: la del panel de detalle y la del tema elegido en
//! el catalogo. Se buscan en otro hilo (la del archivo, la de su carpeta y,
//! si se activo, la de internet) y se recuerdan por archivo mientras el
//! programa esta abierto. Desde el catalogo tambien se pueden descargar de
//! una vez las que faltan.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use super::{Controlador, es_anuncio_hth, es_marcador, programar_en_interfaz};
use crate::audio::{PrioridadDeHilo, fijar_prioridad_del_hilo_actual};
use crate::dominio::{Pista, ReglasDelTipo};
use crate::persistencia::{FichaDeCancion, carpeta_del_usuario};
use crate::servicio::{
    CARPETA_DE_CARATULAS, ConsultaDeCaratula, Miniatura, OrigenDeLaCaratula, PAUSA_ENTRE_CONSULTAS, Pixeles, caratula_de_internet,
    caratula_en_cache, caratula_local,
};

/// Antes de salir a internet se espera esto: si el selector se sigue
/// moviendo, no se pregunta por cada tema que pasa.
const ESPERA_ANTES_DE_INTERNET: Duration = Duration::from_millis(600);
/// Carátulas que se recuerdan (las de internet tambien quedan en disco).
/// Cada una ocupa hasta unos 700 KB con la ampliada.
const CARATULAS_RECORDADAS: usize = 100;

/// Una carátula lista para mostrar: la chica, la ampliada del globo y su
/// origen (se describe al mostrarla, en el idioma de ese momento).
#[derive(Clone)]
pub(super) struct Vista {
    pub(super) chica: Image,
    pub(super) ampliada: Image,
    pub(super) origen: OrigenDeLaCaratula,
}

pub(super) type CaratulaVista = Option<Vista>;

/// Donde se muestra una carátula.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DestinoDeCaratula {
    /// El panel de detalle de la ventana principal.
    Panel,
    /// El tema elegido en el buscador del catalogo.
    Catalogo,
}

fn imagen_de(pixeles: &Pixeles) -> Image {
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&pixeles.rgba, pixeles.ancho, pixeles.alto))
}

/// Lo que se busca en internet de un tema: solo lo que es musica (un aviso
/// o un seco no tiene album), por artista y album o titulo.
fn consulta_de(ficha: &FichaDeCancion) -> Option<ConsultaDeCaratula> {
    if ficha.tipo.sin_mezcla() {
        return None;
    }
    ConsultaDeCaratula::nueva(ficha.artista.as_deref(), ficha.album.as_deref(), ficha.titulo.as_deref())
}

/// Como termino la descarga de las carátulas de un lote.
#[derive(Clone, Copy, Debug, Default)]
struct Cuenta {
    propias: usize,
    ya_descargadas: usize,
    descargadas: usize,
    sin_encontrar: usize,
    sin_datos: usize,
    errores: usize,
}

impl Controlador {
    fn pedido_de(&self, destino: DestinoDeCaratula) -> (&std::cell::RefCell<Option<PathBuf>>, &Arc<AtomicU64>) {
        match destino {
            DestinoDeCaratula::Panel => (&self.caratula_del_detalle, &self.generacion_de_caratula),
            DestinoDeCaratula::Catalogo => (&self.caratula_del_catalogo, &self.generacion_de_caratula_del_catalogo),
        }
    }

    /// Muestra la carátula del tema del panel; si no se conoce todavia, la
    /// busca en otro hilo y mientras tanto el panel queda sin carátula.
    pub(super) fn refrescar_caratula_del_detalle(&self, pista: Option<&Pista>) {
        self.pedir_caratula(DestinoDeCaratula::Panel, pista);
    }

    /// Muestra la carátula de un tema en ese destino: la recordada o, si no
    /// se conoce, la busca en otro hilo (mientras tanto, sin carátula).
    pub(super) fn pedir_caratula(&self, destino: DestinoDeCaratula, pista: Option<&Pista>) {
        let pista = pista.filter(|pista| !es_marcador(pista.archivo()) && !es_anuncio_hth(pista.archivo()));
        let archivo = pista.map(Pista::archivo);
        let (actual, generaciones) = self.pedido_de(destino);
        if actual.borrow().as_deref() == archivo {
            return;
        }
        *actual.borrow_mut() = archivo.map(Path::to_path_buf);
        let generacion = generaciones.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(pista) = pista else {
            self.mostrar_caratula(destino, None);
            return;
        };
        if let Some(vista) = self.caratulas_vistas.borrow().get(pista.archivo()) {
            self.mostrar_caratula(destino, vista.clone());
            return;
        }
        self.mostrar_caratula(destino, None);
        let ficha = self.ficha_de(pista);
        let internet = self.repositorio_de_preferencias.caratulas_de_internet();
        let consulta = consulta_de(&ficha).filter(|_| internet);
        // Sin artista todavia (las etiquetas se leen despues de agregar el
        // tema), la falta de carátula no es definitiva.
        let falta_el_artista = internet && consulta.is_none() && !ficha.tipo.sin_mezcla();
        let vigente = Arc::clone(generaciones);
        let archivo = pista.archivo().to_path_buf();
        let cache = carpeta_del_usuario().join(CARPETA_DE_CARATULAS);
        let lanzado = std::thread::Builder::new().name("caratula".into()).spawn(move || {
            fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Fondo);
            if vigente.load(Ordering::SeqCst) != generacion {
                return;
            }
            let mut encontrada = caratula_local(&archivo);
            let mut definitiva = encontrada.is_some() || !falta_el_artista;
            if encontrada.is_none()
                && let Some(consulta) = consulta
            {
                std::thread::sleep(ESPERA_ANTES_DE_INTERNET);
                if vigente.load(Ordering::SeqCst) != generacion {
                    return;
                }
                match caratula_de_internet(&consulta, &cache) {
                    Ok(miniatura) => encontrada = miniatura,
                    Err(error) => {
                        // Sin conexion: se vuelve a intentar la proxima vez.
                        log::info!("No se pudo buscar la carátula de {} en internet: {error}", archivo.display());
                        definitiva = false;
                    }
                }
            }
            programar_en_interfaz(move |controlador| controlador.recibir_caratula(destino, generacion, archivo, encontrada, definitiva));
        });
        if let Err(error) = lanzado {
            log::warn!("No se pudo iniciar la busqueda de la carátula: {error}");
        }
    }

    fn recibir_caratula(&mut self, destino: DestinoDeCaratula, generacion: u64, archivo: PathBuf, encontrada: Option<Miniatura>, definitiva: bool) {
        let vista: CaratulaVista = encontrada.map(|miniatura| Vista {
            chica: imagen_de(&miniatura.chica),
            ampliada: imagen_de(&miniatura.ampliada),
            origen: miniatura.origen,
        });
        if definitiva {
            let mut vistas = self.caratulas_vistas.borrow_mut();
            if vistas.len() >= CARATULAS_RECORDADAS {
                vistas.clear();
            }
            vistas.insert(archivo, vista.clone());
        }
        if generacion == self.pedido_de(destino).1.load(Ordering::SeqCst) {
            self.mostrar_caratula(destino, vista);
        }
    }

    fn mostrar_caratula(&self, destino: DestinoDeCaratula, vista: CaratulaVista) {
        match destino {
            DestinoDeCaratula::Catalogo => self.mostrar_caratula_del_catalogo(vista),
            DestinoDeCaratula::Panel => self.con_ventana(|ventana| match vista {
                Some(vista) => {
                    ventana.set_detalle_caratula(vista.chica);
                    ventana.set_caratula_ampliada(vista.ampliada);
                    ventana.set_origen_de_la_caratula(vista.origen.describir().into());
                    ventana.set_hay_caratula(true);
                }
                None => {
                    ventana.set_hay_caratula(false);
                    ventana.set_detalle_caratula(Image::default());
                    ventana.set_caratula_ampliada(Image::default());
                    ventana.set_origen_de_la_caratula("".into());
                }
            }),
        }
    }

    /// Menu Ventana: buscar o no en internet la carátula de los temas que no
    /// la traen. Lo recordado se olvida para buscarla de nuevo.
    pub(super) fn alternar_caratulas_de_internet(&mut self) {
        let activo = !self.repositorio_de_preferencias.caratulas_de_internet();
        self.repositorio_de_preferencias.guardar_caratulas_de_internet(activo);
        self.con_ventana(|ventana| ventana.set_caratulas_de_internet(activo));
        self.olvidar_las_caratulas();
        self.refrescar_detalles();
        self.mostrar_estado(&if activo {
            tr!("Se buscan en internet las carátulas que faltan (Deezer, iTunes y MusicBrainz)")
        } else {
            tr!("Ya no se buscan carátulas en internet")
        });
    }

    /// Olvida las carátulas recordadas: se vuelven a buscar al mostrarlas.
    pub(super) fn olvidar_las_caratulas(&self) {
        self.caratulas_vistas.borrow_mut().clear();
        *self.caratula_del_detalle.borrow_mut() = None;
        *self.caratula_del_catalogo.borrow_mut() = None;
    }

    /// Los datos del tema cambiaron (artista, album): su carátula se vuelve a buscar.
    pub(super) fn olvidar_caratula(&self, archivo: &Path) {
        self.caratulas_vistas.borrow_mut().remove(archivo);
        for actual in [&self.caratula_del_detalle, &self.caratula_del_catalogo] {
            if actual.borrow().as_deref() == Some(archivo) {
                *actual.borrow_mut() = None;
            }
        }
    }

    /// Desde el catalogo: busca la carátula de cada tema y descarga de
    /// internet las que faltan (una consulta por segundo), para tenerlas
    /// guardadas. Los que tienen la suya (en el archivo o en la carpeta) no
    /// se consultan. Avisa el avance con `avisar`, en la interfaz.
    pub(super) fn descargar_caratulas(&mut self, archivos: Vec<PathBuf>, avisar: fn(&mut Controlador, String, bool)) {
        let temas: Vec<(PathBuf, Option<ConsultaDeCaratula>)> = archivos
            .into_iter()
            .filter(|archivo| !es_marcador(archivo) && !es_anuncio_hth(archivo))
            .map(|archivo| {
                let ficha = self.repositorio_de_canciones.buscar(&archivo).cloned().unwrap_or_default();
                let consulta = consulta_de(&ficha);
                (archivo, consulta)
            })
            .collect();
        if temas.is_empty() {
            avisar(self, tr!("No hay temas para buscar carátulas."), true);
            return;
        }
        let generacion = self.descarga_de_caratulas.fetch_add(1, Ordering::SeqCst) + 1;
        let vigente = Arc::clone(&self.descarga_de_caratulas);
        let cache = carpeta_del_usuario().join(CARPETA_DE_CARATULAS);
        let total = temas.len();
        let lanzado = std::thread::Builder::new().name("descarga-de-caratulas".into()).spawn(move || {
            fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Fondo);
            let mut cuenta = Cuenta::default();
            let mut consulto = false;
            for (numero, (archivo, consulta)) in temas.into_iter().enumerate() {
                if vigente.load(Ordering::SeqCst) != generacion {
                    return;
                }
                if !archivo.is_file() || caratula_local(&archivo).is_some() {
                    cuenta.propias += usize::from(archivo.is_file());
                    cuenta.sin_datos += usize::from(!archivo.is_file());
                } else if let Some(consulta) = consulta {
                    if let Some(descargada) = caratula_en_cache(&consulta, &cache) {
                        if descargada {
                            cuenta.ya_descargadas += 1;
                        } else {
                            cuenta.sin_encontrar += 1;
                        }
                    } else {
                        // MusicBrainz admite una consulta por segundo.
                        if consulto {
                            std::thread::sleep(PAUSA_ENTRE_CONSULTAS);
                        }
                        consulto = true;
                        match caratula_de_internet(&consulta, &cache) {
                            Ok(Some(_)) => cuenta.descargadas += 1,
                            Ok(None) => cuenta.sin_encontrar += 1,
                            Err(error) => {
                                log::info!("No se pudo descargar la carátula de {}: {error}", archivo.display());
                                cuenta.errores += 1;
                            }
                        }
                    }
                } else {
                    cuenta.sin_datos += 1;
                }
                let hechos = numero + 1;
                if hechos < total {
                    let avance = tr!("Carátulas: {hechos} de {total} tema(s) revisados...", hechos = hechos, total = total);
                    programar_en_interfaz(move |controlador| avisar(controlador, avance, false));
                }
            }
            let resumen = describir(cuenta, total);
            log::info!("{resumen}");
            programar_en_interfaz(move |controlador| {
                // Las que se descargaron se ven al volver a mostrarlas.
                controlador.olvidar_las_caratulas();
                controlador.refrescar_detalles();
                avisar(controlador, resumen, true);
            });
        });
        if let Err(error) = lanzado {
            avisar(self, tr!("No se pudo iniciar la descarga de carátulas: {error}", error = error), true);
        }
    }

    /// Detiene la descarga en curso (al cerrar el catalogo).
    pub(super) fn detener_descarga_de_caratulas(&self) {
        self.descarga_de_caratulas.fetch_add(1, Ordering::SeqCst);
    }
}

/// "Carátulas de 40 tema(s): 25 propias, 10 descargadas, 3 ya descargadas, 2 sin encontrar".
fn describir(cuenta: Cuenta, total: usize) -> String {
    let mut partes = vec![tr!("{} propias (en el archivo o la carpeta)", cuenta.propias)];
    if cuenta.descargadas > 0 {
        partes.push(tr!("{} descargadas de internet", cuenta.descargadas));
    }
    if cuenta.ya_descargadas > 0 {
        partes.push(tr!("{} ya descargadas", cuenta.ya_descargadas));
    }
    if cuenta.sin_encontrar > 0 {
        partes.push(tr!("{} sin una carátula confiable en internet", cuenta.sin_encontrar));
    }
    if cuenta.sin_datos > 0 {
        partes.push(tr!("{} sin artista, sin archivo o que no son musica", cuenta.sin_datos));
    }
    if cuenta.errores > 0 {
        partes.push(tr!("{} sin respuesta (sin conexion)", cuenta.errores));
    }
    tr!("Carátulas de {total} tema(s): {}", partes.join(", "), total = total)
}
