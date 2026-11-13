//! Deteccion aproximada de voz cantada, para sugerir la ventana de overlay
//! en un tramo instrumental. No separa la voz: combina tres indicios por
//! decima de segundo, en la banda de la voz (300 Hz a 3,4 kHz):
//!
//! - Centro: la voz suele estar en el centro del estereo, asi que en esa
//!   banda domina L+R sobre L-R.
//! - Armonia: la voz tiene un tono claro; se mide con la autocorrelacion
//!   normalizada (NSDF) de L+R entre 80 Hz y 1 kHz. Varios instrumentos a la
//!   vez la bajan.
//! - Modulacion: la energia de la voz sube y baja al ritmo de las silabas
//!   (unas 3 a 8 veces por segundo).
//!
//! Cada indicio se normaliza dentro del tema (mediana y desvio absoluto), asi
//! que el puntaje dice cuanto mas "a voz" suena un tramo que el resto del
//! mismo tema. Acierta en la mayoria de los temas con voz; con voces muy
//! procesadas, coros o rap sobre poca musica puede fallar.

/// Tramos por segundo del puntaje de voz.
pub const TRAMOS_POR_SEGUNDO: u32 = 10;
// Envolvente de la banda de la voz, para medir la modulacion silabica.
const ENVOLVENTE_POR_SEGUNDO: u32 = 100;
// Frecuencia aproximada a la que se reduce la banda para buscar el tono.
const FRECUENCIA_DE_TONO: u32 = 8_000;
const TONO_MAXIMO: f64 = 1_000.0;
const TONO_MINIMO: f64 = 80.0;
// Muestras (a la frecuencia reducida) en que se busca el tono de cada tramo.
const VENTANA_DE_TONO: usize = 512;
// Pesos de cada indicio en el puntaje.
const PESO_DE_ARMONIA: f32 = 0.5;
const PESO_DE_CENTRO: f32 = 0.3;
const PESO_DE_MODULACION: f32 = 0.2;
// Suavizado del puntaje, en tramos (un segundo).
const SUAVIZADO: usize = 10;

/// Filtro biquad (formas del "Audio EQ Cookbook" de R. Bristow-Johnson).
#[derive(Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn nuevo(frecuencia: f64, corte: f64, pasa_altos: bool) -> Self {
        let w = std::f64::consts::TAU * corte.min(frecuencia * 0.45) / frecuencia;
        let alfa = w.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let coseno = w.cos();
        let a0 = 1.0 + alfa;
        let b = if pasa_altos {
            [(1.0 + coseno) / 2.0, -(1.0 + coseno), (1.0 + coseno) / 2.0]
        } else {
            [(1.0 - coseno) / 2.0, 1.0 - coseno, (1.0 - coseno) / 2.0]
        };
        Self {
            b: b.map(|valor| (valor / a0) as f32),
            a: [(-2.0 * coseno / a0) as f32, ((1.0 - alfa) / a0) as f32],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn filtrar(&mut self, entrada: f32) -> f32 {
        let salida = self.b[0] * entrada + self.b[1] * self.x[0] + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [entrada, self.x[0]];
        self.y = [salida, self.y[0]];
        salida
    }
}

/// Paso de banda de la voz: pasa altos de 300 Hz y pasa bajos de 3,4 kHz.
#[derive(Clone, Copy)]
struct BandaDeVoz([Biquad; 2]);

impl BandaDeVoz {
    fn nueva(frecuencia: f64) -> Self {
        Self([Biquad::nuevo(frecuencia, 300.0, true), Biquad::nuevo(frecuencia, 3_400.0, false)])
    }

    fn filtrar(&mut self, entrada: f32) -> f32 {
        let paso = self.0[0].filtrar(entrada);
        self.0[1].filtrar(paso)
    }
}

/// Indicios de un tramo.
#[derive(Clone, Copy, Debug, Default)]
struct Rasgos {
    centro: f32,
    armonia: f32,
    energia: f32,
}

/// Recorre el audio (estereo, a una sola frecuencia) y junta los indicios de
/// cada tramo.
pub struct DetectorDeVoz {
    frecuencia: u32,
    centro: BandaDeVoz,
    costado: BandaDeVoz,
    reduccion: usize,
    frames_por_tramo: f64,
    frames_por_envolvente: f64,
    // Tramo en curso.
    frames_en_tramo: f64,
    energia_centro: f64,
    energia_costado: f64,
    reducidas: Vec<f32>,
    contador: usize,
    // Envolvente en curso.
    frames_en_envolvente: f64,
    energia_de_envolvente: f64,
    cantidad_de_envolvente: u32,
    rasgos: Vec<Rasgos>,
    envolvente: Vec<f32>,
}

