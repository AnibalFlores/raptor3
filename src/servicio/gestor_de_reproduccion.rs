//! Servicio de reproduccion que delega la emision de audio en motores.
//!
//! Responsabilidad unica: coordinar el ciclo de vida de la reproduccion
//! (reproducir, pausar, reanudar, detener, volumen, busqueda) e informar el fin
//! de cada pista para que el controlador avance la secuencia.
//!
//! Mezcla: usa dos motores que se alternan. Cuando a la pista en curso le
//! quedan los segundos de mezcla que indica la transicion, la siguiente
//! arranca en el otro motor y se hace un crossfade: la que entra sube de 0 al
//! volumen elegido (en linea recta o con curva de potencia) y la que sale
//! baja a 0 con una pendiente que se acentua hacia el final, para que no
//! tape a la entrante. El avance del fundido se mide con la posicion de la
//! pista entrante, asi que una pausa lo congela sin calculos extra. La misma
//! mezcla se usa cuando el usuario pide pasar a otra pista con el fader.
//!
//! Si la outro de la saliente empieza antes que la mezcla, la saliente ya
//! empieza a bajar desde su outro (unos pocos dB, en forma pareja hasta que
//! entra la siguiente), salvo que el tema tenga un fundido natural.
//!
//! Una transicion sincronizada (la del crossfade inteligente) se prepara un
//! instante antes y programa la entrante en el frame exacto de la salida en
//! que debe arrancar, para que sus golpes caigan sobre los de la saliente.
//!
//! Cada pista puede tener una ganancia propia (la nivelacion de volumen), que
//! se aplica ademas del volumen elegido, y recortes: empieza despues de su
//! recorte inicial y termina su recorte final antes del final del archivo.
//!
//! Detener con fader baja la musica a 0 en los segundos pedidos (medidos con
//! el reloj, no con la pista) y recien entonces la detiene.

use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;
use std::time::Instant;

use crate::audio::Motor;
use crate::dominio::{Pista, PistaInvalida};

// Por debajo de este resto no vale la pena mezclar: la pista termina sola y
// la secuencia sigue por el camino normal.
const RESTO_MINIMO_PARA_MEZCLAR: f64 = 0.2;
// Anticipacion con que se prepara una transicion sincronizada: tiempo para
// que la entrante abra el archivo y llene su buffer antes de sonar.
const ANTICIPACION_DE_SINCRONIZACION: f64 = 0.5;
// Exponente de la caida de la saliente durante la mezcla: (1 - avance)^n.
// Con 2, a mitad de la mezcla queda a -12 dB y a tres cuartos, a -24 dB.
const EXPONENTE_DE_SALIDA: f64 = 2.0;
// Lo que baja la saliente entre el inicio de su outro y el comienzo de la
// mezcla, en dB, con caida pareja en dB.
const ATENUACION_PREVIA_DB: f64 = 6.0;
// Por debajo de este tramo entre la outro y la mezcla no se baja antes.
const TRAMO_MINIMO_DE_ATENUACION_PREVIA: f64 = 0.5;

/// Como suben y bajan las pistas durante la mezcla.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curva {
    /// La que entra sube de 0 al volumen y la que sale baja a 0 en linea recta.
    Lineal,
    /// La que entra sube con el seno (potencia), asi llega pronto a un nivel
    /// audible. Una pista con fundido natural en ese tramo baja (o sube) sola:
    /// la entrante queda a volumen pleno y la saliente solo se acompana en
    /// linea recta, para asegurar que termine en silencio.
    Potencia { saliente_natural: bool, entrante_natural: bool },
}

