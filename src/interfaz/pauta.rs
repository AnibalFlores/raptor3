//! Ventana de la pauta: la tabla de tandas programadas y el formulario para
//! crearlas o cambiarlas (avisos, horarios, dias, tolerancia, modo y
//! prioridad). Lo que se guarda pasa por `programar_tanda`, con las mismas
//! validaciones que por MCP.

use std::path::PathBuf;
use std::rc::Rc;

use jiff::SignedDuration;
use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, VecModel};

use super::avisos::DestinoDeAvisos;
use super::tandas::{describir_minutos, fecha_corta, formatear_cuenta};
use super::{Controlador, con_controlador, formatear_minutos};
use crate::dominio::{
    ModoDeEntrada, Pista, PRIORIDAD_POR_DEFECTO, Recurrencia, TOLERANCIA_POSTERIOR_POR_DEFECTO,
    TOLERANCIA_PREVIA_POR_DEFECTO, TandaProgramada, TipoDeElemento, formatear_hora, nombre_de_archivo, parsear_hora,
};
use crate::persistencia::CambiosDeDatos;
use crate::{AvisoDeTanda, FilaDeTanda, PautaDeTandas};

/// Tipos que se pueden asignar a un aviso desde la pauta y las ordenes, en
/// el orden del selector de la ventana.
const TIPOS_DE_AVISO: [TipoDeElemento; 4] =
    [TipoDeElemento::Comercial, TipoDeElemento::Publicidad, TipoDeElemento::Anuncio, TipoDeElemento::Podcast];
// Hasta cuanto falta para mostrar la cuenta regresiva de la proxima salida.
const HORIZONTE_DE_LA_CUENTA: f64 = 86_400.0;

/// La ventana de la pauta y lo que se esta editando.
pub(super) struct VentanaDePauta {
    ventana: PautaDeTandas,
    filas: Rc<VecModel<FilaDeTanda>>,
    avisos: Rc<VecModel<AvisoDeTanda>>,
    // Nombre guardado de la tanda en el formulario (None si es nueva o no hay).
    elegida: Option<String>,
    es_nueva: bool,
    // Avisos del formulario, todavia sin guardar.
    borrador: Vec<PathBuf>,
    aviso_elegido: Option<usize>,
}

impl VentanaDePauta {
    pub(super) fn nueva() -> Result<Self, String> {
        let ventana = PautaDeTandas::new().map_err(|error| tr!("No se pudo crear la ventana de la pauta: {error}", error = error))?;
        let filas = Rc::new(VecModel::default());
        let avisos = Rc::new(VecModel::default());
        ventana.set_tandas(ModelRc::from(filas.clone()));
        ventana.set_avisos(ModelRc::from(avisos.clone()));
        cablear(&ventana);
        Ok(Self { ventana, filas, avisos, elegida: None, es_nueva: false, borrador: Vec::new(), aviso_elegido: None })
    }

    pub(super) fn visible(&self) -> bool {
        self.ventana.window().is_visible()
    }

    pub(super) fn ventana(&self) -> &slint::Window {
        self.ventana.window()
    }

    /// Nombre escrito en el formulario (vacio si todavia no tiene).
    pub(super) fn nombre_en_el_formulario(&self) -> String {
        self.ventana.get_nombre().trim().to_string()
    }

    /// Avisos del formulario, todavia sin guardar.
    pub(super) fn avisos_en_el_formulario(&self) -> &[PathBuf] {
        &self.borrador
    }

    /// Suma avisos al formulario sin repetir; devuelve cuantos entraron.
    pub(super) fn sumar_avisos(&mut self, archivos: Vec<PathBuf>) -> usize {
        let antes = self.borrador.len();
        for archivo in archivos {
            if !self.borrador.contains(&archivo) {
                self.borrador.push(archivo);
            }
        }
        self.borrador.len() - antes
    }
}

