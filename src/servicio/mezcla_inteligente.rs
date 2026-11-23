//! Crossfade inteligente: decide como mezclar dos pistas a partir de su
//! analisis (silencios, fundidos naturales, sonoridad y ritmo).
//!
//! - Puntos automaticos: el analisis fija el inicio y el fin de cada tema
//!   (sin los silencios), su intro y su outro.
//! - Recorte de silencios: la mezcla termina en el fin de la que sale y la
//!   que entra arranca en su inicio, sin huecos muertos.
//! - Duracion adaptativa: dura lo que la mas larga entre la outro de la que
//!   sale y la intro de la que entra, entre 2 y 12 segundos.
//! - Curvas de potencia constante; una pista que ya tiene un fundido natural
//!   en ese tramo no se atenua de nuevo, para evitar el "bache" de volumen.
//! - Alineacion de golpes: si el final de una y el principio de la otra
//!   tienen el mismo tempo (tambien al doble o a la mitad), la entrante se
//!   corre menos de medio golpe para que caigan juntos.
//! - Nivelacion: cada tema se lleva a la sonoridad elegida (-14, -11 u -8
//!   LUFS, ver `ObjetivoDeSonoridad`), con limites de subida y bajada y un
//!   techo de pico real propios de cada objetivo.

use super::gestor_de_reproduccion::{Curva, Transicion};
use super::sugerencia_de_overlay::{AnalisisParaOverlay, SugerenciaDeOverlay, sugerir_overlay};
use crate::audio::{FormaDeOnda, Ritmo, Tempo, desfase_para_alinear, tempos_coinciden};
use crate::dominio::{Pista, PuntosDeMezcla};

const SEGUNDOS_MINIMOS: f64 = 2.0;
const SEGUNDOS_MAXIMOS: f64 = 12.0;
// Un fundido natural que cubre esta fraccion de la mezcla la reemplaza.
const COBERTURA_DE_FUNDIDO_NATURAL: f64 = 0.75;
// Por debajo de esta confianza el ritmo no se usa para alinear.
const CONFIANZA_MINIMA_DE_RITMO: f64 = 0.3;
/// Acuerdo entre ventanas desde el que el tempo votado se acepta solo.
const ACUERDO_MINIMO_DE_TEMPO: f64 = 0.5;
/// Confianza minima de una estimacion para que cuente al buscar dos que
/// coincidan; por debajo es casi ruido.
const CONFIANZA_PARA_COINCIDIR: f64 = 0.05;
/// Sonoridad de referencia de los efectos cuando no suena musica, en LUFS.
pub const SONORIDAD_DE_REFERENCIA: f64 = -11.0;
/// Pico real maximo al subir el volumen, en dBTP: deja margen para los picos
/// entre muestras.
pub const TECHO_DE_PICO_REAL_DB: f64 = -1.0;
/// Lo que mas puede subir un aviso para llegar al techo: lo que admite el
/// motor de audio (+6 dB).
const SUBIDA_MAXIMA_DE_UN_AVISO_DB: f64 = 6.0;

/// Sonoridad integrada a la que la nivelacion lleva cada tema. Cada objetivo
/// tiene sus propios limites, porque no se llega igual a uno bajo (casi todo
/// se baja) que a uno alto (casi todo se sube y los picos mandan):
///
/// - -14 LUFS: la referencia de las plataformas de streaming. Los masters
///   actuales (-9 a -6 LUFS) bajan mucho, asi que se admite bajar hasta
///   10 dB; se sube como mucho 6 dB y sin pasar de -1 dBTP. Conserva toda la
///   dinamica: para ambientes tranquilos o musica muy dinamica.
/// - -11 LUFS: punto medio (entre los -12 y -10 que conservan impacto y
///   microdinamica). Hasta 6 dB arriba o abajo y sin pasar de -1 dBTP.
/// - -8 LUFS: sonido "de radio" o de pista de baile. Casi todo se sube, y con
///   un techo de -1 dBTP muchos temas no llegarian; por eso se deja que los
///   picos lleguen a +1.5 dBTP y los redondea el recorte suave del
///   mezclador (que empieza en -0.9 dBFS): unos 2 dB de compresion breve en
///   los transitorios, inaudible en musica ya comprimida. Se sube hasta
///   8 dB y se baja hasta 4. Los temas muy dinamicos (rango de sonoridad de
///   15 LU o mas) conservan el techo de -1 dBTP: saturar sus picos se nota.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ObjetivoDeSonoridad {
    Menos14,
    #[default]
    Menos11,
    Menos8,
}

