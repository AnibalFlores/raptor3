//! Efectos de la botonera, emitidos por un tercer motor por encima de la
//! musica.
//!
//! En modo trabado el efecto tiene que sonar entero dentro de la ventana de
//! overlay de la pista en curso:
//! - antes de la ventana, queda en espera hasta que se abra;
//! - dentro de la ventana, suena si todavia cabe completo antes del fin;
//! - si ya no cabe (o la ventana paso), espera la ventana de la pista que
//!   suene despues.
//!
//! Si mientras espera cambia la pista, pasa a esperar la ventana de la nueva.
//! Una pista sin ventana deja sonar el efecto enseguida, igual que la musica
//! detenida. Un efecto nuevo reemplaza al que estaba esperando.
//!
//! Volumen del efecto y atenuacion de la musica: ver `mezcla_de_efectos`. El
//! efecto se programa en un frame exacto de la salida y la musica empieza a
//! bajar un instante antes en el atenuador comun; la vuelta tambien queda
//! programada para cuando el efecto deja de sonar, con una rampa suave. En
//! modo trabado, si la ventana esta por abrirse, el efecto se programa justo
//! en su comienzo.
//!
//! Si en el editor se fijaron a mano el inicio y el fin del archivo del
//! efecto, suena solo ese tramo (ver `RecorteDeEfecto`): arranca en el inicio
//! y al llegar al fin baja en un instante y se corta.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::GestorDeReproduccion;
use super::mezcla_de_efectos::{AjustesDeEfecto, MusicaAlrededor, PerfilDeEfecto, PlanDeEfecto, planificar};
use crate::audio::{Atenuador, Decodificador, FormaDeOnda, Motor};
use crate::dominio::{Pista, PistaInvalida, VentanaDeOverlay};

// Tiempo para abrir el archivo y llenar el buffer antes de que suene el
// efecto, ademas de la anticipacion de la atenuacion.
const MARGEN_DE_ARRANQUE: f64 = 0.06;
// Con la ventana a menos de esto, el efecto se programa en su comienzo exacto.
const PROGRAMAR_CON: f64 = 0.35;
// Vuelta de la musica cuando se corta un efecto a mano.
const RECUPERACION_AL_CORTAR: f64 = 0.4;
// Margen para no perder la ventana entre un ciclo y el siguiente (50 ms)
// cuando el efecto dura mas que la ventana y solo puede entrar al abrirse.
const TOLERANCIA_AL_ABRIR: f64 = 0.15;
// Antes del fin fijado el efecto empieza a bajar (la voz suaviza el cambio de
// volumen), para cortarlo sin chasquido.
const FUNDIDO_AL_RECORTAR: f64 = 0.15;
// Un fin fijado a menos de esto del final del archivo no obliga a cortar.
const MARGEN_DEL_FINAL: f64 = 0.01;

/// Tramo del archivo de un efecto que suena: desde el inicio fijado en el
/// editor hasta `antes_del_final` segundos antes del final del archivo, tal
/// como los guarda el editor (recorte inicial y final). El fin se resuelve
/// con la duracion del archivo al hacerlo sonar, asi que no hace falta que
/// la ficha la conozca (un efecto que nunca estuvo en la lista no la tiene).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RecorteDeEfecto {
    pub desde: f64,
    pub antes_del_final: f64,
}

impl RecorteDeEfecto {
    /// Posicion del archivo en que termina el tramo, si se recorto el final
    /// y se conoce la duracion.
    fn hasta(&self, duracion_del_archivo: Option<f64>) -> Option<f64> {
        if self.antes_del_final < MARGEN_DEL_FINAL {
            return None;
        }
        duracion_del_archivo.map(|duracion| (duracion - self.antes_del_final).max(self.desde))
    }

    /// Lo que dura el tramo, si se conoce la duracion del archivo.
    pub fn duracion(&self, duracion_del_archivo: Option<f64>) -> Option<f64> {
        duracion_del_archivo.map(|duracion| (duracion - self.antes_del_final - self.desde).max(0.0))
    }

