//! Tandas programadas: el planificador que las encola a su hora, en el corte
//! natural entre temas y como bloque fijo, y su manejo desde MCP.

use std::cmp::Reverse;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use jiff::SignedDuration;
use jiff::civil::{Date, DateTime};

use super::ordenes::{OrigenDelAviso, avisos_con_su_orden};
use super::{Controlador, archivo_existe, es_marcador, formatear_minutos, suena_sin_mezcla};
use crate::dominio::{
    Busqueda, DIAS_DE_LA_SEMANA, EntradaDeTanda, TipoDeElemento, MarcadorDeBloque, ModoDeEntrada, Pista, TandaProgramada, bloque_de,
    decidir_entrada, nombre_de_archivo,
};
use crate::persistencia::EmisionDeComercial;

/// Cuantas salidas al aire se detallan como maximo al consultar el registro.
const EMISIONES_A_DETALLAR: usize = 200;
// Desvio que todavia se informa como "a horario".
const DESVIO_A_HORARIO: f64 = 30.0;

/// Corte previsto en la emision: en cuantos segundos termina lo que suena
/// (o el bloque fijo en que esta), cuando terminaria el tema que le sigue y
/// en que posicion de la lista se encola una tanda para que entre ahi.
struct CorteDeEmision {
    segundos: f64,
    siguiente: Option<f64>,
    posicion: usize,
    // Se puede bajar lo que suena para que entre una tanda a hora exacta: es
    // un tema (no un aviso) y la tanda va justo despues (no dentro de un
    // bloque fijo ni detras de otra tanda).
    cortable: bool,
}

/// Tanda a hora exacta ya encolada que espera su hora para bajar lo que
/// suena con el fader.
pub(super) struct CorteDeTanda {
    nombre: String,
    hora: String,
    primer_aviso: Pista,
    cortar_en: Instant,
}

fn ahora() -> DateTime {
    jiff::Zoned::now().datetime()
}

/// "10:30" de una salida.
fn hora_de(salida: DateTime) -> String {
    format!("{:02}:{:02}", salida.hour(), salida.minute())
}

/// "vie 06/11/2026 10:30": con el año, en el formato elegido (columna
/// Proxima salida de la pauta).
pub(super) fn fecha_con_anio(salida: DateTime) -> String {
    let dia = DIAS_DE_LA_SEMANA[salida.date().weekday().to_monday_zero_offset() as usize];
    format!("{dia} {} {}", salida.strftime(crate::formato_de_fecha::actual().completa()), hora_de(salida))
}

/// "vie 06/11 10:30" (dia y mes en el orden elegido).
pub(super) fn fecha_corta(salida: DateTime) -> String {
    let dia = DIAS_DE_LA_SEMANA[salida.date().weekday().to_monday_zero_offset() as usize];
    format!("{dia} {} {}", salida.strftime(crate::formato_de_fecha::actual().dia_y_mes()), hora_de(salida))
}

/// Un aviso de una tanda y la orden de publicidad de la que es (si es de una).
type AvisoConSuOrden = (PathBuf, Option<String>);

/// "entra a las 10:29:12 (48 s antes)".
fn describir_entrada(segundos: f64, desvio: f64) -> String {
    tr!("entra a las {} ({})", hora_de_reloj(segundos), describir_desvio(desvio))
}

/// Hora del reloj dentro de tantos segundos: "10:29:12".
fn hora_de_reloj(segundos: f64) -> String {
    jiff::Zoned::now()
        .checked_add(SignedDuration::try_from_secs_f64(segundos.max(0.0)).unwrap_or_default())
        .map_or_else(|_| "?".into(), |momento| momento.strftime("%H:%M:%S").to_string())
}

/// Cuenta regresiva: "4:12" o "1:05:30".
pub(super) fn formatear_cuenta(segundos: f64) -> String {
    let segundos = segundos.max(0.0).ceil() as u64;
    if segundos >= 3_600 {
        format!("{}:{:02}:{:02}", segundos / 3_600, segundos / 60 % 60, segundos % 60)
    } else {
        format!("{}:{:02}", segundos / 60, segundos % 60)
    }
}

