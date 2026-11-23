//! Ordenes de publicidad en el controlador: la pauta del dia (el reparto de
//! sus avisos en las tandas programadas), las reposiciones de lo que se
//! perdio o se corto, el certificado de emision por anunciante y la ventana
//! de las ordenes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use jiff::civil::{Date, DateTime};
use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, VecModel};

use super::avisos::DestinoDeAvisos;
use super::{Controlador, abrir_con_el_sistema, con_controlador, formatear_minutos};
use crate::dominio::{
    Asignacion, AvisoEntregado, Frecuencia, Hueco, OrdenDePublicidad, PRIORIDAD_POR_DEFECTO, PautaDelDia, Reposicion,
    avisos_sin_entregar, etiqueta_de_salida, formatear_hora, nombre_de_archivo, parsear_hora, repartir, salidas_contratadas,
};
use crate::persistencia::{EmisionDeComercial, escribir_certificado};
use crate::{FilaDeOrden, LineaDePauta, OrdenesDePublicidad};

/// La pauta calculada de un dia, para no repartir otra vez en cada consulta.
pub(super) struct PautaCalculada {
    fecha: Date,
    pauta: PautaDelDia,
}

/// La ventana de las ordenes y lo que se esta editando.
pub(super) struct VentanaDeOrdenes {
    ventana: OrdenesDePublicidad,
    filas: Rc<VecModel<FilaDeOrden>>,
    avisos: Rc<VecModel<crate::AvisoDeTanda>>,
    lineas: Rc<VecModel<LineaDePauta>>,
    elegida: Option<String>,
    es_nueva: bool,
    borrador: Vec<PathBuf>,
    aviso_elegido: Option<usize>,
}

impl VentanaDeOrdenes {
    pub(super) fn nueva() -> Result<Self, String> {
        let ventana = OrdenesDePublicidad::new().map_err(|error| tr!("No se pudo crear la ventana de ordenes: {error}", error = error))?;
        let (filas, avisos, lineas) = (Rc::new(VecModel::default()), Rc::new(VecModel::default()), Rc::new(VecModel::default()));
        ventana.set_ordenes(ModelRc::from(filas.clone()));
        ventana.set_avisos(ModelRc::from(avisos.clone()));
        ventana.set_pauta_de_hoy(ModelRc::from(lineas.clone()));
        cablear(&ventana);
        Ok(Self { ventana, filas, avisos, lineas, elegida: None, es_nueva: false, borrador: Vec::new(), aviso_elegido: None })
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


    pub(super) fn ventana(&self) -> &slint::Window {
        self.ventana.window()
    }
}

fn cablear(ventana: &OrdenesDePublicidad) {
    ventana.on_elegir_orden(|indice| con_controlador(|c| c.elegir_orden(indice as usize)));
    ventana.on_alternar_activa(|indice| con_controlador(|c| c.alternar_orden_activa(indice as usize)));
    ventana.on_nueva(|| con_controlador(|c| {
        c.cargar_formulario_de_orden(None);
        c.avisar_en_ordenes(&tr!("Orden nueva: complete los datos y los avisos, y pulse Cargar orden."), false);
    }));
    let debil = ventana.as_weak();
    ventana.on_quitar(move || {
        let mut nombre = None;
        con_controlador(|c| nombre = c.ventana_de_ordenes.elegida.clone());
        let Some(nombre) = nombre else { return };
        let mut dialogo = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&tr!("Quitar orden"))
            .set_description(tr!("¿Quitar la orden «{nombre}»?\n\nSus avisos dejan de repartirse; lo emitido queda en el registro.", nombre = nombre))
            .set_buttons(rfd::MessageButtons::OkCancel);
        if let Some(ventana) = debil.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        if dialogo.show() == rfd::MessageDialogResult::Ok {
            con_controlador(|c| {
                let resultado = c.quitar_orden(&nombre);
                let primera = c.ordenes.ordenes().first().cloned();
                c.cargar_formulario_de_orden(primera.as_ref());
                match resultado {
                    Ok(mensaje) => c.avisar_en_ordenes(&mensaje, false),
                    Err(error) => c.avisar_en_ordenes(&error, true),
                }
            });
        }
    });
    ventana.on_certificado(|| con_controlador(Controlador::certificado_desde_la_ventana));
    ventana.on_anunciante_editado(|_| con_controlador(|c| c.refrescar_estado_del_anunciante(false)));
    ventana.on_ficha_del_anunciante(|| con_controlador(|c| {
        let nombre = c.ventana_de_ordenes.ventana.get_anunciante().to_string();
        if let Err(error) = c.abrir_anunciantes(Some(&nombre)) {
            c.avisar_en_ordenes(&error, true);
        }
    }));
    ventana.on_guardar(|| con_controlador(Controlador::guardar_orden_de_la_ventana));
    ventana.on_descartar(|| con_controlador(|c| {
        let guardada = c.ventana_de_ordenes.elegida.as_deref().and_then(|nombre| c.ordenes.ordenes().iter().find(|orden| orden.se_llama(nombre)).cloned());
        c.cargar_formulario_de_orden(guardada.as_ref());
        c.avisar_en_ordenes(&tr!("Cambios descartados."), false);
    }));
    ventana.on_agregar_avisos(|| con_controlador(|c| c.abrir_selector_de_avisos(DestinoDeAvisos::Orden)));
    ventana.on_elegir_aviso(|indice| con_controlador(|c| {
        c.ventana_de_ordenes.aviso_elegido = (indice >= 0 && (indice as usize) < c.ventana_de_ordenes.borrador.len()).then_some(indice as usize);
        c.refrescar_avisos_de_la_orden();
    }));
    ventana.on_quitar_aviso(|| con_controlador(|c| {
        let ventana = &mut c.ventana_de_ordenes;
        if let Some(indice) = ventana.aviso_elegido.filter(|&indice| indice < ventana.borrador.len()) {
            ventana.borrador.remove(indice);
            ventana.aviso_elegido = (!ventana.borrador.is_empty()).then(|| indice.min(ventana.borrador.len() - 1));
        }
        c.refrescar_avisos_de_la_orden();
    }));
    ventana.on_subir_aviso(|| con_controlador(|c| c.mover_aviso_de_la_orden(true)));
    ventana.on_bajar_aviso(|| con_controlador(|c| c.mover_aviso_de_la_orden(false)));
    ventana.on_asignar_tipo(|| con_controlador(|c| {
        let Some(archivo) = c.ventana_de_ordenes.aviso_elegido.and_then(|indice| c.ventana_de_ordenes.borrador.get(indice)).cloned() else { return };
        let mensaje = c.asignar_tipo_de_aviso(&archivo, c.ventana_de_ordenes.ventana.get_tipo_a_asignar());
        c.refrescar_avisos_de_la_orden();
        c.avisar_en_ordenes(&mensaje, false);
    }));
    ventana.on_cerrar(|| con_controlador(|c| {
        let _ = c.ventana_de_ordenes.ventana.hide();
    }));
    ventana.window().on_close_requested(|| CloseRequestResponse::HideWindow);
}

