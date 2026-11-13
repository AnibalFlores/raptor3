//! Salida por la que suena la preescucha del editor.
//!
//! Si el usuario eligio otra salida de audio (por ejemplo unos auriculares) y
//! esta conectada, la preescucha abre alli un mezclador propio con dos voces:
//! la del tema y la del efecto de prueba, que asi no se oye por la salida de
//! la musica y no toca su atenuador. Con esa salida se puede preescuchar con
//! la musica sonando.
//!
//! Si no hay otra salida elegida, no esta conectada o resulta ser la misma
//! que la de la musica, la preescucha usa su voz en el mezclador principal,
//! como siempre: solo con la musica detenida o en pausa.
//!
//! En Windows la musica sale por la salida por defecto del sistema y la
//! sigue si el usuario la cambia; por eso se revisa periodicamente que la
//! salida de la preescucha no haya pasado a ser la de la musica.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::audio::{Mezclador, Motor, SalidaDeAudio, listar_salidas};
use crate::servicio::GestorDeOverlay;

// Voces del mezclador aparte: el tema y el efecto de prueba.
const VOCES_DE_LA_SALIDA_APARTE: usize = 2;
const REVISAR_LA_SALIDA_CADA: Duration = Duration::from_secs(1);
/// Primera opcion del combo de salidas.
pub(super) fn texto_de_la_misma_salida() -> String {
    tr!("Misma salida de la musica (con la musica detenida)")
}

/// Situacion de la salida elegida frente a las que hay ahora.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EstadoDeLaElegida {
    /// No se eligio otra salida.
    Ninguna,
    /// Esta conectada y es distinta de la de la musica (con su id actual).
    Disponible(SalidaDeAudio),
    NoConectada,
    /// Es la salida por la que suena la musica.
    EsLaDeLaMusica,
}

/// Busca la salida elegida entre las disponibles: por su identificador o, si
/// cambio (en Windows puede pasar al cambiar un USB de puerto), por su nombre
/// cuando hay una sola con ese nombre.
pub(super) fn buscar_elegida(
    elegida: Option<&SalidaDeAudio>,
    disponibles: &[SalidaDeAudio],
    id_de_la_musica: Option<&str>,
) -> EstadoDeLaElegida {
    let Some(elegida) = elegida else {
        return EstadoDeLaElegida::Ninguna;
    };
    let por_id = disponibles.iter().find(|salida| salida.id == elegida.id);
    let encontrada = por_id.or_else(|| {
        let mut del_mismo_nombre =
            disponibles.iter().filter(|salida| !elegida.nombre.is_empty() && salida.nombre == elegida.nombre);
        match (del_mismo_nombre.next(), del_mismo_nombre.next()) {
            (Some(unica), None) => Some(unica),
            _ => None,
        }
    });
    match encontrada {
        None => EstadoDeLaElegida::NoConectada,
        Some(salida) if Some(salida.id.as_str()) == id_de_la_musica => EstadoDeLaElegida::EsLaDeLaMusica,
        Some(salida) => EstadoDeLaElegida::Disponible(salida.clone()),
    }
}

/// Opcion del combo (despues de la de la misma salida).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OpcionDeSalida {
    pub salida: SalidaDeAudio,
    pub texto: String,
}

/// Opciones del combo: las salidas distintas de la de la musica y, si la
/// elegida no esta entre ellas, la elegida con el motivo. Devuelve tambien la
/// posicion de la elegida en el combo (0: la misma salida).
pub(super) fn opciones_de_salida(
    elegida: Option<&SalidaDeAudio>,
    estado: &EstadoDeLaElegida,
    disponibles: &[SalidaDeAudio],
    id_de_la_musica: Option<&str>,
) -> (Vec<OpcionDeSalida>, usize) {
    let mut opciones: Vec<OpcionDeSalida> = disponibles
        .iter()
        .filter(|salida| Some(salida.id.as_str()) != id_de_la_musica)
        .map(|salida| OpcionDeSalida { salida: salida.clone(), texto: salida.nombre.clone() })
        .collect();
    let posicion = match (estado, elegida) {
        (EstadoDeLaElegida::Disponible(salida), _) => opciones.iter().position(|opcion| opcion.salida.id == salida.id),
        (EstadoDeLaElegida::NoConectada, Some(elegida)) => {
            opciones.push(OpcionDeSalida { salida: elegida.clone(), texto: tr!("{} (no conectada)", elegida.nombre) });
            Some(opciones.len() - 1)
        }
        (EstadoDeLaElegida::EsLaDeLaMusica, Some(elegida)) => {
            opciones.push(OpcionDeSalida { salida: elegida.clone(), texto: tr!("{} (es la de la musica)", elegida.nombre) });
            Some(opciones.len() - 1)
        }
        _ => None,
    };
    (opciones, posicion.map_or(0, |posicion| posicion + 1))
}

