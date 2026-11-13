//! Volumen de un efecto de la botonera y cuanto baja la musica mientras
//! suena ("ducking"), con sus tiempos.
//!
//! Ajuste inteligente:
//! - El efecto se lleva a la sonoridad con que se escucha la musica (la del
//!   tema, con la nivelacion y el volumen elegido), mas el volumen del efecto
//!   que elija el usuario. Asi un efecto grabado bajo y uno grabado fuerte
//!   salen parejos. Nunca se sube por encima de -1 dBTP de pico real.
//! - La musica baja lo justo para quedar `separacion_db` por debajo del
//!   efecto (un ajuste del usuario: 12 dB por defecto), segun lo fuerte que
//!   suena ese tramo del tema: en una intro suave casi no baja y en un
//!   estribillo fuerte baja mas, pero nunca mas de la separacion mas
//!   `MARGEN_DE_ATENUACION_DB`, para que no desaparezca.
//! - La recuperacion dura mas cuanto mas bajo la musica, para que vuelva sin
//!   saltos.
//!
//! Sin ajuste inteligente el efecto suena con el volumen elegido y la musica
//! baja al nivel fijo de "Musica al".
//!
//! En los dos casos la musica baja en una rampa de `anticipacion` segundos
//! (1/25 s por defecto) que termina justo cuando arranca el efecto, y vuelve
//! con una rampa suave cuando el efecto deja de sonar (su ultimo sonido, sin
//! el silencio final).

use super::mezcla_inteligente::SONORIDAD_DE_REFERENCIA;

/// Segundos que la musica empieza a bajar antes del efecto, por defecto (1/25 s).
pub const ANTICIPACION: f64 = 0.04;
const ANTICIPACION_MINIMA: f64 = 0.01;
const ANTICIPACION_MAXIMA: f64 = 2.0;
/// Cuanto por debajo del efecto queda la musica con el ajuste inteligente,
/// por defecto y sus limites, en dB.
pub const SEPARACION_POR_DEFECTO_DB: f64 = 12.0;
pub const SEPARACION_MINIMA_DB: f64 = 4.0;
pub const SEPARACION_MAXIMA_DB: f64 = 24.0;
/// Lo mas que baja la musica con el ajuste inteligente: la separacion mas
/// este margen (en un tramo fuerte, o con un efecto bajo).
const MARGEN_DE_ATENUACION_DB: f64 = 6.0;
const GANANCIA_MINIMA_DB: f64 = -20.0;
/// La salida de una voz admite hasta +6 dB.
const GANANCIA_MAXIMA_DB: f64 = 6.0;
const TECHO_DE_PICO_REAL_DB: f64 = -1.0;
const RECUPERACION_MINIMA: f64 = 0.2;
const RECUPERACION_MAXIMA: f64 = 4.0;

/// Lo que elige el usuario.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AjustesDeEfecto {
    /// Volumen del efecto en dB (0 es el original, o la sonoridad de la
    /// musica con el ajuste inteligente).
    pub volumen_db: f64,
    /// Bajar la musica mientras suena el efecto.
    pub atenuar: bool,
    pub inteligente: bool,
    /// Nivel fijo de la musica (0 a 1) sin ajuste inteligente.
    pub nivel_de_musica: f64,
    /// Segundos que tarda la musica en bajar; la bajada termina cuando
    /// arranca el efecto.
    pub anticipacion: f64,
    /// Segundos que tarda la musica en volver.
    pub recuperacion: f64,
    /// Con el ajuste inteligente, cuantos dB por debajo del efecto queda la
    /// musica (mas separacion, la baja mas). Vale tambien para la voz del
    /// anuncio HTH sobre su base.
    pub separacion_db: f64,
}

impl Default for AjustesDeEfecto {
    fn default() -> Self {
        Self {
            volumen_db: 0.0,
            atenuar: false,
            inteligente: true,
            nivel_de_musica: 0.5,
            anticipacion: ANTICIPACION,
            recuperacion: 0.8,
            separacion_db: SEPARACION_POR_DEFECTO_DB,
        }
    }
}

/// Lo que se sabe del archivo del efecto.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerfilDeEfecto {
    pub sonoridad: Option<f64>,
    /// Pico real, de 0 a 1.
    pub pico: f64,
    /// Segundos desde el comienzo hasta su ultimo sonido.
    pub fin_audible: Option<f64>,
}

/// Como suena la musica alrededor del efecto.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MusicaAlrededor {
    /// Sonoridad del tema tal como se escucha (con nivelacion y volumen), en LUFS.
    pub sonoridad: Option<f64>,
    /// Cuanto mas fuerte (o mas suave, negativo) que lo tipico del tema suena
    /// el tramo donde cae el efecto, en dB.
    pub desvio_local_db: f64,
}