impl Curva {
    /// Factores de volumen (entrante, saliente) para un avance de 0 a 1. La
    /// saliente cae con (1 - avance)^2: lenta al principio y cada vez mas
    /// rapida, para que no siga sonando fuerte encima de la entrante.
    fn factores(self, progreso: f64) -> (f64, f64) {
        let progreso = progreso.clamp(0.0, 1.0);
        let caida = (1.0 - progreso).powf(EXPONENTE_DE_SALIDA);
        match self {
            Curva::Lineal => (progreso, caida),
            Curva::Potencia { saliente_natural, entrante_natural } => (
                if entrante_natural { 1.0 } else { (progreso * FRAC_PI_2).sin() },
                if saliente_natural { 1.0 - progreso } else { caida },
            ),
        }
    }

    fn saliente_natural(self) -> bool {
        matches!(self, Curva::Potencia { saliente_natural: true, .. })
    }
}

/// Factor de volumen de la pista en curso antes de la mezcla: 1 hasta el
/// inicio de su outro (`desde`) y despues baja pareja en dB hasta
/// ATENUACION_PREVIA_DB al llegar la mezcla (`hasta`). Con fundido natural, o
/// si la outro no empieza antes que la mezcla, no se toca.
fn atenuacion_previa(posicion: f64, desde: f64, hasta: f64, natural: bool) -> f64 {
    if natural || hasta - desde < TRAMO_MINIMO_DE_ATENUACION_PREVIA || posicion <= desde {
        return 1.0;
    }
    let avance = ((posicion - desde) / (hasta - desde)).clamp(0.0, 1.0);
    10f64.powf(-ATENUACION_PREVIA_DB * avance / 20.0)
}

/// Mezcla hacia la pista siguiente: cual entra, cuantos segundos se
/// superpone con la que termina y como.
#[derive(Clone, Debug, PartialEq)]
pub struct Transicion {
    pub siguiente: Pista,
    pub segundos: f64,
    /// Desde donde arranca la entrante (por ejemplo, despues de su silencio).
    pub inicio: f64,
    /// Segundos antes del final del archivo de la saliente en que termina la
    /// mezcla (su ultimo sonido).
    pub recorte_final: f64,
    /// Corrimiento del arranque de la entrante, para alinear los golpes;
    /// positivo es mas tarde.
    pub desfase: f64,
    /// Segundos antes del final del archivo de la saliente en que empieza su
    /// outro: desde ahi empieza a bajar. 0 si solo baja durante la mezcla.
    pub outro_saliente: f64,
    pub curva: Curva,
    /// Arrancar la entrante en el frame exacto de la salida.
    pub sincronizada: bool,
    /// Descripcion para la barra de estado.
    pub detalle: String,
}

impl Transicion {
    /// Mezcla lineal desde el principio de la siguiente hasta el final de la
    /// actual.
    pub fn simple(siguiente: Pista, segundos: f64) -> Self {
        Self {
            siguiente,
            segundos,
            inicio: 0.0,
            recorte_final: 0.0,
            desfase: 0.0,
            outro_saliente: 0.0,
            curva: Curva::Lineal,
            sincronizada: false,
            detalle: String::new(),
        }
    }
}

/// Lo que paso durante un ciclo del gestor y el controlador debe atender.
pub enum EventoDeReproduccion {
    /// La pista termino (o fallo) sin haber mezclado con la siguiente.
    PistaFinalizada(Pista),
    /// Arranco la mezcla hacia esta pista, que desde ahora es la actual.
    TransicionIniciada(Pista),
    /// Termino el fundido de salida y la reproduccion quedo detenida.
    Detenida,
    /// No se pudo decodificar una pista (archivo danado, formato raro): su
    /// voz termina sola y la secuencia sigue con la siguiente.
    Fallo(String),
}

struct FundidoDeSalida {
    inicio: Instant,
    segundos: f64,
}

impl FundidoDeSalida {
    fn progreso(&self) -> f64 {
        (self.inicio.elapsed().as_secs_f64() / self.segundos).clamp(0.0, 1.0)
    }
}

