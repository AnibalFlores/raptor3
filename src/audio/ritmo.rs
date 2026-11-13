//! Ritmo de un tramo de audio: tempo y fase de los golpes, estimados por
//! autocorrelacion de una funcion de ataques (onsets) muestreada cien veces
//! por segundo, la misma resolucion que la forma de onda.
//!
//! La funcion de ataques suma los aumentos de energia (comprimida en escala
//! logaritmica) de tres bandas: la grave, donde suelen estar bombo y bajo; la
//! media, con guitarras, piano y voz; y la aguda, con platillos y percusion.
//! Asi un tema sin bombo tambien marca el pulso.

use std::f64::consts::LN_2;

// Rango de tempos que se buscan.
const BPM_MINIMO: f64 = 60.0;
const BPM_MAXIMO: f64 = 200.0;
// Tempo mas probable a priori; corrige confusiones de doble o medio tempo.
// Medido contra BPM conocidos: 100 acierta mas que 120.
const BPM_CENTRAL: f64 = 100.0;
// Ancho (en octavas) de la preferencia por el tempo central.
const OCTAVAS_DE_PREFERENCIA: f64 = 1.0;
// Menos que esto de audio no alcanza para estimar el ritmo.
const SEGUNDOS_MINIMOS: f64 = 8.0;
// Energia minima para el logaritmo: el silencio no genera ataques.
const ENERGIA_MINIMA: f32 = 1e-6;
// Tolerancia entre dos tempos para considerarlos el mismo.
const TOLERANCIA_DE_TEMPO: f64 = 0.04;
// Ventanas en que se divide un tema para votar su tempo, y cada cuanto
// empieza una; se solapan a la mitad.
const SEGUNDOS_POR_VENTANA: f64 = 20.0;
const SEGUNDOS_ENTRE_VENTANAS: f64 = 10.0;
// Con menos ventanas no hay votacion: vale una sola estimacion.
const VENTANAS_MINIMAS: usize = 3;
// Una ventana que da el doble o la mitad apoya el tempo, pero menos.
const PESO_DEL_DOBLE: f64 = 0.5;
// Compresion de la energia (normalizada al maximo del tema) antes del
// logaritmo: un pasaje suave marca sus ataques casi como uno fuerte, y el
// ruido de fondo de un silencio no genera ataques.
const COMPRESION: f32 = 1000.0;
// Radio (en centesimas) de la media movil que se resta a los ataques para
// dejar solo los picos.
const RADIO_DE_LA_MEDIA: usize = 25;
// Peso de la correlacion al doble y al cuadruple del periodo: el periodo
// verdadero se repite tambien ahi, una subdivision no tanto.
const PESO_DEL_ARMONICO: f64 = 0.5;

/// Tempo y fase de un tramo: hay un golpe en `fase` (segundos desde el
/// principio del archivo) y luego cada `periodo` segundos.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ritmo {
    pub periodo: f64,
    pub fase: f64,
    /// Altura del pico de autocorrelacion (0 a 1): cuanto se repite el patron.
    pub confianza: f64,
}

impl Ritmo {
    pub fn bpm(&self) -> f64 {
        60.0 / self.periodo
    }
}

/// Tempo de un tema entero, votado entre las ventanas en que se divide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tempo {
    pub bpm: f64,
    /// Parte del peso de las ventanas (0 a 1) que coincide con este tempo; la
    /// que da el doble o la mitad cuenta a medias.
    pub acuerdo: f64,
    /// Ventanas en que se estimo un ritmo.
    pub ventanas: usize,
}

/// Peso con que un tempo apoya a otro: 1 si es el mismo, `PESO_DEL_DOBLE`
/// si es el doble o la mitad y 0 si no tienen que ver.
fn apoyo(bpm: f64, otro: f64) -> f64 {
    let parecido = |razon: f64| (razon - 1.0).abs() <= TOLERANCIA_DE_TEMPO;
    let razon = bpm / otro;
    if parecido(razon) {
        1.0
    } else if parecido(razon / 2.0) || parecido(razon * 2.0) {
        PESO_DEL_DOBLE
    } else {
        0.0
    }
}

