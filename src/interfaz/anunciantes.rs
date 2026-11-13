//! Anunciantes: las fichas de quien contrata la publicidad (razon social,
//! CUIT, rubro, contacto, agencia) y la ventana para mantenerlas. Las ordenes
//! nombran al anunciante por su nombre comercial; los nombrados en ordenes
//! que no tienen ficha se muestran para crearla.

use std::collections::BTreeMap;
use std::rc::Rc;

use slint::{CloseRequestResponse, ComponentHandle, ModelRc, VecModel};

use super::{Controlador, con_controlador};
use crate::dominio::{Anunciante, Busqueda, OrdenDePublicidad};
use crate::{AnunciantesDeLaEmisora, FilaDeAnunciante, LineaDePauta};

/// La ventana de anunciantes y lo que se esta editando.
pub(super) struct VentanaDeAnunciantes {
    ventana: AnunciantesDeLaEmisora,
    filas: Rc<VecModel<FilaDeAnunciante>>,
    ordenes: Rc<VecModel<LineaDePauta>>,
    // Nombres de las filas de la tabla, en orden.
    nombres: Vec<String>,
    // Nombre guardado del anunciante en el formulario (None si es nuevo).
    elegido: Option<String>,
    es_nuevo: bool,
}

impl VentanaDeAnunciantes {
    pub(super) fn nueva() -> Result<Self, String> {
        let ventana =
            AnunciantesDeLaEmisora::new().map_err(|error| tr!("No se pudo crear la ventana de anunciantes: {error}", error = error))?;
        let (filas, ordenes) = (Rc::new(VecModel::default()), Rc::new(VecModel::default()));
        ventana.set_anunciantes(ModelRc::from(filas.clone()));
        ventana.set_ordenes_del_anunciante(ModelRc::from(ordenes.clone()));
        cablear(&ventana);
        Ok(Self { ventana, filas, ordenes, nombres: Vec::new(), elegido: None, es_nuevo: false })
    }

    pub(super) fn ventana(&self) -> &slint::Window {
        self.ventana.window()
    }
}

fn cablear(ventana: &AnunciantesDeLaEmisora) {
    ventana.on_filtrar(|_| con_controlador(Controlador::refrescar_tabla_de_anunciantes));
    ventana.on_elegir(|indice| con_controlador(|c| c.elegir_anunciante(indice as usize)));
    ventana.on_nuevo(|| con_controlador(|c| {
        c.cargar_formulario_de_anunciante(None, None);
        c.avisar_en_anunciantes(&tr!("Anunciante nuevo: complete al menos el nombre comercial y pulse Cargar anunciante."), false);
    }));
    let debil = ventana.as_weak();
    ventana.on_quitar(move || {
        let mut nombre = None;
        con_controlador(|c| nombre = c.anunciantes_de_la_ventana.elegido.clone());
        let Some(nombre) = nombre else { return };
        let mut dialogo = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&tr!("Quitar anunciante"))
            .set_description(tr!("¿Quitar la ficha del anunciante «{nombre}»?", nombre = nombre))
            .set_buttons(rfd::MessageButtons::OkCancel);
        if let Some(ventana) = debil.upgrade() {
            dialogo = dialogo.set_parent(&ventana.window().window_handle());
        }
        if dialogo.show() == rfd::MessageDialogResult::Ok {
            con_controlador(|c| match c.quitar_anunciante(&nombre) {
                Ok(mensaje) => {
                    c.cargar_formulario_de_anunciante(None, None);
                    c.anunciantes_de_la_ventana.es_nuevo = false;
                    c.anunciantes_de_la_ventana.ventana.set_editando(false);
                    c.avisar_en_anunciantes(&mensaje, false);
                }
                Err(error) => c.avisar_en_anunciantes(&error, true),
            });
        }
    });
    ventana.on_nueva_orden(|| con_controlador(Controlador::nueva_orden_del_anunciante));
    ventana.on_guardar(|| con_controlador(Controlador::guardar_anunciante_de_la_ventana));
    ventana.on_descartar(|| con_controlador(|c| {
        let nombre = c.anunciantes_de_la_ventana.elegido.clone();
        let guardado = nombre.as_deref().and_then(|nombre| c.ficha_de_anunciante(nombre)).cloned();
        c.cargar_formulario_de_anunciante(guardado.as_ref(), None);
        c.avisar_en_anunciantes(&tr!("Cambios descartados."), false);
    }));
    ventana.on_cerrar(|| con_controlador(|c| {
        let _ = c.anunciantes_de_la_ventana.ventana.hide();
    }));
    ventana.window().on_close_requested(|| CloseRequestResponse::HideWindow);
}

