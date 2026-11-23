//! Ordenes de publicidad: lo que contrata cada anunciante (vigencia, dias,
//! franja horaria, frecuencia y avisos en rotacion) y su reparto en las
//! tandas del dia (la pauta del dia). Todo se calcula a partir de los datos
//! recibidos, sin consultar el reloj, para poder probarlo.
//!
//! El reparto ubica primero las salidas mas dificiles (las que tienen menos
//! tandas posibles) y respeta la duracion maxima de cada tanda, que no haya
//! dos avisos del mismo rubro ni dos de la misma orden en una tanda. Lo que
//! no entra queda como "sin ubicar", con su motivo, para avisarlo antes de
//! salir al aire.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use jiff::civil::{Date, DateTime, Time};

use super::tandas::{describir_dias, formatear_hora};

/// Ventana alrededor de una mencion a hora fija en que puede salir.
pub const TOLERANCIA_DE_MENCION: u32 = 15 * 60;
/// Ventana maxima alrededor de cada salida de una orden "cada tanto".
const TOLERANCIA_MAXIMA_DE_INTERVALO: u32 = 30 * 60;
const SEGUNDOS_POR_DIA: u32 = 86_400;

/// Cuantas veces y cuando sale una orden en el dia.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frecuencia {
    /// Cada tantos minutos desde el inicio de la franja ("cada 2 horas").
    CadaMinutos(u32),
    /// Tantas veces por dia, repartidas en partes iguales de la franja.
    VecesPorDia(u32),
    /// A esas horas (segundos desde la medianoche).
    EnHorarios(Vec<u32>),
}

impl Frecuencia {
    pub fn describir(&self) -> String {
        match self {
            Self::CadaMinutos(minutos) if minutos % 60 == 0 => tr!("cada {} h", minutos / 60),
            Self::CadaMinutos(minutos) => tr!("cada {minutos} min", minutos = minutos),
            Self::VecesPorDia(veces) => tr!("{veces} por dia", veces = veces),
            Self::EnHorarios(horarios) => {
                tr!("a las {}", horarios.iter().map(|horario| formatear_hora(*horario)).collect::<Vec<_>>().join(", "))
            }
        }
    }
}

/// Lo que contrato un anunciante.
#[derive(Clone, Debug, PartialEq)]
pub struct OrdenDePublicidad {
    /// Identifica la orden ("Sol - Diciembre").
    pub nombre: String,
    pub anunciante: String,
    /// Rubro ("Supermercados"): dos avisos del mismo rubro no van en la misma tanda.
    pub rubro: String,
    pub desde: Date,
    pub hasta: Date,
    /// De lunes a domingo.
    pub dias: [bool; 7],
    /// Franja horaria en que puede salir (segundos desde la medianoche).
    pub franja: (u32, u32),
    pub frecuencia: Frecuencia,
    /// Avisos que rotan, uno por salida.
    pub avisos: Vec<PathBuf>,
    /// Tandas en que puede salir (vacio: en cualquiera).
    pub tandas: Vec<String>,
    /// De 1 a 9: con la misma dificultad se ubica antes la de mayor prioridad.
    pub prioridad: u8,
    pub activa: bool,
}

impl OrdenDePublicidad {
    /// Valida lo que no puede quedar mal: nombre, fechas, franja, frecuencia
    /// y avisos.
    pub fn validar(&self) -> Result<(), String> {
        if self.nombre.trim().is_empty() {
            return Err(tr!("Indique el nombre de la orden.").into());
        }
        if self.anunciante.trim().is_empty() {
            return Err(tr!("Indique el anunciante.").into());
        }
        if self.hasta < self.desde {
            return Err(tr!("La vigencia termina antes de empezar.").into());
        }
        if !self.dias.contains(&true) {
            return Err(tr!("Elija al menos un dia de la semana.").into());
        }
        let (inicio, fin) = self.franja;
        if fin <= inicio || fin >= SEGUNDOS_POR_DIA {
            return Err(tr!("La franja horaria debe terminar despues de empezar (de 00:00 a 23:59).").into());
        }
        match &self.frecuencia {
            Frecuencia::CadaMinutos(minutos) if !(5..=1_440).contains(minutos) => {
                return Err(tr!("El intervalo va de 5 minutos a 1440 (un dia).").into());
            }
            Frecuencia::VecesPorDia(veces) if !(1..=96).contains(veces) => return Err(tr!("Las veces por dia van de 1 a 96.").into()),
            Frecuencia::EnHorarios(horarios) if horarios.is_empty() => return Err(tr!("Indique al menos un horario.").into()),
            _ => {}
        }
        if self.avisos.is_empty() {
            return Err(tr!("Indique al menos un aviso.").into());
        }
        Ok(())
    }