fn ahora() -> DateTime {
    jiff::Zoned::now().datetime()
}

fn hoy() -> Date {
    jiff::Zoned::now().date()
}

fn hora_corta(momento: DateTime) -> String {
    format!("{:02}:{:02}", momento.hour(), momento.minute())
}

/// Lista separada por comas, sin vacios.
fn separar(texto: &str) -> Vec<String> {
    texto.split([',', ';']).map(str::trim).filter(|parte| !parte.is_empty()).map(str::to_string).collect()
}

impl Controlador {
    // Pauta del dia

    /// La pauta de esa fecha: el reparto de las ordenes que rigen en las
    /// salidas de las tandas activas. Se calcula una vez y se vuelve a
    /// calcular al cambiar una orden, una tanda o las reposiciones.
    pub(super) fn pauta_de(&mut self, fecha: Date) -> &PautaDelDia {
        if self.pauta_calculada.as_ref().is_none_or(|calculada| calculada.fecha != fecha) {
            if fecha == hoy() && self.fecha_de_reposiciones != Some(fecha) {
                self.reposiciones = self.pautas.reposiciones(fecha);
                self.fecha_de_reposiciones = Some(fecha);
            }
            let huecos: Vec<Hueco> = self
                .tandas
                .tandas()
                .iter()
                .filter(|tanda| tanda.activa)
                .flat_map(|tanda| {
                    let ocupado: f64 = tanda.archivos.iter().filter_map(|archivo| self.duracion_de_efecto(archivo)).sum();
                    tanda.recurrencia.horarios_del_dia(fecha).into_iter().map(move |salida| Hueco {
                        tanda: tanda.nombre.clone(),
                        salida,
                        capacidad: tanda.duracion_maxima.map(f64::from),
                        ocupado,
                    })
                })
                .collect();
            let reposiciones = if fecha == hoy() { self.reposiciones.clone() } else { Vec::new() };
            let duracion = |archivo: &std::path::Path| self.duracion_de_efecto(archivo).unwrap_or(30.0);
            let pauta = repartir(fecha, self.ordenes.ordenes(), huecos, &reposiciones, &duracion);
            if fecha == hoy() {
                self.pautas.guardar_pauta(fecha, &pauta, &reposiciones);
            }
            self.pauta_calculada = Some(PautaCalculada { fecha, pauta });
        }
        &self.pauta_calculada.as_ref().expect("recien calculada").pauta
    }

    /// Hay que volver a repartir (cambio una orden, una tanda o una reposicion).
    pub(super) fn invalidar_pauta(&mut self) {
        self.pauta_calculada = None;
    }

    /// Los avisos de las ordenes para esa salida de esa tanda.
    pub(super) fn avisos_de_ordenes(&mut self, tanda: &str, salida: DateTime) -> Vec<Asignacion> {
        self.pauta_de(salida.date()).avisos_de(tanda, salida).into_iter().cloned().collect()
    }

    /// Una salida de una tanda se perdio: sus avisos de ordenes se reponen en
    /// la proxima tanda posible del dia.
    pub(super) fn reponer_salida_perdida(&mut self, tanda: &str, salida: DateTime) {
        if salida.date() != hoy() {
            return;
        }
        let perdidos = self.avisos_de_ordenes(tanda, salida);
        for asignacion in &perdidos {
            self.agregar_reposicion(&asignacion.orden, asignacion.aviso.clone());
        }
    }