fn cablear(ventana: &PautaDeTandas) {
    ventana.on_elegir_tanda(|indice| con_controlador(|c| c.elegir_tanda_de_la_pauta(indice as usize)));
    ventana.on_alternar_activa(|indice| con_controlador(|c| c.alternar_activa_en_la_pauta(indice as usize)));
    ventana.on_nueva(|| con_controlador(Controlador::nueva_tanda_en_la_pauta));
    let debil = ventana.as_weak();
    ventana.on_quitar(move || {
        let mut nombre = None;
        con_controlador(|c| nombre = c.pauta.elegida.clone());
        let Some(nombre) = nombre else { return };
        let mut dialogo = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&tr!("Quitar tanda"))
            .set_description(tr!("¿Quitar la tanda «{nombre}» de la programacion?\n\nLo que ya se encolo en la lista queda.", nombre = nombre))
            .set_buttons(rfd::MessageButtons::OkCancel);
        if let Some(ventana) = debil.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        if dialogo.show() == rfd::MessageDialogResult::Ok {
            con_controlador(Controlador::quitar_tanda_de_la_pauta);
        }
    });
    ventana.on_encolar_ahora(|| con_controlador(Controlador::encolar_desde_la_pauta));
    ventana.on_guardar(|| con_controlador(Controlador::guardar_tanda_de_la_pauta));
    ventana.on_descartar(|| con_controlador(Controlador::descartar_cambios_de_la_pauta));
    ventana.on_agregar_avisos(|| con_controlador(|c| c.abrir_selector_de_avisos(DestinoDeAvisos::Tanda)));
    ventana.on_elegir_aviso(|indice| con_controlador(|c| c.elegir_aviso_de_la_pauta(indice as usize)));
    ventana.on_quitar_aviso(|| con_controlador(Controlador::quitar_aviso_de_la_pauta));
    ventana.on_subir_aviso(|| con_controlador(|c| c.mover_aviso_de_la_pauta(true)));
    ventana.on_bajar_aviso(|| con_controlador(|c| c.mover_aviso_de_la_pauta(false)));
    ventana.on_asignar_tipo(|| con_controlador(Controlador::asignar_tipo_en_la_pauta));
    ventana.on_cerrar(|| con_controlador(|c| {
        let _ = c.pauta.ventana.hide();
    }));
    ventana.window().on_close_requested(|| CloseRequestResponse::HideWindow);
}

fn ahora() -> jiff::civil::DateTime {
    jiff::Zoned::now().datetime()
}

impl Controlador {
    /// Abre (o trae al frente) la pauta. Sin nada elegido, muestra la
    /// primera tanda o, si no hay, el formulario de una nueva.
    pub fn abrir_pauta(&mut self) -> Result<(), String> {
        if self.pauta.elegida.is_none() && !self.pauta.es_nueva {
            match self.tandas.tandas().first().cloned() {
                Some(tanda) => self.cargar_formulario_de_la_pauta(Some(&tanda)),
                None => self.cargar_formulario_de_la_pauta(None),
            }
        }
        self.refrescar_tabla_de_la_pauta();
        let ventana = self.pauta.ventana.clone_strong();
        self.mostrar_sobre_la_principal(ventana.window(), || ventana.show())
    }