/// Lo que hay que hacer para un efecto.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanDeEfecto {
    /// Volumen del efecto (factor).
    pub ganancia: f64,
    /// Nivel de la musica mientras suena (factor; 1 no baja).
    pub nivel_de_musica: f64,
    pub anticipacion: f64,
    /// Segundos desde que arranca el efecto hasta que la musica empieza a
    /// volver; None si no se sabe (se vuelve cuando termina).
    pub volver_tras: Option<f64>,
    pub recuperacion: f64,
}

impl PlanDeEfecto {
    pub fn ganancia_db(&self) -> f64 {
        decibeles(self.ganancia)
    }

    pub fn atenuacion_db(&self) -> f64 {
        -decibeles(self.nivel_de_musica)
    }

    /// Descripcion corta: "efecto +2.1 dB, musica -9.8 dB en 40 ms, vuelve en 0.9 s".
    pub fn descripcion(&self) -> String {
        let mut texto = tr!("efecto {:+.1} dB", self.ganancia_db());
        if self.nivel_de_musica < 0.999 {
            texto.push_str(&tr!(", musica -{:.1} dB en {:.0} ms, vuelve en {:.1} s", self.atenuacion_db(), self.anticipacion * 1000.0, self.recuperacion));
        }
        texto
    }
}

fn decibeles(factor: f64) -> f64 {
    20.0 * factor.max(1e-6).log10()
}

fn factor(decibeles: f64) -> f64 {
    10f64.powf(decibeles / 20.0)
}

