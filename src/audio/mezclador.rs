//! Salida de audio unica: un flujo de cpal que suma las voces activas.
//!
//! Cada voz recibe PCM estereo a la frecuencia de la salida por un buffer
//! circular que llena el hilo de decodificacion de su motor. El callback de
//! audio aplica a cada voz su ganancia, suavizada de forma exponencial para
//! que crossfades, atenuaciones y cambios de volumen suenen sin escalones.
//!
//! El callback lleva un reloj de frames de salida. Una voz puede programarse
//! para arrancar en un frame exacto de ese reloj (asi la pista que entra en
//! una mezcla cae justo sobre los golpes de la que sale), y cada voz anota que
//! frame del reloj corresponde a su posicion para poder calcularlo.
//!
//! Las voces de musica pasan ademas por un atenuador comun: una envolvente
//! que baja y sube la musica mientras suena un efecto, con rampas programadas
//! en frames exactos del reloj (por ejemplo, empezar a bajar 40 ms antes del
//! efecto y volver con una rampa suave cuando termina). Al ser comun, una
//! mezcla que empieza durante el efecto ya entra atenuada.
//!
//! La suma pasa por un recortador suave: por debajo del umbral no cambia nada
//! y por encima se acerca a 1 sin el chasquido de un recorte brusco.
//!
//! Puede haber mas de un mezclador, cada uno en su salida: el principal en la
//! salida por defecto y otro, opcional, para la preescucha del editor en una
//! salida elegida (por ejemplo unos auriculares). Cada uno lleva su propio
//! reloj y su propio atenuador.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};

// Constante de tiempo de la ganancia: llega al 95 % del valor pedido en unos
// 150 ms.
const SEGUNDOS_DE_SUAVIZADO: f32 = 0.05;
// Desde este nivel la salida se comprime suavemente hacia 1.
const UMBRAL_DE_RECORTE: f32 = 0.9;
// Bloque de salida pedido en Linux: unos 40 ms a 48 kHz.
const FRAMES_POR_BLOQUE_DE_SALIDA: u32 = 2048;

/// Estado de una voz que comparten su motor y el callback de audio.
#[derive(Default)]
pub struct EstadoDeVoz {
    pub consumidor: Option<rtrb::Consumer<f32>>,
    /// Marca que pone el hilo de decodificacion cuando ya no queda audio.
    pub fin_de_datos: Arc<AtomicBool>,
    /// Falso durante una pausa: la voz no consume ni suena.
    pub sonando: bool,
    pub ganancia: f32,
    pub frames_emitidos: u64,
    /// La pista termino naturalmente; el motor lo consulta y lo limpia.
    pub termino: bool,
    /// Frame del reloj de salida en que la voz debe empezar a sonar.
    pub inicio_programado: Option<u64>,
    /// Frame del reloj de salida en que sonara el frame `frames_emitidos`
    /// de la voz, segun el ultimo callback en que sono.
    pub referencia: Option<u64>,
}

pub struct Voz {
    pub estado: Mutex<EstadoDeVoz>,
    volumen_objetivo: AtomicU32,
    // La voz es musica y pasa por el atenuador.
    atenuable: AtomicBool,
}

impl Voz {
    pub(super) fn new() -> Self {
        Self {
            estado: Mutex::new(EstadoDeVoz::default()),
            volumen_objetivo: AtomicU32::new(1.0f32.to_bits()),
            atenuable: AtomicBool::new(false),
        }
    }

    pub fn definir_atenuable(&self, atenuable: bool) {
        self.atenuable.store(atenuable, Ordering::Relaxed);
    }

    pub fn definir_volumen(&self, nivel: f32) {
        self.volumen_objetivo.store(nivel.to_bits(), Ordering::Relaxed);
    }

    pub fn volumen(&self) -> f32 {
        f32::from_bits(self.volumen_objetivo.load(Ordering::Relaxed))
    }
}

/// Cambio de nivel programado del atenuador.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rampa {
    /// Frame del reloj en que empieza.
    desde: u64,
    frames: u64,
    nivel_inicial: f32,
    objetivo: f32,
}

