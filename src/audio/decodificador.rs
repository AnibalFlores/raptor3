//! Decodificacion de archivos de audio con symphonia: duracion, busqueda y
//! entrega de bloques PCM estereo en f32.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTag};
use symphonia::core::units::{Duration, Time, TimeBase, Timestamp};

use crate::dominio::nombre_de_archivo;

/// Datos de un archivo que no requieren decodificar el audio.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Analisis {
    pub duracion: Option<f64>,
    pub titulo: Option<String>,
    pub artista: Option<String>,
    pub genero: Option<String>,
    pub album: Option<String>,
    pub anio: Option<u16>,
}

/// Datos tecnicos de un archivo de audio, para mostrar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DatosTecnicos {
    /// Formato segun la extension ("MP3", "FLAC").
    pub formato: String,
    pub frecuencia: Option<u32>,
    pub canales: Option<usize>,
    /// Bits por muestra (formatos sin perdida).
    pub bits: Option<u32>,
    /// Tasa media en kbit/s: tamano del archivo sobre su duracion.
    pub kbps: Option<u32>,
}

impl DatosTecnicos {
    /// "MP3 · 44,1 kHz · estéreo · 320 kbps".
    pub fn describir(&self) -> String {
        let mut partes = vec![self.formato.clone()];
        if let Some(frecuencia) = self.frecuencia {
            let khz = f64::from(frecuencia) / 1000.0;
            // Hasta dos decimales, sin ceros de mas: 44,1 kHz, 22,05 kHz, 48 kHz.
            let texto = format!("{khz:.2}").trim_end_matches('0').trim_end_matches('.').to_string();
            partes.push(format!("{} kHz", if crate::idioma::en_ingles() { texto } else { texto.replace('.', ",") }));
        }
        if let Some(bits) = self.bits {
            partes.push(format!("{bits} bits"));
        }
        match self.canales {
            Some(1) => partes.push(tr!("mono")),
            Some(2) => partes.push(tr!("estéreo")),
            Some(canales) => partes.push(tr!("{canales} canales", canales = canales)),
            None => {}
        }
        if let Some(kbps) = self.kbps {
            partes.push(format!("{kbps} kbps"));
        }
        partes.retain(|parte| !parte.is_empty());
        partes.join(" · ")
    }
}

/// Decodificador de una pista; entrega audio estereo intercalado a la
/// frecuencia original del archivo.
pub struct Decodificador {
    formato: Box<dyn FormatReader>,
    decodificador: Box<dyn AudioDecoder>,
    pista_id: u32,
    base_de_tiempo: Option<TimeBase>,
    duracion: Option<f64>,
    // Tras una busqueda, el formato se posiciona antes del punto pedido; las
    // muestras previas a esta marca se descartan para que la posicion sea exacta.
    descartar_hasta: Option<Timestamp>,
    temporal: Vec<f32>,
}

impl Decodificador {
    pub fn abrir(archivo: &Path) -> Result<Self, String> {
        let nombre = nombre_de_archivo(archivo);
        let fuente = File::open(archivo).map_err(|error| tr!("No se pudo abrir {nombre}: {error}", nombre = nombre, error = error))?;
        let flujo = MediaSourceStream::new(Box::new(fuente), Default::default());

        let mut pista = Hint::new();
        if let Some(extension) = archivo.extension().and_then(|extension| extension.to_str()) {
            pista.with_extension(extension);
        }
        let formato = symphonia::default::get_probe()
            .probe(&pista, flujo, FormatOptions::default(), MetadataOptions::default())
            .map_err(|error| tr!("Formato no reconocido en {nombre}: {error}", nombre = nombre, error = error))?;

        let track = formato
            .default_track(TrackType::Audio)
            .ok_or_else(|| tr!("{nombre} no tiene una pista de audio", nombre = nombre))?;
        let parametros = track
            .codec_params
            .as_ref()
            .and_then(|parametros| parametros.audio())
            .ok_or_else(|| tr!("{nombre} no tiene parametros de audio", nombre = nombre))?;
        let decodificador = symphonia::default::get_codecs()
            .make_audio_decoder(parametros, &AudioDecoderOptions::default())
            .map_err(|error| tr!("Codec no soportado en {nombre}: {error}", nombre = nombre, error = error))?;

        let pista_id = track.id;
        let base_de_tiempo = track
            .time_base
            .or_else(|| parametros.sample_rate.and_then(TimeBase::try_from_recip));
        let duracion = match (track.duration, track.num_frames, base_de_tiempo) {
            (Some(duracion), _, Some(base)) => base.calc_duration(duracion).map(|tiempo| tiempo.as_secs_f64()),
            (None, Some(frames), _) => parametros.sample_rate.map(|frecuencia| frames as f64 / frecuencia as f64),
            _ => None,
        };

        Ok(Self {
            formato,
            decodificador,
            pista_id,
            base_de_tiempo,
            duracion,
            descartar_hasta: None,
            temporal: Vec::new(),
        })
    }

