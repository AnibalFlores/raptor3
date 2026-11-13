//! Forma de onda de una pista: picos minimo y maximo y nivel RMS por cada
//! centesima de segundo, de la mezcla mono del audio. Alcanza para dibujarla
//! con zoom hasta centesimas y para estimar intro y outro. En la misma pasada
//! se miden la sonoridad, el pico real y el rango dinamico (EBU R128), el ritmo
//! del principio y del final, que usa el crossfade inteligente, y cuanto
//! suena a voz cada decima de segundo, para sugerir la ventana de overlay.
//!
//! Calcularla exige decodificar el archivo entero, asi que se guarda en una
//! cache en disco (una por archivo, invalidada si cambia su tamano o fecha).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::Decodificador;
use super::ritmo::{self, Ritmo, Tempo};
use super::voz::DetectorDeVoz;
use crate::dominio::normalizar;

pub const PICOS_POR_SEGUNDO: u32 = 100;
const ENCABEZADO: &[u8] = b"RAPTORONDA";
// Version 2: agrega sonoridad y ritmo. Version 3: pico real y rango dinamico.
// Version 4: puntaje de voz. Version 5: ritmo de todo el tema (BPM).
// Version 6: tempo votado entre ventanas en lugar del ritmo de todo el tema.
// Version 7: ataques de tres bandas para el ritmo y el tempo.
const VERSION_DE_CACHE: u8 = 7;
// Segundos del principio y del final (sin contar el silencio) donde se busca
// el ritmo para alinear los golpes en una mezcla.
const SEGUNDOS_DE_RITMO: f64 = 30.0;
// Cortes de los filtros que separan las bandas grave, media y aguda para
// detectar los golpes.
const CORTE_GRAVE: f64 = 150.0;
const CORTE_MEDIO: f64 = 2000.0;
// Nivel RMS por debajo del cual se considera silencio (unos -60 dB).
const NIVEL_DE_SILENCIO: f32 = 0.001;

/// Intro y outro estimadas desde la forma de onda, en segundos, y el
/// silencio que hay al principio y al final del archivo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sugerencia {
    pub intro: f64,
    pub outro: f64,
    pub silencio_inicial: f64,
    pub silencio_final: f64,
}

/// Resumen de amplitud de un tramo de audio, en la escala -1 a 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pico {
    pub minimo: f32,
    pub maximo: f32,
    pub rms: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormaDeOnda {
    picos_por_segundo: u32,
    picos: Vec<Pico>,
    /// Sonoridad integrada en LUFS; None si el archivo es silencio.
    sonoridad: Option<f64>,
    /// Pico real (entre muestras) de 0 a 1, o mas si el archivo ya satura.
    pico_real: Option<f64>,
    /// Rango de sonoridad (LRA) en LU.
    rango_dinamico: Option<f64>,
    ritmo_inicial: Option<Ritmo>,
    ritmo_final: Option<Ritmo>,
    tempo: Option<Tempo>,
    /// Cuanto suena a voz cada decima de segundo, de 0 a 255.
    voz: Vec<u8>,
}

impl FormaDeOnda {
    /// Decodifica el archivo entero y resume su amplitud.
    pub fn calcular(archivo: &Path) -> Result<Self, String> {
        let mut decodificador = Decodificador::abrir(archivo)?;
        let mut picos = Vec::new();
        let mut bandas = EnergiaPorBandas::default();
        let mut bloque = Vec::new();
        let mut actual = Acumulador::default();
        let mut frames_en_pico = 0.0f64;
        // Medidor de sonoridad, creado con la frecuencia del primer bloque.
        let mut medidor: Option<(u32, ebur128::EbuR128)> = None;
        let mut detector: Option<DetectorDeVoz> = None;
        while let Some(frecuencia) = {
            bloque.clear();
            decodificador.siguiente_bloque(&mut bloque)
        } {
            if medidor.is_none() {
                medidor = ebur128::EbuR128::new(2, frecuencia, ebur128::Mode::I | ebur128::Mode::LRA | ebur128::Mode::TRUE_PEAK).ok().map(|nuevo| (frecuencia, nuevo));
            }
            if let Some((_, medidor)) = medidor.as_mut().filter(|(de, _)| *de == frecuencia) {
                let _ = medidor.add_frames_f32(&bloque);
            }
            let detector = detector.get_or_insert_with(|| DetectorDeVoz::nuevo(frecuencia));
            if detector.frecuencia() == frecuencia {
                detector.sumar(&bloque);
            }
            let frames_por_pico = f64::from(frecuencia) / f64::from(PICOS_POR_SEGUNDO);
            bandas.ajustar_a(frecuencia);
            for frame in bloque.chunks_exact(2) {
                let mono = (frame[0] + frame[1]) * 0.5;
                actual.sumar(mono);
                bandas.sumar(mono);
                frames_en_pico += 1.0;
                if frames_en_pico >= frames_por_pico {
                    bandas.cerrar(actual.cantidad);
                    picos.push(actual.cerrar());
                    frames_en_pico -= frames_por_pico;
                }
            }
        }
        if actual.cantidad > 0 {
            bandas.cerrar(actual.cantidad);
            picos.push(actual.cerrar());
        }
        let medidor = medidor.map(|(_, medidor)| medidor);
        let sonoridad = medidor.as_ref().and_then(|medidor| medidor.loudness_global().ok()).filter(|lufs| lufs.is_finite());
        let pico_real = medidor
            .as_ref()
            .and_then(|medidor| Some(medidor.true_peak(0).ok()?.max(medidor.true_peak(1).ok()?)))
            .filter(|pico| *pico > 0.0 && sonoridad.is_some());
        let rango_dinamico = medidor.as_ref().and_then(|medidor| medidor.loudness_range().ok()).filter(|_| sonoridad.is_some());
        let mut onda = Self {
            picos_por_segundo: PICOS_POR_SEGUNDO,
            picos,
            sonoridad,
            pico_real,
            rango_dinamico,
            ritmo_inicial: None,
            ritmo_final: None,
            tempo: None,
            voz: detector.map(DetectorDeVoz::puntajes).unwrap_or_default().iter().map(|puntaje| (puntaje * 255.0).round() as u8).collect(),
        };
        onda.estimar_ritmos(&bandas);
        Ok(onda)
    }