impl Rampa {
    /// Nivel en un frame, con una curva de coseno: arranca y llega sin
    /// quiebres, asi no se oyen escalones.
    fn nivel_en(&self, frame: u64) -> f32 {
        let avance = (frame.saturating_sub(self.desde) as f32 / self.frames.max(1) as f32).min(1.0);
        let suave = (1.0 - (avance * std::f32::consts::PI).cos()) * 0.5;
        self.nivel_inicial + (self.objetivo - self.nivel_inicial) * suave
    }
}

/// Envolvente del atenuador: nivel actual, rampa en curso y las programadas.
#[derive(Debug)]
struct Envolvente {
    nivel: f32,
    rampa: Option<Rampa>,
    /// Ordenadas por frame de inicio.
    programadas: Vec<(u64, f32, u64)>,
}

impl Envolvente {
    /// Nivel de cada frame del bloque que empieza en `reloj`.
    fn avanzar(&mut self, reloj: u64, niveles: &mut [f32]) {
        for (indice, nivel) in niveles.iter_mut().enumerate() {
            let frame = reloj + indice as u64;
            while self.programadas.first().is_some_and(|(desde, ..)| *desde <= frame) {
                let (desde, objetivo, frames) = self.programadas.remove(0);
                let actual = self.rampa.map_or(self.nivel, |rampa| rampa.nivel_en(frame));
                self.rampa = Some(Rampa { desde: desde.max(frame.saturating_sub(1)), frames, nivel_inicial: actual, objetivo });
            }
            if let Some(rampa) = self.rampa {
                self.nivel = rampa.nivel_en(frame);
                if frame >= rampa.desde + rampa.frames {
                    self.nivel = rampa.objetivo;
                    self.rampa = None;
                }
            }
            *nivel = self.nivel;
        }
    }
}

/// Atenuador comun de la musica, compartido entre el callback de audio y
/// quien programa los efectos.
pub struct Atenuador {
    envolvente: Mutex<Envolvente>,
    // Primer frame del proximo bloque que va a calcular el callback.
    reloj: AtomicU64,
    frecuencia: u32,
}

impl Atenuador {
    fn new(frecuencia: u32) -> Self {
        Self {
            envolvente: Mutex::new(Envolvente { nivel: 1.0, rampa: None, programadas: Vec::new() }),
            reloj: AtomicU64::new(0),
            frecuencia,
        }
    }

    /// Primer frame del reloj de salida que todavia no se calculo: lo mas
    /// pronto que se puede programar algo.
    pub fn reloj(&self) -> u64 {
        self.reloj.load(Ordering::Acquire)
    }

    pub fn frecuencia(&self) -> u32 {
        self.frecuencia
    }

    /// Programa una rampa hacia `objetivo` (0 a 1) que empieza en el frame
    /// `desde` y dura `segundos`. Se suma a las ya programadas.
    pub fn programar(&self, desde: u64, objetivo: f64, segundos: f64) {
        let frames = (segundos.max(0.0) * f64::from(self.frecuencia)).round() as u64;
        if let Ok(mut envolvente) = self.envolvente.lock() {
            let posicion = envolvente.programadas.partition_point(|(inicio, ..)| *inicio <= desde);
            envolvente.programadas.insert(posicion, (desde, objetivo.clamp(0.0, 1.0) as f32, frames.max(1)));
        }
    }

    /// Descarta las rampas programadas que todavia no empezaron.
    pub fn limpiar(&self) {
        if let Ok(mut envolvente) = self.envolvente.lock() {
            envolvente.programadas.clear();
        }
    }

    /// Descarta lo programado y vuelve al nivel pleno en `segundos`, desde ya.
    pub fn restituir(&self, segundos: f64) {
        self.limpiar();
        self.programar(self.reloj(), 1.0, segundos);
    }
}

/// Salida de audio del sistema: el identificador con que se vuelve a abrir
/// (por ejemplo "wasapi:{0.0.0.00000000}.{...}" o
/// "pulseaudio:alsa_output.usb-...") y el nombre que ve el usuario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SalidaDeAudio {
    pub id: String,
    pub nombre: String,
}