    /// Formato, muestreo, canales, bits y tasa del archivo; solo lee el
    /// encabezado (la duracion, si el contenedor no la declara, se recibe).
    pub fn datos_tecnicos(archivo: &Path, duracion_conocida: Option<f64>) -> Result<DatosTecnicos, String> {
        let decodificador = Self::abrir(archivo)?;
        let parametros = decodificador
            .formato
            .tracks()
            .iter()
            .find(|pista| pista.id == decodificador.pista_id)
            .and_then(|pista| pista.codec_params.as_ref())
            .and_then(|parametros| parametros.audio())
            .cloned();
        let duracion = decodificador.duracion.or(duracion_conocida).filter(|segundos| *segundos > 0.0);
        let tamano = std::fs::metadata(archivo).map(|datos| datos.len()).ok();
        Ok(DatosTecnicos {
            formato: archivo.extension().map(|extension| extension.to_string_lossy().to_uppercase()).unwrap_or_default(),
            frecuencia: parametros.as_ref().and_then(|parametros| parametros.sample_rate),
            canales: parametros.as_ref().and_then(|parametros| parametros.channels.as_ref()).map(|canales| canales.count()),
            bits: parametros.as_ref().and_then(|parametros| parametros.bits_per_sample),
            kbps: tamano.zip(duracion).map(|(bytes, segundos)| (bytes as f64 * 8.0 / segundos / 1000.0).round() as u32),
        })
    }

    /// Duracion declarada por el contenedor; si no la declara (algunos MP3 sin
    /// encabezado Xing), se calcula sumando la duracion de los paquetes, sin
    /// decodificarlos.
    pub fn sondear_duracion(archivo: &Path) -> Result<Option<f64>, String> {
        Self::abrir(archivo)?.calcular_duracion()
    }

    /// Duracion y etiquetas (titulo, artista, genero) del archivo. Si no
    /// trae titulo ni artista, el titulo es el nombre del archivo sin la
    /// extension.
    pub fn analizar(archivo: &Path) -> Result<Analisis, String> {
        let mut decodificador = Self::abrir(archivo)?;
        let mut analisis = decodificador.leer_etiquetas_o_nombre(archivo);
        analisis.duracion = decodificador.calcular_duracion()?;
        Ok(analisis)
    }

    /// Solo las etiquetas (sin duracion), que se leen sin recorrer el
    /// archivo: alcanza para comparar temas al agregarlos.
    pub fn etiquetas(archivo: &Path) -> Result<Analisis, String> {
        Ok(Self::abrir(archivo)?.leer_etiquetas_o_nombre(archivo))
    }

    /// Solo lo que dicen las etiquetas, sin completar el titulo con el nombre
    /// del archivo: para releerlas despues de corregirlas con otro programa.
    pub fn solo_etiquetas(archivo: &Path) -> Result<Analisis, String> {
        Ok(Self::abrir(archivo)?.leer_etiquetas())
    }