pub struct GestorDeReproduccion {
    motores: [Motor; 2],
    principal: usize,
    reproduciendo: bool,
    pausado: bool,
    pista_actual: Option<Pista>,
    duracion_total: Option<f64>,
    volumen: f64,
    // Mayor que cero mientras hay un crossfade en curso.
    segundos_de_transicion: f64,
    // Posicion de la entrante en que empezo el crossfade en curso, y su curva.
    inicio_de_transicion: f64,
    curva: Curva,
    // Descripcion de la ultima mezcla iniciada.
    detalle_de_transicion: String,
    // Ganancia propia de cada pista (nivelacion); 1 si no figura.
    ganancias: HashMap<Pista, f64>,
    // Recortes (inicial, final) de cada pista; ninguno si no figura.
    recortes: HashMap<Pista, (f64, f64)>,
    // Duracion ya conocida de cada pista, para no sondear el archivo al cargarla.
    duraciones: HashMap<Pista, f64>,
    // Pista para la que fallo la carga de la siguiente; no se reintenta.
    transicion_descartada: Option<Pista>,
    fundido_de_salida: Option<FundidoDeSalida>,
    // Factor de la pista en curso mientras recorre su outro, antes de la
    // mezcla (1 fuera de ese tramo).
    atenuacion_previa: f64,
    // Factor con que la saliente entro a la mezcla en curso.
    salida_al_mezclar: f64,
}

impl GestorDeReproduccion {
    pub fn new(motor_a: Motor, motor_b: Motor) -> Self {
        Self {
            motores: [motor_a, motor_b],
            principal: 0,
            reproduciendo: false,
            pausado: false,
            pista_actual: None,
            duracion_total: None,
            volumen: 1.0,
            segundos_de_transicion: 0.0,
            inicio_de_transicion: 0.0,
            curva: Curva::Lineal,
            detalle_de_transicion: String::new(),
            ganancias: HashMap::new(),
            recortes: HashMap::new(),
            duraciones: HashMap::new(),
            transicion_descartada: None,
            fundido_de_salida: None,
            atenuacion_previa: 1.0,
            salida_al_mezclar: 1.0,
        }
    }

    fn principal(&self) -> &Motor {
        &self.motores[self.principal]
    }

    fn principal_mut(&mut self) -> &mut Motor {
        &mut self.motores[self.principal]
    }

    fn secundario_mut(&mut self) -> &mut Motor {
        &mut self.motores[1 - self.principal]
    }

    /// Carga y reproduce una pista desde su inicio (despues del recorte
    /// inicial), liberando por completo la anterior.
    pub fn reproducir(&mut self, pista: &Pista) -> Result<(), PistaInvalida> {
        self.detener();
        let duracion = self.duraciones.get(pista).copied();
        self.principal_mut().cargar(pista, duracion)?;
        let inicio = self.recortes_de(pista).0;
        if inicio > 0.0 {
            self.principal_mut().buscar(inicio)?;
        }
        self.principal_mut().reproducir()?;

        self.transicion_descartada = None;
        self.reproduciendo = true;
        self.pausado = false;
        self.pista_actual = Some(pista.clone());
        self.duracion_total = self.principal().duracion();
        self.aplicar_volumenes();
        Ok(())
    }

    /// Pasa a otra pista mezclandola durante los segundos de la transicion:
    /// la actual baja mientras la nueva sube. Sin musica sonando, o con 0
    /// segundos, la nueva arranca directamente.
    pub fn mezclar_hacia(&mut self, transicion: Transicion) -> Result<(), PistaInvalida> {
        if !self.reproduciendo || transicion.segundos <= 0.0 {
            return self.reproducir(&transicion.siguiente);
        }
        self.fundido_de_salida = None;
        self.cancelar_transicion();
        let segundos = transicion.segundos;
        self.arrancar_transicion(transicion, segundos, None)
    }

    /// Fija la ganancia propia de cada pista (la nivelacion de volumen); las
    /// que no figuran quedan en 1.
    pub fn definir_ganancias(&mut self, ganancias: HashMap<Pista, f64>) {
        self.ganancias = ganancias;
        self.aplicar_volumenes();
    }