    /// Perfil del tramo: el ultimo sonido se cuenta desde `desde` y no pasa
    /// del fin del tramo.
    fn perfil(&self, perfil: PerfilDeEfecto, duracion_del_archivo: Option<f64>) -> PerfilDeEfecto {
        let hasta = self.hasta(duracion_del_archivo);
        let fin_audible = perfil
            .fin_audible
            .map(|fin| hasta.map_or(fin, |hasta| fin.min(hasta)))
            .or(hasta)
            .map(|fin| (fin - self.desde).max(0.0));
        PerfilDeEfecto { fin_audible, ..perfil }
    }

    /// Posicion del archivo en que hay que cortar, si es antes de su final.
    fn corte(&self, duracion_del_archivo: Option<f64>) -> Option<f64> {
        self.hasta(duracion_del_archivo)
    }
}

/// Como se dispara un efecto de la botonera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModoDeOverlay {
    /// Suena apenas se pulsa el boton.
    Libre,
    /// Suena entero dentro de la ventana de overlay de la pista en curso.
    Trabada,
}

/// Que hacer con un efecto trabado segun la posicion de la pista.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    Sonar,
    /// Esperar a que se abra la ventana, dentro de estos segundos.
    Esperar(f64),
    /// La ventana ya paso o el efecto no cabe: esperar la pista siguiente.
    ProximaPista,
}

/// Sonoridad, pico y fin audible de un efecto segun su analisis; sin el, solo
/// la duracion.
fn perfil_del_analisis(archivo: &Path, analisis: Result<FormaDeOnda, String>) -> (PerfilDeEfecto, Option<f64>) {
    match analisis {
        Ok(onda) => {
            let duracion = onda.duracion();
            let fin_audible = duracion - onda.sugerir().silencio_final;
            let pico = onda.pico_real().unwrap_or_else(|| f64::from(onda.pico_maximo()));
            (PerfilDeEfecto { sonoridad: onda.sonoridad(), pico, fin_audible: Some(fin_audible) }, Some(duracion))
        }
        Err(error) => {
            log::warn!("No se pudo analizar el efecto {}: {error}", archivo.display());
            let duracion = Decodificador::sondear_duracion(archivo).ok().flatten();
            (PerfilDeEfecto { sonoridad: None, pico: 1.0, fin_audible: None }, duracion)
        }
    }
}

/// Regla del modo trabado: el efecto debe empezar dentro de la ventana y
/// terminar antes de su fin. Si dura mas que la ventana, solo puede entrar
/// al abrirse.
pub fn decidir(posicion: f64, ventana: Option<VentanaDeOverlay>, duracion_del_efecto: Option<f64>) -> Decision {
    let Some(ventana) = ventana else {
        return Decision::Sonar;
    };
    let ultimo_inicio = (ventana.fin() - duracion_del_efecto.unwrap_or(0.0)).max(ventana.inicio() + TOLERANCIA_AL_ABRIR);
    if posicion < ventana.inicio() {
        Decision::Esperar(ventana.inicio() - posicion)
    } else if posicion <= ultimo_inicio {
        Decision::Sonar
    } else {
        Decision::ProximaPista
    }
}

/// Como esta el efecto que espera, para mostrarlo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EstadoDeEspera {
    /// Segundos que faltan para que se abra la ventana de la pista en curso.
    Faltan(f64),
    /// Espera la ventana de la pista que suene despues.
    ProximaPista,
}

struct Espera {
    archivo: PathBuf,
    duracion: Option<f64>,
    /// Pista cuya ventana se espera.
    pista: Pista,
    /// La ventana de `pista` ya paso: se espera a que suene otra.
    a_la_proxima: bool,
}

