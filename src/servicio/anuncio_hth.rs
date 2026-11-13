//! Anuncio de hora, temperatura y humedad (HTH) armado sin internet con
//! locuciones pregrabadas: «Son las quince» + «y treinta.» +
//! «Temperatura: veintidos grados.» + «Humedad: sesenta y cinco por ciento.»
//!
//! Carpeta de locuciones:
//!
//! ```text
//! bases/                                    (opcional: cortinas de fondo)
//! hora/00.mp3 … hora/23.mp3
//! minutos/00.mp3 … minutos/59.mp3          (00 = «en punto»)
//! temperatura/-10.mp3 … temperatura/50.mp3
//! humedad/000.mp3 … humedad/100.mp3
//! ```
//!
//! Con bases, el anuncio es una base musical de la lista y la voz suena
//! encima, como su efecto fijado, dentro de la ventana de overlay de la base.
//!
//! Cada pieza puede tener una segunda toma con apostrofo («15'.mp3»); en cada
//! anuncio se elige al azar una de las dos por pieza, para que no suene
//! siempre identico.

use std::fs;
use std::path::{Path, PathBuf};

use crate::audio::Decodificador;
use crate::dominio::Pista;
use crate::servicio::clima::Clima;

/// Silencio antes de la primera pieza y despues de la ultima, en segundos.
const MARGEN_INICIAL: f64 = 0.05;
const MARGEN_FINAL: f64 = 0.2;
/// Nivel por debajo del cual se considera silencio al recortar las puntas
/// de cada pieza (-40 dBFS).
const UMBRAL_DE_SILENCIO: f32 = 0.01;
/// Lo que se conserva antes y despues del sonido de cada pieza al recortarla,
/// para no comerse el ataque de una consonante ni la cola de una vocal.
const RESPIRO_ANTES: f64 = 0.02;
const RESPIRO_DESPUES: f64 = 0.05;
/// Fundido en las puntas recortadas, para que no hagan clic.
const FUNDIDO: f64 = 0.008;
/// Pico al que se lleva el anuncio completo (-1 dBFS) y ganancia maxima para
/// llegar a el.
const PICO_OBJETIVO: f32 = 0.891;
const GANANCIA_MAXIMA: f32 = 4.0;

/// Una pieza del anuncio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pieza {
    Hora(u8),
    Minutos(u8),
    Temperatura(i32),
    Humedad(u8),
}

impl Pieza {
    fn carpeta(self) -> &'static str {
        match self {
            Pieza::Hora(_) => "hora",
            Pieza::Minutos(_) => "minutos",
            Pieza::Temperatura(_) => "temperatura",
            Pieza::Humedad(_) => "humedad",
        }
    }

    /// Nombre del archivo sin extension: «07», «-3», «065».
    fn nombre(self) -> String {
        match self {
            Pieza::Hora(valor) | Pieza::Minutos(valor) => format!("{valor:02}"),
            Pieza::Temperatura(valor) => valor.to_string(),
            Pieza::Humedad(valor) => format!("{valor:03}"),
        }
    }

    /// Silencio que sigue a la pieza: casi nada entre la hora y los minutos
    /// (son una misma frase) y una pausa de punto entre frases.
    fn pausa_posterior(self) -> f64 {
        match self {
            Pieza::Hora(_) => 0.04,
            Pieza::Minutos(_) => 0.45,
            Pieza::Temperatura(_) => 0.4,
            Pieza::Humedad(_) => 0.0,
        }
    }

    /// La hora y los minutos son imprescindibles; si falta la temperatura o
    /// la humedad (por ejemplo, un valor fuera del rango grabado), el anuncio
    /// sale sin ella.
    fn es_imprescindible(self) -> bool {
        matches!(self, Pieza::Hora(_) | Pieza::Minutos(_))
    }
}