    /// Ritmo de los primeros y de los ultimos segundos con sonido, y el
    /// tempo de todo el tema.
    fn estimar_ritmos(&mut self, bandas: &EnergiaPorBandas) {
        let ataques = ritmo::ataques(&[&bandas.grave, &bandas.media, &bandas.aguda]);
        let por_segundo = f64::from(self.picos_por_segundo);
        let tramo = (SEGUNDOS_DE_RITMO * por_segundo) as usize;
        let primero = self.picos.iter().position(|pico| pico.rms > NIVEL_DE_SILENCIO).unwrap_or(0);
        let ultimo = self.picos.iter().rposition(|pico| pico.rms > NIVEL_DE_SILENCIO).map_or(0, |indice| indice + 1);
        self.ritmo_inicial = ritmo::estimar(&ataques, por_segundo, primero, (primero + tramo).min(ultimo));
        self.ritmo_final = ritmo::estimar(&ataques, por_segundo, ultimo.saturating_sub(tramo).max(primero), ultimo);
        self.tempo = ritmo::votar_tempo(&ataques, por_segundo, primero, ultimo);
    }

    /// Forma de onda guardada en la cache, sin calcularla si falta: para el
    /// hilo de la interfaz, que no debe decodificar un tema entero.
    pub fn de_la_cache(archivo: &Path, carpeta_de_cache: &Path) -> Option<Self> {
        Self::leer(&ruta_de_cache(archivo, carpeta_de_cache)?).ok()
    }

    /// Devuelve la forma de onda guardada en la cache o la calcula y la guarda.
    pub fn cargar_o_calcular(archivo: &Path, carpeta_de_cache: &Path) -> Result<Self, String> {
        let cache = ruta_de_cache(archivo, carpeta_de_cache);
        if let Some(guardada) = cache.as_ref().and_then(|ruta| Self::leer(ruta).ok()) {
            return Ok(guardada);
        }
        let onda = Self::calcular(archivo)?;
        if let Some(ruta) = cache {
            if let Err(error) = onda.escribir(&ruta) {
                log::warn!("No se pudo guardar la forma de onda en {}: {error}", ruta.display());
            }
        }
        Ok(onda)
    }

    pub fn duracion(&self) -> f64 {
        self.picos.len() as f64 / f64::from(self.picos_por_segundo)
    }

    #[cfg(test)]
    pub fn picos(&self) -> &[Pico] {
        &self.picos
    }

    /// Sonoridad integrada (EBU R128) en LUFS; None si es silencio.
    pub fn sonoridad(&self) -> Option<f64> {
        self.sonoridad
    }

    /// Mayor amplitud del archivo, de 0 a 1.
    pub fn pico_maximo(&self) -> f32 {
        self.picos.iter().map(|pico| pico.maximo.max(-pico.minimo)).fold(0.0, f32::max)
    }

    /// Pico real (EBU R128, con los picos entre muestras); None si es silencio.
    pub fn pico_real(&self) -> Option<f64> {
        self.pico_real
    }

    /// Rango de sonoridad (LRA) en LU; None si es silencio.
    pub fn rango_dinamico(&self) -> Option<f64> {
        self.rango_dinamico
    }

    /// Ritmo de los primeros segundos con sonido.
    pub fn ritmo_inicial(&self) -> Option<Ritmo> {
        self.ritmo_inicial
    }

    /// Ritmo de los ultimos segundos con sonido.
    pub fn ritmo_final(&self) -> Option<Ritmo> {
        self.ritmo_final
    }

    /// Tempo de todo el tema, sin el silencio del principio y del final.
    pub fn tempo(&self) -> Option<Tempo> {
        self.tempo
    }

    /// Cuanto suena a voz cada decima de segundo, de 0 a 1 (0,5 es lo tipico
    /// del tema).
    pub fn voz(&self) -> Vec<f32> {
        self.voz.iter().map(|puntaje| f32::from(*puntaje) / 255.0).collect()
    }