/// Salidas de audio disponibles ahora, en el orden del sistema. Si no se
/// pueden consultar devuelve una lista vacia.
pub fn listar_salidas() -> Vec<SalidaDeAudio> {
    let host = elegir_host();
    let dispositivos = match host.output_devices() {
        Ok(dispositivos) => dispositivos,
        Err(error) => {
            log::warn!("No se pudieron listar las salidas de audio: {error}");
            return Vec::new();
        }
    };
    let mut salidas: Vec<SalidaDeAudio> = Vec::new();
    for dispositivo in dispositivos {
        let Ok(id) = dispositivo.id() else {
            continue;
        };
        let id = id.to_string();
        if salidas.iter().any(|salida| salida.id == id) {
            continue;
        }
        let nombre = nombre_de(&dispositivo).unwrap_or_else(|| id.clone());
        salidas.push(SalidaDeAudio { id, nombre });
    }
    salidas
}

/// Nombre visible de un dispositivo.
fn nombre_de(dispositivo: &cpal::Device) -> Option<String> {
    dispositivo.description().ok().map(|descripcion| descripcion.name().to_string())
}

/// Mantiene vivo el flujo de salida mientras exista.
pub struct Mezclador {
    _flujo: Stream,
    dispositivo: cpal::Device,
    frecuencia: u32,
    atenuador: Arc<Atenuador>,
    // La salida dejo de estar disponible (por ejemplo, se desconecto).
    perdida: Arc<AtomicBool>,
}

impl Mezclador {
    /// Abre la salida de audio por defecto con la cantidad de voces pedida.
    pub fn abrir(cantidad_de_voces: usize) -> Result<(Self, Vec<Arc<Voz>>), String> {
        Self::abrir_en(None, cantidad_de_voces)
    }

    /// Abre la salida indicada por su identificador (None: la de por
    /// defecto) con la cantidad de voces pedida.
    pub fn abrir_en(salida: Option<&str>, cantidad_de_voces: usize) -> Result<(Self, Vec<Arc<Voz>>), String> {
        let host = elegir_host();
        let dispositivo = match salida {
            None => host.default_output_device().ok_or_else(|| tr!("No se encontro un dispositivo de salida de audio"))?,
            Some(id) => {
                let id: cpal::DeviceId =
                    id.parse().map_err(|error| tr!("Identificador de salida invalido ({id}): {error}", id = id, error = error))?;
                host.device_by_id(&id).ok_or_else(|| tr!("La salida de audio elegida no esta conectada"))?
            }
        };
        let configuracion = dispositivo
            .default_output_config()
            .map_err(|error| tr!("No se pudo consultar la salida de audio: {error}", error = error))?;
        let voces: Vec<Arc<Voz>> = (0..cantidad_de_voces).map(|_| Arc::new(Voz::new())).collect();

        let formato = configuracion.sample_format();
        let soportado = configuracion.buffer_size().clone();
        let mut config: StreamConfig = configuracion.into();
        // PulseAudio (tambien el de WSLg) pide por defecto bloques de mas de
        // un segundo, mas que el buffer de cada voz: se pide uno chico.
        if cfg!(target_os = "linux") {
            if let cpal::SupportedBufferSize::Range { min, max } = soportado {
                config.buffer_size = cpal::BufferSize::Fixed(FRAMES_POR_BLOQUE_DE_SALIDA.clamp(min, max));
            }
        }
        let frecuencia = config.sample_rate;
        let atenuador = Arc::new(Atenuador::new(frecuencia));
        let perdida = Arc::new(AtomicBool::new(false));
        let compartido = (voces.clone(), atenuador.clone(), perdida.clone());
        let flujo = match formato {
            SampleFormat::F32 => construir::<f32>(&dispositivo, config, compartido),
            SampleFormat::I16 => construir::<i16>(&dispositivo, config, compartido),
            SampleFormat::I32 => construir::<i32>(&dispositivo, config, compartido),
            SampleFormat::U16 => construir::<u16>(&dispositivo, config, compartido),
            SampleFormat::F64 => construir::<f64>(&dispositivo, config, compartido),
            otro => return Err(tr!("Formato de salida no soportado: {otro}", otro = otro)),
        }?;
        flujo.play().map_err(|error| tr!("No se pudo iniciar la salida de audio: {error}", error = error))?;
        log::info!("Salida de audio {} a {frecuencia} Hz", nombre_de(&dispositivo).unwrap_or_default());
        Ok((Self { _flujo: flujo, dispositivo, frecuencia, atenuador, perdida }, voces))
    }