    /// Fija los recortes (inicial, final) de cada pista; las que no figuran
    /// suenan enteras.
    pub fn definir_recortes(&mut self, recortes: HashMap<Pista, (f64, f64)>) {
        self.recortes = recortes;
    }

    /// Fija la duracion ya conocida de cada pista (de su ficha); las que no
    /// figuran se sondean al cargarlas.
    pub fn definir_duraciones(&mut self, duraciones: HashMap<Pista, f64>) {
        self.duraciones = duraciones;
    }

    fn recortes_de(&self, pista: &Pista) -> (f64, f64) {
        self.recortes.get(pista).copied().unwrap_or((0.0, 0.0))
    }

    /// Descripcion de la ultima mezcla iniciada (vacia si fue una simple).
    pub fn detalle_de_transicion(&self) -> &str {
        &self.detalle_de_transicion
    }

    /// Pausa al instante, sin fundido; tambien interrumpe un fundido de
    /// salida, que al reanudar vuelve al volumen pleno.
    pub fn pausar(&mut self) {
        if self.reproduciendo {
            self.fundido_de_salida = None;
            self.principal_mut().pausar();
            if self.hay_transicion_en_curso() {
                self.secundario_mut().pausar();
            }
            self.reproduciendo = false;
            self.pausado = true;
            self.aplicar_volumenes();
        }
    }

    pub fn reanudar(&mut self) -> Result<(), PistaInvalida> {
        if self.pausado {
            self.principal_mut().reproducir()?;
            if self.hay_transicion_en_curso() {
                self.secundario_mut().reproducir()?;
            }
            self.reproduciendo = true;
            self.pausado = false;
        }
        Ok(())
    }

    /// Detiene la reproduccion actual y libera los recursos de los motores.
    pub fn detener(&mut self) {
        self.fundido_de_salida = None;
        self.atenuacion_previa = 1.0;
        self.cancelar_transicion();
        self.principal_mut().detener();
        self.reproduciendo = false;
        self.pausado = false;
        self.duracion_total = None;
    }

    /// Detiene bajando la musica durante los segundos indicados. Sin musica
    /// sonando, con 0 segundos o si ya habia un fundido en curso, detiene al
    /// instante. Devuelve true si quedo un fundido en curso.
    pub fn detener_con_fundido(&mut self, segundos: f64) -> bool {
        if !self.reproduciendo || segundos <= 0.0 || self.fundido_de_salida.is_some() {
            self.detener();
            return false;
        }
        self.cancelar_transicion();
        self.fundido_de_salida = Some(FundidoDeSalida { inicio: Instant::now(), segundos });
        true
    }

    pub fn esta_deteniendo(&self) -> bool {
        self.fundido_de_salida.is_some()
    }

    /// Salta a una posicion de la pista actual. Si habia un crossfade o un
    /// fundido de salida en curso se corta y la actual queda a volumen pleno.
    pub fn buscar(&mut self, segundos: f64) -> Result<(), PistaInvalida> {
        if self.principal().hay_pista_cargada() && self.principal().duracion().is_some() {
            self.fundido_de_salida = None;
            self.atenuacion_previa = 1.0;
            self.cancelar_transicion();
            self.principal_mut().buscar(segundos)?;
        }
        Ok(())
    }

    pub fn ajustar_volumen(&mut self, nivel: f64) {
        self.volumen = nivel.clamp(0.0, 1.0);
        self.aplicar_volumenes();
    }

    pub fn tiempo_transcurrido(&self) -> f64 {
        self.principal().posicion_actual()
    }

    /// Frame de la salida en que sonara esa posicion de la pista actual.
    pub fn frame_de_salida_en(&self, posicion: f64) -> Option<u64> {
        self.principal().frame_de_salida_en(posicion)
    }

    pub fn duracion_total(&self) -> Option<f64> {
        self.duracion_total
    }

    pub fn hay_pista_cargada(&self) -> bool {
        self.principal().hay_pista_cargada()
    }