    /// Despues de cargar o cambiar una orden: lo que la pauta le asigna a
    /// salidas de hoy que ya se atendieron y no se les entrego (la tanda se
    /// encolo antes que la orden) entra al final de su bloque si todavia esta
    /// en la lista, o se repone en la proxima tanda posible. Devuelve que
    /// hizo, para el mensaje; None si no faltaba nada.
    pub(super) fn completar_salidas_atendidas(&mut self) -> Option<String> {
        let fecha = hoy();
        let atendidas: Vec<(String, DateTime)> = self
            .tandas
            .tandas()
            .iter()
            .filter_map(|tanda| tanda.ultima_salida.filter(|ultima| ultima.date() == fecha).map(|ultima| (tanda.nombre.clone(), ultima)))
            .collect();
        if atendidas.is_empty() {
            return None;
        }
        let pauta = self.pauta_de(fecha).clone();
        let mut entregados: Vec<AvisoEntregado> = self
            .pistas_de_tanda
            .iter()
            .flat_map(|(pista, cola)| {
                cola.iter().filter_map(move |origen| {
                    origen.orden.clone().map(|orden| AvisoEntregado { etiqueta: origen.tanda.clone(), orden, aviso: pista.archivo().to_path_buf() })
                })
            })
            .collect();
        match self.registro_de_comerciales.emisiones_entre(fecha, fecha) {
            Ok(emisiones) => entregados.extend(
                emisiones
                    .into_iter()
                    .filter(|emision| !emision.orden.is_empty())
                    .map(|emision| AvisoEntregado { etiqueta: emision.tanda, orden: emision.orden, aviso: emision.archivo }),
            ),
            Err(error) => {
                // Sin saber que salio al aire no se repone nada: seria duplicar.
                log::warn!("No se pudo leer el registro de comerciales para completar las tandas ya atendidas: {error}");
                return None;
            }
        }
        let faltan = avisos_sin_entregar(&pauta, &atendidas, &entregados, &self.reposiciones);
        if faltan.is_empty() {
            return None;
        }
        let (mut en_su_bloque, mut repuestos) = (0, 0);
        for asignacion in faltan {
            if self.sumar_al_bloque_encolado(&asignacion) {
                en_su_bloque += 1;
            } else {
                self.agregar_reposicion(&asignacion.orden, asignacion.aviso.clone());
                repuestos += 1;
            }
        }
        self.refrescar_filas();
        let mut partes = Vec::new();
        if en_su_bloque > 0 {
            partes.push(tr!("{en_su_bloque} aviso(s) se sumaron a su tanda ya encolada", en_su_bloque = en_su_bloque));
        }
        if repuestos > 0 {
            partes.push(tr!("{repuestos} aviso(s) de tandas que ya salieron se reponen en la proxima tanda posible", repuestos = repuestos));
        }
        let resumen = partes.join("; ");
        log::info!("Ordenes en tandas ya atendidas: {resumen}");
        Some(resumen)
    }

    /// Agrega el aviso al final del bloque de su salida si ese bloque todavia
    /// esta en la lista por sonar (o sonando). Las colas de origen van por
    /// archivo y en el orden de la lista: la ocurrencia numero k de un archivo,
    /// desde la pista actual, es la k-esima de su cola.
    fn sumar_al_bloque_encolado(&mut self, asignacion: &Asignacion) -> bool {
        let etiqueta = etiqueta_de_salida(&asignacion.tanda, asignacion.salida);
        let desde = self.lista.indice_actual().unwrap_or(0);
        let mut vistas: HashMap<PathBuf, usize> = HashMap::new();
        let mut ultimo = None;
        for indice in desde..self.lista.tamano() {
            let Some(pista) = self.lista.pista_en(indice) else { continue };
            let Some(cola) = self.pistas_de_tanda.get(pista) else { continue };
            let vista = vistas.entry(pista.archivo().to_path_buf()).or_default();
            if cola.get(*vista).is_some_and(|origen| origen.tanda == etiqueta) {
                ultimo = Some(indice);
            }
            *vista += 1;
        }
        let Some(ultimo) = ultimo else { return false };
        let posicion = ultimo + 1;
        let (seleccion, ancla) = (self.seleccion.clone(), self.ancla_de_seleccion);
        let Some(nueva) = self.insertar_pistas(vec![asignacion.aviso.clone()], Some(posicion), true).into_iter().next() else {
            return false;
        };
        let corrida = |indice: usize| if indice >= posicion { indice + 1 } else { indice };
        self.seleccion = seleccion.into_iter().map(corrida).collect();
        self.ancla_de_seleccion = ancla.map(corrida);
        let antes = (desde..posicion).filter(|&indice| self.lista.pista_en(indice) == Some(&nueva)).count();
        let cola = self.pistas_de_tanda.entry(nueva).or_default();
        let lugar = antes.min(cola.len());
        cola.insert(lugar, OrigenDelAviso { tanda: etiqueta.clone(), orden: Some(asignacion.orden.clone()) });
        log::info!("Aviso {} de la orden {} sumado a la tanda {etiqueta} ya encolada", asignacion.aviso.display(), asignacion.orden);
        true
    }

    /// Un aviso de una orden salio cortado o se perdio: se repone hoy.
    pub(super) fn agregar_reposicion(&mut self, orden: &str, aviso: PathBuf) {
        let _ = self.pauta_de(hoy());
        log::info!("Se repone hoy un aviso de la orden {orden}: {}", aviso.display());
        self.reposiciones.push(Reposicion { orden: orden.to_string(), aviso, desde: ahora() });
        self.invalidar_pauta();
        let _ = self.pauta_de(hoy());
    }

    /// La pauta de un dia en texto: cada salida con sus avisos de ordenes y
    /// lo que no se pudo ubicar.
    pub fn describir_pauta_del_dia(&mut self, fecha: Date) -> String {
        let lineas = self.lineas_de_la_pauta(fecha);
        if lineas.is_empty() {
            return tr!("El {fecha} no hay avisos de ordenes para repartir.", fecha = fecha);
        }
        let mut texto = tr!("Pauta del {fecha}:\n", fecha = fecha);
        for (linea, _) in lineas {
            texto.push_str(&linea);
            texto.push('\n');
        }
        texto
    }