    /// Etiquetas del archivo; sin titulo ni artista, el titulo es el nombre
    /// del archivo sin la extension.
    fn leer_etiquetas_o_nombre(&mut self, archivo: &Path) -> Analisis {
        let mut analisis = self.leer_etiquetas();
        if analisis.titulo.is_none() && analisis.artista.is_none() {
            analisis.titulo = titulo_desde_el_nombre(archivo);
        }
        analisis
    }

    fn leer_etiquetas(&mut self) -> Analisis {
        let mut analisis = Analisis::default();
        let mut metadatos = self.formato.metadata();
        let Some(revision) = metadatos.skip_to_latest() else {
            return analisis;
        };
        let etiquetas = revision.media.tags.iter().chain(revision.per_track.iter().flat_map(|pista| &pista.metadata.tags));
        for etiqueta in etiquetas {
            let anio = match &etiqueta.std {
                Some(StandardTag::RecordingYear(anio) | StandardTag::ReleaseYear(anio)) => Some(*anio),
                Some(StandardTag::RecordingDate(fecha) | StandardTag::ReleaseDate(fecha)) => anio_de_fecha(fecha),
                _ => None,
            };
            if let Some(anio) = anio.filter(|anio| anio_valido(*anio)) {
                analisis.anio.get_or_insert(anio);
                continue;
            }
            let (destino, valor) = match &etiqueta.std {
                Some(StandardTag::TrackTitle(valor)) => (&mut analisis.titulo, valor),
                Some(StandardTag::Artist(valor)) => (&mut analisis.artista, valor),
                Some(StandardTag::Genre(valor)) => (&mut analisis.genero, valor),
                Some(StandardTag::Album(valor)) => (&mut analisis.album, valor),
                _ => continue,
            };
            let valor = valor.trim();
            if destino.is_none() && !valor.is_empty() {
                *destino = Some(valor.to_string());
            }
        }
        analisis
    }