    pub fn esta_reproduciendo(&self) -> bool {
        self.reproduciendo
    }

    pub fn esta_pausado(&self) -> bool {
        self.pausado
    }

    pub fn hay_transicion_en_curso(&self) -> bool {
        self.segundos_de_transicion > 0.0
    }

    pub fn pista_actual(&self) -> Option<&Pista> {
        self.pista_actual.as_ref()
    }

    /// Se ejecuta en el hilo de la interfaz cada pocos milisegundos: detecta
    /// el fin de las pistas, avanza el crossfade o el fundido en curso, o
    /// decide si es momento de empezar una mezcla. La transicion se consulta
    /// en cada ciclo, asi que siempre refleja la lista vigente.
    pub fn ciclo(&mut self, transicion_desde: impl Fn(&Pista) -> Option<Transicion>) -> Vec<EventoDeReproduccion> {
        let mut eventos: Vec<EventoDeReproduccion> =
            self.motores.iter().filter_map(Motor::tomar_error).map(EventoDeReproduccion::Fallo).collect();
        // El fin de la pista que estaba saliendo en un crossfade no avanza la
        // secuencia, porque la siguiente ya esta sonando; tampoco el de una
        // pista que se estaba deteniendo con fundido.
        for indice in 0..self.motores.len() {
            if self.motores[indice].tomar_fin() && indice == self.principal {
                if self.fundido_de_salida.is_some() {
                    self.detener();
                    eventos.push(EventoDeReproduccion::Detenida);
                    continue;
                }
                self.reproduciendo = false;
                self.pausado = false;
                if let Some(pista) = self.pista_actual.clone() {
                    eventos.push(EventoDeReproduccion::PistaFinalizada(pista));
                }
            }
        }
        if let Some(fundido) = &self.fundido_de_salida {
            if fundido.progreso() >= 1.0 {
                self.detener();
                eventos.push(EventoDeReproduccion::Detenida);
            } else {
                self.aplicar_volumenes();
            }
            return eventos;
        }
        if let Some(evento) = self.evaluar_mezcla(transicion_desde) {
            eventos.push(evento);
        } else if let Some(pista) = self.llego_al_fin() {
            eventos.push(EventoDeReproduccion::PistaFinalizada(pista));
        }
        eventos
    }

    /// Termina la pista actual si llego a su recorte final sin mezclar con
    /// otra, como si se hubiera acabado el archivo.
    fn llego_al_fin(&mut self) -> Option<Pista> {
        if !self.reproduciendo || self.hay_transicion_en_curso() {
            return None;
        }
        let pista = self.pista_actual.clone()?;
        let recorte = self.recortes_de(&pista).1;
        let duracion = self.principal().duracion()?;
        if recorte <= 0.0 || self.principal().posicion_actual() < duracion - recorte {
            return None;
        }
        self.principal_mut().detener();
        self.reproduciendo = false;
        self.pausado = false;
        Some(pista)
    }