/// Lo que el gestor necesita saber de la lista: la ventana de overlay de
/// cada pista, si la pista admite efectos trabados (los tipos sin mezcla
/// no), como suena la musica en un tramo (pista, desde, segundos) y que
/// tramo de cada efecto suena.
pub struct Entorno<'a> {
    pub ventana_de: &'a dyn Fn(&Pista) -> Option<VentanaDeOverlay>,
    pub admite_efectos: &'a dyn Fn(&Pista) -> bool,
    pub musica_de: &'a dyn Fn(&Pista, f64, f64) -> Option<MusicaAlrededor>,
    pub recorte_de: &'a dyn Fn(&Path) -> RecorteDeEfecto,
}

pub struct GestorDeOverlay {
    motor: Motor,
    atenuador: Arc<Atenuador>,
    carpeta_de_ondas: PathBuf,
    /// Perfil y duracion de cada efecto ya usado.
    perfiles: HashMap<PathBuf, (PerfilDeEfecto, Option<f64>)>,
    espera: Option<Espera>,
    sonando: Option<PathBuf>,
    ajustes: AjustesDeEfecto,
    // Efectos que siempre bajan la musica, aunque el panel diga que no (la
    // voz del anuncio HTH sobre su base).
    siempre_atenuados: HashSet<PathBuf>,
    ultimo_plan: Option<PlanDeEfecto>,
    // La vuelta de la musica no se pudo programar (no se conoce el final del
    // efecto): se hace cuando termina.
    volver_al_terminar: bool,
    // Posicion del archivo en que se corta el efecto que suena (su fin fijado).
    cortar_en: Option<f64>,
}

impl GestorDeOverlay {
    pub fn new(motor: Motor, atenuador: Arc<Atenuador>, carpeta_de_ondas: PathBuf) -> Self {
        Self {
            motor,
            atenuador,
            carpeta_de_ondas,
            perfiles: HashMap::new(),
            espera: None,
            sonando: None,
            ajustes: AjustesDeEfecto::default(),
            siempre_atenuados: HashSet::new(),
            ultimo_plan: None,
            volver_al_terminar: false,
            cortar_en: None,
        }
    }

    pub fn definir_ajustes(&mut self, ajustes: AjustesDeEfecto) {
        self.ajustes = ajustes;
    }

    /// Este efecto siempre baja la musica mientras suena.
    pub fn atenuar_siempre(&mut self, archivo: &Path) {
        self.siempre_atenuados.insert(archivo.to_path_buf());
    }

    /// Ajustes con que suena un efecto: los del panel. Uno que pide bajar la
    /// musica siempre (la voz del anuncio HTH sobre su base) la baja con el
    /// ajuste inteligente, segun la separacion elegida, para que la voz se
    /// entienda aunque la base suene fuerte.
    fn ajustes_para(&self, archivo: &Path) -> AjustesDeEfecto {
        if self.siempre_atenuados.contains(archivo) {
            return AjustesDeEfecto { atenuar: true, inteligente: true, ..self.ajustes };
        }
        self.ajustes
    }

    pub fn ajustes(&self) -> AjustesDeEfecto {
        self.ajustes
    }

    /// Plan del ultimo efecto que sono.
    pub fn ultimo_plan(&self) -> Option<PlanDeEfecto> {
        self.ultimo_plan
    }

    /// Sonoridad, pico y final de un efecto, del analisis de su forma de onda
    /// (de la cache, si ya se calculo), y su duracion.
    fn perfil(&mut self, archivo: &Path) -> (PerfilDeEfecto, Option<f64>) {
        if let Some(perfil) = self.perfiles.get(archivo) {
            return *perfil;
        }
        let perfil = perfil_del_analisis(archivo, FormaDeOnda::cargar_o_calcular(archivo, &self.carpeta_de_ondas));
        self.perfiles.insert(archivo.to_path_buf(), perfil);
        perfil
    }