/// Piezas del anuncio para una hora (0 a 23) y minutos (0 a 59), con el
/// clima si se conoce.
pub fn piezas_del_anuncio(hora: u8, minutos: u8, clima: Option<Clima>) -> Vec<Pieza> {
    let mut piezas = vec![Pieza::Hora(hora % 24), Pieza::Minutos(minutos % 60)];
    if let Some(clima) = clima {
        piezas.push(Pieza::Temperatura(clima.temperatura.round() as i32));
        piezas.push(Pieza::Humedad(clima.humedad.round().clamp(0.0, 100.0) as u8));
    }
    piezas
}

/// Texto breve de lo que dice el anuncio, para la barra de estado.
pub fn resumen(hora: u8, minutos: u8, clima: Option<Clima>) -> String {
    match clima {
        Some(clima) => tr!("{hora:02}:{minutos:02}, {} °C, {} % de humedad", clima.temperatura.round() as i32, clima.humedad.round().clamp(0.0, 100.0) as u8, hora = hora, minutos = minutos),
        None => tr!("{hora:02}:{minutos:02} (sin datos del clima)", hora = hora, minutos = minutos),
    }
}

/// Carpeta con las locuciones pregrabadas.
#[derive(Clone, Debug, PartialEq)]
pub struct Locuciones {
    carpeta: PathBuf,
}

impl Locuciones {
    pub fn new(carpeta: impl Into<PathBuf>) -> Self {
        Self { carpeta: carpeta.into() }
    }

    pub fn carpeta(&self) -> &Path {
        &self.carpeta
    }

    /// Tomas disponibles de una pieza: la principal y la alternativa («'»).
    pub fn tomas(&self, pieza: Pieza) -> Vec<PathBuf> {
        let carpeta = self.carpeta.join(pieza.carpeta());
        let nombre = pieza.nombre();
        [format!("{nombre}.mp3"), format!("{nombre}'.mp3")]
            .into_iter()
            .map(|archivo| carpeta.join(archivo))
            .filter(|archivo| archivo.is_file())
            .collect()
    }

    /// Piezas de hora y minutos que no tienen ninguna toma: sin ellas no se
    /// puede anunciar la hora. Vacio si la carpeta sirve.
    pub fn faltantes(&self) -> Vec<String> {
        let horas = (0..24).map(Pieza::Hora);
        let minutos = (0..60).map(Pieza::Minutos);
        horas
            .chain(minutos)
            .filter(|pieza| self.tomas(*pieza).is_empty())
            .map(|pieza| format!("{}/{}.mp3", pieza.carpeta(), pieza.nombre()))
            .collect()
    }

    /// Si la carpeta tiene todas las piezas de la hora.
    pub fn sirve(&self) -> bool {
        self.faltantes().is_empty()
    }

    /// Carpeta de las bases musicales sobre las que se dice el anuncio.
    pub fn carpeta_de_bases(&self) -> PathBuf {
        self.carpeta.join("bases")
    }

    /// Bases musicales (archivos de audio de `bases`), por nombre.
    pub fn bases(&self) -> Vec<PathBuf> {
        let Ok(entradas) = fs::read_dir(self.carpeta_de_bases()) else {
            return Vec::new();
        };
        let mut bases: Vec<PathBuf> = entradas
            .filter_map(|entrada| entrada.ok().map(|entrada| entrada.path()))
            .filter(|archivo| archivo.is_file() && Pista::es_formato_soportado(archivo))
            .collect();
        bases.sort();
        bases
    }
}

/// Elige una base al azar, sin repetir la ultima si hay mas de una.
pub fn elegir_base<'a>(bases: &'a [PathBuf], ultima: Option<&Path>, elegir: &mut dyn FnMut(usize) -> usize) -> Option<&'a PathBuf> {
    let candidatas: Vec<&PathBuf> = bases.iter().filter(|base| bases.len() == 1 || Some(base.as_path()) != ultima).collect();
    let candidatas = if candidatas.is_empty() { bases.iter().collect() } else { candidatas };
    candidatas.get(elegir(candidatas.len()).min(candidatas.len().saturating_sub(1))).copied()
}