    /// Cuantos dB mas fuerte (o mas suave, negativo) que lo tipico del tema
    /// suena el tramo [desde, hasta): su nivel RMS contra la mediana de los
    /// tramos con sonido. None si el tema es silencio.
    pub fn desvio_local_db(&self, desde: f64, hasta: f64) -> Option<f64> {
        let mut niveles: Vec<f32> = self.picos.iter().map(|pico| pico.rms).filter(|rms| *rms > NIVEL_DE_SILENCIO).collect();
        if niveles.is_empty() {
            return None;
        }
        niveles.sort_by(f32::total_cmp);
        let tipico = f64::from(niveles[niveles.len() / 2]);
        let tramo = f64::from(self.tramo(desde, hasta).rms).max(1e-6);
        Some((20.0 * (tramo / tipico).log10()).clamp(-40.0, 20.0))
    }

    /// Resumen de un tramo [desde, hasta) en segundos; vacio fuera del audio.
    pub fn tramo(&self, desde: f64, hasta: f64) -> Pico {
        let indice = |segundos: f64| ((segundos * f64::from(self.picos_por_segundo)).floor().max(0.0) as usize).min(self.picos.len());
        let (primero, ultimo) = (indice(desde), indice(hasta).max(indice(desde) + 1).min(self.picos.len()));
        let tramo = &self.picos[primero.min(ultimo)..ultimo];
        if tramo.is_empty() {
            return Pico::default();
        }
        let suma_de_cuadrados: f32 = tramo.iter().map(|pico| pico.rms * pico.rms).sum();
        Pico {
            minimo: tramo.iter().map(|pico| pico.minimo).fold(f32::MAX, f32::min),
            maximo: tramo.iter().map(|pico| pico.maximo).fold(f32::MIN, f32::max),
            rms: (suma_de_cuadrados / tramo.len() as f32).sqrt(),
        }
    }

    /// Dibuja la onda entre `desde` y `hasta` (segundos) en una imagen RGBA de
    /// `ancho` x `alto` con fondo transparente: una barra vertical por columna
    /// con el pico, y encima, mas opaca, la del nivel RMS.
    pub fn dibujar(&self, ancho: u32, alto: u32, desde: f64, hasta: f64, color: [u8; 3]) -> Vec<u8> {
        let (ancho, alto) = (ancho.max(1) as usize, alto.max(1) as usize);
        let mut pixeles = vec![0u8; ancho * alto * 4];
        let segundos_por_columna = (hasta - desde).max(0.0) / ancho as f64;
        let centro = (alto as f32 - 1.0) / 2.0;
        let mut pintar = |columna: usize, desde_y: f32, hasta_y: f32, opacidad: u8| {
            let (arriba, abajo) = (desde_y.min(hasta_y).floor().max(0.0) as usize, hasta_y.max(desde_y).ceil() as usize);
            for fila in arriba..=abajo.min(alto - 1) {
                let pixel = (fila * ancho + columna) * 4;
                pixeles[pixel..pixel + 3].copy_from_slice(&color);
                pixeles[pixel + 3] = pixeles[pixel + 3].max(opacidad);
            }
        };
        for columna in 0..ancho {
            let inicio = desde + columna as f64 * segundos_por_columna;
            let pico = self.tramo(inicio, inicio + segundos_por_columna);
            if pico == Pico::default() {
                continue;
            }
            pintar(columna, centro - pico.maximo * centro, centro - pico.minimo * centro, 140);
            pintar(columna, centro - pico.rms * centro, centro + pico.rms * centro, 255);
        }
        pixeles
    }

    /// Barras de nivel para una vista compacta: la mitad superior de la onda,
    /// en `cantidad` tramos iguales. Cada una da el pico y el nivel RMS de su
    /// tramo, de 0 a 1 respecto del pico mas alto del tema (asi un tema bajo
    /// tambien llena la vista). Es barato: solo recorre los picos ya
    /// calculados.
    pub fn barras_de_nivel(&self, cantidad: usize) -> Vec<(f32, f32)> {
        let duracion = self.duracion();
        if cantidad == 0 || duracion <= 0.0 {
            return Vec::new();
        }
        let tramo = duracion / cantidad as f64;
        let barras: Vec<(f32, f32)> = (0..cantidad)
            .map(|indice| {
                let pico = self.tramo(indice as f64 * tramo, (indice + 1) as f64 * tramo);
                (pico.maximo.abs().max(pico.minimo.abs()), pico.rms)
            })
            .collect();
        let maximo = barras.iter().map(|(pico, _)| *pico).fold(0.0f32, f32::max);
        if maximo <= f32::EPSILON {
            return vec![(0.0, 0.0); cantidad];
        }
        barras.into_iter().map(|(pico, rms)| ((pico / maximo).min(1.0), (rms / maximo).min(1.0))).collect()
    }