pub fn planificar(ajustes: &AjustesDeEfecto, perfil: &PerfilDeEfecto, musica: Option<&MusicaAlrededor>) -> PlanDeEfecto {
    // Sonoridad de referencia: la de la musica como se escucha o, sin
    // musica, la de la nivelacion.
    let referencia = musica.and_then(|musica| musica.sonoridad).unwrap_or(SONORIDAD_DE_REFERENCIA);
    let ganancia_db = match perfil.sonoridad.filter(|_| ajustes.inteligente) {
        Some(sonoridad) => {
            let techo = TECHO_DE_PICO_REAL_DB - decibeles(perfil.pico);
            (referencia + ajustes.volumen_db - sonoridad).min(techo).clamp(GANANCIA_MINIMA_DB, GANANCIA_MAXIMA_DB)
        }
        None => ajustes.volumen_db.clamp(GANANCIA_MINIMA_DB, GANANCIA_MAXIMA_DB),
    };
    let (nivel_de_musica, recuperacion) = if !ajustes.atenuar || musica.is_none() {
        (1.0, ajustes.recuperacion)
    } else if ajustes.inteligente {
        let musica = musica.expect("hay musica");
        // Sonoridad del efecto ya ajustado, y la de la musica en ese tramo.
        let efecto = perfil.sonoridad.map_or(referencia + ajustes.volumen_db, |sonoridad| sonoridad + ganancia_db);
        let tramo = referencia + musica.desvio_local_db;
        let separacion = ajustes.separacion_db.clamp(SEPARACION_MINIMA_DB, SEPARACION_MAXIMA_DB);
        let atenuacion = (tramo - (efecto - separacion)).clamp(0.0, separacion + MARGEN_DE_ATENUACION_DB);
        // 10 dB vuelven en el tiempo elegido; menos, en menos tiempo.
        let recuperacion = ajustes.recuperacion * (0.5 + atenuacion / 20.0);
        (factor(-atenuacion), recuperacion)
    } else {
        (ajustes.nivel_de_musica.clamp(0.0, 1.0), ajustes.recuperacion)
    };
    PlanDeEfecto {
        ganancia: factor(ganancia_db),
        nivel_de_musica,
        anticipacion: ajustes.anticipacion.clamp(ANTICIPACION_MINIMA, ANTICIPACION_MAXIMA),
        volver_tras: perfil.fin_audible,
        recuperacion: recuperacion.clamp(RECUPERACION_MINIMA, RECUPERACION_MAXIMA),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn perfil(sonoridad: f64) -> PerfilDeEfecto {
        PerfilDeEfecto { sonoridad: Some(sonoridad), pico: 0.3, fin_audible: Some(3.0) }
    }

    fn musica(sonoridad: f64, desvio: f64) -> MusicaAlrededor {
        MusicaAlrededor { sonoridad: Some(sonoridad), desvio_local_db: desvio }
    }

    fn inteligente() -> AjustesDeEfecto {
        AjustesDeEfecto { atenuar: true, ..AjustesDeEfecto::default() }
    }

    #[test]
    fn lleva_el_efecto_a_la_sonoridad_de_la_musica() {
        let plan = planificar(&inteligente(), &perfil(-20.0), Some(&musica(-14.0, 0.0)));
        assert!((plan.ganancia_db() - 6.0).abs() < 1e-9, "{plan:?}");
        let plan = planificar(&inteligente(), &perfil(-8.0), Some(&musica(-14.0, 0.0)));
        assert!((plan.ganancia_db() + 6.0).abs() < 1e-9);
        // El volumen del efecto se suma.
        let ajustes = AjustesDeEfecto { volumen_db: 3.0, ..inteligente() };
        assert!((planificar(&ajustes, &perfil(-17.0), Some(&musica(-14.0, 0.0))).ganancia_db() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn no_sube_el_efecto_por_encima_del_pico_tolerado() {
        let fuerte = PerfilDeEfecto { pico: factor(-3.0), ..perfil(-25.0) };
        let plan = planificar(&inteligente(), &fuerte, Some(&musica(-14.0, 0.0)));
        assert!((plan.ganancia_db() - 2.0).abs() < 1e-9, "{plan:?}");
    }

    #[test]
    fn la_musica_baja_segun_el_tramo() {
        let suave = planificar(&inteligente(), &perfil(-14.0), Some(&musica(-14.0, -6.0)));
        assert!((suave.atenuacion_db() - 6.0).abs() < 1e-9, "intro suave: 6 dB");
        let plan = planificar(&inteligente(), &perfil(-14.0), Some(&musica(-14.0, 0.0)));
        assert!((plan.atenuacion_db() - 12.0).abs() < 1e-9, "tramo tipico: la separacion, {plan:?}");
        assert!(plan.recuperacion > suave.recuperacion, "baja mas, vuelve mas lento");
        // En un tramo fuerte baja hasta la separacion mas el margen (18 dB).
        let fuerte = planificar(&inteligente(), &perfil(-14.0), Some(&musica(-14.0, 10.0)));
        assert!((fuerte.atenuacion_db() - 18.0).abs() < 1e-9, "{fuerte:?}");
        // Un efecto que no llega a la sonoridad de la musica tampoco la hunde.
        let bajo = planificar(&inteligente(), &perfil(-30.0), Some(&musica(-14.0, 0.0)));
        assert!((bajo.atenuacion_db() - 18.0).abs() < 1e-9);
        // Un efecto mas fuerte necesita bajar menos la musica.
        let ajustes = AjustesDeEfecto { volumen_db: 6.0, ..inteligente() };
        assert!((planificar(&ajustes, &perfil(-14.0), Some(&musica(-14.0, 0.0))).atenuacion_db() - 6.0).abs() < 1e-9);
        // Mas separacion, la musica baja mas (y queda dentro de sus limites).
        let separada = AjustesDeEfecto { separacion_db: 20.0, ..inteligente() };
        assert!((planificar(&separada, &perfil(-14.0), Some(&musica(-14.0, 0.0))).atenuacion_db() - 20.0).abs() < 1e-9);
        let exagerada = AjustesDeEfecto { separacion_db: 60.0, ..inteligente() };
        assert!((planificar(&exagerada, &perfil(-14.0), Some(&musica(-14.0, 0.0))).atenuacion_db() - SEPARACION_MAXIMA_DB).abs() < 1e-9);
    }

    #[test]
    fn sin_ajuste_inteligente_usa_los_valores_fijos() {
        let ajustes = AjustesDeEfecto {
            inteligente: false,
            atenuar: true,
            volumen_db: -3.0,
            nivel_de_musica: 0.3,
            anticipacion: 0.25,
            recuperacion: 1.5,
            separacion_db: 12.0,
        };
        let plan = planificar(&ajustes, &perfil(-30.0), Some(&musica(-14.0, 0.0)));
        assert!((plan.ganancia_db() + 3.0).abs() < 1e-9);
        assert_eq!((plan.nivel_de_musica, plan.anticipacion, plan.recuperacion), (0.3, 0.25, 1.5));
        assert_eq!(planificar(&inteligente(), &perfil(-14.0), None).anticipacion, 0.04, "1/25 s por defecto");
        assert_eq!(plan.volver_tras, Some(3.0));
    }

    #[test]
    fn sin_atenuar_ni_musica_la_musica_no_baja() {
        let ajustes = AjustesDeEfecto { atenuar: false, ..inteligente() };
        assert_eq!(planificar(&ajustes, &perfil(-14.0), Some(&musica(-14.0, 0.0))).nivel_de_musica, 1.0);
        let plan = planificar(&inteligente(), &perfil(-15.0), None);
        assert_eq!(plan.nivel_de_musica, 1.0);
        assert!((plan.ganancia_db() - (SONORIDAD_DE_REFERENCIA + 15.0)).abs() < 1e-9, "sin musica, a la referencia");
    }
}