    /// Si la orden rige esa fecha (activa, vigente y en un dia elegido).
    pub fn rige(&self, fecha: Date) -> bool {
        self.activa
            && fecha >= self.desde
            && fecha <= self.hasta
            && self.dias[fecha.weekday().to_monday_zero_offset() as usize]
    }

    /// Si puede salir en esa tanda.
    pub fn admite_tanda(&self, tanda: &str) -> bool {
        self.tandas.is_empty() || self.tandas.iter().any(|permitida| simplificar(permitida) == simplificar(tanda))
    }

    pub fn se_llama(&self, nombre: &str) -> bool {
        simplificar(&self.nombre) == simplificar(nombre)
    }

    /// La orden en palabras: "Sol: cada 2 h de 08:00 a 20:00, lun a vie, del 2026-11-01 al 2026-11-30".
    pub fn describir(&self) -> String {
        tr!("{} de {} a {}, {}, del {} al {}", self.frecuencia.describir(), formatear_hora(self.franja.0), formatear_hora(self.franja.1), describir_dias(&self.dias), self.desde, self.hasta)
    }

    /// Las salidas que pide para esa fecha (ninguna si no rige).
    pub fn demandas_del_dia(&self, fecha: Date) -> Vec<Demanda> {
        if !self.rige(fecha) {
            return Vec::new();
        }
        let (inicio, fin) = self.franja;
        let en = |segundos: u32| fecha.to_datetime(hora_del_dia(segundos));
        let demanda = |objetivo: u32, desde: u32, hasta: u32, preferir_la_menos_cargada: bool| Demanda {
            orden: self.nombre.clone(),
            objetivo: en(objetivo),
            desde: en(desde),
            hasta: en(hasta.min(SEGUNDOS_POR_DIA - 1)),
            preferir_la_menos_cargada,
            aviso: None,
            reposicion: false,
        };
        match &self.frecuencia {
            Frecuencia::CadaMinutos(minutos) => {
                let paso = minutos * 60;
                let margen = (paso / 2).min(TOLERANCIA_MAXIMA_DE_INTERVALO);
                (0..)
                    .map(|numero| inicio + numero * paso)
                    .take_while(|objetivo| *objetivo <= fin)
                    .map(|objetivo| demanda(objetivo, objetivo.saturating_sub(margen).max(inicio), (objetivo + margen).min(fin), false))
                    .collect()
            }
            Frecuencia::VecesPorDia(veces) => {
                let largo = (fin - inicio) / veces;
                (0..*veces)
                    .map(|parte| {
                        let desde = inicio + parte * largo;
                        let hasta = if parte + 1 == *veces { fin } else { desde + largo };
                        demanda(desde + (hasta - desde) / 2, desde, hasta, true)
                    })
                    .collect()
            }
            Frecuencia::EnHorarios(horarios) => {
                let mut horarios = horarios.clone();
                horarios.sort_unstable();
                horarios.dedup();
                horarios
                    .into_iter()
                    .map(|objetivo| demanda(objetivo, objetivo.saturating_sub(TOLERANCIA_DE_MENCION), objetivo + TOLERANCIA_DE_MENCION, false))
                    .collect()
            }
        }
    }
}

/// Texto para comparar nombres: sin espacios en los bordes, mayusculas ni acentos.
pub fn simplificar(texto: &str) -> String {
    texto.trim().chars().flat_map(char::to_lowercase).map(super::duplicados::sin_acento).collect()
}