    /// Vuelve a analizar un efecto cuyo archivo se reescribio (la voz del
    /// anuncio HTH), sin guardarlo en la cache de formas de onda: cambia en
    /// cada pase y solo la llenaria de restos.
    pub fn renovar_perfil(&mut self, archivo: &Path) {
        let perfil = perfil_del_analisis(archivo, FormaDeOnda::calcular(archivo));
        self.perfiles.insert(archivo.to_path_buf(), perfil);
    }

    /// Duracion de un efecto (de su analisis).
    pub fn duracion_del_efecto(&mut self, archivo: &Path) -> Option<f64> {
        self.perfil(archivo).1
    }

    /// Dispara un efecto. En modo libre suena ya; en modo trabado sigue la
    /// regla de la ventana de overlay de la pista en curso. Si la pista no
    /// admite efectos, el trabado espera a la siguiente que si los admita.
    pub fn disparar(
        &mut self,
        archivo: &Path,
        modo: ModoDeOverlay,
        musica: &GestorDeReproduccion,
        entorno: &Entorno,
    ) -> Result<(), PistaInvalida> {
        self.espera = None;
        let recorte = (entorno.recorte_de)(archivo);
        let duracion = recorte.duracion(self.perfil(archivo).1);
        let posicion = musica.tiempo_transcurrido();
        let pista = match (modo, pista_en_curso(musica)) {
            (ModoDeOverlay::Trabada, Some(pista)) => pista.clone(),
            (_, en_curso) => {
                let alrededor = en_curso.and_then(|pista| (entorno.musica_de)(pista, posicion, duracion.unwrap_or(0.0)));
                return self.emitir(archivo, None, alrededor, 1.0, recorte);
            }
        };
        let decision = if (entorno.admite_efectos)(&pista) {
            decidir(posicion, (entorno.ventana_de)(&pista), duracion)
        } else {
            Decision::ProximaPista
        };
        match decision {
            Decision::Sonar => {
                let alrededor = (entorno.musica_de)(&pista, posicion, duracion.unwrap_or(0.0));
                self.emitir(archivo, None, alrededor, 1.0, recorte)
            }
            decision => {
                let a_la_proxima = decision == Decision::ProximaPista;
                self.espera = Some(Espera { archivo: archivo.to_path_buf(), duracion, pista, a_la_proxima });
                Ok(())
            }
        }
    }

    /// Cancela el efecto en espera, si lo hay.
    pub fn cancelar_espera(&mut self) {
        self.espera = None;
    }

    /// Corta el efecto que suena, cancela el que esta en espera y devuelve
    /// la musica a su nivel con una rampa corta.
    pub fn detener(&mut self) {
        self.cancelar_espera();
        self.motor.detener();
        self.sonando = None;
        self.volver_al_terminar = false;
        self.cortar_en = None;
        self.atenuador.restituir(RECUPERACION_AL_CORTAR);
    }

    /// Hace sonar un archivo sin mirar ventanas (por ejemplo para probar un
    /// efecto en el editor de pistas): en el frame de salida `arranque` o lo
    /// antes posible, con la musica que lo rodea y solo su tramo `recorte`.
    /// `escala` multiplica la ganancia del plan (el volumen propio de la
    /// preescucha).
    pub fn sonar_ya(
        &mut self,
        archivo: &Path,
        arranque: Option<u64>,
        musica: Option<MusicaAlrededor>,
        escala: f64,
        recorte: RecorteDeEfecto,
    ) -> Result<(), PistaInvalida> {
        self.espera = None;
        self.emitir(archivo, arranque, musica, escala, recorte)
    }

    pub fn en_espera(&self) -> Option<&Path> {
        self.espera.as_ref().map(|espera| espera.archivo.as_path())
    }

    pub fn sonando(&self) -> Option<&Path> {
        self.sonando.as_deref()
    }