impl ObjetivoDeSonoridad {
    /// Los objetivos en el orden en que se ofrecen.
    pub const TODOS: [Self; 3] = [Self::Menos14, Self::Menos11, Self::Menos8];

    pub fn lufs(self) -> f64 {
        match self {
            Self::Menos14 => -14.0,
            Self::Menos11 => -11.0,
            Self::Menos8 => -8.0,
        }
    }

    /// El objetivo de esa sonoridad, si es una de las ofrecidas.
    pub fn de_lufs(lufs: f64) -> Option<Self> {
        Self::TODOS.into_iter().find(|objetivo| (objetivo.lufs() - lufs).abs() < 0.01)
    }

    /// Posicion en `TODOS`, para el combo de la ventana.
    pub fn indice(self) -> usize {
        Self::TODOS.iter().position(|objetivo| *objetivo == self).unwrap_or(1)
    }

    fn subida_maxima_db(self) -> f64 {
        match self {
            Self::Menos14 | Self::Menos11 => 6.0,
            Self::Menos8 => 8.0,
        }
    }

    fn bajada_maxima_db(self) -> f64 {
        match self {
            Self::Menos14 => 10.0,
            Self::Menos11 => 6.0,
            Self::Menos8 => 4.0,
        }
    }

    /// Pico real hasta el que se puede subir un tema con ese rango de
    /// sonoridad, en dBTP.
    fn techo_de_pico_db(self, rango_dinamico: Option<f64>) -> f64 {
        let dinamico = rango_dinamico.is_some_and(|rango| rango >= RANGO_DINAMICO_AMPLIO);
        match self {
            Self::Menos8 if !dinamico => 1.5,
            _ => TECHO_DE_PICO_REAL_DB,
        }
    }

    /// Resumen para la barra de estado y las herramientas MCP.
    pub fn descripcion(self) -> String {
        let techo = match self {
            Self::Menos8 => tr!("picos hasta +1.5 dBTP, redondeados por el recorte suave"),
            _ => tr!("sin pasar de {TECHO_DE_PICO_REAL_DB} dBTP", TECHO_DE_PICO_REAL_DB = TECHO_DE_PICO_REAL_DB),
        };
        tr!("{} LUFS (hasta {} dB arriba y {} dB abajo, {techo})", self.lufs(), self.subida_maxima_db(), self.bajada_maxima_db(), techo = techo)
    }
}
/// Rango de sonoridad (LRA) a partir del cual un tema se marca como muy
/// dinamico: en escucha continua sus partes suaves quedan bajas.
pub const RANGO_DINAMICO_AMPLIO: f64 = 15.0;

/// Lo que el crossfade inteligente necesita saber de cada tema; sale de su
/// forma de onda.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DatosDeMezcla {
    pub duracion: f64,
    pub silencio_inicial: f64,
    pub silencio_final: f64,
    /// Segundos que tarda en llegar a su nivel tipico desde el primer sonido.
    pub fundido_de_entrada: f64,
    /// Segundos que tarda en apagarse desde que deja su nivel tipico.
    pub fundido_de_salida: f64,
    pub sonoridad: Option<f64>,
    /// Pico real (o el de las muestras si no se pudo medir), de 0 a 1.
    pub pico: f64,
    /// Rango de sonoridad (LRA) en LU.
    pub rango_dinamico: Option<f64>,
    pub ritmo_inicial: Option<Ritmo>,
    pub ritmo_final: Option<Ritmo>,
    /// Tempo de todo el tema votado entre ventanas, para mostrar y ordenar
    /// por BPM.
    pub tempo: Option<Tempo>,
    /// Ventana de overlay sobre un tramo instrumental, si hay uno claro.
    pub overlay_sugerido: Option<SugerenciaDeOverlay>,
}