impl DetectorDeVoz {
    pub fn nuevo(frecuencia: u32) -> Self {
        let real = f64::from(frecuencia);
        Self {
            frecuencia,
            centro: BandaDeVoz::nueva(real),
            costado: BandaDeVoz::nueva(real),
            reduccion: (frecuencia / FRECUENCIA_DE_TONO).max(1) as usize,
            frames_por_tramo: real / f64::from(TRAMOS_POR_SEGUNDO),
            frames_por_envolvente: real / f64::from(ENVOLVENTE_POR_SEGUNDO),
            frames_en_tramo: 0.0,
            energia_centro: 0.0,
            energia_costado: 0.0,
            reducidas: Vec::new(),
            contador: 0,
            frames_en_envolvente: 0.0,
            energia_de_envolvente: 0.0,
            cantidad_de_envolvente: 0,
            rasgos: Vec::new(),
            envolvente: Vec::new(),
        }
    }

    pub fn frecuencia(&self) -> u32 {
        self.frecuencia
    }

    /// Suma un bloque estereo intercalado.
    pub fn sumar(&mut self, bloque: &[f32]) {
        for frame in bloque.chunks_exact(2) {
            let centro = self.centro.filtrar((frame[0] + frame[1]) * 0.5);
            let costado = self.costado.filtrar((frame[0] - frame[1]) * 0.5);
            self.energia_centro += f64::from(centro * centro);
            self.energia_costado += f64::from(costado * costado);
            self.energia_de_envolvente += f64::from(centro * centro);
            self.cantidad_de_envolvente += 1;
            self.contador += 1;
            if self.contador >= self.reduccion {
                self.contador = 0;
                self.reducidas.push(centro);
            }
            self.frames_en_envolvente += 1.0;
            if self.frames_en_envolvente >= self.frames_por_envolvente {
                self.frames_en_envolvente -= self.frames_por_envolvente;
                self.cerrar_envolvente();
            }
            self.frames_en_tramo += 1.0;
            if self.frames_en_tramo >= self.frames_por_tramo {
                self.frames_en_tramo -= self.frames_por_tramo;
                self.cerrar_tramo();
            }
        }
    }

    fn cerrar_envolvente(&mut self) {
        let media = self.energia_de_envolvente / f64::from(self.cantidad_de_envolvente.max(1));
        self.envolvente.push(media as f32);
        self.energia_de_envolvente = 0.0;
        self.cantidad_de_envolvente = 0;
    }

    fn cerrar_tramo(&mut self) {
        let total = self.energia_centro + self.energia_costado;
        let frecuencia_reducida = f64::from(self.frecuencia) / self.reduccion as f64;
        self.rasgos.push(Rasgos {
            centro: if total > 0.0 { (self.energia_centro / total) as f32 } else { 0.0 },
            armonia: armonia(&self.reducidas, frecuencia_reducida),
            energia: (self.energia_centro / self.frames_por_tramo) as f32,
        });
        self.energia_centro = 0.0;
        self.energia_costado = 0.0;
        self.reducidas.clear();
    }

    /// Puntaje de voz de cada tramo, de 0 a 1 (0,5 es lo tipico del tema).
    pub fn puntajes(mut self) -> Vec<f32> {
        if self.frames_en_tramo > self.frames_por_tramo * 0.5 {
            self.cerrar_tramo();
        }
        let modulacion = modulacion_por_tramo(&self.envolvente, self.rasgos.len());
        puntuar(&self.rasgos, &modulacion)
    }
}

/// Que tan periodica es la senal (0 a 1): el maximo de la NSDF entre los
/// periodos de 80 Hz y 1 kHz, en una ventana del medio del tramo.
fn armonia(muestras: &[f32], frecuencia: f64) -> f32 {
    let largo = muestras.len().min(VENTANA_DE_TONO);
    let desde = (muestras.len() - largo) / 2;
    let ventana = &muestras[desde..desde + largo];
    let retardo_minimo = (frecuencia / TONO_MAXIMO).floor().max(2.0) as usize;
    let retardo_maximo = ((frecuencia / TONO_MINIMO).ceil() as usize).min(largo / 2);
    let energia: f32 = ventana.iter().map(|muestra| muestra * muestra).sum();
    if retardo_maximo <= retardo_minimo || energia <= 1e-9 {
        return 0.0;
    }
    (retardo_minimo..=retardo_maximo)
        .map(|retardo| {
            let (mut producto, mut cuadrados) = (0.0f32, 0.0f32);
            for indice in 0..largo - retardo {
                let (a, b) = (ventana[indice], ventana[indice + retardo]);
                producto += a * b;
                cuadrados += a * a + b * b;
            }
            if cuadrados > 0.0 { 2.0 * producto / cuadrados } else { 0.0 }
        })
        .fold(0.0, f32::max)
}