/// Mezclador propio de la preescucha en otra salida.
pub(super) struct SalidaAparte {
    salida: SalidaDeAudio,
    mezclador: Mezclador,
    pub(super) motor: Motor,
    pub(super) efectos: GestorDeOverlay,
}

impl SalidaAparte {
    fn abrir(salida: SalidaDeAudio, carpeta_de_ondas: PathBuf) -> Result<Self, String> {
        let (mezclador, voces) = Mezclador::abrir_en(Some(&salida.id), VOCES_DE_LA_SALIDA_APARTE)?;
        let mut voces = voces.into_iter().map(|voz| Motor::new(voz, mezclador.frecuencia()));
        let (motor, motor_de_efectos) =
            (voces.next().expect("voz del tema"), voces.next().expect("voz del efecto"));
        // El tema baja mientras suena el efecto de prueba, como en la lista.
        motor.definir_atenuable(true);
        let efectos = GestorDeOverlay::new(motor_de_efectos, mezclador.atenuador(), carpeta_de_ondas);
        Ok(Self { salida, mezclador, motor, efectos })
    }
}

/// Salida de la preescucha: la voz en el mezclador principal y, si hay, la
/// salida aparte.
pub struct SalidaDePreescucha {
    principal: Mezclador,
    compartida: Motor,
    aparte: Option<SalidaAparte>,
    opciones: Vec<OpcionDeSalida>,
    elegida_en_el_combo: usize,
    carpeta_de_ondas: PathBuf,
    ultima_revision: Instant,
}

impl SalidaDePreescucha {
    /// `compartida` es la voz de preescucha del mezclador principal.
    pub fn new(principal: Mezclador, compartida: Motor, carpeta_de_ondas: PathBuf) -> Self {
        Self {
            principal,
            compartida,
            aparte: None,
            opciones: Vec::new(),
            elegida_en_el_combo: 0,
            carpeta_de_ondas,
            ultima_revision: Instant::now(),
        }
    }

    /// Vuelve a buscar las salidas y abre (o cierra) la salida aparte segun
    /// la elegida. Devuelve un aviso si la elegida no se puede usar.
    pub(super) fn actualizar(&mut self, elegida: Option<&SalidaDeAudio>) -> Option<String> {
        let disponibles = listar_salidas();
        let id_de_la_musica = self.principal.id_de_salida();
        let estado = buscar_elegida(elegida, &disponibles, id_de_la_musica.as_deref());
        let (opciones, posicion) = opciones_de_salida(elegida, &estado, &disponibles, id_de_la_musica.as_deref());
        self.opciones = opciones;
        self.elegida_en_el_combo = posicion;
        let nombre = elegida.map(|salida| salida.nombre.clone()).unwrap_or_default();
        match estado {
            EstadoDeLaElegida::Ninguna => {
                self.aparte = None;
                None
            }
            EstadoDeLaElegida::NoConectada => {
                self.aparte = None;
                Some(tr!("La salida {nombre} no esta conectada: la preescucha usa la de la musica.", nombre = nombre))
            }
            EstadoDeLaElegida::EsLaDeLaMusica => {
                self.aparte = None;
                Some(tr!("{nombre} es la salida de la musica: la preescucha la comparte, con la musica detenida.", nombre = nombre))
            }
            EstadoDeLaElegida::Disponible(salida) => {
                if self.aparte.as_ref().is_some_and(|aparte| aparte.salida.id == salida.id && !aparte.mezclador.perdida()) {
                    return None;
                }
                self.aparte = None;
                match SalidaAparte::abrir(salida, self.carpeta_de_ondas.clone()) {
                    Ok(aparte) => {
                        self.aparte = Some(aparte);
                        None
                    }
                    Err(error) => Some(tr!("No se pudo abrir la salida {nombre} ({error}): la preescucha usa la de la musica.", nombre = nombre, error = error)),
                }
            }
        }
    }