    /// Estima intro y outro a partir de la envolvente del nivel RMS (promedio
    /// movil de medio segundo): la intro termina cuando el tema llega a la
    /// mitad de su nivel tipico y la outro empieza cuando baja de esa mitad
    /// por ultima vez. Cada una se limita al 30 % de la pista. Tambien informa
    /// el silencio al principio y al final.
    pub fn sugerir(&self) -> Sugerencia {
        let duracion = self.duracion();
        let por_segundo = f64::from(self.picos_por_segundo);
        let envolvente = self.envolvente((por_segundo / 2.0).round().max(1.0) as usize);
        let mut ordenados: Vec<f32> = envolvente.iter().copied().filter(|nivel| *nivel > NIVEL_DE_SILENCIO).collect();
        if ordenados.is_empty() {
            return Sugerencia { intro: 0.0, outro: 0.0, silencio_inicial: duracion, silencio_final: duracion };
        }
        ordenados.sort_by(f32::total_cmp);
        let tipico = ordenados[(ordenados.len() as f64 * 0.7) as usize];
        let umbral = tipico * 0.5;
        let tope = duracion * 0.3;
        let segundos = |indice: usize| indice as f64 / por_segundo;
        let primero_alto = envolvente.iter().position(|nivel| *nivel >= umbral).unwrap_or(0);
        let ultimo_alto = envolvente.iter().rposition(|nivel| *nivel >= umbral).unwrap_or(envolvente.len());
        let primero_audible = self.picos.iter().position(|pico| pico.rms > NIVEL_DE_SILENCIO).unwrap_or(0);
        let ultimo_audible = self.picos.iter().rposition(|pico| pico.rms > NIVEL_DE_SILENCIO).map_or(0, |indice| indice + 1);
        let redondear = |segundos: f64| (segundos * 10.0).round() / 10.0;
        Sugerencia {
            intro: redondear(segundos(primero_alto).min(tope)),
            outro: redondear((duracion - segundos(ultimo_alto + 1)).clamp(0.0, tope)),
            silencio_inicial: redondear(segundos(primero_audible)),
            silencio_final: redondear((duracion - segundos(ultimo_audible)).max(0.0)),
        }
    }

    /// Promedio movil centrado del nivel RMS, con la ventana indicada en picos.
    fn envolvente(&self, ventana: usize) -> Vec<f32> {
        let mitad = ventana / 2;
        let mut acumulado = vec![0.0f64; self.picos.len() + 1];
        for (indice, pico) in self.picos.iter().enumerate() {
            acumulado[indice + 1] = acumulado[indice] + f64::from(pico.rms);
        }
        (0..self.picos.len())
            .map(|indice| {
                let (desde, hasta) = (indice.saturating_sub(mitad), (indice + mitad + 1).min(self.picos.len()));
                ((acumulado[hasta] - acumulado[desde]) / (hasta - desde) as f64) as f32
            })
            .collect()
    }

    fn escribir(&self, ruta: &Path) -> std::io::Result<()> {
        if let Some(carpeta) = ruta.parent() {
            fs::create_dir_all(carpeta)?;
        }
        let mut datos = Vec::with_capacity(ENCABEZADO.len() + 73 + self.picos.len() * 6 + self.voz.len());
        datos.extend_from_slice(ENCABEZADO);
        datos.push(VERSION_DE_CACHE);
        datos.extend_from_slice(&self.picos_por_segundo.to_le_bytes());
        datos.extend_from_slice(&(self.picos.len() as u32).to_le_bytes());
        for pico in &self.picos {
            for valor in [pico.minimo, pico.maximo, pico.rms] {
                datos.extend_from_slice(&a_entero(valor).to_le_bytes());
            }
        }
        // Al final: sonoridad, pico real, rango dinamico, los dos ritmos
        // (periodo, fase, confianza) y el tempo (bpm, acuerdo, ventanas); NaN
        // donde no hay valor.
        for valor in [self.sonoridad, self.pico_real, self.rango_dinamico] {
            datos.extend_from_slice(&valor.unwrap_or(f64::NAN).to_le_bytes());
        }
        let tempo = self.tempo.map_or([f64::NAN; 3], |tempo| [tempo.bpm, tempo.acuerdo, tempo.ventanas as f64]);
        let ritmos = [self.ritmo_inicial, self.ritmo_final]
            .map(|ritmo| ritmo.map_or([f64::NAN; 3], |ritmo| [ritmo.periodo, ritmo.fase, ritmo.confianza]));
        for valores in [ritmos[0], ritmos[1], tempo] {
            for valor in valores {
                datos.extend_from_slice(&(valor as f32).to_le_bytes());
            }
        }
        // Y el puntaje de voz, precedido por su cantidad.
        datos.extend_from_slice(&(self.voz.len() as u32).to_le_bytes());
        datos.extend_from_slice(&self.voz);
        let mut temporal = ruta.as_os_str().to_owned();
        temporal.push(".tmp");
        fs::File::create(&temporal)?.write_all(&datos)?;
        fs::rename(&temporal, ruta)
    }