/// Fuerza de la modulacion silabica (3 a 8 Hz aprox.) de la envolvente, por
/// tramo: el logaritmo de la envolvente pasado por la diferencia de dos
/// promedios moviles, y su valor eficaz en un segundo.
fn modulacion_por_tramo(envolvente: &[f32], tramos: usize) -> Vec<f32> {
    let logaritmo: Vec<f32> = envolvente.iter().map(|energia| (energia + 1e-9).log10()).collect();
    let promedio = |ventana: usize| -> Vec<f32> {
        let mut acumulado = vec![0.0f32; logaritmo.len() + 1];
        for (indice, valor) in logaritmo.iter().enumerate() {
            acumulado[indice + 1] = acumulado[indice] + valor;
        }
        (0..logaritmo.len())
            .map(|indice| {
                let (desde, hasta) = (indice.saturating_sub(ventana / 2), (indice + ventana / 2 + 1).min(logaritmo.len()));
                (acumulado[hasta] - acumulado[desde]) / (hasta - desde) as f32
            })
            .collect()
    };
    let (rapido, lento) = (promedio(5), promedio(25));
    let banda: Vec<f32> = rapido.iter().zip(&lento).map(|(a, b)| (a - b) * (a - b)).collect();
    let por_tramo = (ENVOLVENTE_POR_SEGUNDO / TRAMOS_POR_SEGUNDO) as usize;
    let segundo = ENVOLVENTE_POR_SEGUNDO as usize;
    (0..tramos)
        .map(|tramo| {
            let centro = tramo * por_tramo + por_tramo / 2;
            let (desde, hasta) = (centro.saturating_sub(segundo / 2).min(banda.len()), (centro + segundo / 2).min(banda.len()));
            if hasta > desde { (banda[desde..hasta].iter().sum::<f32>() / (hasta - desde) as f32).sqrt() } else { 0.0 }
        })
        .collect()
}

/// Mediana y desvio absoluto mediano (escalado como un desvio estandar).
fn mediana_y_desvio(valores: &[f32]) -> (f32, f32) {
    let mut ordenados = valores.to_vec();
    ordenados.sort_by(f32::total_cmp);
    let mediana = ordenados[ordenados.len() / 2];
    let mut desvios: Vec<f32> = ordenados.iter().map(|valor| (valor - mediana).abs()).collect();
    desvios.sort_by(f32::total_cmp);
    (mediana, (desvios[desvios.len() / 2] * 1.4826).max(1e-6))
}

/// Combina los indicios normalizados dentro del tema y los suaviza. Los
/// tramos casi mudos quedan en 0.
fn puntuar(rasgos: &[Rasgos], modulacion: &[f32]) -> Vec<f32> {
    let maximo = rasgos.iter().map(|rasgo| rasgo.energia).fold(0.0, f32::max);
    // Unos 40 dB por debajo del tramo mas fuerte de la banda.
    let con_sonido: Vec<usize> = (0..rasgos.len()).filter(|&indice| rasgos[indice].energia > maximo * 1e-4).collect();
    if con_sonido.len() < SUAVIZADO {
        return vec![0.0; rasgos.len()];
    }
    let normalizar = |valor: &dyn Fn(usize) -> f32| -> Vec<f32> {
        let valores: Vec<f32> = con_sonido.iter().map(|&indice| valor(indice)).collect();
        let (mediana, desvio) = mediana_y_desvio(&valores);
        (0..rasgos.len()).map(|indice| ((valor(indice) - mediana) / desvio).clamp(-4.0, 4.0)).collect()
    };
    let armonia = normalizar(&|indice| rasgos[indice].armonia);
    let centro = normalizar(&|indice| rasgos[indice].centro);
    let modulacion = normalizar(&|indice| modulacion.get(indice).copied().unwrap_or(0.0));
    let crudo: Vec<f32> = (0..rasgos.len())
        .map(|indice| PESO_DE_ARMONIA * armonia[indice] + PESO_DE_CENTRO * centro[indice] + PESO_DE_MODULACION * modulacion[indice])
        .collect();
    let mudo = |indice: usize| rasgos[indice].energia <= maximo * 1e-4;
    (0..rasgos.len())
        .map(|indice| {
            if mudo(indice) {
                return 0.0;
            }
            let (desde, hasta) = (indice.saturating_sub(SUAVIZADO / 2), (indice + SUAVIZADO / 2).min(rasgos.len()));
            let vecinos: Vec<f32> = (desde..hasta).filter(|&vecino| !mudo(vecino)).map(|vecino| crudo[vecino]).collect();
            let media = vecinos.iter().sum::<f32>() / vecinos.len().max(1) as f32;
            1.0 / (1.0 + (-1.5 * media).exp())
        })
        .collect()
}