    fn evaluar_mezcla(&mut self, transicion_desde: impl Fn(&Pista) -> Option<Transicion>) -> Option<EventoDeReproduccion> {
        if !self.reproduciendo {
            return None;
        }
        if self.hay_transicion_en_curso() {
            if self.progreso_de_transicion() >= 1.0 {
                self.cancelar_transicion();
            } else {
                self.aplicar_volumenes();
            }
            return None;
        }

        let actual = self.pista_actual.clone()?;
        let duracion = self.principal().duracion()?;
        let transicion = if self.transicion_descartada.as_ref() == Some(&actual) {
            None
        } else {
            transicion_desde(&actual).filter(|candidata| candidata.segundos > 0.0)
        };
        let Some(transicion) = transicion else {
            self.fijar_atenuacion_previa(1.0);
            return None;
        };
        let posicion = self.principal().posicion_actual();
        let fin = (duracion - transicion.recorte_final).max(0.0);
        // Posicion de la actual en que debe arrancar la siguiente.
        let momento = fin - transicion.segundos + transicion.desfase;
        // Desde el inicio de la outro, la actual ya empieza a bajar.
        let inicio_de_la_outro = (duracion - transicion.outro_saliente).min(momento);
        let natural = transicion.curva.saliente_natural();
        self.fijar_atenuacion_previa(atenuacion_previa(posicion, inicio_de_la_outro, momento, natural));
        let anticipacion = if transicion.sincronizada { ANTICIPACION_DE_SINCRONIZACION } else { 0.0 };
        if posicion < momento - anticipacion {
            return None;
        }
        // Si todavia se llega, la siguiente se programa en el frame exacto;
        // si no, arranca ya y la mezcla dura lo que falta.
        let programada = if transicion.sincronizada && posicion < momento {
            self.principal().frame_de_salida_en(momento)
        } else {
            None
        };
        let superposicion = if programada.is_some() { fin - momento } else { fin - posicion };
        if superposicion < RESTO_MINIMO_PARA_MEZCLAR {
            return None;
        }
        let siguiente = transicion.siguiente.clone();
        log::info!(
            "Mezcla hacia {} desde {:.2} s: {:.2} s de superposicion, {}",
            siguiente.nombre(),
            transicion.inicio,
            superposicion,
            programada.map_or_else(|| "arranca ya".to_string(), |frame| format!("programada en el frame {frame} (desfase {:+.3} s)", transicion.desfase))
        );
        match self.arrancar_transicion(transicion, superposicion, programada) {
            Ok(()) => Some(EventoDeReproduccion::TransicionIniciada(siguiente)),
            Err(error) => {
                // La pista actual termina sola y la secuencia normal informara el error.
                log::warn!("No se pudo iniciar la mezcla hacia {}: {error}", siguiente.nombre());
                self.transicion_descartada = self.pista_actual.clone();
                None
            }
        }
    }

    fn fijar_atenuacion_previa(&mut self, factor: f64) {
        if (factor - self.atenuacion_previa).abs() > 1e-4 {
            self.atenuacion_previa = factor;
            self.aplicar_volumenes();
        }
    }

    /// Arranca la entrante en el otro motor y empieza el crossfade de
    /// `segundos`. Con `programada`, la entrante suena desde ese frame de la
    /// salida; si no, enseguida.
    fn arrancar_transicion(
        &mut self,
        transicion: Transicion,
        segundos: f64,
        programada: Option<u64>,
    ) -> Result<(), PistaInvalida> {
        let entrante = 1 - self.principal;
        let duracion = self.duraciones.get(&transicion.siguiente).copied();
        let motor = &mut self.motores[entrante];
        motor.detener();
        let arranque = motor.cargar(&transicion.siguiente, duracion).and_then(|_| {
            motor.definir_volumen(0.0);
            motor.buscar(transicion.inicio)?;
            motor.reproducir_en(programada)
        });
        if let Err(error) = arranque {
            motor.detener();
            return Err(error);
        }

        self.principal = entrante;
        let duracion_entrante = self.principal().duracion();
        let inicio = self.principal().posicion_actual();
        self.segundos_de_transicion =
            duracion_entrante.map_or(segundos, |duracion| segundos.min(duracion - inicio)).max(f64::EPSILON);
        self.inicio_de_transicion = inicio;
        self.curva = transicion.curva;
        // La saliente sigue bajando desde donde la dejo su outro.
        self.salida_al_mezclar = self.atenuacion_previa;
        self.atenuacion_previa = 1.0;
        self.detalle_de_transicion = transicion.detalle;
        self.transicion_descartada = None;
        self.pista_actual = Some(transicion.siguiente);
        self.duracion_total = duracion_entrante;
        self.aplicar_volumenes();
        Ok(())
    }