    pub fn estado_de_espera(
        &self,
        musica: &GestorDeReproduccion,
        ventana_de: &dyn Fn(&Pista) -> Option<VentanaDeOverlay>,
    ) -> Option<EstadoDeEspera> {
        let espera = self.espera.as_ref()?;
        if espera.a_la_proxima || musica.pista_actual() != Some(&espera.pista) {
            return Some(EstadoDeEspera::ProximaPista);
        }
        match decidir(musica.tiempo_transcurrido(), ventana_de(&espera.pista), espera.duracion) {
            Decision::Esperar(segundos) => Some(EstadoDeEspera::Faltan(segundos)),
            _ => Some(EstadoDeEspera::Faltan(0.0)),
        }
    }

    /// Se ejecuta en cada ciclo de la interfaz: registra el fin del efecto y
    /// libera el que espera si ya llego su momento.
    pub fn ciclo(&mut self, musica: &GestorDeReproduccion, entorno: &Entorno) -> Result<(), PistaInvalida> {
        self.registrar_fin()?;
        let Some(espera) = self.espera.as_mut() else {
            return Ok(());
        };
        // Sin musica no hay ventana que esperar: el efecto suena ya.
        let Some(actual) = pista_en_curso(musica) else {
            let archivo = self.espera.take().expect("hay espera").archivo;
            let recorte = (entorno.recorte_de)(&archivo);
            return self.emitir(&archivo, None, None, 1.0, recorte);
        };
        if *actual != espera.pista {
            // Cambio la pista: ahora cuenta la ventana de la nueva, salvo que
            // no admita efectos (sigue esperando a la que venga despues).
            espera.pista = actual.clone();
            espera.a_la_proxima = !(entorno.admite_efectos)(actual);
            if espera.a_la_proxima {
                return Ok(());
            }
        } else if espera.a_la_proxima {
            return Ok(());
        }
        let posicion = musica.tiempo_transcurrido();
        let ventana = (entorno.ventana_de)(actual);
        let duracion = espera.duracion.unwrap_or(0.0);
        match decidir(posicion, ventana, espera.duracion) {
            Decision::Sonar => {
                let alrededor = (entorno.musica_de)(actual, posicion, duracion);
                let archivo = self.espera.take().expect("hay espera").archivo;
                let recorte = (entorno.recorte_de)(&archivo);
                self.emitir(&archivo, None, alrededor, 1.0, recorte)
            }
            // Falta muy poco: se programa en el comienzo exacto de la ventana.
            Decision::Esperar(falta) if falta <= PROGRAMAR_CON => {
                let Some(inicio) = ventana.map(|ventana| ventana.inicio()) else {
                    return Ok(());
                };
                let Some(frame) = musica.frame_de_salida_en(inicio) else {
                    return Ok(());
                };
                let alrededor = (entorno.musica_de)(actual, inicio, duracion);
                let archivo = self.espera.take().expect("hay espera").archivo;
                let recorte = (entorno.recorte_de)(&archivo);
                self.emitir(&archivo, Some(frame), alrededor, 1.0, recorte)
            }
            Decision::Esperar(_) => Ok(()),
            Decision::ProximaPista => {
                espera.a_la_proxima = true;
                Ok(())
            }
        }
    }

    /// Registra el fin del efecto que sonaba (y devuelve la musica a su nivel
    /// si quedo pendiente) o el error que impidio reproducirlo. Es la parte
    /// del ciclo que no depende de la lista: alcanza para los efectos de
    /// prueba de la preescucha.
    pub fn registrar_fin(&mut self) -> Result<(), PistaInvalida> {
        if let Some(motivo) = self.motor.tomar_error() {
            self.sonando = None;
            self.cortar_en = None;
            return Err(PistaInvalida(motivo));
        }
        if self.motor.tomar_fin() {
            self.al_terminar_el_efecto();
        } else if let Some(corte) = self.cortar_en.filter(|_| self.sonando.is_some()) {
            let posicion = self.motor.posicion_actual();
            if posicion >= corte {
                self.motor.detener();
                self.al_terminar_el_efecto();
            } else if posicion >= corte - FUNDIDO_AL_RECORTAR {
                self.motor.definir_volumen(0.0);
            }
        }
        Ok(())
    }