fn hora_del_dia(segundos: u32) -> Time {
    let segundos = segundos.min(SEGUNDOS_POR_DIA - 1);
    Time::constant((segundos / 3_600) as i8, (segundos / 60 % 60) as i8, (segundos % 60) as i8, 0)
}

/// Una salida que pide una orden: cuando le gustaria (`objetivo`) y entre
/// que horas puede ir.
#[derive(Clone, Debug, PartialEq)]
pub struct Demanda {
    pub orden: String,
    pub objetivo: DateTime,
    pub desde: DateTime,
    pub hasta: DateTime,
    /// "N por dia": entre las tandas posibles, la menos cargada. Si no, la
    /// mas cercana al objetivo.
    pub preferir_la_menos_cargada: bool,
    /// Aviso que le toca (en la rotacion o el que se repone).
    pub aviso: Option<PathBuf>,
    pub reposicion: bool,
}

/// Una salida de una tanda en el dia, donde pueden ir avisos de las ordenes.
#[derive(Clone, Debug, PartialEq)]
pub struct Hueco {
    pub tanda: String,
    pub salida: DateTime,
    /// Segundos que puede durar la tanda (None: sin limite).
    pub capacidad: Option<f64>,
    /// Segundos que ya ocupan sus avisos fijos.
    pub ocupado: f64,
}

/// Un aviso de una orden ubicado en una salida de una tanda.
#[derive(Clone, Debug, PartialEq)]
pub struct Asignacion {
    pub orden: String,
    pub aviso: PathBuf,
    pub tanda: String,
    pub salida: DateTime,
    /// Repone una salida perdida o cortada.
    pub reposicion: bool,
}

/// Una salida de una orden que no entro en ninguna tanda.
#[derive(Clone, Debug, PartialEq)]
pub struct SinUbicar {
    pub orden: String,
    pub objetivo: DateTime,
    pub motivo: String,
}

/// Una salida a reponer en el dia: el aviso de una orden que se perdio o se
/// corto, desde ese momento.
#[derive(Clone, Debug, PartialEq)]
pub struct Reposicion {
    pub orden: String,
    pub aviso: PathBuf,
    pub desde: DateTime,
}

/// El reparto de las ordenes en las tandas de un dia.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PautaDelDia {
    pub asignaciones: Vec<Asignacion>,
    pub sin_ubicar: Vec<SinUbicar>,
}

impl PautaDelDia {
    /// Los avisos de las ordenes para esa salida de esa tanda, en orden.
    pub fn avisos_de(&self, tanda: &str, salida: DateTime) -> Vec<&Asignacion> {
        self.asignaciones.iter().filter(|asignacion| asignacion.tanda == tanda && asignacion.salida == salida).collect()
    }
}

