//! Tandas programadas: que se emite y cuando. Todo se calcula a partir de la
//! hora que se recibe, sin consultar el reloj, para poder probarlo.
//!
//! Una tanda tiene una regla de recurrencia (por ejemplo, "a las 10:30 y cada
//! 30 minutos hasta las 20:00, de lunes a viernes") y una tolerancia: puede
//! entrar unos minutos antes o despues de la hora pautada, en el corte natural
//! entre dos temas, para no cortar ninguno.

use std::path::PathBuf;

use jiff::civil::{Date, DateTime, Time};
use jiff::{SignedDuration, Span};

/// Abreviaturas de los dias, de lunes a domingo.
pub const DIAS_DE_LA_SEMANA: [&str; 7] = ["lun", "mar", "mie", "jue", "vie", "sab", "dom"];
/// Cuanto antes de la hora pautada puede entrar una tanda, por defecto.
pub const TOLERANCIA_PREVIA_POR_DEFECTO: u32 = 180;
/// Cuanto despues de la hora pautada puede entrar una tanda, por defecto.
pub const TOLERANCIA_POSTERIOR_POR_DEFECTO: u32 = 120;
/// Tolerancia maxima, antes o despues (una hora).
pub const TOLERANCIA_MAXIMA: u32 = 3_600;
/// Prioridad de una tanda: de 1 a 9; con la misma hora sale primero la mayor.
pub const PRIORIDAD_MINIMA: u8 = 1;
pub const PRIORIDAD_MAXIMA: u8 = 9;
pub const PRIORIDAD_POR_DEFECTO: u8 = 5;
/// Con hora exacta, cuantos segundos antes del corte se encola la tanda.
pub const ANTICIPACION_DEL_CORTE: f64 = 90.0;
const SEGUNDOS_POR_DIA: u32 = 86_400;
// Hasta cuantos dias hacia adelante se busca la proxima salida, y hacia
// atras las perdidas (una semana y un dia alcanzan para cualquier regla).
const DIAS_A_EXPLORAR: i64 = 8;

/// Cuando se repite una tanda: un primer horario y, opcionalmente, un
/// intervalo y un horario limite, en los dias de la semana elegidos. Los
/// horarios son segundos desde la medianoche, en la hora local.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recurrencia {
    inicio: u32,
    cada: Option<u32>,
    hasta: Option<u32>,
    // De lunes a domingo.
    dias: [bool; 7],
}

impl Recurrencia {
    /// Valida la regla: el intervalo entre 1 minuto y 24 horas, el limite no
    /// antes del inicio (y solo con intervalo) y al menos un dia.
    pub fn nueva(inicio: u32, cada_minutos: Option<u32>, hasta: Option<u32>, dias: [bool; 7]) -> Result<Self, String> {
        if inicio >= SEGUNDOS_POR_DIA || hasta.is_some_and(|hasta| hasta >= SEGUNDOS_POR_DIA) {
            return Err(tr!("Los horarios van de 00:00 a 23:59.").into());
        }
        let cada = match cada_minutos {
            Some(minutos) if !(1..=1_440).contains(&minutos) => {
                return Err(tr!("El intervalo va de 1 minuto a 1440 (un dia).").into());
            }
            otro => otro.map(|minutos| minutos * 60),
        };
        if hasta.is_some() && cada.is_none() {
            return Err(tr!("El horario limite solo tiene sentido con un intervalo ('cada').").into());
        }
        if hasta.is_some_and(|hasta| hasta < inicio) {
            return Err(tr!("El horario limite no puede ser anterior al primer horario.").into());
        }
        if !dias.contains(&true) {
            return Err(tr!("Elija al menos un dia de la semana.").into());
        }
        Ok(Self { inicio, cada, hasta, dias })
    }

    pub fn inicio(&self) -> u32 {
        self.inicio
    }

    pub fn cada_minutos(&self) -> Option<u32> {
        self.cada.map(|segundos| segundos / 60)
    }

    pub fn hasta(&self) -> Option<u32> {
        self.hasta
    }

    pub fn dias(&self) -> [bool; 7] {
        self.dias
    }