/// "1 min 20 s antes", "a horario", "45 s despues".
fn describir_desvio(desvio: f64) -> String {
    if desvio.abs() < DESVIO_A_HORARIO {
        return tr!("a horario").into();
    }
    let segundos = desvio.abs().round() as u64;
    let cuanto = if segundos >= 60 { tr!("{} min {} s", segundos / 60, segundos % 60) } else { format!("{segundos} s") };
    format!("{cuanto} {}", if desvio < 0.0 { "antes" } else { "despues" })
}

impl Controlador {
    /// Una vez por segundo: atiende la proxima salida de cada tanda activa.
    /// Encola a lo sumo una por vez; la siguiente que coincida se pone
    /// detras en el proximo ciclo.
    pub(super) fn ciclo_de_tandas(&mut self) {
        let ahora = ahora();
        let mut proximas: Vec<(DateTime, Reverse<u8>, String)> = Vec::new();
        let mut perdidas: Vec<(String, Vec<DateTime>)> = Vec::new();
        for tanda in self.tandas.tandas().iter().filter(|tanda| tanda.activa) {
            let pendientes = tanda.pendientes(ahora);
            if !pendientes.perdidas.is_empty() {
                perdidas.push((tanda.nombre.clone(), pendientes.perdidas.clone()));
            }
            if let Some(proxima) = pendientes.proxima {
                proximas.push((proxima, Reverse(tanda.prioridad), tanda.nombre.clone()));
            }
        }
        for (nombre, salidas) in perdidas {
            let (cantidad, ultima) = (salidas.len(), *salidas.last().expect("hay al menos una"));
            self.tandas.anotar_salida(&nombre, ultima);
            for salida in salidas {
                self.reponer_salida_perdida(&nombre, salida);
            }
            let cuales = if cantidad == 1 { tr!("la de las {}", hora_de(ultima)) } else { tr!("{cantidad}, la ultima a las {}", hora_de(ultima), cantidad = cantidad) };
            self.avisar_tanda_perdida(&tr!("Tanda «{nombre}»: no salio {cuales} (no sonaba nada en su horario)", nombre = nombre, cuales = cuales));
        }
        // Mientras una tanda espera su corte a hora exacta, no se encola otra.
        if proximas.is_empty() || self.corte_de_tanda.is_some() {
            return;
        }
        // Por hora y, con la misma hora, la de mayor prioridad primero.
        proximas.sort();
        let corte = self.corte_de_emision();
        for (salida, _, nombre) in proximas {
            let Some(tanda) = self.tandas.tandas().iter().find(|tanda| tanda.se_llama(&nombre)).cloned() else {
                continue;
            };
            let pautada = salida.duration_since(ahora).as_secs_f64();
            let entrada = decidir_entrada(
                pautada,
                f64::from(tanda.antes),
                f64::from(tanda.despues),
                tanda.modo,
                corte.as_ref().map(|corte| corte.segundos),
                corte.as_ref().and_then(|corte| corte.siguiente),
            );
            match (entrada, &corte) {
                (EntradaDeTanda::Encolar { desvio }, Some(corte)) => {
                    let (segundos, posicion) = (corte.segundos, corte.posicion);
                    self.tandas.anotar_salida(&tanda.nombre, salida);
                    let entra = describir_entrada(segundos, desvio);
                    self.encolar_tanda(&tanda, Some(salida), &hora_de(salida), posicion, &entra);
                    return;
                }
                (EntradaDeTanda::Cortar { pautada }, Some(corte)) => {
                    self.tandas.anotar_salida(&tanda.nombre, salida);
                    if !corte.cortable {
                        // No se corta un aviso ni un bloque fijo: entra al final, tarde.
                        let entra = tr!("{}; no se corta lo que suena (aviso o bloque fijo)", describir_entrada(corte.segundos, corte.segundos - pautada));
                        self.encolar_tanda(&tanda, Some(salida), &hora_de(salida), corte.posicion, &entra);
                        return;
                    }
                    let fader = f64::from(self.fader());
                    let cortar_en = Instant::now() + Duration::from_secs_f64((pautada - fader).max(0.0));
                    let entra = tr!("corta lo que suena a las {} con fundido de {} s", hora_de_reloj(pautada), self.fader());
                    if let Some(primer_aviso) = self.encolar_tanda(&tanda, Some(salida), &hora_de(salida), corte.posicion, &entra) {
                        self.corte_de_tanda = Some(CorteDeTanda { nombre: tanda.nombre.clone(), hora: hora_de(salida), primer_aviso, cortar_en });
                    }
                    return;
                }
                (EntradaDeTanda::Perdida, _) => {
                    self.tandas.anotar_salida(&tanda.nombre, salida);
                    self.reponer_salida_perdida(&tanda.nombre, salida);
                    self.avisar_tanda_perdida(&tr!("Tanda «{}»: no salio la de las {} (no sonaba nada en su horario)", tanda.nombre, hora_de(salida)));
                }
                _ => {}
            }
        }
    }

