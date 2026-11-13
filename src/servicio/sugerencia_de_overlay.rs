//! Sugerencia de la ventana de overlay: el tramo instrumental del tema donde
//! mejor cae un efecto de la botonera (una identificacion, una voz), como
//! hacen las radios sobre la intro.
//!
//! 1. Con el puntaje de voz de cada decima de segundo se separan los tramos
//!    con voz de los instrumentales (umbral de Otsu dentro del tema), se
//!    rellenan huecos cortos y se descartan voces sueltas.
//! 2. Candidatos: el tramo instrumental del principio (la intro), los
//!    puentes y el del final antes del fundido, de al menos 4 segundos.
//! 3. Cada candidato se puntua por su largo, que el nivel sea parejo y no
//!    demasiado bajo, que tan lejos de la voz queda su puntaje y su lugar:
//!    se prefiere la intro, despues la outro y por ultimo un puente.
//! 4. La ventana empieza en un golpe, si el ritmo es claro, y termina un
//!    poco antes de que entre la voz.
//!
//! Si el tema no muestra una diferencia clara entre partes con y sin voz, o
//! ningun candidato convence, no se sugiere nada.

use crate::audio::Ritmo;
use crate::dominio::VentanaDeOverlay;

/// Tramos por segundo del puntaje de voz.
const TRAMOS_POR_SEGUNDO: f64 = 10.0;
/// Largo minimo de la ventana, en segundos.
pub const LARGO_MINIMO: f64 = 4.0;
// Largo a partir del cual una ventana ya no suma por ser mas larga.
const LARGO_IDEAL: f64 = 10.0;
// Huecos sin voz mas cortos que esto se consideran parte de la voz, y voces
// mas cortas que esto se ignoran (segundos).
const HUECO_MAXIMO: f64 = 1.0;
const VOZ_MINIMA: f64 = 1.5;
// Margen antes de que entre la voz y despues del primer sonido.
const MARGEN_ANTES_DE_LA_VOZ: f64 = 0.3;
const MARGEN_AL_EMPEZAR: f64 = 0.2;
// Variacion de nivel (dB de desvio) a partir de la cual el tramo ya no
// cuenta como parejo.
const VARIACION_TOLERADA_DB: f64 = 6.0;
// Por debajo de esta confianza no se sugiere nada.
const CONFIANZA_MINIMA: f64 = 0.25;
const CONFIANZA_MINIMA_DE_RITMO: f64 = 0.3;

/// Parte del tema en que cae la ventana sugerida.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zona {
    Intro,
    Puente,
    Outro,
}

impl Zona {
    pub fn nombre(self) -> String {
        match self {
            Zona::Intro => tr!("intro instrumental"),
            Zona::Puente => tr!("puente instrumental"),
            Zona::Outro => tr!("final instrumental"),
        }
    }