    /// El efecto dejo de sonar (termino o llego a su fin fijado): la musica
    /// vuelve si su vuelta no habia quedado programada.
    fn al_terminar_el_efecto(&mut self) {
        self.sonando = None;
        self.cortar_en = None;
        if std::mem::take(&mut self.volver_al_terminar) {
            let recuperacion = self.ultimo_plan.map_or(self.ajustes.recuperacion, |plan| plan.recuperacion);
            self.atenuador.restituir(recuperacion);
        }
    }

    /// Hace sonar el efecto con su plan: la musica empieza a bajar un
    /// instante antes de `arranque` (o de lo antes posible) y su vuelta queda
    /// programada para cuando el efecto deja de sonar.
    fn emitir(
        &mut self,
        archivo: &Path,
        arranque: Option<u64>,
        musica: Option<MusicaAlrededor>,
        escala: f64,
        recorte: RecorteDeEfecto,
    ) -> Result<(), PistaInvalida> {
        let (perfil, duracion) = self.perfil(archivo);
        let perfil = recorte.perfil(perfil, duracion);
        let plan = planificar(&self.ajustes_para(archivo), &perfil, musica.as_ref());
        let frecuencia = f64::from(self.atenuador.frecuencia());
        let frames = |segundos: f64| (segundos * frecuencia).round() as u64;
        let lo_antes_posible = self.atenuador.reloj() + frames(MARGEN_DE_ARRANQUE + plan.anticipacion);
        let inicio = arranque.unwrap_or(lo_antes_posible).max(lo_antes_posible.saturating_sub(frames(MARGEN_DE_ARRANQUE / 2.0)));
        self.motor.detener();
        let resultado = self.motor.cargar(&Pista::new(archivo), duracion).and_then(|_| {
            if recorte.desde > 0.0 {
                self.motor.buscar(recorte.desde)?;
            }
            self.motor.definir_volumen(plan.ganancia * escala.max(0.0));
            self.motor.reproducir_en(Some(inicio))
        });
        match resultado {
            Ok(()) => {
                self.sonando = Some(archivo.to_path_buf());
                // Sin duracion del analisis vale la que sondeo el motor al cargar.
                self.cortar_en = recorte.corte(duracion.or(self.motor.duracion()));
                self.ultimo_plan = Some(plan);
                self.programar_atenuacion(&plan, inicio);
                log::info!("Efecto {} en el frame {inicio}: {}", archivo.display(), plan.descripcion());
                Ok(())
            }
            Err(error) => {
                log::warn!("No se pudo reproducir el efecto {}: {error}", archivo.display());
                self.sonando = None;
                Err(error)
            }
        }
    }

    /// Baja la musica `anticipacion` segundos antes de `inicio` y programa
    /// su vuelta para cuando el efecto deja de sonar.
    fn programar_atenuacion(&mut self, plan: &PlanDeEfecto, inicio: u64) {
        let frecuencia = f64::from(self.atenuador.frecuencia());
        let frames = |segundos: f64| (segundos * frecuencia).round() as u64;
        self.volver_al_terminar = false;
        if plan.nivel_de_musica >= 0.999 {
            self.atenuador.restituir(plan.recuperacion);
            return;
        }
        self.atenuador.limpiar();
        self.atenuador.programar(inicio.saturating_sub(frames(plan.anticipacion)), plan.nivel_de_musica, plan.anticipacion);
        match plan.volver_tras {
            Some(segundos) => self.atenuador.programar(inicio + frames(segundos), 1.0, plan.recuperacion),
            None => self.volver_al_terminar = true,
        }
    }
}