    /// Refresca la tabla (la cuenta regresiva, una vez por segundo).
    pub(super) fn refrescar_tabla_de_la_pauta(&self) {
        let ahora = ahora();
        let filas: Vec<FilaDeTanda> = self
            .tandas
            .tandas()
            .iter()
            .map(|tanda| {
                let duracion: f64 = tanda.archivos.iter().filter_map(|archivo| self.duracion_de_efecto(archivo)).sum();
                let problema = tanda.archivos.iter().any(|archivo| !archivo.is_file() || self.motivo_para_no_ir_en_tanda(archivo).is_some());
                let proxima = match tanda.activa.then(|| tanda.pendientes(ahora).proxima).flatten() {
                    Some(salida) => {
                        let falta = salida.duration_since(ahora).as_secs_f64();
                        if falta < HORIZONTE_DE_LA_CUENTA {
                            format!("{} (en {})", fecha_corta(salida), formatear_cuenta(falta))
                        } else {
                            fecha_corta(salida)
                        }
                    }
                    None if tanda.activa => tr!("sin proximas").into(),
                    None => "inactiva".into(),
                };
                let tolerancia = format!("-{} / +{}", describir_minutos(tanda.antes), describir_minutos(tanda.despues));
                let entrada = match tanda.modo {
                    ModoDeEntrada::Tolerancia => tolerancia,
                    ModoDeEntrada::Exacta => tr!("Exacta ({tolerancia})", tolerancia = tolerancia),
                };
                FilaDeTanda {
                    nombre: tanda.nombre.clone().into(),
                    regla: tanda.recurrencia.describir().into(),
                    entrada: entrada.into(),
                    avisos: format!("{} aviso(s), {}", tanda.archivos.len(), formatear_minutos(duracion)).into(),
                    proxima: proxima.into(),
                    activa: tanda.activa,
                    problema,
                    seleccionada: self.pauta.elegida.as_deref().is_some_and(|elegida| tanda.se_llama(elegida)),
                }
            })
            .collect();
        // Se reemplaza fila por fila para no perder el desplazamiento de la tabla.
        let modelo = &self.pauta.filas;
        if modelo.row_count() == filas.len() {
            for (indice, fila) in filas.into_iter().enumerate() {
                if modelo.row_data(indice).as_ref() != Some(&fila) {
                    modelo.set_row_data(indice, fila);
                }
            }
        } else {
            modelo.set_vec(filas);
        }
    }

    /// Pasa la tanda (o, con None, una nueva con los valores por defecto)
    /// al formulario.
    fn cargar_formulario_de_la_pauta(&mut self, tanda: Option<&TandaProgramada>) {
        let ventana = &self.pauta.ventana;
        let dias = tanda.map_or([true; 7], |tanda| tanda.recurrencia.dias());
        ventana.set_nombre(tanda.map(|tanda| tanda.nombre.clone()).unwrap_or_default().into());
        ventana.set_inicio(tanda.map(|tanda| formatear_hora(tanda.recurrencia.inicio())).unwrap_or_default().into());
        ventana.set_cada_minutos(tanda.and_then(|tanda| tanda.recurrencia.cada_minutos()).unwrap_or(0) as i32);
        ventana.set_hasta(tanda.and_then(|tanda| tanda.recurrencia.hasta()).map(formatear_hora).unwrap_or_default().into());
        ventana.set_lun(dias[0]);
        ventana.set_mar(dias[1]);
        ventana.set_mie(dias[2]);
        ventana.set_jue(dias[3]);
        ventana.set_vie(dias[4]);
        ventana.set_sab(dias[5]);
        ventana.set_dom(dias[6]);
        ventana.set_antes_minutos((tanda.map_or(TOLERANCIA_PREVIA_POR_DEFECTO, |tanda| tanda.antes) / 60) as i32);
        ventana.set_despues_minutos((tanda.map_or(TOLERANCIA_POSTERIOR_POR_DEFECTO, |tanda| tanda.despues) / 60) as i32);
        ventana.set_modo(i32::from(tanda.is_some_and(|tanda| tanda.modo == ModoDeEntrada::Exacta)));
        ventana.set_prioridad(i32::from(tanda.map_or(PRIORIDAD_POR_DEFECTO, |tanda| tanda.prioridad)));
        ventana.set_activa(tanda.is_none_or(|tanda| tanda.activa));
        ventana.set_duracion_maxima(tanda.and_then(|tanda| tanda.duracion_maxima).unwrap_or(0) as i32);
        ventana.set_editando(true);
        ventana.set_es_nueva(tanda.is_none());
        self.pauta.elegida = tanda.map(|tanda| tanda.nombre.clone());
        self.pauta.es_nueva = tanda.is_none();
        self.pauta.borrador = tanda.map(|tanda| tanda.archivos.clone()).unwrap_or_default();
        self.pauta.aviso_elegido = None;
        self.refrescar_avisos_de_la_pauta();
        self.refrescar_tabla_de_la_pauta();
    }