impl DatosDeMezcla {
    pub fn de_la_onda(onda: &FormaDeOnda) -> Self {
        let sugerencia = onda.sugerir();
        let voz = onda.voz();
        let niveles: Vec<f32> = (0..voz.len())
            .map(|tramo| onda.tramo(tramo as f64 / 10.0, (tramo + 1) as f64 / 10.0).rms)
            .collect();
        let fin = onda.duracion() - sugerencia.silencio_final;
        let overlay_sugerido = sugerir_overlay(&AnalisisParaOverlay {
            voz: &voz,
            niveles: &niveles,
            inicio: sugerencia.silencio_inicial,
            fin,
            fin_de_la_musica: onda.duracion() - sugerencia.outro.max(sugerencia.silencio_final),
            ritmo_inicial: onda.ritmo_inicial(),
            ritmo_final: onda.ritmo_final(),
        });
        Self {
            overlay_sugerido,
            duracion: onda.duracion(),
            silencio_inicial: sugerencia.silencio_inicial,
            silencio_final: sugerencia.silencio_final,
            fundido_de_entrada: (sugerencia.intro - sugerencia.silencio_inicial).max(0.0),
            fundido_de_salida: (sugerencia.outro - sugerencia.silencio_final).max(0.0),
            sonoridad: onda.sonoridad(),
            pico: onda.pico_real().unwrap_or_else(|| f64::from(onda.pico_maximo())),
            rango_dinamico: onda.rango_dinamico(),
            ritmo_inicial: onda.ritmo_inicial(),
            ritmo_final: onda.ritmo_final(),
            tempo: onda.tempo(),
        }
    }

    /// Factor de volumen con que suena el elemento: un aviso (`a_volumen_maximo`)
    /// va al maximo sin distorsionar, este o no la nivelacion; lo demas, a la
    /// sonoridad objetivo si se nivela, o como esta.
    pub fn ganancia(&self, objetivo: ObjetivoDeSonoridad, nivelar: bool, a_volumen_maximo: bool) -> f64 {
        if a_volumen_maximo {
            self.ganancia_maxima()
        } else if nivelar {
            self.ganancia_de_nivelacion(objetivo)
        } else {
            1.0
        }
    }

    /// Factor que sube el elemento hasta que su pico real toque el techo
    /// (-1 dBTP), sin pasar de +6 dB ni bajarlo nunca: el maximo volumen sin
    /// distorsion, para que un aviso se destaque de la musica.
    pub fn ganancia_maxima(&self) -> f64 {
        if self.pico <= 0.0 {
            return 1.0;
        }
        let techo = 10f64.powf(TECHO_DE_PICO_REAL_DB / 20.0);
        (techo / self.pico).clamp(1.0, 10f64.powf(SUBIDA_MAXIMA_DE_UN_AVISO_DB / 20.0))
    }

    /// Factor de volumen que lleva el tema a la sonoridad objetivo.
    pub fn ganancia_de_nivelacion(&self, objetivo: ObjetivoDeSonoridad) -> f64 {
        let Some(sonoridad) = self.sonoridad else {
            return 1.0;
        };
        let decibeles = (objetivo.lufs() - sonoridad).clamp(-objetivo.bajada_maxima_db(), objetivo.subida_maxima_db());
        let ganancia = 10f64.powf(decibeles / 20.0);
        let techo = 10f64.powf(objetivo.techo_de_pico_db(self.rango_dinamico) / 20.0);
        if ganancia > 1.0 && self.pico > 0.0 { ganancia.min((techo / self.pico).max(1.0)) } else { ganancia }
    }

    /// Pico real en dBTP.
    pub fn pico_real_db(&self) -> Option<f64> {
        (self.pico > 0.0).then(|| 20.0 * self.pico.log10())
    }