/// Reparte en las salidas de las tandas (`huecos`) lo que piden las ordenes
/// que rigen en la fecha, mas las reposiciones. `duracion` da los segundos
/// de cada aviso.
pub fn repartir(
    fecha: Date,
    ordenes: &[OrdenDePublicidad],
    mut huecos: Vec<Hueco>,
    reposiciones: &[Reposicion],
    duracion: &dyn Fn(&Path) -> f64,
) -> PautaDelDia {
    huecos.sort_by_key(|hueco| hueco.salida);
    let por_nombre: HashMap<String, &OrdenDePublicidad> = ordenes.iter().map(|orden| (simplificar(&orden.nombre), orden)).collect();
    // Cada demanda con el aviso que le toca en la rotacion (que sigue de un dia al otro).
    let mut demandas: Vec<Demanda> = Vec::new();
    for orden in ordenes {
        let del_dia = orden.demandas_del_dia(fecha);
        let base = (fecha.day_of_year() as usize).wrapping_mul(del_dia.len());
        for (numero, mut demanda) in del_dia.into_iter().enumerate() {
            demanda.aviso = Some(orden.avisos[(base + numero) % orden.avisos.len()].clone());
            demandas.push(demanda);
        }
    }
    for reposicion in reposiciones {
        let Some(orden) = por_nombre.get(&simplificar(&reposicion.orden)) else { continue };
        let fin = fecha.to_datetime(hora_del_dia(orden.franja.1));
        let desde = reposicion.desde.max(fecha.to_datetime(hora_del_dia(orden.franja.0)));
        demandas.push(Demanda {
            orden: orden.nombre.clone(),
            objetivo: desde,
            desde,
            hasta: fin,
            preferir_la_menos_cargada: false,
            aviso: Some(reposicion.aviso.clone()),
            reposicion: true,
        });
    }
    let posibles = |demanda: &Demanda| {
        let orden = por_nombre.get(&simplificar(&demanda.orden));
        huecos
            .iter()
            .enumerate()
            .filter(|(_, hueco)| hueco.salida >= demanda.desde && hueco.salida <= demanda.hasta)
            .filter(|(_, hueco)| orden.is_none_or(|orden| orden.admite_tanda(&hueco.tanda)))
            .map(|(indice, _)| indice)
            .collect::<Vec<usize>>()
    };
    // Primero las mas dificiles: menos tandas posibles y, a igual cantidad,
    // la de mayor prioridad y la mas temprana.
    let mut orden_de_ubicacion: Vec<(usize, Vec<usize>)> = demandas.iter().map(posibles).enumerate().collect();
    orden_de_ubicacion.sort_by_key(|(indice, posibles)| {
        let demanda = &demandas[*indice];
        let prioridad = por_nombre.get(&simplificar(&demanda.orden)).map_or(0, |orden| orden.prioridad);
        (posibles.len(), Reverse(prioridad), demanda.objetivo)
    });

    let mut ocupado: Vec<f64> = huecos.iter().map(|hueco| hueco.ocupado).collect();
    let mut en_hueco: Vec<Vec<(String, String)>> = vec![Vec::new(); huecos.len()];
    let mut pauta = PautaDelDia::default();
    let mut ubicadas: Vec<(usize, Asignacion)> = Vec::new();
    for (indice, posibles) in orden_de_ubicacion {
        let demanda = &demandas[indice];
        let aviso = demanda.aviso.clone().expect("cada demanda tiene su aviso");
        let rubro = por_nombre.get(&simplificar(&demanda.orden)).map(|orden| simplificar(&orden.rubro)).unwrap_or_default();
        let segundos = duracion(&aviso);
        let mut motivo = if posibles.is_empty() {
            tr!("no hay tandas entre las {} y las {}", hora_corta(demanda.desde), hora_corta(demanda.hasta))
        } else {
            String::new()
        };
        let candidatas: Vec<usize> = posibles
            .into_iter()
            .filter(|&hueco| {
                let ya = &en_hueco[hueco];
                if ya.iter().any(|(orden, _)| simplificar(orden) == simplificar(&demanda.orden)) {
                    motivo = tr!("ya sale en las tandas posibles").into();
                    return false;
                }
                if !rubro.is_empty() && ya.iter().any(|(_, otro)| *otro == rubro) {
                    motivo = tr!("otro aviso de su rubro ocupa las tandas posibles").into();
                    return false;
                }
                if huecos[hueco].capacidad.is_some_and(|capacidad| ocupado[hueco] + segundos > capacidad + 0.5) {
                    motivo = tr!("las tandas posibles estan llenas").into();
                    return false;
                }
                true
            })
            .collect();
        let distancia = |hueco: usize| huecos[hueco].salida.duration_since(demanda.objetivo).abs();
        let elegida = if demanda.preferir_la_menos_cargada {
            candidatas.iter().copied().min_by(|&una, &otra| {
                ocupado[una].total_cmp(&ocupado[otra]).then(distancia(una).cmp(&distancia(otra)))
            })
        } else {
            candidatas.iter().copied().min_by_key(|&hueco| (distancia(hueco), hueco))
        };
        match elegida {
            Some(hueco) => {
                ocupado[hueco] += segundos;
                en_hueco[hueco].push((demanda.orden.clone(), rubro));
                ubicadas.push((
                    indice,
                    Asignacion {
                        orden: demanda.orden.clone(),
                        aviso,
                        tanda: huecos[hueco].tanda.clone(),
                        salida: huecos[hueco].salida,
                        reposicion: demanda.reposicion,
                    },
                ));
            }
            None => pauta.sin_ubicar.push(SinUbicar { orden: demanda.orden.clone(), objetivo: demanda.objetivo, motivo }),
        }
    }
    // En cada tanda, los avisos en el orden de la pauta (por salida y por orden de ubicacion).
    ubicadas.sort_by_key(|(indice, asignacion)| (asignacion.salida, *indice));
    pauta.asignaciones = ubicadas.into_iter().map(|(_, asignacion)| asignacion).collect();
    pauta.sin_ubicar.sort_by_key(|sin_ubicar| sin_ubicar.objetivo);
    pauta
}

