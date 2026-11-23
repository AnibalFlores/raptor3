//! Ventana de mantenimiento del catalogo: muestra las fichas cuyo archivo
//! ya no existe, los busca en una carpeta, los reubica (pasando la ficha a
//! la nueva ruta y actualizando la lista, la botonera y los efectos fijados)
//! o los quita del catalogo, y limpia la cache de formas de onda.
//!
//! La busqueda recorre carpetas que pueden ser grandes: corre en otro hilo y
//! entrega el resultado al controlador. Una busqueda vieja (si se recargo la
//! lista de fichas mientras tanto) se descarta.

use std::collections::HashMap;
use std::path::PathBuf;

use slint::{CloseRequestResponse, ComponentHandle, Model};

use super::{Controlador, con_controlador, programar_en_interfaz};
use crate::audio::{Decodificador, PrioridadDeHilo, fijar_prioridad_del_hilo_actual, limpiar_cache_de_ondas};
use crate::dominio::{Pista, nombre_de_archivo};
use crate::persistencia::{FichaDeCancion, carpeta_del_usuario};
use crate::servicio::{ArchivoPerdido, Hallazgo, buscar_movidos};
use crate::{FichaSinArchivo, MantenimientoDelCatalogo};

/// Ficha del catalogo cuyo archivo ya no existe, con lo que se encontro.
pub(super) struct FichaPerdida {
    ruta: PathBuf,
    duracion: Option<f64>,
    tema: String,
    elegida: bool,
    hallazgo: Option<Hallazgo>,
}

impl FichaPerdida {
    /// Ruta nueva, si se encontro una sola y sigue existiendo.
    fn nueva_ruta(&self) -> Option<&PathBuf> {
        match &self.hallazgo {
            Some(Hallazgo::Encontrado(ruta)) if ruta.is_file() => Some(ruta),
            _ => None,
        }
    }

    fn fila(&self) -> FichaSinArchivo {
        let hallazgo = match &self.hallazgo {
            None => tr!("Sin buscar"),
            Some(Hallazgo::Encontrado(ruta)) => ruta.display().to_string(),
            Some(Hallazgo::Varios(cantidad)) => tr!("{cantidad} archivos con ese nombre: no se puede elegir uno", cantidad = cantidad),
            Some(Hallazgo::NoEncontrado) => tr!("No esta en la carpeta"),
        };
        FichaSinArchivo {
            elegida: self.elegida,
            tema: self.tema.clone().into(),
            ruta: self.ruta.display().to_string().into(),
            hallazgo: hallazgo.into(),
            encontrado: self.nueva_ruta().is_some(),
        }
    }
}

/// "Titulo - Artista" de la ficha o, si no los tiene, el nombre del archivo.
fn tema_de(ruta: &std::path::Path, ficha: &FichaDeCancion) -> String {
    match (&ficha.titulo, &ficha.artista) {
        (Some(titulo), Some(artista)) => format!("{titulo} - {artista}"),
        (Some(titulo), None) => titulo.clone(),
        _ => nombre_de_archivo(ruta),
    }
}

pub(super) fn cablear(ventana: &MantenimientoDelCatalogo) {
    let debil = ventana.as_weak();
    ventana.on_buscar_en_carpeta(move || {
        // Los dialogos se abren sin tomar el controlador, para no trabar el ciclo.
        let mut dialogo = rfd::FileDialog::new().set_title(&tr!("Carpeta donde buscar los archivos movidos"));
        if let Some(ventana) = debil.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        if let Some(carpeta) = dialogo.pick_folder() {
            con_controlador(|c| c.buscar_movidos_en(carpeta));
        }
    });
    ventana.on_reubicar_encontradas(|| con_controlador(|c| c.reubicar_encontradas()));
    let debil = ventana.as_weak();
    ventana.on_quitar_elegidas(move || {
        let mut cantidad = 0;
        con_controlador(|c| cantidad = c.fichas_perdidas.iter().filter(|ficha| ficha.elegida).count());
        if cantidad == 0 {
            return;
        }
        let mut dialogo = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&tr!("Quitar del catalogo"))
            .set_description(format!(
                "Se borraran {cantidad} ficha(s) del catalogo: puntos de mezcla, overlay, titulo, artista, genero, \
                 ranking, BPM y ultima reproduccion. Los archivos de audio no se tocan.\n\n¿Quitarlas?"
            ))
            .set_buttons(rfd::MessageButtons::YesNo);
        if let Some(ventana) = debil.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        if dialogo.show() == rfd::MessageDialogResult::Yes {
            con_controlador(|c| c.quitar_fichas_elegidas());
        }
    });
    ventana.on_limpiar_ondas(|| con_controlador(|c| c.limpiar_ondas()));
    ventana.on_elegir(|posicion, elegida| con_controlador(|c| c.elegir_ficha_perdida(posicion, elegida)));
    ventana.on_actualizar(|| con_controlador(|c| c.cargar_fichas_perdidas()));
    ventana.on_cerrar(|| con_controlador(|c| c.cerrar_mantenimiento()));
    ventana.window().on_close_requested(|| {
        con_controlador(|c| c.cerrar_mantenimiento());
        CloseRequestResponse::HideWindow
    });
}