/// Pista que esta sonando o en pausa; una pausa mantiene la espera.
fn pista_en_curso(musica: &GestorDeReproduccion) -> Option<&Pista> {
    if !musica.esta_reproduciendo() && !musica.esta_pausado() {
        return None;
    }
    musica.pista_actual()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_recorte_acorta_la_duracion_y_el_fin_audible() {
        let completo = RecorteDeEfecto::default();
        assert_eq!(completo.duracion(Some(6.0)), Some(6.0));
        assert_eq!(completo.corte(Some(6.0)), None);
        // Inicio en 1.5 y fin 2 s antes del final de un archivo de 6 s.
        let tramo = RecorteDeEfecto { desde: 1.5, antes_del_final: 2.0 };
        assert_eq!(tramo.duracion(Some(6.0)), Some(2.5));
        assert_eq!(tramo.corte(Some(6.0)), Some(4.0));
        // Solo con el final recortado (el caso de un efecto que nunca estuvo
        // en la lista): el corte sale de la duracion del archivo.
        let solo_el_final = RecorteDeEfecto { desde: 0.0, antes_del_final: 1.55 };
        assert_eq!(solo_el_final.corte(Some(10.0)), Some(8.45));
        assert_eq!(solo_el_final.corte(None), None, "sin duracion no se sabe donde cortar");
        // El ultimo sonido se cuenta desde el inicio del tramo y no pasa del fin.
        let perfil = PerfilDeEfecto { sonoridad: None, pico: 1.0, fin_audible: Some(5.5) };
        assert_eq!(tramo.perfil(perfil, Some(6.0)).fin_audible, Some(2.5));
        let breve = PerfilDeEfecto { fin_audible: Some(3.0), ..perfil };
        assert_eq!(tramo.perfil(breve, Some(6.0)).fin_audible, Some(1.5));
        let sin_analisis = PerfilDeEfecto { fin_audible: None, ..perfil };
        assert_eq!(tramo.perfil(sin_analisis, Some(6.0)).fin_audible, Some(2.5));
        assert_eq!(RecorteDeEfecto { desde: 1.0, antes_del_final: 0.0 }.perfil(sin_analisis, Some(6.0)).fin_audible, None);
    }

    fn ventana(inicio: f64, fin: f64) -> Option<VentanaDeOverlay> {
        Some(VentanaDeOverlay::nueva(inicio, fin).unwrap())
    }

    #[test]
    fn sin_ventana_suena_ya() {
        assert_eq!(decidir(42.0, None, Some(3.0)), Decision::Sonar);
    }

    #[test]
    fn antes_de_la_ventana_espera_lo_que_falta() {
        assert_eq!(decidir(70.0, ventana(80.0, 105.0), Some(3.0)), Decision::Esperar(10.0));
    }

    #[test]
    fn dentro_de_la_ventana_suena_si_cabe_entero() {
        assert_eq!(decidir(80.0, ventana(80.0, 105.0), Some(3.0)), Decision::Sonar);
        assert_eq!(decidir(102.0, ventana(80.0, 105.0), Some(3.0)), Decision::Sonar);
        assert_eq!(decidir(102.5, ventana(80.0, 105.0), Some(3.0)), Decision::ProximaPista);
    }

    #[test]
    fn despues_de_la_ventana_espera_la_proxima_pista() {
        assert_eq!(decidir(110.0, ventana(80.0, 105.0), Some(1.0)), Decision::ProximaPista);
    }

    #[test]
    fn un_efecto_mas_largo_que_la_ventana_solo_entra_al_abrirse() {
        assert_eq!(decidir(80.05, ventana(80.0, 85.0), Some(10.0)), Decision::Sonar);
        assert_eq!(decidir(81.0, ventana(80.0, 85.0), Some(10.0)), Decision::ProximaPista);
    }

    #[test]
    fn sin_duracion_conocida_basta_con_estar_dentro() {
        assert_eq!(decidir(104.0, ventana(80.0, 105.0), None), Decision::Sonar);
    }
}