    /// Lineas de la pauta de un dia (con true las que avisan un problema).
    fn lineas_de_la_pauta(&mut self, fecha: Date) -> Vec<(String, bool)> {
        let pauta = self.pauta_de(fecha).clone();
        let mut por_salida: Vec<((DateTime, String), Vec<&Asignacion>)> = Vec::new();
        for asignacion in &pauta.asignaciones {
            let clave = (asignacion.salida, asignacion.tanda.clone());
            match por_salida.iter_mut().find(|(otra, _)| *otra == clave) {
                Some((_, avisos)) => avisos.push(asignacion),
                None => por_salida.push((clave, vec![asignacion])),
            }
        }
        let mut lineas = Vec::new();
        for ((salida, tanda), avisos) in por_salida {
            let datos = self.tandas.tandas().iter().find(|otra| otra.se_llama(&tanda)).cloned();
            let fijos: f64 = datos.iter().flat_map(|tanda| &tanda.archivos).filter_map(|archivo| self.duracion_de_efecto(archivo)).sum();
            let de_ordenes: f64 = avisos.iter().filter_map(|asignacion| self.duracion_de_efecto(&asignacion.aviso)).sum();
            let detalle: Vec<String> = avisos
                .iter()
                .map(|asignacion| {
                    let repuesto = if asignacion.reposicion { " (reposicion)" } else { "" };
                    format!("{} ({}){repuesto}", asignacion.orden, nombre_de_archivo(&asignacion.aviso))
                })
                .collect();
            let limite = datos.and_then(|tanda| tanda.duracion_maxima).map_or(String::new(), |maximo| format!(" de {}", formatear_minutos(f64::from(maximo))));
            lineas.push((
                format!("{} {tanda}: {} | {}{limite}", hora_corta(salida), detalle.join(", "), formatear_minutos(fijos + de_ordenes)),
                false,
            ));
        }
        for sin_ubicar in &pauta.sin_ubicar {
            lineas.push((tr!("SIN UBICAR {} {}: {}", hora_corta(sin_ubicar.objetivo), sin_ubicar.orden, sin_ubicar.motivo), true));
        }
        lineas
    }

    // Ordenes

