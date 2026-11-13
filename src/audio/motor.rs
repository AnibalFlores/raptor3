//! Motor de reproduccion: decodifica una pista en su propio hilo y la emite
//! por una voz del mezclador.
//!
//! La busqueda reinicia el hilo desde la posicion pedida. La posicion se mide
//! con los frames que el mezclador ya entrego a la salida, no con los que el
//! decodificador escribio por adelantado en el buffer.
//!
//! Abrir el archivo y buscar la posicion se hace en el hilo de decodificacion,
//! no en el que pide reproducir (el de la interfaz): con discos lentos o
//! archivos sin indice puede tardar decenas de milisegundos. Si falla, el
//! hilo deja el motivo para `tomar_error` y la voz termina sin sonar.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

use super::decodificador::Decodificador;
use super::mezclador::Voz;
use super::prioridad::{PrioridadDeHilo, fijar_prioridad_del_hilo_actual};
use crate::dominio::{Pista, PistaInvalida};

// Un segundo de buffer entre el decodificador y la salida: margen contra
// cortes cuando la maquina esta ocupada. No demora el arranque (la voz suena
// en cuanto hay audio) ni la posicion, que se mide con lo ya emitido.
const SEGUNDOS_DE_BUFFER: f64 = 1.0;
const FRAMES_POR_BLOQUE_DE_REMUESTREO: usize = 1024;
const ESPERA_CON_BUFFER_LLENO: Duration = Duration::from_millis(5);
// Ganancia maxima de una voz: +6 dB, para la nivelacion de volumen.
pub const GANANCIA_MAXIMA: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstadoDeReproduccion {
    /// No hay audio en curso.
    Detenido,
    /// El audio se esta emitiendo hacia la salida.
    Reproduciendo,
    /// La emision esta interrumpida pero se conserva la posicion.
    Pausado,
}

pub struct Motor {
    voz: Arc<Voz>,
    frecuencia_de_salida: u32,
    pista: Option<Pista>,
    duracion: Option<f64>,
    posicion_base: f64,
    estado: EstadoDeReproduccion,
    cancelar_hilo: Option<Arc<AtomicBool>>,
    // Motivo por el que el hilo en curso no pudo abrir o posicionar el
    // archivo; uno nuevo por arranque, para no recibir el de uno anterior.
    error_del_hilo: Arc<Mutex<Option<String>>>,
    // Frame del reloj de salida en que debe arrancar la proxima reproduccion.
    inicio_programado: Option<u64>,
}

impl Motor {
    pub fn new(voz: Arc<Voz>, frecuencia_de_salida: u32) -> Self {
        Self {
            voz,
            frecuencia_de_salida,
            pista: None,
            duracion: None,
            posicion_base: 0.0,
            estado: EstadoDeReproduccion::Detenido,
            cancelar_hilo: None,
            error_del_hilo: Arc::default(),
            inicio_programado: None,
        }
    }

    /// Deja lista una pista para su reproduccion. Con la duracion ya conocida
    /// (de la ficha del tema) no se vuelve a sondear el archivo, que puede
    /// obligar a leerlo entero.
    pub fn cargar(&mut self, pista: &Pista, duracion_conocida: Option<f64>) -> Result<(), PistaInvalida> {
        self.detener_interno();
        if !pista.archivo().is_file() {
            return Err(PistaInvalida(tr!("El archivo no existe o no es accesible: {}", pista.nombre())));
        }
        self.duracion = match duracion_conocida.filter(|duracion| duracion.is_finite() && *duracion > 0.0) {
            Some(duracion) => Some(duracion),
            None => Decodificador::sondear_duracion(pista.archivo()).map_err(PistaInvalida)?,
        };
        self.pista = Some(pista.clone());
        self.posicion_base = 0.0;
        Ok(())
    }

    /// Inicia la reproduccion desde la posicion actual, o reanuda si estaba pausada.
    pub fn reproducir(&mut self) -> Result<(), PistaInvalida> {
        match self.estado {
            _ if self.pista.is_none() => Ok(()),
            EstadoDeReproduccion::Reproduciendo => Ok(()),
            EstadoDeReproduccion::Pausado => {
                self.con_voz(|estado| estado.sonando = true);
                self.estado = EstadoDeReproduccion::Reproduciendo;
                Ok(())
            }
            EstadoDeReproduccion::Detenido => self.iniciar(self.posicion_base),
        }
    }

    /// Como `reproducir`, pero si estaba detenido el audio arranca en el frame
    /// indicado del reloj de salida (None: enseguida).
    pub fn reproducir_en(&mut self, frame_de_salida: Option<u64>) -> Result<(), PistaInvalida> {
        self.inicio_programado = frame_de_salida;
        let resultado = self.reproducir();
        self.inicio_programado = None;
        resultado
    }

