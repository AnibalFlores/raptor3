//! Audio: decodificacion nativa con symphonia, mezcla propia, salida con
//! cpal y forma de onda para el editor de pistas.

mod decodificador;
mod mezclador;
mod motor;
mod onda;
mod prioridad;
mod ritmo;
mod voz;

pub use decodificador::{Analisis, Decodificador, anio_valido};
pub use mezclador::{Atenuador, Mezclador, SalidaDeAudio, listar_salidas};
pub use motor::Motor;
pub use onda::{FormaDeOnda, limpiar_cache_de_ondas};
pub use prioridad::{PrioridadDeHilo, fijar_prioridad_del_hilo_actual};
pub use ritmo::{Ritmo, Tempo, desfase_para_alinear, tempos_coinciden};

/// Utilidades de prueba de los modulos de audio para otros modulos.
#[cfg(test)]
pub mod pruebas {
    pub use super::onda::pruebas::{carpeta_temporal, escribir_estereo, escribir_wav};
    pub use super::voz::pruebas::cancion;
}