    fn peso(self) -> f64 {
        match self {
            Zona::Intro => 1.0,
            Zona::Outro => 0.85,
            Zona::Puente => 0.7,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SugerenciaDeOverlay {
    pub ventana: VentanaDeOverlay,
    /// De 0 a 1.
    pub confianza: f64,
    pub zona: Zona,
}

/// Lo que la sugerencia necesita del analisis de un tema.
pub struct AnalisisParaOverlay<'a> {
    /// Puntaje de voz de cada decima de segundo, de 0 a 1.
    pub voz: &'a [f32],
    /// Nivel RMS de cada decima de segundo.
    pub niveles: &'a [f32],
    /// Primer y ultimo sonido, en segundos.
    pub inicio: f64,
    pub fin: f64,
    /// Donde empieza el fundido final; la ventana no entra en el.
    pub fin_de_la_musica: f64,
    pub ritmo_inicial: Option<Ritmo>,
    pub ritmo_final: Option<Ritmo>,
}

/// Umbral que mejor separa los valores en dos grupos (metodo de Otsu), con
/// la media de cada grupo y que tan separados quedan (0 a 1).
fn otsu(valores: &[f32]) -> Option<(f32, f32, f32, f64)> {
    const CAJAS: usize = 64;
    let mut histograma = [0usize; CAJAS];
    for valor in valores {
        histograma[((valor.clamp(0.0, 1.0) * (CAJAS - 1) as f32).round()) as usize] += 1;
    }
    let total = valores.len() as f64;
    let centro = |caja: usize| caja as f64 / (CAJAS - 1) as f64;
    let media: f64 = histograma.iter().enumerate().map(|(caja, cantidad)| centro(caja) * *cantidad as f64).sum::<f64>() / total;
    let varianza: f64 = histograma.iter().enumerate().map(|(caja, cantidad)| (centro(caja) - media).powi(2) * *cantidad as f64).sum::<f64>() / total;
    if varianza <= 1e-9 {
        return None;
    }
    let (mut peso_bajo, mut suma_baja) = (0.0, 0.0);
    let mut mejor: Option<(f64, usize, f64, f64)> = None;
    for caja in 0..CAJAS - 1 {
        peso_bajo += histograma[caja] as f64 / total;
        suma_baja += centro(caja) * histograma[caja] as f64 / total;
        let peso_alto = 1.0 - peso_bajo;
        if peso_bajo <= 0.0 || peso_alto <= 0.0 {
            continue;
        }
        let (media_baja, media_alta) = (suma_baja / peso_bajo, (media - suma_baja) / peso_alto);
        let entre = peso_bajo * peso_alto * (media_baja - media_alta).powi(2);
        if mejor.is_none_or(|(maximo, ..)| entre > maximo) {
            mejor = Some((entre, caja, media_baja, media_alta));
        }
    }
    let (entre, caja, media_baja, media_alta) = mejor?;
    Some(((centro(caja) + 0.5 / (CAJAS - 1) as f64) as f32, media_baja as f32, media_alta as f32, entre / varianza))
}

/// Marca con voz cada tramo, rellenando huecos cortos y quitando voces sueltas.
fn tramos_con_voz(voz: &[f32], umbral: f32) -> Vec<bool> {
    let mut marcas: Vec<bool> = voz.iter().map(|puntaje| *puntaje > umbral).collect();
    let corridas = |marcas: &[bool], valor: bool| -> Vec<(usize, usize)> {
        let mut resultado = Vec::new();
        let mut desde = None;
        for (indice, marca) in marcas.iter().chain(std::iter::once(&!valor)).enumerate() {
            match (desde, *marca == valor) {
                (None, true) => desde = Some(indice),
                (Some(inicio), false) => {
                    resultado.push((inicio, indice));
                    desde = None;
                }
                _ => {}
            }
        }
        resultado
    };
    let hueco = (HUECO_MAXIMO * TRAMOS_POR_SEGUNDO) as usize;
    for (desde, hasta) in corridas(&marcas, false) {
        if hasta - desde <= hueco && desde > 0 && hasta < marcas.len() {
            marcas[desde..hasta].fill(true);
        }
    }
    let minima = (VOZ_MINIMA * TRAMOS_POR_SEGUNDO) as usize;
    for (desde, hasta) in corridas(&marcas, true) {
        if hasta - desde < minima {
            marcas[desde..hasta].fill(false);
        }
    }
    marcas
}

/// Primer golpe en `desde` o despues, si el ritmo es claro.
fn golpe_desde(ritmo: Option<Ritmo>, desde: f64) -> Option<f64> {
    let ritmo = ritmo.filter(|ritmo| ritmo.confianza >= CONFIANZA_MINIMA_DE_RITMO && ritmo.periodo > 0.0)?;
    Some(ritmo.fase + ((desde - ritmo.fase) / ritmo.periodo).ceil() * ritmo.periodo)
}

pub fn sugerir_overlay(analisis: &AnalisisParaOverlay) -> Option<SugerenciaDeOverlay> {
    let segundos = |indice: usize| indice as f64 / TRAMOS_POR_SEGUNDO;
    let indice = |segundos: f64| ((segundos * TRAMOS_POR_SEGUNDO).round().max(0.0) as usize).min(analisis.voz.len());
    let (primero, ultimo) = (indice(analisis.inicio), indice(analisis.fin));
    if ultimo <= primero + (LARGO_MINIMO * TRAMOS_POR_SEGUNDO) as usize {
        return None;
    }
    let sonido = &analisis.voz[primero..ultimo];
    let (umbral, media_sin_voz, media_con_voz, separacion) = otsu(sonido)?;
    let con_voz = tramos_con_voz(sonido, umbral);
    let decibeles = |nivel: f32| 20.0 * f64::from(nivel.max(1e-6)).log10();
    let mut niveles: Vec<f64> = analisis.niveles.get(primero..ultimo).unwrap_or(&[]).iter().map(|nivel| decibeles(*nivel)).collect();
    let nivel_tipico = {
        let mut ordenados = niveles.clone();
        ordenados.sort_by(f64::total_cmp);
        ordenados.get(ordenados.len() / 2).copied().unwrap_or(-20.0)
    };
    niveles.resize(sonido.len(), nivel_tipico);
    let ultimo_de_musica = indice(analisis.fin_de_la_musica).clamp(primero, ultimo) - primero;
    let hay_voz_despues = |hasta: usize| con_voz[hasta..].iter().any(|marca| *marca);

    let mut mejor: Option<(f64, usize, usize, Zona)> = None;
    let mut desde = 0;
    while desde < ultimo_de_musica {
        if con_voz[desde] {
            desde += 1;
            continue;
        }
        let hasta = (desde..ultimo_de_musica).find(|&tramo| con_voz[tramo]).unwrap_or(ultimo_de_musica);
        let zona = if desde == 0 {
            Zona::Intro
        } else if !hay_voz_despues(hasta) {
            Zona::Outro
        } else {
            Zona::Puente
        };
        let largo = segundos(hasta - desde);
        if largo >= LARGO_MINIMO {
            let tramo = &niveles[desde..hasta];
            let media = tramo.iter().sum::<f64>() / tramo.len() as f64;
            let desvio = (tramo.iter().map(|nivel| (nivel - media).powi(2)).sum::<f64>() / tramo.len() as f64).sqrt();
            let parejo = 1.0 - (desvio / VARIACION_TOLERADA_DB).clamp(0.0, 1.0);
            // Un tramo mucho mas bajo que el resto del tema no sirve de cama.
            let presente = ((media - nivel_tipico + 18.0) / 6.0).clamp(0.0, 1.0);
            let puntaje_medio = sonido[desde..hasta].iter().sum::<f32>() / (hasta - desde) as f32;
            let lejos_de_la_voz = f64::from((media_con_voz - puntaje_medio) / (media_con_voz - media_sin_voz).max(1e-3)).clamp(0.0, 1.0);
            let puntaje = zona.peso() * (largo / LARGO_IDEAL).min(1.0).sqrt() * parejo * presente * lejos_de_la_voz;
            if mejor.is_none_or(|(maximo, ..)| puntaje > maximo) {
                mejor = Some((puntaje, desde, hasta, zona));
            }
        }
        desde = hasta;
    }
    let (puntaje, desde, hasta, zona) = mejor?;
    let confianza = (puntaje * (separacion / 0.5).min(1.0)).clamp(0.0, 1.0);
    if confianza < CONFIANZA_MINIMA {
        return None;
    }
    let mut inicio = analisis.inicio + segundos(desde) + if zona == Zona::Intro { MARGEN_AL_EMPEZAR } else { 0.0 };
    let fin = analisis.inicio + segundos(hasta) - if hasta < ultimo_de_musica { MARGEN_ANTES_DE_LA_VOZ } else { 0.0 };
    let ritmo = if inicio - analisis.inicio < 30.0 { analisis.ritmo_inicial } else { analisis.ritmo_final };
    if let Some(golpe) = golpe_desde(ritmo, inicio).filter(|golpe| fin - golpe >= LARGO_MINIMO) {
        inicio = golpe;
    }
    let redondear = |segundos: f64| (segundos * 100.0).round() / 100.0;
    let ventana = VentanaDeOverlay::nueva(redondear(inicio), redondear(fin)).ok()?;
    Some(SugerenciaDeOverlay { ventana, confianza, zona })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Puntajes por tramo: `partes` es (segundos, con voz).
    fn puntajes(partes: &[(f64, bool)]) -> Vec<f32> {
        let mut resultado = Vec::new();
        for (numero, &(segundos, con_voz)) in partes.iter().enumerate() {
            for tramo in 0..(segundos * TRAMOS_POR_SEGUNDO) as usize {
                // Algo de variacion para que no sea un escalon perfecto.
                let ruido = ((tramo * 7 + numero * 3) % 10) as f32 * 0.01;
                resultado.push(if con_voz { 0.8 + ruido } else { 0.2 + ruido });
            }
        }
        resultado
    }

    fn analisis<'a>(voz: &'a [f32], niveles: &'a [f32], fin_de_la_musica: f64) -> AnalisisParaOverlay<'a> {
        AnalisisParaOverlay {
            voz,
            niveles,
            inicio: 0.0,
            fin: voz.len() as f64 / TRAMOS_POR_SEGUNDO,
            fin_de_la_musica,
            ritmo_inicial: None,
            ritmo_final: None,
        }
    }

    #[test]
    fn prefiere_la_intro_instrumental() {
        let voz = puntajes(&[(8.0, false), (30.0, true), (8.0, false), (30.0, true), (10.0, false)]);
        let niveles = vec![0.3; voz.len()];
        let sugerencia = sugerir_overlay(&analisis(&voz, &niveles, 86.0)).expect("hay sugerencia");
        assert_eq!(sugerencia.zona, Zona::Intro);
        assert_eq!((sugerencia.ventana.inicio(), sugerencia.ventana.fin()), (0.2, 7.7));
        assert!(sugerencia.confianza > 0.5, "{sugerencia:?}");
    }

    #[test]
    fn sin_intro_usa_el_final_antes_del_fundido() {
        let voz = puntajes(&[(1.0, false), (40.0, true), (12.0, false)]);
        let niveles = vec![0.3; voz.len()];
        // El fundido empieza a los 49 s.
        let sugerencia = sugerir_overlay(&analisis(&voz, &niveles, 49.0)).expect("hay sugerencia");
        assert_eq!(sugerencia.zona, Zona::Outro);
        assert_eq!((sugerencia.ventana.inicio(), sugerencia.ventana.fin()), (41.0, 49.0));
    }

    #[test]
    fn rellena_huecos_cortos_entre_frases() {
        let voz = puntajes(&[(6.0, false), (10.0, true), (0.8, false), (10.0, true)]);
        let niveles = vec![0.3; voz.len()];
        let sugerencia = sugerir_overlay(&analisis(&voz, &niveles, 26.8)).unwrap();
        assert_eq!(sugerencia.zona, Zona::Intro, "el hueco de 0,8 s no es un puente");
    }

    #[test]
    fn empieza_en_un_golpe() {
        let voz = puntajes(&[(10.0, false), (30.0, true)]);
        let niveles = vec![0.3; voz.len()];
        let ritmo = Some(Ritmo { periodo: 0.5, fase: 0.35, confianza: 0.8 });
        let sugerencia = sugerir_overlay(&AnalisisParaOverlay { ritmo_inicial: ritmo, ..analisis(&voz, &niveles, 40.0) }).unwrap();
        assert_eq!(sugerencia.ventana.inicio(), 0.35);
    }

    #[test]
    fn no_sugiere_sin_contraste_ni_lugar() {
        let parejo = vec![0.5; 600];
        let niveles = vec![0.3; 600];
        assert!(sugerir_overlay(&analisis(&parejo, &niveles, 60.0)).is_none(), "sin diferencia entre partes");
        let todo_voz = puntajes(&[(1.0, false), (60.0, true), (2.0, false)]);
        let niveles = vec![0.3; todo_voz.len()];
        assert!(sugerir_overlay(&analisis(&todo_voz, &niveles, 63.0)).is_none(), "ningun tramo de 4 s sin voz");
    }

    #[test]
    fn descarta_un_tramo_casi_mudo() {
        let voz = puntajes(&[(8.0, false), (20.0, true), (8.0, false), (20.0, true)]);
        // La intro esta 30 dB por debajo del resto.
        let niveles: Vec<f32> = (0..voz.len()).map(|tramo| if tramo < 80 { 0.01 } else { 0.3 }).collect();
        let sugerencia = sugerir_overlay(&analisis(&voz, &niveles, 56.0)).unwrap();
        assert_eq!(sugerencia.zona, Zona::Puente);
    }
}