    fn avisar_tanda_perdida(&self, aviso: &str) {
        log::warn!("{aviso}");
        self.mostrar_estado(aviso);
    }

    /// Donde terminaria lo que suena si despues viniera una tanda. Sin
    /// reproduccion en curso (detenida, en pausa o en plena mezcla), None.
    fn corte_de_emision(&self) -> Option<CorteDeEmision> {
        let gestor = &self.gestor;
        if !gestor.esta_reproduciendo() || gestor.esta_pausado() || !gestor.hay_pista_cargada() || gestor.hay_transicion_en_curso() {
            return None;
        }
        let actual = gestor.pista_actual()?;
        let indice = self.lista.indice_actual().filter(|&indice| self.lista.pista_en(indice) == Some(actual))?;
        // Antes de una tanda no hay mezcla: el tema suena hasta su final.
        let mut segundos = (gestor.duracion_total()? - gestor.tiempo_transcurrido()).max(0.0);
        let marcadores = self.marcadores_de_la_lista();
        let mut posicion = indice + 1;
        // Dentro de un bloque fijo, la tanda va despues del bloque, para no partirlo.
        if let Some((_, hasta)) = bloque_de(&marcadores, indice).filter(|&(_, hasta)| hasta >= posicion) {
            segundos += self.duracion_entre(posicion, hasta);
            posicion = hasta + 1;
        }
        // Si ya viene una tanda (un bloque fijo solo de temas sin mezcla), esta va detras.
        while marcadores.get(posicion) == Some(&Some(MarcadorDeBloque::Inicio)) {
            let Some((desde, hasta)) = bloque_de(&marcadores, posicion) else { break };
            let es_tanda = hasta > desde + 1
                && (desde + 1..hasta).all(|indice| {
                    self.lista
                        .pista_en(indice)
                        .is_some_and(|pista| es_marcador(pista.archivo()) || suena_sin_mezcla(&self.repositorio_de_canciones, pista))
                });
            if !es_tanda {
                break;
            }
            segundos += self.duracion_entre(posicion, hasta);
            posicion = hasta + 1;
        }
        let siguiente = (posicion..self.lista.tamano())
            .filter_map(|indice| self.lista.pista_en(indice))
            .find(|pista| archivo_existe(pista))
            .map(|pista| segundos + self.duracion_de(pista).unwrap_or(0.0));
        let cortable = posicion == indice + 1 && !suena_sin_mezcla(&self.repositorio_de_canciones, actual);
        Some(CorteDeEmision { segundos, siguiente, posicion, cortable })
    }

    /// Lo que dura lo que se puede reproducir entre esas posiciones (inclusive).
    fn duracion_entre(&self, desde: usize, hasta: usize) -> f64 {
        (desde..=hasta)
            .filter_map(|indice| self.lista.pista_en(indice))
            .filter(|pista| archivo_existe(pista))
            .map(|pista| self.duracion_de(pista).unwrap_or(0.0))
            .sum()
    }