/// Lo que se armo: la duracion y las tomas que se usaron.
#[derive(Clone, Debug, PartialEq)]
pub struct AnuncioArmado {
    pub duracion: f64,
    pub tomas: Vec<PathBuf>,
}

/// Arma el anuncio con las piezas y lo escribe como WAV en `destino`.
/// `elegir(cantidad)` decide que toma usar (de 0 a cantidad - 1).
pub fn armar(
    locuciones: &Locuciones,
    piezas: &[Pieza],
    elegir: &mut dyn FnMut(usize) -> usize,
    destino: &Path,
) -> Result<AnuncioArmado, String> {
    let mut frecuencia: Option<u32> = None;
    let mut audio: Vec<f32> = Vec::new();
    let mut tomas_usadas = Vec::new();
    let mut pausa_pendiente = MARGEN_INICIAL;

    for &pieza in piezas {
        let tomas = locuciones.tomas(pieza);
        if tomas.is_empty() {
            if pieza.es_imprescindible() {
                return Err(tr!("Falta la locucion {}/{}.mp3 en {}", pieza.carpeta(), pieza.nombre(), locuciones.carpeta().display()));
            }
            log::warn!("Sin locucion para {pieza:?}: el anuncio sale sin ese dato");
            continue;
        }
        let toma = &tomas[elegir(tomas.len()).min(tomas.len() - 1)];
        let (frecuencia_de_la_toma, muestras) = decodificar(toma)?;
        let frecuencia = *frecuencia.get_or_insert(frecuencia_de_la_toma);
        if frecuencia_de_la_toma != frecuencia {
            return Err(tr!("{} esta a {frecuencia_de_la_toma} Hz y las demas locuciones a {frecuencia} Hz", toma.display(), frecuencia_de_la_toma = frecuencia_de_la_toma, frecuencia = frecuencia));
        }
        agregar_silencio(&mut audio, frecuencia, pausa_pendiente);
        let mut voz = recortar_silencio(&muestras, frecuencia).to_vec();
        aplicar_fundidos(&mut voz, frecuencia);
        audio.extend_from_slice(&voz);
        pausa_pendiente = pieza.pausa_posterior();
        tomas_usadas.push(toma.clone());
    }

    let frecuencia = frecuencia.ok_or_else(|| tr!("El anuncio no tiene ninguna pieza"))?;
    agregar_silencio(&mut audio, frecuencia, MARGEN_FINAL);
    normalizar(&mut audio);
    escribir_wav(destino, frecuencia, &audio)?;
    Ok(AnuncioArmado { duracion: audio.len() as f64 / 2.0 / f64::from(frecuencia), tomas: tomas_usadas })
}

/// Audio estereo intercalado de un archivo, y su frecuencia.
fn decodificar(archivo: &Path) -> Result<(u32, Vec<f32>), String> {
    let mut decodificador = Decodificador::abrir(archivo)?;
    let mut muestras = Vec::new();
    let mut frecuencia = None;
    while let Some(frecuencia_del_bloque) = decodificador.siguiente_bloque(&mut muestras) {
        frecuencia.get_or_insert(frecuencia_del_bloque);
    }
    let frecuencia = frecuencia.ok_or_else(|| tr!("{} no tiene audio", archivo.display()))?;
    Ok((frecuencia, muestras))
}

fn agregar_silencio(audio: &mut Vec<f32>, frecuencia: u32, segundos: f64) {
    let frames = (segundos * f64::from(frecuencia)).round() as usize;
    audio.resize(audio.len() + frames * 2, 0.0);
}

/// El tramo con sonido de una pieza (estereo intercalado), con un respiro a
/// cada lado. Si es todo silencio, la devuelve entera.
fn recortar_silencio(muestras: &[f32], frecuencia: u32) -> &[f32] {
    let suena = |frame: &[f32]| frame.iter().any(|muestra| muestra.abs() >= UMBRAL_DE_SILENCIO);
    let frames: &[[f32; 2]] = muestras.as_chunks::<2>().0;
    let (Some(primero), Some(ultimo)) =
        (frames.iter().position(|frame| suena(frame)), frames.iter().rposition(|frame| suena(frame)))
    else {
        return muestras;
    };
    let respiro = |segundos: f64| (segundos * f64::from(frecuencia)).round() as usize;
    let desde = primero.saturating_sub(respiro(RESPIRO_ANTES));
    let hasta = (ultimo + 1 + respiro(RESPIRO_DESPUES)).min(frames.len());
    &muestras[desde * 2..hasta * 2]
}