fn hora_corta(momento: DateTime) -> String {
    format!("{:02}:{:02}", momento.hour(), momento.minute())
}

/// Como se anota la salida de una tanda en el registro y en el origen de sus
/// avisos: "Mediodia 12:30".
pub fn etiqueta_de_salida(tanda: &str, salida: DateTime) -> String {
    format!("{tanda} {}", hora_corta(salida))
}

/// Un aviso de una orden que ya se entrego a una salida: encolado en su
/// bloque o emitido. `etiqueta` es la de `etiqueta_de_salida`.
#[derive(Clone, Debug, PartialEq)]
pub struct AvisoEntregado {
    pub etiqueta: String,
    pub orden: String,
    pub aviso: PathBuf,
}

/// Lo que la pauta le asigna a salidas que ya se atendieron (encoladas o
/// emitidas) y nunca se les entrego: pasa cuando una orden se carga o se
/// cambia despues de que su tanda ya se encolo. `atendidas` es cada tanda con
/// su ultima salida atendida. Cada entrega o reposicion cubre una sola
/// asignacion: primero la de la misma orden y el mismo aviso; si no, la de la
/// misma orden (cambio la rotacion) o la del mismo aviso (se renombro la orden).
pub fn avisos_sin_entregar(
    pauta: &PautaDelDia,
    atendidas: &[(String, DateTime)],
    entregados: &[AvisoEntregado],
    reposiciones: &[Reposicion],
) -> Vec<Asignacion> {
    let atendida = |asignacion: &Asignacion| {
        !asignacion.reposicion
            && atendidas.iter().any(|(tanda, ultima)| *tanda == asignacion.tanda && asignacion.salida <= *ultima)
    };
    let pendientes: Vec<&Asignacion> = pauta.asignaciones.iter().filter(|asignacion| atendida(asignacion)).collect();
    let mut cubierta = vec![false; pendientes.len()];
    // Tres pasadas, de la coincidencia mas exacta a la mas floja, para que
    // una entrega floja no le quite la suya a una exacta.
    let mut libres: Vec<&AvisoEntregado> = entregados.iter().collect();
    let mut repuestas: Vec<&Reposicion> = reposiciones.iter().collect();
    let pasadas: [fn(&Asignacion, &str, &Path) -> bool; 3] = [
        |asignacion, orden, aviso| asignacion.orden == orden && asignacion.aviso == aviso,
        |asignacion, orden, _| asignacion.orden == orden,
        |asignacion, _, aviso| asignacion.aviso == aviso,
    ];
    for coincide in pasadas {
        for (indice, asignacion) in pendientes.iter().enumerate() {
            if cubierta[indice] {
                continue;
            }
            let etiqueta = etiqueta_de_salida(&asignacion.tanda, asignacion.salida);
            if let Some(posicion) =
                libres.iter().position(|entregado| entregado.etiqueta == etiqueta && coincide(asignacion, &entregado.orden, &entregado.aviso))
            {
                libres.remove(posicion);
                cubierta[indice] = true;
            } else if let Some(posicion) = repuestas.iter().position(|repuesta| coincide(asignacion, &repuesta.orden, &repuesta.aviso)) {
                repuestas.remove(posicion);
                cubierta[indice] = true;
            }
        }
    }
    pendientes.into_iter().zip(cubierta).filter(|(_, cubierta)| !cubierta).map(|(asignacion, _)| asignacion.clone()).collect()
}