    /// Puntos que el analisis propone: el tema empieza en su primer sonido y
    /// termina en el ultimo; la intro llega hasta que alcanza su nivel y la
    /// outro empieza cuando deja de tenerlo.
    /// La ventana de overlay es la sugerida, si la hay.
    pub fn puntos_automaticos(&self) -> PuntosDeMezcla {
        let overlay = self.overlay_sugerido.map(|sugerencia| sugerencia.ventana);
        let silencios = (self.silencio_inicial + self.silencio_final).min(self.duracion);
        let (inicial, final_) = if silencios < self.duracion - 0.5 { (self.silencio_inicial, self.silencio_final) } else { (0.0, 0.0) };
        let redondear = |segundos: f64| (segundos * 100.0).round() / 100.0;
        PuntosDeMezcla::nuevos(
            redondear((inicial + self.fundido_de_entrada).min(self.duracion)),
            redondear((final_ + self.fundido_de_salida).min(self.duracion)),
            overlay,
        )
        .and_then(|puntos| puntos.con_recortes(redondear(inicial), redondear(final_)))
        .unwrap_or_default()
    }

    /// Tempo del tema, si hay evidencia suficiente, en este orden:
    /// - el votado entre ventanas, si la mayoria coincide;
    /// - el del final o el del principio, si su patron se repite claramente;
    /// - el de dos estimaciones (votado, principio, final) que coinciden,
    ///   iguales o una el doble de la otra, aunque cada una sea debil. Gana
    ///   el votado si es una de ellas; si no, la de mas confianza.
    pub fn bpm(&self) -> Option<f64> {
        estimar_bpm(self.tempo, self.ritmo_inicial, self.ritmo_final)
    }
}

/// El BPM de un tema con lo que guarda la cache de su onda (sin cargarla
/// entera), con el mismo criterio que el analisis. Para el catalogo.
pub fn bpm_de_la_cache(archivo: &std::path::Path, carpeta_de_cache: &std::path::Path) -> Option<f64> {
    let (tempo, ritmo_inicial, ritmo_final) = FormaDeOnda::ritmo_de_la_cache(archivo, carpeta_de_cache)?;
    estimar_bpm(tempo, ritmo_inicial, ritmo_final)
}

/// Ver `DatosDeMezcla::bpm`.
fn estimar_bpm(tempo: Option<Tempo>, ritmo_inicial: Option<Ritmo>, ritmo_final: Option<Ritmo>) -> Option<f64> {
    let votado = tempo.filter(|tempo| tempo.ventanas > 1);
    if let Some(tempo) = votado.filter(|tempo| tempo.acuerdo >= ACUERDO_MINIMO_DE_TEMPO) {
        return Some(tempo.bpm);
    }
    if let Some(ritmo) = confiable(ritmo_final).or(confiable(ritmo_inicial)) {
        return Some(ritmo.bpm());
    }
    let candidatos: Vec<(f64, f64)> = votado
        .map(|tempo| (tempo.bpm, f64::INFINITY))
        .into_iter()
        .chain(
            [ritmo_final, ritmo_inicial]
                .into_iter()
                .flatten()
                .filter(|ritmo| ritmo.confianza >= CONFIANZA_PARA_COINCIDIR)
                .map(|ritmo| (ritmo.bpm(), ritmo.confianza)),
        )
        .collect();
    candidatos
        .iter()
        .enumerate()
        .filter(|&(indice, &(bpm, _))| {
            candidatos.iter().enumerate().any(|(otro, &(otro_bpm, _))| otro != indice && tempos_coinciden(bpm, otro_bpm))
        })
        .max_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
        .map(|(_, &(bpm, _))| bpm)
}

fn confiable(ritmo: Option<Ritmo>) -> Option<Ritmo> {
    ritmo.filter(|ritmo| ritmo.confianza >= CONFIANZA_MINIMA_DE_RITMO)
}