    /// Inserta en esa posicion, como bloque fijo, los avisos fijos de la tanda
    /// y (para una salida de su horario) los de las ordenes de publicidad que
    /// le tocan en la pauta del dia. Devuelve el primer aviso. `hora` es la
    /// pautada ("10:30") y `entra` cuenta como entra, para la barra de
    /// estado. La seleccion del operador no cambia.
    fn encolar_tanda(&mut self, tanda: &TandaProgramada, salida: Option<DateTime>, hora: &str, posicion: usize, entra: &str) -> Option<Pista> {
        let de_ordenes = salida.map(|salida| self.avisos_de_ordenes(&tanda.nombre, salida)).unwrap_or_default();
        let avisos = avisos_con_su_orden(&tanda.archivos, &de_ordenes);
        let total = avisos.len();
        let (existentes, faltan): (Vec<AvisoConSuOrden>, Vec<AvisoConSuOrden>) =
            avisos.into_iter().partition(|(archivo, _)| archivo.is_file());
        if existentes.is_empty() {
            if total == 0 {
                log::info!("Tanda {} de las {hora}: sin avisos en esta salida", tanda.nombre);
            } else {
                self.avisar_tanda_perdida(&tr!("Tanda «{}» de las {hora}: no salio, no se encontro ninguno de sus {total} archivo(s)", tanda.nombre, hora = hora, total = total));
            }
            // Lo de las ordenes que no salio se repone.
            for (archivo, orden) in faltan {
                if let Some(orden) = orden {
                    log::warn!("Falta el aviso {} de la orden {orden}", archivo.display());
                }
            }
            return None;
        }
        let (seleccion, ancla) = (self.seleccion.clone(), self.ancla_de_seleccion);
        let (archivos, ordenes): (Vec<PathBuf>, Vec<Option<String>>) = existentes.into_iter().unzip();
        let nuevas = self.insertar_pistas(archivos, Some(posicion), true);
        let primer_aviso = nuevas.first().cloned()?;
        let cantidad = nuevas.len();
        if let Err(error) = self.fijar_bloque(posicion, posicion + cantidad - 1) {
            log::warn!("La tanda {} no quedo en un bloque fijo: {error}", tanda.nombre);
        }
        // Las filas desde la posicion se corrieron: la tanda y sus dos marcadores.
        let corrida = |indice: usize| if indice >= posicion { indice + cantidad + 2 } else { indice };
        self.seleccion = seleccion.into_iter().map(corrida).collect();
        self.ancla_de_seleccion = ancla.map(corrida);
        self.refrescar_filas();
        let etiqueta = format!("{} {hora}", tanda.nombre);
        for (pista, orden) in nuevas.into_iter().zip(ordenes) {
            // En cola por archivo: el mismo aviso puede estar en varias tandas
            // encoladas, y salen en el orden de la lista.
            self.pistas_de_tanda.entry(pista).or_default().push_back(OrigenDelAviso { tanda: etiqueta.clone(), orden });
        }
        let mut estado = tr!("Tanda «{}» de las {hora} encolada: {cantidad} aviso(s) a partir de la posicion {}; {entra}", tanda.nombre, posicion + 2, hora = hora, cantidad = cantidad, entra = entra);
        if !de_ordenes.is_empty() {
            estado.push_str(&tr!(" ({} de ordenes)", de_ordenes.len()));
        }
        if !faltan.is_empty() {
            estado.push_str(&tr!("; faltan {} archivo(s)", faltan.len()));
            log::warn!("Tanda {}: faltan {:?}", tanda.nombre, faltan);
        }
        log::info!("{estado}");
        self.mostrar_estado(&estado);
        Some(primer_aviso)
    }

    /// En cada ciclo: si una tanda a hora exacta llego a su momento, baja lo
    /// que suena con el fader para que entre. Si ya entro sola (el tema
    /// termino antes) o se la quito de la lista, no hace nada.
    pub(super) fn atender_corte_de_tanda(&mut self) {
        if self.corte_de_tanda.as_ref().is_none_or(|corte| Instant::now() < corte.cortar_en) {
            return;
        }
        let Some(corte) = self.corte_de_tanda.take() else { return };
        if self.gestor.pista_actual() == Some(&corte.primer_aviso) || !self.gestor.esta_reproduciendo() || self.gestor.esta_pausado() {
            return;
        }
        // El mismo aviso puede estar en tandas anteriores de la lista (las
        // pistas del mismo archivo son iguales): se busca despues de la que suena.
        let desde = self.lista.indice_actual().map_or(0, |actual| actual + 1);
        let Some(indice) = (desde..self.lista.tamano()).find(|&indice| self.lista.pista_en(indice) == Some(&corte.primer_aviso)) else {
            log::warn!("La tanda {} de las {} ya no esta en la lista: no se corta", corte.nombre, corte.hora);
            return;
        };
        log::info!("Tanda {} de las {}: corte a hora exacta", corte.nombre, corte.hora);
        self.pasar_a(indice);
        self.mostrar_estado(&tr!("Tanda «{}» de las {}: fundido de {} s y entra a hora exacta", corte.nombre, corte.hora, self.fader()));
    }