impl Controlador {
    /// Abre la ventana con las fichas cuyo archivo ya no existe.
    pub fn abrir_mantenimiento(&mut self) -> Result<(), String> {
        self.fichas_perdidas.clear();
        self.cargar_fichas_perdidas();
        self.mantenimiento.set_aviso("".into());
        let ventana = self.mantenimiento.clone_strong();
        self.mostrar_sobre_la_principal(ventana.window(), || ventana.show())
    }

    pub(super) fn cerrar_mantenimiento(&mut self) {
        // Una busqueda en curso ya no tiene donde mostrarse.
        self.generacion_de_busqueda += 1;
        self.mantenimiento.set_buscando(false);
        self.fichas_perdidas.clear();
        let _ = self.mantenimiento.hide();
    }

    /// Vuelve a mirar que fichas no tienen archivo. Lo que ya se habia
    /// buscado y elegido de las que siguen se conserva.
    fn cargar_fichas_perdidas(&mut self) {
        let anteriores: HashMap<PathBuf, (bool, Option<Hallazgo>)> = self
            .fichas_perdidas
            .drain(..)
            .map(|ficha| (ficha.ruta, (ficha.elegida, ficha.hallazgo)))
            .collect();
        self.fichas_perdidas = self
            .repositorio_de_canciones
            .sin_archivo()
            .into_iter()
            .map(|(ruta, ficha)| {
                let (elegida, hallazgo) = anteriores.get(&ruta).cloned().unwrap_or((true, None));
                FichaPerdida { tema: tema_de(&ruta, &ficha), duracion: ficha.duracion, ruta, elegida, hallazgo }
            })
            .collect();
        self.generacion_de_busqueda += 1;
        self.mantenimiento.set_buscando(false);
        self.refrescar_mantenimiento();
    }

    fn refrescar_mantenimiento(&self) {
        let filas: Vec<FichaSinArchivo> = self.fichas_perdidas.iter().map(FichaPerdida::fila).collect();
        self.modelo_de_mantenimiento.set_vec(filas);
        self.refrescar_cuentas_del_mantenimiento();
        let total = self.repositorio_de_canciones.cantidad();
        let resumen = match self.fichas_perdidas.len() {
            0 => tr!("Las {total} fichas del catalogo tienen su archivo.", total = total),
            perdidas => tr!("{perdidas} de {total} fichas del catalogo tienen un archivo que ya no existe", perdidas = perdidas, total = total),
        };
        self.mantenimiento.set_resumen(resumen.into());
    }

    fn refrescar_cuentas_del_mantenimiento(&self) {
        let elegidas = self.fichas_perdidas.iter().filter(|ficha| ficha.elegida);
        let encontradas = elegidas.clone().filter(|ficha| ficha.nueva_ruta().is_some()).count();
        self.mantenimiento.set_elegidas(elegidas.count() as i32);
        self.mantenimiento.set_encontradas(encontradas as i32);
    }

    /// Marca o desmarca una ficha (o todas, con -1).
    fn elegir_ficha_perdida(&mut self, posicion: i32, elegida: bool) {
        match usize::try_from(posicion) {
            Ok(posicion) => {
                if let Some(ficha) = self.fichas_perdidas.get_mut(posicion) {
                    ficha.elegida = elegida;
                    self.modelo_de_mantenimiento.set_row_data(posicion, ficha.fila());
                }
            }
            Err(_) => {
                for (posicion, ficha) in self.fichas_perdidas.iter_mut().enumerate() {
                    ficha.elegida = elegida;
                    self.modelo_de_mantenimiento.set_row_data(posicion, ficha.fila());
                }
            }
        }
        self.refrescar_cuentas_del_mantenimiento();
    }