    /// Cierra la salida aparte (al cerrar el editor), para no mantener
    /// ocupada esa placa.
    pub(super) fn cerrar(&mut self) {
        self.aparte = None;
    }

    /// Una vez por segundo: la salida aparte se desconecto o paso a ser la
    /// de la musica. Devuelve el motivo; quien llama detiene la preescucha y
    /// luego llama a `cerrar`.
    pub(super) fn revisar(&mut self) -> Option<String> {
        let aparte = self.aparte.as_ref()?;
        if aparte.mezclador.perdida() {
            return Some(tr!("Se desconecto la salida {}: la preescucha vuelve a la de la musica.", aparte.salida.nombre));
        }
        if self.ultima_revision.elapsed() < REVISAR_LA_SALIDA_CADA {
            return None;
        }
        self.ultima_revision = Instant::now();
        (self.principal.id_de_salida().as_deref() == Some(aparte.salida.id.as_str())).then(|| {
            tr!("{} paso a ser la salida de la musica: la preescucha la comparte, con la musica detenida.", aparte.salida.nombre)
        })
    }

    /// Textos del combo: la misma salida y las opciones.
    pub(super) fn textos(&self) -> Vec<String> {
        std::iter::once(texto_de_la_misma_salida()).chain(self.opciones.iter().map(|opcion| opcion.texto.clone())).collect()
    }

    pub(super) fn elegida_en_el_combo(&self) -> usize {
        self.elegida_en_el_combo
    }

    /// Salida de una posicion del combo (0 o fuera de rango: la misma).
    pub(super) fn salida_en(&self, posicion: usize) -> Option<SalidaDeAudio> {
        posicion.checked_sub(1).and_then(|indice| self.opciones.get(indice)).map(|opcion| opcion.salida.clone())
    }

    /// La preescucha suena por otra salida que la musica.
    pub(super) fn es_aparte(&self) -> bool {
        self.aparte.is_some()
    }

    pub(super) fn aparte_mut(&mut self) -> Option<&mut SalidaAparte> {
        self.aparte.as_mut()
    }

    /// Motor que hace sonar el tema en la preescucha.
    pub(super) fn motor(&mut self) -> &mut Motor {
        match self.aparte.as_mut() {
            Some(aparte) => &mut aparte.motor,
            None => &mut self.compartida,
        }
    }