/// Si dos tempos son el mismo, o uno el doble del otro.
pub fn tempos_coinciden(bpm: f64, otro: f64) -> bool {
    apoyo(bpm, otro) > 0.0
}

/// Estima el tempo de los ataques entre `desde` y `hasta` votando entre
/// ventanas solapadas: cada una propone su tempo con el peso de su
/// confianza, y gana el que mas apoyo junta. Asi una intro, un corte o un
/// puente sin ritmo no arruinan la estimacion del tema. El BPM es el
/// promedio, pesado por confianza, de las ventanas que dan ese tempo.
pub fn votar_tempo(ataques: &[f32], por_segundo: f64, desde: usize, hasta: usize) -> Option<Tempo> {
    let hasta = hasta.min(ataques.len());
    let ventana = (SEGUNDOS_POR_VENTANA * por_segundo) as usize;
    let paso = (SEGUNDOS_ENTRE_VENTANAS * por_segundo) as usize;
    let mut estimaciones: Vec<(f64, f64)> = Vec::new();
    let mut inicio = desde;
    while inicio + ventana <= hasta {
        if let Some(ritmo) = estimar(ataques, por_segundo, inicio, inicio + ventana) {
            estimaciones.push((ritmo.bpm(), ritmo.confianza.max(f64::EPSILON)));
        }
        inicio += paso;
    }
    if estimaciones.len() < VENTANAS_MINIMAS {
        let ritmo = estimar(ataques, por_segundo, desde, hasta)?;
        return Some(Tempo { bpm: ritmo.bpm(), acuerdo: ritmo.confianza, ventanas: 1 });
    }
    let total: f64 = estimaciones.iter().map(|(_, peso)| peso).sum();
    let puntaje = |bpm: f64| estimaciones.iter().map(|&(otro, peso)| peso * apoyo(bpm, otro)).sum::<f64>();
    let (ganador, apoyo_total) = estimaciones
        .iter()
        .map(|&(bpm, _)| (bpm, puntaje(bpm)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let (suma, pesos) = estimaciones
        .iter()
        .filter(|&&(bpm, _)| apoyo(ganador, bpm) == 1.0)
        .fold((0.0, 0.0), |(suma, pesos), &(bpm, peso)| (suma + bpm * peso, pesos + peso));
    Some(Tempo { bpm: suma / pesos, acuerdo: (apoyo_total / total).min(1.0), ventanas: estimaciones.len() })
}

/// Funcion de ataques a partir de la energia por centesima de segundo de
/// cada banda: suma de los aumentos de energia comprimida, menos su media
/// movil, sin valores negativos.
pub fn ataques(bandas: &[&[f32]]) -> Vec<f32> {
    let largo = bandas.iter().map(|banda| banda.len()).min().unwrap_or(0);
    let mut suma = vec![0.0f32; largo];
    for banda in bandas {
        let maximo = banda.iter().copied().fold(ENERGIA_MINIMA, f32::max);
        let comprimida = |indice: usize| (COMPRESION * banda[indice] / maximo).ln_1p();
        for indice in 1..largo {
            suma[indice] += (comprimida(indice) - comprimida(indice - 1)).max(0.0);
        }
    }
    restar_media_movil(&suma)
}

/// Resta a cada valor el promedio de su entorno y descarta lo negativo: un
/// tramo con muchos ataques seguidos no tapa los golpes que sobresalen.
fn restar_media_movil(valores: &[f32]) -> Vec<f32> {
    let mut acumulado = vec![0.0f64; valores.len() + 1];
    for (indice, &valor) in valores.iter().enumerate() {
        acumulado[indice + 1] = acumulado[indice] + f64::from(valor);
    }
    (0..valores.len())
        .map(|indice| {
            let desde = indice.saturating_sub(RADIO_DE_LA_MEDIA);
            let hasta = (indice + RADIO_DE_LA_MEDIA + 1).min(valores.len());
            let media = (acumulado[hasta] - acumulado[desde]) / (hasta - desde) as f64;
            (f64::from(valores[indice]) - media).max(0.0) as f32
        })
        .collect()
}

/// Estima el ritmo de los ataques entre los indices `desde` y `hasta`.
/// Devuelve None si el tramo es corto o no tiene ataques.
pub fn estimar(ataques: &[f32], por_segundo: f64, desde: usize, hasta: usize) -> Option<Ritmo> {
    let hasta = hasta.min(ataques.len());
    let tramo = ataques.get(desde..hasta)?;
    if (tramo.len() as f64) < SEGUNDOS_MINIMOS * por_segundo {
        return None;
    }
    let media = tramo.iter().map(|&valor| f64::from(valor)).sum::<f64>() / tramo.len() as f64;
    let centrado: Vec<f64> = tramo.iter().map(|&valor| f64::from(valor) - media).collect();
    let energia: f64 = centrado.iter().map(|valor| valor * valor).sum();
    if energia <= f64::EPSILON {
        return None;
    }
    // Autocorrelacion normalizada y sin sesgo por la cantidad de terminos.
    let largo = centrado.len();
    let correlacion = |retardo: usize| -> f64 {
        if retardo >= largo {
            return 0.0;
        }
        let suma: f64 = centrado[..largo - retardo].iter().zip(&centrado[retardo..]).map(|(a, b)| a * b).sum();
        suma / energia * largo as f64 / (largo - retardo) as f64
    };
    let retardo_minimo = ((60.0 / BPM_MAXIMO) * por_segundo).floor().max(2.0) as usize;
    let retardo_maximo = (((60.0 / BPM_MINIMO) * por_segundo).ceil() as usize).min(largo / 2);
    if retardo_minimo + 2 > retardo_maximo {
        return None;
    }
    let valores: Vec<f64> = (0..=4 * retardo_maximo + 2).map(correlacion).collect();
    let valor = |retardo: usize| valores[retardo];
    // El multiplo de un retardo entero se aleja del periodo fraccionario
    // verdadero, asi que en cada armonico se toma el maximo de su entorno.
    let cerca_de = |retardo: usize, radio: usize| (retardo - radio..=retardo + radio).map(valor).fold(f64::MIN, f64::max);
    let puntaje = |retardo: usize| {
        valor(retardo) + PESO_DEL_ARMONICO * (cerca_de(2 * retardo, 1) + PESO_DEL_ARMONICO * cerca_de(4 * retardo, 2))
    };
    let preferencia = |retardo: f64| {
        let octavas = (60.0 * por_segundo / retardo / BPM_CENTRAL).ln() / LN_2 / OCTAVAS_DE_PREFERENCIA;
        (-0.5 * octavas * octavas).exp()
    };
    let mejor = (retardo_minimo..=retardo_maximo)
        .filter(|&retardo| valor(retardo) >= valor(retardo - 1) && valor(retardo) >= valor(retardo + 1))
        .max_by(|&a, &b| (puntaje(a) * preferencia(a as f64)).total_cmp(&(puntaje(b) * preferencia(b as f64))))?;
    // Interpolacion parabolica del pico para un periodo fraccionario.
    let (anterior, pico, siguiente) = (valor(mejor - 1), valor(mejor), valor(mejor + 1));
    let curvatura = anterior - 2.0 * pico + siguiente;
    let ajuste = if curvatura.abs() > f64::EPSILON { (0.5 * (anterior - siguiente) / curvatura).clamp(-0.5, 0.5) } else { 0.0 };
    let periodo = mejor as f64 + ajuste;

    // Fase: el desplazamiento cuya grilla de golpes junta mas ataques.
    let pasos = (periodo * 4.0).ceil() as usize;
    let suma_en = |desplazamiento: f64| -> f64 {
        let mut suma = 0.0;
        let mut posicion = desplazamiento;
        while (posicion.round() as usize) < tramo.len() {
            suma += f64::from(tramo[posicion.round() as usize]);
            posicion += periodo;
        }
        suma
    };
    let desplazamiento = (0..pasos)
        .map(|paso| paso as f64 / 4.0)
        .max_by(|&a, &b| suma_en(a).total_cmp(&suma_en(b)))
        .unwrap_or(0.0);
    let (desplazamiento, periodo) = ajustar_grilla(tramo, desplazamiento, periodo);
    Some(Ritmo {
        periodo: periodo / por_segundo,
        fase: (desde as f64 + desplazamiento) / por_segundo,
        confianza: pico.clamp(0.0, 1.0),
    })
}

/// Afina la grilla de golpes: busca el ataque mas fuerte cerca de cada golpe
/// previsto y ajusta una recta (cuadrados minimos) a esas posiciones. Asi un
/// error minimo de periodo no se acumula a lo largo del tramo.
fn ajustar_grilla(tramo: &[f32], desplazamiento: f64, periodo: f64) -> (f64, f64) {
    let radio = periodo / 4.0;
    let mut golpes: Vec<(f64, f64)> = Vec::new();
    let mut previsto = desplazamiento;
    let mut numero = 0.0;
    while previsto < tramo.len() as f64 {
        let desde = (previsto - radio).max(0.0).round() as usize;
        let hasta = ((previsto + radio).round() as usize).min(tramo.len() - 1);
        if let Some(mayor) = (desde..=hasta).max_by(|&a, &b| tramo[a].total_cmp(&tramo[b])) {
            golpes.push((numero, mayor as f64));
        }
        previsto += periodo;
        numero += 1.0;
    }
    if golpes.len() < 4 {
        return (desplazamiento, periodo);
    }
    let cantidad = golpes.len() as f64;
    let media_numero = golpes.iter().map(|(numero, _)| numero).sum::<f64>() / cantidad;
    let media_posicion = golpes.iter().map(|(_, posicion)| posicion).sum::<f64>() / cantidad;
    let covarianza: f64 = golpes.iter().map(|(numero, posicion)| (numero - media_numero) * (posicion - media_posicion)).sum();
    let varianza: f64 = golpes.iter().map(|(numero, _)| (numero - media_numero).powi(2)).sum();
    let ajustado = covarianza / varianza;
    // Si el ajuste se aleja mucho del periodo estimado, se conserva el original.
    if (ajustado / periodo - 1.0).abs() > 0.05 {
        return (desplazamiento, periodo);
    }
    let origen = (media_posicion - ajustado * media_numero).rem_euclid(ajustado);
    (origen, ajustado)
}

/// Periodo comun de dos ritmos si sus tempos coinciden (tambien al doble o a
/// la mitad) dentro de la tolerancia; es el menor de los dos, asi cada golpe
/// de uno cae sobre un golpe del otro.
pub fn periodo_comun(a: &Ritmo, b: &Ritmo) -> Option<f64> {
    let razon = a.periodo / b.periodo;
    [1.0, 2.0, 0.5]
        .iter()
        .any(|multiplo| (razon / multiplo - 1.0).abs() <= TOLERANCIA_DE_TEMPO)
        .then(|| a.periodo.min(b.periodo))
}

/// Cuanto correr el arranque de la entrante para que sus golpes caigan sobre
/// los de la saliente. La entrante arranca desde `inicio` (en su propio
/// tiempo) cuando la saliente esta en `momento`. El resultado va entre
/// -periodo/2 y +periodo/2; positivo es arrancar mas tarde.
pub fn desfase_para_alinear(saliente: &Ritmo, entrante: &Ritmo, momento: f64, inicio: f64) -> Option<f64> {
    let periodo = periodo_comun(saliente, entrante)?;
    // Cada grilla se lleva con su propio periodo hasta el punto de mezcla:
    // la fase se midio lejos de ahi y una diferencia minima de tempo se
    // acumularia golpe a golpe.
    let golpe_de_la_saliente = saliente.fase + ((momento - saliente.fase) / saliente.periodo).round() * saliente.periodo;
    let primer_golpe = entrante.fase + ((inicio - entrante.fase) / entrante.periodo).ceil() * entrante.periodo;
    let golpe_de_la_entrante = momento + (primer_golpe - inicio);
    let desfase = (golpe_de_la_saliente - golpe_de_la_entrante).rem_euclid(periodo);
    Some(if desfase > periodo / 2.0 { desfase - periodo } else { desfase })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const POR_SEGUNDO: f64 = 100.0;

    /// Ataques de un patron de golpes regulares, con un golpe mas debil a
    /// mitad de cada tiempo y algo de ruido pseudoaleatorio.
    fn golpes(bpm: f64, primer_golpe: f64, segundos: f64) -> Vec<f32> {
        let largo = (segundos * POR_SEGUNDO) as usize;
        let periodo = 60.0 / bpm * POR_SEGUNDO;
        let mut ataques = vec![0.0f32; largo];
        let mut semilla = 12345u32;
        for valor in &mut ataques {
            semilla = semilla.wrapping_mul(1_103_515_245).wrapping_add(12345);
            *valor = (semilla >> 16) as f32 / 65536.0 * 0.2;
        }
        let mut golpe = primer_golpe * POR_SEGUNDO;
        while (golpe.round() as usize) < largo {
            ataques[golpe.round() as usize] += 1.0;
            let medio = (golpe + periodo / 2.0).round() as usize;
            if medio < largo {
                ataques[medio] += 0.3;
            }
            golpe += periodo;
        }
        ataques
    }

    #[test]
    fn estima_tempo_y_fase_de_golpes_regulares() {
        for bpm in [90.0, 120.0, 128.0, 140.0] {
            let ataques = golpes(bpm, 0.37, 20.0);
            let ritmo = estimar(&ataques, POR_SEGUNDO, 0, ataques.len()).expect("hay ritmo");
            assert!((ritmo.bpm() - bpm).abs() < bpm * 0.015, "esperado {bpm}, estimado {}", ritmo.bpm());
            assert!(ritmo.confianza > 0.4, "confianza {}", ritmo.confianza);
            // La fase cae sobre un golpe (modulo el periodo).
            let distancia = (ritmo.fase - 0.37).rem_euclid(ritmo.periodo);
            let distancia = distancia.min(ritmo.periodo - distancia);
            assert!(distancia < 0.02, "fase {} periodo {}", ritmo.fase, ritmo.periodo);
        }
    }

    #[test]
    fn el_ruido_no_tiene_ritmo_confiable() {
        let mut semilla = 777u32;
        let ruido: Vec<f32> = (0..2000)
            .map(|_| {
                semilla = semilla.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (semilla >> 16) as f32 / 65536.0
            })
            .collect();
        let confianza = estimar(&ruido, POR_SEGUNDO, 0, ruido.len()).map_or(0.0, |ritmo| ritmo.confianza);
        assert!(confianza < 0.2, "confianza {confianza}");
    }

    #[test]
    fn un_tramo_corto_o_en_silencio_no_tiene_ritmo() {
        assert!(estimar(&golpes(120.0, 0.0, 5.0), POR_SEGUNDO, 0, 500).is_none());
        assert!(estimar(&[0.0; 2000], POR_SEGUNDO, 0, 2000).is_none());
    }

    #[test]
    fn la_votacion_ignora_los_tramos_sin_ritmo() {
        // 30 s de ruido (una intro sin golpes) y 90 s a 124 BPM.
        let mut semilla = 99u32;
        let mut ataques: Vec<f32> = (0..3000)
            .map(|_| {
                semilla = semilla.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (semilla >> 16) as f32 / 65536.0
            })
            .collect();
        ataques.extend(golpes(124.0, 0.2, 90.0));
        let tempo = votar_tempo(&ataques, POR_SEGUNDO, 0, ataques.len()).expect("hay tempo");
        assert!((tempo.bpm - 124.0).abs() < 1.5, "{tempo:?}");
        assert!(tempo.acuerdo > 0.6 && tempo.ventanas >= 10, "{tempo:?}");
    }

    #[test]
    fn un_tema_corto_vota_con_una_sola_estimacion() {
        let ataques = golpes(100.0, 0.0, 15.0);
        let tempo = votar_tempo(&ataques, POR_SEGUNDO, 0, ataques.len()).expect("hay tempo");
        assert_eq!(tempo.ventanas, 1);
        assert!((tempo.bpm - 100.0).abs() < 1.5, "{tempo:?}");
    }

    #[test]
    fn coinciden_tempos_iguales_y_al_doble() {
        assert!(tempos_coinciden(120.0, 123.0) && tempos_coinciden(83.0, 167.0) && tempos_coinciden(160.0, 79.0));
        assert!(!tempos_coinciden(120.0, 100.0) && !tempos_coinciden(120.0, 180.0));
    }

    #[test]
    fn tempos_compatibles_tambien_al_doble() {
        let ritmo = |bpm: f64| Ritmo { periodo: 60.0 / bpm, fase: 0.0, confianza: 1.0 };
        assert!(periodo_comun(&ritmo(120.0), &ritmo(123.0)).is_some());
        assert!((periodo_comun(&ritmo(120.0), &ritmo(240.0)).unwrap() - 0.25).abs() < 1e-9);
        assert!(periodo_comun(&ritmo(120.0), &ritmo(100.0)).is_none());
    }

    #[test]
    fn el_desfase_alinea_los_golpes() {
        // Saliente con golpes en 0.1 + k*0.5; la entrante arranca desde 0
        // cuando la saliente esta en 10.0 y tiene su primer golpe en 0.3.
        let saliente = Ritmo { periodo: 0.5, fase: 0.1, confianza: 1.0 };
        let entrante = Ritmo { periodo: 0.5, fase: 0.3, confianza: 1.0 };
        let desfase = desfase_para_alinear(&saliente, &entrante, 10.0, 0.0).unwrap();
        assert!(desfase.abs() <= 0.25);
        // Con el desfase, el golpe de la entrante cae en la grilla de la saliente.
        let golpe = 10.0 + desfase + 0.3;
        let resto = (golpe - 0.1).rem_euclid(0.5);
        assert!(resto.min(0.5 - resto) < 1e-9, "golpe en {golpe}");
    }

    #[test]
    fn la_alineacion_no_acumula_la_diferencia_de_tempo() {
        // Como en un tema real: la fase de la saliente se midio 22 s antes
        // de la mezcla y los tempos difieren un 1 %. La saliente tiene golpes
        // en 1.0 + k*0.5; la entrante (121 BPM) arranca desde 2.0 cuando la
        // saliente esta en 22.9, con su primer golpe en 2.23.
        let saliente = Ritmo { periodo: 0.5, fase: 1.0, confianza: 1.0 };
        let entrante = Ritmo { periodo: 60.0 / 121.0, fase: 2.23, confianza: 1.0 };
        let desfase = desfase_para_alinear(&saliente, &entrante, 22.9, 2.0).unwrap();
        // El golpe de la entrante cae en 22.9 + desfase + 0.23, que debe ser 23.0.
        assert!((22.9 + desfase + 0.23 - 23.0).abs() < 1e-9, "desfase {desfase}");
    }
}