    /// Frame del reloj de salida en que sonara la posicion indicada (segundos)
    /// de la pista, si esta sonando. Sirve para programar otra voz.
    pub fn frame_de_salida_en(&self, posicion: f64) -> Option<u64> {
        if self.estado != EstadoDeReproduccion::Reproduciendo {
            return None;
        }
        let (referencia, emitidos) =
            self.voz.estado.lock().ok().and_then(|estado| estado.referencia.map(|referencia| (referencia, estado.frames_emitidos)))?;
        let objetivo = ((posicion - self.posicion_base) * self.frecuencia_de_salida as f64).round() as i64;
        u64::try_from(referencia as i64 + objetivo - emitidos as i64).ok()
    }

    /// Pista cargada, si hay.
    pub fn pista(&self) -> Option<&Pista> {
        self.pista.as_ref()
    }

    /// Pausa la emision conservando la posicion.
    pub fn pausar(&mut self) {
        if self.estado == EstadoDeReproduccion::Reproduciendo {
            self.con_voz(|estado| estado.sonando = false);
            self.estado = EstadoDeReproduccion::Pausado;
        }
    }

    /// Detiene la emision y libera el hilo de decodificacion.
    pub fn detener(&mut self) {
        self.detener_interno();
        self.posicion_base = 0.0;
    }

    /// Salta a una posicion dentro de la pista cargada.
    pub fn buscar(&mut self, segundos: f64) -> Result<(), PistaInvalida> {
        let Some(duracion) = self.duracion.filter(|_| self.pista.is_some()) else {
            return Ok(());
        };
        let estaba_reproduciendo = self.estado == EstadoDeReproduccion::Reproduciendo;
        let destino = segundos.clamp(0.0, duracion);
        self.detener_interno();
        self.posicion_base = destino;
        if estaba_reproduciendo {
            self.iniciar(destino)?;
        }
        Ok(())
    }

    /// Marca la voz como musica: pasa por el atenuador comun.
    pub fn definir_atenuable(&self, atenuable: bool) {
        self.voz.definir_atenuable(atenuable);
    }

    /// Fija el volumen objetivo (0 a GANANCIA_MAXIMA); la salida lo alcanza
    /// de forma suave.
    pub fn definir_volumen(&self, nivel: f64) {
        self.voz.definir_volumen(nivel.clamp(0.0, GANANCIA_MAXIMA) as f32);
    }

    /// Duracion total de la pista cargada, si se conoce.
    pub fn duracion(&self) -> Option<f64> {
        self.duracion
    }

    /// Posicion transcurrida de la pista cargada, en segundos.
    pub fn posicion_actual(&self) -> f64 {
        let mut segundos = self.posicion_base;
        if self.estado != EstadoDeReproduccion::Detenido {
            let frames = self.voz.estado.lock().map(|estado| estado.frames_emitidos).unwrap_or(0);
            segundos += frames as f64 / self.frecuencia_de_salida as f64;
        }
        if let Some(duracion) = self.duracion {
            segundos = segundos.min(duracion);
        }
        segundos.max(0.0)
    }

    pub fn hay_pista_cargada(&self) -> bool {
        self.pista.is_some()
    }

    /// Motivo por el que no se pudo empezar a decodificar la pista, una sola
    /// vez. La voz termina igual (`tomar_fin`).
    pub fn tomar_error(&self) -> Option<String> {
        self.error_del_hilo.lock().ok().and_then(|mut error| error.take())
    }

    /// Indica, una sola vez, que la pista termino naturalmente desde la
    /// ultima consulta. El motor queda detenido y al principio.
    pub fn tomar_fin(&mut self) -> bool {
        let termino = self.con_voz(|estado| {
            let termino = std::mem::take(&mut estado.termino);
            if termino {
                estado.consumidor = None;
            }
            termino
        });
        if termino && self.estado != EstadoDeReproduccion::Detenido {
            self.estado = EstadoDeReproduccion::Detenido;
            self.posicion_base = 0.0;
            self.cancelar_hilo = None;
            return true;
        }
        false
    }