    fn calcular_duracion(&mut self) -> Result<Option<f64>, String> {
        let decodificador = self;
        if decodificador.duracion.is_some() {
            return Ok(decodificador.duracion);
        }
        let Some(base) = decodificador.base_de_tiempo else {
            return Ok(None);
        };
        let mut total = Duration::new(0);
        loop {
            match decodificador.formato.next_packet() {
                Ok(Some(paquete)) if paquete.track_id == decodificador.pista_id => {
                    total = total.saturating_add(paquete.dur);
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            }
        }
        Ok(base.calc_duration(total).map(|tiempo| tiempo.as_secs_f64()).filter(|segundos| *segundos > 0.0))
    }

    /// Posiciona la lectura en un instante de la pista.
    pub fn buscar(&mut self, segundos: f64) -> Result<(), String> {
        if segundos <= 0.0 {
            return Ok(());
        }
        let tiempo = Time::try_from_secs_f64(segundos).ok_or_else(|| tr!("Posicion de busqueda invalida"))?;
        let destino = self
            .formato
            .seek(SeekMode::Accurate, SeekTo::Time { time: tiempo, track_id: Some(self.pista_id) })
            .map_err(|error| tr!("No se pudo buscar la posicion: {error}", error = error))?;
        self.decodificador.reset();
        self.descartar_hasta = Some(destino.required_ts);
        Ok(())
    }

    /// Decodifica el siguiente bloque y lo agrega a `salida` como estereo
    /// intercalado. Devuelve la frecuencia del bloque, o None al terminar.
    pub fn siguiente_bloque(&mut self, salida: &mut Vec<f32>) -> Option<u32> {
        loop {
            let paquete = match self.formato.next_packet() {
                Ok(Some(paquete)) => paquete,
                Ok(None) => return None,
                Err(error) => {
                    log::warn!("Fin anticipado de la lectura: {error}");
                    return None;
                }
            };
            if paquete.track_id != self.pista_id {
                continue;
            }
            let audio = match self.decodificador.decode(&paquete) {
                Ok(audio) => audio,
                // Un paquete danado se saltea, como haria cualquier reproductor.
                Err(Error::DecodeError(_)) => continue,
                Err(error) => {
                    log::warn!("No se pudo decodificar: {error}");
                    return None;
                }
            };

            let frecuencia = audio.spec().rate();
            let canales = audio.spec().channels().count().max(1);
            let frames = audio.frames();
            if frames == 0 {
                continue;
            }
            self.temporal.resize(audio.samples_interleaved(), 0.0);
            audio.copy_to_slice_interleaved(&mut self.temporal[..]);

            let omitir = self.frames_a_descartar(paquete.pts, frames, frecuencia);
            if omitir >= frames {
                continue;
            }
            for frame in self.temporal.chunks_exact(canales).skip(omitir) {
                let izquierdo = frame[0];
                let derecho = if canales > 1 { frame[1] } else { izquierdo };
                salida.push(izquierdo);
                salida.push(derecho);
            }
            return Some(frecuencia);
        }
    }

    fn frames_a_descartar(&mut self, pts: Timestamp, frames: usize, frecuencia: u32) -> usize {
        let (Some(hasta), Some(base)) = (self.descartar_hasta, self.base_de_tiempo) else {
            return 0;
        };
        if pts.get() >= hasta.get() {
            self.descartar_hasta = None;
            return 0;
        }
        let segundos = base
            .calc_duration(Duration::new((hasta.get() - pts.get()) as u64))
            .map(|tiempo| tiempo.as_secs_f64())
            .unwrap_or(0.0);
        let omitir = (segundos * frecuencia as f64).round() as usize;
        if omitir < frames {
            self.descartar_hasta = None;
        }
        omitir
    }
}

/// Nombre del archivo sin la extension, como titulo de reserva.
fn titulo_desde_el_nombre(archivo: &Path) -> Option<String> {
    let nombre = archivo.file_stem()?.to_string_lossy();
    let nombre = nombre.trim();
    (!nombre.is_empty()).then(|| nombre.to_string())
}

/// Anios que se aceptan para un tema.
pub fn anio_valido(anio: u16) -> bool {
    (1000..=2999).contains(&anio)
}

/// Anio de una fecha de etiqueta ("2001", "2001-05-12", "2001/05").
fn anio_de_fecha(fecha: &str) -> Option<u16> {
    let digitos: String = fecha.trim().chars().take(4).collect();
    (digitos.len() == 4).then(|| digitos.parse().ok()).flatten()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn describe_los_datos_tecnicos() {
        let datos = DatosTecnicos { formato: "FLAC".into(), frecuencia: Some(44_100), canales: Some(2), bits: Some(24), kbps: Some(1_012) };
        assert_eq!(datos.describir(), "FLAC · 44,1 kHz · 24 bits · estéreo · 1012 kbps");
        let mp3 = DatosTecnicos { formato: "MP3".into(), frecuencia: Some(48_000), canales: Some(1), bits: None, kbps: Some(128) };
        assert_eq!(mp3.describir(), "MP3 · 48 kHz · mono · 128 kbps");
        let wav = DatosTecnicos { formato: "WAV".into(), frecuencia: Some(22_050), canales: Some(6), bits: Some(16), kbps: None };
        assert_eq!(wav.describir(), "WAV · 22,05 kHz · 16 bits · 6 canales");
    }

    #[test]
    fn el_anio_sale_de_las_fechas_de_las_etiquetas() {
        assert_eq!(anio_de_fecha("2001-05-12"), Some(2001));
        assert_eq!(anio_de_fecha(" 1987 "), Some(1987));
        assert_eq!(anio_de_fecha("87"), None);
        assert_eq!(anio_de_fecha("sin fecha"), None);
        assert!(anio_valido(1950) && !anio_valido(999) && !anio_valido(3000));
    }

    #[test]
    fn el_titulo_de_reserva_es_el_nombre_sin_extension() {
        assert_eq!(titulo_desde_el_nombre(Path::new("/musica/Banda - Tema.mp3")).as_deref(), Some("Banda - Tema"));
        assert_eq!(titulo_desde_el_nombre(Path::new("C:/musica/tema.final.flac")).as_deref(), Some("tema.final"));
        assert_eq!(titulo_desde_el_nombre(Path::new("/")), None);
    }
}