    /// Carga una orden nueva o reemplaza la del mismo nombre. Sus avisos
    /// deben poder ir en una tanda y las tandas que nombra, existir.
    pub fn programar_orden(&mut self, mut orden: OrdenDePublicidad) -> Result<String, String> {
        orden.validar()?;
        let reemplaza = self.ordenes.ordenes().iter().any(|guardada| guardada.se_llama(&orden.nombre));
        // El anunciante: con ficha, toma su nombre y (si no indica otro) su rubro.
        let sin_ficha = match self.ficha_de_anunciante(&orden.anunciante).cloned() {
            Some(ficha) if !ficha.activo && !reemplaza => {
                return Err(tr!("El anunciante «{}» esta inactivo: activelo en Anunciantes para cargarle ordenes.", ficha.nombre));
            }
            Some(ficha) => {
                orden.anunciante = ficha.nombre;
                if orden.rubro.trim().is_empty() {
                    orden.rubro = ficha.rubro;
                }
                false
            }
            None => true,
        };
        let problemas: Vec<String> = orden
            .avisos
            .iter()
            .filter_map(|aviso| if aviso.is_file() { self.motivo_para_no_ir_en_tanda(aviso) } else { Some(tr!("{}: no existe", aviso.display())) })
            .collect();
        if !problemas.is_empty() {
            return Err(tr!("La orden no se cargo. {}.", problemas.join("; ")));
        }
        let desconocidas: Vec<&String> =
            orden.tandas.iter().filter(|nombre| !self.tandas.tandas().iter().any(|tanda| tanda.se_llama(nombre))).collect();
        if !desconocidas.is_empty() {
            return Err(tr!("La orden no se cargo: no hay tandas llamadas {}.", desconocidas.iter().map(|nombre| format!("«{nombre}»")).collect::<Vec<_>>().join(", ")));
        }
        let (nombre, anunciante) = (orden.nombre.clone(), orden.anunciante.clone());
        self.ordenes.guardar_orden(orden);
        self.invalidar_pauta();
        let resumen = self.resumen_de_orden(&nombre);
        let mut mensaje = if reemplaza { tr!("Orden actualizada: {resumen}", resumen = resumen) } else { tr!("Orden cargada: {resumen}", resumen = resumen) };
        if let Some(completadas) = self.completar_salidas_atendidas() {
            mensaje.push_str(&format!(". {completadas}"));
        }
        if sin_ficha {
            mensaje.push_str(&tr!(". Atencion: «{anunciante}» no tiene ficha de anunciante; carguela en Archivo › Anunciantes (razon social, CUIT, contacto).", anunciante = anunciante));
        }
        if self.anunciantes_de_la_ventana.ventana().is_visible() {
            self.refrescar_tabla_de_anunciantes();
        }
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    pub fn quitar_orden(&mut self, nombre: &str) -> Result<String, String> {
        let quitada = self.ordenes.quitar_orden(nombre).ok_or_else(|| tr!("No hay una orden llamada «{nombre}».", nombre = nombre))?;
        self.invalidar_pauta();
        let mensaje = tr!("Orden «{}» quitada: sus avisos dejan de repartirse", quitada.nombre);
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Cuantas salidas de la orden se ubicaron hoy y cuantas pide, o por que
    /// no rige ("no rige hoy", "vencida", "inactiva").
    fn estado_de_hoy(&mut self, orden: &OrdenDePublicidad) -> (String, bool) {
        let fecha = hoy();
        if !orden.activa {
            return ("inactiva".into(), false);
        }
        if fecha > orden.hasta {
            return ("vencida".into(), false);
        }
        if fecha < orden.desde {
            return (tr!("desde el {}", orden.desde), false);
        }
        let pedidas = orden.demandas_del_dia(fecha).len();
        if pedidas == 0 {
            return (tr!("no rige hoy").into(), false);
        }
        let pauta = self.pauta_de(fecha);
        let ubicadas = pauta.asignaciones.iter().filter(|asignacion| orden.se_llama(&asignacion.orden) && !asignacion.reposicion).count();
        (format!("{ubicadas} de {pedidas}"), ubicadas < pedidas)
    }

    fn resumen_de_orden(&mut self, nombre: &str) -> String {
        let Some(orden) = self.ordenes.ordenes().iter().find(|orden| orden.se_llama(nombre)).cloned() else {
            return String::new();
        };
        let (hoy, _) = self.estado_de_hoy(&orden);
        let tandas = if orden.tandas.is_empty() { tr!("cualquier tanda") } else { orden.tandas.join(", ") };
        tr!("«{}» de {}{}: {}; {} aviso(s) en rotacion, en {tandas}; prioridad {} | hoy: {hoy}", orden.nombre, orden.anunciante, if orden.rubro.is_empty() { String::new() } else { format!(" ({})", orden.rubro) }, orden.describir(), orden.avisos.len(), orden.prioridad, tandas = tandas, hoy = hoy)
    }

    /// Las ordenes cargadas, con su estado de hoy.
    pub fn describir_ordenes(&mut self) -> String {
        let nombres: Vec<String> = self.ordenes.ordenes().iter().map(|orden| orden.nombre.clone()).collect();
        if nombres.is_empty() {
            return tr!("No hay ordenes de publicidad.").into();
        }
        let mut texto = format!("{} orden(es):\n", nombres.len());
        for (numero, nombre) in nombres.iter().enumerate() {
            texto.push_str(&format!("{}. {}\n", numero + 1, self.resumen_de_orden(nombre)));
        }
        texto
    }

    /// Certificado de emision de un anunciante entre dos fechas: por orden,
    /// las salidas contratadas, las emitidas completas y las cortadas, y el
    /// detalle de cada salida. Lo deja tambien en un CSV y devuelve su ruta.
    pub fn certificado_de_emision(&mut self, anunciante: &str, desde: Date, hasta: Date) -> Result<(String, PathBuf), String> {
        let simplificado = crate::dominio::simplificar_nombre;
        let ordenes: Vec<OrdenDePublicidad> =
            self.ordenes.ordenes().iter().filter(|orden| simplificado(&orden.anunciante) == simplificado(anunciante)).cloned().collect();
        let ficha = self.ficha_de_anunciante(anunciante).cloned();
        if ordenes.is_empty() {
            return Err(tr!("No hay ordenes del anunciante «{anunciante}».", anunciante = anunciante));
        }
        let emisiones: Vec<EmisionDeComercial> = self
            .registro_de_comerciales
            .emisiones_entre(desde, hasta)?
            .into_iter()
            .filter(|emision| ordenes.iter().any(|orden| orden.se_llama(&emision.orden)))
            .collect();
        let mut resumen = tr!("Certificado de emision de {} del {desde} al {hasta}:\n", ordenes[0].anunciante, desde = desde, hasta = hasta);
        let mut lineas = vec![tr!("Certificado de emision;{};del {desde} al {hasta}", ordenes[0].anunciante, desde = desde, hasta = hasta)];
        match &ficha {
            Some(ficha) => {
                let campo = |valor: &str| valor.replace(';', ",");
                lineas.push(tr!("Razon social;{};CUIT;{}", campo(&ficha.razon_social), campo(&ficha.cuit)));
                lineas.push(format!("Contacto;{};Telefono;{};Correo;{}", campo(&ficha.contacto), campo(&ficha.telefono), campo(&ficha.correo)));
                if !ficha.agencia.is_empty() {
                    lineas.push(format!("Agencia;{}", campo(&ficha.agencia)));
                }
                if !ficha.razon_social.is_empty() || !ficha.cuit.is_empty() {
                    resumen.push_str(&format!("{} {}\n", ficha.razon_social, if ficha.cuit.is_empty() { String::new() } else { format!("CUIT {}", ficha.cuit) }));
                }
            }
            None => resumen.push_str(&tr!("(sin ficha de anunciante: faltan razon social y CUIT)\n")),
        }
        lineas.push(tr!("Orden;Contratadas;Emitidas completas;Cortadas;Cumplimiento").into());
        for orden in &ordenes {
            let contratadas = salidas_contratadas(orden, desde, hasta);
            let de_la_orden: Vec<&EmisionDeComercial> = emisiones.iter().filter(|emision| orden.se_llama(&emision.orden)).collect();
            let completas = de_la_orden.iter().filter(|emision| emision.completa).count();
            let cortadas = de_la_orden.len() - completas;
            let cumplimiento = if contratadas == 0 { 100.0 } else { completas as f64 * 100.0 / contratadas as f64 };
            resumen.push_str(&tr!("- {}: {completas} de {contratadas} contratada(s) completas ({cumplimiento:.0} %), {cortadas} cortada(s)\n", orden.nombre, completas = completas, contratadas = contratadas, cumplimiento = cumplimiento, cortadas = cortadas));
            lineas.push(format!("{};{contratadas};{completas};{cortadas};{cumplimiento:.0} %", orden.nombre));
        }
        lineas.push(String::new());
        lineas.push(tr!("Inicio;Fin;Segundos emitidos;Completo;Aviso;Orden;Tanda").into());
        for emision in &emisiones {
            lineas.push(format!(
                "{};{};{};{};{};{};{}",
                emision.inicio,
                emision.fin,
                format!("{:.1}", emision.segundos).replace('.', ","),
                if emision.completa { "Si" } else { "No" },
                emision.titulo.replace(';', ","),
                emision.orden.replace(';', ","),
                emision.tanda.replace(';', ",")
            ));
        }
        let archivo = escribir_certificado(&format!("{}_{desde}_{hasta}", ordenes[0].anunciante), &lineas)?;
        resumen.push_str(&tr!("{} salida(s) en el registro. Detalle en {}", emisiones.len(), archivo.display()));
        Ok((resumen, archivo))
    }

    // Ventana de ordenes

    /// Abre (o trae) la ventana de las ordenes.
    pub fn abrir_ordenes(&mut self) -> Result<(), String> {
        if self.ventana_de_ordenes.elegida.is_none() && !self.ventana_de_ordenes.es_nueva {
            let primera = self.ordenes.ordenes().first().cloned();
            self.cargar_formulario_de_orden(primera.as_ref());
        }
        self.refrescar_tabla_de_ordenes();
        let ventana = self.ventana_de_ordenes.ventana.clone_strong();
        self.mostrar_sobre_la_principal(ventana.window(), || ventana.show())
    }

    /// Una vez por segundo con la ventana abierta, y al cambiar algo.
    pub(super) fn refrescar_tabla_de_ordenes(&mut self) {
        let ordenes: Vec<OrdenDePublicidad> = self.ordenes.ordenes().to_vec();
        let mut filas = Vec::with_capacity(ordenes.len());
        for orden in &ordenes {
            let (hoy, problema) = self.estado_de_hoy(orden);
            let sin_archivo = orden.avisos.iter().any(|aviso| !aviso.is_file());
            filas.push(FilaDeOrden {
                nombre: orden.nombre.clone().into(),
                anunciante: orden.anunciante.clone().into(),
                rubro: orden.rubro.clone().into(),
                regla: format!(
                    "{} de {} a {}, {}",
                    orden.frecuencia.describir(),
                    formatear_hora(orden.franja.0),
                    formatear_hora(orden.franja.1),
                    crate::dominio::describir_dias(&orden.dias)
                )
                .into(),
                vigencia: format!("{} al {}", orden.desde, orden.hasta).into(),
                hoy: if sin_archivo { tr!("{hoy} (SIN ARCHIVO)", hoy = hoy) } else { hoy }.into(),
                activa: orden.activa,
                problema: problema || sin_archivo,
                seleccionada: self.ventana_de_ordenes.elegida.as_deref().is_some_and(|elegida| orden.se_llama(elegida)),
            });
        }
        let modelo = &self.ventana_de_ordenes.filas;
        if modelo.row_count() == filas.len() {
            for (indice, fila) in filas.into_iter().enumerate() {
                if modelo.row_data(indice).as_ref() != Some(&fila) {
                    modelo.set_row_data(indice, fila);
                }
            }
        } else {
            modelo.set_vec(filas);
        }
        let lineas: Vec<LineaDePauta> =
            self.lineas_de_la_pauta(hoy()).into_iter().map(|(texto, problema)| LineaDePauta { texto: texto.into(), problema }).collect();
        let sin_ubicar = lineas.iter().filter(|linea| linea.problema).count();
        let resumen = if sin_ubicar > 0 { tr!("{sin_ubicar} salida(s) sin ubicar", sin_ubicar = sin_ubicar) } else { String::new() };
        let ventana = &self.ventana_de_ordenes;
        if ventana.ventana.get_resumen_de_hoy() != resumen.as_str() {
            ventana.ventana.set_resumen_de_hoy(resumen.into());
        }
        let iguales = ventana.lineas.row_count() == lineas.len()
            && lineas.iter().enumerate().all(|(indice, linea)| ventana.lineas.row_data(indice).as_ref() == Some(linea));
        if !iguales {
            ventana.lineas.set_vec(lineas);
        }
    }

    pub(super) fn refrescar_ordenes_si_estan_abiertas(&mut self) {
        if self.ventana_de_ordenes.ventana().is_visible() {
            self.refrescar_tabla_de_ordenes();
        }
    }

    pub(super) fn avisar_en_ordenes(&self, aviso: &str, es_error: bool) {
        self.ventana_de_ordenes.ventana.set_aviso(aviso.into());
        self.ventana_de_ordenes.ventana.set_aviso_es_error(es_error);
    }

    fn cargar_formulario_de_orden(&mut self, orden: Option<&OrdenDePublicidad>) {
        let ventana = &self.ventana_de_ordenes.ventana;
        let texto = |valor: Option<String>| slint::SharedString::from(valor.unwrap_or_default());
        ventana.set_nombre(texto(orden.map(|orden| orden.nombre.clone())));
        ventana.set_anunciante(texto(orden.map(|orden| orden.anunciante.clone())));
        ventana.set_rubro(texto(orden.map(|orden| orden.rubro.clone())));
        let primero_de_mes = hoy().first_of_month();
        ventana.set_desde(orden.map_or(primero_de_mes, |orden| orden.desde).to_string().into());
        ventana.set_hasta(orden.map_or(primero_de_mes.last_of_month(), |orden| orden.hasta).to_string().into());
        let dias = orden.map_or([true; 7], |orden| orden.dias);
        ventana.set_lun(dias[0]);
        ventana.set_mar(dias[1]);
        ventana.set_mie(dias[2]);
        ventana.set_jue(dias[3]);
        ventana.set_vie(dias[4]);
        ventana.set_sab(dias[5]);
        ventana.set_dom(dias[6]);
        ventana.set_franja_desde(formatear_hora(orden.map_or(8 * 3_600, |orden| orden.franja.0)).into());
        ventana.set_franja_hasta(formatear_hora(orden.map_or(20 * 3_600, |orden| orden.franja.1)).into());
        let (tipo, cantidad, horarios) = match orden.map(|orden| &orden.frecuencia) {
            Some(Frecuencia::CadaMinutos(minutos)) => (0, *minutos as i32, String::new()),
            Some(Frecuencia::VecesPorDia(veces)) => (1, *veces as i32, String::new()),
            Some(Frecuencia::EnHorarios(horarios)) => {
                (2, 1, horarios.iter().map(|horario| formatear_hora(*horario)).collect::<Vec<_>>().join(", "))
            }
            None => (0, 120, String::new()),
        };
        ventana.set_tipo_de_frecuencia(tipo);
        ventana.set_cantidad(cantidad);
        ventana.set_horarios(horarios.into());
        ventana.set_tandas_permitidas(texto(orden.map(|orden| orden.tandas.join(", "))));
        ventana.set_prioridad(i32::from(orden.map_or(PRIORIDAD_POR_DEFECTO, |orden| orden.prioridad)));
        ventana.set_activa(orden.is_none_or(|orden| orden.activa));
        ventana.set_editando(true);
        ventana.set_es_nueva(orden.is_none());
        self.refrescar_estado_del_anunciante(false);
        self.ventana_de_ordenes.elegida = orden.map(|orden| orden.nombre.clone());
        self.ventana_de_ordenes.es_nueva = orden.is_none();
        self.ventana_de_ordenes.borrador = orden.map(|orden| orden.avisos.clone()).unwrap_or_default();
        self.ventana_de_ordenes.aviso_elegido = None;
        self.refrescar_avisos_de_la_orden();
        self.refrescar_tabla_de_ordenes();
    }

    fn elegir_orden(&mut self, indice: usize) {
        if let Some(orden) = self.ordenes.ordenes().get(indice).cloned() {
            self.cargar_formulario_de_orden(Some(&orden));
            self.avisar_en_ordenes("", false);
        }
    }

    fn alternar_orden_activa(&mut self, indice: usize) {
        let Some(mut orden) = self.ordenes.ordenes().get(indice).cloned() else { return };
        orden.activa = !orden.activa;
        let (nombre, activa) = (orden.nombre.clone(), orden.activa);
        self.ordenes.guardar_orden(orden);
        self.invalidar_pauta();
        if activa {
            self.completar_salidas_atendidas();
        }
        if self.ventana_de_ordenes.elegida.as_deref() == Some(nombre.as_str()) {
            self.ventana_de_ordenes.ventana.set_activa(activa);
        }
        self.refrescar_tabla_de_ordenes();
        self.avisar_en_ordenes(&tr!("Orden «{nombre}» {}", if activa { "activada" } else { "suspendida" }, nombre = nombre), false);
    }

    /// Muestra bajo el anunciante del formulario su razon social y CUIT, o
    /// que no tiene ficha. Con `completar_rubro`, si el rubro esta vacio toma
    /// el de la ficha.
    pub(super) fn refrescar_estado_del_anunciante(&mut self, completar_rubro: bool) {
        let ventana = &self.ventana_de_ordenes.ventana;
        let nombre = ventana.get_anunciante().trim().to_string();
        let (estado, sin_ficha) = match self.ficha_de_anunciante(&nombre) {
            _ if nombre.is_empty() => (String::new(), false),
            None => (tr!("Sin ficha de anunciante: use Ficha... para cargar razon social, CUIT y contacto"), true),
            Some(ficha) => {
                if completar_rubro && ventana.get_rubro().trim().is_empty() && !ficha.rubro.is_empty() {
                    ventana.set_rubro(ficha.rubro.clone().into());
                }
                let mut datos: Vec<String> = [ficha.razon_social.clone(), if ficha.cuit.is_empty() { String::new() } else { format!("CUIT {}", ficha.cuit) }]
                    .into_iter()
                    .filter(|dato| !dato.is_empty())
                    .collect();
                let faltantes = ficha.faltantes();
                if !faltantes.is_empty() {
                    datos.push(format!("falta {}", faltantes.join(", ")));
                }
                if !ficha.activo {
                    datos.insert(0, "INACTIVO".into());
                }
                (datos.join(" · "), !ficha.activo || !faltantes.is_empty())
            }
        };
        ventana.set_estado_del_anunciante(estado.into());
        ventana.set_anunciante_sin_ficha(sin_ficha);
    }

    /// Abre las ordenes con una orden nueva para ese anunciante (su nombre y
    /// su rubro).
    pub(super) fn abrir_orden_nueva_de(&mut self, anunciante: &crate::dominio::Anunciante) -> Result<(), String> {
        self.abrir_ordenes()?;
        self.cargar_formulario_de_orden(None);
        let ventana = &self.ventana_de_ordenes.ventana;
        ventana.set_anunciante(anunciante.nombre.clone().into());
        ventana.set_rubro(anunciante.rubro.clone().into());
        self.refrescar_estado_del_anunciante(false);
        self.avisar_en_ordenes(&tr!("Orden nueva de {}: complete el nombre, la frecuencia y los avisos.", anunciante.nombre), false);
        Ok(())
    }

    fn orden_del_formulario(&self) -> Result<OrdenDePublicidad, String> {
        let ventana = &self.ventana_de_ordenes.ventana;
        let fecha = |texto: slint::SharedString, que: &str| {
            texto.trim().parse::<Date>().map_err(|_| tr!("La fecha {que} debe ser AAAA-MM-DD (por ejemplo {}).", hoy(), que = que))
        };
        let frecuencia = match ventana.get_tipo_de_frecuencia() {
            0 => Frecuencia::CadaMinutos(ventana.get_cantidad().max(0) as u32),
            1 => Frecuencia::VecesPorDia(ventana.get_cantidad().max(0) as u32),
            _ => Frecuencia::EnHorarios(separar(&ventana.get_horarios()).iter().map(|horario| parsear_hora(horario)).collect::<Result<_, _>>()?),
        };
        let orden = OrdenDePublicidad {
            nombre: ventana.get_nombre().trim().to_string(),
            anunciante: ventana.get_anunciante().trim().to_string(),
            rubro: ventana.get_rubro().trim().to_string(),
            desde: fecha(ventana.get_desde(), "inicial")?,
            hasta: fecha(ventana.get_hasta(), "final")?,
            dias: [ventana.get_lun(), ventana.get_mar(), ventana.get_mie(), ventana.get_jue(), ventana.get_vie(), ventana.get_sab(), ventana.get_dom()],
            franja: (parsear_hora(&ventana.get_franja_desde())?, parsear_hora(&ventana.get_franja_hasta())?),
            frecuencia,
            avisos: self.ventana_de_ordenes.borrador.clone(),
            tandas: separar(&ventana.get_tandas_permitidas()),
            prioridad: ventana.get_prioridad().clamp(1, 9) as u8,
            activa: ventana.get_activa(),
        };
        orden.validar()?;
        Ok(orden)
    }

    fn guardar_orden_de_la_ventana(&mut self) {
        let orden = match self.orden_del_formulario() {
            Ok(orden) => orden,
            Err(error) => return self.avisar_en_ordenes(&error, true),
        };
        let anterior = self.ventana_de_ordenes.elegida.clone();
        let renombrada = anterior.as_deref().filter(|anterior| !orden.se_llama(anterior));
        let ocupado = self.ordenes.ordenes().iter().any(|otra| otra.se_llama(&orden.nombre));
        if (self.ventana_de_ordenes.es_nueva || renombrada.is_some()) && ocupado {
            return self.avisar_en_ordenes(&tr!("Ya hay una orden llamada «{}»: elija otro nombre.", orden.nombre), true);
        }
        let nombre = orden.nombre.clone();
        // Al renombrar, la anterior sale antes de cargar la nueva: si no, las
        // dos estarian en la pauta al completar las tandas ya atendidas. Si la
        // nueva no se carga, la anterior vuelve.
        let quitada = renombrada.and_then(|anterior| self.ordenes.quitar_orden(anterior));
        match self.programar_orden(orden) {
            Ok(mensaje) => {
                let guardada = self.ordenes.ordenes().iter().find(|orden| orden.se_llama(&nombre)).cloned();
                self.cargar_formulario_de_orden(guardada.as_ref());
                self.avisar_en_ordenes(&mensaje, false);
            }
            Err(error) => {
                if let Some(anterior) = quitada {
                    self.ordenes.guardar_orden(anterior);
                    self.invalidar_pauta();
                }
                self.avisar_en_ordenes(&error, true);
            }
        }
    }

    fn certificado_desde_la_ventana(&mut self) {
        let Some(orden) = self.ventana_de_ordenes.elegida.as_deref().and_then(|nombre| self.ordenes.ordenes().iter().find(|orden| orden.se_llama(nombre)).cloned()) else {
            return;
        };
        let fecha = hoy();
        match self.certificado_de_emision(&orden.anunciante, fecha.first_of_month(), fecha) {
            Ok((resumen, archivo)) => {
                self.avisar_en_ordenes(&resumen.replace('\n', " "), false);
                if let Err(error) = abrir_con_el_sistema(&archivo) {
                    log::warn!("No se pudo abrir el certificado: {error}");
                }
            }
            Err(error) => self.avisar_en_ordenes(&error, true),
        }
    }

    fn mover_aviso_de_la_orden(&mut self, subir: bool) {
        let ventana = &mut self.ventana_de_ordenes;
        let Some(indice) = ventana.aviso_elegido else { return };
        let destino = if subir { indice.checked_sub(1) } else { Some(indice + 1).filter(|&destino| destino < ventana.borrador.len()) };
        if let Some(destino) = destino {
            ventana.borrador.swap(indice, destino);
            ventana.aviso_elegido = Some(destino);
        }
        self.refrescar_avisos_de_la_orden();
    }

    pub(super) fn refrescar_avisos_de_la_orden(&self) {
        let avisos = self.avisos_para_mostrar(&self.ventana_de_ordenes.borrador, self.ventana_de_ordenes.aviso_elegido);
        self.ventana_de_ordenes.avisos.set_vec(avisos);
        self.ventana_de_ordenes.ventana.set_hay_aviso_elegido(self.ventana_de_ordenes.aviso_elegido.is_some());
    }
}

/// Origen de un aviso encolado por una tanda: su etiqueta ("Tanda A 10:30")
/// y la orden de publicidad, si es de una.
#[derive(Clone, Debug, Default)]
pub(super) struct OrigenDelAviso {
    pub tanda: String,
    pub orden: Option<String>,
}

/// Para cada aviso de una tanda (fijos y de ordenes), su orden.
pub(super) fn avisos_con_su_orden(fijos: &[PathBuf], de_ordenes: &[Asignacion]) -> Vec<(PathBuf, Option<String>)> {
    let mut avisos: Vec<(PathBuf, Option<String>)> = fijos.iter().map(|archivo| (archivo.clone(), None)).collect();
    avisos.extend(de_ordenes.iter().map(|asignacion| (asignacion.aviso.clone(), Some(asignacion.orden.clone()))));
    avisos
}

/// Mapa vacio de origenes (para construir el controlador).
pub(super) fn sin_origenes() -> HashMap<crate::dominio::Pista, std::collections::VecDeque<OrigenDelAviso>> {
    HashMap::new()
}