    pub(super) fn avisar_en_la_pauta(&self, aviso: &str, es_error: bool) {
        self.pauta.ventana.set_aviso(aviso.into());
        self.pauta.ventana.set_aviso_es_error(es_error);
    }

    fn elegir_tanda_de_la_pauta(&mut self, indice: usize) {
        if let Some(tanda) = self.tandas.tandas().get(indice).cloned() {
            self.cargar_formulario_de_la_pauta(Some(&tanda));
            self.avisar_en_la_pauta("", false);
        }
    }

    fn nueva_tanda_en_la_pauta(&mut self) {
        self.cargar_formulario_de_la_pauta(None);
        self.avisar_en_la_pauta(&tr!("Tanda nueva: complete el nombre, el primer horario y los avisos, y pulse Programar tanda."), false);
    }

    fn descartar_cambios_de_la_pauta(&mut self) {
        let guardada = self.pauta.elegida.as_deref().and_then(|nombre| self.tandas.tandas().iter().find(|tanda| tanda.se_llama(nombre)).cloned());
        self.cargar_formulario_de_la_pauta(guardada.as_ref());
        self.avisar_en_la_pauta(&tr!("Cambios descartados."), false);
    }

    /// Arma la tanda con lo que dice el formulario.
    fn tanda_del_formulario(&self) -> Result<TandaProgramada, String> {
        let ventana = &self.pauta.ventana;
        let nombre = ventana.get_nombre().trim().to_string();
        if nombre.is_empty() {
            return Err(tr!("Indique el nombre de la tanda.").into());
        }
        let inicio = parsear_hora(&ventana.get_inicio())?;
        let cada = u32::try_from(ventana.get_cada_minutos()).ok().filter(|minutos| *minutos > 0);
        let hasta = match ventana.get_hasta().trim() {
            "" => None,
            _ if cada.is_none() => None,
            texto => Some(parsear_hora(texto)?),
        };
        let dias = [ventana.get_lun(), ventana.get_mar(), ventana.get_mie(), ventana.get_jue(), ventana.get_vie(), ventana.get_sab(), ventana.get_dom()];
        let minutos = |valor: i32| (valor.clamp(0, 60) as u32) * 60;
        let guardada = self.pauta.elegida.as_deref().and_then(|elegida| self.tandas.tandas().iter().find(|tanda| tanda.se_llama(elegida)));
        Ok(TandaProgramada {
            nombre,
            archivos: self.pauta.borrador.clone(),
            recurrencia: Recurrencia::nueva(inicio, cada, hasta, dias)?,
            antes: minutos(ventana.get_antes_minutos()),
            despues: minutos(ventana.get_despues_minutos()),
            modo: if ventana.get_modo() == 1 { ModoDeEntrada::Exacta } else { ModoDeEntrada::Tolerancia },
            prioridad: ventana.get_prioridad().clamp(1, 9) as u8,
            duracion_maxima: u32::try_from(ventana.get_duracion_maxima()).ok().filter(|segundos| *segundos > 0),
            activa: ventana.get_activa(),
            // Al renombrarla conserva lo que ya salio.
            ultima_salida: guardada.and_then(|tanda| tanda.ultima_salida),
        })
    }