/// Fundido de entrada y de salida muy corto en las puntas.
fn aplicar_fundidos(voz: &mut [f32], frecuencia: u32) {
    let frames = voz.len() / 2;
    let largo = ((FUNDIDO * f64::from(frecuencia)) as usize).min(frames / 2).max(1);
    for indice in 0..largo {
        let factor = indice as f32 / largo as f32;
        for canal in 0..2 {
            voz[indice * 2 + canal] *= factor;
            voz[(frames - 1 - indice) * 2 + canal] *= factor;
        }
    }
}

/// Lleva el pico del anuncio a -1 dBFS (sin subirlo mas de 12 dB).
fn normalizar(audio: &mut [f32]) {
    let pico = audio.iter().fold(0.0f32, |pico, muestra| pico.max(muestra.abs()));
    if pico <= f32::EPSILON {
        return;
    }
    let ganancia = (PICO_OBJETIVO / pico).min(GANANCIA_MAXIMA);
    audio.iter_mut().for_each(|muestra| *muestra *= ganancia);
}

/// WAV estereo PCM de 16 bits. Se escribe aparte y se renombra, para que nunca
/// quede un archivo a medias en el lugar del anuncio.
fn escribir_wav(destino: &Path, frecuencia: u32, audio: &[f32]) -> Result<(), String> {
    let bytes_de_datos = u32::try_from(audio.len() * 2).map_err(|_| tr!("Anuncio demasiado largo"))?;
    let mut wav = Vec::with_capacity(44 + audio.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes_de_datos).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&frecuencia.to_le_bytes());
    wav.extend_from_slice(&(frecuencia * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes_de_datos.to_le_bytes());
    for muestra in audio {
        let entera = (muestra.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        wav.extend_from_slice(&entera.to_le_bytes());
    }
    if let Some(carpeta) = destino.parent() {
        fs::create_dir_all(carpeta).map_err(|error| tr!("No se pudo crear {}: {error}", carpeta.display(), error = error))?;
    }
    let temporal = destino.with_extension("wav.parcial");
    fs::write(&temporal, wav).map_err(|error| tr!("No se pudo escribir {}: {error}", temporal.display(), error = error))?;
    fs::rename(&temporal, destino).map_err(|error| tr!("No se pudo guardar {}: {error}", destino.display(), error = error))
}

/// Generador pseudoaleatorio chico (xorshift) para elegir las tomas: no hace
/// falta calidad criptografica, solo que no se repita siempre igual.
pub struct Azar(u64);

impl Azar {
    /// Semilla distinta en cada ejecucion.
    pub fn nuevo() -> Self {
        use std::hash::{BuildHasher, Hasher};
        let mut mezcla = std::collections::hash_map::RandomState::new().build_hasher();
        mezcla.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |lapso| lapso.as_nanos()));
        Self(mezcla.finish() | 1)
    }

    /// Un numero de 0 a cantidad - 1.
    pub fn elegir(&mut self, cantidad: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % cantidad.max(1) as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::pruebas::{carpeta_temporal, escribir_wav as escribir_tono};

    /// Carpeta de locuciones de prueba: tonos WAV con el nombre de cada pieza
    /// (el decodificador no mira la extension para reconocer el formato).
    fn locuciones_de_prueba(nombre: &str, con_clima: bool) -> Locuciones {
        let carpeta = carpeta_temporal(nombre);
        let mut piezas: Vec<Pieza> = (0..24).map(Pieza::Hora).chain((0..60).map(Pieza::Minutos)).collect();
        if con_clima {
            piezas.extend((-10..=50).map(Pieza::Temperatura));
            piezas.extend((0..=100).map(Pieza::Humedad));
        }
        for pieza in piezas {
            let archivo = carpeta.join(pieza.carpeta()).join(format!("{}.mp3", pieza.nombre()));
            // 0,1 s de silencio, 0,5 s de voz y 0,3 s de silencio.
            escribir_tono(&archivo, 44_100, &[(0.1, 0.0), (0.5, 0.5), (0.3, 0.0)]);
        }
        Locuciones::new(carpeta)
    }

    #[test]
    fn el_anuncio_lleva_hora_minutos_y_clima_redondeados() {
        let clima = Clima { temperatura: 17.6, humedad: 75.4 };
        assert_eq!(
            piezas_del_anuncio(15, 30, Some(clima)),
            vec![Pieza::Hora(15), Pieza::Minutos(30), Pieza::Temperatura(18), Pieza::Humedad(75)]
        );
        assert_eq!(piezas_del_anuncio(0, 0, None), vec![Pieza::Hora(0), Pieza::Minutos(0)]);
        let bajo_cero = Clima { temperatura: -2.4, humedad: 100.3 };
        assert_eq!(piezas_del_anuncio(7, 5, Some(bajo_cero))[2..], [Pieza::Temperatura(-2), Pieza::Humedad(100)]);
    }

    #[test]
    fn los_nombres_de_archivo_siguen_la_estructura_de_carpetas() {
        assert_eq!(Pieza::Hora(7).nombre(), "07");
        assert_eq!(Pieza::Minutos(0).nombre(), "00");
        assert_eq!(Pieza::Temperatura(-3).nombre(), "-3");
        assert_eq!(Pieza::Temperatura(22).nombre(), "22");
        assert_eq!(Pieza::Humedad(5).nombre(), "005");
        assert_eq!(Pieza::Humedad(100).nombre(), "100");
    }

    #[test]
    fn se_usan_las_dos_tomas_de_cada_pieza() {
        let locuciones = locuciones_de_prueba("hth-tomas", false);
        let carpeta = locuciones.carpeta().join("hora");
        fs::copy(carpeta.join("15.mp3"), carpeta.join("15'.mp3")).unwrap();
        assert_eq!(locuciones.tomas(Pieza::Hora(15)).len(), 2);
        assert_eq!(locuciones.tomas(Pieza::Hora(14)).len(), 1);
        assert!(locuciones.tomas(Pieza::Temperatura(20)).is_empty());

        let destino = locuciones.carpeta().join("anuncio.wav");
        let piezas = [Pieza::Hora(15), Pieza::Minutos(30)];
        let primera = armar(&locuciones, &piezas, &mut |_| 0, &destino).unwrap();
        let segunda = armar(&locuciones, &piezas, &mut |cantidad| cantidad - 1, &destino).unwrap();
        assert_eq!(primera.tomas[0], carpeta.join("15.mp3"));
        assert_eq!(segunda.tomas[0], carpeta.join("15'.mp3"));
    }

    #[test]
    fn el_anuncio_recorta_los_silencios_y_pone_pausas_entre_frases() {
        let locuciones = locuciones_de_prueba("hth-armado", true);
        let destino = locuciones.carpeta().join("servicios").join("anuncio.wav");
        let clima = Clima { temperatura: 22.0, humedad: 65.0 };
        let armado =
            armar(&locuciones, &piezas_del_anuncio(15, 30, Some(clima)), &mut |_| 0, &destino).unwrap();
        assert_eq!(armado.tomas.len(), 4);
        // Cuatro voces de 0,5 s (mas los respiros) y las pausas, sin los
        // 0,4 s de silencio que trae cada archivo.
        let esperado = MARGEN_INICIAL
            + 4.0 * (0.5 + RESPIRO_ANTES + RESPIRO_DESPUES)
            + Pieza::Hora(0).pausa_posterior()
            + Pieza::Minutos(0).pausa_posterior()
            + Pieza::Temperatura(0).pausa_posterior()
            + MARGEN_FINAL;
        assert!((armado.duracion - esperado).abs() < 0.03, "duro {} y se esperaba {esperado}", armado.duracion);
        let leido = Decodificador::sondear_duracion(&destino).unwrap().unwrap();
        assert!((leido - armado.duracion).abs() < 0.01);
        assert!(!destino.with_extension("wav.parcial").exists());
    }

    #[test]
    fn sin_la_locucion_del_clima_sale_solo_la_hora() {
        let locuciones = locuciones_de_prueba("hth-sin-clima", false);
        let destino = locuciones.carpeta().join("anuncio.wav");
        let clima = Clima { temperatura: 60.0, humedad: 10.0 };
        let armado = armar(&locuciones, &piezas_del_anuncio(9, 0, Some(clima)), &mut |_| 0, &destino).unwrap();
        assert_eq!(armado.tomas.len(), 2);
    }

    #[test]
    fn sin_una_pieza_de_la_hora_no_hay_anuncio() {
        let locuciones = locuciones_de_prueba("hth-incompleta", false);
        fs::remove_file(locuciones.carpeta().join("minutos").join("30.mp3")).unwrap();
        assert_eq!(locuciones.faltantes(), vec!["minutos/30.mp3".to_string()]);
        assert!(!locuciones.sirve());
        let destino = locuciones.carpeta().join("anuncio.wav");
        let error = armar(&locuciones, &piezas_del_anuncio(15, 30, None), &mut |_| 0, &destino).unwrap_err();
        assert!(error.contains("minutos/30.mp3"), "{error}");
        assert!(Locuciones::new(locuciones.carpeta().join("no-existe")).faltantes().len() == 84);
    }

    #[test]
    fn las_bases_son_los_audios_de_su_carpeta() {
        let locuciones = locuciones_de_prueba("hth-bases", false);
        assert!(locuciones.bases().is_empty());
        let bases = locuciones.carpeta_de_bases();
        fs::create_dir_all(&bases).unwrap();
        fs::write(bases.join("Base 2.mp3"), b"x").unwrap();
        fs::write(bases.join("Base 1.mp3"), b"x").unwrap();
        fs::write(bases.join("notas.txt"), b"x").unwrap();
        assert_eq!(locuciones.bases(), vec![bases.join("Base 1.mp3"), bases.join("Base 2.mp3")]);
    }

    #[test]
    fn la_base_no_repite_la_ultima() {
        let bases = vec![PathBuf::from("a.mp3"), PathBuf::from("b.mp3"), PathBuf::from("c.mp3")];
        for eleccion in 0..3 {
            let elegida = elegir_base(&bases, Some(Path::new("b.mp3")), &mut |_| eleccion).unwrap();
            assert_ne!(elegida, &PathBuf::from("b.mp3"));
        }
        let una = vec![PathBuf::from("a.mp3")];
        assert_eq!(elegir_base(&una, Some(Path::new("a.mp3")), &mut |_| 0), Some(&una[0]));
        assert_eq!(elegir_base(&[], None, &mut |_| 0), None);
    }

    #[test]
    fn el_azar_reparte_entre_las_tomas() {
        let mut azar = Azar::nuevo();
        let elegidas: Vec<usize> = (0..200).map(|_| azar.elegir(2)).collect();
        assert!(elegidas.iter().all(|&eleccion| eleccion < 2));
        let unos = elegidas.iter().filter(|&&eleccion| eleccion == 1).count();
        assert!((40..=160).contains(&unos), "{unos} de 200");
        assert_eq!(azar.elegir(1), 0);
    }

    #[test]
    fn el_resumen_dice_lo_que_se_anuncia() {
        assert_eq!(resumen(9, 5, Some(Clima { temperatura: -0.4, humedad: 81.6 })), "09:05, 0 °C, 82 % de humedad");
        assert_eq!(resumen(21, 0, None), "21:00 (sin datos del clima)");
    }
}