    /// Encola ya una tanda programada, fuera de su horario, en el proximo
    /// corte (o, sin reproduccion, en el punto de insercion). No cambia su
    /// programacion.
    pub fn encolar_tanda_ahora(&mut self, nombre: &str) -> Result<String, String> {
        let tanda = self.tandas.tandas().iter().find(|tanda| tanda.se_llama(nombre)).cloned().ok_or_else(|| tr!("No hay una tanda llamada «{nombre}».", nombre = nombre))?;
        let (posicion, entra) = match self.corte_de_emision() {
            Some(corte) => (corte.posicion, describir_entrada(corte.segundos, 0.0).replace(&tr!(" (a horario)"), "")),
            None => (self.punto_de_insercion(), tr!("entra al reproducirla")),
        };
        let hora = tr!("{} (a mano)", hora_de(ahora()));
        self.encolar_tanda(&tanda, None, &hora, posicion, &entra).ok_or_else(|| tr!("La tanda «{}» no tiene avisos fijos que existan.", tanda.nombre))?;
        Ok(tr!("Tanda «{}» encolada a mano: {entra}", tanda.nombre, entra = entra))
    }

    /// La proxima tanda para la ventana principal: "Proxima tanda: «A» 10:30
    /// (en 4:12)"; vacio si no hay ninguna activa.
    pub(super) fn texto_de_la_proxima_tanda(&self) -> String {
        if let Some(corte) = &self.corte_de_tanda {
            let falta = corte.cortar_en.saturating_duration_since(Instant::now()).as_secs_f64();
            return tr!("Tanda «{}» {}: corta en {}", corte.nombre, corte.hora, formatear_cuenta(falta));
        }
        let ahora = ahora();
        let proxima = self
            .tandas
            .tandas()
            .iter()
            .filter(|tanda| tanda.activa)
            .filter_map(|tanda| tanda.pendientes(ahora).proxima.map(|salida| (salida, Reverse(tanda.prioridad), tanda.nombre.as_str(), tanda.modo)))
            .min();
        match proxima {
            Some((salida, _, nombre, modo)) => {
                let falta = salida.duration_since(ahora).as_secs_f64();
                let exacta = if modo == ModoDeEntrada::Exacta { " exacta" } else { "" };
                if falta > 86_400.0 {
                    tr!("Proxima tanda: «{nombre}» {}{exacta}", fecha_corta(salida), nombre = nombre, exacta = exacta)
                } else {
                    tr!("Proxima tanda: «{nombre}» {}{exacta} (en {})", hora_de(salida), formatear_cuenta(falta), nombre = nombre, exacta = exacta)
                }
            }
            None => String::new(),
        }
    }

    /// Por que un archivo no puede ir en una tanda: su tipo debe ser
    /// Comercial, Publicidad, Anuncio, Podcast o Seco.
    pub(super) fn motivo_para_no_ir_en_tanda(&self, archivo: &std::path::Path) -> Option<String> {
        let nombre = nombre_de_archivo(archivo);
        match self.repositorio_de_canciones.tipo_de(archivo) {
            None => Some(tr!("{nombre}: no tiene tipo; asignele Comercial, Publicidad o Anuncio", nombre = nombre)),
            Some(TipoDeElemento::Efecto) => Some(tr!("{nombre}: es un Efecto (solo va en la botonera)", nombre = nombre)),
            Some(tipo) if !tipo.va_en_tanda() => {
                Some(tr!("{nombre}: es de tipo {}; en una tanda van Comercial, Publicidad, Anuncio, Podcast o Seco", tipo.nombre_visible(), nombre = nombre))
            }
            Some(_) => None,
        }
    }