    fn guardar_tanda_de_la_pauta(&mut self) {
        let tanda = match self.tanda_del_formulario() {
            Ok(tanda) => tanda,
            Err(error) => return self.avisar_en_la_pauta(&error, true),
        };
        let anterior = self.pauta.elegida.clone();
        let renombrada = anterior.as_deref().filter(|anterior| !tanda.se_llama(anterior));
        let ocupado = self.tandas.tandas().iter().any(|otra| otra.se_llama(&tanda.nombre));
        if (self.pauta.es_nueva || renombrada.is_some()) && ocupado {
            return self.avisar_en_la_pauta(&tr!("Ya hay una tanda llamada «{}»: elija otro nombre.", tanda.nombre), true);
        }
        let nombre = tanda.nombre.clone();
        match self.programar_tanda(tanda) {
            Ok(mensaje) => {
                if let Some(anterior) = renombrada {
                    self.tandas.quitar_tanda(anterior);
                    self.invalidar_pauta();
                }
                let guardada = self.tandas.tandas().iter().find(|tanda| tanda.se_llama(&nombre)).cloned();
                self.cargar_formulario_de_la_pauta(guardada.as_ref());
                self.avisar_en_la_pauta(&mensaje, false);
            }
            Err(error) => self.avisar_en_la_pauta(&error, true),
        }
    }

    fn quitar_tanda_de_la_pauta(&mut self) {
        let Some(nombre) = self.pauta.elegida.clone() else { return };
        match self.quitar_tanda(&nombre) {
            Ok(mensaje) => {
                let primera = self.tandas.tandas().first().cloned();
                self.cargar_formulario_de_la_pauta(primera.as_ref());
                self.avisar_en_la_pauta(&mensaje, false);
            }
            Err(error) => self.avisar_en_la_pauta(&error, true),
        }
    }

    fn encolar_desde_la_pauta(&mut self) {
        let Some(nombre) = self.pauta.elegida.clone() else { return };
        match self.encolar_tanda_ahora(&nombre) {
            Ok(mensaje) => self.avisar_en_la_pauta(&mensaje, false),
            Err(error) => self.avisar_en_la_pauta(&error, true),
        }
    }

    /// Activa o desactiva una tanda desde la tabla, sin pasar por el
    /// formulario. Al activarla, lo que ya paso no se cuenta como perdido.
    fn alternar_activa_en_la_pauta(&mut self, indice: usize) {
        let Some(mut tanda) = self.tandas.tandas().get(indice).cloned() else { return };
        tanda.activa = !tanda.activa;
        if tanda.activa {
            let ahora = ahora();
            let desde = ahora.checked_sub(SignedDuration::from_secs(i64::from(tanda.despues) + 1)).unwrap_or(ahora);
            tanda.ultima_salida = Some(tanda.ultima_salida.map_or(desde, |ultima| ultima.max(desde)));
        }
        let (nombre, activa) = (tanda.nombre.clone(), tanda.activa);
        self.tandas.guardar_tanda(tanda);
        self.invalidar_pauta();
        if self.pauta.elegida.as_deref() == Some(nombre.as_str()) {
            self.pauta.ventana.set_activa(activa);
        }
        self.refrescar_tabla_de_la_pauta();
        self.avisar_en_la_pauta(&tr!("Tanda «{nombre}» {}", if activa { tr!("activada") } else { tr!("desactivada: no sale hasta volver a activarla") }, nombre = nombre), false);
    }

    fn elegir_aviso_de_la_pauta(&mut self, indice: usize) {
        self.pauta.aviso_elegido = (indice < self.pauta.borrador.len()).then_some(indice);
        self.refrescar_avisos_de_la_pauta();
    }

    fn quitar_aviso_de_la_pauta(&mut self) {
        if let Some(indice) = self.pauta.aviso_elegido.filter(|&indice| indice < self.pauta.borrador.len()) {
            self.pauta.borrador.remove(indice);
            self.pauta.aviso_elegido = (!self.pauta.borrador.is_empty()).then(|| indice.min(self.pauta.borrador.len() - 1));
            self.refrescar_avisos_de_la_pauta();
        }
    }

    fn mover_aviso_de_la_pauta(&mut self, subir: bool) {
        let Some(indice) = self.pauta.aviso_elegido else { return };
        let destino = if subir { indice.checked_sub(1) } else { Some(indice + 1).filter(|&destino| destino < self.pauta.borrador.len()) };
        if let Some(destino) = destino {
            self.pauta.borrador.swap(indice, destino);
            self.pauta.aviso_elegido = Some(destino);
            self.refrescar_avisos_de_la_pauta();
        }
    }