    /// Horarios de la tanda en esa fecha (ninguno si el dia no esta elegido).
    pub fn horarios_del_dia(&self, fecha: Date) -> Vec<DateTime> {
        if !self.dias[fecha.weekday().to_monday_zero_offset() as usize] {
            return Vec::new();
        }
        let mut horarios = vec![self.inicio];
        if let Some(cada) = self.cada {
            // Sin limite, se repite hasta el final del dia.
            let limite = self.hasta.unwrap_or(SEGUNDOS_POR_DIA - 1);
            let mut horario = self.inicio + cada;
            while horario <= limite {
                horarios.push(horario);
                horario += cada;
            }
        }
        horarios.into_iter().map(|segundos| fecha.to_datetime(hora_del_dia(segundos))).collect()
    }

    /// Salidas posteriores a `desde` y hasta `hasta` inclusive, en orden.
    pub fn entre(&self, desde: DateTime, hasta: DateTime) -> Vec<DateTime> {
        let mut salidas = Vec::new();
        let mut fecha = desde.date();
        while fecha <= hasta.date() {
            salidas.extend(self.horarios_del_dia(fecha).into_iter().filter(|salida| *salida > desde && *salida <= hasta));
            let Ok(siguiente) = fecha.tomorrow() else { break };
            fecha = siguiente;
        }
        salidas
    }

    /// Primera salida posterior a ese momento, buscando hasta una semana.
    pub fn proxima_despues_de(&self, momento: DateTime) -> Option<DateTime> {
        let limite = momento.checked_add(Span::new().days(DIAS_A_EXPLORAR)).ok()?;
        self.entre(momento, limite).into_iter().next()
    }

    /// La regla en palabras: "10:30 y cada 30 min hasta 20:00, lun a vie".
    pub fn describir(&self) -> String {
        let mut texto = formatear_hora(self.inicio);
        if let Some(minutos) = self.cada_minutos() {
            texto.push_str(&tr!(" y cada {minutos} min", minutos = minutos));
            if let Some(hasta) = self.hasta {
                texto.push_str(&format!(" hasta {}", formatear_hora(hasta)));
            }
        }
        format!("{texto}, {}", describir_dias(&self.dias))
    }
}

/// Texto en minusculas y sin acentos, para comparar nombres y dias.
fn simplificar(texto: &str) -> String {
    texto.chars().flat_map(char::to_lowercase).map(super::duplicados::sin_acento).collect()
}

fn hora_del_dia(segundos: u32) -> Time {
    let segundos = segundos.min(SEGUNDOS_POR_DIA - 1);
    Time::constant((segundos / 3_600) as i8, (segundos / 60 % 60) as i8, (segundos % 60) as i8, 0)
}

/// Horario "10:30" (o "10:30:15" si tiene segundos).
pub fn formatear_hora(segundos: u32) -> String {
    let (horas, minutos, resto) = (segundos / 3_600, segundos / 60 % 60, segundos % 60);
    if resto == 0 { format!("{horas:02}:{minutos:02}") } else { format!("{horas:02}:{minutos:02}:{resto:02}") }
}

/// Lee un horario "10:30", "9:05" o "10:30:15" como segundos desde la medianoche.
pub fn parsear_hora(texto: &str) -> Result<u32, String> {
    let error = || tr!("Horario invalido: '{texto}' (use HH:MM, de 00:00 a 23:59).", texto = texto);
    let partes: Vec<&str> = texto.trim().split(':').collect();
    if !(2..=3).contains(&partes.len()) {
        return Err(error());
    }
    let mut valores = [0u32; 3];
    for (valor, parte) in valores.iter_mut().zip(&partes) {
        if parte.is_empty() || parte.len() > 2 || !parte.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(error());
        }
        *valor = parte.parse().map_err(|_| error())?;
    }
    let [horas, minutos, segundos] = valores;
    if horas > 23 || minutos > 59 || segundos > 59 {
        return Err(error());
    }
    Ok(horas * 3_600 + minutos * 60 + segundos)
}