/// Cuantas salidas pide una orden entre esas fechas (inclusive).
pub fn salidas_contratadas(orden: &OrdenDePublicidad, desde: Date, hasta: Date) -> usize {
    let mut total = 0;
    let mut fecha = desde.max(orden.desde);
    let ultimo = hasta.min(orden.hasta);
    while fecha <= ultimo {
        total += orden.demandas_del_dia(fecha).len();
        let Ok(siguiente) = fecha.tomorrow() else { break };
        fecha = siguiente;
    }
    total
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::dominio::parsear_hora;
    use jiff::civil::date;

    fn hora(texto: &str) -> u32 {
        parsear_hora(texto).unwrap()
    }

    fn orden(nombre: &str, rubro: &str, frecuencia: Frecuencia) -> OrdenDePublicidad {
        OrdenDePublicidad {
            nombre: nombre.into(),
            anunciante: format!("Anunciante {nombre}"),
            rubro: rubro.into(),
            desde: date(2026, 11, 1),
            hasta: date(2026, 11, 30),
            dias: [true; 7],
            franja: (hora("08:00"), hora("20:00")),
            frecuencia,
            avisos: vec![PathBuf::from(format!("{nombre}-1.mp3")), PathBuf::from(format!("{nombre}-2.mp3"))],
            tandas: Vec::new(),
            prioridad: 5,
            activa: true,
        }
    }

    // Tandas cada 30 minutos de 08:00 a 20:00, sin limite.
    fn huecos(fecha: Date, capacidad: Option<f64>) -> Vec<Hueco> {
        (0..=24)
            .map(|numero| Hueco {
                tanda: "Tanda A".into(),
                salida: fecha.to_datetime(hora_del_dia(hora("08:00") + numero * 1_800)),
                capacidad,
                ocupado: 0.0,
            })
            .collect()
    }

    #[test]
    fn cada_dos_horas_sale_siete_veces_en_su_horario() {
        let fecha = date(2026, 11, 7);
        let sol = orden("Sol", "Supermercados", Frecuencia::CadaMinutos(120));
        let pauta = repartir(fecha, std::slice::from_ref(&sol), huecos(fecha, None), &[], &|_| 30.0);
        let horas: Vec<String> = pauta.asignaciones.iter().map(|asignacion| hora_corta(asignacion.salida)).collect();
        assert_eq!(horas, ["08:00", "10:00", "12:00", "14:00", "16:00", "18:00", "20:00"]);
        assert!(pauta.sin_ubicar.is_empty());
        // Los avisos rotan.
        assert_ne!(pauta.asignaciones[0].aviso, pauta.asignaciones[1].aviso);
        assert_eq!(salidas_contratadas(&sol, date(2026, 11, 1), date(2026, 11, 3)), 21);
        assert_eq!(salidas_contratadas(&sol, date(2026, 10, 30), date(2026, 11, 1)), 7, "fuera de vigencia no cuenta");
    }

    #[test]
    fn tres_por_dia_van_una_en_cada_tercio_de_la_franja() {
        let fecha = date(2026, 11, 7);
        let ruta = orden("Ruta", "Autos", Frecuencia::VecesPorDia(3));
        let pauta = repartir(fecha, &[ruta], huecos(fecha, None), &[], &|_| 30.0);
        assert_eq!(pauta.asignaciones.len(), 3);
        let horas: Vec<u32> = pauta.asignaciones.iter().map(|asignacion| asignacion.salida.hour() as u32).collect();
        assert!(horas[0] < 12 && (12..16).contains(&horas[1]) && horas[2] >= 16, "{horas:?}");
    }

    #[test]
    fn la_competencia_y_la_capacidad_se_respetan() {
        let fecha = date(2026, 11, 7);
        // Dos menciones del mismo rubro en tandas distintas conviven.
        let a = orden("A", "Supermercados", Frecuencia::EnHorarios(vec![hora("10:00")]));
        let b = orden("B", "Supermercados", Frecuencia::EnHorarios(vec![hora("10:20")]));
        let pauta = repartir(fecha, &[a.clone(), b], huecos(fecha, None), &[], &|_| 30.0);
        assert!(pauta.sin_ubicar.is_empty(), "{:?}", pauta.sin_ubicar);
        let horas: Vec<String> = pauta.asignaciones.iter().map(|asignacion| hora_corta(asignacion.salida)).collect();
        assert_eq!(horas, ["10:00", "10:30"]);
        // Con una sola tanda posible y otra del mismo rubro, uno queda sin ubicar.
        let una_tanda = vec![Hueco { tanda: "T".into(), salida: fecha.at(10, 0, 0, 0), capacidad: None, ocupado: 0.0 }];
        let c = orden("C", "Supermercados", Frecuencia::EnHorarios(vec![hora("10:00")]));
        let pauta = repartir(fecha, &[a.clone(), c], una_tanda.clone(), &[], &|_| 30.0);
        assert_eq!(pauta.asignaciones.len(), 1);
        assert_eq!(pauta.sin_ubicar[0].motivo, "otro aviso de su rubro ocupa las tandas posibles");
        // Tanda de 60 s con 40 ya ocupados: un aviso de 30 s no entra.
        let llena = vec![Hueco { capacidad: Some(60.0), ocupado: 40.0, ..una_tanda[0].clone() }];
        let pauta = repartir(fecha, &[a], llena, &[], &|_| 30.0);
        assert_eq!(pauta.sin_ubicar[0].motivo, "las tandas posibles estan llenas");
    }

    #[test]
    fn primero_se_ubica_la_orden_mas_dificil() {
        let fecha = date(2026, 11, 7);
        // Solo dos tandas, con lugar para un aviso cada una.
        let dos = vec![
            Hueco { tanda: "T".into(), salida: fecha.at(10, 0, 0, 0), capacidad: Some(30.0), ocupado: 0.0 },
            Hueco { tanda: "T".into(), salida: fecha.at(11, 0, 0, 0), capacidad: Some(30.0), ocupado: 0.0 },
        ];
        // "Flexible" puede ir en las dos; "Fija" solo a las 10. Aunque Flexible
        // pida las 10, Fija se ubica primero y Flexible pasa a las 11.
        let flexible = OrdenDePublicidad { franja: (hora("09:30"), hora("11:30")), ..orden("Flexible", "", Frecuencia::VecesPorDia(1)) };
        let fija = orden("Fija", "", Frecuencia::EnHorarios(vec![hora("10:00")]));
        let fija = OrdenDePublicidad { franja: (hora("09:50"), hora("10:10")), frecuencia: Frecuencia::EnHorarios(vec![hora("10:00")]), ..fija };
        let pauta = repartir(fecha, &[flexible, fija], dos, &[], &|_| 30.0);
        assert!(pauta.sin_ubicar.is_empty(), "{:?}", pauta.sin_ubicar);
        let de = |orden: &str| pauta.asignaciones.iter().find(|asignacion| asignacion.orden == orden).unwrap().salida.hour();
        assert_eq!((de("Fija"), de("Flexible")), (10, 11));
    }

    #[test]
    fn las_reposiciones_van_en_la_proxima_tanda_posible() {
        let fecha = date(2026, 11, 7);
        let sol = orden("Sol", "", Frecuencia::EnHorarios(vec![hora("09:00")]));
        let reponer = Reposicion { orden: "sol".into(), aviso: PathBuf::from("Sol-2.mp3"), desde: fecha.at(12, 10, 0, 0) };
        let pauta = repartir(fecha, &[sol], huecos(fecha, None), &[reponer], &|_| 30.0);
        let repuesta = pauta.asignaciones.iter().find(|asignacion| asignacion.reposicion).unwrap();
        assert_eq!(hora_corta(repuesta.salida), "12:30");
        assert_eq!(repuesta.aviso, PathBuf::from("Sol-2.mp3"));
    }

    #[test]
    fn la_orden_solo_rige_en_su_vigencia_y_sus_dias() {
        let mut sol = orden("Sol", "", Frecuencia::VecesPorDia(2));
        sol.dias = [true, true, true, true, true, false, false];
        assert!(sol.demandas_del_dia(date(2026, 11, 7)).is_empty(), "sabado");
        assert_eq!(sol.demandas_del_dia(date(2026, 11, 6)).len(), 2);
        assert!(sol.demandas_del_dia(date(2026, 12, 1)).is_empty(), "vencida");
        assert!(sol.validar().is_ok());
        assert!(OrdenDePublicidad { franja: (hora("20:00"), hora("08:00")), ..sol.clone() }.validar().is_err());
        assert!(OrdenDePublicidad { avisos: Vec::new(), ..sol.clone() }.validar().is_err());
        assert_eq!(sol.describir(), "2 por dia de 08:00 a 20:00, lun a vie, del 2026-11-01 al 2026-11-30");
        assert!(sol.admite_tanda("cualquiera"));
        let solo_a = OrdenDePublicidad { tandas: vec!["Tanda A".into()], ..sol };
        assert!(solo_a.admite_tanda("tanda a") && !solo_a.admite_tanda("Tanda B"));
    }

    #[test]
    fn lo_que_se_cargo_despues_de_encolar_la_salida_queda_sin_entregar() {
        let fecha = date(2026, 11, 16);
        let (a_las_1230, a_las_1300) = (fecha.at(12, 30, 0, 0), fecha.at(13, 0, 0, 0));
        let asignacion = |orden: &str, aviso: &str, salida: DateTime, reposicion: bool| Asignacion {
            orden: orden.into(),
            aviso: PathBuf::from(aviso),
            tanda: "Mediodia".into(),
            salida,
            reposicion,
        };
        let entregado = |orden: &str, aviso: &str| AvisoEntregado { etiqueta: "Mediodia 12:30".into(), orden: orden.into(), aviso: PathBuf::from(aviso) };
        let pauta = PautaDelDia {
            asignaciones: vec![
                asignacion("Albion", "albion-1.flac", a_las_1230, false),
                asignacion("Brahma", "brahma.flac", a_las_1230, false),
                asignacion("Punto Norte", "punto-1.flac", a_las_1230, false),
                asignacion("Sol", "sol.flac", a_las_1230, true),
                asignacion("Albion", "albion-2.flac", a_las_1300, false),
            ],
            ..PautaDelDia::default()
        };
        let atendidas = [("Mediodia".to_string(), a_las_1230)];
        // Solo salio la apertura fija: las tres de las 12:30 faltan; la de las
        // 13:00 todavia no se atendio y una reposicion no se vuelve a reponer.
        let faltan = avisos_sin_entregar(&pauta, &atendidas, &[], &[]);
        let ordenes: Vec<&str> = faltan.iter().map(|asignacion| asignacion.orden.as_str()).collect();
        assert_eq!(ordenes, ["Albion", "Brahma", "Punto Norte"]);
        // Lo encolado, lo emitido con otro aviso de la rotacion, lo de una
        // orden renombrada y lo ya repuesto cuentan como entregados.
        let entregados = [entregado("Albion", "albion-2.flac"), entregado("Brahma (renombrada)", "brahma.flac")];
        let repuesta = Reposicion { orden: "Punto Norte".into(), aviso: "punto-1.flac".into(), desde: fecha.at(12, 40, 0, 0) };
        assert!(avisos_sin_entregar(&pauta, &atendidas, &entregados, &[repuesta]).is_empty());
        // Una entrega cubre una sola asignacion: dos veces Brahma, una entregada.
        let doble = PautaDelDia {
            asignaciones: vec![asignacion("Brahma", "brahma.flac", a_las_1230, false), asignacion("Brahma", "brahma.flac", a_las_1230, false)],
            ..PautaDelDia::default()
        };
        assert_eq!(avisos_sin_entregar(&doble, &atendidas, &[entregado("Brahma", "brahma.flac")], &[]).len(), 1);
        // Sin salidas atendidas no falta nada.
        assert!(avisos_sin_entregar(&pauta, &[], &[], &[]).is_empty());
    }
}