    fn iniciar(&mut self, desde: f64) -> Result<(), PistaInvalida> {
        let pista = self.pista.clone().expect("iniciar requiere una pista cargada");
        let capacidad = ((self.frecuencia_de_salida as f64 * SEGUNDOS_DE_BUFFER) as usize).max(1024) * 2;
        let (productor, consumidor) = rtrb::RingBuffer::new(capacidad);
        let cancelar = Arc::new(AtomicBool::new(false));
        let fin_de_datos = Arc::new(AtomicBool::new(false));

        self.con_voz(|estado| {
            estado.consumidor = Some(consumidor);
            estado.fin_de_datos = fin_de_datos.clone();
            estado.frames_emitidos = 0;
            estado.termino = false;
            estado.ganancia = self.voz.volumen();
            estado.sonando = true;
            estado.inicio_programado = self.inicio_programado;
            estado.referencia = None;
        });

        let error_del_hilo = Arc::new(Mutex::new(None));
        self.error_del_hilo = error_del_hilo.clone();
        let pedido = PedidoDeDecodificacion {
            archivo: pista.archivo().to_path_buf(),
            desde,
            productor,
            cancelar: cancelar.clone(),
            frecuencia_de_salida: self.frecuencia_de_salida,
        };
        let nombre = pista.nombre();
        thread::Builder::new()
            .name("decodificacion-audio".into())
            .spawn(move || {
                fijar_prioridad_del_hilo_actual(PrioridadDeHilo::Audio);
                match pedido.abrir() {
                    Ok(hilo) => hilo.ejecutar(&nombre),
                    Err(motivo) => {
                        log::warn!("{motivo}");
                        if let Ok(mut error) = error_del_hilo.lock() {
                            *error = Some(motivo);
                        }
                    }
                }
                fin_de_datos.store(true, Ordering::Release);
            })
            .map_err(|error| PistaInvalida(tr!("No se pudo iniciar el hilo de audio: {error}", error = error)))?;

        self.cancelar_hilo = Some(cancelar);
        self.estado = EstadoDeReproduccion::Reproduciendo;
        Ok(())
    }

    fn detener_interno(&mut self) {
        self.estado = EstadoDeReproduccion::Detenido;
        if let Some(cancelar) = self.cancelar_hilo.take() {
            cancelar.store(true, Ordering::Release);
        }
        self.con_voz(|estado| {
            estado.consumidor = None;
            estado.sonando = false;
            estado.termino = false;
        });
    }

    fn con_voz<R>(&self, accion: impl FnOnce(&mut super::mezclador::EstadoDeVoz) -> R) -> R {
        let mut estado = self.voz.estado.lock().unwrap_or_else(|envenenado| envenenado.into_inner());
        accion(&mut estado)
    }
}

impl Drop for Motor {
    fn drop(&mut self) {
        self.detener_interno();
    }
}

/// Lo que necesita el hilo de decodificacion para empezar: el archivo se abre
/// y se posiciona ya dentro del hilo.
struct PedidoDeDecodificacion {
    archivo: PathBuf,
    desde: f64,
    productor: rtrb::Producer<f32>,
    cancelar: Arc<AtomicBool>,
    frecuencia_de_salida: u32,
}

impl PedidoDeDecodificacion {
    fn abrir(self) -> Result<HiloDeDecodificacion, String> {
        let nombre = crate::dominio::nombre_de_archivo(&self.archivo);
        let mut decodificador = Decodificador::abrir(&self.archivo)
            .map_err(|error| tr!("No se pudo iniciar la reproduccion de {nombre}: {error}", nombre = nombre, error = error))?;
        decodificador.buscar(self.desde).map_err(|error| format!("{nombre}: {error}"))?;
        Ok(HiloDeDecodificacion {
            decodificador,
            productor: self.productor,
            cancelar: self.cancelar,
            frecuencia_de_salida: self.frecuencia_de_salida,
        })
    }
}

/// Trabajo del hilo que decodifica, remuestrea a la frecuencia de la salida
/// y llena el buffer de la voz. Escribir bloquea mientras el buffer esta
/// lleno, lo que regula la velocidad de decodificacion (y frena durante una
/// pausa, en la que la voz no consume).
struct HiloDeDecodificacion {
    decodificador: Decodificador,
    productor: rtrb::Producer<f32>,
    cancelar: Arc<AtomicBool>,
    frecuencia_de_salida: u32,
}

impl HiloDeDecodificacion {
    fn ejecutar(mut self, nombre: &str) {
        let mut bloque = Vec::new();
        let mut remuestreo: Option<Remuestreo> = None;
        loop {
            if self.cancelado() {
                return;
            }
            bloque.clear();
            let Some(frecuencia) = self.decodificador.siguiente_bloque(&mut bloque) else {
                break;
            };
            if frecuencia == self.frecuencia_de_salida {
                if !self.escribir(&bloque) {
                    return;
                }
                continue;
            }
            if remuestreo.as_ref().is_none_or(|actual| actual.frecuencia_de_entrada != frecuencia) {
                match Remuestreo::new(frecuencia, self.frecuencia_de_salida) {
                    Ok(nuevo) => remuestreo = Some(nuevo),
                    Err(error) => {
                        log::warn!("No se puede remuestrear {nombre}: {error}");
                        return;
                    }
                }
            }
            let convertido = remuestreo.as_mut().expect("remuestreo creado").procesar(&bloque, false);
            if !self.escribir(&convertido) {
                return;
            }
        }
        if let Some(remuestreo) = remuestreo.as_mut() {
            let resto = remuestreo.procesar(&[], true);
            self.escribir(&resto);
        }
    }