/// Lee los dias de la semana: abreviaturas o nombres ("lun", "martes",
/// "miércoles"...), "todos", "habiles" (lunes a viernes) o "fin de semana".
pub fn parsear_dias(textos: &[String]) -> Result<[bool; 7], String> {
    let mut dias = [false; 7];
    for texto in textos {
        let simplificado: String = simplificar(texto.trim());
        match simplificado.as_str() {
            "todos" | "todos los dias" => dias = [true; 7],
            "habiles" | "dias habiles" | "lun a vie" => dias[..5].fill(true),
            "fin de semana" | "sab y dom" => dias[5..].fill(true),
            otro => {
                let posicion = DIAS_DE_LA_SEMANA
                    .iter()
                    .position(|dia| otro.starts_with(dia))
                    .ok_or_else(|| tr!("Dia desconocido: '{texto}' (use lun, mar, mie, jue, vie, sab, dom, todos o habiles).", texto = texto))?;
                dias[posicion] = true;
            }
        }
    }
    Ok(dias)
}

/// Los dias en palabras: "todos los dias", "lun a vie", "sab y dom" o la lista.
pub fn describir_dias(dias: &[bool; 7]) -> String {
    match dias {
        [true, true, true, true, true, true, true] => tr!("todos los dias").into(),
        [true, true, true, true, true, false, false] => tr!("lun a vie").into(),
        [false, false, false, false, false, true, true] => tr!("sab y dom").into(),
        _ => DIAS_DE_LA_SEMANA.iter().zip(dias).filter(|(_, elegido)| **elegido).map(|(dia, _)| *dia).collect::<Vec<_>>().join(", "),
    }
}

/// Como entra una tanda cuando no hay un corte natural dentro de su
/// tolerancia.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModoDeEntrada {
    /// Espera el final del tema, aunque salga tarde: nunca corta un tema.
    #[default]
    Tolerancia,
    /// A la hora pautada baja el tema con el fader y entra la tanda (para la
    /// hora en punto o un enlace con la red).
    Exacta,
}

impl ModoDeEntrada {
    /// Nombre corto: "tolerancia" o "exacta".
    pub fn clave(self) -> &'static str {
        match self {
            Self::Tolerancia => "tolerancia",
            Self::Exacta => "exacta",
        }
    }

    pub fn desde_clave(clave: &str) -> Result<Self, String> {
        match simplificar(clave.trim()).as_str() {
            "tolerancia" | "suave" => Ok(Self::Tolerancia),
            "exacta" | "hora exacta" => Ok(Self::Exacta),
            _ => Err(tr!("Modo desconocido: '{clave}' (use tolerancia o exacta).", clave = clave)),
        }
    }

    pub fn describir(self) -> String {
        match self {
            Self::Tolerancia => tr!("con tolerancia"),
            Self::Exacta => tr!("a hora exacta"),
        }
    }
}

/// Una tanda que se emite sola segun su regla. `ultima_salida` es la ultima
/// hora pautada ya atendida (encolada o perdida): no se vuelve a emitir, ni
/// siquiera si el programa se reinicia.
#[derive(Clone, Debug, PartialEq)]
pub struct TandaProgramada {
    pub nombre: String,
    pub archivos: Vec<PathBuf>,
    pub recurrencia: Recurrencia,
    /// Segundos que puede adelantarse o atrasarse respecto de la hora pautada.
    pub antes: u32,
    pub despues: u32,
    pub modo: ModoDeEntrada,
    /// De 1 a 9: con la misma hora, la de mayor prioridad sale primero.
    pub prioridad: u8,
    /// Segundos que puede durar con los avisos de las ordenes (None: sin limite).
    pub duracion_maxima: Option<u32>,
    pub activa: bool,
    pub ultima_salida: Option<DateTime>,
}

/// Salidas pendientes de una tanda a cierta hora.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pendientes {
    /// Las que ya pasaron su tolerancia sin emitirse.
    pub perdidas: Vec<DateTime>,
    /// La proxima que todavia puede salir.
    pub proxima: Option<DateTime>,
}

impl TandaProgramada {
    /// Si el nombre es el de esta tanda (sin importar mayusculas ni acentos).
    pub fn se_llama(&self, nombre: &str) -> bool {
        simplificar(self.nombre.trim()) == simplificar(nombre.trim())
    }