/// Cuantas ordenes nombran a cada anunciante (todas y activas), por nombre
/// simplificado.
fn ordenes_por_anunciante(ordenes: &[OrdenDePublicidad]) -> BTreeMap<String, (String, usize, usize)> {
    let mut cuentas: BTreeMap<String, (String, usize, usize)> = BTreeMap::new();
    for orden in ordenes {
        let clave = crate::dominio::simplificar_nombre(&orden.anunciante);
        let entrada = cuentas.entry(clave).or_insert_with(|| (orden.anunciante.trim().to_string(), 0, 0));
        entrada.1 += 1;
        if orden.activa {
            entrada.2 += 1;
        }
    }
    cuentas
}

impl Controlador {
    /// La ficha del anunciante con ese nombre (sin importar mayusculas ni acentos).
    pub fn ficha_de_anunciante(&self, nombre: &str) -> Option<&Anunciante> {
        self.anunciantes.anunciantes().iter().find(|anunciante| anunciante.se_llama(nombre))
    }

    /// Carga o reemplaza la ficha de un anunciante. Con `anterior` distinto
    /// del nombre nuevo, lo renombra: sus ordenes pasan al nombre nuevo.
    pub fn registrar_anunciante(&mut self, anunciante: Anunciante, anterior: Option<&str>) -> Result<String, String> {
        let anunciante = anunciante.validado()?;
        let renombrado = anterior.filter(|anterior| !anunciante.se_llama(anterior)).map(str::to_string);
        if let Some(anterior) = &renombrado {
            if self.ficha_de_anunciante(anterior).is_none() {
                return Err(tr!("No hay un anunciante llamado «{anterior}».", anterior = anterior));
            }
            if self.ficha_de_anunciante(&anunciante.nombre).is_some() {
                return Err(tr!("Ya hay un anunciante llamado «{}»: elija otro nombre.", anunciante.nombre));
            }
        }
        let reemplaza = renombrado.is_none() && self.ficha_de_anunciante(&anunciante.nombre).is_some();
        let nombre = anunciante.nombre.clone();
        let faltantes: Vec<String> = anunciante.faltantes();
        let faltan = if faltantes.is_empty() { String::new() } else { tr!(" Falta cargar: {}.", faltantes.join(", ")) };
        self.anunciantes.guardar_anunciante(anunciante);
        // Las ordenes toman el nombre tal como quedo en la ficha (y el nuevo, si se renombro).
        let viejo = renombrado.clone().unwrap_or_else(|| nombre.clone());
        let a_corregir: Vec<OrdenDePublicidad> = self
            .ordenes
            .ordenes()
            .iter()
            .filter(|orden| crate::dominio::simplificar_nombre(&orden.anunciante) == crate::dominio::simplificar_nombre(&viejo) && orden.anunciante != nombre)
            .cloned()
            .collect();
        let corregidas = a_corregir.len();
        for mut orden in a_corregir {
            orden.anunciante = nombre.clone();
            self.ordenes.guardar_orden(orden);
        }
        if let Some(anterior) = &renombrado {
            self.anunciantes.quitar_anunciante(anterior);
        }
        if corregidas > 0 {
            self.invalidar_pauta();
            self.refrescar_ordenes_si_estan_abiertas();
        }
        let mut mensaje = match &renombrado {
            Some(anterior) => tr!("Anunciante «{anterior}» renombrado a «{nombre}»", anterior = anterior, nombre = nombre),
            None if reemplaza => tr!("Anunciante «{nombre}» actualizado", nombre = nombre),
            None => tr!("Anunciante «{nombre}» cargado", nombre = nombre),
        };
        if corregidas > 0 {
            mensaje.push_str(&tr!("; {corregidas} orden(es) actualizada(s)", corregidas = corregidas));
        }
        mensaje.push('.');
        mensaje.push_str(&faltan);
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Quita la ficha de un anunciante sin ordenes. Si tiene, hay que
    /// quitarlas o desactivarlo.
    pub fn quitar_anunciante(&mut self, nombre: &str) -> Result<String, String> {
        let ficha = self.ficha_de_anunciante(nombre).cloned().ok_or_else(|| tr!("No hay un anunciante llamado «{nombre}».", nombre = nombre))?;
        let ordenes = self.ordenes.ordenes().iter().filter(|orden| ficha.se_llama(&orden.anunciante)).count();
        if ordenes > 0 {
            return Err(tr!("«{}» tiene {ordenes} orden(es): quitelas antes o desactive el anunciante para conservar su historial.", ficha.nombre, ordenes = ordenes));
        }
        self.anunciantes.quitar_anunciante(&ficha.nombre);
        let mensaje = tr!("Anunciante «{}» quitado.", ficha.nombre);
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Los anunciantes con sus datos y ordenes, y los nombrados en ordenes
    /// que no tienen ficha.
    pub fn describir_anunciantes(&self) -> String {
        let cuentas = ordenes_por_anunciante(self.ordenes.ordenes());
        let mut texto = String::new();
        let fichas = self.anunciantes.anunciantes();
        if fichas.is_empty() {
            texto.push_str("No hay anunciantes con ficha.\n");
        } else {
            texto.push_str(&format!("{} anunciante(s):\n", fichas.len()));
        }
        for (numero, ficha) in fichas.iter().enumerate() {
            let (total, activas) = cuentas.get(&crate::dominio::simplificar_nombre(&ficha.nombre)).map_or((0, 0), |cuenta| (cuenta.1, cuenta.2));
            let dato = |etiqueta: &str, valor: &str| if valor.is_empty() { None } else { Some(format!("{etiqueta} {valor}")) };
            let datos: Vec<String> = [
                dato("razon social", &ficha.razon_social),
                dato("CUIT", &ficha.cuit),
                dato("rubro", &ficha.rubro),
                dato("contacto", &ficha.contacto),
                dato("tel.", &ficha.telefono),
                dato("correo", &ficha.correo),
                dato("direccion", &ficha.direccion),
                dato("agencia", &ficha.agencia),
                dato("notas", &ficha.notas),
            ]
            .into_iter()
            .flatten()
            .collect();
            let faltantes = ficha.faltantes();
            texto.push_str(&tr!("{}. {}{}: {}; {total} orden(es), {activas} activa(s){}\n", numero + 1, ficha.nombre, if ficha.activo { "" } else { " (inactivo)" }, if datos.is_empty() { "sin datos".to_string() } else { datos.join(", ") }, if faltantes.is_empty() { String::new() } else { format!("; falta {}", faltantes.join(", ")) }, total = total, activas = activas));
        }
        let sin_ficha: Vec<String> = cuentas
            .iter()
            .filter(|(_, (nombre, _, _))| self.ficha_de_anunciante(nombre).is_none())
            .map(|(_, (nombre, total, _))| tr!("{nombre} ({total} orden(es))", nombre = nombre, total = total))
            .collect();
        if !sin_ficha.is_empty() {
            texto.push_str(&tr!("Nombrados en ordenes sin ficha: {}.", sin_ficha.join(", ")));
        }
        texto.trim_end().to_string()
    }

    /// Abre (o trae al frente) la ventana. Con un nombre, lo elige: su ficha
    /// o, si no tiene, una nueva con ese nombre.
    pub fn abrir_anunciantes(&mut self, nombre: Option<&str>) -> Result<(), String> {
        match nombre.map(str::trim).filter(|nombre| !nombre.is_empty()) {
            Some(nombre) => {
                let ficha = self.ficha_de_anunciante(nombre).cloned();
                self.cargar_formulario_de_anunciante(ficha.as_ref(), Some(nombre));
                if ficha.is_none() {
                    self.avisar_en_anunciantes(&tr!("«{nombre}» no tiene ficha: complete sus datos y pulse Cargar anunciante.", nombre = nombre), false);
                }
            }
            None if self.anunciantes_de_la_ventana.elegido.is_none() && !self.anunciantes_de_la_ventana.es_nuevo => {
                match self.anunciantes.anunciantes().first().cloned() {
                    Some(ficha) => self.cargar_formulario_de_anunciante(Some(&ficha), None),
                    None => self.cargar_formulario_de_anunciante(None, None),
                }
            }
            None => self.refrescar_tabla_de_anunciantes(),
        }
        let ventana = self.anunciantes_de_la_ventana.ventana.clone_strong();
        self.mostrar_sobre_la_principal(ventana.window(), || ventana.show())
    }

    fn avisar_en_anunciantes(&self, aviso: &str, es_error: bool) {
        self.anunciantes_de_la_ventana.ventana.set_aviso(aviso.into());
        self.anunciantes_de_la_ventana.ventana.set_aviso_es_error(es_error);
    }

    /// Pasa la ficha al formulario o, sin ficha, uno nuevo (con el nombre y
    /// el rubro de sus ordenes, si se indica un nombre).
    fn cargar_formulario_de_anunciante(&mut self, ficha: Option<&Anunciante>, nombre_nuevo: Option<&str>) {
        let rubro_de_sus_ordenes = nombre_nuevo.and_then(|nombre| {
            self.ordenes.ordenes().iter().find(|orden| crate::dominio::simplificar_nombre(&orden.anunciante) == crate::dominio::simplificar_nombre(nombre)).map(|orden| orden.rubro.clone())
        });
        let nuevo = Anunciante { nombre: nombre_nuevo.unwrap_or_default().to_string(), rubro: rubro_de_sus_ordenes.unwrap_or_default(), activo: true, ..Anunciante::default() };
        let datos = ficha.unwrap_or(&nuevo);
        let ventana = &self.anunciantes_de_la_ventana.ventana;
        ventana.set_nombre(datos.nombre.clone().into());
        ventana.set_razon_social(datos.razon_social.clone().into());
        ventana.set_cuit(datos.cuit.clone().into());
        ventana.set_rubro(datos.rubro.clone().into());
        ventana.set_contacto(datos.contacto.clone().into());
        ventana.set_telefono(datos.telefono.clone().into());
        ventana.set_correo(datos.correo.clone().into());
        ventana.set_direccion(datos.direccion.clone().into());
        ventana.set_agencia(datos.agencia.clone().into());
        ventana.set_notas(datos.notas.clone().into());
        ventana.set_activo(datos.activo);
        ventana.set_editando(true);
        ventana.set_es_nuevo(ficha.is_none());
        self.anunciantes_de_la_ventana.elegido = ficha.map(|ficha| ficha.nombre.clone());
        self.anunciantes_de_la_ventana.es_nuevo = ficha.is_none();
        self.refrescar_tabla_de_anunciantes();
    }

    /// La tabla (con el filtro) y las ordenes del anunciante del formulario.
    pub(super) fn refrescar_tabla_de_anunciantes(&mut self) {
        let cuentas = ordenes_por_anunciante(self.ordenes.ordenes());
        let filtro = self.anunciantes_de_la_ventana.ventana.get_filtro();
        let busqueda = Busqueda::nueva(&filtro);
        let coincide = |textos: &[&str]| busqueda.as_ref().is_none_or(|busqueda| busqueda.coincide(textos));
        let cuenta_de = |nombre: &str| cuentas.get(&crate::dominio::simplificar_nombre(nombre)).map_or((0, 0), |cuenta| (cuenta.1, cuenta.2));
        let actual = self.anunciantes_de_la_ventana.ventana.get_nombre().to_string();
        let elegido = self.anunciantes_de_la_ventana.elegido.clone().unwrap_or(actual);
        let es_el_elegido = |nombre: &str| crate::dominio::simplificar_nombre(nombre) == crate::dominio::simplificar_nombre(&elegido);
        let mut filas = Vec::new();
        let mut nombres = Vec::new();
        for ficha in self.anunciantes.anunciantes() {
            let textos = [&ficha.nombre, &ficha.razon_social, &ficha.cuit, &ficha.rubro, &ficha.contacto, &ficha.agencia].map(String::as_str);
            if !coincide(&textos) {
                continue;
            }
            let (total, activas) = cuenta_de(&ficha.nombre);
            let faltantes = ficha.faltantes();
            filas.push(FilaDeAnunciante {
                nombre: ficha.nombre.clone().into(),
                razon_social: ficha.razon_social.clone().into(),
                cuit: ficha.cuit.clone().into(),
                rubro: ficha.rubro.clone().into(),
                contacto: ficha.contacto.clone().into(),
                ordenes: if total == 0 { "ninguna".to_string() } else { tr!("{activas} activa(s) de {total}", activas = activas, total = total) }.into(),
                estado: if faltantes.is_empty() { String::new() } else { format!("falta {}", faltantes.join(", ")) }.into(),
                sin_ficha: false,
                activo: ficha.activo,
                seleccionada: es_el_elegido(&ficha.nombre),
            });
            nombres.push(ficha.nombre.clone());
        }
        for (nombre, total, activas) in cuentas.values() {
            if self.ficha_de_anunciante(nombre).is_some() || !coincide(&[nombre.as_str()]) {
                continue;
            }
            filas.push(FilaDeAnunciante {
                nombre: nombre.clone().into(),
                ordenes: tr!("{activas} activa(s) de {total}", activas = activas, total = total).into(),
                estado: tr!("sin ficha: elijalo y carguela").into(),
                sin_ficha: true,
                activo: true,
                seleccionada: es_el_elegido(nombre),
                ..FilaDeAnunciante::default()
            });
            nombres.push(nombre.clone());
        }
        let sin_ficha = filas.iter().filter(|fila| fila.sin_ficha).count();
        let incompletos = filas.iter().filter(|fila| !fila.sin_ficha && !fila.estado.is_empty()).count();
        let mut resumen = tr!("{} anunciante(s) con ficha", self.anunciantes.anunciantes().len());
        if sin_ficha > 0 {
            resumen.push_str(&tr!("; {sin_ficha} nombrado(s) en ordenes SIN FICHA", sin_ficha = sin_ficha));
        }
        if incompletos > 0 {
            resumen.push_str(&tr!("; {incompletos} con datos de facturacion incompletos", incompletos = incompletos));
        }
        resumen.push('.');
        let ventana = &self.anunciantes_de_la_ventana;
        ventana.ventana.set_resumen(resumen.into());
        ventana.filas.set_vec(filas);
        let lineas: Vec<LineaDePauta> = self
            .ordenes
            .ordenes()
            .iter()
            .filter(|orden| es_el_elegido(&orden.anunciante))
            .map(|orden| LineaDePauta {
                texto: format!("{}{}: {}", orden.nombre, if orden.activa { "" } else { " (inactiva)" }, orden.describir()).into(),
                problema: false,
            })
            .collect();
        ventana.ordenes.set_vec(lineas);
        self.anunciantes_de_la_ventana.nombres = nombres;
    }

    fn elegir_anunciante(&mut self, indice: usize) {
        let Some(nombre) = self.anunciantes_de_la_ventana.nombres.get(indice).cloned() else { return };
        let ficha = self.ficha_de_anunciante(&nombre).cloned();
        self.cargar_formulario_de_anunciante(ficha.as_ref(), Some(&nombre));
        let aviso = match &ficha {
            None => tr!("«{nombre}» esta nombrado en ordenes pero no tiene ficha: complete sus datos y pulse Cargar anunciante.", nombre = nombre),
            Some(ficha) if !ficha.faltantes().is_empty() => tr!("Para facturarle falta: {}.", ficha.faltantes().join(", ")),
            Some(_) => String::new(),
        };
        self.avisar_en_anunciantes(&aviso, false);
    }

    fn anunciante_del_formulario(&self) -> Anunciante {
        let ventana = &self.anunciantes_de_la_ventana.ventana;
        Anunciante {
            nombre: ventana.get_nombre().to_string(),
            razon_social: ventana.get_razon_social().to_string(),
            cuit: ventana.get_cuit().to_string(),
            rubro: ventana.get_rubro().to_string(),
            contacto: ventana.get_contacto().to_string(),
            telefono: ventana.get_telefono().to_string(),
            correo: ventana.get_correo().to_string(),
            direccion: ventana.get_direccion().to_string(),
            agencia: ventana.get_agencia().to_string(),
            notas: ventana.get_notas().to_string(),
            activo: ventana.get_activo(),
        }
    }

    fn guardar_anunciante_de_la_ventana(&mut self) {
        let anunciante = self.anunciante_del_formulario();
        if self.anunciantes_de_la_ventana.es_nuevo && self.ficha_de_anunciante(&anunciante.nombre).is_some() {
            return self.avisar_en_anunciantes(&tr!("Ya hay un anunciante llamado «{}»: elija otro nombre.", anunciante.nombre.trim()), true);
        }
        let anterior = self.anunciantes_de_la_ventana.elegido.clone();
        match self.registrar_anunciante(anunciante.clone(), anterior.as_deref()) {
            Ok(mensaje) => {
                let guardado = self.ficha_de_anunciante(anunciante.nombre.trim()).cloned();
                self.cargar_formulario_de_anunciante(guardado.as_ref(), None);
                self.avisar_en_anunciantes(&mensaje, false);
            }
            Err(error) => self.avisar_en_anunciantes(&error, true),
        }
    }

    /// Abre las ordenes con una orden nueva para el anunciante elegido.
    fn nueva_orden_del_anunciante(&mut self) {
        let Some(ficha) = self.anunciantes_de_la_ventana.elegido.as_deref().and_then(|nombre| self.ficha_de_anunciante(nombre)).cloned() else {
            return;
        };
        if !ficha.activo {
            return self.avisar_en_anunciantes(&tr!("«{}» esta inactivo: activelo para cargarle ordenes.", ficha.nombre), true);
        }
        if let Err(error) = self.abrir_orden_nueva_de(&ficha) {
            self.avisar_en_anunciantes(&error, true);
        }
    }
}