    fn leer(ruta: &Path) -> Result<Self, String> {
        let mut datos = Vec::new();
        fs::File::open(ruta).and_then(|mut archivo| archivo.read_to_end(&mut datos)).map_err(|error| error.to_string())?;
        let resto = datos.strip_prefix(ENCABEZADO).ok_or("cache de onda invalida")?;
        let (&version, resto) = resto.split_first().ok_or("cache de onda incompleta")?;
        if version != VERSION_DE_CACHE || resto.len() < 8 {
            return Err(tr!("cache de onda de otra version").into());
        }
        let picos_por_segundo = u32::from_le_bytes(resto[0..4].try_into().expect("4 bytes"));
        let cantidad = u32::from_le_bytes(resto[4..8].try_into().expect("4 bytes")) as usize;
        let valores = &resto[8..];
        let fijo = cantidad * 6 + 60;
        if valores.len() < fijo + 4 || picos_por_segundo == 0 {
            return Err(tr!("cache de onda incompleta").into());
        }
        let cantidad_de_voz = u32::from_le_bytes(valores[fijo..fijo + 4].try_into().expect("4 bytes")) as usize;
        if valores.len() != fijo + 4 + cantidad_de_voz {
            return Err(tr!("cache de onda incompleta").into());
        }
        let numero = |indice: usize| a_real(i16::from_le_bytes([valores[indice], valores[indice + 1]]));
        let picos = (0..cantidad)
            .map(|pico| Pico { minimo: numero(pico * 6), maximo: numero(pico * 6 + 2), rms: numero(pico * 6 + 4) })
            .collect();
        let extra = &valores[cantidad * 6..];
        let doble = |desde: usize| Some(f64::from_le_bytes(extra[desde..desde + 8].try_into().expect("8 bytes"))).filter(|valor| valor.is_finite());
        let real = |desde: usize| f64::from(f32::from_le_bytes(extra[desde..desde + 4].try_into().expect("4 bytes")));
        let ritmo = |desde: usize| {
            let ritmo = Ritmo { periodo: real(desde), fase: real(desde + 4), confianza: real(desde + 8) };
            ritmo.periodo.is_finite().then_some(ritmo)
        };
        Ok(Self {
            picos_por_segundo,
            picos,
            sonoridad: doble(0),
            pico_real: doble(8),
            rango_dinamico: doble(16),
            ritmo_inicial: ritmo(24),
            ritmo_final: ritmo(36),
            tempo: Some(Tempo { bpm: real(48), acuerdo: real(52), ventanas: real(56) as usize })
                .filter(|tempo| tempo.bpm.is_finite()),
            voz: valores[fijo + 4..].to_vec(),
        })
    }
}

/// Energia por centesima de segundo de las bandas grave, media y aguda,
/// separadas con dos filtros pasabajos de un polo.
#[derive(Default)]
struct EnergiaPorBandas {
    grave: Vec<f32>,
    media: Vec<f32>,
    aguda: Vec<f32>,
    frecuencia: u32,
    coeficiente_grave: f32,
    coeficiente_medio: f32,
    filtro_grave: f32,
    filtro_medio: f32,
    sumas: [f32; 3],
}

impl EnergiaPorBandas {
    /// Recalcula los filtros si cambia la frecuencia de muestreo.
    fn ajustar_a(&mut self, frecuencia: u32) {
        if self.frecuencia == frecuencia {
            return;
        }
        let coeficiente = |corte: f64| (1.0 - (-std::f64::consts::TAU * corte / f64::from(frecuencia)).exp()) as f32;
        self.frecuencia = frecuencia;
        self.coeficiente_grave = coeficiente(CORTE_GRAVE);
        self.coeficiente_medio = coeficiente(CORTE_MEDIO);
    }

    fn sumar(&mut self, muestra: f32) {
        self.filtro_grave += (muestra - self.filtro_grave) * self.coeficiente_grave;
        self.filtro_medio += (muestra - self.filtro_medio) * self.coeficiente_medio;
        let media = self.filtro_medio - self.filtro_grave;
        let aguda = muestra - self.filtro_medio;
        self.sumas[0] += self.filtro_grave * self.filtro_grave;
        self.sumas[1] += media * media;
        self.sumas[2] += aguda * aguda;
    }

    fn cerrar(&mut self, cantidad: u32) {
        let cantidad = cantidad.max(1) as f32;
        self.grave.push(self.sumas[0] / cantidad);
        self.media.push(self.sumas[1] / cantidad);
        self.aguda.push(self.sumas[2] / cantidad);
        self.sumas = [0.0; 3];
    }
}

#[derive(Default)]
struct Acumulador {
    minimo: f32,
    maximo: f32,
    suma_de_cuadrados: f32,
    cantidad: u32,
}

impl Acumulador {
    fn sumar(&mut self, muestra: f32) {
        let muestra = muestra.clamp(-1.0, 1.0);
        self.minimo = self.minimo.min(muestra);
        self.maximo = self.maximo.max(muestra);
        self.suma_de_cuadrados += muestra * muestra;
        self.cantidad += 1;
    }

    fn cerrar(&mut self) -> Pico {
        let pico = Pico {
            minimo: self.minimo,
            maximo: self.maximo,
            rms: (self.suma_de_cuadrados / self.cantidad.max(1) as f32).sqrt(),
        };
        *self = Self::default();
        pico
    }
}