    /// Las salidas posteriores a la ultima atendida: las que ya no llegan y
    /// la proxima. Sin ultima salida, se cuenta desde ahora.
    pub fn pendientes(&self, ahora: DateTime) -> Pendientes {
        let despues = SignedDuration::from_secs(i64::from(self.despues));
        let hace_una_semana = ahora.checked_sub(Span::new().days(DIAS_A_EXPLORAR)).unwrap_or(ahora);
        let desde = self.ultima_salida.unwrap_or(ahora).max(hace_una_semana);
        // Una salida sigue a tiempo mientras no haya pasado su tolerancia.
        let a_tiempo = |salida: &DateTime| salida.checked_add(despues).is_ok_and(|limite| limite >= ahora);
        let limite_de_perdidas = ahora.checked_sub(despues).unwrap_or(ahora);
        let perdidas: Vec<DateTime> =
            if desde < limite_de_perdidas { self.recurrencia.entre(desde, limite_de_perdidas) } else { Vec::new() };
        let perdidas: Vec<DateTime> = perdidas.into_iter().filter(|salida| !a_tiempo(salida)).collect();
        let desde_la_proxima = perdidas.last().copied().unwrap_or(desde);
        let proxima = std::iter::successors(self.recurrencia.proxima_despues_de(desde_la_proxima), |salida| {
            self.recurrencia.proxima_despues_de(*salida)
        })
        .take(1_500)
        .find(a_tiempo);
        Pendientes { perdidas, proxima }
    }
}

/// Que hacer ahora con una salida pendiente de una tanda.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EntradaDeTanda {
    /// Todavia no: hay un corte mejor mas adelante.
    Esperar,
    /// Encolarla ahora para que entre en el proximo corte. `desvio` son los
    /// segundos que entra despues (positivo) o antes (negativo) de la hora
    /// pautada.
    Encolar { desvio: f64 },
    /// Encolarla ahora y, a la hora pautada (en `pautada` segundos), bajar
    /// con el fader lo que suena para que entre (modo de hora exacta).
    Cortar { pautada: f64 },
    /// Ya paso su tolerancia sin poder salir.
    Perdida,
}