    /// Termina el crossfade: detiene la pista que salia y deja la actual a
    /// volumen pleno.
    fn cancelar_transicion(&mut self) {
        if self.hay_transicion_en_curso() {
            self.secundario_mut().detener();
            self.segundos_de_transicion = 0.0;
        }
        self.aplicar_volumenes();
    }

    fn progreso_de_transicion(&self) -> f64 {
        ((self.principal().posicion_actual() - self.inicio_de_transicion) / self.segundos_de_transicion).clamp(0.0, 1.0)
    }

    /// Ganancia propia de la pista cargada en un motor.
    fn ganancia_de(&self, motor: usize) -> f64 {
        self.motores[motor].pista().and_then(|pista| self.ganancias.get(pista)).copied().unwrap_or(1.0)
    }

    fn aplicar_volumenes(&mut self) {
        let salida = self.fundido_de_salida.as_ref().map_or(1.0, |fundido| 1.0 - fundido.progreso());
        let nivel = self.volumen * salida;
        let (principal, secundario) = (self.principal, 1 - self.principal);
        if self.hay_transicion_en_curso() {
            let (entra, sale) = self.curva.factores(self.progreso_de_transicion());
            self.motores[principal].definir_volumen(nivel * entra * self.ganancia_de(principal));
            self.motores[secundario].definir_volumen(nivel * sale * self.salida_al_mezclar * self.ganancia_de(secundario));
        } else {
            self.principal().definir_volumen(nivel * self.atenuacion_previa * self.ganancia_de(principal));
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn decibeles(factor: f64) -> f64 {
        20.0 * factor.log10()
    }

    #[test]
    fn la_saliente_cae_cada_vez_mas_rapido() {
        let curva = Curva::Potencia { saliente_natural: false, entrante_natural: false };
        let (entra, sale) = curva.factores(0.5);
        assert!((decibeles(sale) + 12.04).abs() < 0.01, "a mitad de la mezcla, -12 dB");
        assert!((entra - (0.5 * FRAC_PI_2).sin()).abs() < 1e-9);
        assert!(decibeles(curva.factores(0.75).1) < -24.0);
        assert_eq!(curva.factores(0.0).1, 1.0);
        assert_eq!(curva.factores(1.0).1, 0.0);
        // Siempre baja, y la entrante siempre sube.
        let pasos: Vec<(f64, f64)> = (0..=20).map(|paso| curva.factores(f64::from(paso) / 20.0)).collect();
        assert!(pasos.windows(2).all(|par| par[1].1 < par[0].1 && par[1].0 > par[0].0));
        assert_eq!(Curva::Lineal.factores(0.5), (0.5, 0.25));
    }

    #[test]
    fn un_fundido_natural_solo_se_acompana() {
        let curva = Curva::Potencia { saliente_natural: true, entrante_natural: false };
        let (entra, sale) = curva.factores(0.7);
        assert!((sale - 0.3).abs() < 1e-9);
        assert!(entra > 0.8 && entra < 1.0);
        let curva = Curva::Potencia { saliente_natural: false, entrante_natural: true };
        assert_eq!(curva.factores(0.0).0, 1.0);
    }

    #[test]
    fn la_outro_baja_antes_de_la_mezcla() {
        // Outro desde 100 s y mezcla desde 110 s.
        assert_eq!(atenuacion_previa(95.0, 100.0, 110.0, false), 1.0);
        assert!((decibeles(atenuacion_previa(105.0, 100.0, 110.0, false)) + 3.0).abs() < 1e-9);
        assert!((decibeles(atenuacion_previa(110.0, 100.0, 110.0, false)) + 6.0).abs() < 1e-9);
        assert!((decibeles(atenuacion_previa(115.0, 100.0, 110.0, false)) + 6.0).abs() < 1e-9);
        // Con fundido natural, o si la outro no empieza antes que la mezcla, no se toca.
        assert_eq!(atenuacion_previa(105.0, 100.0, 110.0, true), 1.0);
        assert_eq!(atenuacion_previa(105.0, 110.0, 110.0, false), 1.0);
    }
}