    pub fn frecuencia(&self) -> u32 {
        self.frecuencia
    }

    /// Identificador de la salida por la que suena ahora. En Windows la
    /// salida por defecto sigue a la del sistema, asi que puede cambiar con
    /// el mezclador abierto; en Linux queda la del arranque.
    pub fn id_de_salida(&self) -> Option<String> {
        self.dispositivo.id().ok().map(|id| id.to_string())
    }

    /// La salida dejo de estar disponible y ya no suena.
    pub fn perdida(&self) -> bool {
        self.perdida.load(Ordering::Acquire)
    }

    pub fn atenuador(&self) -> Arc<Atenuador> {
        self.atenuador.clone()
    }
}

/// En Linux se prefiere PulseAudio, que tambien es el servidor de audio de
/// WSLg; si no esta disponible se usa el host por defecto (ALSA).
fn elegir_host() -> cpal::Host {
    #[cfg(target_os = "linux")]
    {
        asegurar_cookie_de_pulseaudio();
        match cpal::host_from_id(cpal::HostId::PulseAudio) {
            Ok(host) => return host,
            Err(error) => log::warn!("PulseAudio no esta disponible ({error}); se usa ALSA"),
        }
    }
    cpal::default_host()
}

/// El cliente PulseAudio de cpal se autentica con la cookie del usuario y,
/// a diferencia de libpulse, no la crea si falta (pasa en un WSL recien
/// instalado). Se crea igual que libpulse: 256 bytes aleatorios, modo 0600.
#[cfg(target_os = "linux")]
fn asegurar_cookie_de_pulseaudio() {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    if std::env::var_os("PULSE_COOKIE").is_some() {
        return;
    }
    let Some(inicio) = dirs::home_dir() else {
        return;
    };
    let cookie = inicio.join(".config/pulse/cookie");
    if cookie.exists() || inicio.join(".pulse-cookie").exists() {
        return;
    }
    let mut bytes = [0u8; 256];
    let creada = std::fs::File::open("/dev/urandom")
        .and_then(|mut azar| azar.read_exact(&mut bytes))
        .and_then(|_| std::fs::create_dir_all(cookie.parent().expect("la cookie tiene carpeta")))
        .and_then(|_| {
            use std::io::Write;
            std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&cookie)?.write_all(&bytes)
        });
    if let Err(error) = creada {
        log::warn!("No se pudo crear la cookie de PulseAudio en {}: {error}", cookie.display());
    }
}

fn construir<T>(
    dispositivo: &cpal::Device,
    config: StreamConfig,
    (voces, atenuador, perdida): (Vec<Arc<Voz>>, Arc<Atenuador>, Arc<AtomicBool>),
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let canales = config.channels as usize;
    let coeficiente = 1.0 - (-1.0 / (SEGUNDOS_DE_SUAVIZADO * config.sample_rate as f32)).exp();
    let mut mezcla: Vec<f32> = Vec::with_capacity(16_384);
    let mut niveles: Vec<f32> = Vec::with_capacity(8_192);
    let mut reloj: u64 = 0;

    dispositivo
        .build_output_stream(
            config,
            move |salida: &mut [T], _: &cpal::OutputCallbackInfo| {
                let frames = salida.len() / canales;
                mezcla.clear();
                mezcla.resize(frames * 2, 0.0);
                niveles.clear();
                niveles.resize(frames, 1.0);
                if let Ok(mut envolvente) = atenuador.envolvente.lock() {
                    envolvente.avanzar(reloj, &mut niveles);
                }
                for voz in &voces {
                    let atenuacion = voz.atenuable.load(Ordering::Relaxed).then_some(niveles.as_slice());
                    sumar_voz(voz, &mut mezcla, coeficiente, reloj, atenuacion);
                }
                reloj += frames as u64;
                atenuador.reloj.store(reloj, Ordering::Release);
                for (frame, estereo) in salida.chunks_mut(canales).zip(mezcla.chunks_exact(2)) {
                    let izquierdo = recortar_suave(estereo[0]);
                    let derecho = recortar_suave(estereo[1]);
                    if canales == 1 {
                        frame[0] = T::from_sample((izquierdo + derecho) * 0.5);
                        continue;
                    }
                    for (canal, muestra) in frame.iter_mut().enumerate() {
                        *muestra = T::from_sample(match canal {
                            0 => izquierdo,
                            1 => derecho,
                            _ => 0.0,
                        });
                    }
                }
            },
            move |error| {
                log::warn!("Error en la salida de audio: {error}");
                if matches!(error.kind(), cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::HostUnavailable) {
                    perdida.store(true, Ordering::Release);
                }
            },
            None,
        )
        .map_err(|error| tr!("No se pudo abrir la salida de audio: {error}", error = error))
}