/// Mezcla entre la pista que termina (`saliente`) y la siguiente.
pub fn transicion_inteligente(
    siguiente: Pista,
    saliente: (&PuntosDeMezcla, &DatosDeMezcla),
    entrante: (&PuntosDeMezcla, &DatosDeMezcla),
) -> Transicion {
    let ((puntos_saliente, a), (puntos_entrante, b)) = (saliente, entrante);
    let fin = (a.duracion - puntos_saliente.recorte_final()).max(0.0);
    let inicio = puntos_entrante.recorte_inicial().min(b.duracion);
    // La mezcla no ocupa mas de la mitad del sonido de ninguna de las dos.
    let tope = ((fin - puntos_saliente.recorte_inicial()).min(b.duracion - puntos_entrante.recorte_final() - inicio) / 2.0).max(0.0);
    let (outro, intro) = (puntos_saliente.outro_audible(), puntos_entrante.intro_audible());
    let segundos = outro.max(intro).clamp(SEGUNDOS_MINIMOS, SEGUNDOS_MAXIMOS).min(tope);
    let curva = Curva::Potencia {
        saliente_natural: a.fundido_de_salida >= segundos * COBERTURA_DE_FUNDIDO_NATURAL,
        entrante_natural: b.fundido_de_entrada >= segundos * COBERTURA_DE_FUNDIDO_NATURAL,
    };
    let alineacion = match (confiable(a.ritmo_final), confiable(b.ritmo_inicial)) {
        (Some(ritmo_a), Some(ritmo_b)) if segundos >= SEGUNDOS_MINIMOS => {
            desfase_para_alinear(&ritmo_a, &ritmo_b, fin - segundos, inicio).map(|desfase| (desfase, ritmo_a.bpm()))
        }
        _ => None,
    };
    let mut detalle = tr!("mezcla inteligente de {segundos:.1} s", segundos = segundos);
    if let Some((_, bpm)) = alineacion {
        detalle.push_str(&tr!(", golpes alineados a {bpm:.0} BPM", bpm = bpm));
    }
    Transicion {
        siguiente,
        segundos,
        inicio,
        recorte_final: puntos_saliente.recorte_final(),
        desfase: alineacion.map_or(0.0, |(desfase, _)| desfase),
        outro_saliente: puntos_saliente.outro(),
        curva,
        sincronizada: true,
        detalle,
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn datos(duracion: f64) -> DatosDeMezcla {
        DatosDeMezcla {
            duracion,
            silencio_inicial: 0.0,
            silencio_final: 0.0,
            fundido_de_entrada: 0.0,
            fundido_de_salida: 0.0,
            sonoridad: Some(-14.0),
            pico: 0.9,
            rango_dinamico: None,
            ritmo_inicial: None,
            ritmo_final: None,
            tempo: None,
            overlay_sugerido: None,
        }
    }

    fn puntos(intro: f64, outro: f64) -> PuntosDeMezcla {
        PuntosDeMezcla::nuevos(intro, outro, None).unwrap()
    }

    fn mezclar(a: (&PuntosDeMezcla, &DatosDeMezcla), b: (&PuntosDeMezcla, &DatosDeMezcla)) -> Transicion {
        transicion_inteligente(Pista::new("b.mp3"), a, b)
    }

    #[test]
    fn saltea_los_silencios_de_las_dos_pistas() {
        let (a, b) = (datos(200.0), datos(180.0));
        let puntos_a = puntos(0.0, 6.0).con_recortes(0.0, 3.0).unwrap();
        let puntos_b = puntos(4.0, 0.0).con_recortes(1.5, 0.0).unwrap();
        let transicion = mezclar((&puntos_a, &a), (&puntos_b, &b));
        assert_eq!(transicion.recorte_final, 3.0);
        assert_eq!(transicion.inicio, 1.5);
        // Outro 6 - 3 de silencio = 3; intro 4 - 1.5 de silencio = 2.5.
        assert_eq!(transicion.segundos, 3.0, "la mas larga entre outro e intro, sin los silencios");
        assert!(transicion.sincronizada);
    }

    #[test]
    fn la_duracion_queda_entre_2_y_12_segundos() {
        let (a, b) = (datos(200.0), datos(200.0));
        assert_eq!(mezclar((&puntos(0.0, 0.0), &a), (&puntos(0.0, 0.0), &b)).segundos, 2.0);
        assert_eq!(mezclar((&puntos(0.0, 30.0), &a), (&puntos(0.0, 0.0), &b)).segundos, 12.0);
        // Con temas cortos, no mas de la mitad de su sonido.
        let corto = datos(6.0);
        assert_eq!(mezclar((&puntos(0.0, 10.0), &a), (&puntos(0.0, 0.0), &corto)).segundos, 3.0);
    }

    #[test]
    fn no_atenua_de_nuevo_un_fundido_natural() {
        let a = DatosDeMezcla { fundido_de_salida: 8.0, ..datos(200.0) };
        let b = datos(200.0);
        let transicion = mezclar((&puntos(0.0, 8.0), &a), (&puntos(0.0, 0.0), &b));
        assert_eq!(transicion.curva, Curva::Potencia { saliente_natural: true, entrante_natural: false });
        let b = DatosDeMezcla { fundido_de_entrada: 1.0, ..datos(200.0) };
        let transicion = mezclar((&puntos(0.0, 8.0), &datos(200.0)), (&puntos(0.0, 0.0), &b));
        assert_eq!(transicion.curva, Curva::Potencia { saliente_natural: false, entrante_natural: false });
    }

    #[test]
    fn alinea_los_golpes_si_el_tempo_coincide() {
        let golpes = |fase: f64, bpm: f64| Some(Ritmo { periodo: 60.0 / bpm, fase, confianza: 0.8 });
        let a = DatosDeMezcla { ritmo_final: golpes(0.1, 120.0), ..datos(100.0) };
        let b = DatosDeMezcla { ritmo_inicial: golpes(0.3, 121.0), ..datos(100.0) };
        let transicion = mezclar((&puntos(0.0, 6.0), &a), (&puntos(0.0, 0.0), &b));
        assert!(transicion.desfase.abs() <= 0.25 && transicion.desfase != 0.0, "{}", transicion.desfase);
        assert!(transicion.detalle.contains("120 BPM"), "{}", transicion.detalle);

        let b = DatosDeMezcla { ritmo_inicial: golpes(0.3, 100.0), ..datos(100.0) };
        assert_eq!(mezclar((&puntos(0.0, 6.0), &a), (&puntos(0.0, 0.0), &b)).desfase, 0.0, "tempos distintos");
        let dudoso = DatosDeMezcla { ritmo_inicial: Some(Ritmo { confianza: 0.1, ..golpes(0.3, 120.0).unwrap() }), ..datos(100.0) };
        assert_eq!(mezclar((&puntos(0.0, 6.0), &a), (&puntos(0.0, 0.0), &dudoso)).desfase, 0.0, "ritmo poco confiable");
    }

    #[test]
    fn propone_inicio_fin_intro_y_outro() {
        let analizado = DatosDeMezcla {
            silencio_inicial: 1.0,
            silencio_final: 2.0,
            fundido_de_entrada: 3.0,
            fundido_de_salida: 5.0,
            ..datos(100.0)
        };
        let puntos = analizado.puntos_automaticos();
        assert_eq!((puntos.recorte_inicial(), puntos.recorte_final()), (1.0, 2.0));
        assert_eq!((puntos.intro_audible(), puntos.outro_audible()), (3.0, 5.0));
        assert!(!puntos.manuales());
        // Un archivo todo silencio no se recorta.
        let mudo = DatosDeMezcla { silencio_inicial: 10.0, silencio_final: 10.0, ..datos(10.0) };
        assert_eq!(mudo.puntos_automaticos().recorte_inicial(), 0.0);
    }

    #[test]
    fn sugiere_el_overlay_en_la_intro_de_una_cancion() {
        use crate::audio::pruebas::{cancion, escribir_estereo};
        let carpeta = std::env::temp_dir().join(format!("raptor-overlay-{}", std::process::id()));
        let wav = carpeta.join("cancion.wav");
        let frecuencia = 22_050;
        escribir_estereo(&wav, frecuencia, &cancion(frecuencia, &[(9.0, false), (20.0, true), (6.0, false), (15.0, true), (8.0, false)]));
        let datos = DatosDeMezcla::de_la_onda(&FormaDeOnda::calcular(&wav).unwrap());
        let sugerencia = datos.overlay_sugerido.expect("hay sugerencia");
        let ventana = sugerencia.ventana;
        assert!(ventana.inicio() < 1.0 && (ventana.fin() - 8.7).abs() < 0.6, "{sugerencia:?}");
        assert_eq!(datos.puntos_automaticos().overlay(), Some(ventana));
        let _ = std::fs::remove_dir_all(carpeta);
    }

    #[test]
    fn nivela_hacia_la_referencia_con_limites() {
        let con = |sonoridad: f64, pico: f64| DatosDeMezcla { sonoridad: Some(sonoridad), pico, ..datos(100.0) };
        let decibeles = |ganancia: f64| 20.0 * ganancia.log10();
        let medio = ObjetivoDeSonoridad::Menos11;
        assert!((decibeles(con(-5.0, 1.0).ganancia_de_nivelacion(medio)) + 6.0).abs() < 1e-9, "baja 6 dB");
        assert!((decibeles(con(-8.0, 1.0).ganancia_de_nivelacion(medio)) + 3.0).abs() < 1e-9, "baja a -11");
        assert!((decibeles(con(-30.0, 0.1).ganancia_de_nivelacion(medio)) - 6.0).abs() < 1e-9, "sube como mucho 6 dB");
        // Sube solo hasta que el pico real llega a -1 dBTP.
        let pico = 10f64.powf(-4.0 / 20.0);
        assert!((decibeles(con(-16.0, pico).ganancia_de_nivelacion(medio)) - 3.0).abs() < 1e-9);
        // Si ya pasa del techo no lo sube, pero tampoco lo baja por eso.
        assert_eq!(con(-14.0, 0.95).ganancia_de_nivelacion(medio), 1.0);
        assert_eq!(DatosDeMezcla { sonoridad: None, ..datos(10.0) }.ganancia_de_nivelacion(medio), 1.0);
    }

    #[test]
    fn un_aviso_suena_al_maximo_volumen_sin_distorsionar() {
        let con = |sonoridad: f64, pico_db: f64| DatosDeMezcla { sonoridad: Some(sonoridad), pico: 10f64.powf(pico_db / 20.0), ..datos(30.0) };
        let decibeles = |ganancia: f64| 20.0 * ganancia.log10();
        let medio = ObjetivoDeSonoridad::Menos14;
        // Un aviso fuerte (-9 LUFS) no baja aunque se nivele: queda como esta
        // si ya toca el techo...
        let fuerte = con(-9.0, -0.5);
        assert_eq!(fuerte.ganancia(medio, true, true), 1.0);
        assert!(decibeles(fuerte.ganancia(medio, true, false)) < -4.0, "un tema igual de fuerte si baja");
        // ...y sube hasta que su pico llega a -1 dBTP, con o sin nivelacion.
        let suave = con(-20.0, -4.0);
        assert!((decibeles(suave.ganancia(medio, false, true)) - 3.0).abs() < 1e-9);
        assert!((decibeles(suave.ganancia(medio, true, true)) - 3.0).abs() < 1e-9);
        // Como mucho +6 dB, lo que admite el motor.
        assert!((decibeles(con(-30.0, -20.0).ganancia_maxima()) - 6.0).abs() < 1e-9);
        // Sin nivelacion, lo que no es un aviso suena como esta.
        assert_eq!(suave.ganancia(medio, false, false), 1.0);
        assert_eq!(DatosDeMezcla { pico: 0.0, ..datos(10.0) }.ganancia_maxima(), 1.0);
    }

    #[test]
    fn cada_objetivo_de_sonoridad_tiene_sus_limites() {
        let con = |sonoridad: f64, pico: f64| DatosDeMezcla { sonoridad: Some(sonoridad), pico, ..datos(100.0) };
        let decibeles = |ganancia: f64| 20.0 * ganancia.log10();
        let pico_bajo = 10f64.powf(-12.0 / 20.0);
        // -14: un master a -6 LUFS baja 8 dB (a -11 solo bajaria 6).
        assert!((decibeles(con(-6.0, 1.0).ganancia_de_nivelacion(ObjetivoDeSonoridad::Menos14)) + 8.0).abs() < 1e-9);
        assert!((decibeles(con(-2.0, 1.0).ganancia_de_nivelacion(ObjetivoDeSonoridad::Menos14)) + 10.0).abs() < 1e-9);
        // -8: sube hasta 8 dB y baja como mucho 4.
        assert!((decibeles(con(-20.0, pico_bajo).ganancia_de_nivelacion(ObjetivoDeSonoridad::Menos8)) - 8.0).abs() < 1e-9);
        assert!((decibeles(con(-2.0, 1.0).ganancia_de_nivelacion(ObjetivoDeSonoridad::Menos8)) + 4.0).abs() < 1e-9);
        // -8: los picos pueden llegar a +1.5 dBTP, salvo en temas muy dinamicos.
        let pico = 10f64.powf(-2.0 / 20.0);
        assert!((decibeles(con(-12.0, pico).ganancia_de_nivelacion(ObjetivoDeSonoridad::Menos8)) - 3.5).abs() < 1e-9);
        let dinamico = DatosDeMezcla { rango_dinamico: Some(RANGO_DINAMICO_AMPLIO), ..con(-12.0, pico) };
        assert!((decibeles(dinamico.ganancia_de_nivelacion(ObjetivoDeSonoridad::Menos8)) - 1.0).abs() < 1e-9);
        // Las sonoridades guardadas se reconocen; otras no.
        assert_eq!(ObjetivoDeSonoridad::de_lufs(-8.0), Some(ObjetivoDeSonoridad::Menos8));
        assert_eq!(ObjetivoDeSonoridad::de_lufs(-9.0), None);
        assert_eq!(ObjetivoDeSonoridad::Menos14.indice(), 0);
    }

    #[test]
    fn el_bpm_se_acepta_por_votacion_por_ritmo_claro_o_por_coincidencia() {
        let ritmo = |bpm: f64, confianza: f64| Some(Ritmo { periodo: 60.0 / bpm, fase: 0.0, confianza });
        let tempo = |bpm: f64, acuerdo: f64| Some(Tempo { bpm, acuerdo, ventanas: 12 });
        // La mayoria de las ventanas coincide.
        assert_eq!(DatosDeMezcla { tempo: tempo(124.0, 0.7), ..datos(200.0) }.bpm(), Some(124.0));
        // Votacion dudosa, pero el final tiene un patron claro.
        let claro = DatosDeMezcla { tempo: tempo(90.0, 0.2), ritmo_final: ritmo(128.0, 0.5), ..datos(200.0) };
        assert!((claro.bpm().unwrap() - 128.0).abs() < 1e-9);
        // Todo debil, pero votado y principio coinciden: gana el votado.
        let coinciden = DatosDeMezcla { tempo: tempo(106.9, 0.3), ritmo_inicial: ritmo(106.8, 0.14), ..datos(200.0) };
        assert_eq!(coinciden.bpm(), Some(106.9));
        // Principio y final al doble: gana el de mas confianza.
        let al_doble = DatosDeMezcla { ritmo_inicial: ritmo(83.0, 0.1), ritmo_final: ritmo(166.0, 0.2), ..datos(200.0) };
        assert!((al_doble.bpm().unwrap() - 166.0).abs() < 1e-9);
        // Nada coincide y nada es claro.
        let dudoso = DatosDeMezcla { tempo: tempo(90.0, 0.2), ritmo_inicial: ritmo(130.0, 0.1), ritmo_final: ritmo(75.0, 0.02), ..datos(200.0) };
        assert_eq!(dudoso.bpm(), None);
    }
}