fn a_entero(valor: f32) -> i16 {
    (valor.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

fn a_real(valor: i16) -> f32 {
    f32::from(valor) / f32::from(i16::MAX)
}

/// Borra de la carpeta de cache las ondas que no corresponden a ninguno de
/// los archivos `en_uso` tal como estan ahora: las de archivos que ya no
/// existen, que se movieron o que cambiaron. Devuelve cuantas se borraron y
/// cuantos bytes se liberaron.
pub fn limpiar_cache_de_ondas<'a>(carpeta: &Path, en_uso: impl IntoIterator<Item = &'a Path>) -> std::io::Result<(usize, u64)> {
    let vigentes: std::collections::HashSet<PathBuf> =
        en_uso.into_iter().filter_map(|archivo| ruta_de_cache(archivo, carpeta)).collect();
    let (mut borradas, mut bytes) = (0, 0);
    for entrada in fs::read_dir(carpeta)?.flatten() {
        let ruta = entrada.path();
        if ruta.extension().is_none_or(|extension| extension != "onda") || vigentes.contains(&ruta) {
            continue;
        }
        let largo = entrada.metadata().map_or(0, |datos| datos.len());
        if fs::remove_file(&ruta).is_ok() {
            borradas += 1;
            bytes += largo;
        }
    }
    Ok((borradas, bytes))
}

/// Archivo de cache para un audio: su nombre depende de la ruta, el tamano
/// y la fecha de modificacion, asi un archivo cambiado se vuelve a analizar.
fn ruta_de_cache(archivo: &Path, carpeta: &Path) -> Option<PathBuf> {
    let datos = fs::metadata(archivo).ok()?;
    let modificado = datos.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let clave = format!("{}|{}|{modificado}", normalizar(archivo).to_string_lossy(), datos.len());
    Some(carpeta.join(format!("{:016x}.onda", fnv1a(clave.as_bytes()))))
}

/// Hash FNV-1a de 64 bits: estable entre ejecuciones y versiones de Rust.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
pub mod pruebas {
    use super::*;

    #[test]
    fn las_barras_de_nivel_van_de_0_a_1_respecto_del_pico() {
        let picos = |nivel: f32| Pico { minimo: -nivel, maximo: nivel, rms: nivel / 2.0 };
        let onda = FormaDeOnda {
            picos_por_segundo: 10,
            picos: [vec![picos(0.0); 10], vec![picos(0.25); 10], vec![picos(0.5); 20]].concat(),
            sonoridad: None,
            pico_real: None,
            rango_dinamico: None,
            ritmo_inicial: None,
            ritmo_final: None,
            tempo: None,
            voz: Vec::new(),
        };
        let barras = onda.barras_de_nivel(4);
        assert_eq!(barras, [(0.0, 0.0), (0.5, 0.25), (1.0, 0.5), (1.0, 0.5)]);
        assert!(onda.barras_de_nivel(0).is_empty());
    }

    /// Escribe un WAV mono de 16 bits con los tramos (segundos, amplitud) de
    /// un tono de 440 Hz; amplitud 0 es silencio.
    pub fn escribir_wav(ruta: &Path, frecuencia: u32, tramos: &[(f64, f32)]) {
        let mut muestras: Vec<i16> = Vec::new();
        for &(segundos, amplitud) in tramos {
            let cantidad = (segundos * f64::from(frecuencia)) as usize;
            muestras.extend((0..cantidad).map(|n| {
                let fase = n as f32 * 440.0 * std::f32::consts::TAU / frecuencia as f32;
                (fase.sin() * amplitud * f32::from(i16::MAX)) as i16
            }));
        }
        escribir_muestras(ruta, frecuencia, &muestras);
    }

    /// WAV estereo de 16 bits desde muestras intercaladas de -1 a 1.
    pub fn escribir_estereo(ruta: &Path, frecuencia: u32, muestras: &[f32]) {
        let enteras: Vec<i16> = muestras.iter().map(|muestra| (muestra * f32::from(i16::MAX)) as i16).collect();
        escribir_con_canales(ruta, frecuencia, 2, &enteras);
    }

    fn escribir_muestras(ruta: &Path, frecuencia: u32, muestras: &[i16]) {
        escribir_con_canales(ruta, frecuencia, 1, muestras);
    }

    fn escribir_con_canales(ruta: &Path, frecuencia: u32, canales: u16, muestras: &[i16]) {
        let bytes_de_datos = (muestras.len() * 2) as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + bytes_de_datos).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&canales.to_le_bytes());
        wav.extend_from_slice(&frecuencia.to_le_bytes());
        wav.extend_from_slice(&(frecuencia * 2 * u32::from(canales)).to_le_bytes());
        wav.extend_from_slice(&(2 * canales).to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&bytes_de_datos.to_le_bytes());
        for muestra in muestras {
            wav.extend_from_slice(&muestra.to_le_bytes());
        }
        fs::create_dir_all(ruta.parent().unwrap()).unwrap();
        fs::write(ruta, wav).unwrap();
    }

    pub fn carpeta_temporal(nombre: &str) -> PathBuf {
        let carpeta = std::env::temp_dir().join(format!("raptor-onda-{nombre}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&carpeta);
        fs::create_dir_all(&carpeta).unwrap();
        carpeta
    }

    #[test]
    fn resume_silencio_y_sonido() {
        let carpeta = carpeta_temporal("resumen");
        let wav = carpeta.join("tono.wav");
        escribir_wav(&wav, 8_000, &[(1.0, 0.0), (1.0, 0.5), (1.0, 0.0)]);
        let onda = FormaDeOnda::calcular(&wav).unwrap();
        assert!((onda.duracion() - 3.0).abs() < 0.02, "duracion {}", onda.duracion());
        assert!(onda.tramo(0.1, 0.9).maximo < 0.01);
        let sonido = onda.tramo(1.1, 1.9);
        assert!((sonido.maximo - 0.5).abs() < 0.02 && (sonido.minimo + 0.5).abs() < 0.02, "{sonido:?}");
        assert!((sonido.rms - 0.5 / 2f32.sqrt()).abs() < 0.02, "rms {}", sonido.rms);
        assert!(onda.tramo(2.1, 2.9).maximo < 0.01);
    }

    #[test]
    fn la_cache_devuelve_lo_mismo_y_se_invalida_si_cambia_el_archivo() {
        let carpeta = carpeta_temporal("cache");
        let wav = carpeta.join("tono.wav");
        let cache = carpeta.join("ondas");
        escribir_wav(&wav, 8_000, &[(0.5, 0.0), (0.5, 0.8)]);
        let calculada = FormaDeOnda::cargar_o_calcular(&wav, &cache).unwrap();
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
        let leida = FormaDeOnda::cargar_o_calcular(&wav, &cache).unwrap();
        assert_eq!(leida.picos().len(), calculada.picos().len());
        assert_eq!(leida.sonoridad(), calculada.sonoridad());
        assert_eq!(leida.pico_real(), calculada.pico_real());
        assert_eq!(leida.rango_dinamico(), calculada.rango_dinamico());
        assert!(leida.sonoridad().is_some() && leida.pico_real().is_some());
        for (a, b) in leida.picos().iter().zip(calculada.picos()) {
            assert!((a.maximo - b.maximo).abs() < 1e-3 && (a.rms - b.rms).abs() < 1e-3);
        }

        escribir_wav(&wav, 8_000, &[(2.0, 0.3)]);
        let nueva = FormaDeOnda::cargar_o_calcular(&wav, &cache).unwrap();
        assert!((nueva.duracion() - 2.0).abs() < 0.02);

        // La onda del archivo anterior ya no corresponde a nada: la limpieza
        // la borra y conserva la vigente.
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 2);
        let (borradas, bytes) = limpiar_cache_de_ondas(&cache, [wav.as_path()]).unwrap();
        assert_eq!(borradas, 1);
        assert!(bytes > 0);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);
        assert!(FormaDeOnda::cargar_o_calcular(&wav, &cache).is_ok());
    }

    /// WAV con fundido de entrada, tramo pleno y fundido de salida.
    fn escribir_con_fundidos(ruta: &Path, silencio: f64, entrada: f64, pleno: f64, salida: f64) {
        let frecuencia = 8_000u32;
        let mut tramos = vec![(silencio, 0.0)];
        let pasos = 40;
        tramos.extend((0..pasos).map(|paso| (entrada / pasos as f64, 0.8 * (paso as f32 + 0.5) / pasos as f32)));
        tramos.push((pleno, 0.8));
        tramos.extend((0..pasos).map(|paso| (salida / pasos as f64, 0.8 * (pasos as f32 - paso as f32 - 0.5) / pasos as f32)));
        tramos.push((silencio, 0.0));
        escribir_wav(ruta, frecuencia, &tramos);
    }

    #[test]
    fn sugiere_intro_y_outro_segun_los_fundidos() {
        let carpeta = carpeta_temporal("sugerencia");
        let wav = carpeta.join("fundidos.wav");
        escribir_con_fundidos(&wav, 1.0, 4.0, 20.0, 6.0);
        let sugerencia = FormaDeOnda::calcular(&wav).unwrap().sugerir();
        // El nivel llega a la mitad a mitad de cada fundido.
        assert!((sugerencia.intro - 3.0).abs() <= 0.4, "{sugerencia:?}");
        assert!((sugerencia.outro - 4.0).abs() <= 0.4, "{sugerencia:?}");
        assert!((sugerencia.silencio_inicial - 1.0).abs() <= 0.15, "{sugerencia:?}");
        assert!((sugerencia.silencio_final - 1.0).abs() <= 0.15, "{sugerencia:?}");
    }

    #[test]
    fn sin_fundidos_no_sugiere_intro_ni_outro() {
        let carpeta = carpeta_temporal("sin-fundidos");
        let wav = carpeta.join("pleno.wav");
        escribir_wav(&wav, 8_000, &[(10.0, 0.7)]);
        let sugerencia = FormaDeOnda::calcular(&wav).unwrap().sugerir();
        assert!(sugerencia.intro <= 0.3 && sugerencia.outro <= 0.3, "{sugerencia:?}");
    }

    #[test]
    fn el_silencio_total_no_rompe_la_sugerencia() {
        let carpeta = carpeta_temporal("silencio");
        let wav = carpeta.join("silencio.wav");
        escribir_wav(&wav, 8_000, &[(3.0, 0.0)]);
        let sugerencia = FormaDeOnda::calcular(&wav).unwrap().sugerir();
        assert_eq!((sugerencia.intro, sugerencia.outro), (0.0, 0.0));
    }

    #[test]
    fn mide_la_sonoridad_segun_la_amplitud() {
        let carpeta = carpeta_temporal("sonoridad");
        let fuerte = carpeta.join("fuerte.wav");
        let suave = carpeta.join("suave.wav");
        escribir_wav(&fuerte, 8_000, &[(5.0, 0.8)]);
        escribir_wav(&suave, 8_000, &[(5.0, 0.2)]);
        let fuerte = FormaDeOnda::calcular(&fuerte).unwrap().sonoridad().unwrap();
        let suave = FormaDeOnda::calcular(&suave).unwrap().sonoridad().unwrap();
        // Un cuarto de la amplitud son unos 12 dB menos.
        assert!((fuerte - suave - 12.04).abs() < 0.5, "fuerte {fuerte} suave {suave}");
        let silencio = carpeta.join("silencio.wav");
        escribir_wav(&silencio, 8_000, &[(3.0, 0.0)]);
        let silencio = FormaDeOnda::calcular(&silencio).unwrap();
        assert_eq!((silencio.sonoridad(), silencio.pico_real(), silencio.rango_dinamico()), (None, None, None));
    }

    #[test]
    fn mide_cuanto_se_aparta_un_tramo_de_lo_tipico() {
        let carpeta = carpeta_temporal("desvio");
        let wav = carpeta.join("dos_partes.wav");
        escribir_wav(&wav, 8_000, &[(10.0, 0.8), (10.0, 0.2)]);
        let onda = FormaDeOnda::calcular(&wav).unwrap();
        assert!(onda.desvio_local_db(2.0, 8.0).unwrap().abs() < 0.5);
        let suave = onda.desvio_local_db(12.0, 18.0).unwrap();
        assert!((suave + 12.04).abs() < 0.5, "{suave}");
    }

    #[test]
    fn mide_el_pico_real_y_el_rango_dinamico() {
        let carpeta = carpeta_temporal("pico-real");
        let parejo = carpeta.join("parejo.wav");
        let variable = carpeta.join("variable.wav");
        escribir_wav(&parejo, 8_000, &[(20.0, 0.8)]);
        escribir_wav(&variable, 8_000, &[(10.0, 0.8), (10.0, 0.1)]);
        let parejo = FormaDeOnda::calcular(&parejo).unwrap();
        let pico = parejo.pico_real().unwrap();
        assert!((pico - 0.8).abs() < 0.02, "pico real {pico}");
        assert!(parejo.rango_dinamico().unwrap() < 1.0, "{:?}", parejo.rango_dinamico());
        // Un octavo de la amplitud son 18 dB de diferencia entre las partes.
        let rango = FormaDeOnda::calcular(&variable).unwrap().rango_dinamico().unwrap();
        assert!(rango > 10.0, "rango dinamico {rango}");
    }

    #[test]
    fn detecta_el_ritmo_de_un_tema_con_golpes() {
        let carpeta = carpeta_temporal("ritmo");
        let wav = carpeta.join("golpes.wav");
        escribir_golpes(&wav, 120.0, 20.0);
        let onda = FormaDeOnda::calcular(&wav).unwrap();
        let tempo = onda.tempo().expect("hay tempo");
        assert!((tempo.bpm - 120.0).abs() < 2.0, "{tempo:?}");
        for ritmo in [onda.ritmo_inicial(), onda.ritmo_final()] {
            let ritmo = ritmo.expect("hay ritmo");
            assert!((ritmo.bpm() - 120.0).abs() < 2.0, "{ritmo:?}");
            // Los golpes empiezan en 0 y caen cada medio segundo.
            let resto = ritmo.fase.rem_euclid(0.5);
            assert!(resto.min(0.5 - resto) < 0.03, "{ritmo:?}");
        }
    }

    /// WAV con un golpe grave (tono de 60 Hz que decae) en cada tiempo.
    pub fn escribir_golpes(ruta: &Path, bpm: f64, segundos: f64) {
        let frecuencia = 8_000u32;
        let periodo = (60.0 / bpm * f64::from(frecuencia)) as usize;
        let muestras: Vec<i16> = (0..(segundos * f64::from(frecuencia)) as usize)
            .map(|n| {
                let desde_el_golpe = (n % periodo) as f32 / frecuencia as f32;
                let tono = (desde_el_golpe * 60.0 * std::f32::consts::TAU).sin();
                (tono * (-desde_el_golpe * 18.0).exp() * 0.8 * f32::from(i16::MAX)) as i16
            })
            .collect();
        escribir_muestras(ruta, frecuencia, &muestras);
    }

    #[test]
    fn dibuja_solo_donde_hay_sonido() {
        let carpeta = carpeta_temporal("dibujo");
        let wav = carpeta.join("tono.wav");
        escribir_wav(&wav, 8_000, &[(1.0, 0.0), (1.0, 0.9)]);
        let onda = FormaDeOnda::calcular(&wav).unwrap();
        let (ancho, alto) = (100u32, 40u32);
        let pixeles = onda.dibujar(ancho, alto, 0.0, 2.0, [10, 20, 30]);
        assert_eq!(pixeles.len(), (ancho * alto * 4) as usize);
        let opacos_en_columna = |columna: usize| {
            (0..alto as usize).filter(|fila| pixeles[(fila * ancho as usize + columna) * 4 + 3] > 0).count()
        };
        assert_eq!(opacos_en_columna(20), 0, "el silencio no se dibuja");
        assert!(opacos_en_columna(80) > alto as usize * 3 / 4, "el tono ocupa casi todo el alto");
    }
}