    /// Asigna al aviso elegido el tipo del selector (en el catalogo).
    fn asignar_tipo_en_la_pauta(&mut self) {
        let Some(archivo) = self.pauta.aviso_elegido.and_then(|indice| self.pauta.borrador.get(indice)).cloned() else { return };
        let mensaje = self.asignar_tipo_de_aviso(&archivo, self.pauta.ventana.get_tipo_a_asignar());
        self.refrescar_avisos_de_la_pauta();
        self.refrescar_tabla_de_la_pauta();
        self.avisar_en_la_pauta(&mensaje, false);
    }

    /// Asigna a un aviso uno de los tipos del selector (Comercial,
    /// Publicidad o Anuncio), en el catalogo. Devuelve el mensaje.
    pub(super) fn asignar_tipo_de_aviso(&mut self, archivo: &std::path::Path, opcion: i32) -> String {
        let tipo = TIPOS_DE_AVISO[opcion.clamp(0, TIPOS_DE_AVISO.len() as i32 - 1) as usize];
        self.cambiar_datos(&Pista::new(archivo), CambiosDeDatos { tipo: Some(Some(tipo)), ..CambiosDeDatos::default() });
        self.refrescar_filas();
        self.invalidar_pauta();
        tr!("{}: tipo {}", nombre_de_archivo(archivo), tipo.nombre_visible())
    }

    pub(super) fn refrescar_avisos_de_la_pauta(&self) {
        let avisos = self.avisos_para_mostrar(&self.pauta.borrador, self.pauta.aviso_elegido);
        self.pauta.avisos.set_vec(avisos);
        self.pauta.ventana.set_hay_aviso_elegido(self.pauta.aviso_elegido.is_some());
    }

    /// Filas de una lista de avisos (de una tanda o de una orden).
    pub(super) fn avisos_para_mostrar(&self, archivos: &[PathBuf], elegido: Option<usize>) -> Vec<AvisoDeTanda> {
        archivos
            .iter()
            .enumerate()
            .map(|(indice, archivo)| {
                let ficha = self.repositorio_de_canciones.buscar(archivo);
                let titulo = ficha.and_then(|ficha| ficha.titulo.clone()).filter(|titulo| !titulo.trim().is_empty());
                let artista = ficha.and_then(|ficha| ficha.artista.clone()).filter(|artista| !artista.trim().is_empty());
                let nombre = match (titulo, artista) {
                    (Some(titulo), Some(artista)) => format!("{titulo} - {artista}"),
                    (Some(titulo), None) => titulo,
                    (None, _) => nombre_de_archivo(archivo),
                };
                let problema = if archivo.is_file() {
                    self.motivo_para_no_ir_en_tanda(archivo).unwrap_or_default()
                } else {
                    tr!("No existe el archivo").into()
                };
                AvisoDeTanda {
                    nombre: nombre.into(),
                    tipo: ficha.and_then(|ficha| ficha.tipo).map_or("sin tipo", TipoDeElemento::nombre).into(),
                    duracion: self.duracion_de_efecto(archivo).map(formatear_minutos).unwrap_or_default().into(),
                    ruta: archivo.display().to_string().into(),
                    problema: problema.into(),
                    seleccionado: elegido == Some(indice),
                }
            })
            .collect()
    }

    /// Una vez por segundo: la proxima tanda en la ventana principal y, si
    /// la pauta esta abierta, su tabla.
    pub(super) fn refrescar_pauta(&self) {
        let texto = self.texto_de_la_proxima_tanda();
        self.con_ventana(|ventana| {
            if ventana.get_proxima_tanda() != texto.as_str() {
                ventana.set_proxima_tanda(texto.into());
            }
        });
        if self.pauta.visible() {
            self.refrescar_tabla_de_la_pauta();
        }
    }
}