/// Por debajo del umbral deja la muestra igual; por encima la acerca a 1
/// con una curva (tangente hiperbolica) que empalma sin quiebre.
fn recortar_suave(muestra: f32) -> f32 {
    let magnitud = muestra.abs();
    if magnitud <= UMBRAL_DE_RECORTE {
        return muestra;
    }
    let margen = 1.0 - UMBRAL_DE_RECORTE;
    (UMBRAL_DE_RECORTE + margen * ((magnitud - UMBRAL_DE_RECORTE) / margen).tanh()).copysign(muestra)
}

/// Suma a la mezcla los frames disponibles de una voz. Si el buffer se vacia
/// y el decodificador ya termino, marca el fin de la pista. `reloj` es el
/// frame de salida con que empieza este bloque; `atenuacion`, el nivel del
/// atenuador en cada frame si la voz es musica.
fn sumar_voz(voz: &Voz, mezcla: &mut [f32], coeficiente: f32, reloj: u64, atenuacion: Option<&[f32]>) {
    let objetivo = voz.volumen();
    let Ok(mut estado) = voz.estado.lock() else {
        return;
    };
    if !estado.sonando {
        return;
    }
    let frames = (mezcla.len() / 2) as u64;
    let mut desde = 0;
    if let Some(inicio) = estado.inicio_programado {
        if inicio >= reloj + frames {
            return;
        }
        desde = inicio.saturating_sub(reloj) as usize;
        estado.inicio_programado = None;
    }
    let EstadoDeVoz { consumidor, ganancia, frames_emitidos, fin_de_datos, referencia, .. } = &mut *estado;
    let Some(consumidor) = consumidor.as_mut() else {
        return;
    };
    let mut sumados = 0;
    for (indice, frame) in mezcla.chunks_exact_mut(2).enumerate().skip(desde) {
        if consumidor.slots() < 2 {
            break;
        }
        let izquierdo = consumidor.pop().unwrap_or(0.0);
        let derecho = consumidor.pop().unwrap_or(0.0);
        *ganancia += (objetivo - *ganancia) * coeficiente;
        let nivel = *ganancia * atenuacion.map_or(1.0, |niveles| niveles[indice]);
        frame[0] += izquierdo * nivel;
        frame[1] += derecho * nivel;
        *frames_emitidos += 1;
        sumados += 1;
    }
    if sumados > 0 {
        *referencia = Some(reloj + (desde + sumados) as u64);
    }
    if (objetivo - *ganancia).abs() < 0.001 {
        *ganancia = objetivo;
    }
    // La marca se lee antes que el buffer: si ya estaba puesta, todo el audio
    // ya fue escrito y un buffer vacio significa que realmente termino.
    let decodificacion_terminada = fin_de_datos.load(Ordering::Acquire);
    let terminado = decodificacion_terminada && consumidor.slots() < 2;
    if terminado {
        // El buffer lo libera el motor al enterarse del fin: liberar memoria
        // en el hilo de tiempo real puede demorar el bloque.
        estado.sonando = false;
        estado.termino = true;
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_recorte_suave_no_toca_lo_normal_y_no_pasa_de_uno() {
        assert_eq!(recortar_suave(0.5), 0.5);
        assert_eq!(recortar_suave(-0.9), -0.9);
        for muestra in [0.95f32, 1.0, 1.5, 3.0] {
            let recortada = recortar_suave(muestra);
            assert!(recortada > UMBRAL_DE_RECORTE && recortada <= 1.0, "{muestra} -> {recortada}");
            assert_eq!(recortar_suave(-muestra), -recortada);
        }
        assert!(recortar_suave(1.2) > recortar_suave(1.1), "sigue creciendo");
    }

    #[test]
    fn una_voz_programada_arranca_en_el_frame_exacto() {
        let voz = Voz::new();
        voz.definir_volumen(1.0);
        let (mut productor, consumidor) = rtrb::RingBuffer::new(64);
        for _ in 0..32 {
            productor.push(1.0).unwrap();
        }
        {
            let mut estado = voz.estado.lock().unwrap();
            estado.consumidor = Some(consumidor);
            estado.sonando = true;
            estado.ganancia = 1.0;
            estado.inicio_programado = Some(1_005);
        }
        let mut mezcla = vec![0.0f32; 8 * 2];
        sumar_voz(&voz, &mut mezcla, 1.0, 992, None);
        assert!(mezcla.iter().all(|muestra| *muestra == 0.0), "todavia no arranca");
        sumar_voz(&voz, &mut mezcla, 1.0, 1_000, None);
        let sonando: Vec<bool> = mezcla.chunks_exact(2).map(|frame| frame[0] > 0.0).collect();
        assert_eq!(sonando, [false, false, false, false, false, true, true, true]);
        let estado = voz.estado.lock().unwrap();
        assert_eq!(estado.frames_emitidos, 3);
        assert_eq!(estado.referencia, Some(1_008));
    }

    #[test]
    fn el_atenuador_sigue_las_rampas_programadas() {
        let atenuador = Atenuador::new(1_000);
        // Baja a 0,25 en 40 ms desde el frame 100, y vuelve en 400 ms desde el 500.
        atenuador.programar(500, 1.0, 0.4);
        atenuador.programar(100, 0.25, 0.04);
        let mut niveles = vec![0.0f32; 1_000];
        atenuador.envolvente.lock().unwrap().avanzar(0, &mut niveles);
        assert_eq!(niveles[99], 1.0, "todavia no empieza");
        assert!(niveles[120] < 1.0 && niveles[120] > 0.25, "a mitad de la bajada: {}", niveles[120]);
        assert_eq!(niveles[140], 0.25);
        assert_eq!(niveles[499], 0.25);
        assert!((niveles[700] - 0.625).abs() < 0.01, "a mitad de la subida: {}", niveles[700]);
        assert_eq!(niveles[900], 1.0);
        // Sin escalones: ningun paso entre frames supera lo que da la curva.
        let mayor_paso = niveles.windows(2).map(|par| (par[1] - par[0]).abs()).fold(0.0, f32::max);
        assert!(mayor_paso < 0.03, "paso maximo {mayor_paso}");
    }

    #[test]
    fn restituir_vuelve_desde_el_nivel_actual() {
        let atenuador = Atenuador::new(1_000);
        atenuador.programar(0, 0.2, 0.01);
        atenuador.programar(5_000, 1.0, 1.0);
        let mut niveles = vec![0.0f32; 100];
        atenuador.envolvente.lock().unwrap().avanzar(0, &mut niveles);
        atenuador.reloj.store(100, Ordering::Release);
        atenuador.restituir(0.1);
        let mut siguientes = vec![0.0f32; 120];
        atenuador.envolvente.lock().unwrap().avanzar(100, &mut siguientes);
        assert!((siguientes[0] - 0.2).abs() < 0.01, "arranca donde estaba: {}", siguientes[0]);
        assert_eq!(siguientes[110], 1.0, "y llega al pleno");
    }

    #[test]
    fn la_musica_se_atenua_y_el_efecto_no() {
        let voz = Voz::new();
        let (mut productor, consumidor) = rtrb::RingBuffer::new(64);
        for _ in 0..16 {
            productor.push(1.0).unwrap();
        }
        {
            let mut estado = voz.estado.lock().unwrap();
            estado.consumidor = Some(consumidor);
            estado.sonando = true;
            estado.ganancia = 1.0;
        }
        let mut mezcla = vec![0.0f32; 4 * 2];
        sumar_voz(&voz, &mut mezcla, 1.0, 0, Some(&[0.5, 0.5, 0.25, 0.25]));
        let izquierdos: Vec<f32> = mezcla.chunks_exact(2).map(|frame| frame[0]).collect();
        assert_eq!(izquierdos, [0.5, 0.5, 0.25, 0.25]);
    }
}