#[cfg(test)]
pub mod pruebas {
    use super::*;

    /// Audio estereo de prueba: un acompanamiento abierto (acordes distintos
    /// en cada canal y golpes de ruido) y, donde `con_voz` lo indica, una
    /// "voz": tono centrado de 220 Hz con armonicos, vibrato y silabas.
    pub fn cancion(frecuencia: u32, tramos: &[(f64, bool)]) -> Vec<f32> {
        let real = frecuencia as f32;
        let mut semilla = 12_345u32;
        let mut ruido = move || {
            semilla = semilla.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (semilla >> 9) as f32 / (1u32 << 23) as f32 - 0.5
        };
        let total: f64 = tramos.iter().map(|(segundos, _)| segundos).sum();
        let mut muestras = Vec::with_capacity((total * f64::from(frecuencia)) as usize * 2);
        let mut n = 0usize;
        for &(segundos, con_voz) in tramos {
            for _ in 0..(segundos * f64::from(frecuencia)) as usize {
                let t = n as f32 / real;
                let tono = |frecuencia: f32| (t * frecuencia * std::f32::consts::TAU).sin();
                let izquierda = 0.12 * (tono(196.0) + tono(246.9) + 0.5 * tono(392.0));
                let derecha = 0.12 * (tono(261.6) + tono(329.6) + 0.5 * tono(523.2));
                let golpe = (-((t % 0.5) * 30.0)).exp() * 0.25;
                let (mut l, mut r) = (izquierda + golpe * ruido(), derecha + golpe * ruido());
                if con_voz {
                    let fase = t * 220.0 + 0.6 * (t * 5.5 * std::f32::consts::TAU).sin();
                    let voz: f32 = (1..=8).map(|armonico| (fase * armonico as f32 * std::f32::consts::TAU).sin() / armonico as f32).sum();
                    let silaba = (t * 4.0 * std::f32::consts::PI).sin().powi(2);
                    l += 0.3 * voz * silaba;
                    r += 0.3 * voz * silaba;
                }
                muestras.push(l.clamp(-1.0, 1.0));
                muestras.push(r.clamp(-1.0, 1.0));
                n += 1;
            }
        }
        muestras
    }

    fn promedio(puntajes: &[f32], desde: f64, hasta: f64) -> f32 {
        let tramo = |segundos: f64| (segundos * f64::from(TRAMOS_POR_SEGUNDO)) as usize;
        let parte = &puntajes[tramo(desde)..tramo(hasta)];
        parte.iter().sum::<f32>() / parte.len() as f32
    }

    #[test]
    fn la_voz_puntua_mas_que_el_acompanamiento() {
        let frecuencia = 22_050;
        let mut detector = DetectorDeVoz::nuevo(frecuencia);
        detector.sumar(&cancion(frecuencia, &[(8.0, false), (12.0, true), (6.0, false), (10.0, true)]));
        let puntajes = detector.puntajes();
        assert_eq!(puntajes.len(), 360);
        let (instrumental, cantado) = (promedio(&puntajes, 1.0, 7.0), promedio(&puntajes, 10.0, 19.0));
        assert!(cantado > 0.7 && instrumental < 0.3, "cantado {cantado} instrumental {instrumental}");
        assert!(promedio(&puntajes, 21.0, 25.0) < 0.3, "puente instrumental");
    }

    #[test]
    fn mide_el_tono_de_una_senal_periodica() {
        let tono: Vec<f32> = (0..800).map(|n| (n as f32 * 200.0 * std::f32::consts::TAU / 8_000.0).sin()).collect();
        assert!(armonia(&tono, 8_000.0) > 0.95);
        let mut semilla = 1u32;
        let ruido: Vec<f32> = (0..800)
            .map(|_| {
                semilla = semilla.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (semilla >> 9) as f32 / (1u32 << 23) as f32 - 0.5
            })
            .collect();
        assert!(armonia(&ruido, 8_000.0) < 0.5);
    }
}