/// Decide la entrada de una tanda en el corte natural entre temas. Todos los
/// tiempos son segundos desde ahora:
///
/// - `pautada`: la hora de la tanda; `antes` y `despues`, su tolerancia.
/// - `corte`: cuando termina lo que suena (o el bloque fijo en que esta);
///   None si no suena nada.
/// - `corte_siguiente`: cuando terminaria el tema que viene despues; None si
///   la lista no sigue.
///
/// Si el corte cae dentro de la ventana o despues (no hay uno mejor), se
/// encola. Si cae antes, se espera al siguiente, salvo que ese llegue mas
/// tarde de lo que este llega temprano: entonces conviene entrar antes.
///
/// A hora exacta, un corte que cae despues de la ventana no se espera: a la
/// hora pautada se baja lo que suena (se encola un poco antes).
pub fn decidir_entrada(
    pautada: f64,
    antes: f64,
    despues: f64,
    modo: ModoDeEntrada,
    corte: Option<f64>,
    corte_siguiente: Option<f64>,
) -> EntradaDeTanda {
    let (desde, hasta) = (pautada - antes, pautada + despues);
    let Some(corte) = corte else {
        return if hasta < 0.0 { EntradaDeTanda::Perdida } else { EntradaDeTanda::Esperar };
    };
    if modo == ModoDeEntrada::Exacta && corte > hasta {
        return if pautada <= ANTICIPACION_DEL_CORTE { EntradaDeTanda::Cortar { pautada: pautada.max(0.0) } } else { EntradaDeTanda::Esperar };
    }
    if modo == ModoDeEntrada::Exacta && corte < desde && corte_siguiente.is_some() {
        // Si el siguiente corte tampoco llega, se cortara durante ese tema.
        return EntradaDeTanda::Esperar;
    }
    if corte >= desde {
        // Sin otro corte antes: si la ventana ya paso del todo, igual sale (tarde).
        return EntradaDeTanda::Encolar { desvio: corte - pautada };
    }
    if hasta < 0.0 {
        return EntradaDeTanda::Perdida;
    }
    // Llega temprano. Mas de una tolerancia de adelanto es demasiado.
    let adelanto = desde - corte;
    if adelanto > antes {
        return EntradaDeTanda::Esperar;
    }
    // Si la lista no sigue, mas vale entrar temprano que no entrar.
    let atraso_del_siguiente = corte_siguiente.map_or(f64::INFINITY, |siguiente| siguiente - hasta);
    if atraso_del_siguiente > 0.0 && adelanto <= atraso_del_siguiente {
        EntradaDeTanda::Encolar { desvio: corte - pautada }
    } else {
        EntradaDeTanda::Esperar
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use jiff::civil::date;

    fn hora(texto: &str) -> u32 {
        parsear_hora(texto).unwrap()
    }

    fn cada_media_hora() -> Recurrencia {
        Recurrencia::nueva(hora("10:30"), Some(30), Some(hora("12:00")), [true, true, true, true, true, false, false]).unwrap()
    }

    #[test]
    fn los_horarios_salen_de_la_regla_sin_correrse() {
        // 2026-11-06 es viernes; 2026-11-07, sabado.
        let viernes = cada_media_hora().horarios_del_dia(date(2026, 11, 6));
        let textos: Vec<String> = viernes.iter().map(|salida| salida.time().to_string()).collect();
        assert_eq!(textos, ["10:30:00", "11:00:00", "11:30:00", "12:00:00"]);
        assert!(cada_media_hora().horarios_del_dia(date(2026, 11, 7)).is_empty(), "el sabado no esta elegido");
        let sin_limite = Recurrencia::nueva(hora("22:00"), Some(45), None, [true; 7]).unwrap();
        assert_eq!(sin_limite.horarios_del_dia(date(2026, 11, 7)).len(), 3, "22:00, 22:45 y 23:30");
        let una_vez = Recurrencia::nueva(hora("08:15"), None, None, [true; 7]).unwrap();
        assert_eq!(una_vez.horarios_del_dia(date(2026, 11, 7)), [date(2026, 11, 7).at(8, 15, 0, 0)]);
        // La proxima salida salta el fin de semana.
        let proxima = cada_media_hora().proxima_despues_de(date(2026, 11, 6).at(12, 0, 0, 0));
        assert_eq!(proxima, Some(date(2026, 11, 9).at(10, 30, 0, 0)));
        assert_eq!(cada_media_hora().describir(), "10:30 y cada 30 min hasta 12:00, lun a vie");
    }

    #[test]
    fn la_regla_se_valida() {
        assert!(Recurrencia::nueva(hora("10:00"), Some(0), None, [true; 7]).is_err());
        assert!(Recurrencia::nueva(hora("10:00"), Some(30), Some(hora("09:00")), [true; 7]).is_err());
        assert!(Recurrencia::nueva(hora("10:00"), None, Some(hora("11:00")), [true; 7]).is_err());
        assert!(Recurrencia::nueva(hora("10:00"), None, None, [false; 7]).is_err());
        for invalido in ["24:00", "10", "10:60", "a:30", "10:30:5x", ""] {
            assert!(parsear_hora(invalido).is_err(), "{invalido}");
        }
        assert_eq!(parsear_hora("9:05"), Ok(9 * 3_600 + 5 * 60));
        assert_eq!(formatear_hora(hora("10:30:15")), "10:30:15");
    }

    #[test]
    fn los_dias_se_leen_en_castellano() {
        let leer = |dias: &[&str]| parsear_dias(&dias.iter().map(|dia| dia.to_string()).collect::<Vec<_>>());
        assert_eq!(leer(&["habiles"]), Ok([true, true, true, true, true, false, false]));
        assert_eq!(leer(&["Miércoles", "sab"]), Ok([false, false, true, false, false, true, false]));
        assert_eq!(leer(&["todos"]), Ok([true; 7]));
        assert!(leer(&["feriado"]).is_err());
        assert_eq!(describir_dias(&[true, false, true, false, false, false, false]), "lun, mie");
    }

    fn tanda(ultima: Option<DateTime>) -> TandaProgramada {
        TandaProgramada {
            nombre: "Tanda A".into(),
            archivos: Vec::new(),
            recurrencia: cada_media_hora(),
            antes: 180,
            despues: 120,
            modo: ModoDeEntrada::Tolerancia,
            prioridad: PRIORIDAD_POR_DEFECTO,
            duracion_maxima: None,
            activa: true,
            ultima_salida: ultima,
        }
    }

    #[test]
    fn las_salidas_vencidas_se_pierden_y_sigue_la_proxima() {
        let viernes = date(2026, 11, 6);
        // Atendida la de las 10:30; el programa estuvo cerrado hasta las 11:31.
        let pendientes = tanda(Some(viernes.at(10, 30, 0, 0))).pendientes(viernes.at(11, 31, 0, 0));
        assert_eq!(pendientes.perdidas, [viernes.at(11, 0, 0, 0)]);
        assert_eq!(pendientes.proxima, Some(viernes.at(11, 30, 0, 0)), "las 11:30 todavia estan en tolerancia");
        // Sin ultima salida (recien programada) no hay perdidas.
        let nueva = tanda(None).pendientes(viernes.at(11, 31, 0, 0));
        assert!(nueva.perdidas.is_empty());
        assert_eq!(nueva.proxima, Some(viernes.at(12, 0, 0, 0)));
        // Pasado el ultimo horario del viernes, la proxima es el lunes.
        let tarde = tanda(Some(viernes.at(12, 0, 0, 0))).pendientes(viernes.at(18, 0, 0, 0));
        assert_eq!(tarde.proxima, Some(date(2026, 11, 9).at(10, 30, 0, 0)));
    }

    #[test]
    fn la_tanda_entra_en_el_corte_mas_cercano_a_su_hora() {
        use EntradaDeTanda::*;
        // Pautada en 10 min, tolerancia de 3 min antes y 2 despues.
        let decidir = |corte, siguiente| decidir_entrada(600.0, 180.0, 120.0, ModoDeEntrada::Tolerancia, corte, siguiente);
        // El tema termina dentro de la ventana: se encola.
        assert_eq!(decidir(Some(520.0), Some(760.0)), Encolar { desvio: -80.0 });
        // Termina tarde y no hay un corte antes: sale tarde.
        assert_eq!(decidir(Some(900.0), Some(1_100.0)), Encolar { desvio: 300.0 });
        // Termina temprano y el siguiente entra en la ventana: se espera.
        assert_eq!(decidir(Some(200.0), Some(500.0)), Esperar);
        // Termina un poco temprano y el siguiente se pasa mucho: mejor antes.
        assert_eq!(decidir(Some(380.0), Some(1_000.0)), Encolar { desvio: -220.0 });
        // Termina un poco temprano y el siguiente apenas se pasa: se espera.
        assert_eq!(decidir(Some(380.0), Some(750.0)), Esperar);
        // Demasiado temprano: aunque el siguiente llegue tarde, se espera.
        assert_eq!(decidir(Some(100.0), Some(2_000.0)), Esperar);
        // La lista se acaba: entra temprano antes que perderse.
        assert_eq!(decidir(Some(380.0), None), Encolar { desvio: -220.0 });
        // No suena nada: se espera mientras dure la ventana, despues se pierde.
        assert_eq!(decidir(None, None), Esperar);
        assert_eq!(decidir_entrada(-200.0, 180.0, 120.0, ModoDeEntrada::Tolerancia, None, None), Perdida);
    }

    #[test]
    fn a_hora_exacta_se_corta_si_no_hay_un_corte_en_la_ventana() {
        use EntradaDeTanda::*;
        let exacta = |pautada, corte, siguiente| decidir_entrada(pautada, 60.0, 30.0, ModoDeEntrada::Exacta, corte, siguiente);
        // Un corte natural dentro de la ventana se aprovecha.
        assert_eq!(exacta(300.0, Some(280.0), Some(500.0)), Encolar { desvio: -20.0 });
        // El tema termina despues de la ventana: se corta a la hora, pero se
        // encola recien poco antes.
        assert_eq!(exacta(300.0, Some(400.0), Some(600.0)), Esperar);
        assert_eq!(exacta(80.0, Some(400.0), Some(600.0)), Cortar { pautada: 80.0 });
        // Termina antes de la ventana: se espera al tema siguiente (se cortara ese).
        assert_eq!(exacta(300.0, Some(100.0), Some(600.0)), Esperar);
        // Sin tema siguiente, entra temprano como con tolerancia.
        assert_eq!(exacta(300.0, Some(200.0), None), Encolar { desvio: -100.0 });
        assert_eq!(ModoDeEntrada::desde_clave("Exacta"), Ok(ModoDeEntrada::Exacta));
        assert!(ModoDeEntrada::desde_clave("rapida").is_err());
    }
}