    pub(super) fn motor_ref(&self) -> &Motor {
        self.aparte.as_ref().map_or(&self.compartida, |aparte| &aparte.motor)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn salida(id: &str, nombre: &str) -> SalidaDeAudio {
        SalidaDeAudio { id: id.into(), nombre: nombre.into() }
    }

    fn disponibles() -> Vec<SalidaDeAudio> {
        vec![salida("wasapi:parlantes", "Parlantes"), salida("wasapi:auriculares", "Auriculares"), salida("wasapi:hdmi", "HDMI")]
    }

    #[test]
    fn sin_eleccion_la_preescucha_comparte_la_salida() {
        let estado = buscar_elegida(None, &disponibles(), Some("wasapi:parlantes"));
        assert_eq!(estado, EstadoDeLaElegida::Ninguna);
        let (opciones, posicion) = opciones_de_salida(None, &estado, &disponibles(), Some("wasapi:parlantes"));
        let textos: Vec<&str> = opciones.iter().map(|opcion| opcion.texto.as_str()).collect();
        assert_eq!(textos, ["Auriculares", "HDMI"], "la de la musica no se ofrece");
        assert_eq!(posicion, 0);
    }

    #[test]
    fn la_elegida_se_encuentra_por_id_y_si_cambio_por_nombre() {
        let auriculares = salida("wasapi:auriculares", "Auriculares");
        let estado = buscar_elegida(Some(&auriculares), &disponibles(), Some("wasapi:parlantes"));
        assert_eq!(estado, EstadoDeLaElegida::Disponible(auriculares.clone()));
        let (_, posicion) = opciones_de_salida(Some(&auriculares), &estado, &disponibles(), Some("wasapi:parlantes"));
        assert_eq!(posicion, 1);

        let con_otro_id = salida("wasapi:auriculares-viejo", "Auriculares");
        let estado = buscar_elegida(Some(&con_otro_id), &disponibles(), Some("wasapi:parlantes"));
        assert_eq!(estado, EstadoDeLaElegida::Disponible(auriculares), "se usa el id actual");
    }

    #[test]
    fn un_nombre_repetido_no_alcanza_para_reconocerla() {
        let mut repetidas = disponibles();
        repetidas.push(salida("wasapi:auriculares-2", "Auriculares"));
        let elegida = salida("wasapi:otro", "Auriculares");
        assert_eq!(buscar_elegida(Some(&elegida), &repetidas, None), EstadoDeLaElegida::NoConectada);
    }

    #[test]
    fn la_elegida_desconectada_queda_en_el_combo_con_el_motivo() {
        let usb = salida("wasapi:usb", "Placa USB");
        let estado = buscar_elegida(Some(&usb), &disponibles(), Some("wasapi:parlantes"));
        assert_eq!(estado, EstadoDeLaElegida::NoConectada);
        let (opciones, posicion) = opciones_de_salida(Some(&usb), &estado, &disponibles(), Some("wasapi:parlantes"));
        assert_eq!(posicion, 3);
        assert_eq!(opciones[2].texto, "Placa USB (no conectada)");
        assert_eq!(opciones[2].salida, usb);
    }

    #[test]
    fn la_elegida_no_puede_ser_la_de_la_musica() {
        // En Windows pasa si el usuario hace predeterminada la salida elegida.
        let auriculares = salida("wasapi:auriculares", "Auriculares");
        let estado = buscar_elegida(Some(&auriculares), &disponibles(), Some("wasapi:auriculares"));
        assert_eq!(estado, EstadoDeLaElegida::EsLaDeLaMusica);
        let (opciones, posicion) = opciones_de_salida(Some(&auriculares), &estado, &disponibles(), Some("wasapi:auriculares"));
        let textos: Vec<&str> = opciones.iter().map(|opcion| opcion.texto.as_str()).collect();
        assert_eq!(textos, ["Parlantes", "HDMI", "Auriculares (es la de la musica)"]);
        assert_eq!(posicion, 3);
    }

    #[test]
    #[ignore = "lista las salidas reales del equipo; se pide a mano"]
    fn listar_las_salidas_del_equipo() {
        for salida in listar_salidas() {
            println!("{} -> {}", salida.nombre, salida.id);
        }
    }

    #[test]
    #[ignore = "abre cada salida real del equipo; se pide a mano"]
    fn cada_salida_del_equipo_se_abre_y_avanza() {
        for salida in listar_salidas() {
            let (mezclador, voces) = Mezclador::abrir_en(Some(&salida.id), VOCES_DE_LA_SALIDA_APARTE)
                .unwrap_or_else(|error| panic!("{}: {error}", salida.nombre));
            assert_eq!(voces.len(), VOCES_DE_LA_SALIDA_APARTE);
            assert_eq!(mezclador.id_de_salida().as_deref(), Some(salida.id.as_str()));
            let reloj = mezclador.atenuador().reloj();
            std::thread::sleep(Duration::from_millis(400));
            let avance = mezclador.atenuador().reloj() - reloj;
            println!("{}: {} Hz, {avance} frames en 400 ms", salida.nombre, mezclador.frecuencia());
            assert!(avance > u64::from(mezclador.frecuencia()) / 10, "{} no avanza", salida.nombre);
            assert!(!mezclador.perdida());
        }
    }
}
