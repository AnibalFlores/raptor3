//! Servicios que coordinan los motores de audio: la musica en secuencia con
//! mezcla y los efectos de la botonera.

mod anuncio_hth;
mod catalogo;
mod clima;
mod gestor_de_overlay;
mod gestor_de_reproduccion;
mod mezcla_de_efectos;
mod mezcla_inteligente;
mod musicbrainz;
mod sugerencia_de_overlay;

pub use anuncio_hth::{Azar, Locuciones, armar as armar_anuncio_hth, elegir_base as elegir_base_hth, piezas_del_anuncio, resumen as resumen_del_anuncio};
pub use catalogo::{ArchivoPerdido, Hallazgo, buscar_movidos};
pub use clima::{LecturaDeClima, Localidad, LocalidadEncontrada, ServicioDeClima, buscar_localidades};
// Solo las pruebas arman lecturas de clima a mano.
#[cfg(test)]
pub use clima::Clima;
pub use gestor_de_overlay::{Entorno, EstadoDeEspera, GestorDeOverlay, ModoDeOverlay, RecorteDeEfecto};
pub use gestor_de_reproduccion::{Curva, EventoDeReproduccion, GestorDeReproduccion, Transicion};
pub use mezcla_de_efectos::{AjustesDeEfecto, MusicaAlrededor, SEPARACION_MAXIMA_DB, SEPARACION_MINIMA_DB};
pub use mezcla_inteligente::{DatosDeMezcla, ObjetivoDeSonoridad, RANGO_DINAMICO_AMPLIO, transicion_inteligente};
pub use musicbrainz::{PAUSA_ENTRE_CONSULTAS, buscar_grabacion};
pub use sugerencia_de_overlay::SugerenciaDeOverlay;