    /// Programa una tanda nueva o reemplaza la del mismo nombre. Las salidas
    /// que ya pasaron su tolerancia no se emiten; la que esta en tolerancia
    /// ahora, si.
    pub fn programar_tanda(&mut self, mut tanda: TandaProgramada) -> Result<String, String> {
        if tanda.nombre.trim().is_empty() {
            return Err(tr!("Indique el nombre de la tanda.").into());
        }
        let problemas: Vec<String> = tanda.archivos.iter().filter_map(|archivo| self.motivo_para_no_ir_en_tanda(archivo)).collect();
        if !problemas.is_empty() {
            return Err(tr!("La tanda no se programo. {}.", problemas.join("; ")));
        }
        let ahora = ahora();
        let desde = ahora.checked_sub(SignedDuration::from_secs(i64::from(tanda.despues) + 1)).unwrap_or(ahora);
        // La ultima salida que trae (al renombrarla desde la pauta) o la de la guardada con ese nombre.
        let anterior = tanda
            .ultima_salida
            .or_else(|| self.tandas.tandas().iter().find(|guardada| guardada.se_llama(&tanda.nombre)).and_then(|guardada| guardada.ultima_salida));
        tanda.ultima_salida = Some(anterior.map_or(desde, |anterior| anterior.max(desde)));
        let reemplaza = anterior.is_some() || self.tandas.tandas().iter().any(|guardada| guardada.se_llama(&tanda.nombre));
        let resumen = self.resumen_de_tanda(&tanda, ahora);
        self.tandas.guardar_tanda(tanda);
        self.invalidar_pauta();
        let mensaje = if reemplaza { tr!("Tanda reprogramada: {resumen}", resumen = resumen) } else { tr!("Tanda programada: {resumen}", resumen = resumen) };
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    pub fn quitar_tanda(&mut self, nombre: &str) -> Result<String, String> {
        match self.tandas.quitar_tanda(nombre) {
            Some(tanda) => {
                self.invalidar_pauta();
                let mensaje = tr!("Tanda «{}» quitada de la programacion (lo ya encolado queda en la lista)", tanda.nombre);
                self.mostrar_estado(&mensaje);
                Ok(mensaje)
            }
            None => Err(tr!("No hay una tanda llamada «{nombre}».", nombre = nombre)),
        }
    }

    /// Una tanda en una linea: regla, tolerancia, archivos y proxima salida.
    fn resumen_de_tanda(&self, tanda: &TandaProgramada, ahora: DateTime) -> String {
        let duracion: f64 = tanda
            .archivos
            .iter()
            .filter_map(|archivo| self.repositorio_de_canciones.buscar(archivo).and_then(|ficha| ficha.duracion))
            .sum();
        let faltan = tanda.archivos.iter().filter(|archivo| !archivo.is_file()).count();
        let mut texto = tr!("«{}»{}: {}; {}, entra hasta {} antes o {} despues; prioridad {} | {} aviso(s), {}", tanda.nombre, if tanda.activa { "" } else { " (inactiva)" }, tanda.recurrencia.describir(), tanda.modo.describir(), describir_minutos(tanda.antes), describir_minutos(tanda.despues), tanda.prioridad, tanda.archivos.len(), formatear_minutos(duracion));
        if faltan > 0 {
            texto.push_str(&tr!(" ({faltan} SIN ARCHIVO)", faltan = faltan));
        }
        if tanda.activa {
            match tanda.pendientes(ahora).proxima {
                Some(proxima) => texto.push_str(&tr!(" | proxima: {}", fecha_corta(proxima))),
                None => texto.push_str(&tr!(" | sin proximas salidas")),
            }
        }
        // Al programarla se anota la hora como punto de partida: solo se
        // muestra si es un horario de la tanda.
        if let Some(ultima) = tanda.ultima_salida.filter(|ultima| tanda.recurrencia.horarios_del_dia(ultima.date()).contains(ultima)) {
            texto.push_str(&tr!(" | ultima atendida: {}", fecha_corta(ultima)));
        }
        texto
    }

    /// Las tandas programadas, con sus archivos.
    pub fn describir_tandas(&self) -> String {
        let tandas = self.tandas.tandas();
        if tandas.is_empty() {
            return tr!("No hay tandas programadas.").into();
        }
        let ahora = ahora();
        let mut texto = tr!("{} tanda(s) programada(s):\n", tandas.len());
        for (numero, tanda) in tandas.iter().enumerate() {
            texto.push_str(&format!("{}. {}\n", numero + 1, self.resumen_de_tanda(tanda, ahora)));
            for archivo in &tanda.archivos {
                let marca = if archivo.is_file() { String::new() } else { tr!(" (SIN ARCHIVO)") };
                texto.push_str(&format!("   - {}{marca}\n", archivo.display()));
            }
        }
        texto
    }

    /// Resumen y detalle de las salidas al aire de la propaganda entre esas
    /// fechas, opcionalmente filtradas por titulo, artista, tanda o archivo.
    pub fn consultar_emisiones(&self, desde: Date, hasta: Date, texto: Option<&str>) -> Result<String, String> {
        let busqueda = texto.and_then(Busqueda::nueva);
        let coincide = |emision: &EmisionDeComercial| {
            let archivo = nombre_de_archivo(&emision.archivo);
            busqueda.as_ref().is_none_or(|busqueda| busqueda.coincide(&[&emision.titulo, &emision.artista, &emision.tanda, &archivo]))
        };
        let emisiones: Vec<EmisionDeComercial> = self.registro_de_comerciales.emisiones_entre(desde, hasta)?.into_iter().filter(coincide).collect();
        if emisiones.is_empty() {
            return Ok(tr!("No hay salidas al aire registradas del {desde} al {hasta}.", desde = desde, hasta = hasta));
        }
        let completas = emisiones.iter().filter(|emision| emision.completa).count();
        let segundos: f64 = emisiones.iter().map(|emision| emision.segundos).sum();
        let mut salida = tr!("{} salida(s) del {desde} al {hasta}: {completas} completa(s), {} cortada(s); {} al aire.\n", emisiones.len(), emisiones.len() - completas, formatear_minutos(segundos), desde = desde, hasta = hasta, completas = completas);
        for emision in emisiones.iter().rev().take(EMISIONES_A_DETALLAR).rev() {
            salida.push_str(&format!(
                "{} | {:.1} s | {} | {}{} | {}{}{}\n",
                emision.inicio,
                emision.segundos,
                if emision.completa { "completa" } else { "CORTADA" },
                emision.titulo,
                if emision.artista.is_empty() { String::new() } else { format!(" - {}", emision.artista) },
                if emision.tipo.is_empty() { &emision.genero } else { &emision.tipo },
                if emision.tanda.is_empty() { String::new() } else { tr!(" | tanda {}", emision.tanda) },
                if emision.orden.is_empty() { String::new() } else { tr!(" | orden {}", emision.orden) }
            ));
        }
        if emisiones.len() > EMISIONES_A_DETALLAR {
            salida.push_str(&tr!("(se detallan las ultimas {EMISIONES_A_DETALLAR})\n", EMISIONES_A_DETALLAR = EMISIONES_A_DETALLAR));
        }
        Ok(salida)
    }
}

/// "3 min" o "90 s".
pub(super) fn describir_minutos(segundos: u32) -> String {
    if segundos.is_multiple_of(60) { format!("{} min", segundos / 60) } else { format!("{segundos} s") }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_desvio_se_cuenta_en_palabras() {
        assert_eq!(describir_desvio(-80.0), "1 min 20 s antes");
        assert_eq!(describir_desvio(12.0), "a horario");
        assert_eq!(describir_desvio(45.0), "45 s despues");
        assert_eq!(describir_minutos(180), "3 min");
        assert_eq!(describir_minutos(90), "90 s");
        assert_eq!(fecha_corta(jiff::civil::date(2026, 11, 6).at(10, 30, 0, 0)), "vie 06/11 10:30");
        assert_eq!(fecha_con_anio(jiff::civil::date(2026, 11, 6).at(10, 30, 0, 0)), "vie 06/11/2026 10:30");
        assert_eq!(formatear_cuenta(251.2), "4:12");
        assert_eq!(formatear_cuenta(3_930.0), "1:05:30");
    }
}