    fn cancelado(&self) -> bool {
        self.cancelar.load(Ordering::Acquire)
    }

    /// Escribe todas las muestras, esperando mientras el buffer esta lleno.
    /// Devuelve false si la reproduccion se cancelo en el medio.
    fn escribir(&mut self, muestras: &[f32]) -> bool {
        let mut pendientes = muestras;
        while !pendientes.is_empty() {
            if self.cancelado() || self.productor.is_abandoned() {
                return false;
            }
            let libres = self.productor.slots().min(pendientes.len());
            if libres == 0 {
                thread::sleep(ESPERA_CON_BUFFER_LLENO);
                continue;
            }
            for &muestra in &pendientes[..libres] {
                let _ = self.productor.push(muestra);
            }
            pendientes = &pendientes[libres..];
        }
        true
    }
}

/// Conversion de frecuencia de muestreo (por ejemplo 44,1 kHz del archivo a
/// 48 kHz de la salida) con el remuestreador FFT de rubato.
struct Remuestreo {
    frecuencia_de_entrada: u32,
    remuestreador: Fft<f32>,
    pendiente: Vec<f32>,
    // Frames de retardo del filtro que todavia hay que descartar al principio,
    // para no correr la posicion.
    retardo_por_descartar: usize,
}

impl Remuestreo {
    fn new(entrada: u32, salida: u32) -> Result<Self, String> {
        let remuestreador = Fft::<f32>::new(
            entrada as usize,
            salida as usize,
            FRAMES_POR_BLOQUE_DE_REMUESTREO,
            2,
            FixedSync::Input,
        )
        .map_err(|error| error.to_string())?;
        let retardo_por_descartar = remuestreador.output_delay();
        Ok(Self { frecuencia_de_entrada: entrada, remuestreador, pendiente: Vec::new(), retardo_por_descartar })
    }

    /// Agrega muestras estereo intercaladas y devuelve las ya convertidas. Con
    /// `final_de_pista` se vacia lo pendiente, completando con silencio.
    fn procesar(&mut self, muestras: &[f32], final_de_pista: bool) -> Vec<f32> {
        self.pendiente.extend_from_slice(muestras);
        let mut salida = Vec::new();
        loop {
            let necesarios = self.remuestreador.input_frames_next();
            let disponibles = self.pendiente.len() / 2;
            let parcial = if disponibles >= necesarios {
                None
            } else if final_de_pista && disponibles > 0 {
                Some(disponibles)
            } else {
                break;
            };
            let usados = parcial.unwrap_or(necesarios);
            let mut entrada = self.pendiente[..usados * 2].to_vec();
            entrada.resize(necesarios * 2, 0.0);
            let Ok(adaptador) = InterleavedSlice::new(&entrada[..], 2, necesarios) else {
                break;
            };
            let indexado = parcial.map(|frames| Indexing::new().partial_len(frames));
            match self.remuestreador.process(&adaptador, indexado.as_ref()) {
                Ok(convertido) => {
                    let datos = convertido.take_data();
                    let frames = datos.len() / 2;
                    let descartar = self.retardo_por_descartar.min(frames);
                    self.retardo_por_descartar -= descartar;
                    salida.extend_from_slice(&datos[descartar * 2..]);
                }
                Err(error) => {
                    log::warn!("Fallo el remuestreo: {error}");
                    self.pendiente.clear();
                    break;
                }
            }
            self.pendiente.drain(..usados * 2);
        }
        salida
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::time::Instant;

    #[test]
    fn un_archivo_danado_falla_en_el_hilo_sin_trabar_al_que_pide_reproducir() {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-motor-{}", std::process::id()));
        std::fs::create_dir_all(&carpeta).unwrap();
        let archivo = carpeta.join("danado.mp3");
        std::fs::write(&archivo, b"esto no es audio").unwrap();
        let mut motor = Motor::new(Arc::new(Voz::new()), 48_000);
        // Con la duracion conocida no se sondea: cargar y reproducir no abren el archivo.
        motor.cargar(&Pista::new(&archivo), Some(10.0)).unwrap();
        assert_eq!(motor.duracion(), Some(10.0));
        motor.reproducir().unwrap();
        let limite = Instant::now() + Duration::from_secs(5);
        let motivo = loop {
            if let Some(motivo) = motor.tomar_error() {
                break motivo;
            }
            assert!(Instant::now() < limite, "el hilo no informo el error");
            thread::sleep(Duration::from_millis(10));
        };
        assert!(motivo.contains("danado.mp3"), "{motivo}");
        assert_eq!(motor.tomar_error(), None, "se informa una sola vez");
        let _ = std::fs::remove_dir_all(&carpeta);
    }
}