    /// Busca en otro hilo los archivos de las fichas perdidas dentro de la
    /// carpeta (y sus subcarpetas).
    fn buscar_movidos_en(&mut self, carpeta: PathBuf) {
        let perdidos: Vec<ArchivoPerdido> = self
            .fichas_perdidas
            .iter()
            .map(|ficha| ArchivoPerdido { ruta: ficha.ruta.clone(), duracion: ficha.duracion })
            .collect();
        if perdidos.is_empty() {
            return;
        }
        self.generacion_de_busqueda += 1;
        let generacion = self.generacion_de_busqueda;
        self.mantenimiento.set_buscando(true);
        self.mantenimiento.set_aviso(tr!("Buscando en {}...", carpeta.display()).into());
        let lanzado = std::thread::Builder::new().name("busqueda-de-movidos".into()).spawn(move || {
            fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Fondo);
            let hallazgos = buscar_movidos(&perdidos, &carpeta, |ruta| Decodificador::sondear_duracion(ruta).ok().flatten());
            programar_en_interfaz(move |controlador| controlador.busqueda_terminada(generacion, &carpeta, hallazgos));
        });
        if let Err(error) = lanzado {
            self.mantenimiento.set_buscando(false);
            self.mantenimiento.set_aviso(tr!("No se pudo iniciar la busqueda: {error}", error = error).into());
        }
    }

    fn busqueda_terminada(&mut self, generacion: u64, carpeta: &std::path::Path, hallazgos: Vec<Hallazgo>) {
        if generacion != self.generacion_de_busqueda || hallazgos.len() != self.fichas_perdidas.len() {
            return;
        }
        for (ficha, hallazgo) in self.fichas_perdidas.iter_mut().zip(hallazgos) {
            ficha.hallazgo = Some(hallazgo);
        }
        self.mantenimiento.set_buscando(false);
        let encontradas = self.fichas_perdidas.iter().filter(|ficha| ficha.nueva_ruta().is_some()).count();
        let varios = self.fichas_perdidas.iter().filter(|ficha| matches!(ficha.hallazgo, Some(Hallazgo::Varios(_)))).count();
        let mut aviso = tr!("En {} se encontraron {encontradas} de {}", carpeta.display(), self.fichas_perdidas.len(), encontradas = encontradas);
        if varios > 0 {
            aviso.push_str(&tr!("; {varios} con varios archivos del mismo nombre", varios = varios));
        }
        self.mantenimiento.set_aviso(aviso.into());
        self.refrescar_mantenimiento();
    }

    /// Pasa las fichas elegidas que se encontraron a su nueva ruta y
    /// actualiza la lista, la botonera y los efectos fijados.
    fn reubicar_encontradas(&mut self) {
        let cambios: Vec<(PathBuf, PathBuf)> = self
            .fichas_perdidas
            .iter()
            .filter(|ficha| ficha.elegida)
            .filter_map(|ficha| Some((ficha.ruta.clone(), ficha.nueva_ruta()?.clone())))
            .collect();
        if cambios.is_empty() {
            return;
        }
        let fichas = self.repositorio_de_canciones.reubicar(&cambios);
        let botones = self.repositorio_de_botonera.reubicar(&cambios);
        let en_la_lista: usize =
            cambios.iter().map(|(anterior, nueva)| self.lista.reubicar(&Pista::new(anterior), &Pista::new(nueva))).sum();
        if en_la_lista > 0 {
            self.guardar_lista();
            self.pedir_datos_de_mezcla();
        }
        if botones > 0 {
            self.duraciones_de_efectos.borrow_mut().clear();
        }
        self.refrescar_interfaz();
        self.refrescar_botones();
        self.cargar_fichas_perdidas();
        let aviso = tr!("Se reubicaron {fichas} ficha(s): {en_la_lista} pista(s) de la lista y {botones} boton(es) de efectos.", fichas = fichas, en_la_lista = en_la_lista, botones = botones);
        self.mantenimiento.set_aviso(aviso.clone().into());
        self.mostrar_estado(&aviso);
    }

    fn quitar_fichas_elegidas(&mut self) {
        let rutas: Vec<PathBuf> = self.fichas_perdidas.iter().filter(|ficha| ficha.elegida).map(|ficha| ficha.ruta.clone()).collect();
        let quitadas = self.repositorio_de_canciones.olvidar_varios(&rutas);
        self.cargar_fichas_perdidas();
        self.refrescar_filas();
        self.mantenimiento.set_aviso(tr!("Se quitaron {quitadas} ficha(s) del catalogo.", quitadas = quitadas).into());
    }

    /// Borra las formas de onda que no son de ningun archivo en uso: del
    /// catalogo, de la lista o de la botonera.
    fn limpiar_ondas(&mut self) {
        let mut en_uso: Vec<PathBuf> = self.repositorio_de_canciones.archivos().map(PathBuf::from).collect();
        en_uso.extend(self.lista.pistas().iter().map(|pista| pista.archivo().to_path_buf()));
        en_uso.extend(self.repositorio_de_botonera.efectos().map(|(_, _, archivo)| archivo.to_path_buf()));
        let aviso = match limpiar_cache_de_ondas(&carpeta_del_usuario().join("ondas"), en_uso.iter().map(PathBuf::as_path)) {
            Ok((0, _)) => tr!("La cache de ondas ya estaba limpia."),
            Ok((borradas, bytes)) => {
                tr!("Se borraron {borradas} forma(s) de onda ({:.1} MB).", bytes as f64 / 1_048_576.0, borradas = borradas)
            }
            Err(error) => tr!("No se pudo limpiar la cache de ondas: {error}", error = error),
        };
        self.mantenimiento.set_aviso(aviso.into());
    }
}
